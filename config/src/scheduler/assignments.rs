// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::Profile;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use wildmatch::WildMatch;

#[derive(Default, Debug)]
pub struct Condition {
    pub descends: Option<MatchCondition>,
    pub cgroup: Option<MatchCondition>,
    pub name: Option<MatchCondition>,
    pub parent: Vec<MatchCondition>,
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

#[derive(Default, Debug)]
pub struct Assignments {
    #[allow(clippy::type_complexity)]
    pub conditions: HashMap<Box<str>, (Profile, Vec<(Condition, bool)>)>,
    pub(crate) exceptions_by_name: BTreeSet<Box<str>>,
    pub(crate) exceptions_by_cmdline: BTreeSet<Box<str>>,
    pub exceptions_conditions: Vec<Condition>,
    pub(crate) profiles: BTreeMap<Box<str>, Profile>,
    pub(crate) profile_by_name: BTreeMap<Box<str>, Profile>,
    pub(crate) profile_by_cmdline: BTreeMap<Box<str>, Profile>,
}

impl Assignments {
    pub fn clear(&mut self) {
        self.conditions.clear();
        self.profiles.clear();
        self.profile_by_name.clear();
        self.profile_by_cmdline.clear();
        self.exceptions_by_cmdline.clear();
        self.exceptions_by_name.clear();
        self.exceptions_conditions.clear();
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

    pub fn assign_by_condition(
        &mut self,
        name: &str,
        condition: Condition,
        profile: Profile,
        include: bool,
    ) {
        self.conditions
            .entry(name.into())
            .or_insert_with(|| (profile, Vec::new()))
            .1
            .push((condition, include));
    }

    pub fn assign_by_cmdline(&mut self, name: &str, profile: Profile) {
        self.profile_by_cmdline.insert(name.into(), profile);
    }

    pub fn assign_exception_by_cmdline(&mut self, name: &str) {
        self.exceptions_by_cmdline.insert(name.into());
    }

    pub fn assign_exception_by_condition(&mut self, condition: Condition) {
        self.exceptions_conditions.push(condition);
    }

    pub fn assign_exception_by_name(&mut self, name: &str) {
        self.exceptions_by_name.insert(name.into());
    }
}
