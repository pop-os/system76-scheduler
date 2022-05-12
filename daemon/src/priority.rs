// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::config::IoPriority;

#[derive(Copy, Clone, Debug)]
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

pub fn is_assignable(pid: u32) -> bool {
    let current = priority(pid);
    current <= 9 && current >= -9
}

pub fn priority(pid: u32) -> i32 {
    unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) }
}
