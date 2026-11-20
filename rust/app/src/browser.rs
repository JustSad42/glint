//! Chromium browsers (Chrome, Edge, Brave): bookmarks, history and open tabs.
//! Bookmarks and history are read from the profile files (history opened read-only without
//! locking, so the browser never notices). Open tabs come from UI Automation on the tab strip,
//! which is also how a tab gets selected.
use crate::{fuzzy, icons, wins};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationSelectionItemPattern, TreeScope_Children, TreeScope_Descendants, UIA_ClassNamePropertyId, UIA_ControlTypePropertyId,
    UIA_SelectionItemPatternId, UIA_TabControlTypeId, UIA_TabItemControlTypeId,
};

struct Browser {
    name: &'static str,
    exe: &'static str,
    dir: &'static [&'static str],
}

const BROWSERS: &[Browser] = &[
    Browser { name: "Chrome", exe: "chrome.exe", dir: &["Google", "Chrome", "User Data"] },
    Browser { name: "Edge", exe: "msedge.exe", dir: &["Microsoft", "Edge", "User Data"] },
    Browser { name: "Brave", exe: "brave.exe", dir: &["BraveSoftware", "Brave-Browser", "User Data"] },
];

#[derive(Clone)]
struct Link {
    title: String,
    url: String,
    browser: &'static str,
    visits: i64,
    bookmark: bool,
}

struct Cache {
    at: Option<Instant>,
    links: Vec<Link>,
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache { at: None, links: vec![] }));
static LOADING: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

fn profiles() -> Vec<(&'static Browser, PathBuf)> {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else { return vec![] };
    let mut out = Vec::new();
    for b in BROWSERS {
        let base = b.dir.iter().fold(local.clone(), |p, d| p.join(d));
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if (n == "Default" || n.starts_with("Profile ")) && e.path().join("Preferences").exists() {
                out.push((b, e.path()));
            }
        }
    }
    out
}

fn walk_bookmarks(v: &Value, browser: &'static str, out: &mut Vec<Link>) {
    if v["type"] == "url" {
        if let (Some(t), Some(u)) = (v["name"].as_str(), v["url"].as_str()) {
            if u.starts_with("http") {
                out.push(Link { title: t.to_string(), url: u.to_string(), browser, visits: 0, bookmark: true });
            }
        }
    }
    if let Some(kids) = v["children"].as_array() {
        for k in kids {
            walk_bookmarks(k, browser, out);
        }
    }
}

fn history(path: &Path, browser: &'static str, out: &mut Vec<Link>) {
    // immutable=1: read without taking SQLite locks (the browser keeps the file open)
    let uri = format!("file:{}?immutable=1", path.to_string_lossy().replace('\\', "/").replace(' ', "%20").replace('#', "%23"));
    let Ok(db) = rusqlite::Connection::open_with_flags(&uri, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI) else { return };
    let Ok(mut st) = db.prepare("SELECT url, title, visit_count FROM urls WHERE hidden = 0 AND title != '' ORDER BY visit_count DESC, last_visit_time DESC LIMIT 6000") else { return };
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)));
    if let Ok(rows) = rows {
        for (url, title, visits) in rows.flatten() {
            if url.starts_with("http") {
                out.push(Link { title, url, browser, visits, bookmark: false });
            }
        }
    }
}

fn load(with_history: bool) -> Vec<Link> {
    let mut links = Vec::new();
    for (b, dir) in profiles() {
        if let Ok(s) = std::fs::read_to_string(dir.join("Bookmarks")) {
            if let Ok(v) = serde_json::from_str::<Value>(&s) {
                if let Some(roots) = v["roots"].as_object() {
                    for r in roots.values() {
                        walk_bookmarks(r, b.name, &mut links);
                    }
                }
            }
        }
        if with_history {
            history(&dir.join("History"), b.name, &mut links);
        }
    }
    // one row per URL: a bookmark wins over the same page in history, and keeps its visits
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut out: Vec<Link> = Vec::new();
    for l in links {
        let k = l.url.trim_end_matches('/').to_lowercase();
        match seen.get(&k) {
            Some(&i) => {
                out[i].visits = out[i].visits.max(l.visits);
                out[i].bookmark |= l.bookmark;
            }
            None => {
                seen.insert(k, out.len());
                out.push(l);
            }
        }
    }
    // the same page title on one site, under several URLs (tracking parameters): keep the
    // most visited, and any bookmark
    out.sort_by(|a, b| b.bookmark.cmp(&a.bookmark).then(b.visits.cmp(&a.visits)));
    let mut titles = std::collections::HashSet::new();
    out.retain(|l| l.title.is_empty() || titles.insert(format!("{}|{}", l.title.to_lowercase(), domain(&l.url))));
    out
}

/// Re-read bookmarks/history in the background when the copy is older than a few minutes.
pub fn refresh(with_history: bool) {
    let stale = CACHE.lock().at.map(|t| t.elapsed() > Duration::from_secs(180)).unwrap_or(true);
    if !stale {
        return;
    }
    {
        let mut l = LOADING.lock();
        if *l {
            return;
        }
        *l = true;
    }
    std::thread::spawn(move || {
        let links = load(with_history);
        *CACHE.lock() = Cache { at: Some(Instant::now()), links };
        *LOADING.lock() = false;
    });
}

fn domain(url: &str) -> &str {
    let s = url.split("://").nth(1).unwrap_or(url);
    let s = s.split('/').next().unwrap_or(s);
    s.strip_prefix("www.").unwrap_or(s)
}

pub fn search_links(q: &str, limit: usize, bookmarks_only: bool) -> Vec<(i32, Value)> {
    let ql = q.trim().to_lowercase();
    if ql.chars().count() < 2 {
        return vec![];
    }
    let c = CACHE.lock();
    let mut out: Vec<(i32, Value)> = c
        .links
        .iter()
        .filter(|l| !bookmarks_only || l.bookmark)
        .filter_map(|l| {
            let by_title = fuzzy::score(&ql, &l.title);
            let d = domain(&l.url);
            let by_domain = fuzzy::score(&ql, d).map(|s| s - 40);
            let by_url = (ql.len() >= 3 && l.url.to_lowercase().contains(&ql)).then_some(420);
            let s = by_title.max(by_domain).max(by_url)?;
            if s < 400 {
                return None;
            }
            let s = s + if l.bookmark { 120 } else { 0 } + ((l.visits as f64 + 1.0).ln() * 22.0) as i32;
            Some((
                s,
                json!({
                    "key": format!("link:{}", l.url), "kind": "url", "title": if l.title.is_empty() { d.to_string() } else { l.title.clone() },
                    "subtitle": format!("{} · {}{}", if l.bookmark { "Bookmark" } else { "History" }, d, if l.browser != "Chrome" { format!(" · {}", l.browser) } else { String::new() }),
                    "glyph": if l.bookmark { "star" } else { "history" }, "url": l.url, "score": s,
                }),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

// ------------------------------------------------------------------------------------------
// open tabs

#[derive(Clone)]
struct Tab {
    hwnd: isize,
    title: String,
    pos: usize,
    exe: String,
}

static TABS: Lazy<Mutex<(Option<Instant>, Vec<Tab>)>> = Lazy::new(|| Mutex::new((None, vec![])));

fn uia() -> Option<IUIAutomation> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    }
}

/// The tab items of one browser window. Finds the tab strip first, so the web page itself is
/// never walked (that would make the browser build its accessibility tree).
fn tab_items(a: &IUIAutomation, hwnd: isize) -> Vec<IUIAutomationElement> {
    let mut out = Vec::new();
    unsafe {
        let Ok(win) = a.ElementFromHandle(HWND(hwnd as *mut _)) else { return out };
        let Ok(strip_cond) = a.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_TabControlTypeId.0)) else { return out };
        let Ok(item_cond) = a.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_TabItemControlTypeId.0)) else { return out };
        // the browser's own tab strip (class TabContainer), not a tab list inside a web page
        let Ok(class_cond) = a.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from(windows::core::BSTR::from("TabContainer"))) else { return out };
        let Ok(both) = a.CreateAndCondition(&strip_cond, &class_cond) else { return out };
        let Ok(strip) = win.FindFirst(TreeScope_Descendants, &both) else { return out };
        // tabs sit directly in the strip, or one level down in tab groups
        let grab = |arr: windows::Win32::UI::Accessibility::IUIAutomationElementArray, out: &mut Vec<IUIAutomationElement>| {
            for i in 0..arr.Length().unwrap_or(0) {
                if let Ok(e) = arr.GetElement(i) {
                    out.push(e);
                }
            }
        };
        if let Ok(arr) = strip.FindAll(TreeScope_Children, &item_cond) {
            grab(arr, &mut out);
        }
        if out.is_empty() {
            if let Ok(arr) = strip.FindAll(TreeScope_Descendants, &item_cond) {
                grab(arr, &mut out);
            }
        }
    }
    out
}

/// Chrome adds " - Memory usage - 146 MB" and similar to a tab's name.
fn clean_title(name: &str) -> String {
    let mut t = name.to_string();
    for cut in [" - Memory usage - ", " - High memory usage - "] {
        if let Some(i) = t.find(cut) {
            t.truncate(i);
        }
    }
    for suffix in [" - Audio playing", " - Muted", " - Network error", " - Pinned"] {
        while let Some(x) = t.strip_suffix(suffix) {
            t = x.to_string();
        }
    }
    t
}

fn is_browser(exe: &str) -> bool {
    let e = exe.to_lowercase();
    BROWSERS.iter().any(|b| e.ends_with(&format!("\\{}", b.exe)))
}

fn list_tabs() -> Vec<Tab> {
    {
        let t = TABS.lock();
        if t.0.map(|x| x.elapsed() < Duration::from_secs(4)).unwrap_or(false) {
            return t.1.clone();
        }
    }
    let Some(a) = uia() else { return vec![] };
    let mut out = Vec::new();
    for w in wins::list().into_iter().filter(|w| is_browser(&w.exe)) {
        for (pos, e) in tab_items(&a, w.hwnd).into_iter().enumerate() {
            let name = clean_title(&unsafe { e.CurrentName().map(|b| b.to_string()).unwrap_or_default() });
            if !name.is_empty() {
                out.push(Tab { hwnd: w.hwnd, title: name, pos, exe: w.exe.clone() });
            }
        }
    }
    *TABS.lock() = (Some(Instant::now()), out.clone());
    out
}

/// Warm the tab list (UI Automation takes a moment per window).
pub fn prefetch_tabs() {
    std::thread::spawn(|| {
        let _ = list_tabs();
    });
}

pub fn search_tabs(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let ql = q.trim();
    let mut out: Vec<(i32, Value)> = list_tabs()
        .into_iter()
        .filter_map(|t| {
            // nothing typed ("tabs"): every tab; otherwise an open tab beats the same page in history
            let s = if ql.is_empty() { 500 } else { fuzzy::score(ql, &t.title)? + 300 };
            if s < 700 && !ql.is_empty() {
                return None;
            }
            let browser = Path::new(&t.exe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            Some((
                s + 60,
                json!({
                    "key": format!("tab:{}:{}", t.hwnd, t.title), "kind": "tab", "title": t.title,
                    "subtitle": format!("Open tab · {}", match browser.as_str() { "msedge" => "Edge", "chrome" => "Chrome", "brave" => "Brave", b => b }),
                    "icon": icons::path_url(&t.exe), "hwnd": t.hwnd, "pos": t.pos, "path": t.exe, "score": s + 60,
                }),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

/// Bring the browser window forward and select the tab.
pub fn activate_tab(hwnd: isize, title: &str, pos: usize) -> Result<(), String> {
    wins::activate(hwnd);
    let a = uia().ok_or("UI Automation isn't available")?;
    let items = tab_items(&a, hwnd);
    let named = |e: &IUIAutomationElement| clean_title(&unsafe { e.CurrentName().map(|b| b.to_string()).unwrap_or_default() });
    let target = items.iter().find(|e| named(e) == title).or_else(|| items.get(pos)).ok_or("That tab is gone")?;
    unsafe {
        let p: IUIAutomationSelectionItemPattern = target.GetCurrentPatternAs(UIA_SelectionItemPatternId).map_err(|e| e.to_string())?;
        p.Select().map_err(|e| e.to_string())?;
    }
    *TABS.lock() = (None, vec![]);
    Ok(())
}
