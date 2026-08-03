//! Core types for lumen: colours, the device registry, and the driver interface.
//!
//! This crate performs no I/O. Drivers turn a command into the exact bytes a
//! device expects, which keeps every protocol fully testable without hardware --
//! `lumen-hid` is the only crate that talks to a device.
//!
//! The public surface here is deliberately plain (owned data, no generics or
//! lifetimes escaping) so a SwiftUI front-end can sit on top of it later through
//! a thin FFI wrapper rather than a rewrite.

pub mod color;
pub mod device;
pub mod driver;

pub use color::{ColorParseError, Rgb};
pub use device::{BrightnessMode, DeviceSpec, PowerMode, Registry, RegistryError};
pub use driver::{Driver, DriverError, Packet};
