// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::config::cpu::Config;
use crate::paths::*;
use std::fs;

/// Apply a configuration to CPU scheduler latencies.
pub fn tweak(paths: &SchedPaths, conf: &Config) {
    let modifier = latency_modifier(num_cpus::get() as f64);

    write_value(paths.latency, modifier * conf.latency);
    write_value(paths.min_gran, modifier as f64 * conf.minimum_granularity);
    write_value(paths.wakeup_gran, modifier as f64 * conf.wakeup_granularity);
    write_value(BANDWIDTH_SIZE_PATH, conf.bandwidth_size * 1000);
}

/// Write a value that implements `ToString` to a file
fn write_value<V: ToString>(path: &str, value: V) {
    if let Err(why) = fs::write(path, value.to_string().as_bytes()) {
        eprintln!("failed to set value in {}: {}", path, why);
    }
}

/// Latency modifier to be applied to scheduler latencies based on CPU core count.
fn latency_modifier(nprocs: f64) -> u64 {
    10u64.pow(6) * (1f64 + nprocs.ln() / 2f64.ln()) as u64
}

#[cfg(test)]
mod tests {
    #[test]
    fn latency_modifier() {
        assert_eq!(5000000, super::latency_modifier(16f64));
    }
}
