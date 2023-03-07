mod cfs;
mod scheduler;

use std::path::Path;

use crate::kdl::NodeExt;
use crate::scheduler::ForegroundAssignments;
use crate::{configuration_files, Config, DISTRIBUTION_PATH, SYSTEM_CONF_PATH};
use ::kdl::KdlDocument;
use const_format::concatcp;

pub fn read_config() -> Config {
    let buffer = &mut String::with_capacity(4096);

    let mut config = read_assignments(read_main(buffer), buffer);

    let background = config
        .process_scheduler
        .assignments
        .profiles
        .remove("background");

    let foreground = config
        .process_scheduler
        .assignments
        .profiles
        .remove("foreground");

    if let (Some(background), Some(foreground)) = (background, foreground) {
        config.process_scheduler.foreground = Some(ForegroundAssignments {
            background,
            foreground,
        });
    }

    config.process_scheduler.pipewire = config
        .process_scheduler
        .assignments
        .profiles
        .remove("pipewire");

    config
}

fn read_main(buffer: &mut String) -> Config {
    const DIST_CONF: &str = concatcp!(DISTRIBUTION_PATH, "config.kdl");
    const SYSTEM_CONF: &str = concatcp!(SYSTEM_CONF_PATH, "config.kdl");

    let mut config = Config::default();

    let path = if Path::new(SYSTEM_CONF).exists() {
        SYSTEM_CONF
    } else if Path::new(DIST_CONF).exists() {
        DIST_CONF
    } else {
        return config;
    };

    let span = tracing::warn_span!("parser::read_main", path);
    let _entered = span.enter();

    let Ok(buffer) = crate::read_into_string(buffer, path) else {
        tracing::error!("failed to read file");
        return config;
    };

    let document = match buffer.parse::<KdlDocument>() {
        Ok(document) => document,
        Err(why) => {
            let offset = why.span.offset();

            let mut line_number = 1;

            let mut buffer = &buffer.as_bytes()[..offset];

            while let Some(pos) = memchr::memchr(b'\n', buffer) {
                line_number += 1;
                buffer = &buffer[pos + 1..];
            }

            tracing::error!("parsing error on line {}: {}", line_number, why);
            return config;
        }
    };

    for node in document.nodes() {
        match node.name().value() {
            "autogroup-enabled" => {
                config.autogroup_enabled = node.get_bool(0).unwrap_or(false);
            }
            "cfs-profiles" => config.cfs_profiles.read(node),
            "process-scheduler" => config.process_scheduler.read(node),
            other => {
                tracing::warn!("unknown element: {}", other);
            }
        }
    }

    config
}

fn read_assignments(mut config: Config, buffer: &mut String) -> Config {
    const PATHS: [&str; 2] = [
        concatcp!(DISTRIBUTION_PATH, "assignments/"),
        concatcp!(SYSTEM_CONF_PATH, "assignments/"),
    ];

    for path in configuration_files(&PATHS, ".kdl") {
        if !Path::new(&path).exists() {
            continue;
        }
        let span = tracing::warn_span!("parser::read_assignments", path);
        let _entered = span.enter();

        let Ok(buffer) = crate::read_into_string(buffer, &path) else {
            continue;
        };

        let document = match buffer.parse::<KdlDocument>() {
            Ok(document) => document,
            Err(why) => {
                tracing::error!("parsing error: {}", why);
                continue;
            }
        };

        for node in document.nodes() {
            match node.name().value() {
                "assignments" => {
                    config.process_scheduler.assignments.parse(node);
                }

                "exceptions" => {
                    config.process_scheduler.assignments.parse_exceptions(node);
                }

                other => {
                    tracing::warn!("unknown field: {}", other);
                }
            }
        }
    }

    config
}
