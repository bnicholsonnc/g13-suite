//! Shared config model for the g13 suite (daemon + web UI).
//! Serializable both ways so the UI can round-trip it to TOML on disk.

pub mod backlight;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const DEFAULT_PATH: &str = "/etc/g13d/config.toml";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Named profiles. "default" is active at start.
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,

    /// Single-profile shorthand: merged into profiles["default"] at runtime.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bindings: BTreeMap<String, String>,

    #[serde(default)]
    pub thumbstick: Thumbstick,

    /// Which source key switches to which profile, e.g. M1 = "default".
    #[serde(default)]
    pub profile_keys: BTreeMap<String, String>,
}

/// A profile is a set of key bindings plus an optional backlight colour.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// "r,g,b" backlight colour for this profile (0..255 each). Optional.
    /// (declared before `keys` so TOML serialization emits the value before the table)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// source key (G1..G22, M1.., L1..) -> output ("KEY_SPACE", "1", "LEFTCTRL+1")
    #[serde(default)]
    pub keys: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbstick {
    #[serde(default = "d_mode")]
    pub mode: String, // "keys" | "gamepad" | "off"
    #[serde(default = "d_dz")]
    pub deadzone: i32,
    #[serde(default)]
    pub invert_x: bool,
    #[serde(default)]
    pub invert_y: bool,
    #[serde(default = "d_up")]
    pub up: String,
    #[serde(default = "d_down")]
    pub down: String,
    #[serde(default = "d_left")]
    pub left: String,
    #[serde(default = "d_right")]
    pub right: String,
    /// BTN_THUMB binding (pressing the thumbstick down); empty = unbound.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub thumb: String,
    /// BTN_BASE binding (button left of thumbstick); empty = unbound.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub button1: String,
    /// BTN_BASE2 binding (button below thumbstick); empty = unbound.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub button2: String,
}

impl Default for Thumbstick {
    fn default() -> Self {
        Thumbstick {
            mode: d_mode(), deadzone: d_dz(), invert_x: false, invert_y: false,
            up: d_up(), down: d_down(), left: d_left(), right: d_right(),
            thumb: String::new(), button1: String::new(), button2: String::new(),
        }
    }
}

fn d_mode() -> String { "keys".into() }
fn d_dz() -> i32 { 50 }
fn d_up() -> String { "W".into() }
fn d_down() -> String { "S".into() }
fn d_left() -> String { "A".into() }
fn d_right() -> String { "D".into() }

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg: Config = toml::from_str(&text)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(cfg)
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load(path).unwrap_or_default()
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).context("serializing config")
    }

    /// profile name -> (source key -> output binding), with [bindings] shorthand
    /// merged into "default".
    pub fn resolved(&self) -> BTreeMap<String, BTreeMap<String, String>> {
        let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for (name, p) in &self.profiles {
            out.insert(name.clone(), p.keys.clone());
        }
        if !self.bindings.is_empty() {
            let d = out.entry("default".into()).or_default();
            for (k, v) in &self.bindings {
                d.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        if out.is_empty() {
            out.insert("default".into(), BTreeMap::new());
        }
        out
    }

    /// The backlight colour for a given profile name, if any.
    pub fn color_for(&self, profile: &str) -> Option<(u8, u8, u8)> {
        self.profiles
            .get(profile)
            .and_then(|p| p.color.as_deref())
            .and_then(parse_color)
    }
}

pub fn parse_color(s: &str) -> Option<(u8, u8, u8)> {
    let parts: Vec<_> = s.split(',').map(|x| x.trim().parse::<u8>().ok()).collect();
    if parts.len() == 3 {
        Some((parts[0]?, parts[1]?, parts[2]?))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        let mut def = Profile::default();
        def.color = Some("10,20,30".into());
        def.keys.insert("G15".into(), "SPACE".into());
        def.keys.insert("G1".into(), "LEFTCTRL+1".into());
        c.profiles.insert("default".into(), def);
        c.profile_keys.insert("M1".into(), "default".into());
        c.thumbstick.left = "Q".into();

        let toml = c.to_toml().unwrap();
        let back: Config = toml::from_str(&toml).unwrap();
        assert_eq!(back.color_for("default"), Some((10, 20, 30)));
        let r = back.resolved();
        assert_eq!(r["default"]["G15"], "SPACE");
        assert_eq!(r["default"]["G1"], "LEFTCTRL+1");
        assert_eq!(back.profile_keys["M1"], "default");
        assert_eq!(back.thumbstick.left, "Q");
    }
}
