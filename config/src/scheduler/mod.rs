// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

mod assignments;
pub use assignments::{Assignments, Condition, MatchCondition};

mod profile;
pub use profile::Profile;

use std::{borrow::Cow, str::FromStr};

/// Process scheduling configuration
pub struct Config {
    /// Enables process scheduling
    pub enable: bool,
    /// Enables execsnoop
    pub execsnoop: bool,
    /// Defines the refresh rate for polling processes
    pub refresh_rate: u16,
    /// Process profile assignments
    pub assignments: Assignments,
    /// Foreground profiles
    pub foreground: Option<ForegroundAssignments>,
    /// Pipewire profile
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

/// Foreground process profiles
pub struct ForegroundAssignments {
    /// Background profile
    pub background: Profile,
    /// Foreground profile
    pub foreground: Profile,
}

/// I/O Class
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoClass {
    /// Idle
    Idle,
    /// BestEffort
    #[default]
    BestEffort,
    /// Realtime
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

/// I/O policy
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPolicy {
    /// Idle
    Idle,
    /// Standard
    Standard,
    /// BestEffort
    BestEffort(IoPriority),
    /// Realtime
    Realtime(IoPriority),
}

/// I/O priority
///
/// Restricts the value between 0 through 7.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct IoPriority(u8);

impl IoPriority {
    /// Value as a number
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
    /// Value as a number
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

/// Process assignment
pub enum Process<'a> {
    /// Assign by cmdline
    CmdLine(Cow<'a, str>),
    /// Assign by name
    Name(Cow<'a, str>),
}

/// Scheduler configuration
pub struct Scheduler {
    /// Scheduler policy
    pub policy: SchedPolicy,
    /// Scheduler priority
    pub priority: SchedPriority,
}

/// Scheduler policy
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[repr(i32)]
pub enum SchedPolicy {
    /// SCHED_BATCH
    Batch = libc::SCHED_BATCH,
    /// SCHED_FIFO
    Fifo = libc::SCHED_FIFO,
    /// SCHED_IDLE
    Idle = libc::SCHED_IDLE,
    /// SCHED_OTHER
    #[default]
    Other = libc::SCHED_OTHER,
    /// SCHED_RR
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

impl SchedPolicy {
    /// Whether the policy is realtime (FIFO or RR)
    #[must_use]
    pub fn is_realtime(self) -> bool {
        matches!(self, Self::Fifo | Self::Rr)
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
    /// Value as a number
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
