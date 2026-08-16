---
name: lumen
description: "Use when the user wants their keyboard or mouse lighting changed — 'make my keyboard purple', 'turn the keyboard lights off', 'dim the RGB', 'what colour is my keyboard' — or when RGB lighting on a macOS peripheral is not responding."
version: 1.0.0
author: banozz0
license: MIT
platforms: [macos]
metadata:
  hermes:
    tags: [rgb, keyboard, mouse, lighting, hid, macos, cli]
---

# lumen

`lumen` sets the RGB lighting on supported gaming keyboards and mice from macOS,
without vendor software. It is one binary on PATH; every use is a single
short-lived command:

```
lumen set keyboard --color purple
```

Devices are declared in a registry compiled into the binary, so `lumen list` is
the truth about what this machine can drive — never assume a device is supported
because the user owns it.

## This machine

Which devices are attached, where the binary is installed and whether the
required macOS grant is in place all differ per machine, so they are not in this
file. If a `LOCAL.md` sits beside it, that file is this machine's setup and it
wins over anything general said here — read it before the first run. With no
`LOCAL.md`, `lumen list` and `lumen --help` are the fallback.

## When to Use

Use it when the user wants the lighting on a keyboard or mouse set, dimmed,
turned off, or turned back on, or when they ask why their lighting is not
responding.

The boundary — things it looks like it does but does not:

- **It cannot read the current colour.** No device reports one back. If the user
  asks what colour something is, say the hardware cannot be asked; do not infer
  it from what was set earlier in the conversation.
- **It is not a lighting daemon.** There are no effects, no per-key colours, no
  profiles, and nothing survives a device power cycle beyond what the hardware
  itself latches.
- **It only drives devices in its registry.** For anything else, `lumen probe`
  reports what the hardware declares, but adding it is a code-and-config change,
  not a command.

## Hard rules

**1. Never invent the result.** You cannot see the lighting; the exit status and
the printed line are your only evidence. If the command failed, that *is* the
answer — report it. A confident "done, it's purple" over a failed command leaves
the user staring at an unchanged keyboard, and it is the reason this tool was
once written off as broken when it worked perfectly.

**2. A permission error is not a code problem.** If the output names Input
Monitoring, the fix is a switch in System Settings that only the user can flip —
macOS charges the grant to the terminal app, not to `lumen`. Tell them where the
switch is. Do not debug, rebuild, reinstall, or edit anything.

**3. `--hold` never returns.** It re-sends the colour until Ctrl-C. Run it only
in a background or detached call, never as a blocking foreground command, or the
session hangs until something kills it.

## Commands

| The ask | Run |
|---|---|
| "what can you control?" / "is it plugged in?" | `lumen list` |
| "make my keyboard purple" | `lumen set keyboard --color purple` |
| "set it to #ff0080" | `lumen set keyboard --color '#ff0080'` |
| "change the mouse too" / "all of it" | `lumen set all --color blue` |
| "dim it" / "40 percent" | `lumen set keyboard --brightness 40` |
| "lights off" | `lumen set all --off` |
| "lights back on" | `lumen set all --on` |
| "why isn't my new mouse working?" | `lumen probe` |

- Targets are `all`, a kind (`keyboard`, `mouse`), or a device id from
  `lumen list`. A target that matches nothing is an error, not a silent no-op.
- Colours take names (`red`, `cyan`, `off`, …), `#rrggbb`, or short `#f08`.
- `--dry-run` prints the exact packet bytes and sends nothing. Use it when the
  user wants to know what a command would do.
- Every flag here must exist in the installed build: `lumen --help` and
  `lumen set --help` are the truth, not this table and not the README.

## Never run these

- **`lumen set … --hold` in the foreground.** It is an infinite loop by design.
  A device that does not latch its colour needs it, and `lumen list` marks which
  ones those are — but it belongs in a detached call the user can stop.

## Delivering the answer

- **Asked in conversation** → answer there, quoting what the command actually
  printed. One line is enough: what changed, on which device.
- **A device that does not latch** → say so in the same breath. "Set, but this
  mouse drops the colour the moment lumen stops sending it" is the honest
  version; "done" is not.

## Honest status

- A small solo tool. Two devices are in the registry and only one of them is a
  promise: the other needs `--hold` and never truly holds a colour.
- macOS only, and it needs the Input Monitoring grant to open any device.
  *Listing* devices works without the grant, so a healthy-looking `lumen list` is
  not proof that setting a colour will work.
- A clean run proves a packet was accepted by the device. It does not prove the
  user saw the colour change — only they can confirm that.
- No support is promised for this tool. If it is broken, say so plainly rather
  than working around it.

## The repo is the truth

This file lives in the tool's own repo at `skill/SKILL.md` and that copy is the
source of truth; every installed copy is a derivative. The tool moves faster than
the file: its own help output is always current. When the tool gains a command,
this file changes in the same commit.
