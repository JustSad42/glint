//! Extra facts for the preview pane: what kind of media a file is, which app opens it,
//! version info for programs, shortcut targets, and the contents of zip archives.
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::UI::Shell::{AssocQueryStringW, ASSOCF_NONE, ASSOCSTR_FRIENDLYAPPNAME};

const IMAGES: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "avif", "jfif"];
const VIDEOS: &[&str] = &["mp4", "webm", "mov", "m4v", "mkv", "ogv"];
const AUDIO: &[&str] = &["mp3", "wav", "ogg", "flac", "m4a", "aac", "opus", "weba"];
const ARCHIVES: &[&str] = &["zip", "jar", "apk", "nupkg", "vsix", "xpi", "cbz"];

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// "Visual Studio Code" for ".rs"
pub fn opens_with(ext: &str) -> Option<String> {
    if ext.is_empty() {
        return None;
    }
    let e = wide(&format!(".{ext}"));
    let mut buf = [0u16; 260];
    let mut n = buf.len() as u32;
    let hr = unsafe { AssocQueryStringW(ASSOCF_NONE, ASSOCSTR_FRIENDLYAPPNAME, PCWSTR(e.as_ptr()), PCWSTR::null(), Some(PWSTR(buf.as_mut_ptr())), &mut n) };
    if hr.is_err() {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n.saturating_sub(1) as usize]);
    (!s.trim().is_empty()).then(|| s.trim().to_string())
}

/// FileDescription / CompanyName / ProductVersion of an .exe or .dll.
fn version_info(path: &str) -> Option<Value> {
    let w = wide(path);
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(w.as_ptr()), None) };
    if size == 0 {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    unsafe { GetFileVersionInfoW(PCWSTR(w.as_ptr()), None, size, data.as_mut_ptr() as _) }.ok()?;
    // language + code page of the first translation
    let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
    let mut len = 0u32;
    let tr = wide("\\VarFileInfo\\Translation");
    let lang = if unsafe { VerQueryValueW(data.as_ptr() as _, PCWSTR(tr.as_ptr()), &mut p, &mut len) }.as_bool() && len >= 4 {
        let v = unsafe { std::slice::from_raw_parts(p as *const u16, 2) };
        format!("{:04x}{:04x}", v[0], v[1])
    } else {
        "040904b0".into()
    };
    let get = |name: &str| -> Option<String> {
        let q = wide(&format!("\\StringFileInfo\\{lang}\\{name}"));
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if unsafe { VerQueryValueW(data.as_ptr() as _, PCWSTR(q.as_ptr()), &mut p, &mut len) }.as_bool() && len > 1 {
            let s = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p as *const u16, len as usize - 1) });
            let s = s.trim_matches(char::from(0)).trim().to_string();
            (!s.is_empty()).then_some(s)
        } else {
            None
        }
    };
    Some(json!({
        "description": get("FileDescription"), "company": get("CompanyName"),
        "version": get("ProductVersion").or_else(|| get("FileVersion")),
    }))
}

fn zip_listing(path: &str) -> Option<Value> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let count = z.len();
    let mut total: u64 = 0;
    let mut names = Vec::new();
    for i in 0..count {
        if let Ok(e) = z.by_index(i) {
            total += e.size();
            if names.len() < 16 {
                names.push(json!({"name": e.name().trim_end_matches('/'), "dir": e.is_dir()}));
            }
        }
    }
    Some(json!({"count": count, "unpacked": total, "entries": names}))
}

/// Fill extra preview facts for a file into `d`.
pub fn extra(path: &str, d: &mut Value) {
    let p = Path::new(path);
    let ext = crate::extract::ext_of(p);
    let media = if IMAGES.contains(&ext.as_str()) {
        "image"
    } else if VIDEOS.contains(&ext.as_str()) {
        "video"
    } else if AUDIO.contains(&ext.as_str()) {
        "audio"
    } else {
        ""
    };
    if !media.is_empty() {
        d["media"] = json!(media);
    }
    if p.is_dir() {
        return;
    }
    if let Some(app) = opens_with(&ext) {
        d["opens_with"] = json!(app);
    }
    match ext.as_str() {
        "exe" | "dll" | "msi" | "sys" => {
            if let Some(v) = version_info(path) {
                d["version"] = v;
            }
        }
        "lnk" => {
            if let Some(t) = crate::apps::link_target(p) {
                d["target"] = json!(t);
            }
        }
        "url" => {
            if let Ok(mut f) = std::fs::File::open(p) {
                let mut s = String::new();
                let _ = f.by_ref().take(8192).read_to_string(&mut s);
                if let Some(u) = s.lines().find_map(|l| l.strip_prefix("URL=")) {
                    d["target"] = json!(u.trim());
                }
            }
        }
        e if ARCHIVES.contains(&e) => {
            if let Some(z) = zip_listing(path) {
                d["archive"] = z;
            }
        }
        _ => {}
    }
}
