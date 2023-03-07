// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::Profile;
use std::collections::{BTreeMap, BTreeSet};
use wildmatch::WildMatch;

#[derive(Default, Debug)]
pub struct Condition {
    pub cgroup: Option<MatchCondition>,
    pub parent: Option<MatchCondition>,
}

#[must_use]
#[derive(Debug)]
pub enum MatchCondition {
    Is(WildMatch),
    IsNot(WildMatch),
}

impl MatchCondition {
    pub fn new(input: &str) -> Self {
        if let Some(input) = input.strip_prefix('!') {
            Self::IsNot(WildMatch::new(input))
        } else {
            Self::Is(WildMatch::new(input))
        }
    }

    #[must_use]
    pub fn matches(&self, input: &str) -> bool {
        match self {
            Self::Is(condition) => condition.matches(input),
            Self::IsNot(condition) => !condition.matches(input),
        }
    }
}

#[derive(Copy, Clone)]
pub struct ProcessConditions<'a> {
    pub cgroup: &'a str,
    pub parent: &'a str,
}

#[derive(Default, Debug)]
pub struct Assignments {
    pub(crate) profiles: BTreeMap<Box<str>, Profile>,
    pub(crate) profile_by_name: BTreeMap<Box<str>, Profile>,
    pub(crate) profile_by_cmdline: BTreeMap<Box<str>, Profile>,
    pub(crate) conditions: Vec<(Condition, Profile)>,
    pub(crate) exceptions_by_name: BTreeSet<Box<str>>,
    pub(crate) exceptions_by_cmdline: BTreeSet<Box<str>>,
}

impl Assignments {
    pub fn clear(&mut self) {
        self.profiles.clear();
        self.profile_by_name.clear();
        self.profile_by_cmdline.clear();
        self.conditions.clear();
        self.exceptions_by_cmdline.clear();
        self.exceptions_by_name.clear();
    }

    #[must_use]
    pub fn get_by_name<'a>(&'a self, process: &str) -> Option<&'a Profile> {
        self.profile_by_name.get(process)
    }

    #[must_use]
    pub fn get_by_cmdline<'a>(&'a self, process: &str) -> Option<&'a Profile> {
        self.profile_by_cmdline.get(process)
    }

    #[must_use]
    pub fn get_by_condition<'a>(&'a self, conditions: ProcessConditions) -> Option<&'a Profile> {
        for (pattern, profile) in self.conditions.iter().rev() {
            if let Some(ref cgroup) = pattern.cgroup {
                if !cgroup.matches(conditions.cgroup) {
                    continue;
                }
            }

            if let Some(ref parent) = pattern.parent {
                if !parent.matches(conditions.parent) {
                    continue;
                }
            }

            return Some(profile);
        }

        None
    }

    #[must_use]
    pub fn is_exception_by_name(&self, name: &str) -> bool {
        self.exceptions_by_name.contains(name)
    }

    #[must_use]
    pub fn is_exception_by_cmdline(&self, name: &str) -> bool {
        self.exceptions_by_cmdline.contains(name)
    }

    #[must_use]
    pub fn profile<'a>(&'a self, profile: &str) -> Option<&'a Profile> {
        self.profiles.get(profile)
    }

    pub fn profile_insert(&mut self, name: &str, profile: Profile) {
        self.profiles.insert(name.into(), profile);
    }

    pub fn assign_by_name(&mut self, name: &str, profile: Profile) {
        self.profile_by_name.insert(name.into(), profile);
    }

    pub fn assign_by_condition(&mut self, condition: Condition, profile: Profile) {
        self.conditions.push((condition, profile));
    }

    pub fn assign_by_cmdline(&mut self, name: &str, profile: Profile) {
        self.profile_by_cmdline.insert(name.into(), profile);
    }

    pub fn assign_exception_by_cmdline(&mut self, name: &str) {
        self.exceptions_by_cmdline.insert(name.into());
    }

    pub fn assign_exception_by_name(&mut self, name: &str) {
        self.exceptions_by_name.insert(name.into());
    }
}
