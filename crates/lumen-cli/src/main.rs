//! `lumen` -- set RGB lighting on supported keyboards and mice.

mod menu;
mod paint;
mod usage;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use lumen_core::{BrightnessMode, DeviceSpec, Driver, DriverError, Packet, PowerMode, Registry, Rgb};
use lumen_hid::{Hid, ProbedDevice, access};
use std::time::Duration;

/// How often `--hold` re-sends the colour. Devices that do not latch a direct
/// colour reclaim the LED within a frame; 30ms is fast enough to look solid.
const HOLD_INTERVAL: Duration = Duration::from_millis(30);

#[derive(Parser)]
#[command(name = "lumen", version, about = "Control RGB lighting on keyboards and mice")]
struct Cli {
    /// No subcommand opens the menu. The flags stay complete and unchanged:
    /// the menu is a front-end that builds the same calls.
    #[command(subcommand)]
    command: Option<Command>,
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

    /// Dump every attached HID device and the feature reports each of its
    /// interfaces declares, known to lumen or not. This is where a new device's
    /// `control_report_len` comes from.
    Probe,
}

impl Command {
    /// The name this command is logged under.
    fn name(&self) -> &'static str {
        match self {
            Command::List => "list",
            Command::Set { .. } => "set",
            Command::Probe => "probe",
        }
    }
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
    usage::start();
    let cli = Cli::parse();
    let logged_as = cli.command.as_ref().map_or("menu", Command::name);
    let result = run(cli);
    usage::log(logged_as, result.is_ok());
    result
}

/// Everything after argument parsing, so that a failure to start -- a broken
/// registry, no HID access at all -- is logged as a command that did not serve
/// its answer rather than not logged at all.
fn run(cli: Cli) -> Result<()> {
    let registry = Registry::builtin().context("built-in device registry is broken")?;
    let hid = Hid::new()?;

    let Some(command) = cli.command else {
        return menu::run(&registry, &hid);
    };

    match command {
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

/// Listing devices works without the Input Monitoring grant but setting them
/// does not, so a missing grant prints a healthy-looking table where every write
/// will fail. Say so on stderr, where it cannot spoil a piped table.
fn warn_if_devices_cannot_be_opened() {
    if let Some(fix) = access::input_monitoring().fix() {
        eprintln!(
            "warning: this terminal is not allowed to open HID devices, so nothing \
             below can be set.\n         System Settings -> Privacy & Security -> \
             Input Monitoring: {fix}"
        );
    }
}

fn list(registry: &Registry, hid: &Hid) -> Result<()> {
    let present = hid.present(registry.all())?;
    warn_if_devices_cannot_be_opened();
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

/// What one action came to for one device: the packets to send, a plain
/// description for the line the CLI prints, and the colour that description is
/// about, when there is one. A brightness command has no colour of its own,
/// which is exactly the difference `shown` records.
struct Planned {
    packets: Vec<Packet>,
    what: String,
    shown: Option<Rgb>,
}

/// Turn one action into packets for one device, following the behaviour its
/// registry entry declares rather than anything hardcoded about the device.
fn plan(driver: &dyn Driver, spec: &DeviceSpec, action: Action) -> Result<Planned, DriverError> {
    let (packets, what, shown) = match action {
        Action::Color(c) => (driver.set_color(spec, c)?, c.to_string(), Some(c)),

        Action::Brightness { level, base } => match spec.brightness {
            BrightnessMode::Native => (
                driver.set_brightness(spec, level)?,
                format!("brightness {level}%"),
                None,
            ),
            BrightnessMode::Scaled => {
                let dimmed = base.scaled(level);
                (
                    driver.set_color(spec, dimmed)?,
                    format!("{dimmed} ({base} at {level}%)"),
                    Some(dimmed),
                )
            }
        },

        Action::Power(on) => {
            let word = if on { "on" } else { "off" };
            match spec.power {
                PowerMode::Native => (driver.set_power(spec, on)?, word.to_string(), None),
                PowerMode::ColorBlack => {
                    let c = if on { Rgb::WHITE } else { Rgb::BLACK };
                    (driver.set_color(spec, c)?, format!("{word} ({c})"), Some(c))
                }
            }
        }
    };
    Ok(Planned {
        packets,
        what,
        shown,
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
        let planned =
            plan(driver.as_ref(), spec, action).with_context(|| format!("device `{}`", spec.id))?;
        work.push((*spec, planned));
    }

    // A colour is worth showing, not only naming; `swatch` is empty whenever
    // the output is not a terminal, so a piped line is unchanged.
    let describe = |p: &Planned| {
        let swatch = p.shown.map(paint::swatch).unwrap_or_default();
        format!("{swatch}{}", p.what)
    };

    if dry_run {
        for (spec, planned) in &work {
            println!("{} -> {}", spec.name, describe(planned));
            for p in &planned.packets {
                println!("  report 0x{:02x}, {} bytes", p.report_id(), p.bytes.len());
                println!("  {}", p.hex());
            }
        }
        return Ok(());
    }

    let mut connections = Vec::new();
    for (spec, planned) in work {
        let conn = hid.open(spec)?;
        conn.send_all(&planned.packets)?;
        println!(
            "{} -> {} (interface {}, report 0x{:02x})",
            spec.name,
            describe(&planned),
            conn.interface,
            conn.report_id
        );
        connections.push((spec, conn, planned.packets));
    }

    let volatile: Vec<&str> = connections
        .iter()
        .filter(|(spec, _, _)| !spec.holds_colour)
        .map(|(spec, _, _)| spec.name.as_str())
        .collect();

    if hold {
        println!("holding colour every {HOLD_INTERVAL:?} -- press Ctrl-C to stop");
        // The colour is on the device now; what follows is only keeping it
        // there until Ctrl-C, which never returns to `main` to be logged.
        usage::log("set", true);
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

/// What a probed device calls itself, or a stand-in when it says nothing.
fn describe(device: &ProbedDevice) -> String {
    let name = format!("{} {}", device.manufacturer, device.product);
    match name.trim() {
        "" => "unnamed device".to_string(),
        named => named.to_string(),
    }
}

/// Report every attached device and what each interface declares, so a device
/// lumen has never seen can be added to the registry from measurements instead
/// of guesses. Devices already in the registry are marked as such -- being able
/// to compare a known device against the hardware is half of what makes the
/// unknown one readable.
fn probe(registry: &Registry, hid: &Hid) -> Result<()> {
    warn_if_devices_cannot_be_opened();
    for device in hid.scan()? {
        let known = match registry.by_usb_id(device.vendor_id, device.product_id) {
            Some(spec) => format!("in the registry as `{}`, driver `{}`", spec.id, spec.driver),
            None => "not in the registry".to_string(),
        };
        println!(
            "{:04x}:{:04x}  {}  -- {known}",
            device.vendor_id,
            device.product_id,
            describe(&device)
        );

        for interface in &device.interfaces {
            let what = match &interface.error {
                Some(e) => format!("unreadable ({e})"),
                None if interface.feature_reports.is_empty() => "no feature reports".to_string(),
                None => interface
                    .feature_reports
                    .iter()
                    .map(|(id, len)| format!("feature report 0x{id:02x}, {len} data bytes"))
                    .collect::<Vec<_>>()
                    .join("; "),
            };
            // hidapi reports -1 for anything with no USB interface number of
            // its own, a Bluetooth peripheral for instance.
            match interface.interface {
                n if n < 0 => println!("  no interface number: {what}"),
                n => println!("  interface {n}: {what}"),
            }
        }
    }

    println!(
        "\nA device's control interface is the one declaring a big vendor feature report \
         -- 90 data bytes on a Razer, 263 on the HyperX. That byte count is the \
         `control_report_len` its `[[device]]` block needs. An interface reported as \
         unreadable is usually the Input Monitoring grant rather than the device."
    );
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
        let planned = plan(driver.as_ref(), &spec, action).unwrap();
        (planned.packets, planned.what)
    }

    /// A colour is offered to the terminal exactly when the action has one to
    /// show: a native brightness command does not, a scaled one does, because
    /// there the brightness *is* a dimmed colour.
    fn shown(usb: (u16, u16), action: Action) -> Option<Rgb> {
        let spec = spec(usb);
        let driver = lumen_devices::driver_for(&spec.driver).unwrap();
        plan(driver.as_ref(), &spec, action).unwrap().shown
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
    fn a_colour_is_offered_for_display_only_when_the_action_has_one() {
        let red = Rgb::new(255, 0, 0);
        assert_eq!(shown(KEYBOARD, Action::Color(red)), Some(red));
        // The keyboard has a brightness command, so nothing about the colour
        // changed and there is nothing to show.
        assert_eq!(
            shown(KEYBOARD, Action::Brightness { level: 40, base: red }),
            None
        );
        // The mouse reaches brightness by dimming the colour, so the dimmed
        // colour is what actually went to the device.
        assert_eq!(
            shown(MOUSE, Action::Brightness { level: 40, base: red }),
            Some(red.scaled(40))
        );
        assert_eq!(shown(KEYBOARD, Action::Power(false)), None);
        assert_eq!(shown(MOUSE, Action::Power(false)), Some(Rgb::BLACK));
        assert_eq!(shown(MOUSE, Action::Power(true)), Some(Rgb::WHITE));
    }

    /// Bare `lumen` is the menu, and adding it must not have cost the flag
    /// surface anything: agents and scripts keep using the subcommands.
    #[test]
    fn a_bare_invocation_opens_the_menu_without_touching_the_flags() {
        assert!(
            Cli::try_parse_from(["lumen"]).unwrap().command.is_none(),
            "bare `lumen` must parse to no subcommand"
        );
        assert!(matches!(
            Cli::try_parse_from(["lumen", "set", "keyboard", "--color", "red"])
                .unwrap()
                .command,
            Some(Command::Set { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from(["lumen", "list"]).unwrap().command,
            Some(Command::List)
        ));
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
