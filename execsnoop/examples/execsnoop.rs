// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

fn main() {
    let watcher = execsnoop::watch().unwrap();

    for process in watcher {
        println!("{:?}", process);
    }
}
