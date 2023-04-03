// Copyright 2022 System76 <debug@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::cfs::paths::SchedPaths;
use crate::config::scheduler::Profile;
use crate::process::{self, Process};
use crate::utils::Buffer;
use qcell::{LCell, LCellOwner};
use std::collections::BTreeMap;
use std::time::Duration;
use std::{os::unix::prelude::OsStrExt, sync::Arc};
use system76_scheduler_config::scheduler::Condition;

pub struct Service<'owner> {
    pub config: crate::config::Config,
    cfs_paths: SchedPaths,
    foreground_processes: Vec<u32>,
    foreground: Option<u32>,
    pipewire_processes: Vec<u32>,
    process_map: process::Map<'owner>,
    owner: LCellOwner<'owner>,
}

impl<'owner> Service<'owner> {
    pub fn new(owner: LCellOwner<'owner>, cfs_paths: SchedPaths) -> Self {
        Self {
            config: crate::config::Config::default(),
            cfs_paths,
            foreground: None,
            foreground_processes: Vec::with_capacity(128),
            pipewire_processes: Vec::with_capacity(4),
            process_map: process::Map::default(),
            owner,
        }
    }

    pub fn cfs_apply(&self, config: &crate::config::cfs::Profile) {
        if self.config.cfs_profiles.enable {
            return;
        }

        crate::cfs::tweak(&self.cfs_paths, config);
    }

    pub fn cfs_on_battery(&self, on_battery: bool) {
        self.cfs_apply(if on_battery {
            self.cfs_default_config()
        } else {
            self.cfs_responsive_config()
        });
    }

    pub fn cfs_config(&self, name: &str) -> Option<&crate::config::cfs::Profile> {
        self.config.cfs_profiles.profiles.get(name)
    }

    pub fn cfs_default_config(&self) -> &crate::config::cfs::Profile {
        self.cfs_config("default")
            .unwrap_or(&crate::config::cfs::PROFILE_DEFAULT)
    }

    pub fn cfs_responsive_config(&self) -> &crate::config::cfs::Profile {
        self.cfs_config("responsive")
            .unwrap_or(&crate::config::cfs::PROFILE_RESPONSIVE)
    }

    /// Assign a priority to a newly-created process, and record that process in the map.
    pub async fn assign_new_process(
        &mut self,
        buffer: &mut Buffer,
        pid: u32,
        parent_pid: u32,
        name: String,
        mut cmdline: String,
    ) {
        let parent = self.process_map.get_pid(&self.owner, parent_pid).cloned();

        if parent.is_none() {
            self.process_map_refresh(buffer);
            return;
        }

        if !process::exists(buffer, pid) {
            return;
        }

        if cmdline.is_empty() {
            cmdline = process::cmdline(buffer, pid).unwrap_or_default();
        }

        let mut cgroup = String::new();
        let mut attempts = 0;

        while cgroup.is_empty() || attempts < 2 {
            cgroup = process::cgroup(buffer, pid)
                .map(String::from)
                .unwrap_or_default();
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !process::exists(buffer, pid) {
                return;
            }
            attempts += 1;
        }

        // Add the process to the map, if it does not already exist.
        let process = self.process_map.insert(
            &mut self.owner,
            Process {
                id: pid,
                parent_id: parent_pid,
                cgroup: process::cgroup(buffer, pid)
                    .map(String::from)
                    .unwrap_or_default(),
                cmdline,
                name,
                parent: parent.as_ref().map(Arc::downgrade),
                ..Process::default()
            },
        );

        self.assign_process_priority(&process);
        self.apply_process_priority(buffer, process.ro(&self.owner));
    }

    /// Gets the config-assigned priority of a process.
    #[must_use]
    pub fn process_assignment(&self, pid: u32) -> Priority {
        let Some(process) = self.process_map.get_pid(&self.owner, pid) else {
            return Priority::NotAssignable;
        };

        process.ro(&self.owner).assigned_priority.as_ref()
    }

    pub fn assign_process_priority(&mut self, process: &LCell<'owner, Process<'owner>>) {
        if OwnedPriority::NotAssignable != process.ro(&self.owner).assigned_priority {
            return;
        }

        let priority = (|| {
            let process = process.ro(&self.owner);

            if self.process_is_exception(process) {
                return OwnedPriority::Exception;
            }

            if let Some(profile) = self
                .config
                .process_scheduler
                .assignments
                .get_by_cmdline(&process.cmdline)
            {
                return OwnedPriority::Config(profile.clone());
            }

            if let Some(profile) = self
                .config
                .process_scheduler
                .assignments
                .get_by_name(&process.name)
            {
                return OwnedPriority::Config(profile.clone());
            }

            // True when all conditions for a profile are met by a process.
            let condition_met = |condition: &Condition| {
                if let Some(ref cgroup) = condition.cgroup {
                    if !cgroup.matches(&process.cgroup) {
                        return false;
                    }
                }

                if !condition.parent.is_empty() {
                    let mut has_parent = false;

                    if let Some(parent) = process.parent() {
                        let parent = parent.ro(&self.owner);
                        has_parent = condition
                            .parent
                            .iter()
                            .any(|condition| condition.matches(&parent.name));
                    }

                    if !has_parent {
                        return false;
                    }
                }

                if let Some(ref descends_condition) = condition.descends {
                    let is_ancestor = process.ancestors(&self.owner).any(|parent| {
                        let parent = parent.ro(&self.owner);
                        descends_condition.matches(&parent.name)
                    });

                    if !is_ancestor {
                        return false;
                    }
                }

                true
            };

            'outer: for (profile, conditions) in self
                .config
                .process_scheduler
                .assignments
                .conditions
                .values()
            {
                let mut assigned_profile = None;

                for (condition, include) in conditions {
                    match (condition_met(condition), *include) {
                        // Condition met for an include rule
                        (true, true) => assigned_profile = Some(profile),
                        // Condition met for an exclude rule
                        (true, false) => continue 'outer,
                        _ => (),
                    }
                }

                if let Some(profile) = assigned_profile.take() {
                    return OwnedPriority::Config(profile.clone());
                }
            }

            OwnedPriority::Assignable
        })();

        process.rw(&mut self.owner).assigned_priority = priority;
    }

    // Check if the `process` has descended from the `ancestor`
    pub fn process_descended_from(&self, process: &Process<'owner>, ancestor: u32) -> bool {
        process.ancestors(&self.owner).any(|process| {
            let process = process.ro(&self.owner);
            process.id == ancestor
        })
    }

    // Check if the `process` is excepted from process priority changes
    pub fn process_is_exception(&self, process: &Process<'owner>) -> bool {
        // Return if listed as an exception by its cmdline path
        if self
            .config
            .process_scheduler
            .assignments
            .is_exception_by_cmdline(&process.cmdline)
        {
            return true;
        }

        // Return if listed as an exception by process name
        if self
            .config
            .process_scheduler
            .assignments
            .is_exception_by_name(&process.name)
        {
            return true;
        }

        // Condition-based exceptions
        for condition in &self
            .config
            .process_scheduler
            .assignments
            .exceptions_conditions
        {
            // Checks if the process descends from an excepted parent process.
            if let Some(condition) = &condition.descends {
                if !condition.matches(&process.forked_name) {
                    let ancestry_match = process.ancestors(&self.owner).any(|parent| {
                        let parent = parent.ro(&self.owner);
                        condition.matches(&parent.name) || condition.matches(&parent.forked_name)
                    });

                    if !ancestry_match {
                        continue;
                    }
                }
            }

            // Checks if a process has a direct parent of the same name.
            if !condition.parent.is_empty() {
                let parent_match = condition.parent.iter().any(|condition| {
                    process.parent().map_or(false, |parent| {
                        let parent = parent.ro(&self.owner);
                        condition.matches(&parent.name) || condition.matches(&parent.forked_name)
                    })
                });

                if !parent_match {
                    continue;
                }
            }

            return true;
        }

        false
    }

    pub fn apply_process_priority(&self, buffer: &mut Buffer, process: &Process<'owner>) {
        let profile_default;

        let profile = match process.assigned_priority.as_ref() {
            Priority::Assignable => {
                if let Some(ref profile) = self.config.process_scheduler.pipewire {
                    if self.pipewire_processes.contains(&process.id) {
                        crate::priority::set(buffer, process.id, profile);
                        return;
                    }
                }

                if let (Some(assignments), Some(foreground)) =
                    (&self.config.process_scheduler.foreground, &self.foreground)
                {
                    if process.id == *foreground || self.foreground_processes.contains(&process.id)
                    {
                        &assignments.foreground
                    } else {
                        &assignments.background
                    }
                } else {
                    profile_default = Profile::default();
                    &profile_default
                }
            }

            Priority::Config(profile) => profile,

            _ => return,
        };

        crate::priority::set(buffer, process.id, profile);
    }

    /// Adds a new process to the process map
    pub fn process_map_insert(
        &mut self,
        process: Process<'owner>,
    ) -> Arc<LCell<'owner, Process<'owner>>> {
        self.process_map.insert(&mut self.owner, process)
    }

    /// Refreshes the process map
    pub fn process_map_refresh(&mut self, buffer: &mut Buffer) {
        self.process_map.drain_filter_prepare();

        let mut parents = BTreeMap::new();
        let Ok(procfs) = std::fs::read_dir("/proc/") else {
            tracing::error!("failed to read /proc directory: process monitoring stopped");
            return;
        };

        for proc_entry in procfs.filter_map(Result::ok) {
            let file_name = proc_entry.file_name();

            let mut process = Process::default();

            match atoi::atoi::<u32>(file_name.as_bytes()) {
                Some(pid) => process.id = pid,
                None => continue,
            }

            // Processes without a command line path are kernel threads
            match process::cmdline(buffer, process.id) {
                Some(cmdline) => process.cmdline = cmdline,
                None => continue,
            }

            process.name = process::name(&process.cmdline).to_owned();

            if let Some(cgroup) = process::cgroup(buffer, process.id) {
                process.cgroup = cgroup.to_owned();
            }

            if let Some(ppid) = process::parent_id(buffer, process.id) {
                parents.insert(process.id, ppid);
                process.parent_id = ppid;
            }

            self.process_map.retain_process_tree(&self.owner, &process);
            self.process_map_insert(process);
        }

        for (pid, ppid) in parents {
            if let Some(process) = self.process_map.get_pid(&self.owner, pid) {
                if let Some(parent) = self.process_map.get_pid(&self.owner, ppid) {
                    process.rw(&mut self.owner).parent = Some(Arc::downgrade(parent));
                }
            }
        }

        self.process_map.drain_filter();

        // Refresh priority assignments
        let mut process_map = process::Map::default();
        std::mem::swap(&mut process_map, &mut self.process_map);

        for process in process_map.map.values() {
            self.assign_process_priority(process);
            self.apply_process_priority(buffer, process.ro(&self.owner));
        }

        std::mem::swap(&mut process_map, &mut self.process_map);

        // Reassign foreground processes in case they were overriden.
        if let Some(process) = self.foreground.take() {
            self.set_foreground_process(buffer, process);
        }
    }

    /// Reloads the configuration files.
    pub fn reload_configuration(&mut self) {
        self.config = crate::config::config();
    }

    /// Sets a process as the foreground.
    pub fn set_foreground_process(&mut self, buffer: &mut Buffer, pid: u32) {
        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            self.foreground = Some(pid);
            self.foreground_processes.clear();

            if !self.pipewire_processes.contains(&pid) {
                if let Some(priority) = self
                    .process_assignment(pid)
                    .with_default(&assignments.foreground)
                {
                    crate::priority::set(buffer, pid, priority);
                }
            }

            self.foreground_processes.push(pid);

            for process in self.process_map.map.values() {
                let process = process.ro(&self.owner);

                if process.id == pid {
                    continue;
                }

                if let Priority::Assignable = self.process_assignment(process.id) {
                    let profile = if self.process_descended_from(process, pid) {
                        self.foreground_processes.push(process.id);

                        if self.pipewire_processes.contains(&process.id) {
                            continue;
                        }

                        &assignments.foreground
                    } else if !self.pipewire_processes.contains(&process.id) {
                        &assignments.background
                    } else {
                        continue;
                    };

                    crate::priority::set(buffer, process.id, profile);
                }
            }
        }
    }

    /// Assigns a process to the pipewire profile if it does not already have an assignment.
    pub fn set_pipewire_process(&mut self, buffer: &mut Buffer, process: u32) {
        if let Some(pipewire) = self.config.process_scheduler.pipewire.clone() {
            if !self.pipewire_processes.contains(&process) {
                self.pipewire_processes.push(process);

                if let Priority::Assignable = self.process_assignment(process) {
                    crate::priority::set(buffer, process, &pipewire);
                }
            }
        }
    }

    /// Removes a process from the pipewire profile.
    ///
    /// Assigns the background or foreground process priority, if that feature is enabled.
    pub fn remove_pipewire_process(&mut self, buffer: &mut Buffer, process_id: u32) {
        let Some(index) = self.pipewire_processes.iter().position(|pid| *pid == process_id) else {
            return;
        };

        self.pipewire_processes.remove(index);

        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            if let Priority::Assignable = self.process_assignment(process_id) {
                let profile = if self.foreground_processes.contains(&process_id) {
                    &assignments.foreground
                } else {
                    &assignments.background
                };

                crate::priority::set(buffer, process_id, profile);
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Priority<'a> {
    Assignable,
    Config(&'a Profile),
    Exception,
    NotAssignable,
}

impl<'a> Priority<'a> {
    pub fn with_default(self, priority: &'a Profile) -> Option<&'a Profile> {
        match self {
            Priority::Assignable => Some(priority),
            Priority::Config(config) => Some(config),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum OwnedPriority {
    Assignable,
    Config(Profile),
    Exception,
    #[default]
    NotAssignable,
}

impl OwnedPriority {
    fn as_ref(&self) -> Priority {
        match self {
            Self::Assignable => Priority::Assignable,
            Self::Config(profile) => Priority::Config(profile),
            Self::Exception => Priority::Exception,
            Self::NotAssignable => Priority::NotAssignable,
        }
    }
}
