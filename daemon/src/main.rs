// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate zbus;

use concat_in_place::strcat;
use scheduler_pipewire::processes_from_socket;
pub use system76_scheduler_config as config;
use system76_scheduler_pipewire as scheduler_pipewire;

mod cfs;
mod dbus;
mod pid;
mod priority;
mod service;
mod utils;

use cfs::paths::SchedPaths;
use clap::ArgMatches;
use dbus::{CpuMode, Server};
use std::{
    collections::{BTreeSet, HashMap},
    os::unix::{
        net::UnixStream,
        prelude::{OsStrExt, OwnedFd},
    },
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::mpsc::Sender;
use upower_dbus::UPowerProxy;
use zbus::{Connection, PropertyStream};

use crate::utils::Buffer;

#[derive(Debug)]
enum Event {
    ExecCreate(Instant, execsnoop::Process),
    OnBattery(bool),
    Pipewire(scheduler_pipewire::ProcessEvent),
    ReloadConfiguration,
    SetCpuMode,
    SetCustomCpuMode,
    SetForegroundProcess(u32),
    UpdateProcessMap(HashMap<u32, Option<u32>>, Vec<u32>),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    pipewire::init();

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
                    .subcommand(clap::Command::new("reload").about("reload system configuration")),
            )
            .get_matches();

        match matches.subcommand() {
            Some(("cpu", matches)) => cpu(connection, matches).await,
            Some(("daemon", matches)) => daemon(connection, matches).await,
            _ => Ok(()),
        }
    };

    let result = tokio::task::LocalSet::new().run_until(future).await;

    unsafe {
        pipewire::deinit();
    }

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
async fn daemon(connection: Connection, args: &ArgMatches) -> anyhow::Result<()> {
    let mut buffer = Buffer::new();

    if let Some(("reload", _)) = args.subcommand() {
        return reload(connection).await;
    }

    let service = &mut service::Service::new(SchedPaths::new()?);
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
        // Spawns a process monitor that updates process map info.
        tokio::task::spawn_local(process_monitor(
            tx.clone(),
            service.config.process_scheduler.refresh_rate,
        ));

        // Use execsnoop-bpfcc to watch for new processes being created.
        if service.config.process_scheduler.execsnoop {
            tracing::debug!("monitoring process IDs in realtime with execsnoop");
            if let Ok(watcher) = execsnoop::watch() {
                let tx = tx.clone();

                // Listen for spawned process, scheduling them to be handled with a delay of 1 second after creation.
                // The delay is to ensure that a process has been added to a cgroup
                std::thread::spawn(move || {
                    for process in watcher {
                        let _res = tx.blocking_send(Event::ExecCreate(
                            Instant::now() + Duration::from_secs(1),
                            process,
                        ));
                    }
                });
            }
        }

        // Monitors pipewire-connected processes.
        if service.config.process_scheduler.pipewire.is_some() {
            tokio::task::spawn_local(pipewire_service(tx.clone()));
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

    while let Some(event) = rx.recv().await {
        match event {
            Event::ExecCreate(
                when,
                execsnoop::Process {
                    parent_pid, pid, ..
                },
            ) => {
                tokio::time::sleep_until(tokio::time::Instant::from_std(when)).await;
                service.assign_new_process(&mut buffer, pid, parent_pid);
            }

            Event::UpdateProcessMap(map, background_processes) => {
                service.update_process_map(&mut buffer, map, background_processes);
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

/// Monitor pipewire sockets and the process IDs connected to them.
async fn pipewire_service(tx: Sender<Event>) {
    // TODO: Support stopping and restarting this on config changes.
    enum SocketEvent {
        Add(PathBuf),
        Remove(PathBuf),
    }

    tracing::debug!("monitoring pipewire process IDs");

    let (pw_tx, mut pw_rx) = tokio::sync::mpsc::channel(1);

    let session_monitor = {
        let pw_tx = pw_tx.clone();
        async move {
            loop {
                if let Ok(run_user_dir) = std::fs::read_dir("/run/user") {
                    for entry in run_user_dir.filter_map(Result::ok) {
                        let socket_path = entry.path().join("pipewire-0");
                        if socket_path.exists() {
                            let _res = pw_tx.send(SocketEvent::Add(socket_path)).await;
                        }
                    }
                }

                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        }
    };

    let session_spawner = async move {
        let mut active_sessions = BTreeSet::<PathBuf>::new();

        while let Some(event) = pw_rx.recv().await {
            match event {
                SocketEvent::Add(socket) => {
                    if !active_sessions.contains(&socket) {
                        if let Ok(stream) = UnixStream::connect(&socket) {
                            let tx = tx.clone();
                            let pw_tx = pw_tx.clone();
                            std::thread::spawn(move || {
                                processes_from_socket(&OwnedFd::from(stream), move |event| {
                                    let _res = tx.blocking_send(Event::Pipewire(event));
                                });

                                let _res = pw_tx.blocking_send(SocketEvent::Remove(socket));
                            });
                        }
                    }
                }
                SocketEvent::Remove(socket) => {
                    active_sessions.remove(&socket);
                }
            }
        }
    };

    futures::join!(session_monitor, session_spawner);
}

async fn process_monitor(tx: Sender<Event>, refresh_rate: u16) {
    tracing::debug!("monitoring system process IDs");
    let mut file_buf = Vec::with_capacity(2048);

    let mut background_processes = Vec::with_capacity(256);
    let mut parents = HashMap::<u32, Option<u32>>::with_capacity(256);

    let mut path = String::from("/proc/");
    let proc_truncate = path.len();

    loop {
        path.truncate(proc_truncate);
        let Ok(procfs) = std::fs::read_dir(&path) else {
            tracing::error!("failed to read /proc directory: process monitoring stopped");
            return;
        };

        background_processes.clear();
        parents.clear();

        for proc_entry in procfs.filter_map(Result::ok) {
            let file_name = proc_entry.file_name();

            let Some(pid) = atoi::atoi::<u32>(file_name.as_bytes()) else {
                continue;
            };

            let Some(file_name) = file_name.to_str() else {
                continue;
            };

            path.truncate(proc_truncate);

            strcat!(&mut path, file_name "/status");

            let mut parent = None;

            if let Some(value) = utils::file_key(&mut file_buf, &path, "PPid:") {
                if let Some(ppid) = atoi::atoi::<u32>(value) {
                    parent = Some(ppid);
                }
            }

            parents.insert(pid, parent);

            path.truncate(proc_truncate + file_name.len() + 1);
            path.push_str("exe");

            // Prevents kernel processes from having their priorities changed.
            if let Ok(exe) = std::fs::read_link(&path) {
                if exe.file_name().is_some() {
                    background_processes.push(pid);
                }
            }
        }

        tokio::time::sleep(Duration::from_secs(1)).await;

        let _res = tx
            .send(Event::UpdateProcessMap(
                parents.clone(),
                background_processes.clone(),
            ))
            .await;

        tokio::time::sleep(Duration::from_secs(u64::from(refresh_rate))).await;
    }
}

fn autogroup_set(enable: bool) {
    const PATH: &str = "/proc/sys/kernel/sched_autogroup_enabled";
    let _res = std::fs::write(PATH, if enable { b"1" } else { b"0" });
}
