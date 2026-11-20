//! Installed apps, read from the shell's Apps folder (the same list Start uses: desktop
//! programs and Store apps), plus the Start Menu shortcuts behind them so "Open file location"
//! and "Run as administrator" work.
use crate::{fuzzy, history, icons, settings};
use once_cell::sync::Lazy;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows::core::{Interface, PCWSTR, PWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, IShellLinkW, SHGetKnownFolderItem, ShellLink, KF_FLAG_DEFAULT, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct App {
    /// AppsFolder parsing name: an AUMID for Store apps, a path or app id for desktop apps
    pub id: String,
    pub name: String,
    /// the Start Menu shortcut, when there is one
    #[serde(default)]
    pub lnk: Option<String>,
    /// what the shortcut points at
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub store: bool,
}

struct State {
    apps: Vec<App>,
    loaded: Option<Instant>,
}

static STATE: Lazy<RwLock<State>> = Lazy::new(|| {
    let apps: Vec<App> = std::fs::read_to_string(settings::data_file("apps.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    RwLock::new(State { apps, loaded: None })
});

fn from_pwstr(p: PWSTR) -> String {
    let s = unsafe { p.to_string() }.unwrap_or_default();
    unsafe { CoTaskMemFree(Some(p.0 as _)) };
    s
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Start Menu shortcuts by lowercase name.
fn start_menu_links() -> HashMap<String, PathBuf> {
    let mut out = HashMap::new();
    let roots = [
        std::env::var_os("ProgramData").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(r"Microsoft\Windows\Start Menu\Programs")),
    ];
    for root in roots.into_iter().flatten() {
        for e in walkdir::WalkDir::new(root).max_depth(4).into_iter().flatten() {
            let p = e.path();
            if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
                if let Some(stem) = p.file_stem() {
                    out.insert(stem.to_string_lossy().to_lowercase(), p.to_path_buf());
                }
            }
        }
    }
    out
}

/// Where a .lnk points.
pub fn link_target(lnk: &Path) -> Option<String> {
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let w = wide(&lnk.to_string_lossy());
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..end]);
        (!s.is_empty()).then_some(s)
    }
}

fn enumerate() -> Result<Vec<App>, String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let folder: IShellItem = SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None).map_err(|e| e.to_string())?;
        let items: IEnumShellItems = folder.BindToHandler(None, &BHID_EnumItems).map_err(|e| e.to_string())?;
        let links = start_menu_links();
        let mut out = Vec::new();
        loop {
            let mut buf = [None];
            let mut got = 0u32;
            if items.Next(&mut buf, Some(&mut got)).is_err() || got == 0 {
                break;
            }
            let Some(item) = buf[0].take() else { break };
            let name = item.GetDisplayName(SIGDN_NORMALDISPLAY).map(from_pwstr).unwrap_or_default();
            let id = item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).map(from_pwstr).unwrap_or_default();
            if name.is_empty() || id.is_empty() {
                continue;
            }
            let lower = name.to_lowercase();
            if lower.starts_with("uninstall") || lower.contains(" uninstall") || id.starts_with("http") {
                continue;
            }
            let lnk = links.get(&lower).cloned();
            let target = lnk.as_deref().and_then(link_target);
            let store = id.contains('!') && !id.contains('\\');
            out.push(App { id, name, lnk: lnk.map(|p| p.to_string_lossy().into_owned()), target, store });
        }
        out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }
}

/// Re-read the installed apps (in the background, at most every few minutes unless forced).
pub fn refresh(force: bool) {
    {
        let st = STATE.read();
        if !force {
            if let Some(t) = st.loaded {
                if t.elapsed().as_secs() < 300 {
                    return;
                }
            }
        }
    }
    STATE.write().loaded = Some(Instant::now());
    std::thread::spawn(|| match enumerate() {
        Ok(apps) => {
            if let Ok(t) = serde_json::to_string(&apps) {
                let _ = settings::write_atomic(&settings::data_file("apps.json"), t.as_bytes());
            }
            for a in &apps {
                icons::prefetch_app(&a.id);
            }
            STATE.write().apps = apps;
        }
        Err(e) => eprintln!("apps: {e}"),
    });
}

pub fn key(a: &App) -> String {
    format!("app:{}", a.id)
}

fn subtitle(a: &App) -> String {
    if a.store {
        return "Store app".into();
    }
    // the folder it lives in usually names the vendor or product: "App · Microsoft VS Code"
    let folder = a
        .target
        .as_deref()
        .and_then(|t| Path::new(t).parent())
        .and_then(|d| d.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !matches!(n.to_lowercase().as_str(), "bin" | "app" | "system32" | "syswow64" | "windows"));
    match folder {
        Some(f) if !f.eq_ignore_ascii_case(&a.name) => format!("App · {f}"),
        _ => "App".into(),
    }
}

/// Rows for the page.
pub fn to_item(a: &App, score: i32) -> serde_json::Value {
    serde_json::json!({
        "key": key(a), "kind": "app", "title": a.name,
        "subtitle": subtitle(a),
        "icon": icons::app_url(&a.id), "id": a.id, "path": a.target, "lnk": a.lnk, "store": a.store, "score": score,
    })
}

const MINOR: &[&str] = &["readme", "read me", "help", "website", "documentation", "release notes", "license", "manual", "faq", "changelog", "support"];

pub fn search(q: &str, limit: usize) -> Vec<(i32, serde_json::Value)> {
    let st = STATE.read();
    let mut out: Vec<(i32, serde_json::Value)> = st
        .apps
        .iter()
        .filter_map(|a| {
            // the exe name also counts: "code" finds Visual Studio Code via Code.exe
            let by_exe = a
                .target
                .as_deref()
                .and_then(|t| Path::new(t).file_stem().map(|s| s.to_string_lossy().into_owned()))
                .and_then(|stem| fuzzy::score(q, &stem))
                .map(|s| s - 40);
            let mut s = match (fuzzy::score(q, &a.name), by_exe) {
                (Some(x), Some(y)) => x.max(y),
                (Some(x), None) | (None, Some(x)) => x,
                (None, None) => return None,
            };
            let lower = a.name.to_lowercase();
            if MINOR.iter().any(|m| lower.contains(m)) {
                s -= 250;
            }
            let k = key(a);
            s += history::boost(&k, q);
            Some((s, a))
        })
        .map(|(s, a)| (s, to_item(a, s)))
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

pub fn count() -> usize {
    STATE.read().apps.len()
}
