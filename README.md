# lumen

Control RGB lighting on gaming keyboards and mice from macOS, without vendor
software. Razer Synapse and HyperX NGENUITY do not exist for macOS at all, so
these peripherals otherwise only ever run their factory rainbow.

Currently a CLI. A SwiftUI app will sit on the same core later.

## Supported devices

| Device | USB id | Colour holds by itself? |
|---|---|---|
| Razer Cynosa Lite | `1532:023F` | yes |
| HyperX Pulsefire Core | `03F0:0D8F` | no, needs `--hold` |

Both are verified on real hardware. Nothing is listed here that has not been
seen working.

## Use

```bash
cargo build --release

lumen list                                  # what is supported and plugged in
lumen set keyboard --color blue             # by kind
lumen set razer-cynosa-lite --color '#ff0080'   # by id
lumen set all --color off                   # everything
lumen set mouse --color purple --hold       # keep re-sending; Ctrl-C to stop
lumen set all --color red --dry-run         # print packet bytes, send nothing
lumen set keyboard --brightness 40          # 0-100 percent
lumen set all --off                         # lights out
lumen set all --on                          # back on
lumen probe                                 # report ids and interfaces, for adding devices
```

Colours accept names (`red`, `cyan`, `off`, …), `#rrggbb`, or the short `#f08`.

`--dry-run` and `--hold` work with `--brightness`, `--off` and `--on` the same
way they work with `--color`.

### Brightness and on/off

Devices disagree about what their firmware can do, so the registry says which of
the two routes each one takes and nothing in the code matches on a device id:

| | Razer Cynosa Lite | HyperX Pulsefire Core |
|---|---|---|
| `--brightness` | `brightness = "native"`, one protocol command | `brightness = "scaled"`, dims `--color` |
| `--off` / `--on` | `power = "native"`, the protocol's "no effect" command | `power = "color-black"`, black and white |

Two consequences worth knowing:

- On the mouse, `--brightness 40` dims whatever `--color` says (red by default),
  because scaling a colour is the only way there. On the keyboard the colour is
  left alone: brightness is a command of its own.
- `--on` cannot restore the colour a device had, because no device reports it.
  It sets a known state instead: static white at full brightness.

### Why the mouse needs `--hold`

The Pulsefire Core accepts a direct colour but never latches it: the firmware
reclaims the LED between packets, so the colour only lasts while lumen keeps
re-sending it every 30ms. The keyboard latches in hardware and needs one packet.

Finding the Pulsefire's latch command is still open. It is the same problem that
makes the colour vanish on screen lock.

## Permissions

Reading and writing HID devices needs the **Input Monitoring** grant, and macOS
charges it to whatever launched the process — so it is your *terminal* that needs
the grant, not the `lumen` binary. That is convenient: grant it once and it
survives every rebuild.

If `lumen list` reports that no HID devices are visible at all, that is the
missing grant. System Settings → Privacy & Security → Input Monitoring.

Two traps worth knowing:

- Processes started by `launchd` get an empty HID device list no matter what is
  granted. Anything that needs to run in the background must be a
  LaunchServices-launched `.app`, not a LaunchAgent.
- Two processes fighting over one device leave the loser with an empty list.
  Close OpenRGB before using lumen.

## How it is put together

```
crates/lumen-core      types, the Driver trait, registry loading -- no I/O
crates/lumen-hid       the only crate that touches hardware
crates/lumen-devices   one module per protocol family
crates/lumen-cli       the `lumen` binary
devices/devices.toml   the device registry
```

Devices are data, protocols are code. A device that speaks a protocol lumen
already implements needs only a new `[[device]]` block in `devices/devices.toml`
and no Rust at all.

Drivers build packets; they never send them. Because a packet is a pure function
of (device spec, command), every protocol is covered by byte-exact tests that run
with no hardware attached:

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

`lumen-core`'s public API is kept plain — owned data, no generics or lifetimes
escaping — so the planned SwiftUI front-end can sit on it through a thin FFI
wrapper instead of a rewrite.

## Adding a device

1. Plug it in and run `lumen probe` to find its control report id and length.
2. Add a `[[device]]` block to `devices/devices.toml`.
3. If it speaks an existing protocol, you are done. If not, add a module to
   `lumen-devices` implementing `Driver`, with golden byte tests.

Protocol implementations here are written from published byte-level facts. No
code is copied from OpenRGB or OpenRazer, which keeps lumen's own licensing an
open question rather than a decided one.

## Not yet

Effects, per-key lighting, profiles, persistence across sleep, Windows and Linux
support. No brightness or power command is known for the Pulsefire Core, so both
are emulated there rather than driven.
