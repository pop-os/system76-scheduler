// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

pub mod paths;

use crate::config::cfs::Profile;
use paths::{SchedPaths, BANDWIDTH_SIZE_PATH};
use std::fmt::Display;
use std::io::Write;
use std::{fs, io};

/// Apply a configuration to CPU scheduler latencies.
#[allow(clippy::cast_precision_loss)]
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)]
pub fn tweak(paths: &SchedPaths, conf: &Profile) {
    let modifier = latency_modifier(num_cpus::get() as f64);

    let min_gran = (modifier as f64 * conf.latency as f64 / conf.nr_latency as f64) as u64;
    let wakeup_gran = (modifier as f64 * conf.wakeup_granularity) as u64;

    write_value(paths.latency, modifier * conf.latency);
    write_value(paths.min_gran, min_gran);
    write_value(paths.wakeup_gran, wakeup_gran);
    write_value(BANDWIDTH_SIZE_PATH, conf.bandwidth_size * 1000);

    if let Some(preempt_path) = paths.preempt {
        write_value(preempt_path, conf.preempt);
    }
}

/// Write a value that implements `Display` to a file
fn write_value<V: Display>(path: &str, value: V) {
    let write_to_file = |path, value| -> io::Result<()> {
        let mut file = fs::File::create(path)?;
        write!(file, "{}", value)?;

        Ok(())
    };

    if let Err(why) = write_to_file(path, value) {
        eprintln!("failed to set value in {}: {}", path, why);
    }
}

/// Latency modifier to be applied to scheduler latencies based on CPU core count.
#[allow(clippy::cast_sign_loss)]
#[allow(clippy::cast_possible_truncation)]
fn latency_modifier(nprocs: f64) -> u64 {
    10u64.pow(6) * (1f64 + nprocs.ln() / 2f64.ln()) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn latency_modifier() {
        assert_eq!(5_000_000, super::latency_modifier(16f64));
    }
}
