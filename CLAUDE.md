# g13-suite — project context

Linux userspace support for the Logitech G13 gaming keypad, built **on top of the
kernel `lg-g15` driver** (kernel 6.19+; the G13 appears as input devices, no libusb).
Target user runs **Pop!_OS (COSMIC/Wayland)**, **rustc 1.96**. Goal: idiot-friendly,
shareable, full functionality (remap + profiles + thumbstick + RGB).

## Crates (workspace)
- `g13-config/` — shared config model (serde), TOML load/save, `backlight.rs` (RGB +
  M-key LED sysfs control). Daemon and UIs all use this.
- `g13d/` — the daemon: grabs the keypad/thumbstick evdev devices, re-emits via uinput.
  True-hold passthrough, profile layers, thumbstick modes, applies backlight.
- `g13-gui/` — **native egui configurator (primary UI)**. EXCLUDED from the workspace
  (`exclude = ["g13-gui"]`) so the rest builds on older toolchains. Build separately.
- `g13-config-ui/` — localhost web configurator (fallback, fully working).

## Build / run
```sh
cargo build --release                          # lib + daemon + web UI
cd g13-gui && cargo build --release && cd ..    # native GUI
cargo test -p g13-config                        # config round-trip test
```
Build deps on the target machine: `libudev-dev`, `pkg-config`.

## IMMEDIATE NEXT STEPS (where we left off)
1. **Build `g13-gui` and fix any compile errors.** It was written carefully but NOT
   compiled (previous environment had only rustc 1.75, which can't build egui).
   `eframe`/`egui` are pinned to **`=0.29.1`** — match that version's API exactly
   (e.g. app creator returns `Result`, `ComboBox::from_id_source` not `from_id_salt`,
   `rect_stroke(rect, rounding, stroke)` 3-arg form).
2. Run it, verify the key hotspots line up on the image and the **RGB live preview**
   changes the backlight. Hotspot coords are `KEYS` at the top of `g13-gui/src/main.rs`
   (percent of image); nudge if any are off.
3. **Color-on-save bug to confirm fixed:** changing colour + Save showed no change. Most
   likely cause was the *old* standalone `g13d` (no backlight code) still installed. Make
   sure `/usr/local/bin/g13d` is rebuilt from this suite. The daemon logs to journald:
   `profile '<x>' backlight -> r,g,b` / `... no backlight LED found` / `... has no colour`.

## Hardware facts (verified on the target machine)
- Input device names: `Logitech G13 Gaming Keypad`, `Logitech G13 Thumbstick`.
  Event numbers renumber per boot — the daemon finds devices by **name**, not eventN.
- Keypad evdev key codes: **G1..G22 = KEY_MACRO1..KEY_MACRO22 = codes 656..677**
  (`G_n = 655 + n`). M-keys: MR=0x2b0, M1=0x2b3, M2=0x2b4, M3=0x2b5. L1..L4=0x2b8..0x2bb.
  Keypad reports clean press(1)/release(0), NO autorepeat.
- Thumbstick: ABS_X/ABS_Y 0..255 (centre ~128); buttons BTN_THUMB/THUMB2/BASE/BASE2.
- Backlight LED: **`/sys/class/leds/g13:rgb:kbd_backlight`** — multicolor,
  `multi_index = "red green blue"`, set via `multi_intensity` + `brightness`/`max_brightness`.
- M-key indicator LEDs: `g13:red:macro_preset_{1,2,3}`, `g13:red:macro_record`.

## Config format (`/etc/g13d/config.toml`)
```toml
[profiles.default]
color = "255,128,0"            # optional RGB backlight for this profile
[profiles.default.keys]
G15 = "SPACE"                 # held while pressed; chords like "LEFTCTRL+1"
[thumbstick]
mode = "keys"                 # keys = WASD/backpedal | gamepad = analog | off
deadzone = 50
up="W" down="S" left="A" right="D"
[profile_keys]
M1 = "default"                # M-keys switch profiles
```
Gotcha: serializing with **toml 0.5** requires value fields before table fields —
`Profile.color` is declared BEFORE `Profile.keys` for this reason. Keep that order.

## Conventions
- `BTreeMap` everywhere for deterministic TOML output.
- Bindings are plain strings ("KEY name" or "A+B" chords); all true-hold (no per-key repeat).
- Daemon runs as a root systemd service (`dist/g13d.service`); GUI runs as the user and
  uses `pkexec` to save + reload. Live RGB needs the LED group-writable
  (`dist/99-g13-leds.rules`).

## WoW notes (the main use case)
- Flying ASCEND needs the jump key HELD → true-hold passthrough is mandatory.
- Thumbstick "keys" mode = backpedal/face-forward (combat); "gamepad" mode = move-and-
  auto-face. User wants both selectable.
- WoW launches via Steam Proton; SteamInput may grab gamepads.
