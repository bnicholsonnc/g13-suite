//! Thumbstick: "keys" (WASD/backpedal) or "gamepad" (analog Xbox pad) mode.

use anyhow::{Context, Result};
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AbsInfo, AbsoluteAxisType, AttributeSet, Device, EventType, InputEvent, InputEventKind, Key,
    UinputAbsSetup,
};
use g13_config::Thumbstick as StickCfg;
use crate::keys::output_key;

const RAW_CENTER: i32 = 128;
const RAW_HALF: i32 = 127;
const OUT_MIN: i32 = -32768;
const OUT_MAX: i32 = 32767;

pub struct Stick {
    dev: Device,
    out: VirtualDevice,
    cfg: StickCfg,
    keys_mode: bool,
    k_up: Key, k_down: Key, k_left: Key, k_right: Key,
    k_thumb: Option<Key>, k_btn1: Option<Key>, k_btn2: Option<Key>,
    up_on: bool, down_on: bool, left_on: bool, right_on: bool,
}

impl Stick {
    pub fn new(path: &str, cfg: &StickCfg) -> Result<Self> {
        let mut dev = Device::open(path).with_context(|| format!("opening thumbstick {path}"))?;
        dev.grab().with_context(|| format!("grabbing thumbstick {path}"))?;
        let keys_mode = cfg.mode != "gamepad";
        let (k_up, k_down, k_left, k_right) =
            (output_key(&cfg.up)?, output_key(&cfg.down)?, output_key(&cfg.left)?, output_key(&cfg.right)?);
        let k_thumb = if cfg.thumb.is_empty() { None } else { Some(output_key(&cfg.thumb)?) };
        let k_btn1 = if cfg.button1.is_empty() { None } else { Some(output_key(&cfg.button1)?) };
        let k_btn2 = if cfg.button2.is_empty() { None } else { Some(output_key(&cfg.button2)?) };

        let out = if keys_mode {
            let mut set: AttributeSet<Key> = AttributeSet::new();
            for k in [k_up, k_down, k_left, k_right] { set.insert(k); }
            if let Some(k) = k_thumb { set.insert(k); }
            if let Some(k) = k_btn1 { set.insert(k); }
            if let Some(k) = k_btn2 { set.insert(k); }
            VirtualDeviceBuilder::new().context("vkbd")?
                .name("G13 Virtual Stick-Keyboard").with_keys(&set)?.build()?
        } else {
            let abs = AbsInfo::new(0, OUT_MIN, OUT_MAX, 16, 128, 0);
            let x = UinputAbsSetup::new(AbsoluteAxisType::ABS_X, abs);
            let y = UinputAbsSetup::new(AbsoluteAxisType::ABS_Y, abs);
            let mut b: AttributeSet<Key> = AttributeSet::new();
            for k in [Key::BTN_SOUTH, Key::BTN_EAST, Key::BTN_NORTH, Key::BTN_WEST,
                      Key::BTN_TL, Key::BTN_TR, Key::BTN_SELECT, Key::BTN_START,
                      Key::BTN_THUMBL, Key::BTN_THUMBR] { b.insert(k); }
            VirtualDeviceBuilder::new().context("vpad")?
                .name("G13 Virtual Gamepad")
                .input_id(evdev::InputId::new(evdev::BusType::BUS_USB, 0x045e, 0x028e, 1))
                .with_keys(&b)?.with_absolute_axis(&x)?.with_absolute_axis(&y)?.build()?
        };

        Ok(Stick { dev, out, cfg: cfg.clone(), keys_mode,
            k_up, k_down, k_left, k_right, k_thumb, k_btn1, k_btn2,
            up_on: false, down_on: false, left_on: false, right_on: false })
    }

    fn scale(&self, raw: i32, invert: bool) -> i32 {
        let c = raw - RAW_CENTER;
        let a = if c.abs() <= self.cfg.deadzone { 0 } else { c };
        let s = (a * OUT_MAX / RAW_HALF).clamp(OUT_MIN, OUT_MAX);
        if invert { -s } else { s }
    }

    fn axis_keys(&self, raw: i32, invert: bool, low: Key, high: Key,
                 low_on: &mut bool, high_on: &mut bool, batch: &mut Vec<InputEvent>) {
        let c = raw - RAW_CENTER;
        let (neg, pos) = if invert { (high, low) } else { (low, high) };
        let wn = c < -self.cfg.deadzone;
        let wp = c > self.cfg.deadzone;
        let (wl, wh) = if invert { (wp, wn) } else { (wn, wp) };
        if wl != *low_on { *low_on = wl; batch.push(InputEvent::new(EventType::KEY, neg.code(), wl as i32)); }
        if wh != *high_on { *high_on = wh; batch.push(InputEvent::new(EventType::KEY, pos.code(), wh as i32)); }
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            let events: Vec<InputEvent> =
                self.dev.fetch_events().context("thumbstick disconnected")?.collect();
            let mut batch: Vec<InputEvent> = Vec::new();
            for ev in events {
                match ev.kind() {
                    InputEventKind::AbsAxis(axis) => {
                        if self.keys_mode {
                            if axis == AbsoluteAxisType::ABS_Y {
                                let (mut u, mut d) = (self.up_on, self.down_on);
                                self.axis_keys(ev.value(), self.cfg.invert_y, self.k_up, self.k_down, &mut u, &mut d, &mut batch);
                                self.up_on = u; self.down_on = d;
                            } else if axis == AbsoluteAxisType::ABS_X {
                                let (mut l, mut r) = (self.left_on, self.right_on);
                                self.axis_keys(ev.value(), self.cfg.invert_x, self.k_left, self.k_right, &mut l, &mut r, &mut batch);
                                self.left_on = l; self.right_on = r;
                            }
                        } else if axis == AbsoluteAxisType::ABS_X {
                            let v = self.scale(ev.value(), self.cfg.invert_x);
                            batch.push(InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, v));
                        } else if axis == AbsoluteAxisType::ABS_Y {
                            let v = self.scale(ev.value(), self.cfg.invert_y);
                            batch.push(InputEvent::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_Y.0, v));
                        }
                    }
                    InputEventKind::Key(k) if self.keys_mode => {
                        let mapped = match k {
                            Key::BTN_THUMB => self.k_thumb,
                            Key::BTN_BASE => self.k_btn1,
                            Key::BTN_BASE2 => self.k_btn2,
                            _ => None,
                        };
                        if let Some(mk) = mapped {
                            batch.push(InputEvent::new(EventType::KEY, mk.code(), ev.value()));
                        }
                    }
                    InputEventKind::Key(k) if !self.keys_mode => {
                        let m = match k {
                            Key::BTN_THUMB => Some(Key::BTN_SOUTH),
                            Key::BTN_THUMB2 => Some(Key::BTN_EAST),
                            Key::BTN_BASE => Some(Key::BTN_TL),
                            Key::BTN_BASE2 => Some(Key::BTN_TR),
                            _ => None,
                        };
                        if let Some(mb) = m { batch.push(InputEvent::new(EventType::KEY, mb.code(), ev.value())); }
                    }
                    _ => {}
                }
            }
            if !batch.is_empty() { self.out.emit(&batch).context("emit stick")?; }
        }
    }
}
