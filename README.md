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
own](#adding-your-own-device) is the interesting part.

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
macOS charges the Input Monitoring grant to whatever launched the shell, not to the
`lumen` binary, so it is your terminal that needs it.
System Settings -> Privacy & Security -> Input Monitoring: your terminal is already in
that list with its switch off. Turn it on -- macOS will not ask again by itself -- then
run this again.
```

Two more traps worth knowing:

- **A refusal is permanent.** Once you have clicked Don't Allow, macOS never
  prompts again; only the System Settings toggle (or `tccutil reset ListenEvent
  <bundle-id>`) undoes it. That is why lumen tells you where the switch is
  instead of waiting for a prompt that will not come.
- **`launchd` sees nothing.** A process started by a LaunchAgent gets an empty
  HID device list no matter what is granted. Background lighting has to be a
  LaunchServices-launched `.app`, not a LaunchAgent.

If the device list is empty rather than unopenable, that is not the grant: either
you are under `launchd`, or another process is holding the devices — close
OpenRGB before using lumen.

## Use

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

There are exactly two cases, and which one you are in depends on the vendor, not
on the device.

**Your device's vendor already has a driver here → one TOML block, no Rust.**
That is the whole point of the registry.

**Your vendor is new → somebody writes a driver module.** No config file can
express a protocol nobody has implemented yet: USB HID says how bytes reach a
device and nothing at all about what they should contain, so every vendor invents
its own report format, checksum and command set. That is HID being a zoo, not a
gap in lumen. The good news is that the driver is the only new code — the CLI,
the registry, brightness, power and `--hold` all come free.

### 1. Measure the hardware

```
$ lumen probe
03f0:0d8f  HP, Inc HyperX Pulsefire Core  -- in the registry as `hyperx-pulsefire-core`, driver `hyperx-pulsefire`
  interface 0: no feature reports
  interface 1: no feature reports
  interface 2: feature report 0x07, 263 data bytes
1532:023f  Razer Razer Cynosa Lite  -- in the registry as `razer-cynosa-lite`, driver `razer-extended-matrix`
  interface 0: no feature reports
  interface 1: no feature reports
  interface 2: feature report 0x00, 90 data bytes
```

`probe` reports every attached HID device, in the registry or not. You are
looking for one big vendor-specific **feature report** — 90 data bytes on the
Razer, 263 on the HyperX. That byte count is the `control_report_len` your entry
needs, and reading it off the hardware beats guessing it.

Interfaces that expose a keyboard never open, whatever is granted. That is macOS
protecting keystrokes, and it is why lumen finds the control interface by reading
report descriptors rather than by hardcoding an interface number.

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

`lumen-core`'s public API is kept plain — owned data, no generics or lifetimes
escaping — so a SwiftUI front-end can sit on it through a thin FFI wrapper
instead of a rewrite.

Every protocol here is written from published byte-level facts. No code is copied
from OpenRGB or OpenRazer, which is what lets lumen be MIT rather than inherit
their licences.

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
third device costs a TOML block, and that claim is honest right up to the point
where the vendor is new.

MIT licensed. Solo project — agents wrote the code — and no support is promised:
issues and PRs may sit unanswered. Fork it freely.
