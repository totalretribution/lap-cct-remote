//! Asks BlueZ to scan LE with every duplicate reported, so `light-radio listen` hears every press,
//! not just the first one from an address.

use std::collections::HashMap;

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::Value;

use crate::controller::Error;

/// LE discovery, running until dropped. BlueZ also stops it if this process dies.
pub struct Scan {
    adapter: Proxy<'static>,
}

impl Scan {
    pub fn start(adapter: &str) -> Result<Scan, Error> {
        let conn =
            Connection::system().map_err(|e| Error::Bluetooth(format!("system D-Bus: {e}")))?;
        let adapter = Proxy::new(
            &conn,
            "org.bluez",
            format!("/org/bluez/{adapter}"),
            "org.bluez.Adapter1",
        )
        .map_err(|e| bluez_error(&e))?;
        let filter: HashMap<&str, Value> = HashMap::from([
            ("Transport", Value::from("le")),
            ("DuplicateData", Value::from(true)),
        ]);
        adapter
            .call_method("SetDiscoveryFilter", &(filter,))
            .map_err(|e| bluez_error(&e))?;
        adapter
            .call_method("StartDiscovery", &())
            .map_err(|e| bluez_error(&e))?;
        Ok(Scan { adapter })
    }
}

impl Drop for Scan {
    fn drop(&mut self) {
        let _ = self.adapter.call_method("StopDiscovery", &());
    }
}

fn bluez_error(e: &zbus::Error) -> Error {
    let text = e.to_string();
    Error::Bluetooth(if text.contains("NotReady") {
        "Bluetooth is turned off".to_owned()
    } else if text.contains("ServiceUnknown") {
        "the Bluetooth service isn't running".to_owned()
    } else if text.contains("UnknownObject") {
        "no Bluetooth adapter".to_owned()
    } else {
        format!("BlueZ: {text}")
    })
}
