//! Shortcuts to the newest things:
//!   dl          your latest download (Enter opens it) · "downloads" lists the newest few
//!   ss          your latest screenshot (Enter copies the image) · "screenshots" lists them
//! and "Recent files": what Windows lists as recently opened, for the empty box.
use crate::{files, settings};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const PARTIAL: &[&str] = &["crdownload", "part", "partial", "tmp", "download", "opdownload"];
const IMAGES: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::UI::Shell::{SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

pub fn downloads_dir() -> PathBuf {
    known_folder(&windows::Win32::UI::Shell::FOLDERID_Downloads).unwrap_or_else(|| settings::user_home().join("Downloads"))
}

fn screenshot_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = known_folder(&windows::Win32::UI::Shell::FOLDERID_Screenshots) {
        v.push(p);
    }
    if let Some(p) = known_folder(&windows::Win32::UI::Shell::FOLDERID_Pictures) {
        v.push(p.join("Screenshots"));
    }
    v.push(settings::user_home().join("OneDrive").join("Pictures").join("Screenshots"));
    v.sort();
    v.dedup();
    v
}

fn newest(dirs: &[PathBuf], keep: impl Fn(&Path) -> bool, n: usize) -> Vec<(PathBuf, SystemTime)> {
    let mut all: Vec<(PathBuf, SystemTime)> = Vec::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if !keep(&p) {
                continue;
            }
            if let Ok(m) = e.metadata() {
                if m.is_file() {
                    all.push((p, m.modified().unwrap_or(SystemTime::UNIX_EPOCH)));
                }
            }
        }
    }
    all.sort_by(|a, b| b.1.cmp(&a.1));
    all.truncate(n);
    all
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

fn secs(t: SystemTime) -> u32 {
    t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

fn file_item(p: &Path, t: SystemTime, score: i32, sub_prefix: &str) -> Value {
    let size = std::fs::metadata(p).map(|m| m.len()).ok();
    let mut it = files::item_meta(&p.to_string_lossy(), false, score, Some(secs(t)), size);
    it["subtitle"] = json!(format!("{sub_prefix} · {}", it["subtitle"].as_str().unwrap_or("")));
    it
}

pub fn rows(q: &str) -> Vec<Value> {
    let q = q.trim().to_lowercase();
    match q.as_str() {
        "dl" | "download" | "latest download" | "last download" => newest(&[downloads_dir()], |p| !PARTIAL.contains(&ext(p).as_str()), 1)
            .into_iter()
            .map(|(p, t)| file_item(&p, t, 5800, "Latest download"))
            .collect(),
        "downloads" | "dls" => newest(&[downloads_dir()], |p| !PARTIAL.contains(&ext(p).as_str()), 8)
            .into_iter()
            .enumerate()
            .map(|(i, (p, t))| file_item(&p, t, 5800 - i as i32 * 10, "Download"))
            .collect(),
        "ss" | "screenshot" | "latest screenshot" | "last screenshot" | "screenshots" => {
            let n = if q == "screenshots" { 8 } else { 1 };
            newest(&screenshot_dirs(), |p| IMAGES.contains(&ext(p).as_str()), n)
                .into_iter()
                .enumerate()
                .map(|(i, (p, t))| {
                    let mut it = file_item(&p, t, 5800 - i as i32 * 10, if i == 0 { "Latest screenshot · Enter copies the image" } else { "Screenshot" });
                    it["screenshot"] = json!(true);
                    it["key"] = json!(format!("ss:{}", p.to_string_lossy().to_lowercase()));
                    it
                })
                .collect()
        }
        _ => vec![],
    }
}

/// The right-hand column of the Start-menu layout: your folders.
pub fn start_home() -> Value {
    use windows::Win32::UI::Shell::*;
    let home = settings::user_home();
    let f = |id: &windows::core::GUID, fallback: &str| known_folder(id).unwrap_or_else(|| home.join(fallback));
    let list = [
        ("Home", home.clone(), "folder"),
        ("Documents", f(&FOLDERID_Documents, "Documents"), "filetext"),
        ("Downloads", downloads_dir(), "download"),
        ("Pictures", f(&FOLDERID_Pictures, "Pictures"), "image"),
        ("Music", f(&FOLDERID_Music, "Music"), "volume"),
        ("Videos", f(&FOLDERID_Videos, "Videos"), "film"),
    ];
    let name = settings::load().display_name.trim().to_string();
    let user = if name.is_empty() { std::env::var("USERNAME").unwrap_or_default() } else { name };
    json!({
        "user": user,
        "folders": list.iter().filter(|(_, p, _)| p.exists()).map(|(n, p, g)| {
            let mut it = files::item(&p.to_string_lossy(), true, 0);
            it["title"] = json!(n);
            it["glyph"] = json!(g);
            it
        }).collect::<Vec<_>>(),
    })
}

/// Files Windows lists as recently opened (newest first), skipping ones that are gone.
pub fn recent_files(n: usize) -> Vec<Value> {
    let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) else { return vec![] };
    let dir = appdata.join(r"Microsoft\Windows\Recent");
    let links = newest(&[dir], |p| ext(p) == "lnk", n * 4);
    let mut out = Vec::new();
    for (lnk, t) in links {
        let Some(target) = crate::apps::link_target(&lnk) else { continue };
        let tp = Path::new(&target);
        if !tp.is_file() {
            continue;
        }
        if out.iter().any(|v: &Value| v["path"].as_str() == Some(target.as_str())) {
            continue;
        }
        out.push(file_item(tp, t, 100, "Opened"));
        if out.len() >= n {
            break;
        }
    }
    out
}
