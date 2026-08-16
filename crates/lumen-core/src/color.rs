use thiserror::Error;

/// A 24-bit RGB colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb { r: 0, g: 0, b: 0 };
    pub const WHITE: Rgb = Rgb {
        r: 255,
        g: 255,
        b: 255,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb { r, g, b }
    }

    /// This colour at `level` percent, each channel scaled and rounded to
    /// nearest. Levels above 100 are treated as 100.
    ///
    /// This is how brightness is applied to a device whose protocol has no
    /// brightness command of its own.
    pub fn scaled(self, level: u8) -> Rgb {
        let level = u16::from(level.min(100));
        let scale = |c: u8| ((u16::from(c) * level + 50) / 100) as u8;
        Rgb::new(scale(self.r), scale(self.g), scale(self.b))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ColorParseError {
    #[error("`{0}` is not a colour: use a name like `red` or a hex value like `#ff0080`")]
    Unrecognised(String),
    #[error("`{0}` is not valid hex")]
    BadHex(String),
}

/// Names accepted alongside hex, so the CLI is usable without looking up codes.
const NAMED: &[(&str, Rgb)] = &[
    ("black", Rgb::new(0, 0, 0)),
    ("off", Rgb::new(0, 0, 0)),
    ("white", Rgb::new(255, 255, 255)),
    ("red", Rgb::new(255, 0, 0)),
    ("green", Rgb::new(0, 255, 0)),
    ("blue", Rgb::new(0, 0, 255)),
    ("yellow", Rgb::new(255, 255, 0)),
    ("cyan", Rgb::new(0, 255, 255)),
    ("magenta", Rgb::new(255, 0, 255)),
    ("orange", Rgb::new(255, 96, 0)),
    ("purple", Rgb::new(128, 0, 255)),
];

/// Every colour name the parser accepts, so a menu can offer the list instead of
/// expecting the user to know it. `black` and `off` are the same colour under
/// two names, and both are here: the parser accepts either.
pub fn named_colors() -> &'static [(&'static str, Rgb)] {
    NAMED
}

impl std::str::FromStr for Rgb {
    type Err = ColorParseError;

    /// Accepts `red`, `#ff0080`, `ff0080` and the three-digit form `#f08`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let lower = t.to_ascii_lowercase();
        if let Some((_, c)) = NAMED.iter().find(|(n, _)| *n == lower) {
            return Ok(*c);
        }

        let hex = lower.strip_prefix('#').unwrap_or(&lower);
        let looks_hex = !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit());
        if !looks_hex {
            return Err(ColorParseError::Unrecognised(t.to_string()));
        }
        let digit = |c: char| c.to_digit(16).unwrap() as u8;
        let chars: Vec<char> = hex.chars().collect();
        match chars.len() {
            // #f08 -> #ff0088
            3 => Ok(Rgb::new(
                digit(chars[0]) * 17,
                digit(chars[1]) * 17,
                digit(chars[2]) * 17,
            )),
            6 => Ok(Rgb::new(
                digit(chars[0]) * 16 + digit(chars[1]),
                digit(chars[2]) * 16 + digit(chars[3]),
                digit(chars[4]) * 16 + digit(chars[5]),
            )),
            _ => Err(ColorParseError::BadHex(t.to_string())),
        }
    }
}

impl std::fmt::Display for Rgb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn parses_names_case_insensitively() {
        assert_eq!(Rgb::from_str("red").unwrap(), Rgb::new(255, 0, 0));
        assert_eq!(Rgb::from_str("  GREEN ").unwrap(), Rgb::new(0, 255, 0));
        assert_eq!(Rgb::from_str("off").unwrap(), Rgb::BLACK);
    }

    #[test]
    fn parses_hex_with_and_without_hash() {
        assert_eq!(Rgb::from_str("#ff0080").unwrap(), Rgb::new(255, 0, 128));
        assert_eq!(Rgb::from_str("FF0080").unwrap(), Rgb::new(255, 0, 128));
    }

    #[test]
    fn expands_short_hex() {
        assert_eq!(Rgb::from_str("#f08").unwrap(), Rgb::new(255, 0, 136));
    }

    #[test]
    fn rejects_nonsense() {
        assert!(Rgb::from_str("burgundy").is_err());
        // Hex digits but the wrong length: report it as bad hex, not unknown name.
        assert_eq!(
            Rgb::from_str("#ff00").unwrap_err(),
            ColorParseError::BadHex("#ff00".into())
        );
    }

    #[test]
    fn displays_as_hex() {
        assert_eq!(Rgb::new(255, 0, 128).to_string(), "#ff0080");
    }

    #[test]
    fn scaling_keeps_the_ends_exact() {
        let c = Rgb::new(255, 128, 1);
        assert_eq!(c.scaled(100), c, "full brightness must not alter a colour");
        assert_eq!(c.scaled(0), Rgb::BLACK, "zero brightness is black");
    }

    #[test]
    fn scaling_rounds_to_nearest() {
        // 255 * 50% = 127.5 -> 128, 1 * 50% = 0.5 -> 1.
        assert_eq!(Rgb::new(255, 100, 1).scaled(50), Rgb::new(128, 50, 1));
        // 255 * 33% = 84.15 -> 84.
        assert_eq!(Rgb::new(255, 0, 0).scaled(33), Rgb::new(84, 0, 0));
    }

    #[test]
    fn scaling_treats_over_100_as_100() {
        let c = Rgb::new(10, 20, 30);
        assert_eq!(c.scaled(255), c);
    }
}
