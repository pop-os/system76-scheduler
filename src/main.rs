// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

#[macro_use]
extern crate zbus;

mod config;
mod cpu;
mod dbus;
mod paths;

use crate::config::Config;
use crate::paths::SchedPaths;
use argh::FromArgs;
use dbus::{CpuMode, Server};
use postage::prelude::*;
use zbus::Connection;

enum Event {
    SetCpuMode(CpuMode),
    SetCustomCpuMode(String),
    OnBattery(bool),
}

fn main() -> anyhow::Result<()> {
    futures::executor::block_on(async move {
        let connection = Connection::system().await?;

        let args: Args = argh::from_env();

        match args.subcmd {
            SubCmd::Cpu(args) => cpu(connection, args).await,
            SubCmd::Daemon(_) => daemon(connection).await,
        }
    })
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

    let upower_proxy = upower_dbus::UPowerProxy::new(&connection).await?;

    let (mut tx, mut rx) = postage::mpsc::channel(1);

    let mut cpu_mode = CpuMode::Auto;

    connection
        .object_server()
        .at(
            "/com/system76/Scheduler",
            Server {
                cpu_mode,
                cpu_profile: String::from("auto"),
                tx: tx.clone(),
            },
        )
        .await?;

    connection.request_name("com.system76.Scheduler").await?;

    let mut events = upower_proxy.receive_on_battery_changed().await;

    let battery_service = async move {
        eprintln!("starting battery watch service");

        use futures::StreamExt;
        while let Some(event) = events.next().await {
            if let Ok(on_battery) = event.get().await {
                let _ = tx.send(Event::OnBattery(on_battery)).await;
            }
        }
    };

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
        while let Some(event) = rx.recv().await {
            match event {
                Event::OnBattery(on_battery) => {
                    if let CpuMode::Auto = cpu_mode {
                        apply_config(on_battery);
                    }
                }

                Event::SetCpuMode(new_cpu_mode) => {
                    cpu_mode = new_cpu_mode;
                    match cpu_mode {
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
                    }
                }

                Event::SetCustomCpuMode(profile) => {
                    if let Some(config) = Config::custom_config(&profile) {
                        cpu_mode = CpuMode::Custom;
                        eprintln!("applying {} config", profile);
                        cpu::tweak(&paths, &config);
                    }
                }
            }
        }
    };

    let _ = futures::join!(event_handler, battery_service);

    Ok(())
}

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
