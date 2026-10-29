// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

fn main() {
    let mut watcher = execsnoop::watch().unwrap();

    while let Some(process) = watcher.next() {
        println!(
            "{} {} {} {}",
            process.pid,
            process.parent_pid,
            String::from_utf8_lossy(process.name),
            String::from_utf8_lossy(process.cmd)
        );
    }
}
