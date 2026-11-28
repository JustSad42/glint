//! Shared pieces: key caps, the toast, buttons, switches, segmented controls.
use super::assets::Assets;
use super::theme::{self, f, Font, Theme};
use egui::{pos2, vec2, Color32, CornerRadius, Id, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui};
use std::time::Instant;

pub fn cr(r: f32) -> CornerRadius {
    CornerRadius::same(r.round() as u8)
}

/// Smooth 0..1 for hover/press transitions (CSS `transition: .12s`).
pub fn anim(ui: &Ui, id: Id, on: bool, secs: f32) -> f32 {
    ui.ctx().animate_bool_with_time(id, on, secs)
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    a.lerp_to_gamma(b, t)
}

/// CSS cubic-bezier(.2,.8,.2,1), close enough.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powf(3.2)
}

// ---------------------------------------------------------------- key caps

const KBD_FONT: Font = f(11.0, 600.0);

pub fn kbd_width(ui: &Ui, text: &str) -> f32 {
    let g = theme::line(ui, text, KBD_FONT, Color32::WHITE, 200.0);
    (g.size().x + 12.0).max(20.0)
}

/// A key cap with its left edge at x, centred on cy. Returns its width.
pub fn kbd(ui: &Ui, t: &Theme, x: f32, cy: f32, text: &str) -> f32 {
    let g = theme::line(ui, text, KBD_FONT, t.text2, 200.0);
    let w = (g.size().x + 12.0).max(20.0);
    let r = Rect::from_min_size(pos2(x, cy - 10.0), vec2(w, 20.0));
    let p = ui.painter();
    p.rect_filled(r, cr(6.0), t.kbd);
    p.rect_stroke(r.shrink(0.5), cr(6.0), Stroke::new(1.0, t.line), StrokeKind::Middle);
    // the thicker bottom edge
    p.line_segment([pos2(r.left() + 4.0, r.bottom() - 1.5), pos2(r.right() - 4.0, r.bottom() - 1.5)], Stroke::new(1.0, t.line));
    let gx = r.center().x - g.size().x / 2.0;
    p.galley(pos2(gx, (cy - 0.5 - g.size().y / 2.0).round()), g, Color32::WHITE);
    w
}

/// Several key caps in a row with `gap` between; returns the total width.
pub fn keys(ui: &Ui, t: &Theme, x: f32, cy: f32, ks: &[&str], gap: f32) -> f32 {
    let mut cx = x;
    for (i, k) in ks.iter().enumerate() {
        if i > 0 {
            cx += gap;
        }
        cx += kbd(ui, t, cx, cy, k);
    }
    cx - x
}

pub fn keys_width(ui: &Ui, ks: &[&str], gap: f32) -> f32 {
    ks.iter().map(|k| kbd_width(ui, k)).sum::<f32>() + gap * ks.len().saturating_sub(1) as f32
}

// ---------------------------------------------------------------- toast

pub struct Toast {
    pub msg: String,
    pub err: bool,
    pub at: Instant,
    pub secs: f32,
}

impl Toast {
    pub fn new(msg: impl Into<String>, err: bool, secs: f32) -> Toast {
        Toast { msg: msg.into(), err, at: Instant::now(), secs }
    }

    /// Draw it centred at `cx`, bottom at `bottom`. Returns false once it has faded out.
    pub fn draw(&self, ui: &Ui, t: &Theme, a: &mut Assets, cx: f32, bottom: f32, bg: Color32) -> bool {
        let age = self.at.elapsed().as_secs_f32();
        let (op, dy) = if age < self.secs {
            let k = ease(age / 0.2);
            ((age / 0.15).min(1.0), 8.0 * (1.0 - k))
        } else {
            let k = (age - self.secs) / 0.15;
            if k >= 1.0 {
                return false;
            }
            (1.0 - k, 8.0 * ease((age - self.secs) / 0.2))
        };
        ui.ctx().request_repaint();
        let g = theme::line(ui, &self.msg, f(12.5, 500.0), theme::alpha(t.text, op), 700.0);
        let w = g.size().x + 15.0 + 8.0 + 28.0;
        let r = Rect::from_center_size(pos2(cx, bottom - 17.0 + dy), vec2(w, 34.0));
        let p = ui.painter();
        shadow(p, r, 17.0, theme::alpha(t.shadow, op), 18.0, 50.0);
        p.rect_filled(r, cr(17.0), theme::alpha(bg, op));
        p.rect_stroke(r, cr(17.0), Stroke::new(1.0, theme::alpha(t.line2, op)), StrokeKind::Inside);
        let ic = Rect::from_min_size(pos2(r.left() + 14.0, r.center().y - 7.5), vec2(15.0, 15.0));
        a.glyph(ui, if self.err { "warn" } else { "check" }, ic, theme::alpha(if self.err { t.danger } else { t.ok }, op));
        theme::put(p, &g, ic.right() + 8.0, r.center().y);
        true
    }
}

/// A soft drop shadow (CSS box-shadow: 0 dy blur color).
pub fn shadow(p: &egui::Painter, r: Rect, radius: f32, color: Color32, dy: f32, blur: f32) {
    let s = egui::epaint::Shadow { offset: [0, (dy * 0.5) as i8], blur: (blur * 0.6).min(255.0) as u8, spread: 0, color };
    p.add(s.as_shape(r, cr(radius)));
}

// ---------------------------------------------------------------- buttons

#[derive(Clone, Copy, PartialEq)]
pub enum Btn {
    Normal,
    Primary,
    Danger,
    Small,
}

/// `.btn`: 34 tall, icon + label.
pub fn button(ui: &mut Ui, t: &Theme, a: &mut Assets, icon: Option<&str>, label: &str, kind: Btn, enabled: bool) -> Response {
    let small = kind == Btn::Small;
    let (h, fs, pad, ic) = if small { (28.0, 12.0, 10.0, 14.0) } else { (34.0, 13.0, 14.0, 15.0) };
    let g = theme::line(ui, label, f(fs, 500.0), Color32::WHITE, 400.0);
    let w = pad * 2.0 + g.size().x + if icon.is_some() { ic + 8.0 } else { 0.0 };
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), if enabled { Sense::click() } else { Sense::hover() });
    let hov = anim(ui, resp.id, enabled && resp.hovered(), 0.12);
    let press = enabled && resp.is_pointer_button_down_on();
    let r2 = if press { r.shrink(0.5) } else { r };
    let op = if enabled { 1.0 } else { 0.5 };
    let (bg, border, fg) = match kind {
        Btn::Primary => {
            let b = if hov > 0.0 { lerp_color(t.accent, theme::mix(t.accent, 0.92, Color32::WHITE), hov) } else { t.accent };
            (b, b, Color32::WHITE)
        }
        _ => (
            lerp_color(t.panel2, t.hover, hov),
            lerp_color(t.line2, t.text3, hov),
            if kind == Btn::Danger { t.danger } else { t.text },
        ),
    };
    let p = ui.painter();
    p.rect_filled(r2, cr(9.0), theme::alpha(bg, op));
    p.rect_stroke(r2, cr(9.0), Stroke::new(1.0, theme::alpha(border, op)), StrokeKind::Inside);
    let mut x = r.left() + pad;
    if let Some(i) = icon {
        a.glyph(ui, i, Rect::from_min_size(pos2(x, r.center().y - ic / 2.0), vec2(ic, ic)), theme::alpha(fg, op));
        x += ic + 8.0;
    }
    let g = theme::line(ui, label, f(fs, 500.0), theme::alpha(fg, op), 400.0);
    theme::put(ui.painter(), &g, x, r.center().y);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// `.sw`: the on/off switch.
pub fn switch(ui: &mut Ui, t: &Theme, on: &mut bool) -> Response {
    let (r, mut resp) = ui.allocate_exact_size(vec2(42.0, 24.0), Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let k = ui.ctx().animate_bool_with_time(resp.id, *on, 0.18);
    let ke = ease(k);
    let p = ui.painter();
    let bg = lerp_color(t.panel2, t.accent, k);
    let border = lerp_color(t.line2, t.accent, k);
    p.rect_filled(r, cr(12.0), bg);
    p.rect_stroke(r, cr(12.0), Stroke::new(1.0, border), StrokeKind::Inside);
    let knob = pos2(r.left() + 4.0 + 8.0 + 18.0 * ke, r.center().y);
    p.circle_filled(knob, 8.0, lerp_color(t.text2, Color32::WHITE, k));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// `.seg`: a segmented control. Returns true when the value changed.
pub fn seg(ui: &mut Ui, t: &Theme, value: &mut String, opts: &[(&str, &str)]) -> bool {
    let font = f(12.5, 500.0);
    let widths: Vec<f32> = opts.iter().map(|(_, l)| theme::line(ui, l, font, Color32::WHITE, 300.0).size().x + 28.0).collect();
    let w = widths.iter().sum::<f32>() + 2.0 * (opts.len() as f32 - 1.0) + 6.0 + 2.0;
    let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
    let p = ui.painter().clone();
    p.rect_filled(r, cr(10.0), t.panel2);
    p.rect_stroke(r, cr(10.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
    let mut x = r.left() + 4.0;
    let mut changed = false;
    for (i, (v, l)) in opts.iter().enumerate() {
        let br = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(widths[i], 28.0));
        let resp = ui.interact(br, ui.id().with(("seg", *v)), Sense::click());
        let on = value == v;
        if on {
            shadow(&p, br, 7.0, Color32::from_black_alpha(50), 1.0, 3.0);
            p.rect_filled(br, cr(7.0), t.bg_solid);
        }
        let g = theme::line(ui, l, font, if on { t.text } else { t.text2 }, 300.0);
        theme::put(&p, &g, br.center().x - g.size().x / 2.0, br.center().y);
        if resp.clicked() && !on {
            *value = v.to_string();
            changed = true;
        }
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        x += widths[i] + 2.0;
    }
    changed
}

/// A square icon button (`.icon-btn`, `.x`); returns its response.
#[allow(clippy::too_many_arguments)]
pub fn icon_button(ui: &mut Ui, t: &Theme, a: &mut Assets, r: Rect, icon: &str, size: f32, radius: f32, idle: Color32, hover_fg: Color32) -> Response {
    let resp = ui.interact(r, ui.id().with(("ib", r.min.x as i32, r.min.y as i32, icon)), Sense::click());
    let h = anim(ui, resp.id, resp.hovered(), 0.12);
    if h > 0.0 {
        ui.painter().rect_filled(r, cr(radius), theme::alpha(t.hover, h));
    }
    a.glyph(ui, icon, Rect::from_center_size(r.center(), vec2(size, size)), lerp_color(idle, hover_fg, h));
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn pos(x: f32, y: f32) -> Pos2 {
    pos2(x, y)
}
