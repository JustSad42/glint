//! Icons and thumbnails from the Windows shell, served to the page as PNGs at
//! `http://gicon.localhost/<kind>/<url-encoded key>`:
//!   app/<AppsFolder id>   an installed app
//!   path/<full path>      a file or folder (one icon per extension for ordinary files)
//!   thumb/<full path>     a large thumbnail for the preview pane
//! Extraction runs on a few background STA threads; results are cached in memory, and app and
//! file-type icons also on disk so the first screen after a restart is instant.
use crate::settings;
use crossbeam_channel::{unbounded, Sender};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY, SIIGBF_RESIZETOFIT, SIIGBF_THUMBNAILONLY,
};

pub const BASE: &str = "http://gicon.localhost";

type Png = Arc<Vec<u8>>;

struct Job {
    parsing: String,
    size: i32,
    thumb: bool,
    key: String,
    disk: bool,
    reply: Box<dyn FnOnce(Option<Png>) + Send>,
}

static CACHE: Lazy<Mutex<HashMap<String, Png>>> = Lazy::new(|| Mutex::new(HashMap::new()));

static QUEUE: Lazy<Sender<Job>> = Lazy::new(|| {
    let (tx, rx) = unbounded::<Job>();
    for n in 0..3 {
        let rx = rx.clone();
        std::thread::Builder::new()
            .name(format!("icons-{n}"))
            .spawn(move || {
                unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
                }
                for job in rx.iter() {
                    // someone may have produced it while this job waited
                    if let Some(p) = CACHE.lock().get(&job.key).cloned() {
                        (job.reply)(Some(p));
                        continue;
                    }
                    let png = shell_image(&job.parsing, job.size, job.thumb).map(Arc::new);
                    if let Some(p) = &png {
                        let mut c = CACHE.lock();
                        if c.len() > 4000 {
                            c.clear();
                        }
                        c.insert(job.key.clone(), p.clone());
                        if job.disk {
                            let _ = std::fs::create_dir_all(icon_dir());
                            let _ = std::fs::write(disk_path(&job.key), p.as_slice());
                        }
                    }
                    (job.reply)(png);
                }
            })
            .ok();
    }
    tx
});

fn icon_dir() -> PathBuf {
    settings::data_file("icons")
}

fn disk_path(key: &str) -> PathBuf {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut h);
    icon_dir().join(format!("{:016x}.png", h.finish()))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Extensions whose icon differs per file.
const OWN_ICON: &[&str] = &["exe", "lnk", "ico", "url", "appref-ms", "cur", "ani", "scr", "msc", "cpl", "website", "msi"];

pub fn app_url(id: &str) -> String {
    format!("{BASE}/app/{}", urlencoding::encode(id))
}

pub fn path_url(path: &str) -> String {
    format!("{BASE}/path/{}", urlencoding::encode(path))
}

pub fn thumb_url(path: &str) -> String {
    format!("{BASE}/thumb/{}", urlencoding::encode(path))
}

/// Answer one request for `/kind/key`. Calls `reply` exactly once (maybe from another thread).
pub fn request(url_path: &str, reply: impl FnOnce(Option<Png>) + Send + 'static) {
    let url_path = url_path.trim_start_matches('/');
    let (kind, rest) = url_path.split_once('/').unwrap_or((url_path, ""));
    let rest = urlencoding::decode(rest).map(|c| c.into_owned()).unwrap_or_default();
    if kind == "media" {
        // cover art changes with the track, so it's never cached; WinRT calls go on an MTA thread
        std::thread::spawn(move || {
            unsafe {
                let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
            }
            reply(crate::media::cover().map(Arc::new));
        });
        return;
    }
    let (parsing, key, size, thumb, disk) = match kind {
        "app" => (format!("shell:AppsFolder\\{rest}"), format!("app:{rest}"), 64, false, true),
        "thumb" => (rest.clone(), format!("thumb:{rest}"), 320, true, false),
        "path" => {
            let p = std::path::Path::new(&rest);
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let is_dir = p.is_dir();
            if !is_dir && !ext.is_empty() && !OWN_ICON.contains(&ext.as_str()) {
                (rest.clone(), format!("ext:{ext}"), 64, false, true)
            } else {
                (rest.clone(), format!("path:{}", rest.to_lowercase()), 64, false, false)
            }
        }
        _ => {
            reply(None);
            return;
        }
    };
    if let Some(p) = CACHE.lock().get(&key).cloned() {
        reply(Some(p));
        return;
    }
    if disk {
        if let Ok(bytes) = std::fs::read(disk_path(&key)) {
            let p = Arc::new(bytes);
            CACHE.lock().insert(key, p.clone());
            reply(Some(p));
            return;
        }
    }
    let _ = QUEUE.send(Job { parsing, size, thumb, key, disk, reply: Box::new(reply) });
}

/// Warm the cache for app icons so the first search shows them at once.
pub fn prefetch_app(id: &str) {
    let key = format!("app:{id}");
    if CACHE.lock().contains_key(&key) || disk_path(&key).exists() {
        return;
    }
    let _ = QUEUE.send(Job { parsing: format!("shell:AppsFolder\\{id}"), size: 64, thumb: false, key, disk: true, reply: Box::new(|_| {}) });
}

/// Delete the on-disk icon cache (apps may have changed their icons after an update).
pub fn clear_disk_cache() {
    let _ = std::fs::remove_dir_all(icon_dir());
    CACHE.lock().clear();
}

fn shell_image(parsing: &str, size: i32, thumb: bool) -> Option<Vec<u8>> {
    let w = wide(parsing);
    let factory: IShellItemImageFactory = unsafe { SHCreateItemFromParsingName(PCWSTR(w.as_ptr()), None) }.ok()?;
    let tries: &[SIIGBF] = if thumb {
        &[SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT]
    } else {
        &[SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK, SIIGBF_RESIZETOFIT]
    };
    for flags in tries {
        if let Ok(hbm) = unsafe { factory.GetImage(SIZE { cx: size, cy: size }, *flags) } {
            let png = bitmap_to_png(hbm);
            unsafe {
                let _ = DeleteObject(HGDIOBJ(hbm.0));
            }
            if png.is_some() {
                return png;
            }
        }
    }
    None
}

fn bitmap_to_png(hbm: HBITMAP) -> Option<Vec<u8>> {
    unsafe {
        let mut bm = BITMAP::default();
        if GetObjectW(HGDIOBJ(hbm.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as _)) == 0 {
            return None;
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
        if w <= 0 || h <= 0 || w > 4096 || h > 4096 {
            return None;
        }
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, hbm, 0, h as u32, Some(px.as_mut_ptr() as _), &mut bmi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
        for p in px.chunks_exact_mut(4) {
            let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
            if !has_alpha {
                p.copy_from_slice(&[r, g, b, 255]);
            } else if a == 0 {
                p.copy_from_slice(&[0, 0, 0, 0]);
            } else {
                // the shell hands out premultiplied alpha
                let un = |c: u8| ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8;
                p.copy_from_slice(&[un(r), un(g), un(b), a]);
            }
        }
        let mut out = Vec::with_capacity(px.len() / 3);
        {
            let mut enc = png::Encoder::new(&mut out, w as u32, h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut writer = enc.write_header().ok()?;
            writer.write_image_data(&px).ok()?;
        }
        Some(out)
    }
}
