// Copyright 2022 System76 <debug@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::config::scheduler::Profile;
use crate::process::{self, Process};
use crate::utils::Buffer;
use crate::{cfs::paths::SchedPaths, utils};
use concat_in_place::strcat;
use qcell::LCellOwner;
use std::collections::BTreeMap;
use std::{os::unix::prelude::OsStrExt, sync::Arc};

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

    /// Assign a priority to a process
    pub fn assign(&self, buffer: &mut Buffer, pid: u32, default: &Profile) -> AssignmentStatus {
        if let Some(profile) = self.assigned_priority(pid).with_default(default) {
            crate::priority::set(buffer, pid, profile);

            return AssignmentStatus::Assigned;
        }

        AssignmentStatus::Unset
    }

    /// Gets the config-assigned priority of a process.
    #[must_use]
    pub fn assigned_priority(&self, pid: u32) -> Priority {
        let Some(process) = self.process_map.get_pid(&self.owner, pid) else {
            return Priority::NotAssignable;
        };

        let process = process.ro(&self.owner);

        if process.exception {
            return Priority::Exception;
        }

        if let Some(profile) = self
            .config
            .process_scheduler
            .assignments
            .get_by_cmdline(&process.cmdline)
        {
            return Priority::Config(profile);
        }

        if let Some(profile) = self
            .config
            .process_scheduler
            .assignments
            .get_by_name(&process.name)
        {
            return Priority::Config(profile);
        }

        for (condition, profile) in &self.config.process_scheduler.assignments.conditions {
            if let Some(ref cgroup) = condition.cgroup {
                if !cgroup.matches(&process.cgroup) {
                    continue;
                }
            }

            if let Some(ref parent_condition) = condition.parent {
                if let Some(parent) = process.parent() {
                    let parent = parent.ro(&self.owner);
                    if !parent_condition.matches(&parent.name) {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            if let Some(ref descends_condition) = condition.descends {
                for parent in process.ancestors(&self.owner) {
                    let parent = parent.ro(&self.owner);
                    if descends_condition.matches(&parent.name) {
                        continue;
                    }
                }
            }

            return Priority::Config(profile);
        }

        Priority::Assignable
    }

    /// Assign a priority to a newly-created process, and record that process in the map.
    pub fn assign_new_process(
        &mut self,
        buffer: &mut Buffer,
        pid: u32,
        parent_pid: u32,
        name: String,
        cmdline: String,
    ) {
        let Some(parent) = self.process_map.get_pid(&self.owner, parent_pid) else {
            return;
        };

        // Add the process to the map, if it does not already exist.
        self.insert_process(Process {
            id: pid,
            parent_id: parent_pid,
            cgroup: process::cgroup(buffer, pid)
                .map(String::from)
                .unwrap_or_default(),
            cmdline,
            name,
            parent: Some(Arc::downgrade(parent)),
            ..Process::default()
        });

        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            if self.foreground_processes.contains(&parent_pid)
                && !self.foreground_processes.contains(&pid)
            {
                if let AssignmentStatus::Assigned =
                    self.assign(buffer, pid, &assignments.foreground)
                {
                    self.foreground_processes.push(pid);
                    return;
                }
            }
        }

        match self.assigned_priority(pid) {
            // Apply preferred config
            Priority::Config(config) => {
                self.assign(buffer, pid, config);
            }

            // When foreground process management is enabled, apply it.
            Priority::Assignable => {
                if let Some(ref assignments) = self.config.process_scheduler.foreground {
                    if self.foreground_processes.contains(&pid) {
                        return;
                    }

                    let _status = self.assign(buffer, pid, &assignments.background);
                }
            }

            _ => (),
        }
    }

    pub fn insert_process(&mut self, process: Process<'owner>) {
        let process = self.process_map.insert(&mut self.owner, process);
        if self.process_is_exception(process.ro(&self.owner)) {
            process.rw(&mut self.owner).exception = true;
        }
    }

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
            if let Some(condition) = &condition.parent {
                let parent_match = process.parent().map_or(false, |parent| {
                    let parent = parent.ro(&self.owner);
                    condition.matches(&parent.name) || condition.matches(&parent.forked_name)
                });

                if !parent_match {
                    continue;
                }
            }

            return true;
        }

        false
    }

    /// Reloads the configuration files.
    pub fn reload_configuration(&mut self) {
        self.config = crate::config::config();
    }

    /// Refreshes the process map
    pub fn refresh_process_map(&mut self, buffer: &mut Buffer) {
        self.process_map.drain_filter_prepare();

        let mut parents = BTreeMap::new();

        let mut path = String::from("/proc/");
        let proc_truncate = path.len();

        let Ok(procfs) = std::fs::read_dir(&path) else {
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

            let Some(file_name) = file_name.to_str() else {
                continue;
            };

            path.truncate(proc_truncate);

            strcat!(&mut path, file_name "/status");

            if let Some(name) = process::name(buffer, process.id) {
                process.name = name.to_owned();

                if let Some(cgroup) = process::cgroup(buffer, process.id) {
                    process.cgroup = cgroup.to_owned();
                }

                if let Some(value) = utils::file_key(&mut buffer.file_raw, &path, "PPid:") {
                    if let Some(ppid) = atoi::atoi::<u32>(value) {
                        parents.insert(process.id, ppid);
                        process.parent_id = ppid;
                    }
                }

                self.process_map.retain_process_tree(&self.owner, &process);

                self.insert_process(process);
            }
        }

        for (pid, ppid) in parents {
            if let Some(process) = self.process_map.get_pid(&self.owner, pid) {
                if let Some(parent) = self.process_map.get_pid(&self.owner, ppid) {
                    process.rw(&mut self.owner).parent = Some(Arc::downgrade(parent));
                }
            }
        }

        self.process_map.drain_filter();
    }

    /// Sets a process as the foreground.
    pub fn set_foreground_process(&mut self, buffer: &mut Buffer, pid: u32) {
        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            self.foreground = Some(pid);

            // Unset priorities of previously-set processes.
            let mut foreground = Vec::new();
            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            for process in foreground.drain(..) {
                if !self.pipewire_processes.contains(&process) {
                    if let Some(priority) = self
                        .assigned_priority(process)
                        .with_default(&assignments.background)
                    {
                        crate::priority::set(buffer, process, priority);
                    }
                }
            }

            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            if !self.pipewire_processes.contains(&pid) {
                if let Some(priority) = self
                    .assigned_priority(pid)
                    .with_default(&assignments.foreground)
                {
                    crate::priority::set(buffer, pid, priority);
                }
            }

            self.foreground_processes.push(pid);

            'outer: loop {
                for process in self.process_map.map.values() {
                    let process = process.ro(&self.owner);
                    if let Some(parent) = process.parent() {
                        let parent = parent.ro(&self.owner);
                        if self.foreground_processes.contains(&parent.id)
                            && !self.foreground_processes.contains(&process.id)
                        {
                            if let Some(priority) = self
                                .assigned_priority(process.id)
                                .with_default(&assignments.foreground)
                            {
                                if !self.pipewire_processes.contains(&process.id) {
                                    crate::priority::set(buffer, process.id, priority);
                                }
                                self.foreground_processes.push(process.id);
                                continue 'outer;
                            }
                        }
                    }
                }

                break;
            }
        }
    }

    /// Assigns a process to the pipewire profile if it does not already have an assignment.
    pub fn set_pipewire_process(
        &mut self,
        buffer: &mut Buffer,
        process: system76_scheduler_pipewire::Process,
    ) {
        if let Some(pipewire) = self.config.process_scheduler.pipewire.clone() {
            if !self.pipewire_processes.contains(&process.id) {
                self.pipewire_processes.push(process.id);

                if let Priority::Assignable = self.assigned_priority(process.id) {
                    crate::priority::set(buffer, process.id, &pipewire);
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
            if let Priority::Assignable = self.assigned_priority(process_id) {
                let profile = if self.foreground_processes.contains(&process_id) {
                    &assignments.foreground
                } else {
                    &assignments.background
                };

                crate::priority::set(buffer, process_id, profile);
            }
        }
    }

    /// Updates the list of assignable processes, and reassigns priorities.
    pub fn assign_process_map_priorities(&mut self, buffer: &mut Buffer) {
        for process in self.process_map.map.values() {
            let process = process.ro(&self.owner);

            if self.pipewire_processes.contains(&process.id)
                || self.foreground_processes.contains(&process.id)
            {
                continue;
            }

            match self.assigned_priority(process.id) {
                // If enabled, assign background priority to all processes
                Priority::Assignable => {
                    if let Some(ref assignments) = self.config.process_scheduler.foreground {
                        crate::priority::set(buffer, process.id, &assignments.background);
                    }
                }

                Priority::Config(profile) => {
                    crate::priority::set(buffer, process.id, profile);
                }

                _ => (),
            }
        }

        // Reassign foreground processes in case they were overriden.
        if let Some(process) = self.foreground.take() {
            self.set_foreground_process(buffer, process);
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

pub enum AssignmentStatus {
    Assigned,
    Unset,
}
