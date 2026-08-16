//! One JSON line per command that reached an answer.
//!
//! ```json
//! {"ts":"2026-08-16T07:45:02.113Z","cmd":"set","src":"cli","ok":true,"ms":12}
//! ```
//!
//! The log is local: nothing is ever sent anywhere. It goes to `usage.jsonl` in
//! the repo the binary was built from, or wherever `LUMEN_USAGE_LOG` points;
//! setting that variable to nothing at all turns logging off.
//!
//! Two rules from the contract this follows. Log when the answer lands, not
//! when the command is typed -- so `--hold` logs the moment the colour is on the
//! device, not when the user eventually presses Ctrl-C. And fail silent:
//! telemetry that breaks a command is worse than no telemetry.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Set once at startup so any code path can log without a clock being threaded
/// through arguments that have nothing to do with telemetry.
static START: OnceLock<Instant> = OnceLock::new();

pub fn start() {
    let _ = START.set(Instant::now());
}

/// Record a served command, timed from startup.
pub fn log(cmd: &str, ok: bool) {
    let ms = START.get().map_or(0, |s| s.elapsed().as_millis());
    write(cmd, ok, ms);
}

/// Record one action out of a menu session, timed from when that action began
/// rather than from startup -- a menu run is many commands, and each one's
/// duration is its own.
pub fn log_since(cmd: &str, ok: bool, started: Instant) {
    write(cmd, ok, started.elapsed().as_millis());
}

/// `cmd` is always one of lumen's own fixed names -- a subcommand, or `menu` for
/// a menu session -- never anything the user typed, so the line needs no JSON
/// escaping.
fn write(cmd: &str, ok: bool, ms: u128) {
    let Some(path) = log_path() else { return };
    let line = line(&timestamp(SystemTime::now()), cmd, ok, ms);
    let _ = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

/// Where the line goes, or `None` when logging is off.
fn log_path() -> Option<PathBuf> {
    match std::env::var("LUMEN_USAGE_LOG") {
        Ok(p) if p.is_empty() => None,
        Ok(p) => Some(PathBuf::from(p)),
        // The repo root is two levels above this crate. A binary whose repo has
        // been moved or deleted logs nothing rather than creating stray files.
        Err(_) => {
            let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent()?.parent()?;
            root.is_dir().then(|| root.join("usage.jsonl"))
        }
    }
}

fn line(ts: &str, cmd: &str, ok: bool, ms: u128) -> String {
    format!("{{\"ts\":\"{ts}\",\"cmd\":\"{cmd}\",\"src\":\"cli\",\"ok\":{ok},\"ms\":{ms}}}\n")
}

/// UTC, in the shape the contract's example uses: `2026-08-14T07:45:02.113Z`.
fn timestamp(now: SystemTime) -> String {
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap_or_default();
    let (secs, millis) = (since_epoch.as_secs(), since_epoch.subsec_millis());
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let time_of_day = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60
    )
}

/// Days since 1970-01-01 to a civil date, by Howard Hinnant's algorithm: shift
/// the year to start in March so a leap day falls at the end of it and every
/// other month length becomes a formula.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(unix_secs: u64, millis: u32) -> String {
        timestamp(UNIX_EPOCH + Duration::new(unix_secs, millis * 1_000_000))
    }

    #[test]
    fn the_line_matches_the_contract() {
        assert_eq!(
            line("2026-08-14T07:45:02.113Z", "set", true, 12),
            "{\"ts\":\"2026-08-14T07:45:02.113Z\",\"cmd\":\"set\",\"src\":\"cli\",\"ok\":true,\"ms\":12}\n"
        );
        assert!(line("t", "probe", false, 0).contains("\"ok\":false"));
    }

    #[test]
    fn timestamps_are_utc_iso_8601() {
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(1_755_000_000, 113), "2025-08-12T12:00:00.113Z");
    }

    /// The month arithmetic is the part that goes wrong, so check the seams:
    /// a leap day, the day after it, and both ends of a year.
    #[test]
    fn dates_survive_leap_years_and_year_boundaries() {
        assert_eq!(at(1_709_164_800, 0), "2024-02-29T00:00:00.000Z");
        assert_eq!(at(1_709_251_200, 0), "2024-03-01T00:00:00.000Z");
        assert_eq!(at(1_735_689_599, 999), "2024-12-31T23:59:59.999Z");
        assert_eq!(at(1_735_689_600, 0), "2025-01-01T00:00:00.000Z");
        // 2000 was a leap year, 1900 and 2100 are not; the era maths is what
        // gets that right.
        assert_eq!(at(951_782_400, 0), "2000-02-29T00:00:00.000Z");
    }
}
