// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use compact_str::CompactStr;

pub fn exe_of_pid(pid: u32) -> Option<CompactStr> {
    let mut itoa = itoa::Buffer::new();
    let exe = concat_in_place::strcat!("/proc/" itoa.format(pid) "/exe");

    if let Ok(exe) = std::fs::read_link(Path::new(&exe)) {
        if let Some(exe) = exe.file_name().and_then(std::ffi::OsStr::to_str) {
            if let Some(exe) = exe.split_ascii_whitespace().next() {
                return Some(CompactStr::from(exe));
            }
        }
    }

    None
}

pub fn name_of_pid(pid: u32) -> Option<CompactStr> {
    let mut itoa = itoa::Buffer::new();
    let path = concat_in_place::strcat!("/proc/" itoa.format(pid) "/status");

    if let Ok(buffer) = std::fs::read_to_string(&path) {
        if let Some(name) = buffer.lines().next() {
            if let Some(name) = name.strip_prefix("Name:") {
                return Some(CompactStr::from(name.trim()));
            }
        }
    }

    None
}

pub fn read_into_string<P: AsRef<OsStr>>(buf: &mut String, path: P) -> io::Result<&str> {
    let mut file = File::open(path.as_ref())?;
    buf.clear();
    file.read_to_string(buf)?;
    Ok(&*buf)
}
