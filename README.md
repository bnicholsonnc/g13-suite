# g13-suite

Full-featured Linux support for the Logitech G13 gaming keypad.

The G13's official software is Windows-only, and the long-standing community userspace drivers fight the hardware over raw USB — which makes keys stick, breaks hold-to-act, and tends to fall apart in games. g13-suite takes a different approach: it builds entirely on top of the in-kernel lg-g15 driver, reading the keypad and thumbstick as ordinary input devices and re-emitting clean events through uinput. No libusb, no stuck keys.

## Features

- **True-hold remapping** — every G-key stays held for exactly as long as you hold it, so hold-to-act bindings work correctly (e.g. holding Space to ascend on a flying mount in WoW). Single keys or chords like `LEFTCTRL+1`.
- **Live profiles** — switch profiles on the fly with the M-keys, each with its own bindings and backlight color. The M-key indicator lights show which profile is active.
- **Two thumbstick modes** — WASD/backpedal (combat-style, stay facing forward) or analog gamepad (move-and-auto-face), with adjustable deadzone and axis inversion.
- **RGB backlight** — per-profile colors via the kernel multicolor LED interface, applied on startup and on every profile switch.
- **A native configurator** — click keys directly on a picture of the G13 to bind them, manage profiles, tune the thumbstick, and pick colors. No config-file editing required, though the TOML config is clean and hand-editable if you prefer. A localhost web UI is included as an alternative.

## How it works

A small Rust workspace: a hotplug-aware daemon (`g13d`) that grabs the keypad and thumbstick and re-emits through uinput, a shared config library, and the GUI. Devices are found by name, so it survives the event-number shuffle across reboots.

Tested on Pop!_OS (COSMIC/Wayland); should work on any modern Linux with the lg-g15 driver.
