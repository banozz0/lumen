//! Razer "extended matrix" protocol.
//!
//! Razer peripherals take a fixed 90-byte control report on their vendor HID
//! collection. Verified against a Cynosa Lite (1532:023F), which exposes it as an
//! unnumbered feature report on interface 2.
//!
//! Report body layout, offsets into the 90 bytes:
//!
//! ```text
//!   0      status              0x00
//!   1      transaction id      0x3F
//!   2..3   remaining packets   0x0000
//!   4      protocol type       0x00
//!   5      data size           number of meaningful argument bytes
//!   6      command class       0x0F = extended matrix
//!   7      command id          0x02 = set effect, 0x04 = set brightness
//!   8..87  arguments
//!   88     crc                 XOR of bytes 2..87
//!   89     reserved            0x00
//! ```
//!
//! Commands used here, as published wire captures (`data size` is the count of
//! meaningful argument bytes and is *not* always the count of non-zero ones):
//!
//! ```text
//!   static   size 09  class 0f  cmd 02  args 01 05 01 00 00 01 RR GG BB
//!   none     size 06  class 0f  cmd 02  args 01 05 00 00 00 00
//!   bright   size 03  class 0f  cmd 04  args 01 05 LL
//! ```
//!
//! where `01` is VARSTORE, `05` is the backlight LED and `LL` is brightness on
//! a 0-255 scale.

use lumen_core::{DeviceSpec, Driver, DriverError, Packet, Rgb};

pub const NAME: &str = "razer-extended-matrix";

/// Bytes in the report body, excluding the leading hidapi report-id byte.
const REPORT_LEN: usize = 90;

const TRANSACTION_ID: u8 = 0x3F;
const CLASS_EXTENDED_MATRIX: u8 = 0x0F;
const CMD_SET_EFFECT: u8 = 0x02;
const CMD_SET_BRIGHTNESS: u8 = 0x04;

/// Write to the device's variable store, so the setting survives a replug.
const VARSTORE: u8 = 0x01;
const BACKLIGHT_LED: u8 = 0x05;
const EFFECT_STATIC: u8 = 0x01;
const EFFECT_NONE: u8 = 0x00;

pub struct ExtendedMatrix;

impl ExtendedMatrix {
    /// Build a 90-byte report body and checksum it.
    fn report(command_class: u8, command_id: u8, args: &[u8]) -> [u8; REPORT_LEN] {
        debug_assert!(args.len() <= 80, "arguments overflow the report");
        let mut r = [0u8; REPORT_LEN];
        r[1] = TRANSACTION_ID;
        r[5] = args.len() as u8;
        r[6] = command_class;
        r[7] = command_id;
        r[8..8 + args.len()].copy_from_slice(args);
        r[88] = r[2..88].iter().fold(0u8, |crc, b| crc ^ b);
        r
    }

    /// Wrap a report body in the wire buffer this device expects: it uses
    /// unnumbered reports, so byte 0 is the report id 0x00 and the body follows.
    fn packet(body: [u8; REPORT_LEN]) -> Packet {
        let mut bytes = Vec::with_capacity(REPORT_LEN + 1);
        bytes.push(0x00);
        bytes.extend_from_slice(&body);
        Packet::new(bytes)
    }

    /// Percent to the 0-255 scale the protocol uses, rounded to nearest.
    fn brightness_byte(level: u8) -> u8 {
        ((u16::from(level.min(100)) * 255 + 50) / 100) as u8
    }
}

impl Driver for ExtendedMatrix {
    fn name(&self) -> &'static str {
        NAME
    }

    fn control_report_len(&self) -> u32 {
        REPORT_LEN as u32
    }

    fn set_color(&self, spec: &DeviceSpec, c: Rgb) -> Result<Vec<Packet>, DriverError> {
        self.check(spec)?;
        let body = Self::report(
            CLASS_EXTENDED_MATRIX,
            CMD_SET_EFFECT,
            &[
                VARSTORE,
                BACKLIGHT_LED,
                EFFECT_STATIC,
                0x00,
                0x00,
                0x01,
                c.r,
                c.g,
                c.b,
            ],
        );
        Ok(vec![Self::packet(body)])
    }

    fn set_brightness(&self, spec: &DeviceSpec, level: u8) -> Result<Vec<Packet>, DriverError> {
        self.check(spec)?;
        let body = Self::report(
            CLASS_EXTENDED_MATRIX,
            CMD_SET_BRIGHTNESS,
            &[VARSTORE, BACKLIGHT_LED, Self::brightness_byte(level)],
        );
        Ok(vec![Self::packet(body)])
    }

    fn set_power(&self, spec: &DeviceSpec, on: bool) -> Result<Vec<Packet>, DriverError> {
        self.check(spec)?;
        if !on {
            // The "none" effect. Its data size is 6, not the 3 argument bytes
            // that carry meaning -- the three trailing zeroes are part of the
            // command as captured, so they are sent explicitly.
            let body = Self::report(
                CLASS_EXTENDED_MATRIX,
                CMD_SET_EFFECT,
                &[VARSTORE, BACKLIGHT_LED, EFFECT_NONE, 0x00, 0x00, 0x00],
            );
            return Ok(vec![Self::packet(body)]);
        }

        // There is no "on" command: the LEDs come back as soon as an effect is
        // set. The device never reports the colour it had before, so lumen
        // cannot restore it and instead picks a known state -- static white at
        // full brightness. Brightness is re-sent too, otherwise turning on
        // after `--brightness 0` would leave the keyboard dark and look broken.
        let mut packets = self.set_color(spec, Rgb::WHITE)?;
        packets.extend(self.set_brightness(spec, 100)?);
        Ok(packets)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumen_core::Registry;

    fn cynosa() -> DeviceSpec {
        Registry::builtin()
            .unwrap()
            .by_usb_id(0x1532, 0x023F)
            .expect("Cynosa Lite in registry")
            .clone()
    }

    /// Golden bytes: this exact buffer turned the keyboard red on hardware.
    #[test]
    fn static_red_matches_verified_bytes() {
        let packets = ExtendedMatrix.set_color(&cynosa(), Rgb::new(255, 0, 0)).unwrap();
        assert_eq!(packets.len(), 1);
        let b = &packets[0].bytes;

        assert_eq!(b.len(), 91, "report id byte plus 90-byte body");
        assert_eq!(b[0], 0x00, "unnumbered report");
        // Header: status, transaction id, remaining, protocol, size, class, command.
        assert_eq!(
            &b[1..9],
            &[0x00, 0x3f, 0x00, 0x00, 0x00, 0x09, 0x0f, 0x02]
        );
        // Arguments, ending in the colour.
        assert_eq!(
            &b[9..18],
            &[0x01, 0x05, 0x01, 0x00, 0x00, 0x01, 0xff, 0x00, 0x00]
        );
        assert_eq!(b[90], 0x00, "reserved");
    }

    #[test]
    fn checksum_is_xor_of_body_bytes_2_to_88() {
        for colour in [Rgb::new(255, 0, 0), Rgb::new(0, 255, 0), Rgb::new(1, 2, 3)] {
            let p = ExtendedMatrix.set_color(&cynosa(), colour).unwrap();
            let body = &p[0].bytes[1..];
            let expected = body[2..88].iter().fold(0u8, |crc, b| crc ^ b);
            assert_eq!(body[88], expected, "crc wrong for {colour}");
        }
    }

    #[test]
    fn colour_lands_in_the_last_three_argument_bytes() {
        let p = ExtendedMatrix
            .set_color(&cynosa(), Rgb::new(0x12, 0x34, 0x56))
            .unwrap();
        assert_eq!(&p[0].bytes[15..18], &[0x12, 0x34, 0x56]);
    }

    /// Golden bytes: brightness is class 0x0f command 0x04, data size 3,
    /// arguments VARSTORE, backlight, level on a 0-255 scale.
    #[test]
    fn full_brightness_matches_the_documented_command() {
        let packets = ExtendedMatrix.set_brightness(&cynosa(), 100).unwrap();
        assert_eq!(packets.len(), 1);
        let b = &packets[0].bytes;

        assert_eq!(b.len(), 91, "report id byte plus 90-byte body");
        assert_eq!(b[0], 0x00, "unnumbered report");
        assert_eq!(&b[1..9], &[0x00, 0x3f, 0x00, 0x00, 0x00, 0x03, 0x0f, 0x04]);
        assert_eq!(&b[9..12], &[0x01, 0x05, 0xff]);
        assert!(b[12..89].iter().all(|x| *x == 0), "arguments must end there");
        assert_eq!(b[89], b[3..89].iter().fold(0u8, |crc, x| crc ^ x), "crc");
        assert_eq!(b[90], 0x00, "reserved");
    }

    #[test]
    fn brightness_percent_maps_onto_the_protocol_0_255_scale() {
        let level_byte = |pct| ExtendedMatrix.set_brightness(&cynosa(), pct).unwrap()[0].bytes[11];
        assert_eq!(level_byte(0), 0x00);
        assert_eq!(level_byte(50), 0x80, "127.5 rounds up");
        assert_eq!(level_byte(100), 0xff);
        // Out of range is clamped rather than wrapping round to a dim value.
        assert_eq!(level_byte(200), 0xff);
    }

    /// Golden bytes: the "none" effect. Data size is 6 even though only three
    /// argument bytes carry meaning.
    #[test]
    fn off_matches_the_documented_none_effect() {
        let packets = ExtendedMatrix.set_power(&cynosa(), false).unwrap();
        assert_eq!(packets.len(), 1);
        let b = &packets[0].bytes;

        assert_eq!(b.len(), 91);
        assert_eq!(b[0], 0x00, "unnumbered report");
        assert_eq!(&b[1..9], &[0x00, 0x3f, 0x00, 0x00, 0x00, 0x06, 0x0f, 0x02]);
        assert_eq!(&b[9..15], &[0x01, 0x05, 0x00, 0x00, 0x00, 0x00]);
        assert_eq!(b[89], b[3..89].iter().fold(0u8, |crc, x| crc ^ x), "crc");
        assert_eq!(b[90], 0x00, "reserved");
    }

    /// Off is its own command, not the colour black: black is still a static
    /// effect and would leave the backlight driven.
    #[test]
    fn off_is_not_the_same_packet_as_the_colour_black() {
        let off = ExtendedMatrix.set_power(&cynosa(), false).unwrap();
        let black = ExtendedMatrix.set_color(&cynosa(), Rgb::BLACK).unwrap();
        assert_ne!(off[0].bytes, black[0].bytes);
    }

    /// The protocol has no "on", so lumen restores a known state: static white,
    /// then full brightness so a previous `--brightness 0` cannot hide it.
    #[test]
    fn on_reapplies_static_white_at_full_brightness() {
        let packets = ExtendedMatrix.set_power(&cynosa(), true).unwrap();
        assert_eq!(packets.len(), 2);
        assert_eq!(
            packets[0].bytes,
            ExtendedMatrix.set_color(&cynosa(), Rgb::WHITE).unwrap()[0].bytes
        );
        assert_eq!(
            packets[1].bytes,
            ExtendedMatrix.set_brightness(&cynosa(), 100).unwrap()[0].bytes
        );
    }

    #[test]
    fn rejects_a_device_whose_report_length_disagrees() {
        let mut spec = cynosa();
        spec.control_report_len = 264;
        assert!(matches!(
            ExtendedMatrix.set_color(&spec, Rgb::new(1, 1, 1)),
            Err(DriverError::ReportLenMismatch { .. })
        ));
        assert!(matches!(
            ExtendedMatrix.set_brightness(&spec, 50),
            Err(DriverError::ReportLenMismatch { .. })
        ));
        assert!(matches!(
            ExtendedMatrix.set_power(&spec, false),
            Err(DriverError::ReportLenMismatch { .. })
        ));
    }
}
