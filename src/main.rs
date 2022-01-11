// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate zbus;

mod config;
mod cpu;
mod dbus;
mod nice;
mod paths;

use std::{path::Path, time::Duration};

use crate::config::Config;
use crate::paths::SchedPaths;
use argh::FromArgs;
use dbus::{CpuMode, Server};
use postage::mpsc::Sender;
use postage::prelude::*;
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
/// Launch the DBus service.
struct DaemonArgs {}

enum Event {
    SetAutoBackgroundPriority(u32),
    SetCpuMode,
    SetCustomCpuMode,
    SetForegroundProcess(u32),
    OnBattery(bool),
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
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
        Some(profile) => connection.set_cpu_profile(&profile).await?,
        None => println!("{}", connection.cpu_profile().await?),
    }

    Ok(())
}

async fn daemon(connection: Connection) -> anyhow::Result<()> {
    let paths = SchedPaths::new()?;

    let upower_proxy = UPowerProxy::new(&connection).await?;

    let (tx, mut rx) = postage::mpsc::channel(1);

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

    let battery_service =
        battery_monitor(upower_proxy.receive_on_battery_changed().await, tx.clone());

    let apply_config = |on_battery: bool| {
        cpu::tweak(
            &paths,
            &if on_battery {
                eprintln!("auto config applying default config");
                Config::default_config()
            } else {
                eprintln!("auto config applying responsive config");
                Config::responsive_config()
            },
        );
    };

    apply_config(upower_proxy.on_battery().await.unwrap_or(false));

    let event_handler = async {
        eprintln!("starting event handler");

        let mut foreground_process = None;

        while let Some(event) = rx.recv().await {
            let interface_result = connection
                .object_server()
                .interface::<_, Server>("/com/system76/Scheduler")
                .await;

            let iface_handle = match interface_result {
                Ok(iface_handler) => iface_handler,
                Err(why) => {
                    eprintln!("DBus interface not reachable: {:#?}", why);
                    break;
                }
            };

            let interface = iface_handle.get().await;

            match event {
                Event::SetAutoBackgroundPriority(pid) => {
                    if let Some(current) = foreground_process.as_ref() {
                        if *current == pid {
                            continue;
                        }
                    }

                    let current = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) };

                    // Only change priority of a process which has an unset priority
                    if current == 0 || current == -5 || current == 5 {
                        crate::nice::set_priority(pid, 5);
                    }
                }

                Event::SetForegroundProcess(pid) => {
                    if let Some(prev) = foreground_process.take() {
                        crate::nice::set_priority(prev, 5);
                        crate::nice::set_priority(pid, -5);
                    } else {
                        tokio::spawn(process_monitor(tx.clone(), pid));
                    }

                    foreground_process = Some(pid);
                }

                Event::OnBattery(on_battery) => {
                    if let CpuMode::Auto = interface.cpu_mode {
                        apply_config(on_battery);
                    }
                }

                Event::SetCpuMode => match interface.cpu_mode {
                    CpuMode::Auto => {
                        eprintln!("applying auto config");
                        apply_config(upower_proxy.on_battery().await.unwrap_or(false));
                    }

                    CpuMode::Default => {
                        eprintln!("applying default config");
                        cpu::tweak(&paths, &Config::default_config());
                    }

                    CpuMode::Responsive => {
                        eprintln!("applying responsive config");
                        cpu::tweak(&paths, &Config::responsive_config());
                    }

                    _ => (),
                },

                Event::SetCustomCpuMode => {
                    if let Some(config) = Config::custom_config(&interface.cpu_profile) {
                        eprintln!("applying {} config", interface.cpu_profile);
                        cpu::tweak(&paths, &config);
                    }
                }
            }
        }
    };

    let _ = futures::join!(event_handler, battery_service);

    Ok(())
}

async fn battery_monitor(mut events: PropertyStream<'_, bool>, mut tx: Sender<Event>) {
    eprintln!("starting battery watch service");
    use futures::StreamExt;
    while let Some(event) = events.next().await {
        if let Ok(on_battery) = event.get().await {
            let _ = tx.send(Event::OnBattery(on_battery)).await;
        }
    }
}

// Important desktop processes.
const HIGH_PRIORITY: &[&str] = &["gnome-shell", "kwin", "Xorg"];

// Typically common compiler processes
const LOW_PRIORITY: &[&str] = &[
    "bash",
    "c++",
    "cargo",
    "clang",
    "cpp",
    "g++",
    "gcc",
    "lld",
    "make",
    "rust-analyzer",
    "rustc",
    "sh",
];

async fn process_monitor(mut tx: Sender<Event>, foreground: u32) {
    let mut initial = Some(foreground);

    loop {
        if let Ok(procfs) = Path::new("/proc").read_dir() {
            for proc_entry in procfs.filter_map(Result::ok) {
                let proc_path = proc_entry.path();

                let pid = if let Some(pid) = proc_path
                    .file_name()
                    .and_then(|p| p.to_str())
                    .and_then(|p| p.parse::<u32>().ok())
                {
                    pid
                } else {
                    continue;
                };

                // Prevents kernel processes from having their priorities changed.
                if let Ok(exe) = proc_path.join("exe").canonicalize() {
                    if let Some(exe) = exe.file_name().and_then(|x| x.to_str()) {
                        if HIGH_PRIORITY.contains(&exe) {
                            // Automatically raise priority for processes that are important.
                            crate::nice::set_priority(pid, -5);
                            continue;
                        } else if LOW_PRIORITY.contains(&exe) {
                            // Automatically lower priority for known background processes.
                            crate::nice::set_priority(pid, 15);
                            continue;
                        }
                    }

                    let _ = tx.send(Event::SetAutoBackgroundPriority(pid)).await;
                }
            }
        }

        if let Some(pid) = initial.take() {
            crate::nice::set_priority(pid, -5);
        }

        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}
