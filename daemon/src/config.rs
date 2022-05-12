// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use concat_in_place::strcat;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

const DISTRIBUTION_PATH: &str = "/usr/share/";
const SYSTEM_CONF_PATH: &str = "/etc/";

const CONFIG_PATH: &str = "system76-scheduler/config.ron";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub enum IoPriority {
    Idle,
    Standard,
    BestEffort(PriorityLevel),
    Realtime(PriorityLevel),
}

impl From<IoPriority> for ioprio::Priority {
    fn from(priority: IoPriority) -> ioprio::Priority {
        use ioprio::{BePriorityLevel, Class, Priority, RtPriorityLevel};
        match priority {
            IoPriority::BestEffort(value) => {
                let level = BePriorityLevel::from_level(value.get()).unwrap();
                Priority::new(Class::BestEffort(level))
            }
            IoPriority::Idle => Priority::new(Class::Idle),
            IoPriority::Realtime(value) => {
                let level = RtPriorityLevel::from_level(value.get()).unwrap();
                Priority::new(Class::Realtime(level))
            }
            IoPriority::Standard => Priority::standard(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(from = "AssignmentRaw")]
pub struct Assignment(pub CpuPriority, pub IoPriority);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(untagged)]
pub enum AssignmentRaw {
    Cpu(CpuPriority),
    Io(IoPriority),
    Both((CpuPriority, IoPriority)),
}

impl From<AssignmentRaw> for Assignment {
    fn from(raw: AssignmentRaw) -> Self {
        match raw {
            AssignmentRaw::Cpu(cpu) => Assignment(cpu, IoPriority::Standard),
            AssignmentRaw::Io(io) => Assignment(CpuPriority::from(0), io),
            AssignmentRaw::Both((cpu, io)) => Assignment(cpu, io),
        }
    }
}

/// Restricts the value between -20 through 19.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct CpuPriority(i8);

impl CpuPriority {
    pub fn get(self) -> i8 {
        self.0
    }
}

impl From<i8> for CpuPriority {
    fn from(level: i8) -> Self {
        Self(level.min(19).max(-20))
    }
}

/// Restricts the value between 0 through 7.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Hash, PartialEq, Eq, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PriorityLevel(u8);

impl PriorityLevel {
    pub fn get(self) -> u8 {
        self.0
    }
}

impl From<u8> for PriorityLevel {
    fn from(level: u8) -> Self {
        Self(level.min(7))
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
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

        let mut buffer = String::with_capacity(4096);

        for path in directories {
            if let Ok(config) = crate::utils::read_into_string(&mut buffer, &path) {
                match ron::from_str(config) {
                    Ok(config) => return config,
                    Err(why) => {
                        tracing::error!(
                            "{:?}: {} on line {}, column {}",
                            path,
                            why.code,
                            why.position.line,
                            why.position.col
                        );
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

    pub fn automatic_assignments() -> BTreeMap<String, Assignment> {
        let mut assignments = BTreeMap::<String, Assignment>::new();

        let directories = [
            Path::new("/usr/share/system76-scheduler/assignments/"),
            Path::new("/etc/system76-scheduler/assignments/"),
        ];

        for directory in directories {
            if let Ok(dir) = directory.read_dir() {
                for entry in dir.filter_map(Result::ok) {
                    let path = entry.path();
                    if let Ok(string) = fs::read_to_string(&path) {
                        match ron::from_str::<BTreeMap<Assignment, HashSet<String>>>(&string) {
                            Ok(buffer) => {
                                if tracing::event_enabled!(tracing::Level::INFO) {
                                    let log = fomat_macros::fomat!(
                                        (path.display()) ":\n"
                                        for value in &buffer {
                                            "\t" [value] "\n"
                                        }
                                    );

                                    tracing::info!("{}", log);
                                }

                                for (priority, commands) in buffer {
                                    for command in commands {
                                        assignments.insert(command, priority);
                                    }
                                }
                            }
                            Err(why) => {
                                tracing::error!(
                                    "{:?}: {} on line {}, column {}",
                                    entry.path(),
                                    why.code,
                                    why.position.line,
                                    why.position.col
                                );
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
    use serde::{Deserialize, Serialize};
    use std::{borrow::Cow, fs, path::Path};

    use super::{preempt_default, strcat, DISTRIBUTION_PATH, SYSTEM_CONF_PATH};

    const PROFILES_PATH: &str = "system76-scheduler/cpu/";

    const DEFAULT_CONFIG: Config = Config {
        latency: 6,
        nr_latency: 8,
        wakeup_granularity: 1.0,
        bandwidth_size: 5,
        preempt: preempt_default(),
    };

    const RESPONSIVE_CONFIG: Config = Config {
        latency: 4,
        nr_latency: 10,
        wakeup_granularity: 0.5,
        bandwidth_size: 3,
        preempt: Cow::Borrowed("full"),
    };

    #[derive(Deserialize, Serialize)]
    pub struct Config {
        /// Preemption latency for CPU-bound tasks in ns
        pub latency: u64,
        /// Used to calculate the minimum preemption granularity
        pub nr_latency: u64,
        /// Wakeup preemption granularity for CPU-bound tasks in ms
        pub wakeup_granularity: f64,
        /// Amount of time to allocate from global to local pool in us
        pub bandwidth_size: u64,
        /// The type of preemption to use.
        #[serde(default = "preempt_default")]
        pub preempt: Cow<'static, str>,
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

const fn preempt_default() -> Cow<'static, str> {
    Cow::Borrowed("voluntary")
}
