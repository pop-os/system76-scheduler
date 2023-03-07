// Copyright 2022 System76 <debug@system76.com>
// SPDX-License-Identifier: MPL-2.0

use system76_scheduler_config::scheduler::assignments::ProcessConditions;

use crate::cfs::paths::SchedPaths;
use crate::config::scheduler::Profile;
use crate::pid;
use crate::utils::Buffer;
use std::collections::HashMap;

type ProcessMap = HashMap<u32, Option<u32>>;

pub struct Service {
    pub config: crate::config::Config,
    pub cfs_paths: SchedPaths,
    pub foreground: Option<u32>,
    pub foreground_processes: Vec<u32>,
    pub pipewire_processes: Vec<u32>,
    pub process_map: ProcessMap,
}

impl Service {
    pub fn new(cfs_paths: SchedPaths) -> Self {
        Self {
            config: crate::config::Config::default(),
            cfs_paths,
            foreground: None,
            foreground_processes: Vec::with_capacity(128),
            pipewire_processes: Vec::with_capacity(4),
            process_map: ProcessMap::default(),
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
            tracing::debug!("auto config applying default config");
            self.cfs_default_config()
        } else {
            tracing::debug!("auto config applying responsive config");
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
        if let Some(profile) = self.assigned_priority(buffer, pid).with_default(default) {
            crate::priority::set(buffer, pid, profile);

            return AssignmentStatus::Assigned;
        }

        AssignmentStatus::Unset
    }

    /// Gets the config-assigned priority of a process.
    #[must_use]
    pub fn assigned_priority(&self, buffer: &mut Buffer, pid: u32) -> Priority {
        // Processes without a command line path are kernel threads
        let Some(ref cmdline) = pid::cmdline(buffer, pid) else {
            return Priority::NotAssignable;
        };

        // Return if listed as an exception by its cmdline path
        if self
            .config
            .process_scheduler
            .assignments
            .is_exception_by_cmdline(cmdline)
        {
            return Priority::Exception;
        }

        let name = pid::name(buffer, pid);

        // Return if listed as an exception by process name
        if let Some(name) = name {
            if self
                .config
                .process_scheduler
                .assignments
                .is_exception_by_name(name)
            {
                return Priority::Exception;
            }
        }

        if let Some(profile) = self
            .config
            .process_scheduler
            .assignments
            .get_by_cmdline(cmdline)
        {
            return Priority::Config(profile);
        }

        if let Some(name) = name {
            if let Some(profile) = self.config.process_scheduler.assignments.get_by_name(name) {
                return Priority::Config(profile);
            }
        }

        if let Some(cgroup) = pid::cgroup(buffer, pid) {
            let cgroup = cgroup.to_owned();
            if let Some(Some(parent_pid)) = self.process_map.get(&pid) {
                if let Some(parent) = pid::name(buffer, *parent_pid) {
                    if let Some(profile) =
                        self.config.process_scheduler.assignments.get_by_condition(
                            ProcessConditions {
                                cgroup: &cgroup,
                                parent,
                            },
                        )
                    {
                        return Priority::Config(profile);
                    }
                }
            }
        }

        Priority::Assignable
    }

    /// Assign a priority to a newly-created process.
    pub fn assign_new_process(&mut self, buffer: &mut Buffer, pid: u32, parent_pid: u32) {
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

        // Child processes inherit the priority of their parent.
        // We want exceptions to avoid inheriting that priority.
        if Priority::Exception == self.assigned_priority(buffer, pid) {
            let parent_priority = crate::priority::get(parent_pid);
            let child_priority = crate::priority::get(pid);

            if parent_priority == child_priority {
                crate::priority::set(buffer, pid, &Profile::default());
            }

            return;
        }

        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            if self.foreground_processes.contains(&pid) {
                return;
            }

            let _status = self.assign(buffer, pid, &assignments.background);
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

            // Unset priorities of previously-set processes.
            let mut foreground = Vec::new();
            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            for process in foreground.drain(..) {
                if !self.pipewire_processes.contains(&process) {
                    if let Some(priority) = self
                        .assigned_priority(buffer, process)
                        .with_default(&assignments.background)
                    {
                        crate::priority::set(buffer, process, priority);
                    }
                }
            }

            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            if !self.pipewire_processes.contains(&pid) {
                if let Some(priority) = self
                    .assigned_priority(buffer, pid)
                    .with_default(&assignments.foreground)
                {
                    crate::priority::set(buffer, pid, priority);
                }
            }

            self.foreground_processes.push(pid);

            'outer: loop {
                for (pid, parent) in &self.process_map {
                    if let Some(parent) = parent {
                        if self.foreground_processes.contains(parent)
                            && !self.foreground_processes.contains(pid)
                        {
                            if let Some(priority) = self
                                .assigned_priority(buffer, *pid)
                                .with_default(&assignments.foreground)
                            {
                                if !self.pipewire_processes.contains(pid) {
                                    crate::priority::set(buffer, *pid, priority);
                                }
                                self.foreground_processes.push(*pid);
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
                tracing::debug!("assigning {} to pipewire profile", process.id);
                self.pipewire_processes.push(process.id);

                if let Priority::Assignable = self.assigned_priority(buffer, process.id) {
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

        tracing::debug!("removing {} from pipewire profile", index);
        self.pipewire_processes.remove(index);

        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            if let Priority::Assignable = self.assigned_priority(buffer, process_id) {
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
    pub fn update_process_map(
        &mut self,
        buffer: &mut Buffer,
        map: ProcessMap,
        background_processes: Vec<u32>,
    ) {
        self.process_map = map;

        // If enabled, assign background priority to all processes
        if let Some(ref assignments) = self.config.process_scheduler.foreground {
            for pid in background_processes {
                if self.pipewire_processes.contains(&pid)
                    || self.foreground_processes.contains(&pid)
                {
                    continue;
                }

                if let Some(priority) = self
                    .assigned_priority(buffer, pid)
                    .with_default(&assignments.background)
                {
                    crate::priority::set(buffer, pid, priority);
                }
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
