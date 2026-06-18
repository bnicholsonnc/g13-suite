//! g13-config-ui — a localhost web UI for configuring the G13.
//!
//! Serves the config page and exposes a small JSON API the page uses to read and
//! write /etc/g13d/config.toml, live-preview the RGB backlight, and reload the
//! daemon. Bind is 127.0.0.1 only.

use anyhow::Result;
use g13_config::{backlight, Config};
use std::io::Cursor;
use std::path::PathBuf;
use std::process::Command;
use tiny_http::{Header, Method, Response, Server};

const INDEX: &str = include_str!("../assets/index.html");
const IMG: &[u8] = include_bytes!("../assets/g13.png");

fn cfg_path() -> PathBuf {
    std::env::var("G13_CONFIG").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(g13_config::DEFAULT_PATH))
}

fn json(body: String, code: u32) -> Response<Cursor<Vec<u8>>> {
    let h = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    Response::from_string(body).with_header(h).with_status_code(code)
}

fn main() -> Result<()> {
    let addr = std::env::var("G13_UI_ADDR").unwrap_or_else(|_| "127.0.0.1:8137".into());
    let server = Server::http(&addr).map_err(|e| anyhow::anyhow!("bind {addr}: {e}"))?;
    eprintln!("g13-config-ui: open http://{addr}/ in your browser");

    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let url = req.url().to_string();
        let path = url.split('?').next().unwrap_or("/");

        let resp_result = match (&method, path) {
            (Method::Get, "/") => {
                let h = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..]).unwrap();
                req.respond(Response::from_string(INDEX).with_header(h))
            }
            (Method::Get, "/g13.png") => {
                let h = Header::from_bytes(&b"Content-Type"[..], &b"image/png"[..]).unwrap();
                req.respond(Response::from_data(IMG).with_header(h))
            }
            (Method::Get, "/api/config") => {
                let cfg = Config::load_or_default(&cfg_path());
                let body = serde_json::to_string(&cfg).unwrap_or_else(|_| "{}".into());
                req.respond(json(body, 200))
            }
            (Method::Get, "/api/leds") => {
                let mut v: Vec<String> = Vec::new();
                if let Ok(rd) = std::fs::read_dir("/sys/class/leds") {
                    for e in rd.flatten() { v.push(e.file_name().to_string_lossy().into_owned()); }
                }
                let disc = backlight::discover().map(|p| p.to_string_lossy().into_owned());
                let body = serde_json::json!({ "leds": v, "discovered": disc }).to_string();
                req.respond(json(body, 200))
            }
            (Method::Post, "/api/rgb") => {
                // live preview: body = {"r":..,"g":..,"b":..,"led":optional path}
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(v) => {
                        let r = v["r"].as_u64().unwrap_or(0) as u8;
                        let g = v["g"].as_u64().unwrap_or(0) as u8;
                        let b = v["b"].as_u64().unwrap_or(0) as u8;
                        let led = v["led"].as_str().map(|s| s.to_string());
                        let (ok, msg) = match backlight::apply(r, g, b, led.as_deref()) {
                            Ok(true) => (true, String::new()),
                            Ok(false) => (false, "no backlight LED found".into()),
                            Err(e) => (false, e.to_string()),
                        };
                        req.respond(json(serde_json::json!({"ok": ok, "error": msg}).to_string(), 200))
                    }
                    Err(e) => req.respond(json(serde_json::json!({"ok":false,"error":e.to_string()}).to_string(), 400)),
                }
            }
            (Method::Post, "/api/config") => {
                let mut body = String::new();
                let _ = req.as_reader().read_to_string(&mut body);
                match serde_json::from_str::<Config>(&body) {
                    Ok(cfg) => {
                        let outcome = save_and_reload(&cfg);
                        let ok = outcome.is_ok();
                        let msg = outcome.err().map(|e| e.to_string()).unwrap_or_default();
                        req.respond(json(serde_json::json!({"ok": ok, "error": msg}).to_string(), if ok {200} else {500}))
                    }
                    Err(e) => req.respond(json(serde_json::json!({"ok":false,"error":e.to_string()}).to_string(), 400)),
                }
            }
            _ => req.respond(Response::from_string("not found").with_status_code(404)),
        };
        if let Err(e) = resp_result { eprintln!("response error: {e}"); }
    }
    Ok(())
}

fn save_and_reload(cfg: &Config) -> Result<()> {
    let toml = cfg.to_toml()?;
    let path = cfg_path();
    if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).ok(); }
    std::fs::write(&path, toml)?;
    // best-effort reload; ignore failure (e.g. running unprivileged for preview)
    let _ = Command::new("systemctl").args(["restart", "g13d"]).status();
    Ok(())
}
