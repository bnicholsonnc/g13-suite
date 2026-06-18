//! Source-token and output-key resolution.
use anyhow::{bail, Result};
use evdev::Key;

pub const G_KEY_BASE: u16 = 656; // KEY_MACRO1 = G1

pub fn source_code(token: &str) -> Result<u16> {
    let t = token.trim().to_uppercase();
    if let Some(num) = t.strip_prefix('G') {
        let n: u16 = num.parse().map_err(|_| anyhow::anyhow!("bad G-key: {token}"))?;
        if !(1..=22).contains(&n) {
            bail!("G-key out of range: {token}");
        }
        return Ok(G_KEY_BASE + (n - 1));
    }
    match t.as_str() {
        "MR" => Ok(0x2b0),
        "M1" => Ok(0x2b3),
        "M2" => Ok(0x2b4),
        "M3" => Ok(0x2b5),
        "L1" => Ok(0x2b8),
        "L2" => Ok(0x2b9),
        "L3" => Ok(0x2ba),
        "L4" => Ok(0x2bb),
        "LIGHT" => Ok(0x21e),
        _ => bail!("unknown source key token: {token}"),
    }
}

pub fn output_key(name: &str) -> Result<Key> {
    let n = name.trim().to_uppercase();
    let pref = if n.starts_with("KEY_") || n.starts_with("BTN_") { n.clone() } else { format!("KEY_{n}") };
    if let Ok(k) = pref.parse::<Key>() { return Ok(k); }
    if let Ok(k) = n.parse::<Key>() { return Ok(k); }
    bail!("unknown output key: {name}")
}
