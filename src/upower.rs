// Copyright 2021 System76 <info@system76.com>
// SPDX-License-Identifier: MPL-2.0

use zbus::{zvariant::Value, Connection, PropertyChangedHandlerId, Proxy};

const UPOWER_IFACE: &str = "org.freedesktop.UPower";
const UPOWER_PATH: &str = "/org/freedesktop/UPower";
pub struct UPowerProxy<'a>(pub Proxy<'a>);

impl<'a> UPowerProxy<'a> {
    pub async fn new(connection: &Connection) -> zbus::Result<UPowerProxy<'a>> {
        let proxy = Proxy::new(&connection, UPOWER_IFACE, UPOWER_PATH, UPOWER_IFACE).await?;

        Ok(Self(proxy))
    }

    pub async fn on_battery(&self) -> bool {
        self.0
            .get_property::<bool>("OnBattery")
            .await
            .unwrap_or(false)
    }

    pub async fn connect_on_battery<H: FnMut(bool) + Send + 'static>(
        &self,
        mut handler: H,
    ) -> zbus::Result<PropertyChangedHandlerId> {
        self.0
            .connect_property_changed("OnBattery", move |m| {
                if let Some(v) = m {
                    if let Value::Bool(v) = v {
                        handler(*v);
                    }
                }

                Box::pin(async move {})
            })
            .await
    }
}
