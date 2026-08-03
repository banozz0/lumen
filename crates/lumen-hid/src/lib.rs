//! The only crate in lumen that touches hardware.
//!
//! Two macOS specifics drive the design:
//!
//! * hidapi opens HID devices exclusively by default, which makes peripherals
//!   that other processes hold unopenable. The `macos-shared-device` feature
//!   switches to a shared open, and this crate depends on it.
//! * One macOS HID "path" is one USB interface, and an interface exposing a
//!   keyboard usage is protected: opening it fails with `kIOReturnNotPermitted`
//!   no matter what. The control interface is therefore found by reading each
//!   interface's report descriptor and picking the one that declares the feature
//!   report the driver expects, rather than by hardcoding interface numbers.

pub mod descriptor;

use hidapi::HidApi;
use lumen_core::{DeviceSpec, Packet};
use thiserror::Error;

pub use descriptor::feature_reports;

#[derive(Debug, Error)]
pub enum HidError {
    #[error("could not start HID access: {0}")]
    Init(String),
    #[error(
        "no HID devices are visible at all.\n\
         Grant Input Monitoring to your terminal in System Settings -> Privacy & \
         Security -> Input Monitoring, then run this again."
    )]
    NoDevicesVisible,
    #[error("{name} is not plugged in")]
    NotPresent { name: String },
    #[error(
        "found {name} but no interface exposes its {expected}-byte control report.\n\
         Tried:\n  {tried}"
    )]
    NoControlInterface {
        name: String,
        expected: u32,
        tried: String,
    },
    #[error("sending to {name} failed: {source}")]
    Send {
        name: String,
        #[source]
        source: hidapi::HidError,
    },
}

pub struct Hid {
    api: HidApi,
}

/// An opened control interface, ready to take packets.
pub struct Connection {
    device: hidapi::HidDevice,
    name: String,
    pub interface: i32,
    pub report_id: u8,
}

impl Hid {
    pub fn new() -> Result<Self, HidError> {
        HidApi::new()
            .map(|api| Hid { api })
            .map_err(|e| HidError::Init(e.to_string()))
    }

    /// USB ids currently attached. Empty means Input Monitoring is not granted,
    /// which is a different problem from "your device is unplugged".
    pub fn present_usb_ids(&self) -> Result<Vec<(u16, u16)>, HidError> {
        let mut ids: Vec<(u16, u16)> = self
            .api
            .device_list()
            .map(|d| (d.vendor_id(), d.product_id()))
            .collect();
        if ids.is_empty() {
            return Err(HidError::NoDevicesVisible);
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    /// Which registry entries are actually attached right now.
    pub fn present<'a>(&self, known: &'a [DeviceSpec]) -> Result<Vec<&'a DeviceSpec>, HidError> {
        let ids = self.present_usb_ids()?;
        Ok(known
            .iter()
            .filter(|d| ids.contains(&(d.vendor_id, d.product_id)))
            .collect())
    }

    /// Open the interface that declares the device's control feature report.
    pub fn open(&self, spec: &DeviceSpec) -> Result<Connection, HidError> {
        // Several HID entries can share one interface path; dedupe so each
        // interface is opened at most once.
        let mut paths: Vec<(String, i32)> = self
            .api
            .device_list()
            .filter(|d| d.vendor_id() == spec.vendor_id && d.product_id() == spec.product_id)
            .map(|d| (d.path().to_string_lossy().into_owned(), d.interface_number()))
            .collect();
        paths.sort();
        paths.dedup();

        if paths.is_empty() {
            self.present_usb_ids()?; // surfaces NoDevicesVisible when nothing is granted
            return Err(HidError::NotPresent {
                name: spec.name.clone(),
            });
        }

        let mut tried = Vec::new();
        for (path, interface) in &paths {
            let cpath = match std::ffi::CString::new(path.as_str()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let device = match self.api.open_path(&cpath) {
                Ok(d) => d,
                Err(e) => {
                    tried.push(format!("interface {interface}: cannot open ({e})"));
                    continue;
                }
            };
            let mut buf = vec![0u8; 4096];
            let len = match device.get_report_descriptor(&mut buf) {
                Ok(n) => n,
                Err(e) => {
                    tried.push(format!("interface {interface}: no report descriptor ({e})"));
                    continue;
                }
            };
            match feature_reports(&buf[..len])
                .into_iter()
                .find(|(_, len)| *len == spec.control_report_len)
            {
                Some((report_id, _)) => {
                    return Ok(Connection {
                        device,
                        name: spec.name.clone(),
                        interface: *interface,
                        report_id,
                    });
                }
                None => tried.push(format!(
                    "interface {interface}: no {}-byte feature report",
                    spec.control_report_len
                )),
            }
        }

        Err(HidError::NoControlInterface {
            name: spec.name.clone(),
            expected: spec.control_report_len,
            tried: tried.join("\n  "),
        })
    }
}

impl Connection {
    pub fn send(&self, packet: &Packet) -> Result<(), HidError> {
        self.device
            .send_feature_report(&packet.bytes)
            .map_err(|source| HidError::Send {
                name: self.name.clone(),
                source,
            })
    }

    pub fn send_all(&self, packets: &[Packet]) -> Result<(), HidError> {
        for p in packets {
            self.send(p)?;
        }
        Ok(())
    }
}
