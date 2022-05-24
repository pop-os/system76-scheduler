// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use compact_str::CompactStr;
use concat_in_place::strcat;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

const DISTRIBUTION_PATH: &str = "/usr/share/system76-scheduler/";
const SYSTEM_CONF_PATH: &str = "/etc/system76-scheduler/";

pub type Exceptions = BTreeSet<CompactStr>;
pub type Assignments = BTreeMap<CompactStr, Assignment>;

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
            IoPriority::Standard => {
                let level = BePriorityLevel::from_level(7).unwrap();
                Priority::new(Class::BestEffort(level))
            }
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

#[derive(Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub background: Option<i8>,

    #[serde(default)]
    pub foreground: Option<i8>,

    #[serde(default)]
    pub use_execsnoop: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            background: Some(5),
            foreground: Some(-5),
            use_execsnoop: true,
        }
    }
}

impl Config {
    pub fn read() -> Config {
        let directories = [
            strcat!(SYSTEM_CONF_PATH "config.ron"),
            strcat!(DISTRIBUTION_PATH "config.ron"),
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

        Config::default()
    }
}

/// Process names that should have assignments ignored.
#[allow(clippy::doc_markdown)]
pub fn exceptions() -> Exceptions {
    let paths = [
        Path::new(concatcp!(DISTRIBUTION_PATH, "exceptions/")),
        Path::new(concatcp!(SYSTEM_CONF_PATH, "exceptions/")),
    ];

    mk_gen!(let generator = configuration_files(&paths));

    let mut exceptions = BTreeSet::new();

    for path in generator {
        if let Ok(string) = fs::read_to_string(&path) {
            match ron::from_str::<Exceptions>(&string) {
                Ok(rules) => {
                    exceptions.extend(rules.into_iter());
                }

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

    exceptions
}

/// Stores process names and their preferred assignments.
#[allow(clippy::doc_markdown)]
pub fn assignments(exceptions: &Exceptions) -> Assignments {
    let paths = [
        Path::new(concatcp!(DISTRIBUTION_PATH, "assignments/")),
        Path::new(concatcp!(SYSTEM_CONF_PATH, "assignments/")),
    ];

    mk_gen!(let generator = configuration_files(&paths));

    let mut assignments = BTreeMap::<CompactStr, Assignment>::new();

    for path in generator {
        if let Ok(string) = fs::read_to_string(&path) {
            match ron::from_str::<BTreeMap<Assignment, Exceptions>>(&string) {
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

                    for (assignment, commands) in buffer {
                        for command in commands {
                            if !exceptions.contains(&command) {
                                assignments.insert(command, assignment);
                            }
                        }
                    }
                }
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

    assignments
}

#[generator(yield(PathBuf))]
fn configuration_files(paths: &[&Path]) {
    for directory in paths {
        if let Ok(dir) = directory.read_dir() {
            for entry in dir.filter_map(Result::ok) {
                let path = entry.path();
                if path.extension() == Some(OsStr::new("ron")) {
                    yield_!(path);
                }
            }
        }
    }
}

pub mod cpu {
    use serde::{Deserialize, Serialize};
    use std::{borrow::Cow, fs, path::Path};

    use super::{preempt_default, strcat, DISTRIBUTION_PATH, SYSTEM_CONF_PATH};

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
            let mut path = strcat!(SYSTEM_CONF_PATH "cpu/" config ".ron");

            if Path::new(&path).exists() {
                return Some(path);
            }

            path.clear();
            strcat!(&mut path, DISTRIBUTION_PATH "cpu/" config ".ron");

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
