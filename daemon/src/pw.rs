use crate::Event;
use std::{
    collections::BTreeSet,
    io::Write,
    os::unix::{net::UnixStream, prelude::OwnedFd},
    path::PathBuf,
    time::Duration,
};
use system76_scheduler_pipewire::{processes_from_socket, ProcessEvent};
use tokio::{io::AsyncBufReadExt, sync::mpsc::Sender};

pub async fn main() -> anyhow::Result<()> {
    pipewire::init();
    let (tx, mut rx) = tokio::sync::mpsc::channel(4);

    let service = async move {
        pipewire_service(tx).await;
        anyhow::bail!("pipewire service exited")
    };

    let main = async move {
        let stdout = &mut std::io::stdout().lock();

        while let Some(event) = rx.recv().await {
            event.to_bytes(stdout)?;
            stdout.write_all(b"\n")?;
        }

        Ok(())
    };

    let result = futures_lite::future::race(service, main).await;

    unsafe {
        pipewire::deinit();
    }

    result
}

/// Monitor pipewire sockets and the process IDs connected to them.
async fn pipewire_service(tx: Sender<ProcessEvent>) {
    // TODO: Support stopping and restarting this on config changes.
    enum SocketEvent {
        Add(PathBuf),
        Remove(PathBuf),
    }

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
                                    let _res = tx.blocking_send(event);
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

    futures_lite::future::zip(session_monitor, session_spawner).await;
}

/// Spawns and manages a child process that monitors pipewire events from the pipewire subcommand.
///
/// This is done to isolate libpipewire from the daemon. If a crash occurs from the pipewire-rs bindings,
/// or the libpipewire library itelf, this will gracefully restart the process without losing any data.
pub(crate) async fn monitor(tx: Sender<Event>) {
    let mut managed = BTreeSet::<u32>::new();

    loop {
        tokio::time::sleep(Duration::from_secs(3)).await;

        let result = std::process::Command::new("system76-scheduler")
            .arg("pipewire")
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .spawn();

        let Ok(mut child) = result else {
            tracing::error!("failed to spawn pipewire watcher: {:?}", result.err());
            continue;
        };

        let Some(stdout) = child.stdout.take() else {
            continue;
        };

        let Ok(stdout) = tokio::process::ChildStdout::from_std(stdout) else {
            continue;
        };

        let mut stdout = tokio::io::BufReader::new(stdout);
        let mut line = Vec::new();

        loop {
            line.clear();

            match stdout.read_until(b'\n', &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => (),
            }

            if let Some(event) = ProcessEvent::from_bytes(&line) {
                match event {
                    ProcessEvent::Add(pid) => {
                        if !managed.insert(pid) {
                            continue;
                        }
                    }
                    ProcessEvent::Remove(pid) => {
                        if !managed.remove(&pid) {
                            continue;
                        }
                    }
                }

                let _res = tx.send(Event::Pipewire(event)).await;
            }
        }
    }
}
