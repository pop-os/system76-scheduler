// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use super::Profile;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
};
use wildmatch::WildMatch;

/// Conditional assignment
#[derive(Default, Debug)]
pub struct Condition {
    /// Match by process descendant
    pub descends: Option<MatchCondition>,
    /// Match by cgroup
    pub cgroup: Option<MatchCondition>,
    /// Match by process name
    pub name: Option<MatchCondition>,
    /// Match by process parent
    pub parent: Vec<MatchCondition>,
}

/// A wildcard string match which either is or isn't
#[must_use]
#[derive(Debug)]
pub enum MatchCondition {
    /// Is a match for the wildcard
    Is(WildMatch),
    /// Is not a match for the wildcard
    IsNot(WildMatch),
}

impl MatchCondition {
    /// Parses a `MatchCondition`
    pub fn new(input: &str) -> Self {
        if let Some(input) = input.strip_prefix('!') {
            Self::IsNot(WildMatch::new(input))
        } else {
            Self::Is(WildMatch::new(input))
        }
    }

    /// Identifies if the input is a match for the condition
    #[must_use]
    pub fn matches(&self, input: &str) -> bool {
        match self {
            Self::Is(condition) => condition.matches(input),
            Self::IsNot(condition) => !condition.matches(input),
        }
    }
}

/// Process scheduler assignments
#[derive(Default, Debug)]
pub struct Assignments {
    /// Conditional assignments
    #[allow(clippy::type_complexity)]
    pub conditions: HashMap<Box<str>, (Profile, Vec<(Condition, bool)>)>,
    /// Exceptions by name
    pub(crate) exceptions_by_name: BTreeSet<Box<str>>,
    /// Exceptions by cmdline
    pub(crate) exceptions_by_cmdline: BTreeSet<Box<str>>,
    /// Conditional exceptions
    pub exceptions_conditions: Vec<Condition>,
    /// Assignment profiles
    pub(crate) profiles: BTreeMap<Arc<str>, Profile>,
    /// Profiles mapped by name
    pub(crate) profile_by_name: BTreeMap<Box<str>, Profile>,
    /// Profiles mapped by cmdline
    pub(crate) profile_by_cmdline: BTreeMap<Box<str>, Profile>,
}

impl Assignments {
    /// Clears all assignments
    pub fn clear(&mut self) {
        self.conditions.clear();
        self.profiles.clear();
        self.profile_by_name.clear();
        self.profile_by_cmdline.clear();
        self.exceptions_by_cmdline.clear();
        self.exceptions_by_name.clear();
        self.exceptions_conditions.clear();
    }

    /// Get a matching profile for a process by its name
    #[must_use]
    pub fn get_by_name<'a>(&'a self, process: &str) -> Option<&'a Profile> {
        self.profile_by_name.get(process)
    }

    /// Get a matching profile for a process by its cmdline
    #[must_use]
    pub fn get_by_cmdline<'a>(&'a self, process: &str) -> Option<&'a Profile> {
        self.profile_by_cmdline.get(process)
    }

    /// Check if a process is excepted by its name
    #[must_use]
    pub fn is_exception_by_name(&self, name: &str) -> bool {
        self.exceptions_by_name.contains(name)
    }

    /// Check if a process is excepted by its cmdline
    #[must_use]
    pub fn is_exception_by_cmdline(&self, name: &str) -> bool {
        self.exceptions_by_cmdline.contains(name)
    }

    /// Get a profile by the profile's name
    #[must_use]
    pub fn profile<'a>(&'a self, profile: &str) -> Option<&'a Profile> {
        self.profiles.get(profile)
    }

    /// Insert a new profile
    pub fn profile_insert(&mut self, name: Arc<str>, profile: Profile) {
        self.profiles.insert(name, profile);
    }

    /// Assign a process to a profile by the process's name
    pub fn assign_by_name(&mut self, name: &str, profile: Profile) {
        self.profile_by_name.insert(name.into(), profile);
    }

    /// Assign a condition to a profile
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

    /// Assign a process to a profile by the process's cmdline
    pub fn assign_by_cmdline(&mut self, name: &str, profile: Profile) {
        self.profile_by_cmdline.insert(name.into(), profile);
    }

    /// Assign a process as an exception by its cmdline
    pub fn assign_exception_by_cmdline(&mut self, name: &str) {
        self.exceptions_by_cmdline.insert(name.into());
    }

    /// Assign a condition as an exception
    pub fn assign_exception_by_condition(&mut self, condition: Condition) {
        self.exceptions_conditions.push(condition);
    }

    /// Assign a process as an exception by its name
    pub fn assign_exception_by_name(&mut self, name: &str) {
        self.exceptions_by_name.insert(name.into());
    }
}
