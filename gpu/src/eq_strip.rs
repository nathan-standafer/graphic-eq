//! Illuminated EQ faders. The native slider handles input and accessibility; the painter supplies the skin.
use crate::bands;
use eframe::egui::{self, Color32, FontId, Rect, RichText, Stroke, StrokeKind, pos2, vec2};

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgb(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
    )
}

fn glow(c: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha)
}

fn gradient(p: &egui::Painter, rect: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::epaint::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(egui::Shape::mesh(mesh));
}

fn gain_control(ui: &mut egui::Ui, control: Rect, gain: &mut f32) -> egui::Response {
    let before = *gain;
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(control));
    child.spacing_mut().slider_width = control.height();
    child.spacing_mut().interact_size.y = control.width();
    let mut response = child.scope(|ui| {
        ui.set_opacity(0.0);
        ui.add(egui::Slider::new(gain, bands::GAIN_MIN..=bands::GAIN_MAX)
            .vertical().step_by(0.5).show_value(false)
            .handle_shape(egui::style::HandleShape::Rect { aspect_ratio: 0.6 }))
    }).inner.interact(egui::Sense::click());
    if response.double_clicked() {
        *gain = 0.0;
    }
    if *gain != before {
        response.mark_changed();
    }
    response
}

pub fn show(ui: &mut egui::Ui, gains: &mut [f32; 11], levels: &[f32; 11], theme: u32) -> bool {
    ui.add_space(7.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("GRAPHIC EQ").strong().size(11.0).color(theme_color(theme, 0.3)));
        ui.label(RichText::new("11 BANDS  /  ±12 dB").monospace().size(10.0).color(Color32::from_gray(115)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(RichText::new("DOUBLE-CLICK TO RESET").size(9.0).color(Color32::from_gray(95)));
        });
    });
    ui.add_space(5.0);
    let mut changed = false;
    ui.columns(bands::COUNT, |cols| {
        for (i, col) in cols.iter_mut().enumerate() {
            let color = theme_color(theme, i as f32 / (bands::COUNT - 1) as f32);
            let level = levels[i].clamp(0.0, 1.0);
            let (card, _) = col.allocate_exact_size(vec2(col.available_width(), 218.0), egui::Sense::hover());
            let p = col.painter().clone();
            p.rect_filled(card, 7.0, Color32::from_rgb(12, 15, 22));
            gradient(&p, card.shrink(4.0), mix(Color32::from_rgb(20, 25, 34), color, 0.09), Color32::from_rgb(10, 13, 20));
            p.rect_stroke(card, 7.0, Stroke::new(1.0, mix(Color32::from_rgb(33, 40, 51), color, 0.16)), StrokeKind::Inside);
            let cap = Rect::from_center_size(pos2(card.center().x, card.top() + 2.5), vec2(card.width() - 14.0, 2.0));
            p.rect_filled(cap, 1.0, glow(color, 150));

            // Give the native control a generous hit target, then draw its visible rail and handle at exactly
            // the same positions (egui's rectangular handle inset is width / 2.5 * aspect_ratio).
            let width = (card.width() - 24.0).clamp(28.0, 42.0);
            let control = Rect::from_center_size(pos2(card.center().x - 3.0, card.top() + 112.0), vec2(width, 154.0));
            let response = gain_control(col, control, &mut gains[i]);
            changed |= response.changed();
            response.widget_info(|| egui::WidgetInfo::slider(true, gains[i] as f64, format!("{} Hz gain", bands::FREQS[i])));
            let hot = response.hovered() || response.dragged() || response.has_focus();
            let rect = response.rect;
            let inset = rect.width() / 2.5 * 0.6;
            let top = rect.top() + inset;
            let bottom = rect.bottom() - inset;
            let y_at = |db: f32| egui::lerp(bottom..=top, (db - bands::GAIN_MIN) / (bands::GAIN_MAX - bands::GAIN_MIN));
            let x = rect.center().x;
            let zero = y_at(0.0);
            let y = y_at(gains[i]);
            let rail = Rect::from_min_max(pos2(x - 4.0, top), pos2(x + 4.0, bottom));
            p.rect_filled(rail.expand(2.0), 5.0, Color32::from_rgb(31, 37, 48));
            p.rect_filled(rail, 4.0, Color32::from_rgb(3, 5, 9));
            p.line_segment([pos2(x + 3.0, top + 4.0), pos2(x + 3.0, bottom - 4.0)], Stroke::new(1.0, Color32::from_rgb(48, 55, 67)));
            let fill = Rect::from_min_max(pos2(x - 2.0, y.min(zero)), pos2(x + 2.0, y.max(zero)));
            if fill.height() > 0.5 {
                p.rect_filled(fill.expand(3.0), 4.0, glow(color, 25));
                p.rect_filled(fill, 2.0, mix(color, Color32::WHITE, 0.15));
            }
            for db in [-12.0f32, -9.0, -6.0, -3.0, 0.0, 3.0, 6.0, 9.0, 12.0] {
                let tick_y = y_at(db);
                let len = if db == 0.0 { 8.0 } else if db.abs() % 6.0 == 0.0 { 5.0 } else { 3.0 };
                let ink = if db == 0.0 { glow(color, 180) } else { Color32::from_rgb(59, 67, 80) };
                p.line_segment([pos2(x - 10.0 - len, tick_y), pos2(x - 10.0, tick_y)], Stroke::new(1.0, ink));
            }
            // A separate, segmented signal meter keeps audio level distinct from the gain setting.
            let meter_x = card.right() - 10.0;
            for step in 0..20 {
                let yy = bottom - step as f32 * (bottom - top) / 20.0;
                let led = Rect::from_min_size(pos2(meter_x - 2.0, yy - 3.5), vec2(4.0, 3.5));
                let on = level * 20.0 > step as f32;
                p.rect_filled(led, 1.0, if on { mix(color, Color32::WHITE, step as f32 / 60.0) } else { mix(Color32::from_rgb(20, 25, 33), color, 0.10) });
            }
            let handle = Rect::from_center_size(pos2(x, y), vec2(width + 2.0, 2.0 * inset));
            for (spread, alpha) in [(7.0, 8), (4.0, 16), (2.0, 25)] {
                p.rect_filled(handle.expand(spread), 7.0, glow(color, if hot { alpha * 2 } else { alpha }));
            }
            p.rect_filled(handle.translate(vec2(0.0, 3.0)), 4.0, Color32::from_black_alpha(140));
            p.rect_filled(handle, 4.0, mix(Color32::from_rgb(40, 49, 64), color, if hot { 0.40 } else { 0.20 }));
            gradient(&p, handle.shrink(2.0), mix(Color32::from_rgb(110, 124, 142), color, 0.28), Color32::from_rgb(25, 31, 42));
            p.rect_stroke(handle, 4.0, Stroke::new(1.0, mix(color, Color32::WHITE, if hot { 0.65 } else { 0.3 })), StrokeKind::Inside);
            let slit = Rect::from_center_size(handle.center(), vec2(handle.width() - 9.0, 2.5));
            p.rect_filled(slit.expand(2.0), 2.0, glow(color, 55));
            p.rect_filled(slit, 1.0, mix(color, Color32::WHITE, 0.6));
            if response.has_focus() {
                p.rect_stroke(handle.expand(3.0), 6.0, Stroke::new(1.5, color), StrokeKind::Outside);
            }
            response.on_hover_text(format!("{} Hz · {:+.1} dB\nDrag or use arrow keys · Double-click to reset", bands::FREQS[i], gains[i]));

            let badge = Rect::from_center_size(pos2(card.center().x, card.top() + 21.0), vec2((card.width() - 12.0).min(66.0), 24.0));
            p.rect_filled(badge, 5.0, mix(Color32::from_rgb(10, 15, 23), color, 0.12));
            let value = if gains[i].abs() < 0.05 { "0.0".to_owned() } else { format!("{:+.1}", gains[i]) };
            p.text(badge.center(), egui::Align2::CENTER_CENTER, value, FontId::monospace(14.0), mix(color, Color32::WHITE, 0.65));
            p.text(pos2(card.center().x, card.bottom() - 15.0), egui::Align2::CENTER_CENTER, bands::label(bands::FREQS[i]), FontId::proportional(13.0), mix(color, Color32::WHITE, 0.5));
        }
    });
    ui.add_space(6.0);
    changed
}
/// A theme's colour along its dominant -> supporting -> accent ramp, for UI accents (0..1).
pub fn theme_color(theme: u32, t: f32) -> Color32 {
    let ramp = |a: [f32; 3], b: [f32; 3], c: [f32; 3]| -> [f32; 3] {
        let (lo, hi) = (((t / 0.6).clamp(0.0, 1.0)), ((t - 0.6) / 0.4).clamp(0.0, 1.0));
        let m = |x: [f32; 3], y: [f32; 3], k: f32| [x[0] + (y[0] - x[0]) * k, x[1] + (y[1] - x[1]) * k, x[2] + (y[2] - x[2]) * k];
        if t < 0.6 { m(a, b, lo) } else { m(b, c, hi) }
    };
    let c = match theme {
        0 => ramp([0.10, 1.0, 0.35], [1.0, 0.9, 0.2], [1.0, 0.15, 0.2]),
        1 => ramp([1.0, 0.65, 0.0], [1.0, 0.4, 0.0], [1.0, 0.12, 0.0]),
        2 => ramp([0.0, 0.9, 1.0], [0.3, 0.5, 1.0], [0.75, 0.3, 1.0]),
        3 => ramp([1.0, 0.15, 0.85], [0.5, 0.35, 1.0], [0.0, 0.9, 1.0]),
        _ => {
            let h = (0.85 - 0.8 * t).rem_euclid(1.0);
            let hsv = egui::ecolor::Hsva::new(h, 0.85, 1.0, 1.0);
            let c = Color32::from(hsv);
            return c;
        }
    };
    Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skinned_fader_preserves_drag_keyboard_and_reset() {
        let ctx = egui::Context::default();
        let mut gain = 0.0;
        let mut time = 0.0;
        let mut frame = |gain: &mut f32, events: Vec<egui::Event>| {
            time += 0.05;
            let mut response = None;
            let mut output = ctx.run_ui(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 240.0))),
                time: Some(time), events, ..Default::default()
            }, |ui| {
                response = Some(gain_control(ui, Rect::from_min_size(pos2(50.0, 40.0), vec2(42.0, 154.0)), gain));
            });
            output.textures_delta.clear();
            response.unwrap()
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE,
        };
        let rect = frame(&mut gain, vec![]).rect;
        let center = rect.center();
        frame(&mut gain, vec![egui::Event::PointerMoved(center), button(center, true)]);
        let top = pos2(center.x, rect.top());
        let moved = frame(&mut gain, vec![egui::Event::PointerMoved(top)]);
        assert_eq!(gain, bands::GAIN_MAX);
        assert!(moved.changed());
        frame(&mut gain, vec![button(top, false)]);
        // Tab focus and keyboard changes must continue working with the native widget hidden.
        ctx.memory_mut(|m| m.request_focus(moved.id));
        gain = 0.0;
        let key = frame(&mut gain, vec![egui::Event::Key {
            key: egui::Key::ArrowUp, physical_key: None, pressed: true,
            repeat: false, modifiers: egui::Modifiers::NONE,
        }]);
        assert_eq!(gain.abs(), 0.5);
        assert!(key.changed());
        // Reset at a nonzero position so a click-to-position cannot masquerade as a reset.
        let raised = pos2(center.x, rect.top() + 35.0);
        frame(&mut gain, vec![egui::Event::PointerMoved(raised), button(raised, true)]);
        frame(&mut gain, vec![button(raised, false)]);
        frame(&mut gain, vec![button(raised, true)]);
        let reset = frame(&mut gain, vec![button(raised, false)]);
        assert!(reset.double_clicked());
        assert!(reset.changed());
        assert_eq!(gain, 0.0);
    }
}
