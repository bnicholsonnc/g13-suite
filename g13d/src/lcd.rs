//! G13 LCD system monitor — 160×43 monochrome display.
//!
//! Pixel format (from community G13 driver): row-major linear packing.
//!   pixel index = y * 160 + x
//!   byte  = DATA_OFFSET + pixel_index / 8
//!   bit   = pixel_index % 8  (LSB = leftmost pixel in the byte)
//!
//! The LCD is driven via USB interrupt OUT endpoint 0x02 using USBDEVFS_BULK,
//! bypassing the kernel HID SET_REPORT path which the G13 ignores for LCD data.
//! Packet: [0x03][0x00][0x00][0x00][860 bytes pixel data][padding to 992]

use embedded_graphics::{
    mono_font::{ascii::FONT_5X8, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    primitives::{PrimitiveStyle, Rectangle},
    text::{Baseline, Text},
};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

const LCD_W: i32 = 160;
const LCD_H: i32 = 43;
const DATA_OFFSET: usize = 4;
const BUF_LEN: usize = 992;
const LCD_EP_OUT: u32 = 0x02;
const USB_TIMEOUT_MS: u32 = 1000;

// ── USBDEVFS_BULK ioctl ──────────────────────────────────────────────────────
// _IOWR('U', 2, struct usbdevfs_bulktransfer)
// struct size on 64-bit: 3×u32 + *mut u8 = 12 + 8 = 20 = 0x14
// ioctl = (3<<30) | (0x14<<16) | ('U'<<8) | 2 = 0xC0145502

#[repr(C)]
struct UsbBulk {
    ep: u32,
    len: u32,
    timeout: u32,
    data: *mut u8,
}
const USBDEVFS_BULK: u64 = 0xC014_5502;

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
}

impl DrawTarget for G13Display {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where I: IntoIterator<Item = Pixel<Self::Color>>
    {
        for Pixel(Point { x, y }, color) in pixels {
            if x < 0 || x >= LCD_W || y < 0 || y >= LCD_H { continue; }
            // Row-major: pixel = y*160+x, byte offset, LSB = leftmost pixel
            let pixel = y as usize * LCD_W as usize + x as usize;
            let idx = DATA_OFFSET + pixel / 8;
            let bit = 1u8 << (pixel % 8);
            if color.is_on() { self.0[idx] |= bit; } else { self.0[idx] &= !bit; }
        }
        Ok(())
    }
}

impl OriginDimensions for G13Display {
    fn size(&self) -> Size { Size::new(LCD_W as u32, LCD_H as u32) }
}

// ── USB device discovery ─────────────────────────────────────────────────────

fn find_usb_device() -> Option<String> {
    for e in fs::read_dir("/sys/bus/usb/devices").ok()?.flatten() {
        let p = e.path();
        let vid = fs::read_to_string(p.join("idVendor")).ok()?;
        let pid = fs::read_to_string(p.join("idProduct")).ok()?;
        if vid.trim() == "046d" && pid.trim() == "c21c" {
            let bus: u32 = fs::read_to_string(p.join("busnum")).ok()?.trim().parse().ok()?;
            let dev: u32 = fs::read_to_string(p.join("devnum")).ok()?.trim().parse().ok()?;
            return Some(format!("/dev/bus/usb/{bus:03}/{dev:03}"));
        }
    }
    None
}

// ── Send via raw USB interrupt OUT endpoint ──────────────────────────────────

fn send_usb(usb_path: &str, buf: &mut [u8]) -> std::io::Result<()> {
    let file = OpenOptions::new().read(true).write(true).open(usb_path)?;
    let mut xfer = UsbBulk {
        ep: LCD_EP_OUT,
        len: buf.len() as u32,
        timeout: USB_TIMEOUT_MS,
        data: buf.as_mut_ptr(),
    };
    let ret = unsafe { libc::ioctl(file.as_raw_fd(), USBDEVFS_BULK, &mut xfer) };
    if ret < 0 { Err(std::io::Error::last_os_error()) } else { Ok(()) }
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
    ((total.saturating_sub(avail) * 100) / total) as u32
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

const ROW: i32 = 9;
const LABEL_W: i32 = 20;
const BAR_X: i32 = LABEL_W;
const PCT_W: i32 = 20;
const BAR_W: i32 = LCD_W - BAR_X - PCT_W;

fn style() -> MonoTextStyle<'static, BinaryColor> {
    MonoTextStyle::new(&FONT_5X8, BinaryColor::On)
}

fn put(d: &mut G13Display, x: i32, row: i32, s: &str) {
    Text::with_baseline(s, Point::new(x, row * ROW), style(), Baseline::Top)
        .draw(d).ok();
}

fn draw_bar(d: &mut G13Display, row: i32, pct: u32) {
    let y = row * ROW;
    Rectangle::new(Point::new(BAR_X, y), Size::new(BAR_W as u32, 7))
        .into_styled(PrimitiveStyle::with_stroke(BinaryColor::On, 1))
        .draw(d).ok();
    let inner_w = ((pct.min(100) as i32 * (BAR_W - 2)) / 100).max(0) as u32;
    if inner_w > 0 {
        Rectangle::new(Point::new(BAR_X + 1, y + 1), Size::new(inner_w, 5))
            .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
            .draw(d).ok();
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

// ── Run loop ─────────────────────────────────────────────────────────────────

pub fn run() {
    let mut display = G13Display::new();
    let mut prev = match read_cpu() { Some(s) => s, None => return };
    let mut usb_path: Option<String> = None;

    loop {
        std::thread::sleep(Duration::from_secs(1));

        if usb_path.is_none() {
            usb_path = find_usb_device();
            if let Some(ref p) = usb_path {
                eprintln!("g13d: LCD → {p} (EP 0x02)");
            }
        }

        let curr = match read_cpu() { Some(s) => s, None => continue };
        let cpu = cpu_pct(prev, curr);
        prev = curr;

        if let Some(ref p) = usb_path.clone() {
            render(&mut display, cpu, read_mem_pct());
            if let Err(e) = send_usb(p, &mut display.0) {
                eprintln!("g13d: LCD error: {e}");
                usb_path = None;
            }
        }
    }
}
