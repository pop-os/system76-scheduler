// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use compact_str::CompactString;
use kdl::KdlNode;
use std::collections::BTreeMap;

pub struct Config {
    pub enable: bool,
    pub profiles: BTreeMap<CompactString, Profile>,
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            enable: false,
            profiles: BTreeMap::new(),
        };

        config
            .profiles
            .insert("default".into(), crate::cfs::PROFILE_DEFAULT);
        config
            .profiles
            .insert("responsive".into(), crate::cfs::PROFILE_RESPONSIVE);
        config
    }
}

pub const PROFILE_DEFAULT: Profile = Profile {
    latency: 6,
    nr_latency: 8,
    wakeup_granularity: 1.0,
    bandwidth_size: 5,
    preempt: "voluntary",
};

pub const PROFILE_RESPONSIVE: Profile = Profile {
    latency: 4,
    nr_latency: 10,
    wakeup_granularity: 0.5,
    bandwidth_size: 3,
    preempt: "full",
};

pub struct Profile {
    /// Preemption latency for CPU-bound tasks in ns
    pub latency: u64,
    /// Used to calculate the minimum preemption granularity
    pub nr_latency: u64,
    /// Wakeup preemption granularity for CPU-bound tasks in ms
    pub wakeup_granularity: f64,
    /// Amount of time to allocate from global to local pool in us
    pub bandwidth_size: u64,
    /// The type of preemption to use.
    pub preempt: &'static str,
}

pub fn parse(nodes: &[KdlNode]) -> impl Iterator<Item = (&str, Profile)> {
    nodes.iter().map(|node| {
        let mut config = PROFILE_DEFAULT;

        for (name, entry) in crate::kdl::iter_properties(node) {
            match name {
                "latency" =>
                {
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(value) = entry.value().as_i64() {
                        config.latency = value as u64;
                    }
                }

                "nr-latency" =>
                {
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(value) = entry.value().as_i64() {
                        config.nr_latency = value as u64;
                    }
                }

                "wakeup-granularity" => {
                    if let Some(value) = entry.value().as_f64() {
                        config.wakeup_granularity = value;
                    }
                }

                "bandwidth-size" =>
                {
                    #[allow(clippy::cast_sign_loss)]
                    if let Some(value) = entry.value().as_i64() {
                        config.bandwidth_size = value as u64;
                    }
                }

                "preempt" => {
                    if let Some(value) = entry.value().as_string() {
                        match value {
                            "voluntary" => config.preempt = "voluntary",
                            "full" => config.preempt = "full",
                            _ => tracing::warn!("preempt expected one of: voluntary full"),
                        }
                    }
                }

                _ => (),
            }
        }

        (node.name().value(), config)
    })
}
