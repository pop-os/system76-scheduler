// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

pub mod assignments;
pub use assignments::{Assignments, Condition, MatchCondition};

mod profile;
pub use profile::Profile;

use std::{borrow::Cow, str::FromStr};

pub struct Config {
    pub enable: bool,
    pub execsnoop: bool,
    pub refresh_rate: u16,
    pub assignments: Assignments,
    pub foreground: Option<ForegroundAssignments>,
    pub pipewire: Option<Profile>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enable: false,
            execsnoop: false,
            refresh_rate: 60,
            assignments: Assignments::default(),
            foreground: None,
            pipewire: None,
        }
    }
}

pub struct ForegroundAssignments {
    pub background: Profile,
    pub foreground: Profile,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoClass {
    Idle,
    #[default]
    BestEffort,
    Realtime,
}

impl FromStr for IoClass {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let class = match s {
            "idle" => IoClass::Idle,
            "best-effort" => IoClass::BestEffort,
            "realtime" => IoClass::Realtime,
            _ => return Err(()),
        };

        Ok(class)
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPolicy {
    Idle,
    Standard,
    BestEffort(IoPriority),
    Realtime(IoPriority),
}

/// Restricts the value between 0 through 7.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct IoPriority(u8);

impl IoPriority {
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

impl Default for IoPriority {
    fn default() -> Self {
        Self(7)
    }
}

impl From<u8> for IoPriority {
    fn from(level: u8) -> Self {
        Self(level.min(7))
    }
}

/// Restricts the value between -20 through 19.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct Niceness(i8);

impl Niceness {
    #[must_use]
    pub fn get(self) -> i8 {
        self.0
    }
}

impl From<i8> for Niceness {
    fn from(level: i8) -> Self {
        Self(level.min(19).max(-20))
    }
}

pub enum Process<'a> {
    CmdLine(Cow<'a, str>),
    Name(Cow<'a, str>),
}

pub struct Scheduler {
    pub policy: SchedPolicy,
    pub priority: SchedPriority,
}

#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum SchedPolicy {
    Batch = libc::SCHED_BATCH,
    Fifo = libc::SCHED_FIFO,
    Idle = libc::SCHED_IDLE,
    #[default]
    Other = libc::SCHED_OTHER,
    Rr = libc::SCHED_RR,
}

impl FromStr for SchedPolicy {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let policy = match s {
            "batch" => Self::Batch,
            "fifo" => Self::Fifo,
            "idle" => Self::Idle,
            "other" => Self::Other,
            "rr" => Self::Rr,
            _ => return Err(()),
        };

        Ok(policy)
    }
}

/// A value between 1 and 99
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchedPriority(u8);

impl Default for SchedPriority {
    fn default() -> Self {
        Self(1)
    }
}

impl SchedPriority {
    #[must_use]
    pub fn get(self) -> u8 {
        self.0
    }
}

impl From<u8> for SchedPriority {
    fn from(level: u8) -> Self {
        Self(level.min(99).max(1))
    }
}
