// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

pub mod cfs;
pub mod kdl;
mod parser;
pub mod scheduler;

use std::{
    fs::File,
    io::{self, Read},
};

const DISTRIBUTION_PATH: &str = "/usr/share/system76-scheduler/";
const SYSTEM_CONF_PATH: &str = "/etc/system76-scheduler/";

#[must_use]
#[derive(Default)]
pub struct Config {
    pub autogroup_enabled: bool,
    pub cfs_profiles: cfs::Config,
    pub process_scheduler: scheduler::Config,
}

pub fn config() -> Config {
    parser::read_config()
}

pub fn configuration_files<'a>(
    paths: &'a [&str],
    extension: &'a str,
) -> impl Iterator<Item = String> + 'a {
    generator::Gn::new_scoped(move |mut scope| {
        for directory in paths {
            if let Ok(dir) = std::fs::read_dir(directory) {
                for entry in dir.filter_map(Result::ok) {
                    if let Some(file_name) = entry.file_name().to_str() {
                        if file_name.ends_with(extension) {
                            scope.yield_([directory, "/", file_name].concat());
                        }
                    }
                }
            }
        }

        generator::done!()
    })
}

fn read_into_string<'a>(buf: &'a mut String, path: &str) -> io::Result<&'a str> {
    let mut file = File::open(path)?;
    buf.clear();
    file.read_to_string(buf)?;
    Ok(&*buf)
}
