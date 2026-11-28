//! The settings window: a port of the old settings.html/js/css.
use super::assets::Assets;
use super::fmt;
use super::jobs::{b, n, s, Jobs, Reply};
use super::theme::{self, f, fd, fm, Theme};
use super::widgets::{self, anim, cr, ease, lerp_color, Btn, Toast};
use crate::host;
use egui::{pos2, vec2, Color32, Id, Rect, Sense, Stroke, StrokeKind, Ui};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const PAGES: &[(&str, &str, &str, bool)] = &[
    ("welcome", "Welcome", "sparkle", true),
    ("general", "General", "gear", false),
    ("keyboard", "Keyboard", "keyboard", false),
    ("search", "Search", "search", false),
    ("content", "File contents", "filetext", false),
    ("extras", "Extras & pins", "bolt", false),
    ("appearance", "Appearance", "palette", false),
    ("privacy", "Privacy & data", "shield", false),
    ("about", "About & updates", "info", false),
];

const ACCENTS: &[&str] = &["#7c6cff", "#3b82f6", "#06b6d4", "#10b981", "#f59e0b", "#f97316", "#ef4444", "#ec4899"];

pub const TITLE: &str = "Glint Settings";

enum Tag {
    Settings,
    Status,
    Save(Option<String>),
    StartIndexer,
    Rebuild,
    Update,
    Install,
    Pins,
    PinsChanged(&'static str),
    PickFolder(&'static str),
    PickFile,
    Toast(&'static str),
    Ignore,
}

pub struct SettingsWin {
    jobs: Jobs<Tag>,
    pub page: String,
    s: Value,
    status: Option<Value>,
    toast: Option<Toast>,
    status_at: Instant,
    update: Option<Result<Value, String>>,
    checking: bool,
    installing: Option<String>,
    install_text: Option<String>,
    pins: Option<Value>,
    display_name: String,
    exclude_input: String,
    page_at: Instant,
    scroll_top: bool,
    max_files: f64,
    max_mb: f64,
    custom: [u8; 3],
    pub close: bool,
    loaded: bool,
}

impl SettingsWin {
    pub fn new(ctx: &egui::Context, page: &str) -> SettingsWin {
        let jobs = Jobs::default();
        jobs.call(ctx, Tag::Settings, "settings_get", json!({}));
        jobs.call(ctx, Tag::Status, "status", json!({}));
        let mut w = SettingsWin {
            jobs,
            page: if page.is_empty() { "general".into() } else { page.into() },
            s: json!({}),
            status: None,
            toast: None,
            status_at: Instant::now(),
            update: None,
            checking: false,
            installing: None,
            install_text: None,
            pins: None,
            display_name: String::new(),
            exclude_input: String::new(),
            page_at: Instant::now(),
            scroll_top: true,
            max_files: 8.0,
            max_mb: 30.0,
            custom: [0x7c, 0x6c, 0xff],
            close: false,
            loaded: false,
        };
        w.on_page(ctx);
        w
    }

    pub fn go(&mut self, ctx: &egui::Context, page: &str) {
        if !page.is_empty() {
            self.page = page.to_string();
        }
        self.page_at = Instant::now();
        self.scroll_top = true;
        self.on_page(ctx);
    }

    fn on_page(&mut self, ctx: &egui::Context) {
        match self.page.as_str() {
            "about" => {
                self.checking = true;
                self.jobs.call(ctx, Tag::Update, "update_check", json!({"force": false}));
            }
            "extras" => self.jobs.call(ctx, Tag::Pins, "pins_list", json!({})),
            _ => {}
        }
    }

    /// Settings were saved somewhere else (Ctrl+P in the launcher): re-read them.
    pub fn reload(&mut self, ctx: &egui::Context) {
        self.jobs.call(ctx, Tag::Settings, "settings_get", json!({}));
    }

    fn sv(&self, k: &str) -> String {
        s(&self.s, k).to_string()
    }

    fn sb(&self, k: &str) -> bool {
        b(&self.s, k)
    }

    fn save(&mut self, ctx: &egui::Context, patch: Value, msg: Option<&str>) {
        if let (Some(cur), Some(p)) = (self.s.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                cur.insert(k.clone(), v.clone());
            }
        }
        self.jobs.call(ctx, Tag::Save(msg.map(String::from)), "settings_set", patch);
    }

    fn toast(&mut self, msg: impl Into<String>, err: bool) {
        self.toast = Some(Toast::new(msg, err, 2.2));
    }

    fn poll(&mut self, ctx: &egui::Context) {
        for (tag, r) in self.jobs.poll() {
            self.on_reply(ctx, tag, r);
        }
        if self.status_at.elapsed() > Duration::from_millis(1500) {
            self.status_at = Instant::now();
            self.jobs.call(ctx, Tag::Status, "status", json!({}));
        }
        ctx.request_repaint_after(Duration::from_millis(1500).saturating_sub(self.status_at.elapsed()));
    }

    fn on_reply(&mut self, ctx: &egui::Context, tag: Tag, r: Reply) {
        match tag {
            Tag::Settings => {
                if let Ok(v) = r {
                    self.display_name = s(&v, "display_name").to_string();
                    self.max_files = n(&v, "max_files").unwrap_or(8.0);
                    self.max_mb = n(&v, "content_max_mb").unwrap_or(30.0);
                    if let Some(c) = theme::hex(s(&v, "accent")) {
                        self.custom = [c.r(), c.g(), c.b()];
                    }
                    self.s = v;
                    self.loaded = true;
                }
            }
            Tag::Status => {
                if let Ok(v) = r {
                    self.status = Some(v);
                }
            }
            Tag::Save(msg) => match r {
                Ok(_) => {
                    if let Some(m) = msg {
                        self.toast(m, false);
                    }
                }
                Err(e) => self.toast(e, true),
            },
            Tag::StartIndexer => match r {
                Ok(_) => {
                    self.toast("Starting the indexer…", false);
                    self.status_at = Instant::now() - Duration::from_millis(1500) + Duration::from_millis(3000);
                }
                Err(e) => {
                    if e != "Cancelled" {
                        self.toast(e, true)
                    }
                }
            },
            Tag::Rebuild => {
                self.toast("Reading your documents again", false);
                self.status_at = Instant::now() - Duration::from_millis(700);
            }
            Tag::Update => {
                self.checking = false;
                self.update = Some(r);
            }
            Tag::Install => match r {
                Ok(_) => {
                    self.installing = None;
                    self.install_text = Some("Installing — Glint will restart.".into());
                }
                Err(e) => {
                    self.installing = None;
                    self.toast(e, true);
                }
            },
            Tag::Pins => {
                if let Ok(v) = r {
                    self.pins = Some(v);
                }
            }
            Tag::PinsChanged(msg) => {
                self.toast(msg, false);
                self.jobs.call(ctx, Tag::Pins, "pins_list", json!({}));
            }
            Tag::PickFolder(key) => {
                if let Ok(Value::String(p)) = r {
                    let mut list: Vec<Value> = self.s[key].as_array().cloned().unwrap_or_default();
                    if !list.iter().any(|x| x.as_str() == Some(&p)) {
                        list.push(json!(p));
                        self.save(ctx, json!({ key: list }), Some("Saved"));
                    }
                }
            }
            Tag::PickFile => {
                if let Ok(Value::String(p)) = r {
                    self.save(ctx, json!({"filepilot_path": p, "win_e_filepilot": true}), Some("File Pilot set"));
                }
            }
            Tag::Toast(m) => {
                if r.is_ok() {
                    self.toast(m, false)
                }
            }
            Tag::Ignore => {}
        }
    }

    // ------------------------------------------------------------------ window

    pub fn ui(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !ctx.egui_wants_keyboard_input() {
            self.close = true;
        }
        let full = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(full, cr(0.0), t.bg_solid);
        self.title_bar(ui, t, a, Rect::from_min_size(full.min, vec2(full.width(), 42.0)));
        let side = Rect::from_min_max(pos2(full.left(), full.top() + 42.0), pos2(full.left() + 230.0, full.bottom()));
        self.sidebar(ui, t, a, side);
        let content = Rect::from_min_max(pos2(side.right(), side.top()), full.max);
        if self.loaded {
            self.content(ui, t, a, content);
        }
        if let Some(toast) = &self.toast {
            if !toast.draw(ui, t, a, full.center().x, full.bottom() - 24.0, t.panel2) {
                self.toast = None;
            }
        }
        self.resize_edges(ui, full);
    }

    fn resize_edges(&mut self, ui: &mut Ui, r: Rect) {
        use egui::viewport::ResizeDirection as D;
        let m = 5.0;
        let zones = [
            (Rect::from_min_max(r.left_top(), pos2(r.left() + m, r.bottom())), D::West, egui::CursorIcon::ResizeWest),
            (Rect::from_min_max(pos2(r.right() - m, r.top()), r.max), D::East, egui::CursorIcon::ResizeEast),
            (Rect::from_min_max(pos2(r.left(), r.bottom() - m), r.max), D::South, egui::CursorIcon::ResizeSouth),
            (Rect::from_min_max(r.min, pos2(r.right() - 100.0, r.top() + 3.0)), D::North, egui::CursorIcon::ResizeNorth),
            (Rect::from_min_max(pos2(r.right() - 12.0, r.bottom() - 12.0), r.max), D::SouthEast, egui::CursorIcon::ResizeSouthEast),
            (Rect::from_min_max(pos2(r.left(), r.bottom() - 12.0), pos2(r.left() + 12.0, r.bottom())), D::SouthWest, egui::CursorIcon::ResizeSouthWest),
        ];
        for (i, (zr, dir, cur)) in zones.into_iter().enumerate() {
            let resp = ui.interact(zr, Id::new(("resize", i)), Sense::drag());
            if resp.hovered() {
                ui.ctx().set_cursor_icon(cur);
            }
            if resp.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::BeginResize(dir));
            }
        }
    }

    fn title_bar(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, r: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let drag = ui.interact(Rect::from_min_max(r.min, pos2(r.right() - 92.0, r.bottom())), Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started() {
            ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let max = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
        }
        a.glyph(ui, "glint", Rect::from_min_size(pos2(r.left() + 16.0, r.center().y - 8.0), vec2(16.0, 16.0)), t.accent);
        let g = theme::line(ui, "Glint Settings", f(12.5, 600.0), t.text2, 300.0);
        theme::put(&p, &g, r.left() + 16.0 + 16.0 + 10.0, r.center().y);
        let close = Rect::from_min_max(pos2(r.right() - 46.0, r.top()), r.max);
        let min = Rect::from_min_max(pos2(close.left() - 46.0, r.top()), pos2(close.left(), r.bottom()));
        let rm = ui.interact(min, Id::new("tb-min"), Sense::click());
        let hm = anim(ui, rm.id, rm.hovered(), 0.1);
        if hm > 0.0 {
            p.rect_filled(min, cr(0.0), theme::alpha(t.hover, hm));
        }
        a.glyph(ui, "minus", Rect::from_center_size(min.center(), vec2(14.0, 14.0)), lerp_color(t.text2, t.text, hm));
        if rm.on_hover_text("Minimize").clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }
        let rc = ui.interact(close, Id::new("tb-close"), Sense::click());
        let hc = anim(ui, rc.id, rc.hovered(), 0.1);
        if hc > 0.0 {
            p.rect_filled(close, cr(0.0), theme::alpha(Color32::from_rgb(0xe8, 0x11, 0x23), hc));
        }
        a.glyph(ui, "x", Rect::from_center_size(close.center(), vec2(14.0, 14.0)), lerp_color(t.text2, Color32::WHITE, hc));
        if rc.on_hover_text("Close").clicked() {
            self.close = true;
        }
    }

    fn sidebar(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, r: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let mut y = r.top() + 8.0;
        let mut go = None;
        for (id, label, icon, hidden) in PAGES {
            if *hidden && self.page != *id {
                continue;
            }
            let nr = Rect::from_min_size(pos2(r.left() + 12.0, y), vec2(r.width() - 24.0, 38.0));
            let resp = ui.interact(nr, Id::new(("nav", *id)), Sense::click());
            let on = self.page == *id;
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            if on {
                p.rect_filled(nr, cr(9.0), t.panel2);
                p.rect_filled(Rect::from_min_max(pos2(nr.left(), nr.top() + 10.0), pos2(nr.left() + 3.0, nr.bottom() - 10.0)), cr(1.5), t.accent);
            } else if h > 0.0 {
                p.rect_filled(nr, cr(9.0), theme::alpha(t.hover, h));
            }
            a.glyph(ui, icon, Rect::from_min_size(pos2(nr.left() + 12.0, nr.center().y - 8.5), vec2(17.0, 17.0)), if on { t.accent } else { t.text3 });
            let g = theme::line(ui, label, f(13.5, 500.0), if on { t.text } else { lerp_color(t.text2, t.text, h) }, 200.0);
            theme::put(&p, &g, nr.left() + 12.0 + 17.0 + 12.0, nr.center().y);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                go = Some(*id);
            }
            y += 40.0;
        }
        let ver = self.status.as_ref().map(|st| s(st, "version").to_string()).unwrap_or_else(|| self.sv("_version"));
        let g = theme::line(ui, &format!("Glint {ver}"), f(11.5, 400.0), t.text3, 200.0);
        p.galley(pos2(r.left() + 24.0, r.bottom() - 16.0 - g.size().y), g, Color32::WHITE);
        if let Some(pg) = go {
            self.go(&ctx, pg);
        }
    }

    fn content(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, r: Rect) {
        let k = ease((self.page_at.elapsed().as_secs_f32() / 0.18).min(1.0));
        if k < 1.0 {
            ui.ctx().request_repaint();
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(r);
        let mut sa = egui::ScrollArea::vertical().id_salt("settings-content").auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_top) {
            sa = sa.vertical_scroll_offset(0.0);
        }
        sa.show(&mut child, |ui| {
            ui.set_opacity(k);
            ui.add_space(8.0 + 4.0 * (1.0 - k));
            let avail = ui.available_width();
            let w = (avail - 22.0 - 34.0).min(720.0);
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    let page = self.page.clone();
                    match page.as_str() {
                        "welcome" => self.page_welcome(ui, t, a, w),
                        "keyboard" => self.page_keyboard(ui, t, a, w),
                        "search" => self.page_search(ui, t, a, w),
                        "content" => self.page_content(ui, t, a, w),
                        "extras" => self.page_extras(ui, t, a, w),
                        "appearance" => self.page_appearance(ui, t, a, w),
                        "privacy" => self.page_privacy(ui, t, a, w),
                        "about" => self.page_about(ui, t, a, w),
                        _ => self.page_general(ui, t, a, w),
                    }
                    ui.add_space(40.0);
                });
            });
        });
    }

    // ------------------------------------------------------------------ building blocks

    fn h1(&self, ui: &mut Ui, t: &Theme, text: &str, lead: &str, w: f32) {
        ui.add_space(6.0);
        let g = theme::wrapped(ui, text, fd(26.0, 650.0), t.text, w);
        let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, Color32::WHITE);
        ui.add_space(4.0);
        let mut j = theme::rich_accent(lead, f(13.5, 400.0), t.text2, t);
        j.sections.iter_mut().for_each(|s| s.format.line_height = Some(21.0));
        let g = theme::layout(ui, j, w, false);
        let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, Color32::WHITE);
        ui.add_space(22.0);
    }

    fn h2(&self, ui: &mut Ui, t: &Theme, text: &str, w: f32) {
        ui.add_space(26.0);
        let mut j = theme::job(&text.to_uppercase(), f(12.0, 600.0), t.text3);
        j.sections[0].format.extra_letter_spacing = 0.72;
        let g = theme::layout(ui, j, w, true);
        let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, Color32::WHITE);
        ui.add_space(10.0);
    }

    fn note(&self, ui: &mut Ui, t: &Theme, text: &str, w: f32, top: f32) {
        ui.add_space(top);
        let mut j = theme::rich_accent(text, f(12.5, 400.0), t.text3, t);
        j.sections.iter_mut().for_each(|s| s.format.line_height = Some(19.4));
        let g = theme::layout(ui, j, w, false);
        let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
        ui.painter().galley(r.min, g, Color32::WHITE);
    }

    /// One `.item` row inside a card: icon, title + description, and the control on the right.
    #[allow(clippy::too_many_arguments)]
    fn item(ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32, first: bool, icon: &str, title: &str, desc: &str, ctl_w: f32, ctl: impl FnOnce(&mut Ui, Rect, &mut Assets)) {
        let lbl_x = 18.0 + 36.0 + 16.0;
        let lbl_w = (w - lbl_x - 16.0 - ctl_w - 18.0).max(80.0);
        let tg = theme::wrapped(ui, title, f(13.5, 550.0), t.text, lbl_w);
        let dg = (!desc.is_empty()).then(|| {
            let mut j = theme::rich(desc, f(12.5, 400.0), t.text3, t);
            j.sections.iter_mut().for_each(|s| s.format.line_height = Some(18.1));
            theme::layout(ui, j, lbl_w, false)
        });
        let text_h = tg.size().y + dg.as_ref().map(|g| 3.0 + g.size().y).unwrap_or(0.0);
        let h = (text_h + 24.0).max(62.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let p = ui.painter().clone();
        if !first {
            p.line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
        }
        let ic = Rect::from_min_size(pos2(r.left() + 18.0, r.center().y - 18.0), vec2(36.0, 36.0));
        p.rect_filled(ic, cr(10.0), t.panel2);
        a.glyph(ui, icon, Rect::from_center_size(ic.center(), vec2(18.0, 18.0)), t.text2);
        let mut y = r.center().y - text_h / 2.0;
        p.galley(pos2(r.left() + lbl_x, y), tg.clone(), Color32::WHITE);
        y += tg.size().y + 3.0;
        if let Some(dg) = dg {
            p.galley(pos2(r.left() + lbl_x, y), dg, Color32::WHITE);
        }
        let cr_ = Rect::from_min_max(pos2(r.right() - 18.0 - ctl_w, r.top()), pos2(r.right() - 18.0, r.bottom()));
        // the control lives in a detached child: it must not move this column's cursor, and its ids are per row
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(cr_).id_salt(("item", title)));
        ctl(&mut child, cr_, a);
    }

    fn card(ui: &mut Ui, t: &Theme, w: f32, body: impl FnOnce(&mut Ui)) {
        let top = ui.cursor().top();
        let x = ui.cursor().left();
        let idx = ui.painter().add(egui::Shape::Noop);
        ui.vertical(|ui| {
            ui.set_width(w);
            body(ui);
        });
        let bottom = ui.min_rect().bottom().max(top);
        let r = Rect::from_min_max(pos2(x, top), pos2(x + w, ui.cursor().top().max(bottom)));
        let r = Rect::from_min_max(r.min, pos2(r.right(), ui.cursor().top()));
        ui.painter().set(idx, egui::epaint::RectShape::new(r, cr(14.0), t.panel, Stroke::new(1.0, t.line), StrokeKind::Inside));
    }

    fn switch_ctl(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, key: &str) {
        let mut on = self.sb(key);
        let r = Rect::from_center_size(pos2(rect.right() - 21.0, rect.center().y), vec2(42.0, 24.0));
        let resp = ui.put(r, |ui: &mut Ui| widgets::switch(ui, t, &mut on));
        if resp.changed() {
            let ctx = ui.ctx().clone();
            self.save(&ctx, json!({ key: on }), Some("Saved"));
        }
    }

    fn seg_w(ui: &Ui, opts: &[(&str, &str)]) -> f32 {
        opts.iter().map(|(_, l)| theme::line(ui, l, f(12.5, 500.0), Color32::WHITE, 300.0).size().x + 28.0).sum::<f32>() + 2.0 * (opts.len() as f32 - 1.0) + 8.0
    }

    fn seg_ctl(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, key: &str, opts: &[(&str, &str)]) {
        let w = Self::seg_w(ui, opts);
        let r = Rect::from_center_size(pos2(rect.right() - w / 2.0, rect.center().y), vec2(w, 36.0));
        let mut v = self.sv(key);
        let mut changed = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            changed = widgets::seg(ui, t, &mut v, opts);
        });
        if changed {
            let ctx = ui.ctx().clone();
            self.save(&ctx, json!({ key: v }), None);
        }
    }

    fn select_ctl(&mut self, ui: &mut Ui, t: &Theme, rect: Rect, key: &str, opts: &[(&str, &str)]) {
        let cur = self.sv(key);
        let label = opts.iter().find(|o| o.0 == cur).map(|o| o.1).unwrap_or("");
        let r = Rect::from_min_size(pos2(rect.right() - 170.0, rect.center().y - 17.0), vec2(170.0, 34.0));
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            ui.spacing_mut().interact_size.y = 34.0;
            ui.spacing_mut().button_padding = vec2(12.0, 8.0);
            ui.style_mut().visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, t.line2);
            egui::ComboBox::from_id_salt(key).selected_text(theme::job(label, f(13.0, 400.0), t.text)).width(170.0).show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (v, l) in opts {
                    if ui.selectable_label(*v == cur, theme::job(l, f(13.0, 400.0), t.text)).clicked() {
                        pick = Some(v.to_string());
                    }
                }
            });
        });
        if let Some(v) = pick {
            let ctx = ui.ctx().clone();
            self.save(&ctx, json!({ key: v }), Some("Saved"));
        }
    }

    fn button_at(ui: &mut Ui, t: &Theme, a: &mut Assets, rect: Rect, icon: Option<&str>, label: &str, kind: Btn, enabled: bool) -> bool {
        let g = theme::line(ui, label, f(if kind == Btn::Small { 12.0 } else { 13.0 }, 500.0), Color32::WHITE, 400.0);
        let pad = if kind == Btn::Small { 10.0 } else { 14.0 };
        let ic = if icon.is_some() { (if kind == Btn::Small { 14.0 } else { 15.0 }) + 8.0 } else { 0.0 };
        let w = pad * 2.0 + g.size().x + ic;
        let h = if kind == Btn::Small { 28.0 } else { 34.0 };
        let r = Rect::from_min_size(pos2(rect.right() - w, rect.center().y - h / 2.0), vec2(w, h));
        let mut clicked = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            clicked = widgets::button(ui, t, a, icon, label, kind, enabled).clicked();
        });
        clicked
    }

    fn btn_w(ui: &Ui, label: &str, icon: bool, small: bool) -> f32 {
        let g = theme::line(ui, label, f(if small { 12.0 } else { 13.0 }, 500.0), Color32::WHITE, 400.0);
        (if small { 20.0 } else { 28.0 }) + g.size().x + (if icon { if small { 22.0 } else { 23.0 } } else { 0.0 })
    }

    /// A two-column table of tips: text on the left, keys or code on the right.
    fn keys_table(ui: &mut Ui, t: &Theme, w: f32, rows: &[(&str, &str)]) {
        Self::card(ui, t, w, |ui| {
            for (i, (l, rt)) in rows.iter().enumerate() {
                let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                if i > 0 {
                    ui.painter().line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
                }
                let right_w = inline_keys_width(ui, rt);
                let g = theme::line(ui, l, f(13.0, 400.0), t.text2, w - 36.0 - right_w - 12.0);
                theme::put(ui.painter(), &g, r.left() + 18.0, r.center().y);
                inline_keys(ui, t, r.right() - 18.0 - right_w, r.center().y, rt);
            }
        });
    }

    // ------------------------------------------------------------------ status cards

    fn stat(ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32, tone: &str, icon: &str, s1: &str, s2: &str, pills: &[String], progress: Option<Option<f32>>, button_w: f32, button: impl FnOnce(&mut Ui, Rect, &mut Assets)) {
        let x0 = 18.0 + 44.0 + 14.0;
        let tw = w - x0 - 18.0 - if button_w > 0.0 { button_w + 14.0 } else { 0.0 };
        let g1 = theme::wrapped(ui, s1, f(14.0, 600.0), t.text, tw);
        let mut j = theme::rich(s2, f(12.5, 400.0), t.text3, t);
        j.sections.iter_mut().for_each(|s| s.format.line_height = Some(18.1));
        let g2 = theme::layout(ui, j, tw, false);
        // pills wrap
        let mut pill_rows: Vec<Vec<(String, f32)>> = vec![];
        if !pills.is_empty() {
            let mut row = vec![];
            let mut x = 0.0;
            for pl in pills {
                let pw = theme::line(ui, pl, f(11.5, 400.0), Color32::WHITE, 400.0).size().x + 20.0;
                if x + pw > tw && !row.is_empty() {
                    pill_rows.push(std::mem::take(&mut row));
                    x = 0.0;
                }
                row.push((pl.clone(), pw));
                x += pw + 6.0;
            }
            pill_rows.push(row);
        }
        let pills_h = if pill_rows.is_empty() { 0.0 } else { 8.0 + pill_rows.len() as f32 * 23.0 + (pill_rows.len() as f32 - 1.0) * 6.0 };
        let prog_h = if progress.is_some() { 14.0 } else { 0.0 };
        let text_h = g1.size().y + 3.0 + g2.size().y + pills_h + prog_h;
        let h = (text_h + 32.0).max(44.0 + 32.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let p = ui.painter().clone();
        let col = match tone {
            "ok" => t.ok,
            "warn" => t.warn,
            _ => t.accent,
        };
        let ic = Rect::from_min_size(pos2(r.left() + 18.0, r.center().y - 22.0), vec2(44.0, 44.0));
        p.rect_filled(ic, cr(12.0), theme::alpha(col, 0.15));
        a.glyph(ui, icon, Rect::from_center_size(ic.center(), vec2(22.0, 22.0)), col);
        let mut y = r.center().y - text_h / 2.0;
        let x = r.left() + x0;
        p.galley(pos2(x, y), g1.clone(), Color32::WHITE);
        y += g1.size().y + 3.0;
        p.galley(pos2(x, y), g2.clone(), Color32::WHITE);
        y += g2.size().y;
        if !pill_rows.is_empty() {
            y += 8.0;
            for row in &pill_rows {
                let mut px = x;
                for (pl, pw) in row {
                    let pr = Rect::from_min_size(pos2(px, y), vec2(*pw, 23.0));
                    p.rect_filled(pr, cr(11.5), t.panel2);
                    p.rect_stroke(pr, cr(11.5), Stroke::new(1.0, t.line), StrokeKind::Inside);
                    let g = theme::line(ui, pl, f(11.5, 400.0), t.text2, 400.0);
                    theme::put(&p, &g, pr.left() + 10.0, pr.center().y);
                    px += pw + 6.0;
                }
                y += 29.0;
            }
            y -= 6.0;
        }
        if let Some(pg) = progress {
            y += 10.0;
            let bar = Rect::from_min_size(pos2(x, y), vec2(tw, 4.0));
            p.rect_filled(bar, cr(2.0), t.panel2);
            match pg {
                Some(frac) => {
                    p.rect_filled(Rect::from_min_size(bar.min, vec2(tw * frac.clamp(0.0, 1.0), 4.0)), cr(2.0), t.accent);
                }
                None => {
                    // the indeterminate bar slides across
                    let ph = (ui.input(|i| i.time) as f32 / 1.2).fract();
                    let k = if ph < 0.5 { 2.0 * ph * ph } else { 1.0 - (-2.0 * ph + 2.0).powi(2) / 2.0 };
                    let bw = tw * 0.3;
                    let bx = bar.left() - bw + (tw + bw * 3.4 - bw) * k;
                    let seg = Rect::from_min_max(pos2(bx.max(bar.left()), bar.top()), pos2((bx + bw).min(bar.right()), bar.bottom()));
                    if seg.width() > 0.0 {
                        p.rect_filled(seg, cr(2.0), t.accent);
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(16));
                }
            }
        }
        if button_w > 0.0 {
            button(ui, Rect::from_min_max(pos2(r.right() - 18.0 - button_w, r.top()), pos2(r.right() - 18.0, r.bottom())), a);
        }
    }

    fn indexer_card(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        let st = self.status.clone().unwrap_or(json!({}));
        let idx = &st["indexer"];
        let ctx = ui.ctx().clone();
        if let Some(vols) = idx["volumes"].as_array().filter(|v| !v.is_empty()) {
            let ready = vols.iter().all(|v| b(v, "ready"));
            let err = vols.iter().find_map(|v| v["error"].as_str()).map(|e| format!(" Problem: {e}")).unwrap_or_default();
            let pills: Vec<String> = vols
                .iter()
                .map(|v| {
                    let scan = n(v, "scan_ms").filter(|x| *x > 0.0).map(|ms| format!(" · scanned in {:.1}s", ms / 1000.0)).unwrap_or_default();
                    format!("{}: · {}{}", s(v, "letter"), fmt::count(n(v, "entries").unwrap_or(0.0)), scan)
                })
                .collect();
            let s1 = if ready { format!("{} files and folders indexed", fmt::count(n(idx, "total").unwrap_or(0.0))) } else { "Reading your drives…".into() };
            let s2 = format!("The indexer service reads each drive's file table directly and follows changes as they happen.{err}");
            Self::stat(ui, t, a, w, if ready { "ok" } else { "busy" }, if ready { "check" } else { "hdd" }, &s1, &s2, &pills, if ready { None } else { Some(None) }, 0.0, |_, _, _| {});
            return;
        }
        let can = b(&st, "indexer_installed");
        let s2 = format!("Without it, Glint only searches your user folder, and more slowly. {}", if can { "Starting it needs administrator permission once." } else { "It comes with the installer." });
        let bw = if can { Self::btn_w(ui, "Start indexer", true, false) } else { 0.0 };
        let mut start = false;
        Self::stat(ui, t, a, w, "warn", "warn", "The file indexer isn't running", &s2, &[], None, bw, |ui, r, a| {
            start = Self::button_at(ui, t, a, r, Some("bolt"), "Start indexer", Btn::Primary, true);
        });
        if start {
            self.jobs.call(&ctx, Tag::StartIndexer, "install_indexer", json!({}));
        }
    }

    fn content_card(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        let st = self.status.clone().unwrap_or(json!({}));
        let c = &st["content"];
        if c.is_null() {
            return;
        }
        let ctx = ui.ctx().clone();
        if !self.sb("content_search") {
            Self::stat(ui, t, a, w, "warn", "filetext", "File-content search is off", "Turn it on to find documents by the words inside them.", &[], None, 0.0, |_, _, _| {});
            return;
        }
        let phase = s(c, "phase").to_string();
        let busy = !phase.is_empty() && phase != "Up to date";
        let frac = (|| {
            let i = phase.rfind('(')?;
            let inner = phase[i + 1..].split(')').next()?;
            let (a, b2) = inner.split_once('/')?;
            Some(a.trim().parse::<f32>().ok()? / b2.trim().parse::<f32>().ok()?.max(1.0))
        })();
        let mut s2 = if phase.is_empty() { "Starting…".to_string() } else { phase.clone() };
        if let Some(e) = c["error"].as_str().filter(|e| !e.is_empty()) {
            s2.push_str(&format!(" · {e}"));
        }
        if let Some(lf) = n(c, "last_full").filter(|x| *x > 0.0) {
            s2.push_str(&format!(" · last full check {}", fmt::date(lf)));
        }
        let s1 = format!("{} documents searchable", fmt::count(n(c, "indexed").unwrap_or(0.0)));
        let bw = Self::btn_w(ui, "Rebuild", true, false);
        let mut rebuild = false;
        Self::stat(ui, t, a, w, if busy { "busy" } else { "ok" }, if busy { "refresh" } else { "check" }, &s1, &s2, &[], if busy { Some(frac) } else { None }, bw, |ui, r, a| {
            rebuild = Self::button_at(ui, t, a, r, Some("refresh"), "Rebuild", Btn::Normal, true);
        });
        if rebuild {
            self.jobs.call(&ctx, Tag::Rebuild, "rebuild", json!({}));
        }
    }

    // ------------------------------------------------------------------ pages

    fn page_welcome(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        let ctx = ui.ctx().clone();
        ui.add_space(26.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 84.0), Sense::hover());
        let logo = Rect::from_center_size(r.center(), vec2(84.0, 84.0));
        widgets::shadow(ui.painter(), logo, 22.0, theme::alpha(Color32::from_rgb(0x7c, 0x3a, 0xed), 0.45), 16.0, 40.0);
        diag(ui.painter(), logo, 22.0, Color32::from_rgb(0x4f, 0x46, 0xe5), Color32::from_rgb(0xa8, 0x55, 0xf7));
        a.glyph(ui, "glint", Rect::from_center_size(logo.center(), vec2(44.0, 44.0)), Color32::WHITE);
        ui.add_space(18.0 + 6.0);
        center_text(ui, "Glint is ready", fd(30.0, 650.0), t.text, w);
        ui.add_space(4.0);
        let mut j = theme::job(
            "Your new search lives on the Windows key. Apps, files on every drive, the text inside your documents, open windows, settings and a calculator — all in one box.",
            f(13.5, 400.0),
            t.text2,
        );
        j.halign = egui::Align::Center;
        j.sections.iter_mut().for_each(|s| s.format.line_height = Some(21.0));
        let g = theme::layout(ui, j, 520.0_f32.min(w), false);
        let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
        ui.painter().galley(pos2(r.center().x, r.top()), g, Color32::WHITE);
        ui.add_space(22.0 + 10.0 + 22.0);
        let fp = self.sb("_filepilot_found");
        let feats = [
            ("keyboard", "Press <kbd>⊞ Win</kbd>", "Tap the Windows key on its own to open Glint. Win+ shortcuts still work.".to_string()),
            (
                "folder",
                if fp { "<kbd>Win</kbd>+<kbd>E</kbd> → File Pilot" } else { "<kbd>Win</kbd>+<kbd>E</kbd> → Explorer" },
                if fp { "File Pilot opens instead of File Explorer.".into() } else { "Install File Pilot and turn this on in Keyboard.".into() },
            ),
            ("bolt", "Right arrow for more", "Show in folder, copy path, run as admin, terminal here, properties…".into()),
        ];
        let cw = (w - 20.0) / 3.0;
        let heights: Vec<f32> = feats
            .iter()
            .map(|(_, h, d)| {
                let g1 = theme::layout(ui, theme::rich(h, f(13.5, 600.0), t.text, t), cw - 32.0, false);
                let mut j = theme::job(d, f(12.5, 400.0), t.text3);
                j.sections.iter_mut().for_each(|s| s.format.line_height = Some(18.75));
                let g2 = theme::layout(ui, j, cw - 32.0, false);
                16.0 + 36.0 + 12.0 + g1.size().y + 4.0 + g2.size().y + 16.0
            })
            .collect();
        let fh = heights.iter().cloned().fold(0.0, f32::max);
        let (gr, _) = ui.allocate_exact_size(vec2(w, fh), Sense::hover());
        for (i, (icon, h, d)) in feats.iter().enumerate() {
            let r = Rect::from_min_size(pos2(gr.left() + i as f32 * (cw + 10.0), gr.top()), vec2(cw, fh));
            let p = ui.painter();
            p.rect_filled(r, cr(14.0), t.panel);
            p.rect_stroke(r, cr(14.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
            let fi = Rect::from_min_size(pos2(r.left() + 16.0, r.top() + 16.0), vec2(36.0, 36.0));
            p.rect_filled(fi, cr(10.0), theme::alpha(t.accent, 0.14));
            a.glyph(ui, icon, Rect::from_center_size(fi.center(), vec2(18.0, 18.0)), t.accent);
            let g1 = theme::layout(ui, theme::rich(h, f(13.5, 600.0), t.text, t), cw - 32.0, false);
            let mut j = theme::job(d, f(12.5, 400.0), t.text3);
            j.sections.iter_mut().for_each(|s| s.format.line_height = Some(18.75));
            let g2 = theme::layout(ui, j, cw - 32.0, false);
            let y = fi.bottom() + 12.0;
            ui.painter().galley(pos2(r.left() + 16.0, y), g1.clone(), Color32::WHITE);
            ui.painter().galley(pos2(r.left() + 16.0, y + g1.size().y + 4.0), g2, Color32::WHITE);
        }
        self.h2(ui, t, "File indexer", w);
        Self::card(ui, t, w, |ui| self.indexer_card(ui, t, a, w));
        ui.add_space(24.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
        let bw1 = Self::btn_w(ui, "Try it now", true, false);
        let bw2 = Self::btn_w(ui, "Look at settings", false, false);
        let x0 = r.center().x - (bw1 + 10.0 + bw2) / 2.0;
        if Self::button_at(ui, t, a, Rect::from_min_size(pos2(x0, r.top()), vec2(bw1, 34.0)), Some("search"), "Try it now", Btn::Primary, true) {
            self.close = true;
            host::show_launcher();
        }
        if Self::button_at(ui, t, a, Rect::from_min_size(pos2(x0 + bw1 + 10.0, r.top()), vec2(bw2, 34.0)), None, "Look at settings", Btn::Normal, true) {
            self.go(&ctx, "general");
        }
    }

    fn page_general(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "General", "How Glint starts and where it appears.", w);
        let ctx = ui.ctx().clone();
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            let sw = 42.0;
            Self::item(ui, t, a, w, true, "rocket", "Start with Windows", "Glint waits quietly in the background so it opens instantly.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "start_with_windows"));
            Self::item(ui, t, a, w, false, "layout", "Show tray icon", "Off runs Glint fully hidden. You can still reach settings with the gear in the search window, by typing “settings”, or from “Glint Settings” in the Start menu. Type “quit glint” to close it.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "show_tray"));
            let o = [("popup", "Pop-up"), ("start", "Start menu")];
            Self::item(ui, t, a, w, false, "grid", "Style", "Pop-up floats in the middle of the screen. Start menu docks Glint above the taskbar, bottom-left, like the classic Windows Start menu, with your pinned apps, All apps, folders and power buttons.", Self::seg_w(ui, &o), |ui, r, a| me.seg_ctl(ui, t, r, "layout", &o));
            let o = [("left", "Bottom left"), ("bottom", "Bottom center"), ("center", "Center")];
            Self::item(ui, t, a, w, false, "layout", "Start menu position", "Where the Start-menu style opens.", Self::seg_w(ui, &o), |ui, r, a| me.seg_ctl(ui, t, r, "start_position", &o));
            let user = me.sv("_user");
            Self::item(ui, t, a, w, false, "user", "Your name", "Shown at the top of the Start menu. Leave empty to use your Windows user name.", 160.0, |ui, r, a| {
                let ir = Rect::from_min_size(pos2(r.left(), r.center().y - 17.0), vec2(160.0, 34.0));
                let resp = text_input(ui, t, ir, &mut me.display_name, if user.is_empty() { "Name" } else { &user }, Id::new("display-name"), 40);
                if resp.lost_focus() && me.display_name.trim() != me.sv("display_name") {
                    let v = me.display_name.trim().to_string();
                    me.save(&ctx, json!({"display_name": v}), Some("Saved"));
                }
            });
            let o = [("cursor", "Screen with mouse"), ("primary", "Main screen")];
            Self::item(ui, t, a, w, false, "monitor", "Opens on", "With more than one screen: the one your mouse is on, or always your main screen.", Self::seg_w(ui, &o), |ui, r, a| me.seg_ctl(ui, t, r, "monitor", &o));
            let o = [("top", "Upper third"), ("center", "Center")];
            Self::item(ui, t, a, w, false, "layout", "Pop-up position", "Where the pop-up sits on the screen.", Self::seg_w(ui, &o), |ui, r, a| me.seg_ctl(ui, t, r, "position", &o));
            Self::item(ui, t, a, w, false, "layout", "Show the taskbar with Glint", "If your taskbar auto-hides, it comes up while Glint is open, like with the Windows Start menu. Your windows stay as they are; the taskbar just sits over them.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "show_taskbar"));
            Self::item(ui, t, a, w, false, "clock", "Show time and date", "In the search bar, always in view when Glint is open.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "show_clock"));
            Self::item(ui, t, a, w, false, "props", "Preview pane", "Details, thumbnails and text previews next to the results. <kbd>Ctrl</kbd>+<kbd>P</kbd> toggles it.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "show_preview"));
            Self::item(ui, t, a, w, false, "download", "Check for updates", "Once a day, quietly. You choose when to install.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "check_updates"));
            Self::item(ui, t, a, w, false, "cpu", "Sleep while hidden", "Uses less memory while you don’t see it, and wakes instantly.", sw, |ui, r, a| me.switch_ctl(ui, t, r, "suspend_hidden"));
        });
    }

    fn page_keyboard(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "Keyboard", "Shortcuts that open Glint, and what Win+E does.", w);
        let ctx = ui.ctx().clone();
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            Self::item(ui, t, a, w, true, "glint", "Windows key opens Glint", "Tap <kbd>⊞ Win</kbd> by itself. The Start menu is still on the taskbar button, <kbd>Ctrl</kbd>+<kbd>Esc</kbd>, or type “start menu”.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "win_key"));
            Self::item(ui, t, a, w, false, "keyboard", "Alt+Space opens Glint", "A second shortcut, like other launchers.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "alt_space"));
            let fp = me.sv("filepilot_path");
            let desc = if fp.is_empty() { "File Pilot wasn't found. Pick FPilot.exe to turn this on.".to_string() } else { format!("Uses <code>{fp}</code>") };
            Self::item(ui, t, a, w, false, "folder", "Win+E opens File Pilot", &desc, 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "win_e_filepilot"));
            let bw = Self::btn_w(ui, "Choose…", true, true);
            Self::item(ui, t, a, w, false, "external", "File Pilot location", "Where FPilot.exe is installed.", bw, |ui, r, a| {
                if Self::button_at(ui, t, a, r, Some("folder"), "Choose…", Btn::Small, true) {
                    me.jobs.call(&ctx, Tag::PickFile, "pick_file", json!({}));
                }
            });
            let o = [("filepilot", "File Pilot"), ("explorer", "File Explorer")];
            Self::item(ui, t, a, w, false, "folder", "Open folders with", "What opens when you pick a folder in Glint.", 170.0, |ui, r, a| me.select_ctl(ui, t, r, "folder_opener", &o));
        });
        self.note(ui, t, "Keys pressed while an app running as administrator is focused don't reach Glint (Windows blocks that), so there the Windows key opens Start as usual.", w, 10.0);
        self.h2(ui, t, "In the search window", w);
        Self::keys_table(
            ui,
            t,
            w,
            &[
                ("Open", "<kbd>↵</kbd>"),
                ("Show in folder / file location", "<kbd>Ctrl</kbd><kbd>↵</kbd>"),
                ("Run as administrator", "<kbd>Shift</kbd><kbd>↵</kbd>"),
                ("All actions for the selection", "<kbd>→</kbd> or <kbd>Ctrl</kbd><kbd>K</kbd>"),
                ("Copy path · copy the file itself", "<kbd>Ctrl</kbd><kbd>C</kbd> · <kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>C</kbd>"),
                ("Switch filter (All, Apps, Files…)", "<kbd>Tab</kbd>"),
                ("Open the 1st–9th result", "<kbd>Ctrl</kbd><kbd>1</kbd>…<kbd>9</kbd>"),
                ("Previous searches", "<kbd>↑</kbd> on an empty box"),
                ("Properties · Open with · Terminal here", "<kbd>Alt</kbd><kbd>↵</kbd> · <kbd>Ctrl</kbd><kbd>O</kbd> · <kbd>Ctrl</kbd><kbd>T</kbd>"),
                ("Move to Recycle Bin", "<kbd>Shift</kbd><kbd>Del</kbd> twice"),
                ("Toggle preview", "<kbd>Ctrl</kbd><kbd>P</kbd>"),
                ("Quick Look (big preview, ←→ to step)", "<kbd>Shift</kbd><kbd>Space</kbd>"),
                ("Pin / unpin the selection", "<kbd>Ctrl</kbd><kbd>Shift</kbd><kbd>P</kbd>"),
                ("Clipboard clip: copy without pasting", "<kbd>Ctrl</kbd><kbd>↵</kbd>"),
            ],
        );
    }

    fn page_search(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "Search", "What shows up, and how much of it.", w);
        let ctx = ui.ctx().clone();
        Self::card(ui, t, w, |ui| self.indexer_card(ui, t, a, w));
        self.h2(ui, t, "Results", w);
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            Self::item(ui, t, a, w, true, "file", "Files per search", "How many file and folder matches to show in the All view.", 200.0, |ui, r, a| {
                if let Some(v) = range(ui, t, r, &mut me.max_files, 3.0..=20.0, 1.0, "") {
                    me.save(&ctx, json!({"max_files": v as i64}), Some("Saved"));
                }
            });
            Self::item(ui, t, a, w, false, "globe", "Offer a web search", "Adds a “Search the web” row at the bottom. Never shows web results by itself.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "web_fallback"));
            let o = [("google", "Google"), ("duckduckgo", "DuckDuckGo"), ("bing", "Bing"), ("brave", "Brave"), ("kagi", "Kagi"), ("perplexity", "Perplexity")];
            Self::item(ui, t, a, w, false, "search", "Search engine", "Used by the web row and by <code>?words</code>.", 170.0, |ui, r, a| me.select_ctl(ui, t, r, "web_engine", &o));
        });
        self.h2(ui, t, "Tricks", w);
        Self::keys_table(
            ui,
            t,
            w,
            &[
                ("Only PDFs · several types", "<code>*.pdf</code> · <code>ext:docx,xlsx</code>"),
                ("Only folders · only files", "<code>folder:</code> · <code>file:</code>"),
                ("Words in the folder path count too", "<code>projects glint main</code>"),
                ("Exact phrase", "<code>\"annual report\"</code>"),
                ("Browse a folder (Tab completes)", "<code>C:\\</code> · <code>~\\Downloads</code> · <code>%appdata%</code>"),
                ("Calculator and units", "<code>2^10/4</code> · <code>10 km to mi</code> · <code>255 to hex</code>"),
                ("Run in PowerShell", "<code>&gt;ipconfig /all</code>"),
                ("End a process", "<code>kill chrome</code>"),
                ("Web search", "<code>?how to …</code>"),
                ("Changed recently", "<code>dm:today</code> · <code>dm:3d</code> · <code>dm:2025-06</code> · <code>dm:&gt;2025-01-01</code>"),
                ("By size", "<code>size:&gt;100mb</code> · <code>size:1gb..4gb</code> · <code>size:huge</code> · <code>size:empty</code>"),
                ("Newest big videos", "<code>*.mp4 size:&gt;500mb dm:month</code>"),
                ("Volume, mute, radios", "<code>vol 40</code> · <code>mute</code> · <code>wifi off</code> · <code>bt on</code>"),
                ("System info (Enter copies)", "<code>ip</code> · <code>battery</code> · <code>uptime</code> · <code>ram</code> · <code>disk</code>"),
                ("Move the window you came from", "<code>left</code> · <code>right</code> · <code>max</code> · <code>center</code> · <code>third</code> · <code>next monitor</code>"),
                ("Media", "<code>play</code> · <code>pause</code> · <code>next</code> · <code>prev</code>"),
                ("Clipboard history (ClipVault)", "<code>clip invoice</code> · <code>cb </code>"),
                ("Timers and reminders", "<code>timer 10m tea</code> · <code>remind 3pm call</code> · <code>remind in 20m stretch</code> · <code>timers</code>"),
                ("Per-app volume, output device", "<code>vol spotify 30</code> · <code>mute discord</code> · <code>audio headphones</code>"),
                ("What takes the space (Tab looks inside)", "<code>biggest</code> · <code>biggest ~\\Downloads</code>"),
                ("Latest download · screenshot", "<code>dl</code> · <code>ss</code> (Enter copies it) · <code>downloads</code>"),
                ("Browser", "<code>tab youtube</code> · <code>bm recipes</code>"),
            ],
        );
    }

    fn page_content(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "File contents", "Find documents by the words inside them: text and code files, Word, Excel, PowerPoint, OpenDocument, EPUB and PDF.", w);
        let ctx = ui.ctx().clone();
        Self::card(ui, t, w, |ui| self.content_card(ui, t, a, w));
        self.h2(ui, t, "Options", w);
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            Self::item(ui, t, a, w, true, "filetext", "Search inside files", "Reads documents in the background at low priority.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "content_search"));
            Self::item(ui, t, a, w, false, "hdd", "Largest file to read", "Bigger files are still found by name.", 220.0, |ui, r, a| {
                if let Some(v) = range(ui, t, r, &mut me.max_mb, 5.0..=200.0, 5.0, " MB") {
                    me.save(&ctx, json!({"content_max_mb": v as i64}), Some("Saved"));
                }
            });
        });
        self.h2(ui, t, "Folders to read", w);
        self.list_editor(ui, t, a, w, "content_roots", "folder", true, "No folders — nothing will be read.", "");
        self.h2(ui, t, "Leave out", w);
        self.list_editor(ui, t, a, w, "content_excludes", "x", false, "Nothing extra is left out.", "A folder name or part of a path, e.g. Archive or \\Old Projects\\");
        self.note(ui, t, "Always skipped: AppData, hidden folders, build and package folders (node_modules, target, .git…), online-only OneDrive files (reading them would download them), and files that usually hold secrets (.env, keys, password databases).", w, 10.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn list_editor(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32, key: &'static str, icon: &str, pick: bool, empty: &str, placeholder: &str) {
        let ctx = ui.ctx().clone();
        let values: Vec<String> = self.s[key].as_array().map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let mut remove = None;
        let mut add: Option<String> = None;
        let mut pick_folder = false;
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            if values.is_empty() {
                li(ui, t, a, w, true, None, empty, true, || false);
            }
            for (i, v) in values.iter().enumerate() {
                if li(ui, t, a, w, i == 0, Some(icon), v, false, || true) {
                    remove = Some(i);
                }
            }
            // the add row
            let (r, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
            ui.painter().line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
            if pick {
                let bw = Self::btn_w(ui, "Add folder…", true, true);
                if Self::button_at(ui, t, a, Rect::from_min_size(pos2(r.left() + 18.0, r.top()), vec2(bw, r.height())), Some("folder"), "Add folder…", Btn::Small, true) {
                    pick_folder = true;
                }
            } else {
                let bw = Self::btn_w(ui, "Add", false, true);
                let ir = Rect::from_min_max(pos2(r.left() + 18.0, r.center().y - 17.0), pos2(r.right() - 18.0 - bw - 8.0, r.center().y + 17.0));
                let resp = text_input(ui, t, ir, &mut me.exclude_input, placeholder, Id::new(("list-inp", key)), 300);
                let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if Self::button_at(ui, t, a, Rect::from_min_max(pos2(ir.right() + 8.0, r.top()), pos2(r.right() - 18.0, r.bottom())), None, "Add", Btn::Small, true) || enter {
                    let v = me.exclude_input.trim().to_string();
                    if !v.is_empty() {
                        add = Some(v);
                    }
                }
            }
        });
        if let Some(i) = remove {
            let next: Vec<&String> = values.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, v)| v).collect();
            self.save(&ctx, json!({ key: next }), Some("Saved"));
        }
        if let Some(v) = add {
            if !values.contains(&v) {
                let mut next = values.clone();
                next.push(v);
                self.exclude_input.clear();
                self.save(&ctx, json!({ key: next }), Some("Saved"));
            }
        }
        if pick_folder {
            self.jobs.call(&ctx, Tag::PickFolder(key), "pick_folder", json!({}));
        }
    }

    fn page_extras(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "Extras & pins", "Media controls, clipboard history, and the things you've pinned or given a keyword.", w);
        let ctx = ui.ctx().clone();
        let clipvault = self.status.as_ref().map(|st| b(st, "clipvault")).unwrap_or(false);
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            Self::item(ui, t, a, w, true, "play", "Media controls", "A “Now playing” card when Glint opens, plus play, pause, next and previous. Works with Spotify, browsers and anything that shows in the Windows media flyout.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "media_controls"));
            let d = if clipvault { "Search what you copied with <code>clip …</code>, the Clipboard tab, or the top matches in All." } else { "ClipVault wasn't found on this PC. Install it to search your clipboard history here." };
            Self::item(ui, t, a, w, false, "clipboard", "ClipVault history", d, 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "clipvault"));
            Self::item(ui, t, a, w, false, "globe", "Browser tabs and bookmarks", "Find open tabs (Enter switches to it) and bookmarks from Chrome, Edge and Brave. <code>tab …</code> and <code>bm …</code> search only those.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "browser_search"));
            Self::item(ui, t, a, w, false, "history", "Browser history", "Also pages you visited often. Read on this PC only.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "browser_history"));
            Self::item(ui, t, a, w, false, "enter", "Enter pastes the clip", "Puts it on the clipboard and pastes it into the window you were in. Off just copies it. <kbd>Ctrl</kbd>+<kbd>↵</kbd> always only copies.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "clip_paste"));
        });
        let pins = self.pins.clone().unwrap_or(json!({"pinned": [], "keywords": [], "hotkeys": []}));
        self.h2(ui, t, "Pinned", w);
        self.note(ui, t, "Pinned items show first when Glint opens and rise to the top of results. Pin anything with <kbd>→</kbd> › Pin, or <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd>.", w, -4.0);
        ui.add_space(10.0);
        let mut unpin = None;
        Self::card(ui, t, w, |ui| {
            let list = pins["pinned"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                li(ui, t, a, w, true, None, "Nothing pinned yet.", true, || false);
            }
            for (i, x) in list.iter().enumerate() {
                let icon = match s(x, "kind") {
                    "app" => "app",
                    "file" => "file",
                    "folder" => "folder",
                    "action" => "bolt",
                    "url" => "globe",
                    "layout" => "layout",
                    "toggle" => "sliders",
                    "command" => "terminal",
                    _ => "pin",
                };
                if li(ui, t, a, w, i == 0, Some(icon), s(x, "title"), false, || true) {
                    unpin = Some(s(x, "key").to_string());
                }
            }
        });
        if let Some(k) = unpin {
            self.jobs.call(&ctx, Tag::PinsChanged("Unpinned"), "unpin", json!({"key": k}));
        }
        self.h2(ui, t, "Global shortcuts", w);
        self.note(ui, t, "Open anything with a key combination, from anywhere. Add one with <kbd>→</kbd> › Set a global shortcut….", w, -4.0);
        ui.add_space(10.0);
        let mut rm_hk = None;
        Self::card(ui, t, w, |ui| {
            let list = pins["hotkeys"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                li(ui, t, a, w, true, None, "No shortcuts yet.", true, || false);
            }
            for (i, x) in list.iter().enumerate() {
                if li_code(ui, t, a, w, i == 0, s(x, "combo"), s(x, "title")) {
                    rm_hk = Some(s(x, "combo").to_string());
                }
            }
        });
        if let Some(c) = rm_hk {
            self.jobs.call(&ctx, Tag::PinsChanged("Shortcut removed"), "hotkey_remove", json!({"combo": c}));
        }
        self.h2(ui, t, "Your scripts", w);
        let mut open = false;
        Self::card(ui, t, w, |ui| {
            let bw = Self::btn_w(ui, "Open folder", true, false);
            Self::item(ui, t, a, w, true, "terminal", "Scripts folder", "Put .ps1, .bat, .cmd, .py or .js files here and they show up in Glint by name. Words after the name are passed as arguments. Enter runs it hidden and shows the result; Shift+Enter runs it in a terminal.", bw, |ui, r, a| {
                open = Self::button_at(ui, t, a, r, Some("folder"), "Open folder", Btn::Normal, true);
            });
        });
        if open {
            self.jobs.call(&ctx, Tag::Ignore, "open_scripts", json!({}));
        }
        self.h2(ui, t, "Keywords", w);
        self.note(ui, t, "Type a keyword and its item comes first. Add one with <kbd>→</kbd> › Add keyword….", w, -4.0);
        ui.add_space(10.0);
        let mut rm_kw = None;
        Self::card(ui, t, w, |ui| {
            let list = pins["keywords"].as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                li(ui, t, a, w, true, None, "No keywords yet.", true, || false);
            }
            for (i, x) in list.iter().enumerate() {
                if li_code(ui, t, a, w, i == 0, s(x, "keyword"), s(x, "title")) {
                    rm_kw = Some(s(x, "keyword").to_string());
                }
            }
        });
        if let Some(k) = rm_kw {
            self.jobs.call(&ctx, Tag::PinsChanged("Keyword removed"), "keyword_remove", json!({"word": k}));
        }
    }

    fn page_appearance(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "Appearance", "Make it yours.", w);
        let ctx = ui.ctx().clone();
        Self::card(ui, t, w, |ui| {
            let me = &mut *self;
            let o = [("system", "System"), ("dark", "Dark"), ("light", "Light")];
            Self::item(ui, t, a, w, true, "contrast", "Theme", "", Self::seg_w(ui, &o), |ui, r, a| me.seg_ctl(ui, t, r, "theme", &o));
            let sw_w = ACCENTS.len() as f32 * 34.0 + 26.0;
            Self::item(ui, t, a, w, false, "palette", "Accent color", "", sw_w, |ui, r, a| {
                let cur = me.sv("accent").to_lowercase();
                let mut x = r.left();
                for c in ACCENTS {
                    let col = theme::hex(c).unwrap();
                    let sr = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(26.0, 26.0));
                    let resp = ui.interact(sr, Id::new(("sw", *c)), Sense::click());
                    let on = cur == c.to_lowercase();
                    let p = ui.painter();
                    p.circle_filled(sr.center(), 13.0, col);
                    if on {
                        p.circle_stroke(sr.center(), 12.0, Stroke::new(2.0, t.text));
                        p.circle_stroke(sr.center(), 10.0, Stroke::new(2.0, t.bg_solid));
                    } else {
                        p.circle_stroke(sr.center(), 12.5, Stroke::new(1.0, Color32::from_white_alpha(38)));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        me.save(&ctx, json!({"accent": c}), None);
                    }
                    x += 34.0;
                }
                // a custom colour
                let cr_ = Rect::from_min_size(pos2(x, r.center().y - 13.0), vec2(26.0, 26.0));
                let mut col = me.custom;
                let mut changed = false;
                ui.scope_builder(egui::UiBuilder::new().max_rect(cr_), |ui| {
                    ui.spacing_mut().interact_size = vec2(26.0, 26.0);
                    changed = egui::widgets::color_picker::color_edit_button_srgb(ui, &mut col).on_hover_text("Custom").changed();
                });
                if changed {
                    me.custom = col;
                    me.save(&ctx, json!({"accent": theme::to_hex(Color32::from_rgb(col[0], col[1], col[2]))}), None);
                }
            });
            Self::item(ui, t, a, w, false, "sparkle", "Translucent window", "Blurs what's behind the search window. Turn off for a solid background.", 42.0, |ui, r, a| me.switch_ctl(ui, t, r, "blur"));
        });
    }

    fn page_privacy(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "Privacy & data", "Everything stays on this PC. Glint never sends your searches or files anywhere. It only goes online for the update check (GitHub), and for “Public IP” when you press Enter on it (api.ipify.org).", w);
        let ctx = ui.ctx().clone();
        let dir = self.status.as_ref().map(|st| s(st, "data_dir").to_string()).unwrap_or_default();
        let (mut clear, mut open) = (false, false);
        Self::card(ui, t, w, |ui| {
            let bw = Self::btn_w(ui, "Clear", true, false);
            Self::item(ui, t, a, w, true, "history", "Search history", "Glint learns what you open to rank it higher next time, and remembers recent searches for the Up arrow.", bw, |ui, r, a| {
                clear = Self::button_at(ui, t, a, r, Some("eraser"), "Clear", Btn::Danger, true);
            });
            let bw = Self::btn_w(ui, "Open", true, false);
            Self::item(ui, t, a, w, false, "folder", "Data folder", &format!("<code>{dir}</code>"), bw, |ui, r, a| {
                open = Self::button_at(ui, t, a, r, Some("external"), "Open", Btn::Normal, true);
            });
        });
        if clear {
            self.jobs.call(&ctx, Tag::Toast("History cleared"), "clear_history", json!({}));
        }
        if open {
            self.jobs.call(&ctx, Tag::Ignore, "open_path", json!({"path": dir}));
        }
    }

    fn page_about(&mut self, ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32) {
        self.h1(ui, t, "About Glint", "A fast launcher and search for Windows, written in Rust.", w);
        let ctx = ui.ctx().clone();
        let ver = self.sv("_version");
        let (text, btn): (String, Option<(&str, Option<&str>, Btn, String)>) = if let Some(it) = &self.install_text {
            (it.clone(), None)
        } else if self.checking {
            ("Checking for updates…".into(), None)
        } else {
            match &self.update {
                None => ("Checking for updates…".into(), None),
                Some(Err(e)) => (e.clone(), Some(("Check again", None, Btn::Normal, "recheck".into()))),
                Some(Ok(u)) => {
                    if let Some(e) = u["error"].as_str() {
                        (e.to_string(), Some(("Check again", None, Btn::Normal, "recheck".into())))
                    } else if b(u, "available") {
                        let latest = s(u, "latest").to_string();
                        if self.installing.is_some() {
                            (format!("Version {latest} is available."), Some(("Downloading…", None, Btn::Primary, "busy".into())))
                        } else if b(u, "can_install") {
                            (format!("Version {latest} is available."), Some(("", Some("download"), Btn::Primary, format!("install:{latest}"))))
                        } else {
                            (format!("Version {latest} is available."), Some(("Download", None, Btn::Normal, format!("url:{}", s(u, "page")))))
                        }
                    } else {
                        ("You have the newest version.".into(), Some(("Check again", None, Btn::Normal, "recheck".into())))
                    }
                }
            }
        };
        let mut action: Option<String> = None;
        Self::card(ui, t, w, |ui| {
            let label = match &btn {
                Some((l, _, _, act)) if act.starts_with("install:") => {
                    let _ = l;
                    format!("Install {}", &act[8..])
                }
                Some((l, ..)) => l.to_string(),
                None => String::new(),
            };
            let bw = btn.as_ref().map(|b2| Self::btn_w(ui, &label, b2.1.is_some(), false)).unwrap_or(0.0);
            Self::stat(ui, t, a, w, "busy", "glint", &format!("Glint {ver}"), &text, &[], None, bw, |ui, r, a| {
                if let Some((_, icon, kind, act)) = &btn {
                    if Self::button_at(ui, t, a, r, *icon, &label, *kind, act != "busy") {
                        action = Some(act.clone());
                    }
                }
            });
        });
        ui.add_space(10.0);
        Self::card(ui, t, w, |ui| {
            let bw = Self::btn_w(ui, "Open", true, false);
            Self::item(ui, t, a, w, true, "code", "Source code", "github.com/JustSad42/glint", bw, |ui, r, a| {
                if Self::button_at(ui, t, a, r, Some("external"), "Open", Btn::Normal, true) {
                    action = Some("url:https://github.com/JustSad42/glint".into());
                }
            });
            Self::item(ui, t, a, w, false, "warn", "Report a problem", "", bw, |ui, r, a| {
                if Self::button_at(ui, t, a, r, Some("external"), "Open", Btn::Normal, true) {
                    action = Some("url:https://github.com/JustSad42/glint/issues".into());
                }
            });
        });
        if let Some(act) = action {
            if act == "recheck" {
                self.checking = true;
                self.jobs.call(&ctx, Tag::Update, "update_check", json!({"force": true}));
            } else if let Some(u) = act.strip_prefix("url:") {
                self.jobs.call(&ctx, Tag::Ignore, "open_url", json!({"url": u}));
            } else if act.starts_with("install:") {
                self.installing = Some(act[8..].to_string());
                self.jobs.call(&ctx, Tag::Install, "update_install", json!({}));
            }
        }
    }
}

// ---------------------------------------------------------------------- helpers

fn diag(p: &egui::Painter, r: Rect, radius: f32, a: Color32, b2: Color32) {
    let shape = egui::epaint::RectShape::filled(r, cr(radius), Color32::WHITE);
    let mut mesh = egui::Mesh::default();
    let mut tess = egui::epaint::Tessellator::new(1.0, Default::default(), [1, 1], vec![]);
    tess.tessellate_rect(&shape, &mut mesh);
    for v in &mut mesh.vertices {
        let k = (((v.pos.x - r.left()) * 0.77 + (v.pos.y - r.top()) * 0.64) / (r.width() * 0.77 + r.height() * 0.64)).clamp(0.0, 1.0);
        let alpha = v.color.a() as f32 / 255.0;
        v.color = a.lerp_to_gamma(b2, k).gamma_multiply(alpha);
    }
    p.add(mesh);
}

fn center_text(ui: &mut Ui, text: &str, font: theme::Font, color: Color32, w: f32) {
    let mut j = theme::job(text, font, color);
    j.halign = egui::Align::Center;
    let g = theme::layout(ui, j, w, false);
    let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
    ui.painter().galley(pos2(r.center().x, r.top()), g, Color32::WHITE);
}

/// `.inp`: a 34px text box.
fn text_input(ui: &mut Ui, t: &Theme, r: Rect, buf: &mut String, placeholder: &str, id: Id, limit: usize) -> egui::Response {
    let focused = ui.memory(|m| m.has_focus(id));
    let p = ui.painter().clone();
    p.rect_filled(r, cr(9.0), t.panel2);
    p.rect_stroke(r, cr(9.0), Stroke::new(1.0, if focused { t.accent } else { t.line2 }), StrokeKind::Inside);
    let font = f(13.0, 400.0);
    let tc = t.text;
    let mut layouter = move |ui: &Ui, b2: &dyn egui::TextBuffer, _w: f32| ui.fonts_mut(|f| f.layout_job(theme::job(b2.as_str(), font, tc)));
    let inner = r.shrink2(vec2(12.0, 0.0));
    ui.put(
        inner,
        egui::TextEdit::singleline(buf)
            .id(id)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .char_limit(limit)
            .hint_text(theme::job(placeholder, font, t.text3))
            .layouter(&mut layouter)
            .desired_width(inner.width()),
    )
}

/// `input[type=range]` plus its value.
fn range(ui: &mut Ui, t: &Theme, r: Rect, v: &mut f64, rng: std::ops::RangeInclusive<f64>, step: f64, suffix: &str) -> Option<f64> {
    let val = format!("{}{}", *v as i64, suffix);
    let g = theme::line(ui, &val, f(12.5, 400.0), t.text2, 100.0);
    let vw = g.size().x.max(24.0);
    theme::put_right(ui.painter(), &g, r.right(), r.center().y);
    let sr = Rect::from_min_size(pos2(r.right() - vw - 8.0 - 160.0, r.center().y - 10.0), vec2(160.0, 20.0));
    let mut out = None;
    ui.scope_builder(egui::UiBuilder::new().max_rect(sr), |ui| {
        ui.spacing_mut().slider_width = 160.0;
        ui.spacing_mut().slider_rail_height = 4.0;
        ui.style_mut().visuals.selection.bg_fill = t.accent;
        ui.style_mut().visuals.widgets.inactive.bg_fill = t.accent;
        ui.style_mut().visuals.widgets.hovered.bg_fill = t.accent;
        ui.style_mut().visuals.widgets.active.bg_fill = t.accent;
        ui.style_mut().visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, t.accent);
        let resp = ui.add(egui::Slider::new(v, rng).step_by(step).show_value(false).trailing_fill(true));
        if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
            out = Some(*v);
        }
    });
    out
}

/// `.li`: one row of a list card, with a remove button. Returns true when removed.
#[allow(clippy::too_many_arguments)]
fn li(ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32, first: bool, icon: Option<&str>, text: &str, muted: bool, removable: impl FnOnce() -> bool) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
    if !first {
        ui.painter().line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
    }
    let mut x = r.left() + 18.0;
    if let Some(i) = icon {
        a.glyph(ui, i, Rect::from_min_size(pos2(x, r.center().y - 8.0), vec2(16.0, 16.0)), t.text3);
        x += 16.0 + 12.0;
    }
    let can_remove = removable();
    let g = theme::line(ui, text, f(13.0, 400.0), if muted { t.text3 } else { t.text }, r.right() - 18.0 - x - if can_remove { 40.0 } else { 0.0 });
    theme::put(ui.painter(), &g, x, r.center().y);
    if can_remove {
        let br = Rect::from_center_size(pos2(r.right() - 18.0 - 14.0, r.center().y), vec2(28.0, 28.0));
        let resp = widgets::icon_button(ui, t, a, br, "x", 14.0, 7.0, t.text3, t.danger);
        return resp.on_hover_text("Remove").clicked();
    }
    false
}

/// A list row that starts with a `code.kw` chip (keywords, shortcuts).
fn li_code(ui: &mut Ui, t: &Theme, a: &mut Assets, w: f32, first: bool, code: &str, text: &str) -> bool {
    let (r, _) = ui.allocate_exact_size(vec2(w, 42.0), Sense::hover());
    if !first {
        ui.painter().line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
    }
    let g = theme::line(ui, code, fm(12.0), t.accent, 300.0);
    let cr_ = Rect::from_min_size(pos2(r.left() + 18.0, r.center().y - 11.0), vec2(g.size().x + 16.0, 22.0));
    ui.painter().rect_filled(cr_, cr(6.0), theme::alpha(t.accent, 0.14));
    theme::put(ui.painter(), &g, cr_.left() + 8.0, r.center().y);
    let x = cr_.right() + 12.0;
    let g = theme::line(ui, text, f(13.0, 400.0), t.text, r.right() - 18.0 - 40.0 - x);
    theme::put(ui.painter(), &g, x, r.center().y);
    let br = Rect::from_center_size(pos2(r.right() - 18.0 - 14.0, r.center().y), vec2(28.0, 28.0));
    widgets::icon_button(ui, t, a, br, "x", 14.0, 7.0, t.text3, t.danger).on_hover_text("Remove").clicked()
}

enum Piece {
    Kbd(String),
    Code(String),
    Text(String),
}

fn pieces(s: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let k = rest.find("<kbd>");
        let c = rest.find("<code>");
        let next = match (k, c) {
            (Some(a), Some(b2)) => Some(a.min(b2)),
            (a, b2) => a.or(b2),
        };
        let Some(i) = next else {
            out.push(Piece::Text(theme::unescape(rest)));
            break;
        };
        if i > 0 {
            out.push(Piece::Text(theme::unescape(&rest[..i])));
        }
        let is_kbd = rest[i..].starts_with("<kbd>");
        let (open, close) = if is_kbd { ("<kbd>", "</kbd>") } else { ("<code>", "</code>") };
        let body_start = i + open.len();
        let end = rest[body_start..].find(close).map(|e| body_start + e).unwrap_or(rest.len());
        let body = theme::unescape(&rest[body_start..end]);
        out.push(if is_kbd { Piece::Kbd(body) } else { Piece::Code(body) });
        rest = &rest[(end + close.len()).min(rest.len())..];
    }
    out
}

fn inline_keys_width(ui: &Ui, s: &str) -> f32 {
    let mut w = 0.0;
    let mut prev_kbd = false;
    for p in pieces(s) {
        match p {
            Piece::Kbd(k) => {
                if prev_kbd {
                    w += 4.0;
                }
                w += widgets::kbd_width(ui, &k);
                prev_kbd = true;
            }
            Piece::Code(c) => {
                w += theme::line(ui, &c, fm(12.0), Color32::WHITE, 600.0).size().x;
                prev_kbd = false;
            }
            Piece::Text(tx) => {
                w += theme::line(ui, &tx, f(13.0, 400.0), Color32::WHITE, 600.0).size().x;
                prev_kbd = false;
            }
        }
    }
    w
}

/// Draw `<kbd>`/`<code>` markup on one line as real key caps and code.
fn inline_keys(ui: &Ui, t: &Theme, x: f32, cy: f32, s: &str) {
    let mut x = x;
    let mut prev_kbd = false;
    for p in pieces(s) {
        match p {
            Piece::Kbd(k) => {
                if prev_kbd {
                    x += 4.0;
                }
                x += widgets::kbd(ui, t, x, cy, &k);
                prev_kbd = true;
            }
            Piece::Code(c) => {
                let g = theme::line(ui, &c, fm(12.0), t.accent, 600.0);
                x += theme::put(ui.painter(), &g, x, cy);
                prev_kbd = false;
            }
            Piece::Text(tx) => {
                let g = theme::line(ui, &tx, f(13.0, 400.0), t.text, 600.0);
                x += theme::put(ui.painter(), &g, x, cy);
                prev_kbd = false;
            }
        }
    }
}
