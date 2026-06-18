//! G13 LCD system monitor.
//!
//! The 160×43 monochrome LCD is written via HID output report 0x03 on the
//! G13's hidraw device. Format: 1 byte report ID + 3 bytes padding + 960 bytes
//! column-major 1bpp pixel data (160 cols × 6 bytes) + 28 bytes padding = 992 total.
//! Bit 0 (LSB) of each byte is the topmost pixel in that column group.

use embedded_graphics::{
    mono_font::{ascii::FONT_5X8, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::time::Duration;

const LCD_W: i32 = 160;
const LCD_H: i32 = 43;
const DATA_OFFSET: usize = 4;         // report ID + 3 padding bytes
const BUF_LEN: usize = 992;           // 1 + 991 payload bytes

// ── Display buffer ───────────────────────────────────────────────────────────

struct G13Display([u8; BUF_LEN]);

impl G13Display {
    fn new() -> Self {
        let mut buf = [0u8; BUF_LEN];
        buf[0] = 0x03;
        Self(buf)
    }

    fn clear(&mut self) {
        self.0[1..].fill(0);
    }

    fn send(&self, path: &str) -> std::io::Result<()> {
        OpenOptions::new().write(true).open(path)?.write_all(&self.0)
    }
}

impl DrawTarget for G13Display {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where I: IntoIterator<Item = Pixel<Self::Color>>
    {
        for Pixel(Point { x, y }, color) in pixels {
            if x < 0 || x >= LCD_W || y < 0 || y >= LCD_H { continue; }
            // Page-major: page = y/8, byte = page*160 + x, bit = y%8 (LSB = top row)
            let idx = DATA_OFFSET + (y as usize / 8) * LCD_W as usize + x as usize;
            let bit = 1u8 << (y as usize % 8);
            if color.is_on() { self.0[idx] |= bit; } else { self.0[idx] &= !bit; }
        }
        Ok(())
    }
}

impl OriginDimensions for G13Display {
    fn size(&self) -> Size { Size::new(LCD_W as u32, LCD_H as u32) }
}

// ── System stats ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct CpuSample { total: u64, idle: u64 }

fn read_cpu() -> Option<CpuSample> {
    let data = fs::read_to_string("/proc/stat").ok()?;
    let nums: Vec<u64> = data.lines().next()?
        .split_whitespace().skip(1)
        .filter_map(|s| s.parse().ok()).collect();
    if nums.len() < 4 { return None; }
    let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
    Some(CpuSample { total: nums.iter().sum(), idle })
}

fn cpu_pct(prev: CpuSample, curr: CpuSample) -> u32 {
    let dt = curr.total.saturating_sub(prev.total);
    if dt == 0 { return 0; }
    let used = dt.saturating_sub(curr.idle.saturating_sub(prev.idle));
    ((used * 100) / dt) as u32
}

fn read_mem_pct() -> u32 {
    let data = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let (mut total, mut avail) = (0u64, 0u64);
    for line in data.lines() {
        let mut p = line.split_whitespace();
        match p.next() {
            Some("MemTotal:")     => { total = p.next().and_then(|s| s.parse().ok()).unwrap_or(0); }
            Some("MemAvailable:") => { avail = p.next().and_then(|s| s.parse().ok()).unwrap_or(0); }
            _ => {}
        }
    }
    if total == 0 { return 0; }
    let used = total.saturating_sub(avail);
    ((used * 100) / total) as u32
}

fn uptime_str() -> String {
    let secs = fs::read_to_string("/proc/uptime").ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let (d, h, m) = (secs / 86400, (secs % 86400) / 3600, (secs % 3600) / 60);
    if d > 0 { format!("Up {}d {:02}h {:02}m", d, h, m) }
    else if h > 0 { format!("Up {}h {:02}m", h, m) }
    else { format!("Up {}m", m) }
}

fn load_str() -> String {
    fs::read_to_string("/proc/loadavg").ok()
        .map(|s| format!("Load {}", s.split_whitespace().take(3).collect::<Vec<_>>().join(" ")))
        .unwrap_or_else(|| "Load -".into())
}

// ── Rendering ────────────────────────────────────────────────────────────────

const ROW: i32 = 9;        // row pitch: 8px char + 1px gap
const LABEL_W: i32 = 20;   // "CPU " / "RAM " label width (4 chars × 5px)
const BAR_X: i32 = LABEL_W;
const PCT_W: i32 = 20;     // " xx%" width (4 chars × 5px)
const BAR_W: i32 = LCD_W - BAR_X - PCT_W; // 120px

fn label_style() -> MonoTextStyle<'static, BinaryColor> {
    MonoTextStyle::new(&FONT_5X8, BinaryColor::On)
}

fn put(d: &mut G13Display, x: i32, row: i32, s: &str) {
    Text::with_baseline(s, Point::new(x, row * ROW), label_style(), Baseline::Top)
        .draw(d).ok();
}

fn draw_bar(d: &mut G13Display, row: i32, pct: u32) {
    let y = row * ROW;
    let outline = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);

    Rectangle::new(Point::new(BAR_X, y), Size::new(BAR_W as u32, 7))
        .into_styled(outline).draw(d).ok();

    let inner_w = ((pct.min(100) as i32 * (BAR_W - 2)) / 100).max(0) as u32;
    if inner_w > 0 {
        Rectangle::new(Point::new(BAR_X + 1, y + 1), Size::new(inner_w, 5))
            .into_styled(fill).draw(d).ok();
    }
}

fn render(d: &mut G13Display, cpu: u32, mem: u32) {
    d.clear();
    put(d, 0, 0, "G13 Suite");

    put(d, 0, 1, "CPU ");
    draw_bar(d, 1, cpu);
    put(d, BAR_X + BAR_W, 1, &format!("{:3}%", cpu));

    put(d, 0, 2, "RAM ");
    draw_bar(d, 2, mem);
    put(d, BAR_X + BAR_W, 2, &format!("{:3}%", mem));

    put(d, 0, 3, &uptime_str());
    put(d, 0, 4, &load_str());
}

// ── Discovery + run loop ─────────────────────────────────────────────────────

fn discover_hidraw() -> Option<String> {
    for e in fs::read_dir("/sys/bus/hid/devices").ok()?.flatten() {
        if e.file_name().to_string_lossy().to_uppercase().contains("046D:C21C") {
            if let Ok(hr) = fs::read_dir(e.path().join("hidraw")) {
                if let Some(hre) = hr.flatten().next() {
                    return Some(format!("/dev/{}", hre.file_name().to_string_lossy()));
                }
            }
        }
    }
    None
}

pub fn run() {
    let mut display = G13Display::new();
    let mut prev = match read_cpu() { Some(s) => s, None => return };
    let mut path: Option<String> = None;

    loop {
        std::thread::sleep(Duration::from_secs(1));

        // Re-discover hidraw on startup or after disconnect.
        if path.is_none() {
            path = discover_hidraw();
            if let Some(ref p) = path {
                eprintln!("g13d: LCD → {p}");
            }
        }

        let curr = match read_cpu() { Some(s) => s, None => continue };
        let cpu = cpu_pct(prev, curr);
        prev = curr;
        let mem = read_mem_pct();

        if let Some(ref p) = path.clone() {
            render(&mut display, cpu, mem);
            if let Err(e) = display.send(p) {
                eprintln!("g13d: LCD write error: {e}");
                path = None; // trigger re-discovery next tick
            }
        }
    }
}
