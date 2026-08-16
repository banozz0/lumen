//! Showing a colour instead of only naming it.
//!
//! A tool whose whole subject is colour should not describe `#8000ff` in grey.
//! Two rules keep that from becoming a nuisance: never write escape codes into
//! something that is not a terminal -- a pipe, a file, a redirect into `jq` --
//! and honour `NO_COLOR`, which is the convention for turning this off.
//!
//! With colour off every function here returns a plain string, so output stays
//! exactly what it was before any of this existed.

use lumen_core::Rgb;
use std::io::IsTerminal;
use std::sync::OnceLock;

/// Decided once: the terminal does not stop being a terminal mid-run.
fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
    })
}

/// A block of the colour itself, to sit before the name or hex that follows it.
/// Empty when colour is off, so callers can prefix it unconditionally.
///
/// The colour goes in the *background* rather than the text: white text on a
/// white background is unreadable, and a palette has to show the pale ones as
/// clearly as the bright ones.
pub fn swatch(c: Rgb) -> String {
    if !enabled() {
        return String::new();
    }
    format!("\x1b[48;2;{};{};{}m  \x1b[0m ", c.r, c.g, c.b)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests do not run against a terminal, so this also pins the property that
    /// matters most: piped output carries no escape codes.
    #[test]
    fn a_swatch_is_empty_when_there_is_no_terminal_to_colour() {
        assert_eq!(swatch(Rgb::new(1, 2, 3)), "");
    }
}
