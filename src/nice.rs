// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use concat_in_place::strcat;

pub fn set_priority(process: u32, priority: i32) {
    let mut buffer = itoa::Buffer::new();
    let tasks = strcat!("/proc/" buffer.format(process) "/task");

    if let Ok(tasks) = std::fs::read_dir(tasks) {
        for task in tasks.filter_map(Result::ok) {
            if let Some(process) = task
                .file_name()
                .to_str()
                .and_then(|num| num.parse::<u32>().ok())
            {
                tracing::debug!("set_priority {}: {}", process, priority);
                unsafe {
                    libc::setpriority(libc::PRIO_PROCESS, process, priority);
                }
            }
        }
    }
}
