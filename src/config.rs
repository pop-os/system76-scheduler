// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use serde::Deserialize;
use std::{fs, path::Path};

const DISTRIBUTION_PATH: &str = "/usr/lib/";
const SYSTEM_CONF_PATH: &str = "/etc/";

const PROFILES_PATH: &str = "system76-scheduler/cpu/";

const DEFAULT_CONFIG: Config = Config {
    latency: 6,
    minimum_granularity: 0.75,
    wakeup_granularity: 1.0,
    migration_cost: 0.5,
    bandwidth_size: 5,
};

const RESPONSIVE_CONFIG: Config = Config {
    latency: 4,
    minimum_granularity: 0.4,
    wakeup_granularity: 0.5,
    migration_cost: 0.25,
    bandwidth_size: 3,
};

#[derive(Deserialize)]
pub struct Config {
    /// Preemption latency for CPU-bound tasks in ns
    pub latency: u64,
    /// Minimum preemption granularity for CPU-bound tasks in ms
    pub minimum_granularity: f64,
    /// Wakeup preemption granularity for CPU-bound tasks in ms
    pub wakeup_granularity: f64,
    /// Cost of CPU task migration in ms
    pub migration_cost: f64,
    /// Amount of time to allocate from global to local pool in us
    pub bandwidth_size: u64,
}

impl Config {
    pub fn config_path(config: &str) -> Option<String> {
        let mut path = [SYSTEM_CONF_PATH, PROFILES_PATH, config, ".toml"].concat();

        if Path::new(&path).exists() {
            return Some(path);
        }

        path = [DISTRIBUTION_PATH, PROFILES_PATH, config, ".toml"].concat();

        if Path::new(&path).exists() {
            return Some(path);
        }

        None
    }

    pub fn custom_config(profile: &str) -> Option<Self> {
        if let Some(path) = Self::config_path(profile) {
            if let Ok(file) = fs::read_to_string(&path) {
                if let Ok(conf) = toml::from_str(&file) {
                    return Some(conf);
                }
            }
        }

        None
    }

    pub fn responsive_config() -> Self {
        let mut conf = RESPONSIVE_CONFIG;

        if let Some(config) = Self::custom_config("responsive") {
            conf = config;
        }

        conf
    }

    pub fn default_config() -> Self {
        let mut conf = DEFAULT_CONFIG;

        if let Some(config) = Self::custom_config("default") {
            conf = config;
        }

        conf
    }
}
