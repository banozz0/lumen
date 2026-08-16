//! The menu `lumen` opens when it is run with no arguments.
//!
//! It is a front-end and nothing more: every screen ends by building the same
//! [`Action`] the flags build and calling the same functions, so the menu can
//! never do something the flag surface cannot, or do it differently.
//!
//! The shape is fixed by convention rather than by taste. Numbered lists all the
//! way down, `0` is back everywhere and exit at the root, anything lumen can
//! enumerate is offered as a list rather than typed, free text only where a list
//! genuinely cannot carry the answer (a hex colour, a percentage), and after
//! every action the tool returns to the menu instead of dropping to the shell.

use crate::{Action, set};
use anyhow::Result;
use lumen_core::{DeviceSpec, Registry, Rgb, named_colors};
use lumen_hid::Hid;
use std::io::Write;

/// What a numbered screen came back with.
enum Choice {
    /// Zero-based index into the items the screen offered.
    Item(usize),
    /// `0`, or end of input: go back a level, or leave at the root.
    Back,
}

/// Print a prompt and read one line. `None` means the input ended -- a piped
/// script running out, or Ctrl-D -- which is treated as leaving, never as an
/// answer.
fn ask(prompt: &str) -> Option<String> {
    print!("{prompt}");
    std::io::stdout().flush().ok()?;
    let mut line = String::new();
    match std::io::stdin().read_line(&mut line) {
        Ok(0) | Err(_) => {
            println!();
            None
        }
        Ok(_) => Some(line.trim().to_string()),
    }
}

/// Show a numbered list and take a pick. Re-asks until the answer is one of the
/// numbers on screen, so a typo never costs a level.
fn choose(title: &str, items: &[String], zero: &str) -> Choice {
    loop {
        println!("\n{title}\n");
        for (i, item) in items.iter().enumerate() {
            println!("  {}  {item}", i + 1);
        }
        println!("  0  {zero}");

        let Some(answer) = ask("\n> ") else {
            return Choice::Back;
        };
        match answer.parse::<usize>() {
            Ok(0) => return Choice::Back,
            Ok(n) if n <= items.len() => return Choice::Item(n - 1),
            _ => println!("\nPick a number from the list."),
        }
    }
}

/// Pause on a finished action, then go back to the menu. Rule: the tool loops;
/// it does not dump you back in the shell after one job.
fn after_action() -> bool {
    matches!(ask("\nEnter = menu, 0 = exit > "), Some(a) if a != "0")
}

/// The devices a command can actually be aimed at right now, plus `all` when
/// there is more than one. Nothing not plugged in is ever offered.
fn targets<'a>(registry: &'a Registry, hid: &Hid) -> Vec<(String, &'a str)> {
    let present = hid.present(registry.all()).unwrap_or_default();
    let mut out: Vec<(String, &str)> = present
        .iter()
        .map(|spec| (label(spec), spec.id.as_str()))
        .collect();
    if out.len() > 1 {
        out.push(("Everything connected".to_string(), "all"));
    }
    out
}

fn label(spec: &DeviceSpec) -> String {
    if spec.holds_colour {
        format!("{} ({})", spec.name, spec.kind)
    } else {
        format!("{} ({}, does not keep a colour)", spec.name, spec.kind)
    }
}

/// Pick a device, or `None` to go back. Says so plainly when there is nothing to
/// pick rather than showing an empty list.
fn pick_target(registry: &Registry, hid: &Hid) -> Option<String> {
    let choices = targets(registry, hid);
    if choices.is_empty() {
        println!("\nNothing lumen can drive is plugged in. `Show what is connected` explains more.");
        return None;
    }
    let labels: Vec<String> = choices.iter().map(|(l, _)| l.clone()).collect();
    match choose("Which device?", &labels, "Back") {
        Choice::Item(i) => Some(choices[i].1.to_string()),
        Choice::Back => None,
    }
}

/// Pick a colour by name, or type a hex value at the one leaf where a list
/// cannot carry the answer.
fn pick_color() -> Option<Rgb> {
    // `off` is the same colour as `black` under a second name, and the menu has
    // its own "turn the lighting off", so it is left out of the palette.
    let palette: Vec<(&str, Rgb)> = named_colors()
        .iter()
        .filter(|(name, _)| *name != "off")
        .copied()
        .collect();

    let mut labels: Vec<String> = palette.iter().map(|(name, _)| (*name).to_string()).collect();
    labels.push("Type a hex value".to_string());

    loop {
        match choose("Which colour?", &labels, "Back") {
            Choice::Back => return None,
            Choice::Item(i) if i < palette.len() => return Some(palette[i].1),
            Choice::Item(_) => {
                let typed = ask("\nHex value, e.g. #ff0080 (0 = back) > ")?;
                if typed == "0" {
                    continue;
                }
                match typed.parse::<Rgb>() {
                    Ok(rgb) => return Some(rgb),
                    Err(e) => println!("\n{e}"),
                }
            }
        }
    }
}

/// A percentage, with the value Enter would keep shown in brackets.
fn pick_brightness() -> Option<u8> {
    loop {
        let typed = ask("\nBrightness percent 0-100 [100], 0 = back > ")?;
        if typed.is_empty() {
            return Some(100);
        }
        if typed == "0" {
            return None;
        }
        match typed.parse::<u8>() {
            Ok(level) if level <= 100 => return Some(level),
            _ => println!("\nA whole number from 1 to 100, or Enter for 100."),
        }
    }
}

/// Run an action the menu has finished assembling, and report what happened.
/// The error is printed rather than returned: one mistyped colour should not end
/// the session.
fn apply(registry: &Registry, hid: &Hid, target: &str, action: Action, hold: bool) -> bool {
    let started = std::time::Instant::now();
    let outcome = set(registry, hid, target, action, false, hold);
    if let Err(e) = &outcome {
        println!("\n{e:#}");
    }
    crate::usage::log_since("set", outcome.is_ok(), started);
    outcome.is_ok()
}

/// Offer to keep re-sending, but only for a device that needs it -- and say what
/// that costs, because it runs until Ctrl-C.
fn hold_wanted(registry: &Registry, hid: &Hid, target: &str) -> bool {
    let present = hid.present(registry.all()).unwrap_or_default();
    let volatile = registry
        .resolve(target, &present)
        .iter()
        .any(|spec| !spec.holds_colour);
    if !volatile {
        return false;
    }
    matches!(
        choose(
            "This device drops the colour the moment lumen stops sending it.",
            &["Hold it lit until I press Ctrl-C".to_string()],
            "Just send it once",
        ),
        Choice::Item(_)
    )
}

/// Device, then colour. `0` at the colour list steps back to the device list
/// rather than out of the flow: back is always one level, everywhere.
/// Returns whether anything was actually sent.
fn colour_flow(registry: &Registry, hid: &Hid) -> bool {
    loop {
        let Some(target) = pick_target(registry, hid) else {
            return false;
        };
        let Some(rgb) = pick_color() else { continue };
        let hold = hold_wanted(registry, hid, &target);
        apply(registry, hid, &target, Action::Color(rgb), hold);
        return true;
    }
}

/// Device, then percentage, with the same one-level-back rule.
fn brightness_flow(registry: &Registry, hid: &Hid) -> bool {
    loop {
        let Some(target) = pick_target(registry, hid) else {
            return false;
        };
        let Some(level) = pick_brightness() else {
            continue;
        };
        // A device with no brightness command of its own dims a colour instead,
        // and red is what the flags default to.
        let action = Action::Brightness {
            level,
            base: Rgb::new(255, 0, 0),
        };
        apply(registry, hid, &target, action, false);
        return true;
    }
}

/// The menu, until the user leaves it.
pub fn run(registry: &Registry, hid: &Hid) -> Result<()> {
    let entries = [
        "Set a colour".to_string(),
        "Set brightness".to_string(),
        "Turn the lighting off".to_string(),
        "Turn the lighting on".to_string(),
        "Show what is connected".to_string(),
        "Inspect the hardware".to_string(),
    ];

    loop {
        let Choice::Item(picked) = choose("lumen -- RGB lighting", &entries, "Exit") else {
            return Ok(());
        };

        let acted = match picked {
            0 => colour_flow(registry, hid),
            1 => brightness_flow(registry, hid),

            2 | 3 => {
                let on = picked == 3;
                match pick_target(registry, hid) {
                    Some(target) => {
                        apply(registry, hid, &target, Action::Power(on), false);
                        true
                    }
                    None => false,
                }
            }

            4 => {
                let started = std::time::Instant::now();
                println!();
                let outcome = crate::list(registry, hid);
                if let Err(e) = &outcome {
                    println!("\n{e:#}");
                }
                crate::usage::log_since("list", outcome.is_ok(), started);
                true
            }

            _ => {
                let started = std::time::Instant::now();
                println!();
                let outcome = crate::probe(registry, hid);
                if let Err(e) = &outcome {
                    println!("\n{e:#}");
                }
                crate::usage::log_since("probe", outcome.is_ok(), started);
                true
            }
        };

        if acted && !after_action() {
            return Ok(());
        }
    }
}
