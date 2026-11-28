//! Notices in the bottom-right corner (timer done, reminder, script result): the old notice.html.
use super::assets::Assets;
use super::jobs::s;
use super::theme::{self, f, Theme};
use super::widgets::{self, anim, cr, ease, lerp_color};
use egui::{pos2, vec2, Color32, CornerRadius, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

pub const TITLE: &str = "Glint notice";
pub const W: f32 = 380.0;

pub fn height(n: usize) -> f32 {
    16.0 + n as f32 * 112.0
}

pub enum NoticeAction {
    Dismiss(u64),
    Snooze(u64, i64),
}

/// Draw the cards (newest at the bottom) and return what was clicked.
pub fn ui(ui: &mut Ui, t: &Theme, a: &mut Assets, list: &[Value], seen: &mut HashMap<u64, Instant>) -> Vec<NoticeAction> {
    let mut out = Vec::new();
    let full = ui.max_rect();
    let mut y = full.bottom() - 8.0;
    let now = Instant::now();
    seen.retain(|id, _| list.iter().any(|n| n["id"].as_u64() == Some(*id)));
    for n in list.iter().rev() {
        let id = n["id"].as_u64().unwrap_or(0);
        let since = seen.entry(id).or_insert(now).elapsed().as_secs_f32();
        let k = ease((since / 0.22).min(1.0));
        if k < 1.0 {
            ui.ctx().request_repaint();
        }
        let kind = s(n, "kind");
        let r = Rect::from_min_max(pos2(full.left() + 8.0 + 24.0 * (1.0 - k), y - 96.0), pos2(full.right() - 8.0 + 24.0 * (1.0 - k), y));
        y -= 96.0 + 8.0;
        let op = k;
        let p = ui.painter().clone();
        let mut pp = p.clone();
        pp.multiply_opacity(op);
        widgets::shadow(&pp, r, 12.0, Color32::from_black_alpha(90), 8.0, 28.0);
        pp.rect_filled(r, cr(12.0), t.bg_solid);
        let stripe = match kind {
            "err" => t.danger,
            "ok" => t.ok,
            _ => t.accent,
        };
        pp.rect_filled(Rect::from_min_max(r.min, pos2(r.left() + 3.0, r.bottom())), CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 }, stripe);
        pp.rect_stroke(r, cr(12.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
        // icon (the bell rings three times)
        let ic = Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 14.0), vec2(38.0, 38.0));
        pp.rect_filled(ic, cr(10.0), theme::alpha(stripe, 0.15));
        let glyph = match kind {
            "timer" => "bell",
            "err" => "warn",
            "ok" => "check",
            _ => "glint",
        };
        let angle = if kind == "timer" && since < 3.0 {
            ui.ctx().request_repaint();
            let ph = since.fract();
            let keys = [(0.0, 0.0), (0.2, -14.0), (0.4, 12.0), (0.6, -8.0), (0.8, 5.0), (1.0, 0.0)];
            let mut deg = 0.0;
            for w in keys.windows(2) {
                if ph >= w[0].0 && ph <= w[1].0 {
                    let tt = (ph - w[0].0) / (w[1].0 - w[0].0);
                    let tt = tt * tt * (3.0 - 2.0 * tt);
                    deg = w[0].1 + (w[1].1 - w[0].1) * tt;
                }
            }
            deg.to_radians()
        } else {
            0.0
        };
        a.glyph_rotated(ui, glyph, Rect::from_center_size(ic.center(), vec2(20.0, 20.0)), theme::alpha(stripe, op), angle);
        // text
        let x = ic.right() + 14.0;
        let tw = r.right() - 14.0 - x - 20.0;
        let g1 = theme::line(ui, s(n, "title"), f(14.0, 600.0), theme::alpha(t.text, op), tw);
        let mut j = theme::job(s(n, "body"), f(12.0, 400.0), theme::alpha(t.text2, op));
        j.wrap.max_width = tw;
        j.wrap.max_rows = 2;
        j.wrap.break_anywhere = false;
        j.wrap.overflow_character = Some('…');
        let g2 = ui.fonts_mut(|fo| fo.layout_job(j));
        let ty = r.top() + 12.0;
        pp.galley(pos2(x, ty), g1.clone(), Color32::WHITE);
        pp.galley(pos2(x, ty + g1.size().y + 3.0), g2.clone(), Color32::WHITE);
        // buttons
        let by = (ty + g1.size().y + 3.0 + g2.size().y + 6.0).min(r.bottom() - 12.0 - 26.0);
        let buttons: Vec<(&str, &str, bool)> = if kind == "timer" { vec![("ok", "Done", true), ("s5", "Snooze 5 min", false), ("s15", "15 min", false)] } else { vec![("ok", "OK", false)] };
        let mut bx = x;
        for (act, label, primary) in buttons {
            let g = theme::line(ui, label, f(12.0, 500.0), if primary { Color32::WHITE } else { t.text }, 200.0);
            let br = Rect::from_min_size(pos2(bx, by), vec2(g.size().x + 20.0, 26.0));
            let resp = ui.interact(br, Id::new(("nb", id, act)), Sense::click());
            let h = anim(ui, resp.id, resp.hovered(), 0.1);
            if primary {
                pp.rect_filled(br, cr(7.0), t.accent);
            } else {
                pp.rect_filled(br, cr(7.0), lerp_color(t.panel2, t.hover, h));
                pp.rect_stroke(br, cr(7.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
            }
            theme::put(&pp, &g, br.left() + 10.0, br.center().y);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                out.push(match act {
                    "s5" => NoticeAction::Snooze(id, 5),
                    "s15" => NoticeAction::Snooze(id, 15),
                    _ => NoticeAction::Dismiss(id),
                });
            }
            bx = br.right() + 6.0;
        }
        // the ×
        let xr = Rect::from_min_size(pos2(r.right() - 8.0 - 24.0, r.top() + 8.0), vec2(24.0, 24.0));
        let resp = ui.interact(xr, Id::new(("nx", id)), Sense::click());
        let h = anim(ui, resp.id, resp.hovered(), 0.1);
        a.glyph(ui, "x", Rect::from_center_size(xr.center(), vec2(13.0, 13.0)), theme::alpha(lerp_color(t.text3, t.text, h), op));
        if resp.on_hover_text("Dismiss").clicked() {
            out.push(NoticeAction::Dismiss(id));
        }
    }
    out
}
