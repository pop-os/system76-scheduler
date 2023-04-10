// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::sync::Arc;

use crate::scheduler::{Niceness, SchedPolicy, SchedPriority};

#[must_use]
#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Profile {
    pub name: Arc<str>,
    /// Niceness priority level
    pub nice: Option<Niceness>,
    /// I/O priority class
    pub io: ioprio::Class,
    /// Scheduler policy for a process
    pub sched_policy: SchedPolicy,
    /// Scheduler policy priority
    pub sched_priority: SchedPriority,
}

impl Profile {
    pub const fn new(name: Arc<str>) -> Self {
        Self {
            name,
            nice: None,
            io: ioprio::Class::BestEffort(ioprio::BePriorityLevel::lowest()),
            sched_policy: SchedPolicy::Other,
            sched_priority: SchedPriority(1),
        }
    }
}
