//! `lumen` -- set RGB lighting on supported keyboards and mice.

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use lumen_core::{BrightnessMode, DeviceSpec, Driver, DriverError, Packet, PowerMode, Registry, Rgb};
use lumen_hid::Hid;
use std::time::Duration;

/// How often `--hold` re-sends the colour. Devices that do not latch a direct
/// colour reclaim the LED within a frame; 30ms is fast enough to look solid.
const HOLD_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Parser)]
#[command(name = "lumen", version, about = "Control RGB lighting on keyboards and mice")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show supported devices and which are plugged in.
    List,

    /// Set a colour, a brightness, or turn the lighting off and on.
    Set {
        /// `all`, a device id from `lumen list`, or a kind such as `mouse`.
        target: String,

        /// Colour name or hex, e.g. `red`, `#ff0080`, `off`.
        #[arg(short, long, default_value = "red")]
        color: String,

        /// Brightness percent, 0-100. On a device with no brightness command of
        /// its own this dims --color instead; on one that has it, the colour is
        /// left alone.
        #[arg(short, long, value_name = "PERCENT",
              value_parser = clap::value_parser!(u8).range(0..=100),
              conflicts_with_all = ["off", "on"])]
        brightness: Option<u8>,

        /// Turn the lighting off.
        #[arg(long, conflicts_with = "on")]
        off: bool,

        /// Turn the lighting back on. The device does not report the colour it
        /// had, so this sets static white at full brightness.
        #[arg(long)]
        on: bool,

        /// Print the exact packet bytes instead of sending anything.
        #[arg(long)]
        dry_run: bool,

        /// Keep re-sending so devices that do not latch a colour stay lit.
        /// Runs until interrupted with Ctrl-C.
        #[arg(long)]
        hold: bool,
    },

    /// Dump the feature reports each interface declares. Use this when adding a
    /// device to the registry to find its `control_report_len`.
    Probe,
}

/// What one `lumen set` run does to every device it targets.
#[derive(Debug, Clone, Copy)]
enum Action {
    Color(Rgb),
    /// A percentage, plus the colour to dim for devices that reach brightness
    /// by scaling rather than by a command.
    Brightness { level: u8, base: Rgb },
    /// True is on, false is off.
    Power(bool),
}

impl Action {
    /// Only one of `--off`, `--on` and `--brightness` can be given at a time;
    /// clap rejects the combinations before this runs.
    fn from_flags(color: &str, brightness: Option<u8>, off: bool, on: bool) -> Result<Self> {
        let base: Rgb = color.parse()?;
        Ok(match (off, on, brightness) {
            (true, _, _) => Action::Power(false),
            (_, true, _) => Action::Power(true),
            (_, _, Some(level)) => Action::Brightness { level, base },
            _ => Action::Color(base),
        })
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let registry = Registry::builtin().context("built-in device registry is broken")?;
    let hid = Hid::new()?;

    match cli.command {
        Command::List => list(&registry, &hid),
        Command::Set {
            target,
            color,
            brightness,
            off,
            on,
            dry_run,
            hold,
        } => {
            let action = Action::from_flags(&color, brightness, off, on)?;
            set(&registry, &hid, &target, action, dry_run, hold)
        }
        Command::Probe => probe(&registry, &hid),
    }
}

fn list(registry: &Registry, hid: &Hid) -> Result<()> {
    let present = hid.present(registry.all())?;
    println!("{:<24} {:<24} {:<9} STATUS", "ID", "DEVICE", "KIND");
    for spec in registry.all() {
        let here = present.iter().any(|p| p.id == spec.id);
        let status = match (here, spec.holds_colour) {
            (false, _) => "not plugged in".to_string(),
            (true, true) => "connected".to_string(),
            (true, false) => "connected (needs --hold)".to_string(),
        };
        println!(
            "{:<24} {:<24} {:<9} {}",
            spec.id, spec.name, spec.kind, status
        );
    }
    Ok(())
}

/// Turn one action into packets for one device, following the behaviour its
/// registry entry declares rather than anything hardcoded about the device.
/// Returns the packets and a plain description of what they do, for the line
/// the CLI prints.
fn plan(
    driver: &dyn Driver,
    spec: &DeviceSpec,
    action: Action,
) -> Result<(Vec<Packet>, String), DriverError> {
    Ok(match action {
        Action::Color(c) => (driver.set_color(spec, c)?, c.to_string()),

        Action::Brightness { level, base } => match spec.brightness {
            BrightnessMode::Native => (
                driver.set_brightness(spec, level)?,
                format!("brightness {level}%"),
            ),
            BrightnessMode::Scaled => {
                let dimmed = base.scaled(level);
                (
                    driver.set_color(spec, dimmed)?,
                    format!("{dimmed} ({base} at {level}%)"),
                )
            }
        },

        Action::Power(on) => {
            let word = if on { "on" } else { "off" };
            match spec.power {
                PowerMode::Native => (driver.set_power(spec, on)?, word.to_string()),
                PowerMode::ColorBlack => {
                    let c = if on { Rgb::WHITE } else { Rgb::BLACK };
                    (driver.set_color(spec, c)?, format!("{word} ({c})"))
                }
            }
        }
    })
}

fn set(
    registry: &Registry,
    hid: &Hid,
    target: &str,
    action: Action,
    dry_run: bool,
    hold: bool,
) -> Result<()> {
    let present = hid.present(registry.all())?;
    let targets = registry.resolve(target, &present);

    if targets.is_empty() {
        let names: Vec<&str> = present.iter().map(|d| d.id.as_str()).collect();
        if names.is_empty() {
            bail!("no supported devices are plugged in");
        }
        bail!(
            "`{target}` matched nothing. Connected devices: {}, or use `all`",
            names.join(", ")
        );
    }

    // Build every packet before opening anything, so a protocol error cannot
    // leave some devices set and others not.
    let mut work = Vec::new();
    for spec in &targets {
        let driver = lumen_devices::driver_for(&spec.driver)
            .with_context(|| format!("device `{}`", spec.id))?;
        let (packets, what) =
            plan(driver.as_ref(), spec, action).with_context(|| format!("device `{}`", spec.id))?;
        work.push((*spec, packets, what));
    }

    if dry_run {
        for (spec, packets, what) in &work {
            println!("{} -> {what}", spec.name);
            for p in packets {
                println!("  report 0x{:02x}, {} bytes", p.report_id(), p.bytes.len());
                println!("  {}", p.hex());
            }
        }
        return Ok(());
    }

    let mut connections = Vec::new();
    for (spec, packets, what) in work {
        let conn = hid.open(spec)?;
        conn.send_all(&packets)?;
        println!(
            "{} -> {what} (interface {}, report 0x{:02x})",
            spec.name, conn.interface, conn.report_id
        );
        connections.push((spec, conn, packets));
    }

    let volatile: Vec<&str> = connections
        .iter()
        .filter(|(spec, _, _)| !spec.holds_colour)
        .map(|(spec, _, _)| spec.name.as_str())
        .collect();

    if hold {
        println!("holding colour every {HOLD_INTERVAL:?} -- press Ctrl-C to stop");
        loop {
            for (_, conn, packets) in &connections {
                conn.send_all(packets)?;
            }
            std::thread::sleep(HOLD_INTERVAL);
        }
    } else if !volatile.is_empty() {
        println!(
            "note: {} does not latch a colour, so it has already reverted. \
             Re-run with --hold to make it stick.",
            volatile.join(", ")
        );
    }
    Ok(())
}

fn probe(registry: &Registry, hid: &Hid) -> Result<()> {
    let present = hid.present(registry.all())?;
    if present.is_empty() {
        println!("No supported devices plugged in.");
        return Ok(());
    }
    for spec in present {
        match hid.open(spec) {
            Ok(conn) => println!(
                "{}: control report 0x{:02x} of {} data bytes on interface {}",
                spec.name, conn.report_id, spec.control_report_len, conn.interface
            ),
            Err(e) => println!("{}: {e}", spec.name),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn spec(usb: (u16, u16)) -> DeviceSpec {
        Registry::builtin()
            .unwrap()
            .by_usb_id(usb.0, usb.1)
            .expect("device in registry")
            .clone()
    }

    fn packets(usb: (u16, u16), action: Action) -> (Vec<Packet>, String) {
        let spec = spec(usb);
        let driver = lumen_devices::driver_for(&spec.driver).unwrap();
        plan(driver.as_ref(), &spec, action).unwrap()
    }

    const KEYBOARD: (u16, u16) = (0x1532, 0x023F);
    const MOUSE: (u16, u16) = (0x03F0, 0x0D8F);

    #[test]
    fn brightness_uses_the_native_command_where_the_registry_declares_one() {
        let kb = spec(KEYBOARD);
        let (built, what) = packets(
            KEYBOARD,
            Action::Brightness {
                level: 40,
                base: Rgb::new(255, 0, 0),
            },
        );
        let native = lumen_devices::driver_for(&kb.driver)
            .unwrap()
            .set_brightness(&kb, 40)
            .unwrap();
        assert_eq!(built, native);
        assert_eq!(what, "brightness 40%");
    }

    #[test]
    fn brightness_scales_the_colour_where_the_registry_declares_no_command() {
        let (built, what) = packets(
            MOUSE,
            Action::Brightness {
                level: 40,
                base: Rgb::new(255, 0, 0),
            },
        );
        let mouse = spec(MOUSE);
        let scaled = lumen_devices::driver_for(&mouse.driver)
            .unwrap()
            .set_color(&mouse, Rgb::new(102, 0, 0))
            .unwrap();
        assert_eq!(built, scaled);
        assert_eq!(what, "#660000 (#ff0000 at 40%)");
    }

    #[test]
    fn power_uses_the_native_command_where_the_registry_declares_one() {
        let kb = spec(KEYBOARD);
        let (built, what) = packets(KEYBOARD, Action::Power(false));
        let native = lumen_devices::driver_for(&kb.driver)
            .unwrap()
            .set_power(&kb, false)
            .unwrap();
        assert_eq!(built, native);
        assert_eq!(what, "off");
    }

    #[test]
    fn power_falls_back_to_black_and_white_where_the_registry_declares_no_command() {
        let mouse = spec(MOUSE);
        let driver = lumen_devices::driver_for(&mouse.driver).unwrap();

        let (off, what) = packets(MOUSE, Action::Power(false));
        assert_eq!(off, driver.set_color(&mouse, Rgb::BLACK).unwrap());
        assert_eq!(what, "off (#000000)");

        let (on, what) = packets(MOUSE, Action::Power(true));
        assert_eq!(on, driver.set_color(&mouse, Rgb::WHITE).unwrap());
        assert_eq!(what, "on (#ffffff)");
    }

    #[test]
    fn a_plain_colour_is_unaffected_by_the_new_commands() {
        let (built, what) = packets(KEYBOARD, Action::Color(Rgb::new(255, 0, 0)));
        let kb = spec(KEYBOARD);
        let direct = lumen_devices::driver_for(&kb.driver)
            .unwrap()
            .set_color(&kb, Rgb::new(255, 0, 0))
            .unwrap();
        assert_eq!(built, direct);
        assert_eq!(what, "#ff0000");
    }

    #[test]
    fn flags_pick_one_action_with_power_winning_over_brightness() {
        assert!(matches!(
            Action::from_flags("blue", None, false, false).unwrap(),
            Action::Color(Rgb { b: 255, .. })
        ));
        assert!(matches!(
            Action::from_flags("red", Some(20), false, false).unwrap(),
            Action::Brightness { level: 20, .. }
        ));
        assert!(matches!(
            Action::from_flags("red", None, true, false).unwrap(),
            Action::Power(false)
        ));
        assert!(matches!(
            Action::from_flags("red", None, false, true).unwrap(),
            Action::Power(true)
        ));
        assert!(Action::from_flags("burgundy", None, false, false).is_err());
    }

    #[test]
    fn the_cli_refuses_contradictory_flags_and_out_of_range_brightness() {
        let parse = |args: &[&str]| Cli::command().try_get_matches_from(args);
        assert!(parse(&["lumen", "set", "all", "--off", "--on"]).is_err());
        assert!(parse(&["lumen", "set", "all", "--off", "--brightness", "50"]).is_err());
        assert!(parse(&["lumen", "set", "all", "--on", "--brightness", "50"]).is_err());
        assert!(parse(&["lumen", "set", "all", "--brightness", "101"]).is_err());
        assert!(parse(&["lumen", "set", "all", "--brightness", "100"]).is_ok());
        // A colour still works alongside brightness: scaled devices need it.
        assert!(parse(&["lumen", "set", "all", "--color", "blue", "-b", "40"]).is_ok());
    }
}
