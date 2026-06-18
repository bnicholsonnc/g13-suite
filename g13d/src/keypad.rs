//! Keypad remapper with true-hold passthrough + profile layers + backlight.

use anyhow::{Context, Result};
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, Device, EventType, InputEvent, InputEventKind, Key,
};
use std::collections::BTreeMap;

use g13_config::Config;
use crate::keys::{output_key, source_code};

#[derive(Clone)]
struct Action { keys: Vec<Key> }

pub struct Keypad {
    dev: Device,
    out: VirtualDevice,
    profiles: BTreeMap<String, BTreeMap<u16, Action>>,
    colors: BTreeMap<String, (u8, u8, u8)>,
    profile_switch: BTreeMap<u16, String>,
    mkey_profiles: Vec<(String, String)>,
    led_override: Option<String>,
    active: String,
}

impl Keypad {
    pub fn new(path: &str, cfg: &Config, led_override: Option<String>) -> Result<Self> {
        let mut dev = Device::open(path).with_context(|| format!("opening keypad {path}"))?;
        dev.grab().with_context(|| format!("grabbing keypad {path}"))?;

        let resolved = cfg.resolved();
        let mut all_keys: AttributeSet<Key> = AttributeSet::new();
        let mut profiles: BTreeMap<String, BTreeMap<u16, Action>> = BTreeMap::new();

        for (pname, binds) in &resolved {
            let mut compiled = BTreeMap::new();
            for (src, out_str) in binds {
                let code = source_code(src)
                    .with_context(|| format!("source '{src}' in profile '{pname}'"))?;
                let mut keys = Vec::new();
                for part in out_str.split('+') {
                    let k = output_key(part)
                        .with_context(|| format!("output '{part}' for '{src}'"))?;
                    all_keys.insert(k);
                    keys.push(k);
                }
                compiled.insert(code, Action { keys });
            }
            profiles.insert(pname.clone(), compiled);
        }

        let mut colors = BTreeMap::new();
        for name in profiles.keys() {
            if let Some(c) = cfg.color_for(name) {
                colors.insert(name.clone(), c);
            }
        }

        let mut profile_switch = BTreeMap::new();
        let mut mkey_profiles = Vec::new();
        for (src, pname) in &cfg.profile_keys {
            profile_switch.insert(source_code(src)?, pname.clone());
            if src.trim().to_uppercase().starts_with('M') {
                mkey_profiles.push((src.trim().to_uppercase(), pname.clone()));
            }
        }

        let out = VirtualDeviceBuilder::new()
            .context("creating virtual keyboard")?
            .name("G13 Virtual Keyboard")
            .with_keys(&all_keys)?
            .build()?;

        let active = if profiles.contains_key("default") {
            "default".to_string()
        } else {
            profiles.keys().next().cloned().unwrap_or_else(|| "default".into())
        };

        let kp = Keypad { dev, out, profiles, colors, profile_switch, mkey_profiles, led_override, active };
        kp.apply_indicators();
        Ok(kp)
    }

    fn apply_indicators(&self) {
        match self.colors.get(&self.active) {
            Some(&(r, g, b)) => match g13_config::backlight::apply(r, g, b, self.led_override.as_deref()) {
                Ok(true) => eprintln!("g13d: profile '{}' backlight -> {r},{g},{b}", self.active),
                Ok(false) => eprintln!("g13d: profile '{}' colour {r},{g},{b} set, but no backlight LED found", self.active),
                Err(e) => eprintln!("g13d: backlight error: {e:#}"),
            },
            None => eprintln!("g13d: profile '{}' has no colour set", self.active),
        }
        // light the M-key LED(s) that select the current profile; clear the rest
        for (mkey, prof) in &self.mkey_profiles {
            g13_config::backlight::set_mkey_led(mkey, prof == &self.active);
        }
    }

    pub fn run(&mut self) -> Result<()> {
        loop {
            let events: Vec<InputEvent> =
                self.dev.fetch_events().context("keypad disconnected")?.collect();
            let mut batch: Vec<InputEvent> = Vec::new();

            for ev in events {
                let (code, value) = match ev.kind() {
                    InputEventKind::Key(k) => (k.code(), ev.value()),
                    _ => continue,
                };

                if value == 1 {
                    if let Some(target) = self.profile_switch.get(&code).cloned() {
                        if self.profiles.contains_key(&target) {
                            self.active = target;
                            self.apply_indicators();
                        }
                        continue;
                    }
                }

                let action = match self.profiles.get(&self.active).and_then(|m| m.get(&code)) {
                    Some(a) => a.clone(),
                    None => continue,
                };
                if value == 2 { continue; } // drop source autorepeat -> pure hold

                match value {
                    1 => for k in &action.keys {
                        batch.push(InputEvent::new(EventType::KEY, k.code(), 1));
                    },
                    0 => for k in action.keys.iter().rev() {
                        batch.push(InputEvent::new(EventType::KEY, k.code(), 0));
                    },
                    _ => {}
                }
            }

            if !batch.is_empty() {
                self.out.emit(&batch).context("emit keyboard")?;
            }
        }
    }
}
