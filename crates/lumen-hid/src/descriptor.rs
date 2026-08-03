//! Just enough HID report descriptor parsing to find a device's control report.
//!
//! A descriptor is a flat stream of items. Each starts with a prefix byte holding
//! a tag, a type and a payload size; global items (report id, report size, report
//! count) accumulate as state, and a main item (Input/Output/Feature) emits a
//! report using whatever state is current.

/// Every feature report a descriptor declares, as `(report_id, data_byte_count)`.
/// The count excludes the report-id byte that hidapi expects at `bytes[0]`.
pub fn feature_reports(desc: &[u8]) -> Vec<(u8, u32)> {
    const TYPE_MAIN: u8 = 0;
    const TYPE_GLOBAL: u8 = 1;
    const TAG_FEATURE: u8 = 0xB;
    const TAG_REPORT_SIZE: u8 = 0x7;
    const TAG_REPORT_ID: u8 = 0x8;
    const TAG_REPORT_COUNT: u8 = 0x9;

    let (mut report_id, mut report_size, mut report_count) = (0u32, 0u32, 0u32);
    let mut out = Vec::new();
    let mut i = 0usize;

    while i < desc.len() {
        let prefix = desc[i];
        i += 1;
        // A size field of 3 means four payload bytes, not three.
        let size = match prefix & 0x03 {
            3 => 4,
            n => n as usize,
        };
        if i + size > desc.len() {
            break; // truncated descriptor; report what we have
        }
        let mut data = 0u32;
        for (k, b) in desc[i..i + size].iter().enumerate() {
            data |= (*b as u32) << (8 * k);
        }
        i += size;

        match ((prefix >> 2) & 0x03, prefix >> 4) {
            (TYPE_GLOBAL, TAG_REPORT_SIZE) => report_size = data,
            (TYPE_GLOBAL, TAG_REPORT_ID) => report_id = data,
            (TYPE_GLOBAL, TAG_REPORT_COUNT) => report_count = data,
            (TYPE_MAIN, TAG_FEATURE) => {
                out.push((report_id as u8, (report_size * report_count).div_ceil(8)))
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::feature_reports;

    /// Real tail of the Pulsefire Core descriptor: vendor page 0xff01, report id
    /// 0x07, 8-bit fields, count 0x0107 = 263, feature.
    #[test]
    fn finds_the_hyperx_control_report() {
        let desc = [
            0x06, 0x01, 0xff, 0x09, 0x01, 0xa1, 0x01, 0x85, 0x07, 0x15, 0x00, 0x26, 0xff, 0x00,
            0x09, 0x20, 0x75, 0x08, 0x96, 0x07, 0x01, 0xb1, 0x02, 0xc0,
        ];
        assert_eq!(feature_reports(&desc), vec![(0x07, 263)]);
    }

    /// Real tail of the Cynosa Lite descriptor: vendor page 0xff00, unnumbered
    /// report, 8-bit fields, count 0x5a = 90, feature.
    #[test]
    fn finds_the_razer_control_report() {
        let desc = [
            0x06, 0x00, 0xff, 0x09, 0x02, 0x15, 0x00, 0x25, 0x01, 0x75, 0x08, 0x95, 0x5a, 0xb1,
            0x01, 0xc0,
        ];
        assert_eq!(feature_reports(&desc), vec![(0x00, 90)]);
    }

    /// The Cynosa Lite's interface 0 is a plain keyboard: no feature reports, so
    /// lumen must skip it rather than try to drive it.
    #[test]
    fn ignores_interfaces_with_no_feature_report() {
        let desc = [
            0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
            0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x81, 0x01, 0xc0,
        ];
        assert!(feature_reports(&desc).is_empty());
    }

    #[test]
    fn a_truncated_descriptor_does_not_panic() {
        // Prefix claims a 4-byte payload that is not there.
        assert!(feature_reports(&[0x96, 0x07]).is_empty());
    }

    #[test]
    fn handles_an_empty_descriptor() {
        assert!(feature_reports(&[]).is_empty());
    }
}
