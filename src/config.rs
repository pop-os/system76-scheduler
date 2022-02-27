// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use concat_in_place::strcat;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

const DISTRIBUTION_PATH: &str = "/usr/share/";
const SYSTEM_CONF_PATH: &str = "/etc/";

const CONFIG_PATH: &str = "system76-scheduler/config.ron";

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    pub background: Option<i8>,
    pub foreground: Option<i8>,
}

impl Config {
    pub fn read() -> Config {
        let directories = [
            strcat!(SYSTEM_CONF_PATH CONFIG_PATH),
            strcat!(DISTRIBUTION_PATH CONFIG_PATH),
        ];

        for path in directories {
            if let Ok(config) = std::fs::read_to_string(&path) {
                match ron::from_str(&config) {
                    Ok(config) => return config,
                    Err(why) => {
                        tracing::error!("{}: {:?}", path, why);
                    }
                }
            }
        }

        tracing::info!("Using default config values due to config error");

        Config {
            background: Some(5),
            foreground: Some(-5),
        }
    }

    pub fn automatic_assignments() -> BTreeMap<String, i8> {
        let mut assignments = BTreeMap::<String, i8>::new();

        let directories = [
            Path::new("/usr/share/system76-scheduler/assignments/"),
            Path::new("/etc/system76-scheduler/assignments/"),
        ];

        for directory in directories {
            if let Ok(dir) = directory.read_dir() {
                for entry in dir.filter_map(Result::ok) {
                    if let Ok(string) = fs::read_to_string(entry.path()) {
                        if let Ok(buffer) = ron::from_str::<BTreeMap<i8, HashSet<String>>>(&string)
                        {
                            for (priority, commands) in buffer {
                                for command in commands {
                                    assignments.insert(command, priority);
                                }
                            }
                        }
                    }
                }
            }
        }

        assignments
    }
}

pub mod cpu {
    use serde::Deserialize;
    use std::{fs, path::Path};

    use super::*;

    const PROFILES_PATH: &str = "system76-scheduler/cpu/";

    const DEFAULT_CONFIG: Config = Config {
        latency: 6,
        nr_latency: 8,
        wakeup_granularity: 1.0,
        bandwidth_size: 5,
    };

    const RESPONSIVE_CONFIG: Config = Config {
        latency: 4,
        nr_latency: 10,
        wakeup_granularity: 0.5,
        bandwidth_size: 3,
    };

    #[derive(Deserialize)]
    pub struct Config {
        /// Preemption latency for CPU-bound tasks in ns
        pub latency: u64,
        /// Used to calculate the minimum preemption granularity
        pub nr_latency: u64,
        /// Wakeup preemption granularity for CPU-bound tasks in ms
        pub wakeup_granularity: f64,
        /// Amount of time to allocate from global to local pool in us
        pub bandwidth_size: u64,
    }

    impl Config {
        pub fn config_path(config: &str) -> Option<String> {
            let mut path = strcat!(SYSTEM_CONF_PATH PROFILES_PATH config ".ron");

            if Path::new(&path).exists() {
                return Some(path);
            }

            path.clear();
            strcat!(&mut path, DISTRIBUTION_PATH PROFILES_PATH config ".ron");

            if Path::new(&path).exists() {
                return Some(path);
            }

            None
        }

        pub fn custom_config(profile: &str) -> Option<Self> {
            if let Some(path) = Self::config_path(profile) {
                if let Ok(file) = fs::read_to_string(&path) {
                    if let Ok(conf) = ron::from_str(&file) {
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
}
