//! Search ClipVault's clipboard history (read-only: %APPDATA%\ClipVault\history.json) and put
//! an old clip back on the clipboard, optionally pasting it into the window you came from.
use crate::{hook, winmgmt, wins};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::SystemTime;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Clip {
    id: String,
    kind: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    image_file: Option<String>,
    #[serde(default)]
    pixel_width: u32,
    #[serde(default)]
    pixel_height: u32,
    #[serde(default)]
    ocr_text: Option<String>,
    #[serde(default)]
    source_name: Option<String>,
    #[serde(default)]
    copied_at: Option<String>,
    #[serde(default)]
    pinned: bool,
}

struct Cache {
    stamp: Option<SystemTime>,
    clips: Vec<Clip>,
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache { stamp: None, clips: vec![] }));

fn dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_default().join("ClipVault")
}

pub fn available() -> bool {
    dir().join("history.json").exists()
}

/// Re-read the history when ClipVault has saved since last time.
fn load() -> Vec<Clip> {
    let path = dir().join("history.json");
    let stamp = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut c = CACHE.lock();
    if stamp.is_some() && stamp == c.stamp {
        return c.clips.clone();
    }
    let clips: Vec<Clip> = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    c.stamp = stamp;
    c.clips = clips.clone();
    clips
}

/// "2026-11-20T01:23:45.1234567+01:00" -> unix seconds
fn parse_time(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp())
        .ok()
        .or_else(|| chrono::NaiveDateTime::parse_from_str(&s[..s.len().min(19)], "%Y-%m-%dT%H:%M:%S").ok().map(|d| d.and_utc().timestamp()))
}

fn ago(t: Option<i64>) -> String {
    let Some(t) = t else { return String::new() };
    let d = (crate::history::now() as i64 - t).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86400),
    }
}

fn one_line(s: &str, max: usize) -> String {
    let flat: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max {
        flat.chars().take(max).collect::<String>() + "…"
    } else {
        flat
    }
}

fn item(c: &Clip, score: i32) -> Value {
    let when = ago(c.copied_at.as_deref().and_then(parse_time));
    let src = c.source_name.as_deref().filter(|s| !s.is_empty()).map(|s| format!(" · from {s}")).unwrap_or_default();
    let pin = if c.pinned { " · pinned" } else { "" };
    let (title, kind_label, icon, glyph) = match c.kind.as_str() {
        "Image" => {
            let file = c.image_file.as_deref().map(|f| dir().join("images").join(f).to_string_lossy().into_owned());
            let title = c.ocr_text.as_deref().filter(|t| !t.trim().is_empty()).map(|t| one_line(t, 90)).unwrap_or_else(|| format!("Image {}×{}", c.pixel_width, c.pixel_height));
            (title, "Image", file.map(|f| crate::icons::thumb_url(&f)), "image")
        }
        "Files" => {
            let names: Vec<String> = c
                .text
                .as_deref()
                .unwrap_or("")
                .lines()
                .filter_map(|l| std::path::Path::new(l.trim()).file_name().map(|n| n.to_string_lossy().into_owned()))
                .collect();
            (if names.is_empty() { "Files".into() } else { one_line(&names.join(", "), 90) }, "Files", None, "file")
        }
        _ => (one_line(c.text.as_deref().unwrap_or(""), 110), "Text", None, "copy"),
    };
    let path = c.image_file.as_deref().map(|f| dir().join("images").join(f).to_string_lossy().into_owned());
    json!({
        "key": format!("clip:{}", c.id), "kind": "clip", "title": title, "subtitle": format!("{kind_label} · {when}{src}{pin}"),
        "icon": icon, "glyph": glyph, "id": c.id, "clipkind": c.kind, "text": c.text.as_deref().map(|t| t.chars().take(4000).collect::<String>()),
        "image": path, "score": score,
    })
}

/// Clips containing every word, newest first (pinned clips a bit higher).
pub fn search(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    let clips = load();
    let n = clips.len() as i32;
    clips
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            if words.is_empty() {
                return true;
            }
            let hay = format!("{} {} {}", c.text.as_deref().unwrap_or(""), c.ocr_text.as_deref().unwrap_or(""), c.source_name.as_deref().unwrap_or("")).to_lowercase();
            words.iter().all(|w| hay.contains(w.as_str()))
        })
        .take(limit)
        .map(|(i, c)| {
            // ClipVault keeps newest first
            let s = 300 + (n - i as i32).min(300) / 3 + if c.pinned { 40 } else { 0 };
            (s, item(c, s))
        })
        .collect()
}

/// Build a .bmp file in memory from a PNG, for the clipboard's bitmap format.
/// Put an image file on the clipboard (PNG directly; other formats via the shell's decoder
/// aren't supported, so they're copied as a file instead).
pub fn copy_image(path: &str) -> Result<(), String> {
    if !path.to_lowercase().ends_with(".png") {
        return crate::shellops::copy_files(&[path.to_string()]);
    }
    let bmp = png_to_bmp(path).ok_or("Couldn't read the image")?;
    use clipboard_win::Setter;
    let _clip = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    clipboard_win::raw::empty().map_err(|e| e.to_string())?;
    clipboard_win::formats::Bitmap.write_clipboard(&bmp).map_err(|e| e.to_string())
}

fn png_to_bmp(path: &str) -> Option<Vec<u8>> {
    let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    let mut dec = dec;
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        _ => 1,
    };
    let row = w * 4;
    let mut px = vec![0u8; row * h];
    for y in 0..h {
        for x in 0..w {
            let s = &buf[(y * w + x) * ch..(y * w + x) * ch + ch];
            let (r, g, b, a) = match ch {
                4 => (s[0], s[1], s[2], s[3]),
                3 => (s[0], s[1], s[2], 255),
                2 => (s[0], s[0], s[0], s[1]),
                _ => (s[0], s[0], s[0], 255),
            };
            // bottom-up rows, BGRA
            let d = (h - 1 - y) * row + x * 4;
            px[d..d + 4].copy_from_slice(&[b, g, r, a]);
        }
    }
    let header = 14 + 40;
    let mut out = Vec::with_capacity(header + px.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((header + px.len()) as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(header as u32).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]); // BI_RGB
    out.extend_from_slice(&(px.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&px);
    Some(out)
}

/// Put a clip back on the clipboard; with `paste`, also paste it where you were.
pub fn restore(id: &str, paste: bool) -> Result<String, String> {
    let clips = load();
    let c = clips.iter().find(|c| c.id == id).ok_or("That clip is no longer in ClipVault")?;
    match c.kind.as_str() {
        "Image" => {
            let p = c.image_file.as_deref().map(|f| dir().join("images").join(f)).ok_or("Image file missing")?;
            let bmp = png_to_bmp(&p.to_string_lossy()).ok_or("Couldn't read the image")?;
            use clipboard_win::Setter;
            let _clip = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
            clipboard_win::raw::empty().map_err(|e| e.to_string())?;
            clipboard_win::formats::Bitmap.write_clipboard(&bmp).map_err(|e| e.to_string())?;
        }
        "Files" => {
            let paths: Vec<String> = c.text.as_deref().unwrap_or("").lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
            crate::shellops::copy_files(&paths)?;
        }
        _ => crate::shellops::copy_text(c.text.as_deref().unwrap_or(""))?,
    }
    if paste {
        if let Some(h) = winmgmt::prev() {
            let raw = h.0 as isize;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(90));
                wins::force_foreground(windows::Win32::Foundation::HWND(raw as *mut _));
                std::thread::sleep(std::time::Duration::from_millis(120));
                hook::send_paste();
            });
            return Ok("Pasted".into());
        }
    }
    Ok("Copied to the clipboard".into())
}
