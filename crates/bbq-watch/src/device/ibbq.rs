//! iBBQ GATT protocol constants and pure parse/encode functions.
//!
//! Everything lives under GATT service `0xFFF0`. The parse functions are pure
//! and unit-tested; the BLE plumbing that uses them lives in [`super::ble`].

use crate::model::Unit;

/// GATT service.
pub const SERVICE: u16 = 0xFFF0;
/// Command replies / battery notifications.
pub const CHAR_SETTINGS_RESULT: u16 = 0xFFF1;
/// Login characteristic (write).
pub const CHAR_ACCOUNT_AND_VERIFY: u16 = 0xFFF2;
/// Realtime temperature notifications.
pub const CHAR_REALTIME_DATA: u16 = 0xFFF4;
/// Command characteristic (write).
pub const CHAR_SETTINGS_DATA: u16 = 0xFFF5;

/// Login payload; must be the first write after connecting.
pub const LOGIN: &[u8] = &[
    0x21, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01, 0xB8, 0x22, 0x00, 0x00, 0x00, 0x00, 0x00,
];
/// Enable realtime notifications.
pub const ENABLE_REALTIME: &[u8] = &[0x0B, 0x01, 0x00, 0x00, 0x00, 0x00];
/// Set the device screen to Celsius.
pub const UNITS_CELSIUS: &[u8] = &[0x02, 0x00, 0x00, 0x00, 0x00, 0x00];
/// Set the device screen to Fahrenheit.
pub const UNITS_FAHRENHEIT: &[u8] = &[0x02, 0x01, 0x00, 0x00, 0x00, 0x00];
/// Request the battery status.
pub const REQUEST_BATTERY: &[u8] = &[0x08, 0x24, 0x00, 0x00, 0x00, 0x00];

/// Sentinel raw value meaning "probe unplugged".
pub const UNPLUGGED: u16 = 0xFFF6;

/// Build the command payload that sets the device screen unit.
#[must_use]
pub fn encode_units(unit: Unit) -> &'static [u8] {
    match unit {
        Unit::Celsius => UNITS_CELSIUS,
        Unit::Fahrenheit => UNITS_FAHRENHEIT,
    }
}

/// Convert a 16-bit short UUID into a full UUID.
#[must_use]
pub fn uuid(short: u16) -> uuid::Uuid {
    btleplug::api::bleuuid::uuid_from_u16(short)
}

/// Parse a realtime (`0xFFF4`) payload: one little-endian `u16` per probe,
/// tenths of a degree Celsius. `None` means the probe is unplugged.
#[must_use]
pub fn parse_realtime(data: &[u8]) -> Vec<Option<f32>> {
    data.as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let raw = i16::from_le_bytes([pair[0], pair[1]]);
            if raw as u16 == UNPLUGGED {
                None
            } else {
                Some(f32::from(raw) / 10.0)
            }
        })
        .collect()
}

/// Parse a battery reply (`0xFFF1`): byte 0 = `0x24`, then current voltage and
/// max voltage as little-endian `u16`s. Returns the percentage.
#[must_use]
pub fn parse_battery(data: &[u8]) -> Option<u8> {
    if data.len() < 5 || data[0] != 0x24 {
        return None;
    }
    let current = f32::from(u16::from_le_bytes([data[1], data[2]]));
    let max = match u16::from_le_bytes([data[3], data[4]]) {
        0 => 6550.0,
        value => f32::from(value),
    };
    let percent = (current / max * 100.0).clamp(0.0, 100.0).round();
    Some(percent as u8)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn parses_realtime_payload() {
        let parsed = parse_realtime(&[0xE0, 0x05, 0xF6, 0xFF, 0x2C, 0x06, 0x9C, 0x09]);
        assert_eq!(parsed.len(), 4);
        assert!((parsed[0].unwrap() - 150.4).abs() < 0.001);
        assert!(parsed[1].is_none());
        assert!((parsed[2].unwrap() - 158.0).abs() < 0.001);
        assert!((parsed[3].unwrap() - 246.0).abs() < 0.001);
    }

    #[test]
    fn parses_negative_temperature() {
        // -5.1 °C = -51 tenths -> 0xFFCD
        let parsed = parse_realtime(&[0xCD, 0xFF]);
        assert!((parsed[0].unwrap() + 5.1).abs() < 0.001);
    }

    #[test]
    fn parses_battery_percentage() {
        assert_eq!(parse_battery(&[0x24, 0x10, 0x0E, 0x00, 0x00]), Some(55));
    }

    #[test]
    fn ignores_other_settings_messages() {
        assert_eq!(parse_battery(&[0x01, 0x00, 0x00, 0x00, 0x00]), None);
        assert_eq!(parse_battery(&[0x24, 0x10]), None);
    }
}
