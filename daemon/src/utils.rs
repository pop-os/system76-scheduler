// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use std::ffi::OsStr;
use std::fs::File;
use std::io::{self, Read};

pub fn read_into_string<P: AsRef<OsStr>>(buf: &mut String, path: P) -> io::Result<&str> {
    let mut file = File::open(path.as_ref())?;
    buf.clear();
    file.read_to_string(buf)?;
    Ok(&*buf)
}
