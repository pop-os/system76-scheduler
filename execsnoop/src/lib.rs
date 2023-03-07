// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use atoi::atoi;
use bstr::{BStr, ByteSlice};
use bytelines::ByteLines;
use std::io::{self, BufReader};
use std::process::{Command, Stdio};

#[derive(Clone, Debug)]
pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
}

pub struct ProcessIterator<I> {
    child: std::process::Child,
    iterator: I,
}

impl<I: Iterator<Item = Process> + Send> Iterator for ProcessIterator<I> {
    type Item = Process;

    fn next(&mut self) -> Option<Process> {
        self.iterator.next()
    }
}

impl<I> Drop for ProcessIterator<I> {
    fn drop(&mut self) {
        let _res = self.child.kill();
        let _res = self.child.wait();
    }
}

/// Watches process creation and destruction on Linux.
///
/// # Errors
///
/// Requires the `execsnoop-bpfcc` binary from `bpfcc-tools`
pub fn watch() -> io::Result<ProcessIterator<impl Iterator<Item = Process> + Send>> {
    Command::new(std::env!(
        "EXECSNOOP_PATH",
        "must set EXECSNOOP_PATH env to execsnoop-bpfcc path"
    ))
    .env("LC_ALL", "C")
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .stdin(Stdio::null())
    .spawn()
    .and_then(|mut child| {
        let stdout = child.stdout.take().ok_or_else(|| {
            let _res = child.kill();
            let _res = child.wait();
            io::Error::new(io::ErrorKind::Other, "execsnoop-bpfcc lacks stdout pipe")
        })?;

        let mut reader = ByteLines::new(BufReader::with_capacity(16 * 1024, stdout));

        Ok(ProcessIterator {
            child,
            iterator: std::iter::from_fn(move || {
                while let Some(Ok(line)) = reader.next() {
                    let mut fields = BStr::new(line).fields();

                    let pid = fields.nth(1);
                    let parent_pid = fields.next();

                    if let Some((pid, parent_pid)) = pid.zip(parent_pid) {
                        let pid = atoi::<u32>(pid);
                        let parent_pid = atoi::<u32>(parent_pid);

                        if let Some((pid, parent_pid)) = pid.zip(parent_pid) {
                            return Some(Process { pid, parent_pid });
                        }
                    }
                }

                None
            }),
        })
    })
}
