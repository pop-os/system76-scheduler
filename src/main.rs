// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate zbus;

mod config;
mod cpu;
mod dbus;
mod nice;
mod paths;
mod priority;
mod utils;

use crate::config::{cpu::Config as CpuConfig, Config};
use crate::paths::SchedPaths;
use crate::priority::{is_assignable, Priority};
use argh::FromArgs;
use config::IoPriority;
use dbus::{CpuMode, Server};
use std::{collections::HashMap, path::Path, time::Duration};
use tokio::sync::mpsc::Sender;
use upower_dbus::UPowerProxy;
use zbus::{Connection, PropertyStream};

#[derive(FromArgs, PartialEq, Debug)]
/// System76 Scheduler Tweaker
struct Args {
    #[argh(subcommand)]
    subcmd: SubCmd,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum SubCmd {
    Cpu(CpuArgs),
    Daemon(DaemonArgs),
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "cpu")]
/// Change the CPU scheduler configuration.
struct CpuArgs {
    #[argh(positional)]
    profile: Option<String>,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "daemon")]
#[allow(clippy::doc_markdown)]
/// Launch the DBus service.
struct DaemonArgs {}

#[derive(Debug)]
enum Event {
    OnBattery(bool),
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

    let args: Args = argh::from_env();

    match args.subcmd {
        SubCmd::Cpu(args) => cpu(connection, args).await,
        SubCmd::Daemon(_) => daemon(connection).await,
    }
}

async fn cpu(connection: Connection, args: CpuArgs) -> anyhow::Result<()> {
    let mut connection = dbus::ClientProxy::new(&connection).await?;

    match args.profile.as_ref() {
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
async fn daemon(connection: Connection) -> anyhow::Result<()> {
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

    let mut fg_processes: Vec<u32> = Vec::with_capacity(256);
    let mut process_map = HashMap::new();
    let mut foreground_process = None;

    let config = Config::read();
    let automatic_assignments = Config::automatic_assignments();

    let mut exe_buf = String::with_capacity(64);

    // Gets the config-assigned priority of a process.
    let mut assigned_priority = |pid: u32| -> Priority {
        if !is_assignable(pid) {
            return Priority::NotAssignable;
        }

        if let Some(exe) = exe_of_pid(&mut exe_buf, pid) {
            return automatic_assignments
                .get(exe)
                .map_or(Priority::Assignable, |v| {
                    Priority::Config((v.0.get().into(), v.1))
                });
        }

        Priority::NotAssignable
    };

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
            Event::UpdateProcessMap(map, background_processes) => {
                process_map = map;

                for pid in background_processes {
                    if fg_processes.contains(&pid) {
                        continue;
                    }

                    if let Some(priority) = assigned_priority(pid)
                        .with_default((i32::from(config.background.unwrap_or(0)), IoPriority::Idle))
                    {
                        crate::nice::set_priority(pid, priority);
                    }
                }

                // Reassign foreground processes.
                if let Some(process) = foreground_process.take() {
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        let _res = tx.send(Event::SetForegroundProcess(process)).await;
                    });
                }
            }

            Event::SetForegroundProcess(pid) => {
                if let Some(foreground_priority) = config.foreground {
                    foreground_process = Some(pid);
                    let background_priority =
                        (i32::from(config.background.unwrap_or(0)), IoPriority::Idle);

                    let foreground_priority =
                        (i32::from(foreground_priority), IoPriority::Standard);

                    for process in fg_processes.drain(..) {
                        if let Some(priority) =
                            assigned_priority(process).with_default(background_priority)
                        {
                            crate::nice::set_priority(process, priority);
                        }
                    }

                    if let Some(priority) = assigned_priority(pid).with_default(foreground_priority)
                    {
                        crate::nice::set_priority(pid, priority);
                        fg_processes.push(pid);
                    }

                    'outer: loop {
                        for (pid, parent) in &process_map {
                            if let Some(parent) = parent {
                                if fg_processes.contains(parent) && !fg_processes.contains(pid) {
                                    if let Some(priority) =
                                        assigned_priority(*pid).with_default(foreground_priority)
                                    {
                                        crate::nice::set_priority(*pid, priority);
                                        fg_processes.push(*pid);
                                        continue 'outer;
                                    }
                                }
                            }
                        }

                        break;
                    }
                }
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

fn exe_of_pid(buf: &mut String, pid: u32) -> Option<&str> {
    let mut itoa = itoa::Buffer::new();
    let exe = concat_in_place::strcat!("/proc/" itoa.format(pid) "/exe");
    let exe_path = Path::new(&exe);

    if let Ok(exe) = std::fs::read_link(exe_path) {
        if let Some(exe) = exe.file_name().and_then(std::ffi::OsStr::to_str) {
            buf.clear();
            buf.push_str(exe);
            return Some(&*buf);
        }
    }

    None
}
