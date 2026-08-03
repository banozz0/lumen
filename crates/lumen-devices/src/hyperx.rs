//! HyperX Pulsefire direct-colour protocol.
//!
//! Verified against a Pulsefire Core (03F0:0D8F), which declares feature report
//! 0x07 with 263 data bytes on interface 1 (vendor usage page 0xFF01).
//!
//! Wire buffer, 264 bytes including the leading report id:
//!
//! ```text
//!   0      report id     0x07
//!   1      packet id     0x0A = direct colour
//!   2..4   R, G, B
//!   8      0xA0          terminator the firmware requires
//! ```
//!
//! Known limitation: direct mode is not latched. The firmware reclaims the LED
//! between packets, so the colour only holds while the packet is re-sent. The
//! registry records this as `holds_colour = false`; finding the latch command is
//! still open.
//!
//! Direct colour is the only command known for this protocol: no brightness and
//! no power command have been published. Both are therefore refused here rather
//! than faked, and the registry declares how they are reached instead
//! (`brightness = "scaled"`, `power = "color-black"`). Keeping the refusal in
//! the driver means a caller can never mistake an emulated brightness for a
//! real one, and it leaves one obvious place to put the commands if they are
//! ever found.

use lumen_core::{DeviceSpec, Driver, DriverError, Packet, Rgb};

pub const NAME: &str = "hyperx-pulsefire";

/// Data bytes, excluding the leading report-id byte.
const REPORT_LEN: usize = 263;

const PACKET_ID_DIRECT: u8 = 0x0A;
const TERMINATOR: u8 = 0xA0;

pub struct Pulsefire;

impl Driver for Pulsefire {
    fn name(&self) -> &'static str {
        NAME
    }

    fn control_report_len(&self) -> u32 {
        REPORT_LEN as u32
    }

    fn set_color(&self, spec: &DeviceSpec, c: Rgb) -> Result<Vec<Packet>, DriverError> {
        self.check(spec)?;
        let mut bytes = vec![0u8; REPORT_LEN + 1];
        bytes[0x00] = 0x07;
        bytes[0x01] = PACKET_ID_DIRECT;
        bytes[0x02] = c.r;
        bytes[0x03] = c.g;
        bytes[0x04] = c.b;
        bytes[0x08] = TERMINATOR;
        Ok(vec![Packet::new(bytes)])
    }

    fn set_brightness(&self, spec: &DeviceSpec, _level: u8) -> Result<Vec<Packet>, DriverError> {
        Err(DriverError::NoNativeCommand {
            device: spec.id.clone(),
            command: "brightness".to_string(),
        })
    }

    fn set_power(&self, spec: &DeviceSpec, _on: bool) -> Result<Vec<Packet>, DriverError> {
        Err(DriverError::NoNativeCommand {
            device: spec.id.clone(),
            command: "power".to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::Registry;

    fn pulsefire() -> DeviceSpec {
        Registry::builtin()
            .unwrap()
            .by_usb_id(0x03F0, 0x0D8F)
            .expect("Pulsefire Core in registry")
            .clone()
    }

    /// Golden bytes: this exact buffer lit the mouse red on hardware.
    #[test]
    fn direct_red_matches_verified_bytes() {
        let packets = Pulsefire.set_color(&pulsefire(), Rgb::new(255, 0, 0)).unwrap();
        assert_eq!(packets.len(), 1);
        let b = &packets[0].bytes;

        assert_eq!(b.len(), 264, "report id byte plus 263 data bytes");
        assert_eq!(&b[..10], &[0x07, 0x0a, 0xff, 0x00, 0x00, 0, 0, 0, 0xa0, 0]);
    }

    #[test]
    fn only_the_documented_offsets_are_ever_set() {
        let b = &Pulsefire
            .set_color(&pulsefire(), Rgb::new(0x11, 0x22, 0x33))
            .unwrap()[0]
            .bytes;
        assert_eq!(&b[0x02..0x05], &[0x11, 0x22, 0x33]);
        assert_eq!(b[0x08], 0xa0);
        // Everything past the header must stay zeroed.
        assert!(b[0x09..].iter().all(|x| *x == 0), "unexpected trailing bytes");
        assert_eq!(b[0x05..0x08], [0, 0, 0]);
    }

    #[test]
    fn black_is_a_valid_colour_not_a_special_case() {
        let b = &Pulsefire.set_color(&pulsefire(), Rgb::BLACK).unwrap()[0].bytes;
        assert_eq!(&b[..10], &[0x07, 0x0a, 0x00, 0x00, 0x00, 0, 0, 0, 0xa0, 0]);
    }

    #[test]
    fn rejects_a_device_whose_report_length_disagrees() {
        let mut spec = pulsefire();
        spec.control_report_len = 90;
        assert!(matches!(
            Pulsefire.set_color(&spec, Rgb::new(1, 1, 1)),
            Err(DriverError::ReportLenMismatch { .. })
        ));
    }

    #[test]
    fn brightness_and_power_are_refused_rather_than_faked() {
        assert!(matches!(
            Pulsefire.set_brightness(&pulsefire(), 50),
            Err(DriverError::NoNativeCommand { .. })
        ));
        assert!(matches!(
            Pulsefire.set_power(&pulsefire(), false),
            Err(DriverError::NoNativeCommand { .. })
        ));
    }

    /// Golden bytes: brightness is reached by scaling the colour, so a dimmed
    /// colour is an ordinary direct-colour packet and nothing else.
    #[test]
    fn a_scaled_colour_is_just_a_direct_colour_packet() {
        let dim = Rgb::new(255, 0, 0).scaled(40);
        assert_eq!(dim, Rgb::new(102, 0, 0));
        let b = &Pulsefire.set_color(&pulsefire(), dim).unwrap()[0].bytes;
        assert_eq!(&b[..10], &[0x07, 0x0a, 0x66, 0x00, 0x00, 0, 0, 0, 0xa0, 0]);
    }
}
