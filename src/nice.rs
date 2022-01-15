// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

pub fn set_priority(process: u32, priority: i32) {
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, process, priority);
    }
}
