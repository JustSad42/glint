//! Glint's design tokens (the same values as the old common.css), fonts and text helpers.
use crate::settings::Settings;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Galley, Painter, Pos2, Ui};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub translucent: bool,
    pub accent: Color32,
    pub bg: Color32,
    pub bg_solid: Color32,
    pub panel: Color32,
    pub panel2: Color32,
    pub line: Color32,
    pub line2: Color32,
    pub text: Color32,
    pub text2: Color32,
    pub text3: Color32,
    pub hover: Color32,
    pub sel: Color32,
    pub sel_line: Color32,
    pub mark: Color32,
    pub kbd: Color32,
    pub danger: Color32,
    pub ok: Color32,
    pub warn: Color32,
    pub shadow: Color32,
}

fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, (a * 255.0).round() as u8)
}

pub fn hex(s: &str) -> Option<Color32> {
    let s = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(s.get(i..i + 2)?, 16).ok();
    match s.len() {
        6 => Some(Color32::from_rgb(v(0)?, v(2)?, v(4)?)),
        3 => {
            let d = |i: usize| u8::from_str_radix(&s[i..i + 1], 16).ok().map(|x| x * 17);
            Some(Color32::from_rgb(d(0)?, d(1)?, d(2)?))
        }
        _ => None,
    }
}

pub fn to_hex(c: Color32) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r(), c.g(), c.b())
}

// ---------------------------------------------------------------- color-mix(in oklab, ...)

fn lin(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}
fn unlin(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.0031308 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}
fn to_oklab(c: Color32) -> [f32; 3] {
    let (r, g, b) = (lin(c.r()), lin(c.g()), lin(c.b()));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}
fn from_oklab(p: [f32; 3]) -> Color32 {
    let l = p[0] + 0.396_337_78 * p[1] + 0.215_803_76 * p[2];
    let m = p[0] - 0.105_561_346 * p[1] - 0.063_854_17 * p[2];
    let s = p[0] - 0.089_484_18 * p[1] - 1.291_485_5 * p[2];
    let (l, m, s) = (l * l * l, m * m * m, s * s * s);
    Color32::from_rgb(
        unlin(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        unlin(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        unlin(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
    )
}

/// color-mix(in oklab, a p, b)
pub fn mix(a: Color32, p: f32, b: Color32) -> Color32 {
    let (x, y) = (to_oklab(a), to_oklab(b));
    from_oklab([x[0] * p + y[0] * (1.0 - p), x[1] * p + y[1] * (1.0 - p), x[2] * p + y[2] * (1.0 - p)])
}

/// color-mix(in oklab, c p, transparent)
pub fn alpha(c: Color32, p: f32) -> Color32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    Color32::from_rgba_unmultiplied(r, g, b, (a as f32 * p).round() as u8)
}

/// Paint `top` over `under` (both maybe translucent) into one opaque-ish colour.
pub fn over(under: Color32, top: Color32) -> Color32 {
    let [r1, g1, b1, _] = under.to_srgba_unmultiplied();
    let [r2, g2, b2, a2] = top.to_srgba_unmultiplied();
    let t = a2 as f32 / 255.0;
    let m = |a: u8, b: u8| (a as f32 * (1.0 - t) + b as f32 * t).round() as u8;
    Color32::from_rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

pub fn is_dark(s: &Settings) -> bool {
    match s.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => system_dark(),
    }
}

/// Windows' "app mode" (Settings > Personalization > Colors).
pub fn system_dark() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        .and_then(|k| k.get_value::<u32, _>("AppsUseLightTheme"))
        .map(|v| v == 0)
        .unwrap_or(true)
}

impl Theme {
    pub fn from_settings(s: &Settings, translucent: bool) -> Theme {
        let accent = hex(&s.accent).unwrap_or(Color32::from_rgb(0x7c, 0x6c, 0xff));
        Theme::new(is_dark(s), accent, translucent)
    }

    pub fn new(dark: bool, accent: Color32, translucent: bool) -> Theme {
        if dark {
            Theme {
                dark,
                translucent,
                accent,
                bg: if translucent { rgba(24, 24, 30, 0.80) } else { Color32::from_rgb(0x1b, 0x1b, 0x22) },
                bg_solid: Color32::from_rgb(0x18, 0x18, 0x1e),
                panel: rgba(255, 255, 255, 0.035),
                panel2: rgba(255, 255, 255, 0.06),
                line: rgba(255, 255, 255, 0.075),
                line2: rgba(255, 255, 255, 0.12),
                text: Color32::from_rgb(0xf2, 0xf2, 0xf7),
                text2: rgba(235, 235, 245, 0.66),
                text3: rgba(235, 235, 245, 0.42),
                hover: rgba(255, 255, 255, 0.05),
                sel: alpha(accent, 0.22),
                sel_line: alpha(accent, 0.55),
                mark: mix(accent, 0.70, Color32::WHITE),
                kbd: rgba(255, 255, 255, 0.08),
                danger: Color32::from_rgb(0xff, 0x6b, 0x6b),
                ok: Color32::from_rgb(0x4a, 0xde, 0x80),
                warn: Color32::from_rgb(0xfb, 0xbf, 0x24),
                shadow: rgba(0, 0, 0, 0.45),
            }
        } else {
            Theme {
                dark,
                translucent,
                accent,
                bg: if translucent { rgba(250, 250, 253, 0.84) } else { Color32::from_rgb(0xfa, 0xfa, 0xfd) },
                bg_solid: Color32::from_rgb(0xf7, 0xf7, 0xfa),
                panel: rgba(0, 0, 0, 0.025),
                panel2: rgba(0, 0, 0, 0.045),
                line: rgba(0, 0, 0, 0.07),
                line2: rgba(0, 0, 0, 0.12),
                text: Color32::from_rgb(0x16, 0x16, 0x1d),
                text2: rgba(22, 22, 29, 0.66),
                text3: rgba(22, 22, 29, 0.44),
                hover: rgba(0, 0, 0, 0.04),
                sel: alpha(accent, 0.16),
                sel_line: alpha(accent, 0.50),
                mark: mix(accent, 0.85, Color32::BLACK),
                kbd: rgba(0, 0, 0, 0.06),
                danger: Color32::from_rgb(0xe0, 0x31, 0x31),
                ok: Color32::from_rgb(0x16, 0xa3, 0x4a),
                warn: Color32::from_rgb(0xd9, 0x77, 0x06),
                shadow: rgba(0, 0, 0, 0.18),
            }
        }
    }

    pub fn tint(&self, c: Color32, p: f32) -> Color32 {
        alpha(c, p)
    }

    /// Apply to egui's own widgets (text boxes, scroll bars, combo boxes, sliders).
    pub fn apply(&self, ctx: &egui::Context) {
        let t = self.clone();
        ctx.all_styles_mut(move |s| {
            let v = &mut s.visuals;
            *v = if t.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
            v.panel_fill = Color32::TRANSPARENT;
            v.window_fill = t.bg_solid;
            v.window_stroke = egui::Stroke::new(1.0, t.line2);
            v.window_corner_radius = egui::CornerRadius::same(14);
            v.menu_corner_radius = egui::CornerRadius::same(10);
            v.window_shadow = egui::Shadow { offset: [0, 18], blur: 50, spread: 0, color: t.shadow };
            v.popup_shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: t.shadow };
            v.override_text_color = Some(t.text);
            v.extreme_bg_color = t.panel2;
            v.faint_bg_color = t.panel;
            v.selection.bg_fill = alpha(t.accent, 0.35);
            v.selection.stroke = egui::Stroke::new(1.0, t.text);
            v.text_cursor.stroke = egui::Stroke::new(2.0, t.accent);
            v.hyperlink_color = t.accent;
            for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
                w.corner_radius = egui::CornerRadius::same(9);
                w.fg_stroke = egui::Stroke::new(1.0, t.text);
                w.expansion = 0.0;
            }
            v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, t.line);
            v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, t.text2);
            v.widgets.inactive.bg_fill = t.line2;
            v.widgets.inactive.weak_bg_fill = t.panel2;
            v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, t.line2);
            v.widgets.hovered.bg_fill = t.text3;
            v.widgets.hovered.weak_bg_fill = t.hover;
            v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, t.text3);
            v.widgets.active.bg_fill = t.accent;
            v.widgets.active.weak_bg_fill = t.hover;
            v.widgets.active.bg_stroke = egui::Stroke::new(1.0, t.accent);
            v.widgets.open.weak_bg_fill = t.panel2;
            v.widgets.open.bg_stroke = egui::Stroke::new(1.0, t.accent);
            let mut sc = egui::style::ScrollStyle::solid();
            sc.bar_width = 4.0;
            sc.handle_min_length = 24.0;
            sc.bar_inner_margin = 3.0;
            sc.bar_outer_margin = 0.0;
            sc.foreground_color = false;
            sc.dormant_background_opacity = 0.0;
            sc.active_background_opacity = 0.0;
            sc.interact_background_opacity = 0.0;
            sc.dormant_handle_opacity = 1.0;
            sc.active_handle_opacity = 1.0;
            sc.interact_handle_opacity = 1.0;
            s.spacing.scroll = sc;
            s.spacing.item_spacing = egui::vec2(0.0, 0.0);
            s.spacing.button_padding = egui::vec2(12.0, 6.0);
            s.spacing.interact_size.y = 34.0;
            s.interaction.selectable_labels = false;
            s.text_styles.insert(egui::TextStyle::Body, FontId::new(13.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Button, FontId::new(13.0, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Small, FontId::new(11.5, FontFamily::Proportional));
            s.text_styles.insert(egui::TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace));
        });
    }
}

// ---------------------------------------------------------------- fonts

/// Segoe UI Variable (with its weight and optical-size axes), Cascadia for code, and Segoe UI
/// Symbol for arrows and keys like ↵ ⊞.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::empty();
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    let load = |f: &str| std::fs::read(dir.join(f)).ok();
    let mut prop = Vec::new();
    if let Some(b) = load("SegUIVar.ttf") {
        let mut fd = egui::FontData::from_owned(b);
        // the web view sat glyphs a little higher in their line box
        fd.tweak.y_offset = tune("GLINT_TEST_YOFF", -1.0);
        fonts.font_data.insert("segvar".into(), Arc::new(fd));
        prop.push("segvar".to_string());
        VARIABLE.store(true, std::sync::atomic::Ordering::SeqCst);
    } else if let Some(b) = load("segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(egui::FontData::from_owned(b)));
        prop.push("segoe".to_string());
        if let Some(b) = load("seguisb.ttf") {
            fonts.font_data.insert("segoe-sb".into(), Arc::new(egui::FontData::from_owned(b)));
        }
    }
    for f in ["seguisym.ttf", "seguiemj.ttf"] {
        if let Some(b) = load(f) {
            fonts.font_data.insert(f.into(), Arc::new(egui::FontData::from_owned(b)));
            prop.push(f.to_string());
        }
    }
    let mono = ["CascadiaCode.ttf", "CascadiaMono.ttf", "consola.ttf"].iter().find_map(|f| load(f));
    let mut monos = Vec::new();
    if let Some(b) = mono {
        fonts.font_data.insert("mono".into(), Arc::new(egui::FontData::from_owned(b)));
        monos.push("mono".to_string());
    }
    monos.extend(prop.iter().cloned());
    let mut bold = Vec::new();
    if fonts.font_data.contains_key("segoe-sb") {
        bold.push("segoe-sb".to_string());
    }
    bold.extend(prop.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    fonts.families.insert(FontFamily::Monospace, monos);
    fonts.families.insert(FontFamily::Name("bold".into()), bold);
    ctx.set_fonts(fonts);
}

static VARIABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn tune(var: &str, default: f32) -> f32 {
    std::env::var(var).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// DirectWrite (what the web view used) draws text a touch heavier than egui's rasterizer;
/// asking for a slightly bolder weight evens that out.
static BOOST: once_cell::sync::Lazy<f32> = once_cell::sync::Lazy::new(|| tune("GLINT_TEST_WBOOST", 120.0));

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Face {
    /// "Segoe UI Variable Text"
    Text,
    /// "Segoe UI Variable Display"
    Display,
    Mono,
}

/// One run of text: size in points, CSS weight.
#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub size: f32,
    pub weight: f32,
    pub face: Face,
}

pub const fn f(size: f32, weight: f32) -> Font {
    Font { size, weight, face: Face::Text }
}
pub const fn fd(size: f32, weight: f32) -> Font {
    Font { size, weight, face: Face::Display }
}
pub const fn fm(size: f32) -> Font {
    Font { size, weight: 400.0, face: Face::Mono }
}

pub fn format(font: Font, color: Color32) -> TextFormat {
    let variable = VARIABLE.load(std::sync::atomic::Ordering::Relaxed);
    let family = match font.face {
        Face::Mono => FontFamily::Monospace,
        _ if !variable && font.weight >= 560.0 => FontFamily::Name("bold".into()),
        _ => FontFamily::Proportional,
    };
    let mut tf = TextFormat { font_id: FontId::new(font.size, family), color, ..Default::default() };
    if variable && font.face != Face::Mono {
        let opsz = if font.face == Face::Display { 36.0 } else { 10.5 };
        tf.coords = egui::epaint::text::VariationCoords::new([(b"wght", (font.weight + *BOOST).min(700.0)), (b"opsz", opsz)]);
    }
    tf
}

pub fn job(text: &str, font: Font, color: Color32) -> LayoutJob {
    let mut j = LayoutJob::default();
    j.append(text, 0.0, format(font, color));
    j
}

/// Lay out one line, cut with an ellipsis at `max_w`.
pub fn line(ui: &Ui, text: &str, font: Font, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut j = job(text, font, color);
    j.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(1.0));
    ui.fonts_mut(|f| f.layout_job(j))
}

/// Lay out wrapped text.
pub fn wrapped(ui: &Ui, text: &str, font: Font, color: Color32, max_w: f32) -> Arc<Galley> {
    let mut j = job(text, font, color);
    j.wrap.max_width = max_w.max(1.0);
    ui.fonts_mut(|f| f.layout_job(j))
}

pub fn layout(ui: &Ui, mut j: LayoutJob, max_w: f32, truncate: bool) -> Arc<Galley> {
    if truncate {
        j.wrap = egui::text::TextWrapping::truncate_at_width(max_w.max(1.0));
    } else {
        j.wrap.max_width = max_w.max(1.0);
    }
    ui.fonts_mut(|f| f.layout_job(j))
}

/// Paint a galley with its left edge at x, vertically centred on y.
pub fn put(p: &Painter, g: &Arc<Galley>, x: f32, cy: f32) -> f32 {
    let pos = Pos2::new(x, (cy - g.size().y / 2.0).round());
    p.galley(pos, g.clone(), Color32::WHITE);
    g.size().x
}

/// Paint a galley right-aligned at x.
pub fn put_right(p: &Painter, g: &Arc<Galley>, right: f32, cy: f32) -> f32 {
    let pos = Pos2::new(right - g.size().x, (cy - g.size().y / 2.0).round());
    p.galley(pos, g.clone(), Color32::WHITE);
    g.size().x
}

/// Inline markup used in descriptions: `<kbd>…</kbd>`, `<code>…</code>`, `<b>…</b>`.
/// `<code>` takes the surrounding color, as in setting descriptions; see `rich_accent`.
pub fn rich(text: &str, font: Font, color: Color32, t: &Theme) -> LayoutJob {
    rich_with(text, font, color, t, color)
}

/// Same, with `<code>` in the accent color (page leads, notes and launcher tips).
pub fn rich_accent(text: &str, font: Font, color: Color32, t: &Theme) -> LayoutJob {
    rich_with(text, font, color, t, t.accent)
}

fn rich_with(text: &str, font: Font, color: Color32, t: &Theme, code: Color32) -> LayoutJob {
    let mut j = LayoutJob::default();
    let mut rest = text;
    while !rest.is_empty() {
        let next = ["<kbd>", "<code>", "<b>"].iter().filter_map(|tag| rest.find(tag).map(|i| (i, *tag))).min_by_key(|x| x.0);
        let Some((i, tag)) = next else {
            j.append(&unescape(rest), 0.0, format(font, color));
            break;
        };
        if i > 0 {
            j.append(&unescape(&rest[..i]), 0.0, format(font, color));
        }
        let close = tag.replace('<', "</");
        let body_start = i + tag.len();
        let end = rest[body_start..].find(&close).map(|e| body_start + e).unwrap_or(rest.len());
        let body = unescape(&rest[body_start..end]);
        match tag {
            "<kbd>" => {
                let mut tf = format(Font { size: font.size - 1.5, weight: 600.0, face: Face::Text }, t.text2);
                tf.background = t.kbd;
                tf.expand_bg = 2.0;
                j.append("\u{2009}", 0.0, format(font, color));
                j.append(&format!("\u{2009}{body}\u{2009}"), 0.0, tf);
                j.append("\u{2009}", 0.0, format(font, color));
            }
            "<code>" => j.append(&body, 0.0, format(Font { size: font.size - 0.5, weight: 400.0, face: Face::Mono }, code)),
            _ => j.append(&body, 0.0, format(Font { weight: 600.0, ..font }, t.text)),
        }
        rest = &rest[(end + close.len()).min(rest.len())..];
    }
    j
}

pub fn unescape(s: &str) -> String {
    s.replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&").replace("&quot;", "\"")
}
