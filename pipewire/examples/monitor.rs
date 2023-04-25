use std::os::unix::{net::UnixStream, prelude::OwnedFd};

use system76_scheduler_pipewire::processes_from_socket;

fn main() {
    let (tx, rx) = std::sync::mpsc::sync_channel(0);

    std::thread::spawn(move || {
        let file = UnixStream::connect("/run/user/1000/pipewire-0").unwrap();

        processes_from_socket(&OwnedFd::from(file), move |event| {
            let _res = tx.send(event);
        });
    });

    while let Ok(event) = rx.recv() {
        println!("{:#?}", event);
    }
}
