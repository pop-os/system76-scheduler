// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::config::{Assignment, Assignments, Config, Exceptions, IoPriority};
use std::collections::HashMap;

type ProcessMap = HashMap<u32, Option<u32>>;

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Priority {
    Assignable,
    Config((i32, IoPriority)),
    NotAssignable,
}

impl Priority {
    pub fn with_default(self, priority: (i32, IoPriority)) -> Option<(i32, ioprio::Priority)> {
        let (cpu, io) = match self {
            Priority::Assignable => priority,
            Priority::Config(config) => config,
            Priority::NotAssignable => return None,
        };

        Some((cpu, io.into()))
    }
}

pub enum AssignmentStatus {
    Assigned,
    Unset,
}

#[derive(Default)]
pub struct Service {
    pub assignments: Assignments,
    pub config: Config,
    pub exceptions: Exceptions,
    pub foreground: Option<u32>,
    pub foreground_processes: Vec<u32>,
    pub process_map: ProcessMap,
}

impl Service {
    /// Assign a priority to a process
    pub fn assign(&self, pid: u32, default: (i32, IoPriority)) -> AssignmentStatus {
        if let Some(priority) = self.assigned_priority(pid).with_default(default) {
            crate::nice::set_priority(pid, priority);
            return AssignmentStatus::Assigned;
        }

        AssignmentStatus::Unset
    }

    /// Gets the config-assigned priority of a process.
    #[must_use]
    pub fn assigned_priority(&self, pid: u32) -> Priority {
        if !is_assignable(pid) {
            return Priority::NotAssignable;
        }

        let name = crate::utils::name_of_pid(pid);

        if let Some(name) = &name {
            if self.exceptions.contains(name) {
                return Priority::NotAssignable;
            }
        }

        if let Some(exe) = crate::utils::exe_of_pid(pid) {
            if self.exceptions.contains(&*exe) {
                return Priority::NotAssignable;
            }

            if let Some(Assignment(cpu, io)) = self.assignments.get(&*exe) {
                return Priority::Config((cpu.get().into(), *io));
            }

            if let Some(name) = name {
                if let Some(Assignment(cpu, io)) = self.assignments.get(&*name) {
                    return Priority::Config((cpu.get().into(), *io));
                }
            }

            return Priority::Assignable;
        }

        Priority::NotAssignable
    }

    /// Assign a priority to a newly-created process.
    pub fn assign_new_process(&mut self, pid: u32, parent_pid: u32) {
        if let Some(foreground) = self.config.foreground {
            if self.foreground_processes.contains(&parent_pid)
                && !self.foreground_processes.contains(&pid)
            {
                let default = (i32::from(foreground), IoPriority::Standard);

                if let AssignmentStatus::Assigned = self.assign(pid, default) {
                    self.foreground_processes.push(pid);
                    return;
                }
            }
        }

        if let Some(background) = self.config.background {
            if self.foreground_processes.contains(&pid)
                || Priority::NotAssignable == self.assigned_priority(pid)
            {
                return;
            }

            let default = (i32::from(background), IoPriority::Idle);
            let _status = self.assign(pid, default);
        }
    }

    /// Reloads the configuration files.
    pub fn reload_configuration(&mut self) {
        self.config = Config::read();
        self.exceptions = crate::config::exceptions();
        self.assignments = crate::config::assignments(&self.exceptions);
    }

    /// Sets a process as the foreground.
    pub fn set_foreground_process(&mut self, pid: u32) {
        if let Some(foreground_priority) = self.config.foreground {
            self.foreground = Some(pid);

            let background_priority = (
                i32::from(self.config.background.unwrap_or(0)),
                IoPriority::Idle,
            );

            let foreground_priority = (i32::from(foreground_priority), IoPriority::Standard);

            // Unset priorities of previously-set processes.
            let mut foreground = Vec::new();
            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            for process in foreground.drain(..) {
                if let Some(priority) = self
                    .assigned_priority(process)
                    .with_default(background_priority)
                {
                    crate::nice::set_priority(process, priority);
                }
            }

            std::mem::swap(&mut foreground, &mut self.foreground_processes);

            if let Some(priority) = self
                .assigned_priority(pid)
                .with_default(foreground_priority)
            {
                crate::nice::set_priority(pid, priority);
                self.foreground_processes.push(pid);
            }

            'outer: loop {
                for (pid, parent) in &self.process_map {
                    if let Some(parent) = parent {
                        if self.foreground_processes.contains(parent)
                            && !self.foreground_processes.contains(pid)
                        {
                            if let Some(priority) = self
                                .assigned_priority(*pid)
                                .with_default(foreground_priority)
                            {
                                crate::nice::set_priority(*pid, priority);
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

    /// Updates the list of assignable processes, and reassigns priorities.
    pub fn update_process_map(&mut self, map: ProcessMap, background_processes: Vec<u32>) {
        self.process_map = map;

        // Assign background priority to all processes.
        if let Some(background) = self.config.background {
            for pid in background_processes {
                if self.foreground_processes.contains(&pid) {
                    continue;
                }

                if let Some(priority) = self
                    .assigned_priority(pid)
                    .with_default((i32::from(background), IoPriority::Idle))
                {
                    crate::nice::set_priority(pid, priority);
                }
            }
        }

        // Reassign foreground processes in case they were overriden.
        if let Some(process) = self.foreground.take() {
            self.set_foreground_process(process);
        }
    }
}

/// A process is assignable if its priority is less than 9 or greater than -9.
pub fn is_assignable(pid: u32) -> bool {
    let current = priority(pid);
    current <= 9 && current >= -9
}

/// Get the priority of a process.
pub fn priority(pid: u32) -> i32 {
    unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) }
}
