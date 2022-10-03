// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate const_format;
#[macro_use]
extern crate next_gen;
#[macro_use]
extern crate zbus;

mod config;
mod cpu;
mod dbus;
mod nice;
mod paths;
mod priority;
mod utils;

use crate::config::cpu::Config as CpuConfig;
use crate::paths::SchedPaths;
use clap::ArgMatches;
use dbus::{CpuMode, Server};
use std::{collections::HashMap, path::Path, time::Duration};
use tokio::sync::mpsc::Sender;
use upower_dbus::UPowerProxy;
use zbus::{Connection, PropertyStream};

#[derive(Debug)]
enum Event {
    ExecCreate(execsnoop::Process),
    OnBattery(bool),
    ReloadConfiguration,
    SetCpuMode,
    SetCustomCpuMode,
    SetForegroundProcess(u32),
    UpdateProcessMap(HashMap<u32, Option<u32>>, Vec<u32>),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .without_time()
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
                .subcommand(clap::Command::new("reload").about("reload system configuration")),
        )
        .get_matches();

    match matches.subcommand() {
        Some(("cpu", matches)) => cpu(connection, matches).await,
        Some(("daemon", matches)) => daemon(connection, matches).await,
        _ => Ok(()),
    }
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
async fn daemon(connection: Connection, args: &ArgMatches) -> anyhow::Result<()> {
    match args.subcommand() {
        Some(("reload", _)) => return reload(connection).await,
        _ => (),
    }

    tracing::info!("starting daemon service");

    let paths = SchedPaths::new()?;

    let upower_proxy = UPowerProxy::new(&connection).await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

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

    // Spawns an async task that watches for battery status notifications.
    tokio::spawn(battery_monitor(
        upower_proxy.receive_on_battery_changed().await,
        tx.clone(),
    ));

    // Spawns a process monitor that updates process map info.
    tokio::spawn(process_monitor(tx.clone()));

    // Use execsnoop-bpfcc to watch for new processes being created.
    if let Ok(watcher) = execsnoop::watch() {
        let handle = tokio::runtime::Handle::current();
        let tx = tx.clone();
        std::thread::spawn(move || {
            handle.block_on(async move {
                for process in watcher {
                    let _res = tx.send(Event::ExecCreate(process)).await;
                }
            });
        });
    }

    let apply_config = |on_battery: bool| {
        cpu::tweak(
            &paths,
            &if on_battery {
                tracing::debug!("auto config applying default config");
                CpuConfig::default_config()
            } else {
                tracing::debug!("auto config applying responsive config");
                CpuConfig::responsive_config()
            },
        );
    };

    apply_config(upower_proxy.on_battery().await.unwrap_or(false));

    let mut priority_service = priority::Service::default();
    priority_service.reload_configuration();

    while let Some(event) = rx.recv().await {
        let interface_result = connection
            .object_server()
            .interface::<_, Server>("/com/system76/Scheduler")
            .await;

        let iface_handle = match interface_result {
            Ok(iface_handler) => iface_handler,
            Err(why) => {
                tracing::error!("DBus interface not reachable: {:#?}", why);
                break;
            }
        };

        let interface = iface_handle.get().await;

        match event {
            Event::ExecCreate(execsnoop::Process {
                parent_pid, pid, ..
            }) => {
                priority_service.assign_new_process(pid, parent_pid);
            }

            Event::UpdateProcessMap(map, background_processes) => {
                priority_service.update_process_map(map, background_processes);
            }

            Event::SetForegroundProcess(pid) => {
                priority_service.set_foreground_process(pid);
            }

            Event::OnBattery(on_battery) => {
                if let CpuMode::Auto = interface.cpu_mode {
                    apply_config(on_battery);
                }
            }

            Event::SetCpuMode => match interface.cpu_mode {
                CpuMode::Auto => {
                    tracing::debug!("applying auto config");
                    apply_config(upower_proxy.on_battery().await.unwrap_or(false));
                }

                CpuMode::Default => {
                    tracing::debug!("applying default config");
                    cpu::tweak(&paths, &CpuConfig::default_config());
                }

                CpuMode::Responsive => {
                    tracing::debug!("applying responsive config");
                    cpu::tweak(&paths, &CpuConfig::responsive_config());
                }

                CpuMode::Custom => (),
            },

            Event::SetCustomCpuMode => {
                if let Some(config) = CpuConfig::custom_config(&interface.cpu_profile) {
                    tracing::debug!("applying {} config", interface.cpu_profile);
                    cpu::tweak(&paths, &config);
                }
            }

            Event::ReloadConfiguration => {
                priority_service.reload_configuration();
            }
        }
    }
    Ok(())
}

async fn battery_monitor(mut events: PropertyStream<'_, bool>, tx: Sender<Event>) {
    use futures::StreamExt;
    while let Some(event) = events.next().await {
        if let Ok(on_battery) = event.get().await {
            let _res = tx.send(Event::OnBattery(on_battery)).await;
        }
    }
}

async fn process_monitor(tx: Sender<Event>) {
    let mut file_buf = String::with_capacity(2048);

    let mut background_processes = Vec::with_capacity(256);
    let mut parents = HashMap::<u32, Option<u32>>::with_capacity(256);

    loop {
        if let Ok(procfs) = Path::new("/proc").read_dir() {
            background_processes.clear();
            parents.clear();

            for proc_entry in procfs.filter_map(Result::ok) {
                let proc_path = proc_entry.path();

                let pid = if let Some(pid) = proc_path
                    .file_name()
                    .and_then(std::ffi::OsStr::to_str)
                    .and_then(|p| p.parse::<u32>().ok())
                {
                    pid
                } else {
                    continue;
                };

                let mut parent = None;

                let status = proc_path.join("status");

                if let Ok(status) = crate::utils::read_into_string(&mut file_buf, &*status) {
                    for line in status.lines() {
                        if let Some(ppid) = line.strip_prefix("PPid:") {
                            if let Ok(ppid) = ppid.trim().parse::<u32>() {
                                parent = Some(ppid);
                            }

                            break;
                        }
                    }
                }

                parents.insert(pid, parent);

                // Prevents kernel processes from having their priorities changed.
                if let Ok(exe) = std::fs::read_link(proc_path.join("exe")) {
                    if exe.file_name().is_some() {
                        background_processes.push(pid);
                    }
                }
            }

            let _res = tx
                .send(Event::UpdateProcessMap(
                    parents.clone(),
                    background_processes.clone(),
                ))
                .await;
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
