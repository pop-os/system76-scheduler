// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::{
    io,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
};

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
}

/// Watches process creation and destruction on Linux.
///
/// # Errors
///
/// Requires the `execsnoop-bpfcc` binary from `bpfcc-tools`
pub fn watch() -> io::Result<impl Iterator<Item = Process>> {
    Command::new(std::env!(
        "EXECSNOOP_PATH",
        "must set EXECSNOOP_PATH env to execsnoop-bpfcc path"
    ))
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .stdin(Stdio::null())
    .spawn()
    .and_then(|mut child| {
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "execsnoop-bpfcc lacks stdout pipe")
        })?;

        let mut reader = BufReader::new(stdout);

        let mut line = String::with_capacity(128);

        Ok(std::iter::from_fn(move || loop {
            match reader.read_line(&mut line) {
                Err(_) | Ok(0) => return None,
                _ => (),
            }

            let mut fields = line.split_ascii_whitespace();

            let pid = fields.nth(1);
            let parent_pid = fields.next();

            if let Some((pid, parent_pid)) = pid.zip(parent_pid) {
                let pid = pid.parse::<u32>().ok();
                let parent_pid = parent_pid.parse::<u32>().ok();

                if let Some((pid, parent_pid)) = pid.zip(parent_pid) {
                    let process = Process { pid, parent_pid };

                    line.clear();
                    return Some(process);
                }
            }

            line.clear();
        }))
    })
}
