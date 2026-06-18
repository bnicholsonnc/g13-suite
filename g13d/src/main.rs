//! g13d — Logitech G13 userspace daemon (keypad remap + thumbstick + RGB),
//! built on the kernel lg-g15 driver. Hotplug-aware; finds devices by name.

mod keypad;
mod keys;
mod lcd;
mod thumbstick;

use anyhow::{Context, Result};
use g13_config::Config;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

const KEYPAD_NAME: &str = "Logitech G13 Gaming Keypad";
const STICK_NAME: &str = "Logitech G13 Thumbstick";

fn main() -> Result<()> {
    let cfg_path = std::env::args().nth(1).unwrap_or_else(|| g13_config::DEFAULT_PATH.into());
    let led_override = std::env::var("G13_LED").ok();
    let cfg_path = PathBuf::from(cfg_path);

    eprintln!("g13d: config {}", cfg_path.display());
    let cfg = Config::load(&cfg_path)?;

    thread::spawn(lcd::run);

    loop {
        match run_once(&cfg, led_override.clone()) {
            Ok(()) => eprintln!("g13d: device gone; waiting…"),
            Err(e) => eprintln!("g13d: {e:#}; retrying…"),
        }
        thread::sleep(Duration::from_secs(2));
    }
}

fn find_by_name(target: &str) -> Option<String> {
    for entry in std::fs::read_dir("/dev/input").ok()? {
        let path = entry.ok()?.path();
        let f = path.file_name()?.to_str()?;
        if !f.starts_with("event") { continue; }
        if let Ok(d) = evdev::Device::open(&path) {
            if d.name() == Some(target) {
                return Some(path.to_string_lossy().into_owned());
            }
        }
    }
    None
}

fn wait_for_devices() -> (String, String) {
    loop {
        if let (Some(k), Some(s)) = (find_by_name(KEYPAD_NAME), find_by_name(STICK_NAME)) {
            eprintln!("g13d: keypad={k} stick={s}");
            return (k, s);
        }
        thread::sleep(Duration::from_millis(500));
    }
}

fn run_once(cfg: &Config, led_override: Option<String>) -> Result<()> {
    let (keypad_path, stick_path) = wait_for_devices();
    let mut kp = keypad::Keypad::new(&keypad_path, cfg, led_override).context("keypad init")?;

    let stick_handle = if cfg.thumbstick.mode != "off" {
        let sp = stick_path.clone();
        let scfg = cfg.thumbstick.clone();
        Some(thread::spawn(move || -> Result<()> {
            let mut st = thumbstick::Stick::new(&sp, &scfg)?;
            st.run()
        }))
    } else { None };

    let r = kp.run();
    if let Some(h) = stick_handle { let _ = h.join(); }
    r
}
