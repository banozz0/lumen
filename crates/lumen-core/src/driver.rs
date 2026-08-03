use crate::{DeviceSpec, Rgb};
use thiserror::Error;

/// One HID feature report, ready for the wire.
///
/// `bytes[0]` is the report id, following the hidapi convention: a device that
/// uses unnumbered reports carries a leading `0x00` that is stripped before the
/// report reaches the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    pub bytes: Vec<u8>,
}

impl Packet {
    pub fn new(bytes: Vec<u8>) -> Self {
        Packet { bytes }
    }

    pub fn report_id(&self) -> u8 {
        self.bytes[0]
    }

    pub fn hex(&self) -> String {
        self.bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DriverError {
    #[error("no driver named `{0}`")]
    Unknown(String),
    #[error(
        "device `{device}` declares a {actual}-byte control report but driver `{driver}` \
         needs {expected}; re-check the value with `lumen probe`"
    )]
    ReportLenMismatch {
        device: String,
        driver: String,
        expected: u32,
        actual: u32,
    },
    #[error(
        "device `{device}` has no {command} command of its own; \
         send a colour instead -- its registry entry says how"
    )]
    NoNativeCommand { device: String, command: String },
}

/// Turns a command into the exact bytes a device expects.
///
/// Drivers build packets and never send them. That keeps protocol work a pure
/// function of (spec, command), so it is covered by byte-exact tests rather than
/// by plugging hardware in.
pub trait Driver {
    /// The driver name used in the registry's `driver` field.
    fn name(&self) -> &'static str;

    /// Data bytes this driver's control report must have, excluding the report id.
    fn control_report_len(&self) -> u32;

    fn set_color(&self, spec: &DeviceSpec, color: Rgb) -> Result<Vec<Packet>, DriverError>;

    /// Set brightness, where `level` is a percentage from 0 (dark) to 100
    /// (full). Percent is the scale everywhere above the driver layer; each
    /// driver converts to whatever its protocol wants, because devices disagree
    /// (Razer takes 0-255).
    ///
    /// A protocol with no brightness command returns
    /// [`DriverError::NoNativeCommand`] rather than silently faking one. The
    /// caller reads the device's declared [`crate::device::BrightnessMode`] and
    /// scales the colour instead.
    fn set_brightness(&self, spec: &DeviceSpec, level: u8) -> Result<Vec<Packet>, DriverError>;

    /// Turn the lighting off (`on = false`) or back on.
    ///
    /// A protocol with no power command returns
    /// [`DriverError::NoNativeCommand`]; the caller reads the device's declared
    /// [`crate::device::PowerMode`] and sends black or white instead.
    fn set_power(&self, spec: &DeviceSpec, on: bool) -> Result<Vec<Packet>, DriverError>;

    /// Reject a registry entry wired to the wrong driver before any I/O happens.
    fn check(&self, spec: &DeviceSpec) -> Result<(), DriverError> {
        if spec.control_report_len == self.control_report_len() {
            Ok(())
        } else {
            Err(DriverError::ReportLenMismatch {
                device: spec.id.clone(),
                driver: self.name().to_string(),
                expected: self.control_report_len(),
                actual: spec.control_report_len,
            })
        }
    }
}
