//! One keystroke -> ranked sections: calculator, apps, windows, commands, files, file contents,
//! web. Special prefixes switch modes:
//!   >cmd      run a command in PowerShell        kill name   end a process
//! > ?words    search the web                     C:\path     browse folders (Tab completes)
use crate::{actions, apps, audio, browser, calc, clipvault, content, files, history, icons, media, pins, preview, quick, scripts, settings, shellops, sysinfo, timers, winmgmt, wins};
use glint_core::query::Kind;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Instant;

pub fn web_url(engine: &str, q: &str) -> String {
    let e = urlencoding::encode(q);
    match engine {
        "duckduckgo" => format!("https://duckduckgo.com/?q={e}"),
        "bing" => format!("https://www.bing.com/search?q={e}"),
        "brave" => format!("https://search.brave.com/search?q={e}"),
        "kagi" => format!("https://kagi.com/search?q={e}"),
        "perplexity" => format!("https://www.perplexity.ai/search?q={e}"),
        _ => format!("https://www.google.com/search?q={e}"),
    }
}

fn engine_name(engine: &str) -> &'static str {
    match engine {
        "duckduckgo" => "DuckDuckGo",
        "bing" => "Bing",
        "brave" => "Brave",
        "kagi" => "Kagi",
        "perplexity" => "Perplexity",
        _ => "Google",
    }
}

fn web_item(q: &str) -> Value {
    let s = settings::load();
    json!({
        "key": "web", "kind": "web", "title": format!("Search {} for “{}”", engine_name(&s.web_engine), q.trim()),
        "subtitle": "Opens in your browser", "glyph": "search", "url": web_url(&s.web_engine, q.trim()), "score": 0,
    })
}

const TLDS: &[&str] = &[
    "com", "net", "org", "io", "dev", "app", "ai", "gg", "tv", "me", "co", "uk", "de", "fr", "nl", "ca", "us", "info", "xyz", "edu", "gov", "ly", "to", "sh", "so",
    "be", "eu", "ru", "jp", "in", "it", "es", "pl", "se", "ch", "at", "au", "br", "kr", "fm", "gl", "cc", "page", "site", "tech", "online", "store", "blog",
    "news", "wiki", "games", "art", "moe",
];

/// "github.com/x", "https://...", "localhost:3000" -> a URL to open.
fn as_url(input: &str) -> Option<String> {
    let s = input.trim();
    if s.contains(char::is_whitespace) || s.len() < 4 {
        return None;
    }
    let lower = s.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(s.to_string());
    }
    if lower.starts_with("localhost:") || lower.starts_with("127.0.0.1:") {
        return Some(format!("http://{s}"));
    }
    let host = lower.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() < 2 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) {
        return None;
    }
    let tld = parts.last().unwrap();
    if !TLDS.contains(tld) {
        return None;
    }
    // "notes.md" style file names aren't URLs, but "x.io" and "www.x.co" are
    if parts.len() == 2 && parts[0].len() < 2 {
        return None;
    }
    Some(format!("https://{s}"))
}

/// "#ff8800", "#f80", "rgb(255, 136, 0)" -> conversions.
fn color(input: &str) -> Option<Value> {
    let s = input.trim().to_lowercase();
    let (r, g, b) = if let Some(h) = s.strip_prefix('#') {
        let h: String = h.chars().collect();
        if !h.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        match h.len() {
            3 => {
                let v: Vec<u8> = h.chars().map(|c| u8::from_str_radix(&c.to_string().repeat(2), 16).unwrap()).collect();
                (v[0], v[1], v[2])
            }
            6 | 8 => (u8::from_str_radix(&h[0..2], 16).ok()?, u8::from_str_radix(&h[2..4], 16).ok()?, u8::from_str_radix(&h[4..6], 16).ok()?),
            _ => return None,
        }
    } else {
        let inner = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")).and_then(|x| x.strip_suffix(')'))?;
        let v: Vec<u8> = inner.split([',', ' ', '/']).filter(|x| !x.is_empty()).take(3).filter_map(|x| x.trim().parse().ok()).collect();
        if v.len() != 3 {
            return None;
        }
        (v[0], v[1], v[2])
    };
    let hex = format!("#{r:02X}{g:02X}{b:02X}");
    let (rf, gf, bf) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    let d = max - min;
    let (h, sat) = if d == 0.0 {
        (0.0, 0.0)
    } else {
        let sat = d / (1.0 - (2.0 * l - 1.0).abs());
        let h = if max == rf {
            60.0 * (((gf - bf) / d) % 6.0)
        } else if max == gf {
            60.0 * ((bf - rf) / d + 2.0)
        } else {
            60.0 * ((rf - gf) / d + 4.0)
        };
        ((h + 360.0) % 360.0, sat)
    };
    let rgb = format!("rgb({r}, {g}, {b})");
    let hsl = format!("hsl({:.0}, {:.0}%, {:.0}%)", h, sat * 100.0, l * 100.0);
    Some(json!({
        "key": "color", "kind": "color", "title": hex, "subtitle": format!("{rgb} · {hsl} · Enter copies"), "swatch": hex,
        "copy": hex, "alts": [hex, rgb, hsl], "score": 2000,
    }))
}

fn looks_like_path(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/'))
        || s.starts_with("\\\\")
        || s.starts_with('~')
        || (s.starts_with('%') && s[1..].contains('%'))
}

/// Folder browsing: list what's in the typed folder, filtered by the part after the last slash.
fn browse(input: &str) -> Vec<Value> {
    let full = shellops::expand_path(input);
    let full = if full.len() == 2 && full.ends_with(':') { format!("{full}\\") } else { full };
    let (dir, part) = if full.ends_with('\\') {
        (full.clone(), String::new())
    } else {
        match full.rfind('\\') {
            Some(i) => (full[..=i].to_string(), full[i + 1..].to_lowercase()),
            None => return vec![],
        }
    };
    let mut out = Vec::new();
    let typed = Path::new(&full);
    if !part.is_empty() && typed.exists() {
        let mut it = files::item(&full, typed.is_dir(), 3000);
        it["title"] = json!(format!("Open {}", typed.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(full.clone())));
        it["subtitle"] = json!(full.clone());
        out.push(it);
    }
    let Ok(rd) = std::fs::read_dir(&dir) else { return out };
    let mut entries: Vec<(bool, String, String)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let lower = name.to_lowercase();
            if !part.is_empty() && !lower.contains(&part) {
                return None;
            }
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            Some((is_dir, name, e.path().to_string_lossy().into_owned()))
        })
        .collect();
    entries.sort_by(|a, b| {
        let pa = a.1.to_lowercase().starts_with(&part);
        let pb = b.1.to_lowercase().starts_with(&part);
        pb.cmp(&pa).then(b.0.cmp(&a.0)).then(a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    for (is_dir, _, path) in entries.into_iter().take(60) {
        let mut it = files::item(&path, is_dir, 1000);
        if is_dir {
            it["complete"] = json!(format!("{path}\\"));
        }
        out.push(it);
    }
    out
}

fn section(title: &str, items: Vec<Value>) -> Value {
    json!({"title": title, "items": items})
}

fn strip(v: Vec<(i32, Value)>) -> Vec<Value> {
    v.into_iter().map(|(_, x)| x).collect()
}

/// Search everything. `filter`: all | apps | files | folders | contents | windows | actions
pub fn run(input: &str, filter: &str) -> Value {
    let t = Instant::now();
    let s = settings::load();
    let q = input.trim();
    let mut sections: Vec<(i32, Value)> = Vec::new();
    let mut live = true;

    if q.is_empty() {
        let recent: Vec<Value> = history::recent(8)
            .into_iter()
            .filter_map(|(_, u)| {
                let mut it = u.item;
                // drop files that are gone
                if let Some(p) = it["path"].as_str() {
                    // network files are kept unchecked: asking the server would hold up the screen
                    if matches!(it["kind"].as_str(), Some("file" | "folder" | "content")) && !files::is_remote(p) && !Path::new(p).exists() {
                        return None;
                    }
                }
                it["recent"] = json!(true);
                Some(it)
            })
            .collect();
        let pinned: Vec<Value> = pins::pinned()
            .into_iter()
            .map(|mut it| {
                it["pinned"] = json!(true);
                it
            })
            .collect();
        if !pinned.is_empty() && filter == "all" {
            sections.push((0, section("Pinned", pinned)));
        }
        if s.media_controls && filter == "all" {
            let m = media::rows("", true);
            if !m.is_empty() {
                sections.push((0, section("Now playing", m)));
            }
        }
        if filter == "clipboard" {
            let c = strip(clipvault::search("", 60));
            return json!({"sections": [section("Clipboard history", c)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "clip"});
        }
        let tm = timers::active_rows();
        if !tm.is_empty() && filter == "all" {
            sections.push((0, section("Timers", tm)));
        }
        let recent_paths: Vec<String> = recent.iter().filter_map(|r| r["path"].as_str().map(|p| p.to_lowercase())).collect();
        if !recent.is_empty() {
            sections.push((0, section("Recent", recent)));
        }
        if filter == "all" || filter == "files" {
            let rf: Vec<Value> = quick::recent_files(6).into_iter().filter(|f| !recent_paths.contains(&f["path"].as_str().unwrap_or("").to_lowercase())).take(4).collect();
            if !rf.is_empty() {
                sections.push((0, section("Recent files", rf)));
            }
        }
        if s.browser_search {
            browser::refresh(s.browser_history);
            browser::prefetch_tabs();
        }
        let w = strip(wins::search("", 5));
        if !w.is_empty() && filter == "all" {
            sections.push((0, section("Open windows", w)));
        }
        return json!({"sections": sections.into_iter().map(|(_, v)| v).collect::<Vec<_>>(), "ms": t.elapsed().as_millis() as u64, "live": true});
    }

    // modes
    if let Some(cmd) = q.strip_prefix('>') {
        let cmd = cmd.trim();
        let items = if cmd.is_empty() {
            vec![json!({"key":"cmd","kind":"hint","title":"Type a command to run in PowerShell","subtitle":"Enter runs it · Shift+Enter runs it as administrator","glyph":"terminal"})]
        } else {
            vec![json!({"key": format!("cmd:{cmd}"), "kind": "command", "title": format!("Run: {cmd}"), "subtitle": "PowerShell · Shift+Enter runs as administrator", "glyph": "terminal", "command": cmd})]
        };
        return json!({"sections": [section("Command", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "command"});
    }
    if let Some(rest) = q.strip_prefix("clip ").or_else(|| q.strip_prefix("cb ")).or_else(|| (filter == "clipboard").then_some(q)) {
        let items = strip(clipvault::search(rest, 60));
        return json!({"sections": [section("Clipboard history", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "clip"});
    }
    if let Some(rest) = q.strip_prefix("biggest").filter(|r| r.is_empty() || r.starts_with(' ')) {
        return biggest(rest.trim(), t);
    }
    if s.browser_search {
        if let Some(rest) = q.strip_prefix("tab ").or_else(|| (q == "tabs").then_some("")) {
            let items = strip(browser::search_tabs(rest, 60));
            return json!({"sections": [section("Open tabs", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "tabs"});
        }
        if let Some(rest) = q.strip_prefix("bm ") {
            browser::refresh(s.browser_history);
            let items = strip(browser::search_links(rest, 30, true));
            return json!({"sections": [section("Bookmarks", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "bookmarks"});
        }
    }
    if let Some(rest) = q.strip_prefix("kill ").or_else(|| (q == "kill").then_some("")) {
        let items = strip(wins::search_processes(rest, 30));
        return json!({"sections": [section("Processes", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "kill"});
    }
    if let Some(rest) = q.strip_prefix('?') {
        let items = if rest.trim().is_empty() { vec![] } else { vec![web_item(rest)] };
        return json!({"sections": [section("Web", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "web"});
    }
    if looks_like_path(q) {
        let items = browse(q);
        return json!({"sections": [section("Folder", items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "path"});
    }

    let all = filter == "all";
    // a calculation or color code isn't something to look for in files
    let mut special = false;
    if all {
        let kw = pins::keyword_hits(q);
        if !kw.is_empty() {
            sections.push((200_000, section("Keyword", kw)));
        }
        let mut quick = audio::rows(q);
        quick.extend(sysinfo::rows(q));
        quick.extend(timers::rows(q));
        quick.extend(quick::rows(q));
        quick.sort_by(|a, b| b["score"].as_i64().unwrap_or(0).cmp(&a["score"].as_i64().unwrap_or(0)));
        if !quick.is_empty() {
            special = quick.iter().any(|r| r["score"].as_i64().unwrap_or(0) >= 5000);
            sections.push((150_000, section("Quick", quick)));
        }
        if s.media_controls {
            let m = media::rows(q, false);
            if !m.is_empty() {
                sections.push((140_000, section("Media", m)));
            }
        }
        if let Some(c) = calc::evaluate(q) {
            special = true;
            sections.push((
                100_000,
                section(
                    "Calculator",
                    vec![json!({"key":"calc","kind":"calc","title":c.display,"subtitle":format!("{} · Enter copies", c.caption),"copy":c.copy,"glyph":"calc","score":2000})],
                ),
            ));
        }
        if let Some(c) = color(q) {
            special = true;
            sections.push((99_000, section("Color", vec![c])));
        }
        if let Some(url) = as_url(q) {
            sections.push((
                98_000,
                section("Website", vec![json!({"key": format!("url:{url}"), "kind":"url", "title": format!("Open {url}"), "subtitle":"In your browser","glyph":"globe","url":url,"score":1500})]),
            ));
        }
    }

    let mut push = |title: &str, offset: i32, mut v: Vec<(i32, Value)>| {
        // with a strong match on top, far weaker ones only add noise
        if let Some(top) = v.first().map(|x| x.0) {
            if top >= 700 {
                v.retain(|x| x.0 >= top - 520);
            }
        }
        if let Some(top) = v.first().map(|x| x.0) {
            sections.push((top + offset, section(title, strip(v))));
        }
    };
    if all || filter == "apps" {
        push("Apps", 0, apps::search(q, if all { 5 } else { 30 }));
    }
    if all || filter == "windows" {
        push("Windows", -20, wins::search(q, if all { 4 } else { 30 }));
    }
    if all || filter == "actions" {
        push("Commands", -40, actions::search(q, if all { 4 } else { 30 }));
        push("Window layout", -60, winmgmt::search(q, if all { 3 } else { 20 }));
    }
    if all || filter == "actions" {
        push("Your scripts", -30, scripts::search(q, if all { 3 } else { 30 }));
    }
    if all && s.browser_search && q.chars().count() >= 2 {
        browser::refresh(s.browser_history);
        let mut b = browser::search_tabs(q, 3);
        b.extend(browser::search_links(q, 3, false));
        b.sort_by(|a, c| c.0.cmp(&a.0));
        b.truncate(4);
        push("Browser", -200, b);
    }
    if all && s.clipvault && q.chars().count() >= 3 && clipvault::available() {
        push("Clipboard history", -260, clipvault::search(q, 3));
    }
    if !special && (all || filter == "files" || filter == "folders") {
        let kind = match filter {
            "folders" => Some(Kind::Dir),
            "files" => Some(Kind::File),
            _ => None,
        };
        let (v, l) = files::search(q, kind, if all { s.max_files.clamp(3, 20) } else { 40 });
        live = l;
        push(if filter == "folders" { "Folders" } else { "Files & folders" }, 0, v);
    }
    // date/size filters only apply to names; the words around them still search contents
    let words: String = q
        .split_whitespace()
        .filter(|w| !["dm:", "modified:", "date:", "size:"].iter().any(|p| w.to_lowercase().starts_with(p)))
        .collect::<Vec<_>>()
        .join(" ");
    if !special && !words.is_empty() && s.content_search && (all || filter == "contents") {
        let v = content::search(&words, if all { 5 } else { 30 });
        push("In file contents", -150, v);
    }
    // pinned items jump to the top of their section, and their section moves up with them
    for (best, sec) in sections.iter_mut() {
        if let Some(items) = sec["items"].as_array_mut() {
            let mut bumped = false;
            for it in items.iter_mut() {
                if pins::is_pinned(it["key"].as_str().unwrap_or("")) {
                    let sc = it["score"].as_i64().unwrap_or(0) + 1500;
                    it["score"] = json!(sc);
                    it["pinned"] = json!(true);
                    *best = (*best).max(sc as i32);
                    bumped = true;
                }
            }
            if bumped {
                items.sort_by(|a, b| b["pinned"].as_bool().unwrap_or(false).cmp(&a["pinned"].as_bool().unwrap_or(false)));
            }
        }
    }
    sections.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out: Vec<Value> = sections.into_iter().map(|(_, v)| v).collect();
    // a content hit for a file already listed by name adds nothing
    dedupe(&mut out);
    if all && s.web_fallback {
        out.push(section("Web", vec![web_item(q)]));
    }
    json!({"sections": out, "ms": t.elapsed().as_millis() as u64, "live": live})
}

fn dedupe(sections: &mut [Value]) {
    let mut seen = std::collections::HashSet::new();
    for s in sections.iter_mut() {
        if let Some(items) = s["items"].as_array_mut() {
            items.retain(|it| {
                let k = it["key"].as_str().unwrap_or("").to_string();
                k.is_empty() || seen.insert(k)
            });
        }
    }
}

/// Details for the preview pane.
pub fn details(item: &Value) -> Value {
    let kind = item["kind"].as_str().unwrap_or("");
    let path = item["path"].as_str().unwrap_or("");
    let mut d = json!({"kind": kind});
    if matches!(kind, "file" | "folder" | "content") && !path.is_empty() {
        let p = Path::new(path);
        if let Ok(m) = std::fs::metadata(p) {
            d["size"] = json!(m.len());
            d["modified"] = json!(m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()));
            d["created"] = json!(m.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()));
            d["is_dir"] = json!(m.is_dir());
        } else {
            d["missing"] = json!(true);
        }
        d["path"] = json!(path);
        d["pretty"] = json!(files::pretty_dir(path));
        preview::extra(path, &mut d);
        if kind == "folder" {
            if let Ok(r) = files::children(path, 0) {
                d["total"] = json!(r.total);
            }
        }
        let ext = crate::extract::ext_of(p);
        const THUMBS: &[&str] = &[
            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "avif", "ico", "mp4", "mkv", "mov", "avi", "webm", "wmv", "pdf", "psd", "svg", "jfif",
        ];
        if THUMBS.contains(&ext.as_str()) {
            d["thumb"] = json!(icons::thumb_url(path));
        }
        if p.is_dir() {
            if let Ok(rd) = std::fs::read_dir(p) {
                let mut names: Vec<(bool, String)> =
                    rd.flatten().take(400).map(|e| (e.file_type().map(|t| t.is_dir()).unwrap_or(false), e.file_name().to_string_lossy().into_owned())).collect();
                d["count"] = json!(names.len());
                names.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.to_lowercase().cmp(&b.1.to_lowercase())));
                d["children"] = json!(names.into_iter().take(14).map(|(dir, n)| json!({"name": n, "dir": dir})).collect::<Vec<_>>());
            }
        } else if kind == "content" {
            let terms: Vec<String> = item["terms"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            d["snippet"] = content::snippet(path, &terms);
        } else if crate::extract::TEXT_EXTS.contains(&ext.as_str()) && !crate::extract::is_secret(p) {
            if let Ok(bytes) = std::fs::read(p).map(|b| b.into_iter().take(6000).collect::<Vec<u8>>()) {
                if !bytes.iter().take(2000).any(|&b| b == 0) {
                    let text = String::from_utf8_lossy(&bytes);
                    d["text"] = json!(text.lines().take(40).collect::<Vec<_>>().join("\n"));
                }
            }
        }
    } else if kind == "app" {
        d["path"] = item["path"].clone();
        d["lnk"] = item["lnk"].clone();
        if let Some(id) = item["id"].as_str() {
            d["id"] = json!(id);
        }
    } else if kind == "window" || kind == "process" {
        d["path"] = item["path"].clone();
    } else if kind == "clip" {
        if let Some(img) = item["image"].as_str() {
            d["image"] = json!(img);
            d["media"] = json!("image");
        }
    }
    let key = item["key"].as_str().unwrap_or("");
    d["pinned"] = json!(pins::is_pinned(key));
    d["keywords"] = json!(pins::keywords_for(key));
    d["hotkey"] = json!(pins::hotkey_for(key));
    d
}

/// "biggest [folder]": what takes the space, from the indexer's folder totals.
fn biggest(arg: &str, t: Instant) -> Value {
    let home = settings::user_home().to_string_lossy().into_owned();
    let mut path = if arg.is_empty() { "C:\\".to_string() } else if let Some(r) = arg.strip_prefix('~') { format!("{home}{r}") } else { arg.to_string() };
    if path.len() == 2 && path.ends_with(':') {
        path.push('\\');
    }
    let done = |title: String, items: Vec<Value>| json!({"sections": [section(&title, items)], "ms": t.elapsed().as_millis() as u64, "live": true, "mode": "biggest"});
    match files::children(&path, 40) {
        Err(e) => done(
            "Biggest".into(),
            vec![json!({"key": "big:err", "kind": "hint", "title": e, "subtitle": "Type a folder: biggest C:\\Users · biggest ~\\Downloads", "glyph": "hdd"})],
        ),
        Ok(r) => {
            let Some(total) = r.total else {
                return done("Biggest".into(), vec![json!({"key": "big:wait", "kind": "hint", "title": "Still adding up sizes…", "subtitle": "The indexer finishes this a minute after it starts", "glyph": "hdd"})]);
            };
            let items: Vec<Value> = r
                .items
                .into_iter()
                .filter(|h| h.size.unwrap_or(0) > 0)
                .map(|h| {
                    let size = h.size.unwrap_or(0);
                    let mut it = files::item_meta(&h.path, h.dir, 0, h.mtime, Some(size));
                    it["share"] = json!(if total > 0 { size as f64 / total as f64 } else { 0.0 });
                    it["subtitle"] = json!(format!("{} · {:.1}%", files::human(size), if total > 0 { size as f64 * 100.0 / total as f64 } else { 0.0 }));
                    if h.dir {
                        it["complete"] = json!(format!("biggest {}\\", h.path));
                    }
                    it
                })
                .collect();
            done(format!("Biggest in {} · {}", files::pretty_dir(path.trim_end_matches('\\')), files::human(total)), items)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(as_url("github.com/foo").as_deref(), Some("https://github.com/foo"));
        assert_eq!(as_url("https://x.y/z").as_deref(), Some("https://x.y/z"));
        assert_eq!(as_url("localhost:3000").as_deref(), Some("http://localhost:3000"));
        assert!(as_url("notes.md").is_none());
        assert!(as_url("report.pdf").is_none());
        assert!(as_url("hello world.com").is_none());
    }

    #[test]
    fn colors_and_paths() {
        assert_eq!(color("#f80").unwrap()["title"], "#FF8800");
        assert_eq!(color("rgb(255, 136, 0)").unwrap()["title"], "#FF8800");
        assert!(color("#xyz").is_none());
        assert!(looks_like_path(r"C:\Users"));
        assert!(looks_like_path("c:"));
        assert!(looks_like_path("~\\Downloads"));
        assert!(looks_like_path("%appdata%"));
        assert!(!looks_like_path("chrome"));
    }
}
