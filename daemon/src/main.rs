// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#![deny(missing_docs)]

//! System76 Scheduler

#[macro_use]
extern crate zbus;

use qcell::LCellOwner;
pub use system76_scheduler_config as config;
use system76_scheduler_pipewire as scheduler_pipewire;

mod cfs;
mod dbus;
mod priority;
mod process;
mod pw;
mod service;
mod utils;

use clap::ArgMatches;
use dbus::{CpuMode, Server};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;
use upower_dbus::UPowerProxy;
use zbus::{Connection, PropertyStream};

use crate::utils::Buffer;

#[derive(Debug)]
enum Event {
    ExecCreate(ExecCreate),
    OnBattery(bool),
    Pipewire(scheduler_pipewire::ProcessEvent),
    RefreshProcessMap,
    ReloadConfiguration,
    SetCpuMode,
    SetCustomCpuMode,
    SetForegroundProcess(u32),
}

#[derive(Debug)]
struct ExecCreate {
    pid: u32,
    parent_pid: u32,
    name: String,
    cmdline: String,
}

fn main() -> anyhow::Result<()> {
    let mut result = Ok(());

    LCellOwner::scope(|owner| {
        pipewire::init();

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let main = async {
            let future = async {
                if std::env::var_os("RUST_LOG").is_none() {
                    std::env::set_var("RUST_LOG", "info");
                }

                tracing_subscriber::fmt()
                    .pretty()
                    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
                    .with_writer(std::io::stderr)
                    .without_time()
                    .with_line_number(false)
                    .with_file(false)
                    .with_target(false)
                    .init();

                let connection = Connection::system().await?;

                let matches = clap::command!()
                    .propagate_version(true)
                    .subcommand_required(true)
                    .arg_required_else_help(true)
                    .subcommand(
                        clap::Command::new("cpu")
                            .about("select a CFS scheduler profile")
                            .arg(clap::arg!([PROFILE])),
                    )
                    .subcommand(
                        clap::Command::new("daemon")
                            .about("launch the system daemon")
                            .subcommand(
                                clap::Command::new("reload").about("reload system configuration"),
                            ),
                    )
                    .subcommand(
                        clap::Command::new("pipewire")
                            .about("monitor pipewire process ID activities"),
                    )
                    .get_matches();

                match matches.subcommand() {
                    Some(("cpu", matches)) => cpu(connection, matches).await,
                    Some(("daemon", matches)) => daemon(connection, matches, owner).await,
                    Some(("pipewire", _matches)) => pw::main().await,
                    _ => Ok(()),
                }
            };

            result = tokio::task::LocalSet::new().run_until(future).await;

            unsafe {
                pipewire::deinit();
            }
        };

        runtime.block_on(main);
    });

    result
}

async fn reload(connection: Connection) -> anyhow::Result<()> {
    dbus::ClientProxy::new(&connection)
        .await?
        .reload_configuration()
        .await?;

    Ok(())
}

async fn cpu(connection: Connection, args: &ArgMatches) -> anyhow::Result<()> {
    let mut connection = dbus::ClientProxy::new(&connection).await?;

    match args.get_one::<&str>("PROFILE") {
        Some(profile) => {
            connection.set_cpu_profile(profile).await?;
        }
        None => {
            println!("{}", connection.cpu_profile().await?);
        }
    }

    Ok(())
}

#[allow(clippy::too_many_lines)]
async fn daemon(
    connection: Connection,
    args: &ArgMatches,
    owner: LCellOwner<'_>,
) -> anyhow::Result<()> {
    let mut buffer = Buffer::new();

    if let Some(("reload", _)) = args.subcommand() {
        return reload(connection).await;
    }

    let service = &mut service::Service::new(owner);
    service.reload_configuration();

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    let upower = UPowerProxy::new(&connection).await?;

    // Spawns an async task that watches for battery status notifications.
    tokio::task::spawn_local(battery_monitor(
        upower.receive_on_battery_changed().await,
        tx.clone(),
    ));

    // Controls the kernel's sched_autogroup setting.
    autogroup_set(service.config.autogroup_enabled);

    // Tweaks CFS parameters based on battery status.
    if service.config.cfs_profiles.enable {
        service.cfs_on_battery(upower.on_battery().await.unwrap_or(false));
    }

    // If enabled, monitors processes and applies priorities to them.
    if service.config.process_scheduler.enable {
        // Schedules process updates
        tokio::task::spawn_local({
            let refresh_rate =
                Duration::from_secs(u64::from(service.config.process_scheduler.refresh_rate));
            let tx = tx.clone();
            async move {
                let _res = tx.send(Event::RefreshProcessMap).await;
                tokio::time::sleep(refresh_rate).await;
            }
        });

        // Use execsnoop-bpfcc to watch for new processes being created.
        if service.config.process_scheduler.execsnoop {
            tracing::debug!("monitoring process IDs in realtime with execsnoop");
            let tx = tx.clone();
            let (scheduled_tx, mut scheduled_rx) = tokio::sync::mpsc::unbounded_channel();
            std::thread::spawn(move || {
                match execsnoop::watch() {
                    Ok(mut watcher) => {
                        // Listen for spawned process, scheduling them to be handled with a delay of 1 second after creation.
                        // The delay is to ensure that a process has been added to a cgroup
                        while let Some(process) = watcher.next() {
                            let Ok(cmdline) = std::str::from_utf8(process.cmd) else {
                                continue
                            };

                            let name = process::name(cmdline);

                            tracing::debug!("{:?} created by {:?} ({name})", process.pid, process.parent_pid);
                            let _res = scheduled_tx.send((
                                Instant::now() + Duration::from_secs(2),
                                ExecCreate {
                                    pid: process.pid,
                                    parent_pid: process.parent_pid,
                                    name: name.to_owned(),
                                    cmdline: cmdline.to_owned(),
                                },
                            ));
                        }
                    },
                    Err(error) => {
                        tracing::error!("failed to start execsnoop: {error}");
                    }
                }
            });

            tokio::task::spawn_local(async move {
                while let Some((delay, process)) = scheduled_rx.recv().await {
                    tokio::time::sleep_until(delay.into()).await;
                    let _res = tx.send(Event::ExecCreate(process)).await;
                }
            });
        }

        // Monitors pipewire-connected processes.
        if service.config.process_scheduler.pipewire.is_some() {
            tokio::task::spawn_local(pw::monitor(tx.clone()));
        }
    }

    connection
        .object_server()
        .at(
            "/com/system76/Scheduler",
            Server {
                cpu_mode: CpuMode::Auto,
                cpu_profile: String::from("auto"),
                tx: tx.clone(),
            },
        )
        .await?;

    connection.request_name("com.system76.Scheduler").await?;

    // Start service after system uptime is above 10 seconds
    if let Some(uptime) = uptime() {
        if uptime < 10 {
            std::thread::sleep(std::time::Duration::from_secs(10 - uptime));
        }
    }

    while let Some(event) = rx.recv().await {
        match event {
            Event::ExecCreate(ExecCreate {
                pid,
                parent_pid,
                name,
                cmdline,
            }) => {
                service.assign_new_process(&mut buffer, pid, parent_pid, name, cmdline);
                service.assign_children(&mut buffer, pid);
            }

            Event::RefreshProcessMap => {
                service.process_map_refresh(&mut buffer);
            }

            Event::SetForegroundProcess(pid) => {
                tracing::debug!("setting {pid} as foreground process");
                service.set_foreground_process(&mut buffer, pid);
            }

            Event::Pipewire(scheduler_pipewire::ProcessEvent::Add(process)) => {
                service.set_pipewire_process(&mut buffer, process);
            }

            Event::Pipewire(scheduler_pipewire::ProcessEvent::Remove(process)) => {
                service.remove_pipewire_process(&mut buffer, process);
            }

            Event::OnBattery(on_battery) => {
                let Some(handle) = dbus::interface_handle(&connection).await else {
                    break;
                };

                let interface = handle.get().await;

                if let CpuMode::Auto = interface.cpu_mode {
                    service.cfs_on_battery(on_battery);
                }
            }

            Event::SetCpuMode => {
                let Some(handle) = dbus::interface_handle(&connection).await else {
                    break;
                };

                let interface = handle.get().await;

                match interface.cpu_mode {
                    CpuMode::Auto => {
                        tracing::debug!("applying auto config");
                        service.cfs_on_battery(upower.on_battery().await.unwrap_or(false));
                    }

                    CpuMode::Default => {
                        tracing::debug!("applying default config");
                        service.cfs_apply(service.cfs_default_config());
                    }

                    CpuMode::Responsive => {
                        tracing::debug!("applying responsive config");
                        service.cfs_apply(service.cfs_responsive_config());
                    }

                    CpuMode::Custom => (),
                }
            }

            Event::SetCustomCpuMode => {
                let Some(handle) = dbus::interface_handle(&connection).await else {
                    break;
                };

                let interface = handle.get().await;

                if let Some(profile) = service.cfs_config(&interface.cpu_profile) {
                    tracing::debug!("applying {} config", interface.cpu_profile);
                    service.cfs_apply(profile);
                }
            }

            Event::ReloadConfiguration => {
                tracing::debug!("reloading configuration");
                service.reload_configuration();
                autogroup_set(service.config.autogroup_enabled);
            }
        }
    }
    Ok(())
}

async fn battery_monitor(mut events: PropertyStream<'_, bool>, tx: Sender<Event>) {
    use futures::StreamExt;

    tracing::debug!("monitoring udev for battery hotplug events");
    while let Some(event) = events.next().await {
        if let Ok(on_battery) = event.get().await {
            let _res = tx.send(Event::OnBattery(on_battery)).await;
        }
    }
}

fn autogroup_set(enable: bool) {
    const PATH: &str = "/proc/sys/kernel/sched_autogroup_enabled";
    let _res = std::fs::write(PATH, if enable { b"1" } else { b"0" });
}

fn uptime() -> Option<u64> {
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let seconds = uptime.split('.').next()?;
    seconds.parse::<u64>().ok()
}
