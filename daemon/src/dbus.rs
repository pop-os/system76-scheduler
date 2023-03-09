// Copyright 2021-2022 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use crate::Event;
use serde_repr::{Deserialize_repr, Serialize_repr};
use tokio::sync::mpsc::Sender;
use zvariant::{OwnedValue, Type, Value};

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    Deserialize_repr,
    Serialize_repr,
    Value,
    OwnedValue,
    Type,
)]
#[repr(u8)]
pub enum CpuMode {
    Auto = 0,
    Custom = 1,
    Default = 2,
    Responsive = 3,
}

pub(crate) struct Server {
    pub cpu_mode: CpuMode,
    pub cpu_profile: String,
    pub tx: Sender<Event>,
}

#[dbus_proxy(
    default_service = "com.system76.Scheduler",
    interface = "com.system76.Scheduler",
    default_path = "/com/system76/Scheduler"
)]
pub trait Client {
    #[dbus_proxy(property)]
    fn cpu_mode(&self) -> zbus::fdo::Result<CpuMode>;

    #[dbus_proxy(property)]
    fn cpu_profile(&self) -> zbus::fdo::Result<String>;

    fn reload_configuration(&self) -> zbus::fdo::Result<()>;

    fn set_cpu_mode(&mut self, cpu_mode: CpuMode) -> zbus::fdo::Result<()>;

    fn set_cpu_profile(&mut self, profile: &str) -> zbus::fdo::Result<()>;

    /// This process will have its process group prioritized over background processes
    fn set_foreground_process(&mut self, pid: u32) -> zbus::fdo::Result<()>;
}

#[dbus_interface(name = "com.system76.Scheduler")]
impl Server {
    #[dbus_interface(property)]
    fn cpu_mode(&self) -> CpuMode {
        self.cpu_mode
    }

    #[dbus_interface(property)]
    fn cpu_profile(&self) -> &str {
        &self.cpu_profile
    }

    async fn reload_configuration(&self) {
        let _res = self.tx.send(Event::ReloadConfiguration).await;
    }

    async fn set_cpu_mode(&mut self, cpu_mode: CpuMode) {
        self.cpu_mode = cpu_mode;

        let _res = self.tx.send(Event::SetCpuMode).await;
    }

    async fn set_cpu_profile(&mut self, profile: String) {
        self.cpu_profile = profile.clone();
        match profile.as_str() {
            "auto" => self.set_cpu_mode(CpuMode::Auto).await,
            "default" => self.set_cpu_mode(CpuMode::Default).await,
            "responsive" => self.set_cpu_mode(CpuMode::Responsive).await,
            "" => (),
            _ => {
                self.cpu_mode = CpuMode::Custom;

                let _res = self.tx.send(Event::SetCustomCpuMode).await;
            }
        }
    }

    /// This process will have its process group prioritized over background processes
    async fn set_foreground_process(&mut self, pid: u32) {
        let _res = self.tx.send(Event::SetForegroundProcess(pid)).await;
    }
}

pub(crate) async fn interface_handle(
    connection: &zbus::Connection,
) -> Option<zbus::InterfaceRef<Server>> {
    let interface_result = connection
        .object_server()
        .interface::<_, Server>("/com/system76/Scheduler")
        .await;

    let iface_handle = match interface_result {
        Ok(iface_handler) => iface_handler,
        Err(why) => {
            tracing::error!("DBus interface not reachable: {:#?}", why);
            return None;
        }
    };

    Some(iface_handle)
}
