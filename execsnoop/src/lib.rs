// Copyright 2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#![deny(missing_docs)]

//! Watches process executions with the kernel's process events connector.
//!
//! Needs a kernel built with `CONFIG_PROC_EVENTS`, and `CAP_NET_ADMIN` on kernels older than 6.6.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Read};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;

// linux/netlink.h
const NLMSG_DONE: u16 = 3;

// linux/connector.h
const CN_IDX_PROC: u32 = 1;
const CN_VAL_PROC: u32 = 1;

// linux/cn_proc.h
const PROC_CN_MCAST_LISTEN: u32 = 1;
const PROC_CN_MCAST_IGNORE: u32 = 2;
const PROC_EVENT_EXEC: u32 = 2;

/// Size of `struct nlmsghdr`.
const NLMSG_HDRLEN: usize = 16;
/// Size of `struct cn_msg`, without its data.
const CN_MSG_LEN: usize = 20;
/// Offset of `event_data.exec.process_tgid` in `struct proc_event`.
const EXEC_TGID: usize = 20;

/// Process info
#[derive(Clone, Debug)]
pub struct Process<'a> {
    /// Process name, as in `/proc/<pid>/comm`
    pub name: &'a [u8],
    /// Path of the program that was executed: the script for a script, else the executable
    pub cmd: &'a [u8],
    /// Process PID
    pub pid: u32,
    /// Process parent PID
    pub parent_pid: u32,
}

/// Process iterator
pub struct ProcessIterator {
    socket: OwnedFd,
    buffer: Vec<u8>,
    path: String,
    stat: Vec<u8>,
    name_buffer: Vec<u8>,
    cmd_buffer: Vec<u8>,
}

impl ProcessIterator {
    /// Waits for the next process execution.
    ///
    /// Returns `None` if the socket fails. Processes that exit before they can be
    /// read from `/proc` are skipped.
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Process<'_>> {
        loop {
            // SAFETY: the buffer is valid for writes of its whole length.
            let read = unsafe {
                libc::recv(
                    self.socket.as_raw_fd(),
                    self.buffer.as_mut_ptr().cast(),
                    self.buffer.len(),
                    0,
                )
            };

            let Ok(read) = usize::try_from(read) else {
                let error = io::Error::last_os_error();
                match error.raw_os_error() {
                    Some(libc::EINTR) => continue,
                    // The socket buffer overflowed during a burst of events.
                    Some(libc::ENOBUFS) => {
                        tracing::warn!("some process events were lost");
                        continue;
                    }
                    _ => {
                        tracing::error!("failed to read process events: {error}");
                        return None;
                    }
                }
            };

            let Some(pid) = exec_event(&self.buffer[..read]) else {
                continue;
            };

            if let Some(parent_pid) = self.read_process(pid) {
                return Some(Process {
                    name: &self.name_buffer,
                    cmd: &self.cmd_buffer,
                    pid,
                    parent_pid,
                });
            }
        }
    }

    /// Reads the name, executable path and parent of a process, returning its parent PID.
    fn read_process(&mut self, pid: u32) -> Option<u32> {
        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/stat");

        self.stat.clear();
        File::open(&self.path)
            .and_then(|mut file| file.read_to_end(&mut self.stat))
            .ok()?;

        let (name, parent_pid) = parse_stat(&self.stat)?;
        self.name_buffer.clear();
        self.name_buffer.extend_from_slice(name);

        self.path.clear();
        let _ = write!(self.path, "/proc/{pid}/exe");

        let exe = std::fs::read_link(&self.path).ok()?;
        self.cmd_buffer.clear();
        self.cmd_buffer.extend_from_slice(exe.as_os_str().as_bytes());

        // A script runs as its interpreter, but the kernel names the process after the script,
        // like the path given to execve that execsnoop reported: find that path in the arguments.
        if !file_name(&self.cmd_buffer).starts_with(&self.name_buffer) {
            self.path.clear();
            let _ = write!(self.path, "/proc/{pid}/cmdline");

            self.stat.clear();
            if File::open(&self.path)
                .and_then(|mut file| file.read_to_end(&mut self.stat))
                .is_ok()
            {
                if let Some(path) = executed(&self.stat, &self.name_buffer) {
                    self.cmd_buffer.clear();
                    self.cmd_buffer.extend_from_slice(path);
                }
            }
        }

        Some(parent_pid)
    }
}

impl Drop for ProcessIterator {
    fn drop(&mut self) {
        let _res = control(&self.socket, PROC_CN_MCAST_IGNORE, None);
    }
}

/// Watches process executions on Linux.
///
/// # Errors
///
/// Fails if the kernel lacks the process events connector.
pub fn watch() -> io::Result<ProcessIterator> {
    // SAFETY: plain socket(2) call.
    let fd = unsafe {
        libc::socket(
            libc::AF_NETLINK,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            libc::NETLINK_CONNECTOR,
        )
    };

    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `fd` was just created and nothing else owns it.
    let socket = unsafe { OwnedFd::from_raw_fd(fd) };

    // SAFETY: `sockaddr_nl` is plain data, for which all zeroes is valid.
    let mut address: libc::sockaddr_nl = unsafe { mem::zeroed() };
    #[allow(clippy::cast_possible_truncation)]
    {
        address.nl_family = libc::AF_NETLINK as libc::sa_family_t;
    }
    address.nl_groups = CN_IDX_PROC;

    // SAFETY: `address` is a valid `sockaddr_nl` of the given size.
    #[allow(clippy::cast_possible_truncation)]
    let bound = unsafe {
        libc::bind(
            socket.as_raw_fd(),
            std::ptr::addr_of!(address).cast(),
            mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
        )
    };

    if bound < 0 {
        return Err(io::Error::last_os_error());
    }

    // Listen to every event: the only request that kernels older than 6.6 understand.
    control(&socket, PROC_CN_MCAST_LISTEN, None)?;

    // Since Linux 6.6, only exec events are needed. Older kernels ignore this request.
    control(&socket, PROC_CN_MCAST_LISTEN, Some(PROC_EVENT_EXEC))?;

    tracing::debug!("listening to process events");

    Ok(ProcessIterator {
        socket,
        buffer: vec![0; 4096],
        path: String::with_capacity(32),
        stat: Vec::with_capacity(512),
        name_buffer: Vec::with_capacity(16),
        cmd_buffer: Vec::with_capacity(128),
    })
}

/// Sends a multicast request to the process events connector: a `cn_msg` holding an
/// `enum proc_cn_mcast_op`, or a `struct proc_input` when an event type is given.
#[allow(clippy::cast_possible_truncation)] // a message of at most 44 bytes
fn control(socket: &OwnedFd, op: u32, event: Option<u32>) -> io::Result<()> {
    let data_len = if event.is_some() { 8 } else { 4 };
    let len = NLMSG_HDRLEN + CN_MSG_LEN + data_len;
    let mut message = [0u8; NLMSG_HDRLEN + CN_MSG_LEN + 8];

    // struct nlmsghdr
    message[0..4].copy_from_slice(&(len as u32).to_ne_bytes());
    message[4..6].copy_from_slice(&NLMSG_DONE.to_ne_bytes());

    // struct cn_msg
    let cn = NLMSG_HDRLEN;
    message[cn..cn + 4].copy_from_slice(&CN_IDX_PROC.to_ne_bytes());
    message[cn + 4..cn + 8].copy_from_slice(&CN_VAL_PROC.to_ne_bytes());
    message[cn + 16..cn + 18].copy_from_slice(&(data_len as u16).to_ne_bytes());

    let data = cn + CN_MSG_LEN;
    message[data..data + 4].copy_from_slice(&op.to_ne_bytes());
    if let Some(event) = event {
        message[data + 4..data + 8].copy_from_slice(&event.to_ne_bytes());
    }

    // SAFETY: the message is valid for reads of `len` bytes.
    let sent = unsafe { libc::send(socket.as_raw_fd(), message.as_ptr().cast(), len, 0) };

    if sent < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// Returns the PID of the process from an exec event. The kernel sends each event in its
/// own datagram: a `nlmsghdr`, followed by a `cn_msg`, followed by a `proc_event`.
fn exec_event(datagram: &[u8]) -> Option<u32> {
    let u32_at = |offset: usize| {
        let bytes = datagram.get(offset..offset + 4)?;
        Some(u32::from_ne_bytes(bytes.try_into().ok()?))
    };

    let kind = datagram.get(4..6)?;
    if u16::from_ne_bytes([kind[0], kind[1]]) != NLMSG_DONE {
        return None;
    }

    let cn = NLMSG_HDRLEN;
    if u32_at(cn)? != CN_IDX_PROC || u32_at(cn + 4)? != CN_VAL_PROC {
        return None;
    }

    // The event starts at offset 36, so its 64-bit timestamp is not aligned: read fields by offset.
    let event = cn + CN_MSG_LEN;
    if u32_at(event)? != PROC_EVENT_EXEC {
        return None;
    }

    u32_at(event + EXEC_TGID)
}

fn file_name(path: &[u8]) -> &[u8] {
    path.rsplit(|&byte| byte == b'/').next().unwrap_or(path)
}

/// Finds, in the arguments of a process, the path whose file name matches the name of the
/// process, which the kernel cuts to 15 bytes.
fn executed<'a>(cmdline: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    cmdline
        .split(|&byte| byte == 0)
        .find(|arg| !arg.is_empty() && file_name(arg).starts_with(name))
}

/// Gets the name and parent PID from `/proc/<pid>/stat`: `<pid> (<name>) <state> <ppid> ...`.
/// The name may contain spaces and parentheses, so it ends at the last parenthesis.
fn parse_stat(stat: &[u8]) -> Option<(&[u8], u32)> {
    let open = stat.iter().position(|&byte| byte == b'(')?;
    let close = stat.iter().rposition(|&byte| byte == b')')?;
    let name = stat.get(open + 1..close)?;

    let mut fields = stat.get(close + 2..)?.split(|&byte| byte == b' ');
    let _state = fields.next()?;
    let parent_pid = atoi::atoi::<u32>(fields.next()?)?;

    Some((name, parent_pid))
}

#[cfg(test)]
#[allow(clippy::cast_possible_truncation)]
mod tests {
    use super::*;

    fn datagram(what: u32, tgid: u32) -> Vec<u8> {
        let mut datagram = vec![0u8; NLMSG_HDRLEN + CN_MSG_LEN + 40];
        let len = datagram.len() as u32;
        datagram[0..4].copy_from_slice(&len.to_ne_bytes());
        datagram[4..6].copy_from_slice(&NLMSG_DONE.to_ne_bytes());
        datagram[16..20].copy_from_slice(&CN_IDX_PROC.to_ne_bytes());
        datagram[20..24].copy_from_slice(&CN_VAL_PROC.to_ne_bytes());
        datagram[32..34].copy_from_slice(&40u16.to_ne_bytes());
        datagram[36..40].copy_from_slice(&what.to_ne_bytes());
        datagram[52..56].copy_from_slice(&tgid.to_ne_bytes());
        datagram[56..60].copy_from_slice(&tgid.to_ne_bytes());
        datagram
    }

    #[test]
    fn exec_events() {
        assert_eq!(exec_event(&datagram(PROC_EVENT_EXEC, 4242)), Some(4242));
        // fork events, and the acknowledgement of a request
        assert_eq!(exec_event(&datagram(1, 4242)), None);
        assert_eq!(exec_event(&datagram(0, 0)), None);
        assert_eq!(exec_event(&datagram(PROC_EVENT_EXEC, 4242)[..50]), None);
    }

    #[test]
    fn executed_paths() {
        let script = b"/usr/bin/python3\0/usr/games/lutris\0--verbose\0";
        assert_eq!(executed(script, b"lutris"), Some(&b"/usr/games/lutris"[..]));

        // the name of the process is cut to 15 bytes
        let long = b"/usr/bin/python3\0/opt/very-long-launcher-name\0";
        assert_eq!(
            executed(long, b"very-long-launc"),
            Some(&b"/opt/very-long-launcher-name"[..])
        );

        // a process that renamed itself
        let renamed = b"/usr/lib/firefox/firefox\0-contentproc\0";
        assert_eq!(executed(renamed, b"Isolated Web Co"), None);
    }

    #[test]
    fn stat_names() {
        let stat = b"4242 (Web Content) S 1984 4242 4242 0 -1 4194560";
        assert_eq!(parse_stat(stat), Some((&b"Web Content"[..], 1984)));

        let stat = b"77 (a) (b)) R 2 0 0";
        assert_eq!(parse_stat(stat), Some((&b"a) (b)"[..], 2)));

        assert_eq!(parse_stat(b"77 (truncated"), None);
    }

    #[test]
    #[ignore = "needs CAP_NET_ADMIN on kernels older than 6.6: cargo test -- --ignored"]
    fn watches_executions() {
        let mut watcher = watch().unwrap();

        // Fail instead of waiting forever if no event comes.
        let timeout = libc::timeval {
            tv_sec: 5,
            tv_usec: 0,
        };
        // SAFETY: `timeout` is a valid `timeval` of the given size.
        unsafe {
            libc::setsockopt(
                watcher.socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                std::ptr::addr_of!(timeout).cast(),
                mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }

        let mut child = std::process::Command::new("sleep").arg("1").spawn().unwrap();

        let found = std::iter::from_fn(|| watcher.next().map(|process| process.pid))
            .take(1000)
            .any(|pid| pid == child.id());

        child.wait().unwrap();
        assert!(found);
    }
}
