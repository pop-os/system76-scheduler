// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::path::Path;

pub const BANDWIDTH_SIZE_PATH: &str = "/proc/sys/kernel/sched_cfs_bandwidth_slice_us";
pub const PREEMPT_PATH: &str = "/sys/kernel/debug/sched/preempt";

#[derive(Debug, thiserror::Error)]
pub enum SchedPathsError {
    #[error("kernel does not support tweaking the scheduler")]
    NotSupported,
}

pub struct SchedPaths {
    pub latency: &'static str,
    pub min_gran: &'static str,
    pub wakeup_gran: &'static str,
    pub migration_cost: &'static str,
    pub preempt: Option<&'static str>,
}

impl SchedPaths {
    pub fn new() -> Result<Self, SchedPathsError> {
        let mut paths = Self {
            latency: "/sys/kernel/debug/sched/latency_ns",
            min_gran: "/sys/kernel/debug/sched/min_granularity_ns",
            wakeup_gran: "/sys/kernel/debug/sched/wakeup_granularity_ns",
            migration_cost: "/sys/kernel/debug/sched/migration_cost_ns",
            preempt: None,
        };

        if !Path::new(paths.latency).exists() {
            paths.latency = "/proc/sys/kernel/sched_latency_ns";

            if !Path::new(paths.latency).exists() {
                return Err(SchedPathsError::NotSupported);
            }

            paths.min_gran = "/proc/sys/kernel/sched_min_granularity_ns";
            paths.wakeup_gran = "/proc/sys/kernel/sched_wakeup_granularity_ns";
            paths.migration_cost = "/proc/sys/kernel/sched_migration_cost_ns";
        }

        if Path::new(PREEMPT_PATH).exists() {
            paths.preempt = Some(PREEMPT_PATH);
        }

        Ok(paths)
    }
}
