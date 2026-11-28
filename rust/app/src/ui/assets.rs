//! Textures: line icons (rasterized from SVG at the exact pixel size), shell icons and
//! thumbnails (the old `gicon` URLs), and pictures from files for the preview and Quick Look.
use super::glyph_data::GLYPHS;
use crossbeam_channel::{unbounded, Receiver, Sender};
use egui::{Color32, ColorImage, Context, Rect, TextureHandle, TextureOptions, Ui};
use std::collections::HashMap;
use std::sync::Arc;

pub struct Pic {
    pub tex: TextureHandle,
    /// the picture's real size (before it was scaled down for the screen)
    pub dims: [u32; 2],
}

enum Slot {
    Loading,
    Ready(Arc<Pic>),
    Failed,
}

pub struct Assets {
    glyphs: HashMap<(&'static str, u32), TextureHandle>,
    images: HashMap<String, Slot>,
    tx: Sender<(String, Option<(ColorImage, [u32; 2])>)>,
    rx: Receiver<(String, Option<(ColorImage, [u32; 2])>)>,
    /// big pictures from files (Quick Look, preview), dropped first when there are many
    big: Vec<String>,
}

impl Default for Assets {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        Assets { glyphs: HashMap::new(), images: HashMap::new(), tx, rx, big: Vec::new() }
    }
}

fn svg_for(name: &str) -> &'static str {
    GLYPHS.iter().find(|(n, _)| *n == name).or_else(|| GLYPHS.iter().find(|(n, _)| *n == "sparkle")).map(|(_, b)| *b).unwrap_or("")
}

fn static_name(name: &str) -> &'static str {
    GLYPHS.iter().find(|(n, _)| *n == name).map(|(n, _)| *n).unwrap_or("sparkle")
}

fn rasterize(name: &str, px: u32) -> ColorImage {
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="white" color="white" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
        svg_for(name).replace("currentColor", "white")
    );
    let mut pm = resvg::tiny_skia::Pixmap::new(px, px).unwrap();
    if let Ok(tree) = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()) {
        let s = px as f32 / 24.0;
        resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(s, s), &mut pm.as_mut());
    }
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data())
}

impl Assets {
    /// Draw a line icon centred in `rect` (its size in points = rect width).
    pub fn glyph(&mut self, ui: &Ui, name: &str, rect: Rect, color: Color32) {
        let ppp = ui.ctx().pixels_per_point();
        let px = (rect.width() * ppp).round().max(4.0) as u32;
        let key = (static_name(name), px);
        let tex = self.glyphs.entry(key).or_insert_with(|| ui.ctx().load_texture(format!("glyph-{}-{px}", key.0), rasterize(key.0, px), TextureOptions::LINEAR)).clone();
        // snap to whole pixels so 1.8px strokes stay crisp
        let min = (rect.min.to_vec2() * ppp).round() / ppp;
        let r = Rect::from_min_size(min.to_pos2(), rect.size());
        ui.painter().image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), color);
    }

    /// Same, rotated around its centre (the notice bell rings).
    pub fn glyph_rotated(&mut self, ui: &Ui, name: &str, rect: Rect, color: Color32, angle: f32) {
        let ppp = ui.ctx().pixels_per_point();
        let px = (rect.width() * ppp).round().max(4.0) as u32;
        let key = (static_name(name), px);
        let tex = self.glyphs.entry(key).or_insert_with(|| ui.ctx().load_texture(format!("glyph-{}-{px}", key.0), rasterize(key.0, px), TextureOptions::LINEAR)).clone();
        egui::Image::new((tex.id(), rect.size())).tint(color).rotate(angle, egui::vec2(0.5, 0.5)).paint_at(ui, rect);
    }

    /// Bring in what the loader threads finished. Returns true if anything arrived.
    pub fn poll(&mut self, ctx: &Context) -> bool {
        let mut any = false;
        while let Ok((key, img)) = self.rx.try_recv() {
            any = true;
            let slot = match img {
                Some((ci, dims)) => Slot::Ready(Arc::new(Pic { tex: ctx.load_texture(&key, ci, TextureOptions::LINEAR), dims })),
                None => Slot::Failed,
            };
            self.images.insert(key, slot);
        }
        any
    }

    /// A shell icon / thumbnail / cover by its old URL (`http://gicon.localhost/...`), or a file
    /// picture (`file:<path>`). None while loading or if it failed; `failed` tells which.
    pub fn image(&mut self, ctx: &Context, key: &str) -> Option<Arc<Pic>> {
        match self.images.get(key) {
            Some(Slot::Ready(p)) => return Some(p.clone()),
            Some(_) => return None,
            None => {}
        }
        if self.images.len() > 1500 {
            self.images.retain(|_, s| matches!(s, Slot::Loading));
            self.big.clear();
        }
        self.images.insert(key.to_string(), Slot::Loading);
        let tx = self.tx.clone();
        let ctx2 = ctx.clone();
        let k = key.to_string();
        if let Some(path) = key.strip_prefix("file:") {
            self.big.push(k.clone());
            if self.big.len() > 6 {
                let old = self.big.remove(0);
                self.images.remove(&old);
            }
            let path = path.to_string();
            std::thread::spawn(move || {
                let r = load_file_image(&path, 1600);
                let _ = tx.send((k, r));
                ctx2.request_repaint();
            });
        } else if let Some(rest) = key.strip_prefix(crate::icons::BASE) {
            crate::icons::request(rest, move |png| {
                let r = png.and_then(|p| decode(&p, 1024));
                let _ = tx.send((k, r));
                ctx2.request_repaint();
            });
        } else {
            self.images.insert(key.to_string(), Slot::Failed);
        }
        None
    }

    pub fn failed(&self, key: &str) -> bool {
        matches!(self.images.get(key), Some(Slot::Failed))
    }

    /// Forget a picture that changes (the media cover).
    pub fn forget(&mut self, key: &str) {
        self.images.remove(key);
    }
}

fn to_color_image(img: image::DynamicImage, max: u32) -> (ColorImage, [u32; 2]) {
    let dims = [img.width(), img.height()];
    let img = if img.width() > max || img.height() > max { img.thumbnail(max, max) } else { img };
    let rgba = img.to_rgba8();
    (ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()), dims)
}

fn decode(bytes: &[u8], max: u32) -> Option<(ColorImage, [u32; 2])> {
    image::load_from_memory(bytes).ok().map(|i| to_color_image(i, max))
}

pub fn load_file_image(path: &str, max: u32) -> Option<(ColorImage, [u32; 2])> {
    let img = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    Some(to_color_image(img, max))
}

/// Picture formats Glint can draw itself.
pub fn decodable(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico")
}
