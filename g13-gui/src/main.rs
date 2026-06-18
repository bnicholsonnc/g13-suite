//! g13-gui — native configurator for the G13 (egui/eframe).
//!
//! Click keys on a picture of the G13 to bind them, manage profiles (switched by
//! the M-keys), configure the thumbstick, and pick a per-profile backlight colour.
//! Live colour preview writes the LED directly (needs the LED group-writable; see
//! dist/99-g13-leds.rules). "Save & Apply" writes /etc/g13d/config.toml via pkexec
//! and reloads the daemon.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use g13_config::{parse_color, Config, Profile};
use std::path::PathBuf;
use std::process::Command;

/// Hotspot layout: id, centre x%, centre y%, is-M-key.
const KEYS: &[(&str, f32, f32, bool)] = &[
    ("M1", 27.0, 18.8, true), ("M2", 41.0, 18.8, true), ("M3", 55.0, 18.8, true), ("MR", 68.1, 18.8, true),
    ("G1", 22.3, 30.0, false), ("G2", 31.8, 30.0, false), ("G3", 41.2, 30.0, false), ("G4", 50.6, 30.0, false),
    ("G5", 60.0, 30.0, false), ("G6", 69.5, 30.0, false), ("G7", 78.9, 30.0, false),
    ("G8", 24.4, 38.7, false), ("G9", 34.0, 38.7, false), ("G10", 43.6, 38.7, false), ("G11", 53.2, 38.7, false),
    ("G12", 62.5, 38.7, false), ("G13", 71.9, 38.7, false), ("G14", 80.3, 38.7, false),
    ("G15", 30.2, 48.5, false), ("G16", 40.7, 48.5, false), ("G17", 51.1, 48.5, false), ("G18", 61.4, 48.5, false),
    ("G19", 72.4, 48.5, false),
    ("G20", 36.6, 57.6, false), ("G21", 49.7, 57.6, false), ("G22", 60.2, 57.6, false),
];

const QUICK: &[&str] = &[
    "SPACE", "TAB", "LEFTSHIFT", "LEFTCTRL", "LEFTALT", "ESC", "B", "M",
    "GRAVE", "MINUS", "EQUAL", "1", "2", "3", "4", "5", "6", "F1", "F2",
];

enum Edit {
    Key(String),
    Mkey(String),
}

struct App {
    cfg: Config,
    active: String,
    led_path: Option<String>,
    led_options: Vec<String>,
    texture: Option<egui::TextureHandle>,
    edit: Option<Edit>,
    edit_value: String,
    edit_profile: String,
    status: String,
    status_err: bool,
}

fn config_path() -> PathBuf {
    PathBuf::from(g13_config::DEFAULT_PATH)
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // load the embedded image into a texture
        let texture = {
            let bytes = include_bytes!("../assets/g13.png");
            match image::load_from_memory(bytes) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    let size = [img.width() as usize, img.height() as usize];
                    let pixels = img.into_raw();
                    let color = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);
                    Some(cc.egui_ctx.load_texture("g13", color, egui::TextureOptions::LINEAR))
                }
                Err(_) => None,
            }
        };

        let mut cfg = Config::load_or_default(&config_path());
        // fold any [bindings] shorthand into profiles.default so the UI is uniform
        if !cfg.bindings.is_empty() {
            let d = cfg.profiles.entry("default".into()).or_default();
            for (k, v) in cfg.bindings.clone() {
                d.keys.entry(k).or_insert(v);
            }
            cfg.bindings.clear();
        }
        if cfg.profiles.is_empty() {
            cfg.profiles.insert("default".into(), Profile::default());
        }
        let active = if cfg.profiles.contains_key("default") {
            "default".to_string()
        } else {
            cfg.profiles.keys().next().cloned().unwrap()
        };

        let led_path = g13_config::backlight::discover().map(|p| p.to_string_lossy().into_owned());
        let mut led_options = Vec::new();
        if let Ok(rd) = std::fs::read_dir("/sys/class/leds") {
            for e in rd.flatten() {
                led_options.push(format!("/sys/class/leds/{}", e.file_name().to_string_lossy()));
            }
        }
        led_options.sort();

        App {
            cfg,
            active,
            led_path,
            led_options,
            texture,
            edit: None,
            edit_value: String::new(),
            edit_profile: String::new(),
            status: String::new(),
            status_err: false,
        }
    }

    fn set_status(&mut self, msg: impl Into<String>, err: bool) {
        self.status = msg.into();
        self.status_err = err;
    }

    fn live_color(&mut self, rgb: [u8; 3]) {
        let led = self.led_path.clone();
        match g13_config::backlight::apply(rgb[0], rgb[1], rgb[2], led.as_deref()) {
            Ok(true) => self.set_status(format!("Preview {},{},{}", rgb[0], rgb[1], rgb[2]), false),
            Ok(false) => self.set_status("No backlight LED found", true),
            Err(e) => self.set_status(format!("Backlight: {e}"), true),
        }
    }

    fn save(&mut self) {
        // ensure bindings shorthand is cleared; colours already Option
        self.cfg.bindings.clear();
        let toml = match self.cfg.to_toml() {
            Ok(t) => t,
            Err(e) => {
                self.set_status(format!("Serialize error: {e}"), true);
                return;
            }
        };
        let tmp = std::env::temp_dir().join("g13d-config.toml");
        if let Err(e) = std::fs::write(&tmp, &toml) {
            self.set_status(format!("Write temp failed: {e}"), true);
            return;
        }
        let script = format!(
            "install -Dm644 '{}' /etc/g13d/config.toml && systemctl restart g13d",
            tmp.display()
        );
        match Command::new("pkexec").arg("sh").arg("-c").arg(script).status() {
            Ok(s) if s.success() => self.set_status("Saved & daemon reloaded.", false),
            Ok(s) => self.set_status(format!("pkexec exited with {s}"), true),
            Err(e) => self.set_status(format!("pkexec failed: {e}"), true),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("G13 Configurator");
            ui.separator();
            ui.label("Profile:");
            let names: Vec<String> = self.cfg.profiles.keys().cloned().collect();
            egui::ComboBox::from_id_source("profile_sel")
                .selected_text(self.active.clone())
                .show_ui(ui, |ui| {
                    for n in &names {
                        ui.selectable_value(&mut self.active, n.clone(), n);
                    }
                });
            if ui.button("New").clicked() {
                let mut i = 1;
                let mut name = format!("profile{i}");
                while self.cfg.profiles.contains_key(&name) {
                    i += 1;
                    name = format!("profile{i}");
                }
                self.cfg.profiles.insert(name.clone(), Profile::default());
                self.active = name;
            }
            if ui.button("Rename").clicked() {
                self.edit = None;
                self.edit_value = self.active.clone();
                self.edit_profile = "__rename__".into();
            }
            if ui.button("Delete").clicked() && self.cfg.profiles.len() > 1 {
                let removed = self.active.clone();
                self.cfg.profiles.remove(&removed);
                self.cfg.profile_keys.retain(|_, v| v != &removed);
                self.active = self.cfg.profiles.keys().next().cloned().unwrap();
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Save & Apply").clicked() {
                    self.save();
                }
                if !self.status.is_empty() {
                    let col = if self.status_err {
                        egui::Color32::from_rgb(224, 107, 107)
                    } else {
                        egui::Color32::from_rgb(111, 207, 127)
                    };
                    ui.colored_label(col, &self.status);
                }
            });
        });
    }

    fn board(&mut self, ui: &mut egui::Ui) {
        let avail = ui.available_size();
        let side = avail.x.min(avail.y).min(600.0).max(360.0);
        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());

        if let Some(tid) = self.texture.as_ref().map(|t| t.id()) {
            ui.painter().image(
                tid,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        } else {
            ui.painter().rect_filled(rect, 6.0, egui::Color32::from_gray(30));
        }

        let bw = 0.082 * rect.width();
        let bh = 0.075 * rect.height();
        let mut clicked: Option<Edit> = None;

        for &(id, xp, yp, is_m) in KEYS {
            let center = egui::pos2(
                rect.min.x + xp / 100.0 * rect.width(),
                rect.min.y + yp / 100.0 * rect.height(),
            );
            let kr = egui::Rect::from_center_size(center, egui::vec2(bw, bh));
            let resp = ui.interact(kr, egui::Id::new(("hot", id)), egui::Sense::click());

            let (border, fill) = if is_m {
                (egui::Color32::from_rgb(120, 170, 255), egui::Color32::from_rgba_unmultiplied(120, 170, 255, 40))
            } else {
                (egui::Color32::from_rgb(232, 161, 58), egui::Color32::from_rgba_unmultiplied(20, 22, 28, 90))
            };
            let fill = if resp.hovered() {
                egui::Color32::from_rgba_unmultiplied(border.r(), border.g(), border.b(), 60)
            } else {
                fill
            };
            ui.painter().rect_filled(kr, 4.0, fill);
            ui.painter().rect_stroke(kr, 4.0, egui::Stroke::new(1.5, border));

            // labels
            let sub = if is_m {
                self.cfg.profile_keys.get(id).map(|p| format!("\u{2192} {p}")).unwrap_or_default()
            } else {
                self.cfg.profiles.get(&self.active).and_then(|p| p.keys.get(id)).cloned().unwrap_or_default()
            };
            ui.painter().text(
                egui::pos2(kr.center().x, kr.top() + 8.0),
                egui::Align2::CENTER_CENTER,
                id,
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(235),
            );
            if !sub.is_empty() {
                ui.painter().text(
                    egui::pos2(kr.center().x, kr.bottom() - 7.0),
                    egui::Align2::CENTER_CENTER,
                    sub,
                    egui::FontId::proportional(9.5),
                    egui::Color32::from_gray(170),
                );
            }

            if resp.clicked() {
                clicked = Some(if is_m { Edit::Mkey(id.to_string()) } else { Edit::Key(id.to_string()) });
            }
        }

        if let Some(e) = clicked {
            match &e {
                Edit::Key(k) => {
                    self.edit_value = self
                        .cfg
                        .profiles
                        .get(&self.active)
                        .and_then(|p| p.keys.get(k))
                        .cloned()
                        .unwrap_or_default();
                }
                Edit::Mkey(m) => {
                    self.edit_profile = self
                        .cfg
                        .profile_keys
                        .get(m)
                        .cloned()
                        .unwrap_or_else(|| self.cfg.profiles.keys().next().cloned().unwrap());
                }
            }
            self.edit = Some(e);
        }

        ui.add_space(6.0);
        ui.label(
            egui::RichText::new(
                "Click a grey key to bind it (held while pressed). Click a blue M-key to switch profiles.",
            )
            .color(egui::Color32::from_gray(150))
            .size(12.0),
        );
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(egui::RichText::new("THUMBSTICK").strong().color(egui::Color32::from_gray(150)));
        ui.add_space(4.0);
        {
        let t = &mut self.cfg.thumbstick;
        egui::ComboBox::from_id_source("ts_mode")
            .selected_text(match t.mode.as_str() {
                "gamepad" => "Gamepad (analog)",
                "off" => "Off (raw)",
                _ => "Keys (WASD/backpedal)",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut t.mode, "keys".into(), "Keys (WASD/backpedal)");
                ui.selectable_value(&mut t.mode, "gamepad".into(), "Gamepad (analog)");
                ui.selectable_value(&mut t.mode, "off".into(), "Off (raw)");
            });
        ui.add(egui::Slider::new(&mut t.deadzone, 0..=120).text("Deadzone"));
        ui.checkbox(&mut t.invert_x, "Invert X");
        ui.checkbox(&mut t.invert_y, "Invert Y");
        let keys_mode = t.mode == "keys";
        ui.add_enabled_ui(keys_mode, |ui| {
            egui::Grid::new("ts_keys").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                ui.label("Up");
                ui.text_edit_singleline(&mut t.up);
                ui.end_row();
                ui.label("Down");
                ui.text_edit_singleline(&mut t.down);
                ui.end_row();
                ui.label("Left");
                ui.text_edit_singleline(&mut t.left);
                ui.end_row();
                ui.label("Right");
                ui.text_edit_singleline(&mut t.right);
                ui.end_row();
                ui.label("Thumb (press down)");
                ui.text_edit_singleline(&mut t.thumb);
                ui.end_row();
                ui.label("Btn1 (left of stick)");
                ui.text_edit_singleline(&mut t.button1);
                ui.end_row();
                ui.label("Btn2 (below stick)");
                ui.text_edit_singleline(&mut t.button2);
                ui.end_row();
            });
        });
        ui.label(
            egui::RichText::new("Strafe (face forward while moving sideways): set Left/Right to Q/E.")
                .color(egui::Color32::from_gray(150))
                .size(11.0),
        );
        }

        ui.separator();
        ui.label(egui::RichText::new("BACKLIGHT (this profile)").strong().color(egui::Color32::from_gray(150)));
        ui.add_space(4.0);

        // colour for the active profile
        let cur = self.cfg.profiles.get(&self.active).and_then(|p| p.color.clone());
        let mut rgb = cur.as_deref().and_then(parse_color).map(|(r, g, b)| [r, g, b]).unwrap_or([0, 0, 0]);
        ui.horizontal(|ui| {
            ui.label("Colour");
            if ui.color_edit_button_srgb(&mut rgb).changed() {
                if let Some(p) = self.cfg.profiles.get_mut(&self.active) {
                    p.color = Some(format!("{},{},{}", rgb[0], rgb[1], rgb[2]));
                }
                self.live_color(rgb);
            }
            if ui.button("No colour").clicked() {
                if let Some(p) = self.cfg.profiles.get_mut(&self.active) {
                    p.color = None;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label("LED node");
            let cur = self.led_path.clone().unwrap_or_else(|| "(auto-detect)".into());
            egui::ComboBox::from_id_source("led_sel").selected_text(cur).show_ui(ui, |ui| {
                let mut none: Option<String> = self.led_path.clone();
                if ui.selectable_label(self.led_path.is_none(), "(auto-detect)").clicked() {
                    none = g13_config::backlight::discover().map(|p| p.to_string_lossy().into_owned());
                }
                for opt in &self.led_options {
                    if ui.selectable_label(self.led_path.as_deref() == Some(opt.as_str()), opt).clicked() {
                        none = Some(opt.clone());
                    }
                }
                self.led_path = none;
            });
        });
        if let Some(p) = &self.led_path {
            ui.label(egui::RichText::new(format!("Using {p}")).color(egui::Color32::from_gray(150)).size(11.0));
        } else {
            ui.label(egui::RichText::new("No G13 backlight LED detected.").color(egui::Color32::from_gray(150)).size(11.0));
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let btn = egui::Button::new("Sponsor on GitHub")
                .fill(egui::Color32::from_rgb(36, 163, 74));
            if ui.add(btn).clicked() {
                let _ = std::process::Command::new("xdg-open")
                    .arg("https://github.com/sponsors/bnicholsonnc")
                    .spawn();
            }
            ui.label(
                egui::RichText::new("If you find g13-suite useful, please consider\nsponsoring to support future development.")
                    .color(egui::Color32::from_gray(150))
                    .size(11.0),
            );
            ui.separator();
        });
    }

    fn edit_window(&mut self, ctx: &egui::Context) {
        let mut open = self.edit.is_some() || self.edit_profile == "__rename__";
        if !open {
            return;
        }

        // profile rename dialog
        if self.edit_profile == "__rename__" {
            let mut do_close = false;
            let mut apply = false;
            egui::Window::new("Rename profile")
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.text_edit_singleline(&mut self.edit_value);
                    ui.horizontal(|ui| {
                        if ui.button("OK").clicked() {
                            apply = true;
                            do_close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            do_close = true;
                        }
                    });
                });
            if apply {
                let new = self.edit_value.trim().to_string();
                if !new.is_empty() && new != self.active && !self.cfg.profiles.contains_key(&new) {
                    if let Some(p) = self.cfg.profiles.remove(&self.active) {
                        self.cfg.profiles.insert(new.clone(), p);
                    }
                    let old = self.active.clone();
                    for v in self.cfg.profile_keys.values_mut() {
                        if *v == old {
                            *v = new.clone();
                        }
                    }
                    self.active = new;
                }
            }
            if do_close || !open {
                self.edit_profile.clear();
            }
            return;
        }

        // key / M-key editor
        let title;
        let is_m;
        match self.edit.as_ref().unwrap() {
            Edit::Key(k) => {
                title = format!("Bind {k}");
                is_m = false;
            }
            Edit::Mkey(m) => {
                title = format!("M-key {m}");
                is_m = true;
            }
        }

        let mut do_close = false;
        let mut apply = false;
        let profile_names: Vec<String> = self.cfg.profiles.keys().cloned().collect();

        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                if is_m {
                    ui.label("Pressing this M-key activates:");
                    egui::ComboBox::from_id_source("mkey_target")
                        .selected_text(self.edit_profile.clone())
                        .show_ui(ui, |ui| {
                            for n in &profile_names {
                                ui.selectable_value(&mut self.edit_profile, n.clone(), n);
                            }
                        });
                } else {
                    ui.label("Output key or chord (e.g. SPACE, 1, LEFTCTRL+1):");
                    ui.text_edit_singleline(&mut self.edit_value);
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        for q in QUICK {
                            if ui.small_button(*q).clicked() {
                                self.edit_value = (*q).to_string();
                            }
                        }
                    });
                    ui.label(
                        egui::RichText::new("Leave blank to unbind.")
                            .color(egui::Color32::from_gray(150))
                            .size(11.0),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        apply = true;
                        do_close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        do_close = true;
                    }
                });
            });

        if apply {
            let target = match self.edit.as_ref().unwrap() {
                Edit::Key(k) => Edit::Key(k.clone()),
                Edit::Mkey(m) => Edit::Mkey(m.clone()),
            };
            match target {
                Edit::Key(k) => {
                    let v = self.edit_value.trim().to_string();
                    if let Some(p) = self.cfg.profiles.get_mut(&self.active) {
                        if v.is_empty() {
                            p.keys.remove(&k);
                        } else {
                            p.keys.insert(k, v);
                        }
                    }
                }
                Edit::Mkey(m) => {
                    self.cfg.profile_keys.insert(m, self.edit_profile.clone());
                }
            }
        }
        if do_close || !open {
            self.edit = None;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("topbar").show(ctx, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::SidePanel::right("controls").min_width(300.0).show(ctx, |ui| {
            self.controls(ui);
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            self.board(ui);
        });
        self.edit_window(ctx);
    }
}

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([960.0, 640.0])
            .with_title("G13 Configurator"),
        ..Default::default()
    };
    eframe::run_native(
        "G13 Configurator",
        native_options,
        Box::new(|cc| Ok(Box::new(App::new(cc)) as Box<dyn eframe::App>)),
    )
}
