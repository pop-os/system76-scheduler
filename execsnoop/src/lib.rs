// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#![deny(missing_docs)]

//! Spawns the execsnoop-bpfcc application to watch process executions.

use atoi::atoi;
use bstr::{BStr, ByteSlice};
use bytelines::ByteLines;
use std::io::{self, BufReader};
use std::process::{Command, Stdio};

/// Process info
#[derive(Clone, Debug)]
pub struct Process<'a> {
    /// Process name
    pub name: &'a [u8],
    /// Process cmdline
    pub cmd: &'a [u8],
    /// Process PID
    pub pid: u32,
    /// Process parent PID
    pub parent_pid: u32,
}

/// Process iterator
pub struct ProcessIterator {
    child: std::process::Child,
    stream: ByteLines<BufReader<std::process::ChildStdout>>,
    name_buffer: Vec<u8>,
    cmd_buffer: Vec<u8>,
}

impl ProcessIterator {
    /// Get the next process from the iterator
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Process> {
        while let Some(Ok(line)) = self.stream.next() {
            let mut fields = BStr::new(line).fields();

            if let (Some(name), Some(pid), Some(parent_pid)) =
                (fields.next(), fields.next(), fields.next())
            {
                let cmd = fields.nth(1).unwrap_or_default();

                if let (Some(pid), Some(parent_pid)) = (atoi::<u32>(pid), atoi::<u32>(parent_pid)) {
                    self.name_buffer.clear();
                    self.name_buffer.extend_from_slice(name);

                    self.cmd_buffer.clear();
                    self.cmd_buffer.extend_from_slice(cmd);

                    return Some(Process {
                        name: &self.name_buffer,
                        cmd: &self.cmd_buffer,
                        pid,
                        parent_pid,
                    });
                }
            }
        }

        None
    }
}

impl Drop for ProcessIterator {
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
pub fn watch() -> io::Result<ProcessIterator> {
    Command::new(std::env!(
        "EXECSNOOP_PATH",
        "must set EXECSNOOP_PATH env to execsnoop-bpfcc path"
    ))
    .env("LC_ALL", "C")
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .stdin(Stdio::null())
    .spawn()
    .and_then(move |mut child| {
        let stdout = child.stdout.take().ok_or_else(|| {
            let _res = child.kill();
            let _res = child.wait();
            io::Error::new(io::ErrorKind::Other, "execsnoop-bpfcc lacks stdout pipe")
        })?;

        let stream = ByteLines::new(BufReader::with_capacity(16 * 1024, stdout));

        Ok(ProcessIterator {
            child,
            stream,
            name_buffer: Vec::with_capacity(64),
            cmd_buffer: Vec::with_capacity(128),
        })
    })
}
