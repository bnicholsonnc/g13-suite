//! G13 RGB backlight + M-key indicator LEDs via the kernel `lg-g15` sysfs LEDs.
//!
//! Backlight node is typically `g13:rgb:kbd_backlight` (a multicolor LED exposing
//! `multi_intensity` + `multi_index` + `brightness`). The M-key lights are
//! single-channel `g13:red:macro_preset_{1,2,3}` and `g13:red:macro_record`.

use anyhow::{bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const LEDS: &str = "/sys/class/leds";

/// Find the G13 RGB backlight LED directory.
pub fn discover() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    for e in fs::read_dir(LEDS).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if (name.contains("g13") || name.contains("g15"))
            && (name.contains("rgb") || name.contains("backlight") || e.path().join("multi_intensity").exists())
            && !name.contains("macro")
        {
            candidates.push(e.path());
        }
    }
    candidates.sort_by_key(|p| {
        let n = p.file_name().unwrap().to_string_lossy().to_lowercase();
        if n.contains("rgb") || p.join("multi_intensity").exists() { 0 } else { 1 }
    });
    candidates.into_iter().next()
}

fn max_brightness(dir: &Path) -> u32 {
    fs::read_to_string(dir.join("max_brightness"))
        .ok().and_then(|s| s.trim().parse().ok()).unwrap_or(255)
}

/// Set an RGB backlight LED directory to (r,g,b).
pub fn set_color(led_dir: &Path, r: u8, g: u8, b: u8) -> Result<()> {
    let multi = led_dir.join("multi_intensity");
    if multi.exists() {
        // Respect whatever channel order multi_index advertises.
        let index = fs::read_to_string(led_dir.join("multi_index")).unwrap_or_else(|_| "red green blue".into());
        let vals: Vec<String> = index.split_whitespace().map(|ch| match ch {
            "red" => r, "green" => g, "blue" => b, _ => 0,
        }.to_string()).collect();
        let payload = if vals.is_empty() { format!("{r} {g} {b}") } else { vals.join(" ") };
        fs::write(&multi, &payload).with_context(|| format!("writing {}", multi.display()))?;
        let bright = led_dir.join("brightness");
        if bright.exists() {
            let _ = fs::write(&bright, max_brightness(led_dir).to_string());
        }
        return Ok(());
    }
    // per-channel fallback
    let mut wrote = false;
    for (name, v) in [("red", r), ("green", g), ("blue", b)] {
        let p = led_dir.join(name);
        if p.exists() {
            fs::write(&p, v.to_string()).with_context(|| format!("writing {}", p.display()))?;
            wrote = true;
        }
    }
    if !wrote { bail!("no writable colour interface under {}", led_dir.display()); }
    Ok(())
}

/// Discover (or use override) and set the backlight.
/// Returns Ok(true) if a LED was written, Ok(false) if none was found.
pub fn apply(r: u8, g: u8, b: u8, override_path: Option<&str>) -> Result<bool> {
    let dir = match override_path {
        Some(p) => PathBuf::from(p),
        None => match discover() { Some(d) => d, None => return Ok(false) },
    };
    set_color(&dir, r, g, b)?;
    Ok(true)
}

/// sysfs path for an M-key indicator LED (M1/M2/M3/MR), if it exists.
pub fn mkey_led(mkey: &str) -> Option<PathBuf> {
    let node = match mkey.trim().to_uppercase().as_str() {
        "M1" => "g13:red:macro_preset_1",
        "M2" => "g13:red:macro_preset_2",
        "M3" => "g13:red:macro_preset_3",
        "MR" => "g13:red:macro_record",
        _ => return None,
    };
    let p = Path::new(LEDS).join(node);
    p.exists().then_some(p)
}

/// Turn an M-key indicator LED on or off (best effort).
pub fn set_mkey_led(mkey: &str, on: bool) {
    if let Some(p) = mkey_led(mkey) {
        let v = if on { max_brightness(&p) } else { 0 };
        let _ = fs::write(p.join("brightness"), v.to_string());
    }
}
