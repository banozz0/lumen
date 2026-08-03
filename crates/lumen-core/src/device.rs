use serde::Deserialize;
use thiserror::Error;

/// What a device is and how to talk to it. Parsed from `devices/devices.toml`.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DeviceSpec {
    /// Stable slug, e.g. `razer-cynosa-lite`. Used to target the device on the CLI.
    pub id: String,
    pub name: String,
    pub vendor: String,
    /// `keyboard` or `mouse`. Also usable as a CLI target.
    pub kind: String,
    #[serde(deserialize_with = "hex_u16")]
    pub vendor_id: u16,
    #[serde(deserialize_with = "hex_u16")]
    pub product_id: u16,
    /// Name of the protocol driver in `lumen-devices` that handles this device.
    pub driver: String,
    /// Data bytes in the device's control feature report, excluding the report id.
    pub control_report_len: u32,
    /// Whether the hardware keeps the colour on its own. When false, the colour
    /// only lasts while lumen keeps re-sending it.
    pub holds_colour: bool,
    /// How this device gets its brightness set.
    pub brightness: BrightnessMode,
    /// How this device gets turned off and back on.
    pub power: PowerMode,
}

/// How a device's brightness is reached, declared per device rather than
/// decided by a match on device id in the code that uses it.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BrightnessMode {
    /// The protocol has a brightness command: the driver builds it.
    Native,
    /// The protocol has none, so brightness is applied by scaling the colour.
    Scaled,
}

/// How a device is turned off and back on.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PowerMode {
    /// The protocol has an off command: the driver builds it.
    Native,
    /// The protocol has none, so off is the colour black and on is white.
    ColorBlack,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    device: Vec<DeviceSpec>,
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("device registry is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("duplicate device id `{0}` in the registry")]
    DuplicateId(String),
    #[error("devices `{0}` and `{1}` both claim USB id {2:04x}:{3:04x}")]
    DuplicateUsbId(String, String, u16, u16),
}

/// Every device lumen knows how to drive.
#[derive(Debug, Clone)]
pub struct Registry {
    devices: Vec<DeviceSpec>,
}

impl Registry {
    /// The registry compiled into this binary.
    pub fn builtin() -> Result<Self, RegistryError> {
        Self::parse(include_str!("../../../devices/devices.toml"))
    }

    pub fn parse(src: &str) -> Result<Self, RegistryError> {
        let file: RegistryFile = toml::from_str(src)?;
        for (i, d) in file.device.iter().enumerate() {
            if let Some(prev) = file.device[..i].iter().find(|p| p.id == d.id) {
                return Err(RegistryError::DuplicateId(prev.id.clone()));
            }
            if let Some(prev) = file.device[..i]
                .iter()
                .find(|p| p.vendor_id == d.vendor_id && p.product_id == d.product_id)
            {
                return Err(RegistryError::DuplicateUsbId(
                    prev.id.clone(),
                    d.id.clone(),
                    d.vendor_id,
                    d.product_id,
                ));
            }
        }
        Ok(Registry {
            devices: file.device,
        })
    }

    pub fn all(&self) -> &[DeviceSpec] {
        &self.devices
    }

    pub fn by_usb_id(&self, vendor_id: u16, product_id: u16) -> Option<&DeviceSpec> {
        self.devices
            .iter()
            .find(|d| d.vendor_id == vendor_id && d.product_id == product_id)
    }

    /// Resolve a CLI target: `all`, a device id, or a kind such as `mouse`.
    /// Matching is restricted to `present`, so targeting never returns a device
    /// that is not plugged in.
    pub fn resolve<'a>(&'a self, target: &str, present: &[&'a DeviceSpec]) -> Vec<&'a DeviceSpec> {
        if target == "all" {
            return present.to_vec();
        }
        present
            .iter()
            .copied()
            .filter(|d| d.id == target || d.kind == target)
            .collect()
    }
}

/// Accept USB ids written the way they are documented: `"0x1532"`.
fn hex_u16<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    use serde::de::Error as _;
    let s = String::deserialize(d)?;
    let t = s.trim();
    let digits = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X"));
    match digits {
        Some(h) => u16::from_str_radix(h, 16)
            .map_err(|e| D::Error::custom(format!("bad USB id `{t}`: {e}"))),
        None => Err(D::Error::custom(format!(
            "USB id `{t}` must be hex with an 0x prefix, e.g. \"0x1532\""
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ONE: &str = r#"
        [[device]]
        id = "test-kb"
        name = "Test Keyboard"
        vendor = "Test"
        kind = "keyboard"
        vendor_id = "0x1532"
        product_id = "0x023F"
        driver = "razer-extended-matrix"
        control_report_len = 90
        holds_colour = true
        brightness = "native"
        power = "native"
    "#;

    #[test]
    fn builtin_registry_parses_and_covers_both_devices() {
        let reg = Registry::builtin().expect("builtin registry must parse");
        assert!(reg.by_usb_id(0x1532, 0x023F).is_some(), "Cynosa Lite");
        assert!(reg.by_usb_id(0x03F0, 0x0D8F).is_some(), "Pulsefire Core");
    }

    #[test]
    fn parses_hex_usb_ids() {
        let reg = Registry::parse(ONE).unwrap();
        assert_eq!(reg.all()[0].vendor_id, 0x1532);
        assert_eq!(reg.all()[0].product_id, 0x023F);
    }

    #[test]
    fn rejects_malformed_toml() {
        assert!(Registry::parse("[[device]]\nid = ").is_err());
    }

    #[test]
    fn parses_declared_brightness_and_power_behaviour() {
        let reg = Registry::builtin().unwrap();
        let kb = reg.by_usb_id(0x1532, 0x023F).unwrap();
        assert_eq!(kb.brightness, BrightnessMode::Native);
        assert_eq!(kb.power, PowerMode::Native);

        let mouse = reg.by_usb_id(0x03F0, 0x0D8F).unwrap();
        assert_eq!(mouse.brightness, BrightnessMode::Scaled);
        assert_eq!(mouse.power, PowerMode::ColorBlack);
    }

    #[test]
    fn rejects_an_unknown_brightness_mode() {
        let src = ONE.replace("brightness = \"native\"", "brightness = \"guess\"");
        assert!(Registry::parse(&src).is_err());
    }

    #[test]
    fn rejects_usb_id_without_0x_prefix() {
        let src = ONE.replace("\"0x1532\"", "\"1532\"");
        let err = Registry::parse(&src).unwrap_err().to_string();
        assert!(err.contains("0x"), "unhelpful error: {err}");
    }

    #[test]
    fn rejects_duplicate_ids() {
        let src = format!("{ONE}{ONE}");
        assert!(matches!(
            Registry::parse(&src),
            Err(RegistryError::DuplicateId(_))
        ));
    }

    #[test]
    fn rejects_two_devices_sharing_a_usb_id() {
        let src = format!("{ONE}{}", ONE.replace("test-kb", "other-kb"));
        assert!(matches!(
            Registry::parse(&src),
            Err(RegistryError::DuplicateUsbId(..))
        ));
    }

    #[test]
    fn resolve_matches_id_kind_and_all_but_only_when_present() {
        let reg = Registry::builtin().unwrap();
        let kb = reg.by_usb_id(0x1532, 0x023F).unwrap();
        let present = vec![kb];

        assert_eq!(reg.resolve("keyboard", &present).len(), 1);
        assert_eq!(reg.resolve("razer-cynosa-lite", &present).len(), 1);
        assert_eq!(reg.resolve("all", &present).len(), 1);
        // The mouse is in the registry but not plugged in, so it never resolves.
        assert_eq!(reg.resolve("mouse", &present).len(), 0);
        assert_eq!(reg.resolve("nonsense", &present).len(), 0);
    }
}
