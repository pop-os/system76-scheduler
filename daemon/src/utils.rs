// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};

use bstr::{BStr, ByteSlice};

pub struct Buffer {
    pub path: String,
    pub file: String,
    pub file_raw: Vec<u8>,
    pub itoa: itoa::Buffer,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            path: String::with_capacity(256),
            file: String::with_capacity(4096),
            file_raw: Vec::with_capacity(4096),
            itoa: itoa::Buffer::new(),
        }
    }
}

pub fn read_into_string<P: AsRef<OsStr>>(buf: &mut String, path: P) -> io::Result<&str> {
    let mut file = File::open(path.as_ref())?;
    buf.clear();
    file.read_to_string(buf)?;
    Ok(&*buf)
}

pub fn read_into_vec<P: AsRef<OsStr>>(buf: &mut Vec<u8>, path: P) -> io::Result<&[u8]> {
    let mut file = File::open(path.as_ref())?;
    buf.clear();
    file.read_to_end(buf)?;
    Ok(&*buf)
}

pub fn file_key<'a>(buf: &'a mut Vec<u8>, path: &str, key: &str) -> Option<&'a [u8]> {
    buf.clear();

    let Ok(mut status) = crate::utils::read_into_vec(buf, path) else {
        return None;
    };

    while let Some(pos) = memchr::memchr(b'\n', status) {
        let line = BStr::new(&status[..pos]);

        if let Some(ppid) = line.strip_prefix(key.as_bytes()) {
            return Some(BStr::new(ppid).trim());
        }

        status = &status[pos + 1..];
    }

    None
}
