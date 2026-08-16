//! The only crate in lumen that touches hardware.
//!
//! Two macOS specifics drive the design:
//!
//! * hidapi opens HID devices exclusively by default, which makes peripherals
//!   that other processes hold unopenable. The `macos-shared-device` feature
//!   switches to a shared open, and this crate depends on it.
//! * One macOS HID "path" is one USB interface, and which interface carries a
//!   device's control report is a per-device fact, not a convention -- the
//!   keyboard verified here uses interface 2, the mouse interface 1. The control
//!   interface is therefore found by reading each interface's report descriptor
//!   and picking the one that declares the feature report the driver expects,
//!   rather than by hardcoding a number.
//! * Without the Input Monitoring grant, opening an interface fails with
//!   `kIOReturnNotPermitted` -- but not uniformly: on the hardware here every
//!   interface of the keyboard was refused while one of the mouse's three still
//!   opened. So a refusal cannot be read as "this interface is special"; see
//!   [`access`] for how the grant is checked instead of inferred.

pub mod access;
pub mod descriptor;

use hidapi::HidApi;
use lumen_core::{DeviceSpec, Packet};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub use descriptor::feature_reports;

#[derive(Debug, Error)]
pub enum HidError {
    #[error("could not start HID access: {0}")]
    Init(String),
    #[error(
        "no HID devices are visible at all.\n\
         This is not the Input Monitoring grant -- listing devices works without it. \
         Either this process was started by launchd, which always sees an empty list, \
         or another process is holding the devices."
    )]
    NoDevicesVisible,
    #[error(
        "cannot open {name}: this terminal is not allowed to talk to HID devices.\n\
         macOS charges the Input Monitoring grant to whatever launched the shell, not \
         to the `lumen` binary, so it is your terminal that needs it.\n\
         System Settings -> Privacy & Security -> Input Monitoring: {fix}\n\
         What each interface said:\n  {tried}"
    )]
    InputMonitoring {
        name: String,
        fix: &'static str,
        tried: String,
    },
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

/// One USB interface as `lumen probe` found it.
pub struct ProbedInterface {
    pub interface: i32,
    /// Feature reports this interface declares, as `(report id, data bytes)`.
    /// The data-byte count is what a registry entry's `control_report_len` holds.
    pub feature_reports: Vec<(u8, u32)>,
    /// Why the interface could not be read, when it could not. Some interfaces
    /// are unopenable by design -- a keyboard collection always is.
    pub error: Option<String>,
}

/// One attached device and every interface it exposes, known to lumen or not.
pub struct ProbedDevice {
    pub vendor_id: u16,
    pub product_id: u16,
    pub product: String,
    pub manufacturer: String,
    pub interfaces: Vec<ProbedInterface>,
}

/// The permission error to raise when a device will not open, or `None` when
/// Input Monitoring is granted and the cause is something else. `tried` is what
/// each interface said: the cause leads, but the diagnosis is kept under it,
/// because a missing grant is only the likeliest reason an open failed.
fn permission_error(name: &str, tried: &[String]) -> Option<HidError> {
    access::input_monitoring()
        .fix()
        .map(|fix| HidError::InputMonitoring {
            name: name.to_string(),
            fix,
            tried: tried.join("\n  "),
        })
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
        let mut refused_any = false;
        for (path, interface) in &paths {
            let cpath = match std::ffi::CString::new(path.as_str()) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let device = match self.api.open_path(&cpath) {
                Ok(d) => d,
                Err(e) => {
                    tried.push(format!("interface {interface}: cannot open ({e})"));
                    refused_any = true;
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

        // An interface refused to open and the grant is missing: say that, and
        // keep the refusals underneath it. Printing only the refusals is what
        // once made lumen look broken for an hour.
        //
        // The test is "any interface refused", not "none opened": a device can
        // hand over a harmless interface and refuse the control one -- which is
        // exactly what the mouse does -- and that case needs the same answer.
        // When access is granted this never fires and the list stands on its
        // own: a keyboard collection refuses to open however it is asked.
        if refused_any && let Some(e) = permission_error(&spec.name, &tried) {
            return Err(e);
        }

        Err(HidError::NoControlInterface {
            name: spec.name.clone(),
            expected: spec.control_report_len,
            tried: tried.join("\n  "),
        })
    }

    /// Every attached HID device with the feature reports each interface
    /// declares -- what `lumen probe` needs to describe hardware lumen has never
    /// heard of. Devices are keyed by USB id, because one device shows up in the
    /// HID list once per interface.
    pub fn scan(&self) -> Result<Vec<ProbedDevice>, HidError> {
        let mut devices: BTreeMap<(u16, u16), ProbedDevice> = BTreeMap::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();

        for info in self.api.device_list() {
            let path = info.path().to_string_lossy().into_owned();
            // Several HID entries can share one interface path; read it once.
            if !seen.insert(path.clone()) {
                continue;
            }
            devices
                .entry((info.vendor_id(), info.product_id()))
                .or_insert_with(|| ProbedDevice {
                    vendor_id: info.vendor_id(),
                    product_id: info.product_id(),
                    product: info.product_string().unwrap_or_default().to_string(),
                    manufacturer: info.manufacturer_string().unwrap_or_default().to_string(),
                    interfaces: Vec::new(),
                })
                .interfaces
                .push(self.read_interface(&path, info.interface_number()));
        }

        if devices.is_empty() {
            return Err(HidError::NoDevicesVisible);
        }

        let mut out: Vec<ProbedDevice> = devices.into_values().collect();
        for device in &mut out {
            device.interfaces.sort_by_key(|i| i.interface);
        }
        Ok(out)
    }

    /// Read one interface's feature reports, recording why not when it fails.
    /// Probing never gives up on a device because one of its interfaces is shut.
    fn read_interface(&self, path: &str, interface: i32) -> ProbedInterface {
        let mut probed = ProbedInterface {
            interface,
            feature_reports: Vec::new(),
            error: None,
        };
        let Ok(cpath) = std::ffi::CString::new(path) else {
            probed.error = Some("device path is not usable".to_string());
            return probed;
        };
        match self.api.open_path(&cpath) {
            Err(e) => probed.error = Some(e.to_string()),
            Ok(device) => {
                let mut buf = vec![0u8; 4096];
                match device.get_report_descriptor(&mut buf) {
                    Ok(n) => probed.feature_reports = feature_reports(&buf[..n]),
                    Err(e) => probed.error = Some(e.to_string()),
                }
            }
        }
        probed
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
