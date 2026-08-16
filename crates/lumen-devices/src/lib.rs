//! Protocol drivers.
//!
//! One module per protocol family, not per device: several devices can share a
//! driver, which is why adding a device usually means editing the registry and
//! writing no Rust at all.
//!
//! Every driver is written from published byte-level protocol facts. No code is
//! copied from OpenRGB or OpenRazer, which is what lets lumen be MIT rather than
//! inherit their licences.

pub mod hyperx;
pub mod razer;

use lumen_core::{Driver, DriverError};

/// Look up a driver by the name used in the registry's `driver` field.
pub fn driver_for(name: &str) -> Result<Box<dyn Driver>, DriverError> {
    match name {
        razer::NAME => Ok(Box::new(razer::ExtendedMatrix)),
        hyperx::NAME => Ok(Box::new(hyperx::Pulsefire)),
        other => Err(DriverError::Unknown(other.to_string())),
    }
}

/// Every driver name lumen ships, for help text and diagnostics.
pub fn driver_names() -> &'static [&'static str] {
    &[razer::NAME, hyperx::NAME]
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::{BrightnessMode, PowerMode, Registry};

    /// The registry says how each device reaches brightness and power, and the
    /// driver has to agree: a device declared `native` must build the packet,
    /// and one declared otherwise must refuse rather than fake it. Without this
    /// the data and the code could drift apart silently.
    #[test]
    fn declared_behaviour_matches_what_each_driver_implements() {
        for spec in Registry::builtin().unwrap().all() {
            let driver = driver_for(&spec.driver).unwrap();

            let brightness = driver.set_brightness(spec, 50);
            match spec.brightness {
                BrightnessMode::Native => assert!(
                    brightness.is_ok(),
                    "`{}` declares native brightness but the driver refused it",
                    spec.id
                ),
                BrightnessMode::Scaled => assert!(
                    brightness.is_err(),
                    "`{}` declares scaled brightness but the driver built a packet",
                    spec.id
                ),
            }

            let power = driver.set_power(spec, false);
            match spec.power {
                PowerMode::Native => assert!(
                    power.is_ok(),
                    "`{}` declares native power but the driver refused it",
                    spec.id
                ),
                PowerMode::ColorBlack => assert!(
                    power.is_err(),
                    "`{}` declares emulated power but the driver built a packet",
                    spec.id
                ),
            }
        }
    }

    #[test]
    fn every_registry_entry_has_a_working_driver() {
        let reg = Registry::builtin().unwrap();
        for spec in reg.all() {
            let driver = driver_for(&spec.driver)
                .unwrap_or_else(|e| panic!("device `{}`: {e}", spec.id));
            driver
                .check(spec)
                .unwrap_or_else(|e| panic!("device `{}`: {e}", spec.id));
        }
    }

    #[test]
    fn unknown_driver_is_an_error() {
        assert!(driver_for("does-not-exist").is_err());
    }

    #[test]
    fn all_named_drivers_resolve() {
        for name in driver_names() {
            assert!(driver_for(name).is_ok(), "{name} is named but missing");
        }
    }
}
