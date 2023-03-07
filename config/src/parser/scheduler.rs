// Copyright 2023 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::kdl::NodeExt;
use crate::scheduler::{Assignments, Condition, Config, MatchCondition, Profile};
use crate::{
    kdl::EntryExt,
    scheduler::{IoClass, Niceness, SchedPolicy, SchedPriority},
};
use kdl::{KdlEntry, KdlIdentifier, KdlNode};

impl Config {
    pub fn read(&mut self, node: &KdlNode) {
        self.enable = node.enabled().unwrap_or(true);

        if !self.enable {
            return;
        }

        if let Some(fields) = node.children() {
            for (name, node) in crate::kdl::fields(fields) {
                match name {
                    "refresh-rate" => {
                        if let Some(value) = node.get_u16(0) {
                            self.refresh_rate = value;
                        }
                    }

                    "execsnoop" => {
                        if let Some(value) = node.get_bool(0) {
                            self.execsnoop = value;
                        }
                    }

                    "assignments" => self.assignments.parse(node),

                    "exceptions" => self.assignments.parse_exceptions(node),

                    other => {
                        tracing::warn!("unknown element: {}", other);
                    }
                }
            }
        }
    }
}

impl Assignments {
    pub fn parse(&mut self, node: &KdlNode) {
        let Some(document) = node.children() else {
            return;
        };

        for profile_node in document.nodes() {
            let profile_name = profile_node.name().value();

            let span = tracing::warn_span!("Assignments::parse", profile = profile_name);
            let _entered = span.enter();

            // Stores the properties defined for this profile profile.
            let (exists, profile) = self
                .profile(profile_name)
                .map_or_else(|| (false, Profile::default()), |p| (true, p.clone()));

            let profile = profile.parse(profile_node);

            if !exists {
                self.profile_insert(profile_name, profile.clone());
            }

            if let Some(rules) = profile_node.children() {
                for (number, pattern) in rules.nodes().iter().enumerate() {
                    let name = pattern.name().value();

                    let span = tracing::warn_span!("assignment", number = number + 1, name);
                    let _entered = span.enter();

                    if name == "*" {
                        let mut condition = Condition::default();
                        let mut profile = profile.clone();

                        for (property, entry) in
                            profile.parse_properties(crate::kdl::iter_properties(pattern))
                        {
                            match property {
                                "cgroup" => {
                                    condition.cgroup =
                                        entry.value().as_string().map(MatchCondition::new);
                                }
                                "parent" => {
                                    condition.parent =
                                        entry.value().as_string().map(MatchCondition::new);
                                }
                                _ => {
                                    tracing::error!("unknown property: {}", property);
                                }
                            }
                        }

                        if condition.cgroup.is_some() || condition.parent.is_some() {
                            self.assign_by_condition(condition, profile);
                        }
                    } else {
                        let profile = profile.clone().parse(pattern);
                        if name.starts_with('/') {
                            self.assign_by_cmdline(name, profile);
                        } else {
                            self.assign_by_name(name, profile);
                        }
                    }
                }
            }
        }
    }

    pub fn parse_exceptions(&mut self, node: &KdlNode) {
        let Some(document) = node.children() else {
            return;
        };

        for node in document.nodes() {
            let exception = node.name().value();

            if exception.starts_with('/') {
                self.assign_exception_by_cmdline(exception);
            } else {
                self.assign_exception_by_name(exception);
            }
        }
    }
}

impl Profile {
    pub fn parse(mut self, node: &KdlNode) -> Self {
        for (property, _) in self.parse_properties(crate::kdl::iter_properties(node)) {
            tracing::error!("unknown property: {}", property);
        }

        self
    }

    pub fn parse_properties<'a>(
        &'a mut self,
        entries: impl Iterator<Item = (&'a str, &'a KdlEntry)> + 'a,
    ) -> impl Iterator<Item = (&'a str, &'a KdlEntry)> + 'a {
        entries.filter(|&(property, entry)| {
            match property {
                "io" => self.parse_io(entry),
                "nice" => self.parse_nice(entry),
                "sched" => self.parse_sched(entry),
                _ => return true,
            }

            false
        })
    }

    #[tracing::instrument(skip_all)]
    pub fn parse_io(&mut self, entry: &KdlEntry) {
        let class = entry
            .ty()
            .map(KdlIdentifier::value)
            .or_else(|| entry.value().as_string());

        let Some(class) = class else {
            tracing::warn!("expects class: idle best-effort realtime");
            return;
        };

        let Ok(class) = class.parse::<IoClass>() else {
            tracing::error!("unknown class: {}", class);
            return;
        };

        self.io = match class {
            IoClass::BestEffort => ioprio::Class::BestEffort(
                ioprio::BePriorityLevel::from_level(entry.as_u8().unwrap_or(7))
                    .unwrap_or_else(ioprio::BePriorityLevel::lowest),
            ),

            IoClass::Idle => ioprio::Class::Idle,

            IoClass::Realtime => ioprio::Class::Realtime(
                ioprio::RtPriorityLevel::from_level(entry.as_u8().unwrap_or(7))
                    .unwrap_or_else(ioprio::RtPriorityLevel::lowest),
            ),
        };
    }

    #[tracing::instrument(skip_all)]
    pub fn parse_nice(&mut self, entry: &KdlEntry) {
        let Some(niceness) = entry.as_i8() else {
            tracing::error!("expects number between -20 and 19");
            return
        };

        self.nice = Niceness::from(niceness);
    }

    #[tracing::instrument(skip_all)]
    pub fn parse_sched(&mut self, entry: &KdlEntry) {
        if let Some(policy) = entry.ty().map(KdlIdentifier::value) {
            let Ok(policy) = policy.parse::<SchedPolicy>() else {
                tracing::error!("unknown sched policy");
                return
            };

            let Some(priority) = entry.as_u8() else {
                tracing::error!("expected priority assignment between 1-99");
                return
            };

            self.sched_policy = policy;
            self.sched_priority = SchedPriority::from(priority);

            return;
        }

        let Some(policy) = entry.parse_to::<SchedPolicy>() else {
            tracing::error!("expected one of: batch deadline fifo idle other rr");
            return
        };

        self.sched_policy = policy;
    }
}
