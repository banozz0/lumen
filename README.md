# lumen

Set the RGB lighting on gaming keyboards and mice from macOS, without vendor
software. Razer Synapse and HyperX NGENUITY do not exist for macOS at all, so
these peripherals otherwise only ever run their factory rainbow.

```bash
lumen set keyboard --color '#ff0080'
```

Devices are **data**, protocols are **code**: a device is a block of TOML in
`devices/devices.toml`, and nothing anywhere in lumen matches on a device id. One
device is verified end to end on real hardware; the registry is the point.

## Devices

| Device | USB id | Status |
|---|---|---|
| Razer Cynosa Lite | `1532:023F` | **supported** — latches in hardware, one packet and the colour stays |
| HyperX Pulsefire Core | `03F0:0D8F` | *experimental* — see below, it never really holds a colour |

Both were driven on real hardware; nothing is listed here that has not been seen
working. Only the keyboard is a promise. The mouse's firmware reclaims the LED
between packets, so a colour lasts only while `--hold` keeps re-sending it every
30ms — it is in the tree because a second vendor keeps the registry honest, not
because it works well.

Your device is almost certainly not in that table. [Adding your
own](#adding-your-own-device) is the interesting part, and a PR that adds it is
the whole point — see [Contributing](#contributing).

## Install

```bash
git clone https://github.com/banozz0/lumen.git
cd lumen
cargo install --path crates/lumen-cli
```

That puts `lumen` on your PATH. The device registry is compiled in, so the binary
needs nothing else at runtime — but it is compiled in, which means **adding a
device means reinstalling.**

Rust 1.88 or newer (edition 2024). macOS only.

## Permissions

Talking to a HID device needs the **Input Monitoring** grant, and macOS charges
it to whatever launched the process — so it is your *terminal* that needs the
grant, not the `lumen` binary. Grant it once and it survives every rebuild.

The asymmetry that makes this confusing: *listing* devices works without the
grant, *opening* one does not. A missing grant therefore prints a healthy-looking
device table where every write fails. lumen checks the grant itself and says so
rather than letting you read three `(iokit/common) not permitted` lines and
conclude the code is broken:

```
$ lumen set keyboard --color purple
Error: cannot open Razer Cynosa Lite: this terminal is not allowed to talk to HID devices.
macOS charges the Input Monitoring grant to whatever launched the shell, not to the `lumen` binary, so it is your terminal that needs it.
System Settings -> Privacy & Security -> Input Monitoring: your terminal is already in that list with its switch off. Turn it on -- macOS will not ask again by itself -- then run this again.
What each interface said:
  interface 2: cannot open (hidapi error: hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002E2) (iokit/common) not permitted)
  interface 1: cannot open (hidapi error: hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002E2) (iokit/common) not permitted)
  interface 0: cannot open (hidapi error: hid_open_path: failed to open IOHIDDevice from mach entry: (0xE00002E2) (iokit/common) not permitted)
```

The cause leads and the per-interface detail stays underneath it. Those three
lines used to be the whole message, which is how an afternoon once went into
debugging working code.

Two more traps worth knowing:

- **A refusal is permanent.** Once you have clicked Don't Allow, macOS never
  prompts again; only the System Settings toggle (or `tccutil reset ListenEvent
  <bundle-id>`) undoes it. That is why lumen tells you where the switch is
  instead of waiting for a prompt that will not come.
- **`launchd` sees nothing.** A process started by a LaunchAgent got an empty
  HID device list here whatever was granted — observed while building this, not
  re-tested since. If you want background lighting, expect to need a
  LaunchServices-launched `.app` rather than a LaunchAgent.

If the device list is empty rather than unopenable, that is not the grant: either
you are under `launchd`, or another process is holding the devices — close
OpenRGB before using lumen.

## Use

Run it with no arguments and it opens a menu — numbered lists all the way down,
`0` always steps back one level, and it returns to the menu after every action
instead of dropping you at the shell:

```
$ lumen

lumen -- RGB lighting

  1  Set a colour
  2  Set brightness
  3  Turn the lighting off
  4  Turn the lighting on
  5  Show what is connected
  6  Inspect the hardware
  0  Exit

>
```

Anything lumen can enumerate is a list rather than something to type: devices
come from what is actually plugged in, colours from the names it accepts. You
type only where a list cannot carry the answer — a hex value, a percentage.

The menu is a front-end and nothing else: it builds exactly the same calls the
flags do, so scripts and agents keep the full flag surface.

```bash
lumen list                                      # what is supported and plugged in
lumen set keyboard --color blue                 # by kind
lumen set razer-cynosa-lite --color '#ff0080'   # by id
lumen set all --color off                       # everything
lumen set mouse --color purple --hold           # keep re-sending; Ctrl-C to stop
lumen set all --color red --dry-run             # print packet bytes, send nothing
lumen set keyboard --brightness 40              # 0-100 percent
lumen set all --off                             # lights out
lumen set all --on                              # back on
lumen probe                                     # what every attached device declares
```

Colours accept names (`red`, `cyan`, `off`, …), `#rrggbb`, or the short `#f08`.
`--dry-run` and `--hold` work with `--brightness`, `--off` and `--on` exactly as
they work with `--color`.

### Brightness and on/off

Devices disagree about what their firmware can do, so the registry declares which
of two routes each one takes:

| | Razer Cynosa Lite | HyperX Pulsefire Core |
|---|---|---|
| `--brightness` | `brightness = "native"`, one protocol command | `brightness = "scaled"`, dims `--color` |
| `--off` / `--on` | `power = "native"`, the protocol's "no effect" command | `power = "color-black"`, black and white |

Two consequences worth knowing:

- On the mouse, `--brightness 40` dims whatever `--color` says (red by default),
  because scaling a colour is the only route there. On the keyboard the colour is
  left alone: brightness is a command of its own.
- `--on` cannot restore the colour a device had, because no device reports it. It
  sets a known state instead: static white at full brightness.

## Adding your own device

Two cases, and the vendor is what usually decides which one you are in — though
not always, as the caveat below spells out.

**Your device's vendor already has a driver here → one TOML block, no Rust.**
That is the whole point of the registry.

**Your vendor is new → somebody writes a driver module.** No config file can
express a protocol nobody has implemented yet: USB HID says how bytes reach a
device and nothing at all about what they should contain, so every vendor invents
its own report format, checksum and command set. That is HID being a zoo, not a
gap in lumen. The good news is that the driver is the only new code — the CLI,
the registry, brightness, power and `--hold` all come free.

One caveat on the easy case, because "same vendor" is not quite the boundary: a
driver declares the exact `control_report_len` it builds for, and lumen refuses a
registry entry that disagrees rather than sending a packet of the wrong size. A
device from a vendor already here but with a different report length — or a
different LED id inside the same protocol — needs a change in the driver too. It
is a smaller change than a new protocol, but it is not zero.

### 1. Measure the hardware

```
$ lumen probe
03f0:0d8f  HP, Inc HyperX Pulsefire Core  -- in the registry as `hyperx-pulsefire-core`, driver `hyperx-pulsefire`
  interface 0: no feature reports
  interface 1: feature report 0x07, 263 data bytes
  interface 2: no feature reports
1532:023f  Razer Razer Cynosa Lite  -- in the registry as `razer-cynosa-lite`, driver `razer-extended-matrix`
  interface 0: no feature reports
  interface 1: no feature reports
  interface 2: feature report 0x00, 90 data bytes
```

`probe` reports every attached HID device, in the registry or not, and one line
per interface. (Devices lumen does not drive are listed too; they are trimmed
here.) You are looking for a big vendor-specific **feature report**: 90 data
bytes on this keyboard, 263 on this mouse. That byte count is the
`control_report_len` your entry needs, and reading it off the hardware beats
guessing it.

Note that the two devices keep their control report on *different* interfaces —
1 and 2. That is the norm, and it is why lumen finds the control interface by
reading report descriptors rather than by hardcoding a number.

An interface reported as `unreadable` is almost always the Input Monitoring
grant. Without it, all three of this keyboard's interfaces refuse to open, while
the mouse still hands over one of its three — so partial output is not a clue
about the hardware, it is a clue about the grant.

### 2. Write the block

Copy the HyperX entry in `devices/devices.toml` and change the facts:

```toml
[[device]]
id = "hyperx-pulsefire-core"      # slug you type on the CLI
name = "HyperX Pulsefire Core"
vendor = "HyperX"
kind = "mouse"                    # also a CLI target: `lumen set mouse ...`
vendor_id = "0x03F0"              # from `lumen probe`, hex with the 0x
product_id = "0x0D8F"
driver = "hyperx-pulsefire"       # a driver that already exists, or your new one
control_report_len = 263          # the data-byte count `lumen probe` printed
holds_colour = false              # true if the colour survives without --hold
brightness = "scaled"             # "native" if the protocol has a brightness command
power = "color-black"             # "native" if it has an off command
```

`brightness` and `power` are how a device with a poorer protocol is described
rather than special-cased: `scaled` means lumen dims the colour itself, and
`color-black` means off is the colour black. Declare what the hardware can do and
the CLI behaves correctly without knowing which device it is talking to.

Then `cargo install --path crates/lumen-cli` again — the registry is compiled in
— and `lumen list` should show it.

### 3. Only if the vendor is new: the driver

Add a module to `crates/lumen-devices` implementing the `Driver` trait, name it
in `driver_for()`, and put that name in your `[[device]]` block. A driver builds
packets and never sends them, so a packet is a pure function of (device spec,
command) and the whole protocol is covered by byte-exact tests that need no
hardware:

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

`crates/lumen-devices/src/razer.rs` is the one to copy: its module docs write out
the wire format the tests then pin, byte by byte.

## Contributing

**This is an open project, and more devices are exactly what it is for.** The
registry exists so lumen can grow past the two peripherals one person happens to
own. Every other Razer, every other HyperX, and every vendor nobody has touched
yet is a gap that somebody holding the hardware can close in an evening — pull
requests are genuinely wanted, not tolerated.

In rough order of how easy they are to say yes to:

- **A device that speaks a driver already here** — one `[[device]]` block, no
  Rust. The easiest possible PR.
- **A new vendor** — a driver module in `crates/lumen-devices` with byte-exact
  tests, plus its `[[device]]` block. Say where the protocol facts came from.
- **A fix to a device already listed.** The open one is the Pulsefire Core's
  latch command: nobody has found it, and finding it would turn the mouse from
  experimental into supported.
- **Anything under [Not yet](#what-it-deliberately-does-not-do)** — effects,
  per-key lighting, Linux. None of it is refused on principle; it is just not
  written.

Two things every device PR needs, because they are what keeps the table worth
reading: **the `lumen probe` output your numbers came from**, and **a plain
sentence about what you saw the hardware actually do**. This README claims every
listed device was seen working, and that has to keep being true — including
whether the colour survives on its own, which is the one thing that decides
whether a device is a promise or an experiment.

Protocol facts here are written from published byte-level documentation. No code
is copied from OpenRGB or OpenRazer, and PRs need to keep it that way: those are
GPL, lumen is MIT, and mixing them would quietly relicense the project.

## How it is put together

```
crates/lumen-core      types, the Driver trait, registry loading -- no I/O
crates/lumen-hid       the only crate that touches hardware
crates/lumen-devices   one module per protocol family
crates/lumen-cli       the `lumen` binary
devices/devices.toml   the device registry
```

One module per protocol *family*, not per device: several devices can share a
driver, which is why adding one usually means writing no Rust.

`lumen-core`'s public API is kept plain — owned data, no generic parameters,
nothing a caller has to instantiate — so a SwiftUI front-end can sit on it
through a thin FFI wrapper instead of a rewrite.

Every protocol here is written from published byte-level facts rather than
adapted from another project's source — see [Contributing](#contributing) for
why that matters and has to stay true.

## Usage log

lumen appends one JSON line per command to `usage.jsonl` in the repo it was built
from. It is local and git-ignored — nothing is sent anywhere, and it exists so a
review loop can see which commands actually get used. Point it elsewhere with
`LUMEN_USAGE_LOG=/some/path`, or turn it off with an empty `LUMEN_USAGE_LOG=`.

## What it deliberately does not do

Effects, per-key lighting, profiles, persistence across sleep, a GUI, Windows,
Linux. No brightness or power command is known for the Pulsefire Core, so both
are emulated there rather than driven, and finding its latch command is still
open — it is the same unknown that makes its colour vanish on screen lock.

## Status

A working tool that solves one problem for one person on one Mac. Two devices are
in the registry; one of them is a promise. The interesting claim is that the
third device can cost a TOML block and nothing else — true when it speaks a
protocol already here at the same report length, and honestly documented where
it is not.

MIT licensed. A solo project, and agents wrote the code, so no support is
promised — an issue asking someone to debug your setup may sit for a while.
Contributions are the opposite: a PR adding a device is the best thing that can
happen to this repo. Fork it freely.
