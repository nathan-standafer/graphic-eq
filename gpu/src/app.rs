use crate::analyzer::{Levels, CURVE_POINTS, FLOOR_DB};
use crate::audio::Audio;
use crate::bands;
use crate::render::{Gpu, SceneCallback, SceneParams, HIST_W};
use crate::routing;
use crate::settings::{Settings, PRESETS};
use eframe::egui::{self, Color32, RichText};
use eframe::egui_wgpu;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const CUSTOM: &str = "Custom";
const FALL_DB_PER_S: [f32; 3] = [14.0, 38.0, 110.0];
const PEAK_HOLD_S: f32 = 1.1;
const PEAK_FALL_DB_PER_S: f32 = 22.0;
const HISTORY_RATE: f32 = 60.0;
const THEMES: [&str; 5] = ["Classic green/red", "Amber", "Ice", "Neon", "Rainbow"];

pub struct App {
    cfg: Settings,
    audio: Audio,
    quit: Arc<AtomicBool>,
    disp: [f32; bands::COUNT],
    peak: [f32; bands::COUNT],
    peak_age: [f32; bands::COUNT],
    curve: [f32; CURVE_POINTS],
    bass: f32,
    hist_acc: f32,
    sinks: Vec<(String, String)>,
    dirty_since: Option<std::time::Instant>,
    shot: Option<(String, f64)>, // GEQ_SHOT=path[,seconds]: save a screenshot of this window and exit
    shot_sent: bool,
    fps_frames: u32,
    fps_t0: f64,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let cfg = Settings::load();
        let rs = cc.wgpu_render_state.as_ref().expect("wgpu backend required");
        let gpu = Gpu::new(&rs.device, rs.target_format);
        rs.renderer.write().callback_resources.insert(gpu);

        let quit = Arc::new(AtomicBool::new(false));
        let q = quit.clone();
        let _ = ctrlc::set_handler(move || q.store(true, Ordering::SeqCst));

        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(14, 16, 22);
        visuals.window_fill = Color32::from_rgb(22, 26, 34);
        visuals.handle_shape = egui::style::HandleShape::Rect { aspect_ratio: 0.55 };
        cc.egui_ctx.set_visuals(visuals);
        cc.egui_ctx.all_styles_mut(|s| s.spacing.slider_rail_height = 8.0);

        let eff = effective(&cfg);
        let audio = Audio::new(cfg.output.clone(), cfg.source_pre, eff);
        App {
            cfg,
            audio,
            quit,
            disp: [FLOOR_DB; bands::COUNT],
            peak: [FLOOR_DB; bands::COUNT],
            peak_age: [0.0; bands::COUNT],
            curve: [FLOOR_DB; CURVE_POINTS],
            bass: 0.0,
            hist_acc: 0.0,
            sinks: Vec::new(),
            dirty_since: None,
            shot: std::env::var("GEQ_SHOT").ok().map(|v| {
                let mut p = v.splitn(2, ',');
                let path = p.next().unwrap().to_string();
                (path, p.next().and_then(|s| s.parse().ok()).unwrap_or(6.0))
            }),
            shot_sent: false,
            fps_frames: 0,
            fps_t0: 0.0,
        }
    }

    fn frac(&self, db: f32) -> f32 {
        ((db + self.cfg.sensitivity + self.cfg.range) / self.cfg.range).clamp(0.0, 1.0)
    }

    fn touch(&mut self) {
        self.dirty_since = Some(std::time::Instant::now());
    }

    fn push_gains(&mut self) {
        self.audio.set_gains(effective(&self.cfg));
    }

    fn step_levels(&mut self, dt: f32) {
        let lv: Levels = self.audio.levels().unwrap_or_default();
        let fall = FALL_DB_PER_S[self.cfg.fall as usize % 3];
        for i in 0..bands::COUNT {
            let r = lv.bands[i];
            self.disp[i] = if r >= self.disp[i] { r } else { (self.disp[i] - fall * dt).max(r) };
            if self.disp[i] >= self.peak[i] {
                self.peak[i] = self.disp[i];
                self.peak_age[i] = 0.0;
            } else {
                self.peak_age[i] += dt;
                if self.peak_age[i] > PEAK_HOLD_S {
                    self.peak[i] = (self.peak[i] - PEAK_FALL_DB_PER_S * dt).max(self.disp[i]);
                }
            }
        }
        for i in 0..CURVE_POINTS {
            let r = lv.curve[i];
            self.curve[i] = if r >= self.curve[i] { r } else { (self.curve[i] - fall * dt).max(r) };
        }
        let bass_now = (0..3).map(|i| self.frac(self.disp[i])).sum::<f32>() / 3.0;
        let k = if bass_now > self.bass { 0.5 } else { 0.08 };
        self.bass += (bass_now - self.bass) * k;
    }
}

fn effective(cfg: &Settings) -> [f32; bands::COUNT] {
    if cfg.eq_on { cfg.gains } else { [0.0; bands::COUNT] }
}

fn fmt_gain(g: f32) -> String {
    if g.abs() < 0.05 { "0".into() } else { format!("{g:+.1}") }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.quit.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let Ok(v) = std::env::var("GEQ_SCENE") {
            self.cfg.scene = v.parse().unwrap_or(0);
        }
        if let Ok(v) = std::env::var("GEQ_THEME") {
            self.cfg.theme = v.parse().unwrap_or(0);
        }
        if let Some((path, after)) = self.shot.clone() {
            let now = ctx.input(|i| i.time);
            if now > after && !self.shot_sent {
                self.shot_sent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            }
            let img = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = img {
                let [w, h] = img.size;
                let _ = image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8);
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if std::env::var_os("GEQ_FPS").is_some() {
            let now = ctx.input(|i| i.time);
            self.fps_frames += 1;
            if now - self.fps_t0 >= 2.0 {
                eprintln!("fps {:.1}  ({} frames, {}x{})", self.fps_frames as f64 / (now - self.fps_t0), self.fps_frames,
                    ctx.content_rect().width() as u32, ctx.content_rect().height() as u32);
                self.fps_frames = 0;
                self.fps_t0 = now;
            }
        }
        self.audio.poll();
        let dt = ctx.input(|i| i.unstable_dt).clamp(0.0, 0.1);
        self.step_levels(dt);
        ctx.request_repaint();

        // debounce settings writes
        if self.dirty_since.map_or(false, |t| t.elapsed().as_millis() > 500) {
            self.cfg.save();
            self.dirty_since = None;
        }

        // ---------------------------------------------------------------- header
        egui::Panel::top("top").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                let mut on = self.cfg.eq_on;
                if ui.add_enabled(self.audio.ready, egui::Checkbox::new(&mut on, "EQ")).changed() {
                    self.cfg.eq_on = on;
                    self.push_gains();
                    self.touch();
                }
                egui::ComboBox::from_id_salt("preset").selected_text(self.cfg.preset.clone()).show_ui(ui, |ui| {
                    for (name, g) in PRESETS.iter() {
                        if ui.selectable_label(self.cfg.preset == *name, *name).clicked() {
                            self.cfg.preset = (*name).into();
                            self.cfg.gains = *g;
                            self.push_gains();
                            self.touch();
                        }
                    }
                    ui.add_enabled(false, egui::Button::new(CUSTOM));
                });
                if ui.button("Reset").on_hover_text("All bands to 0 dB").clicked() {
                    self.cfg.preset = "Flat".into();
                    self.cfg.gains = [0.0; bands::COUNT];
                    self.push_gains();
                    self.touch();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(6.0);
                    self.options_menu(ui);
                    if !self.audio.status.is_empty() {
                        ui.label(RichText::new(&self.audio.status).color(Color32::from_rgb(224, 164, 88)));
                    }
                });
            });
        });

        // --------------------------------------------------------------- sliders
        egui::Panel::bottom("sliders").show_inside(ui, |ui| {
            ui.add_space(6.0);
            let mut changed = false;
            ui.columns(bands::COUNT, |cols| {
                for (i, col) in cols.iter_mut().enumerate() {
                    col.vertical_centered(|ui| {
                        let mut g = self.cfg.gains[i];
                        ui.label(RichText::new(fmt_gain(g)).monospace().size(12.0).color(
                            if g.abs() < 0.05 { Color32::from_gray(130) } else { Color32::WHITE },
                        ));
                        ui.spacing_mut().slider_width = 150.0;
                        let pad = (ui.available_width() - ui.spacing().interact_size.y) / 2.0;
                        let r = ui
                            .horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                ui.add_space(pad.max(0.0));
                                ui.add(
                                    egui::Slider::new(&mut g, bands::GAIN_MAX..=bands::GAIN_MIN)
                                        .vertical()
                                        .step_by(0.5)
                                        .show_value(false),
                                )
                            })
                            .inner;
                        if r.double_clicked() {
                            g = 0.0;
                        }
                        if r.changed() || r.double_clicked() {
                            self.cfg.gains[i] = g;
                            changed = true;
                        }
                        ui.label(RichText::new(bands::label(bands::FREQS[i])).strong().size(14.0));
                    });
                }
            });
            if changed {
                self.cfg.preset = CUSTOM.into();
                self.push_gains();
                self.touch();
            }
            ui.add_space(6.0);
        });

        // ------------------------------------------------------------------ scene
        egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
            let rect = ui.available_rect_before_wrap();
            ui.allocate_rect(rect, egui::Sense::hover());
            let ppp = ctx.pixels_per_point();
            let size_px = [(rect.width() * ppp) as u32, (rect.height() * ppp) as u32];

            // waterfall history rows at a fixed rate, independent of frame rate
            self.hist_acc += dt;
            let mut rows = Vec::new();
            while self.hist_acc >= 1.0 / HISTORY_RATE {
                self.hist_acc -= 1.0 / HISTORY_RATE;
                let mut row = [0u8; HIST_W as usize];
                for (k, v) in self.curve.iter().enumerate() {
                    row[k] = (self.frac(*v) * 255.0) as u8;
                }
                rows.push(row);
            }
            let mut levels = [0f32; 11];
            let mut peaks = [0f32; 11];
            for i in 0..11 {
                levels[i] = self.frac(self.disp[i]);
                peaks[i] = if self.cfg.peak_caps { self.frac(self.peak[i]) } else { 0.0 };
            }
            let cb = SceneCallback {
                params: SceneParams {
                    time: ctx.input(|i| i.time) as f32,
                    mode: self.cfg.scene,
                    theme: self.cfg.theme,
                    bloom: self.cfg.bloom,
                    bass: self.bass,
                    levels,
                    peaks,
                },
                size_px,
                rows,
            };
            ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, cb));
        });
    }

    fn on_exit(&mut self) {
        self.cfg.save();
        self.audio.shutdown();
    }
}

/// A row of mutually exclusive choices (no nested popups, which would close the parent menu).
fn choice<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for (v, label) in options {
            if ui.selectable_label(*value == *v, *label).clicked() && *value != *v {
                *value = *v;
                changed = true;
            }
        }
    });
    changed
}

impl App {
    fn options_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Display", |ui| {
            ui.set_min_width(300.0);
            let mut touched = false;
            ui.label(RichText::new("SCENE").small().weak());
            touched |= choice(ui, &mut self.cfg.scene, &[(0, "3D columns"), (1, "Waterfall")]);
            ui.add_space(4.0);
            ui.label(RichText::new("COLORS").small().weak());
            let themes: Vec<(u32, &str)> = THEMES.iter().enumerate().map(|(i, n)| (i as u32, *n)).collect();
            touched |= choice(ui, &mut self.cfg.theme, &themes);
            ui.add_space(4.0);
            ui.label(RichText::new("RANGE").small().weak());
            touched |= choice(ui, &mut self.cfg.range, &[(40.0, "40 dB"), (60.0, "60 dB"), (80.0, "80 dB")]);
            ui.add_space(4.0);
            ui.label(RichText::new("FALL SPEED").small().weak());
            touched |= choice(ui, &mut self.cfg.fall, &[(0, "Slow"), (1, "Medium"), (2, "Fast")]);
            ui.add_space(4.0);
            egui::Grid::new("opts").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Glow");
                touched |= ui.add(egui::Slider::new(&mut self.cfg.bloom, 0.0..=2.0)).changed();
                ui.end_row();
                ui.label("Sensitivity (dB)");
                touched |= ui.add(egui::Slider::new(&mut self.cfg.sensitivity, -20.0..=20.0)).changed();
                ui.end_row();
                ui.label("Peak caps");
                touched |= ui.checkbox(&mut self.cfg.peak_caps, "").changed();
                ui.end_row();
            });
            ui.separator();
            ui.label(RichText::new("ANALYZE").small().weak());
            let mut pre = self.cfg.source_pre;
            choice(ui, &mut pre, &[(false, "After EQ (what you hear)"), (true, "Before EQ")]);
            if pre != self.cfg.source_pre {
                self.cfg.source_pre = pre;
                self.audio.set_source_pre(pre);
                touched = true;
            }
            ui.add_space(4.0);
            ui.label(RichText::new("OUTPUT DEVICE").small().weak());
            if self.sinks.is_empty() {
                self.sinks = routing::list_sinks();
            }
            let current = self.audio.real_sink.clone().unwrap_or_default();
            let mut pick: Option<String> = None;
            for (name, desc) in &self.sinks {
                if ui.selectable_label(*name == current, desc).clicked() && *name != current {
                    pick = Some(name.clone());
                }
            }
            if let Some(n) = pick {
                self.cfg.output = Some(n.clone());
                self.audio.set_output(n);
                touched = true;
            }
            if touched {
                self.touch();
            }
        })
        .response
        .on_hover_text("Display and audio options");
    }
}
