// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::os::unix::prelude::OsStrExt;

use concat_in_place::strcat;
use ioprio::{Pid, Target};
use system76_scheduler_config::scheduler::{Profile, SchedPolicy, SchedPriority};

use crate::utils::Buffer;

/// Get the priority of a process.
pub fn get(pid: u32) -> i32 {
    unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) }
}

pub fn set(buffer: &mut Buffer, process: u32, profile: &Profile) {
    buffer.path.clear();
    let tasks = strcat!(&mut buffer.path, "/proc/" buffer.itoa.format(process) "/task");

    let Ok(tasks) = std::fs::read_dir(tasks) else {
        return;
    };

    for task in tasks.filter_map(Result::ok) {
        let Some(process) = atoi::atoi::<u32>(task.file_name().as_bytes()) else {
            return;
        };

        unsafe {
            libc::setpriority(
                libc::PRIO_PROCESS,
                process,
                libc::c_int::from(profile.nice.get()),
            );
        }

        set_policy(process, profile.sched_policy, profile.sched_priority);

        #[allow(clippy::cast_possible_wrap)]
        if let Err(why) = ioprio::set_priority(
            Target::Process(Pid::from_raw(process as i32)),
            ioprio::Priority::new(profile.io),
        ) {
            tracing::error!("failed to set ioprio: {:?}", why);
        }
    }
}

pub fn set_policy(pid: u32, policy: SchedPolicy, sched_priority: SchedPriority) {
    let param = libc::sched_param {
        sched_priority: libc::c_int::from(sched_priority.get()),
    };

    unsafe {
        #[allow(clippy::cast_possible_wrap)]
        libc::sched_setscheduler(pid as libc::c_int, policy as libc::c_int, &param);
    }
}
