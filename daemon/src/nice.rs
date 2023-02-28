// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use ioprio::{Pid, Target};
use procfs::process::Process;

pub fn set_priority(process: u32, priority: (i32, ioprio::Priority)) {
    // TODO: refactor to if let chain when stabilized
    if let Ok(pid) = i32::try_from(process) {
        if let Ok(process) = Process::new(pid) {
            if let Ok(tasks) = process.tasks() {
                for task in tasks.filter_map(Result::ok) {
                    if let Ok(tid_u32) = task.tid.try_into() {
                        tracing::debug!("set_priority {}: {:?}", task.tid, priority);
                        unsafe {
                            libc::setpriority(libc::PRIO_PROCESS, tid_u32, priority.0);
                        }
                        if let Err(why) = ioprio::set_priority(
                            Target::Process(Pid::from_raw(task.tid)),
                            priority.1,
                        ) {
                            tracing::error!("failed to set ioprio: {:?}", why);
                        }
                    }
                }
            }
        }
    }
}
