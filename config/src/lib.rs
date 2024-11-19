// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#![deny(missing_docs)]

//! System76 Scheduler's configuration parsing and logic.

/// CFS configurations
pub mod cfs;

pub(crate) mod kdl;

mod parser;

/// Process scheduler configurations
pub mod scheduler;

use std::{
    fs::File,
    io::{self, Read},
};

const DISTRIBUTION_PATH: &str = "/usr/share/system76-scheduler/";
const SYSTEM_CONF_PATH: &str = "/etc/system76-scheduler/";

/// System76 Scheduler configuration
#[must_use]
#[derive(Default)]
pub struct Config {
    /// Controls autogrouping status
    pub autogroup_enabled: bool,

    /// CFS profiles
    pub cfs_profiles: cfs::Config,

    /// Process scheduler config
    pub process_scheduler: scheduler::Config,
}

/// Parses the scheduler's configuration files
pub fn config() -> Config {
    parser::read_config()
}

/// Locates configuration files of a given extension from the given paths.
pub fn configuration_files(
    paths: &'static [&'static str],
    extension: &'static str,
) -> impl Iterator<Item = String> {
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
