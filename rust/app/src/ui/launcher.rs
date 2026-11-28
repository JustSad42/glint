//! The launcher: search box, filter tabs, ranked results, preview pane, action menu, Quick Look
//! and the classic Start-menu layout. A port of the old launcher.js/launcher.css.
use super::assets::{decodable, Assets};
use super::jobs::{b, n, s, truthy, Jobs, Reply};
use super::theme::{self, f, fd, fm, Font, Theme};
use super::widgets::{self, anim, cr, ease, kbd, keys, keys_width, lerp_color, shadow, Toast};
use super::fmt;
use crate::host;
use crate::settings::Settings;
use egui::text::LayoutJob;
use egui::{pos2, vec2, Color32, CornerRadius, Id, Key, Modifiers, Pos2, Rect, Sense, Stroke, StrokeKind, Ui};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub struct Cx<'a> {
    pub t: &'a Theme,
    pub a: &'a mut Assets,
    pub s: &'a Settings,
}

const FILTERS: &[(&str, &str, &str)] = &[
    ("all", "All", "sparkle"),
    ("apps", "Apps", "grid"),
    ("files", "Files", "file"),
    ("folders", "Folders", "folder"),
    ("contents", "In files", "filetext"),
    ("windows", "Windows", "window"),
    ("actions", "Commands", "bolt"),
    ("clipboard", "Clipboard", "clipboard"),
];

struct Tip {
    q: &'static str,
    icon: &'static str,
    t1: &'static str,
    t2: &'static str,
}

const TIPS: &[Tip] = &[
    Tip { q: "", icon: "grid", t1: "Apps & settings", t2: "Type a name — \"vsc\" finds VS Code" },
    Tip { q: "*.pdf ", icon: "file", t1: "Files anywhere", t2: "<code>*.pdf</code> <code>folder:</code> <code>ext:docx</code>" },
    Tip { q: "=", icon: "calc", t1: "Calculator", t2: "<code>2^10/4</code> <code>10 km to mi</code>" },
    Tip { q: ">", icon: "terminal", t1: "Run a command", t2: "<code>&gt;ipconfig</code> in PowerShell" },
    Tip { q: "kill ", icon: "activity", t1: "End a process", t2: "<code>kill chrome</code>" },
    Tip { q: "~\\", icon: "folder", t1: "Browse folders", t2: "<code>C:\\</code> <code>~\\Downloads</code>, Tab completes" },
    Tip { q: "?", icon: "globe", t1: "Web search", t2: "<code>?rust traits</code>" },
    Tip { q: "#7c6cff", icon: "palette", t1: "Colors", t2: "<code>#7c6cff</code> <code>rgb(…)</code>" },
    Tip { q: "dm:week ", icon: "clock", t1: "Recent & big files", t2: "<code>dm:today</code> <code>size:>100mb</code>" },
    Tip { q: "vol ", icon: "sliders", t1: "Quick toggles", t2: "<code>vol 40</code> <code>wifi off</code> <code>ip</code>" },
    Tip { q: "left", icon: "layout", t1: "Snap a window", t2: "<code>left</code> <code>center</code> <code>next monitor</code>" },
    Tip { q: "clip ", icon: "clipboard", t1: "Clipboard history", t2: "<code>clip invoice</code> from ClipVault" },
    Tip { q: "timer 10m ", icon: "clock", t1: "Timers & reminders", t2: "<code>timer 10m tea</code> <code>remind 3pm call</code>" },
    Tip { q: "biggest", icon: "hdd", t1: "What takes the space", t2: "<code>biggest</code> <code>biggest ~\\Downloads</code>" },
];

fn kind_glyph(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "app" => "app",
        "file" => "file",
        "folder" => "folder",
        "content" => "filetext",
        "window" => "window",
        "process" => "activity",
        "calc" => "calc",
        "web" => "search",
        "url" => "globe",
        "command" => "terminal",
        "action" => "bolt",
        "color" => "palette",
        "hint" => "info",
        "toggle" => "sliders",
        "info" => "info",
        "layout" => "layout",
        "media" => "play",
        "clip" => "clipboard",
        "timer" => "clock",
        "tab" => "globe",
        "script" => "terminal",
        _ => return None,
    })
}

fn kind_badge(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "app" => "App",
        "window" => "Window",
        "content" => "Match",
        "action" => "Command",
        "process" => "Process",
        "folder" => "Folder",
        "layout" => "Layout",
        "clip" => "Clip",
        "tab" => "Tab",
        "script" => "Script",
        "timer" => "Timer",
        _ => return None,
    })
}

const PINNABLE: &[&str] = &["app", "file", "folder", "action", "url", "layout", "toggle", "command", "script"];
const RUNNABLE: &[&str] = &["exe", "bat", "cmd", "ps1", "msi", "lnk", "com"];
const QL_IMG: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "avif"];
const QL_VID: &[&str] = &["mp4", "webm", "mov", "mkv", "m4v"];
const QL_AUD: &[&str] = &["mp3", "wav", "flac", "ogg", "m4a", "aac", "opus"];
const QL_TXT: &[&str] = &[
    "txt", "md", "json", "js", "ts", "tsx", "jsx", "rs", "py", "css", "html", "xml", "yml", "yaml", "toml", "ini", "log", "csv", "c", "cpp", "h", "hpp", "cs", "java", "go", "sh", "ps1", "bat", "cmd", "sql", "lua", "cfg", "conf", "env",
    "gitignore", "srt",
];

#[derive(Clone, Debug)]
pub struct Act {
    pub label: String,
    pub icon: String,
    pub mode: String,
    pub keys: Vec<String>,
    pub danger: bool,
}

fn act(label: &str, icon: &str, mode: &str, k: &[&str]) -> Act {
    Act { label: label.into(), icon: icon.into(), mode: mode.into(), keys: k.iter().map(|x| x.to_string()).collect(), danger: false }
}

enum MenuKind {
    List,
    Keyword(String),
    Hotkey(String),
}

struct Menu {
    items: Vec<Act>,
    sel: usize,
    item: Value,
    kind: MenuKind,
    opened: Instant,
    scroll_to_sel: bool,
}

enum QlBody {
    Loading,
    Empty(String),
    Text(String),
    /// a picture key for `Assets::image` (file:… or a thumbnail URL); bool = play badge
    Picture(String, bool),
    Audio(String),
    Info(Option<Value>),
}

struct Ql {
    key: String,
    body: QlBody,
}

enum Tag {
    Search(String),
    Details(String),
    Status,
    Queries,
    Home,
    AllApps,
    Activate(Value, String),
    Power,
    QlText(String),
    QlDetails(String),
    StartIndexer,
    Ignore,
}

pub struct Launcher {
    /// the 45 s query reset already happened while hidden
    reset_while_hidden: bool,
    /// the window is open (set by the app each pass); hidden, nothing animates
    pub visible: bool,
    /// screen cursor position last frame: hover only selects when the mouse really moves,
    /// not when the window appears under a resting cursor
    last_cursor: (i32, i32),
    jobs: Jobs<Tag>,
    pub query: String,
    filter: &'static str,
    sections: Vec<(String, Vec<Value>)>,
    flat: Vec<Value>,
    sel: usize,
    mode: Option<String>,
    live: Option<bool>,
    ms: Option<f64>,
    rendered_query: Option<String>,
    last_query: Option<String>,
    inflight: bool,
    pending: bool,
    confirm_key: Option<String>,
    menu: Option<Menu>,
    hidden_at: Option<Instant>,
    query_history: Vec<String>,
    history_idx: i32,
    status: Option<Value>,
    vscode: bool,
    details_for: Option<String>,
    details: Option<Value>,
    details_due: Option<(Instant, Value)>,
    home: Option<Value>,
    all_apps: bool,
    ql: Option<Ql>,
    toast: Option<Toast>,
    scroll_to_sel: bool,
    scroll_top: bool,
    shown_at: Option<Instant>,
    armed_power: Option<(String, Instant)>,
    select_all: bool,
    research_at: Option<Instant>,
    status_at: Option<Instant>,
    /// cover art changes with the track; drop the old one when results refresh
    cover_keys: Vec<String>,
}

impl Launcher {
    pub fn new() -> Launcher {
        Launcher {
            reset_while_hidden: false,
            visible: false,
            last_cursor: (0, 0),
            jobs: Jobs::default(),
            query: String::new(),
            filter: "all",
            sections: Vec::new(),
            flat: Vec::new(),
            sel: 0,
            mode: None,
            live: None,
            ms: None,
            rendered_query: None,
            last_query: None,
            inflight: false,
            pending: false,
            confirm_key: None,
            menu: None,
            hidden_at: None,
            query_history: Vec::new(),
            history_idx: -1,
            status: None,
            vscode: false,
            details_for: None,
            details: None,
            details_due: None,
            home: None,
            all_apps: false,
            ql: None,
            toast: None,
            scroll_to_sel: false,
            scroll_top: true,
            shown_at: None,
            armed_power: None,
            select_all: false,
            research_at: None,
            status_at: None,
            cover_keys: Vec::new(),
        }
    }

    fn tabs(&self, s: &Settings) -> Vec<(&'static str, &'static str, &'static str)> {
        let clip = s.clipvault && self.status.as_ref().map(|st| b(st, "clipvault")).unwrap_or(false);
        FILTERS.iter().copied().filter(|f| f.0 != "clipboard" || clip).collect()
    }

    fn is_start(s: &Settings) -> bool {
        s.layout == "start"
    }

    // ------------------------------------------------------------------ lifecycle

    pub fn on_shown(&mut self, ctx: &egui::Context, s: &Settings) {
        self.last_cursor = screen_cursor();
        if Self::is_start(s) {
            self.jobs.call(ctx, Tag::Home, "start_home", json!({}));
        }
        self.all_apps = false;
        self.shown_at = None;
        self.menu = None;
        // a recent query stays (selected, so typing replaces it); an old one is cleared
        if self.hidden_at.map(|t| t.elapsed() > Duration::from_secs(45)).unwrap_or(true) {
            self.query.clear();
            self.filter = "all";
        }
        self.select_all = true;
        self.history_idx = -1;
        // test copies: start with a query (GLINT_TEST_QUERY), for screenshots
        if let Ok(q) = std::env::var("GLINT_TEST_QUERY") {
            self.query = q;
            self.select_all = false;
            self.move_cursor_end(ctx);
        }
        self.search(ctx, true);
        self.jobs.call(ctx, Tag::Status, "status", json!({}));
        self.jobs.call(ctx, Tag::Queries, "queries", json!({}));
    }

    /// The window was actually revealed: start the opening animation now.
    pub fn on_revealed(&mut self) {
        self.shown_at = Some(Instant::now());
    }

    /// Reopened showing its last frame: no opening animation (it would blink from that frame
    /// to transparent and back).
    pub fn on_shown_warm(&mut self) {
        self.shown_at = Some(Instant::now() - Duration::from_secs(1));
    }

    pub fn on_hidden(&mut self) {
        self.ql = None;
        self.hidden_at = Some(Instant::now());
        self.confirm_key = None;
        self.menu = None;
        self.toast = None;
        self.reset_while_hidden = false;
    }

    /// While hidden, do what the next opening would (clear an old query, search again), so the
    /// hidden window's frame is what that opening shows first.
    fn hidden_upkeep(&mut self, ctx: &egui::Context) {
        if self.visible || self.reset_while_hidden {
            return;
        }
        if self.hidden_at.is_some_and(|t| t.elapsed() > Duration::from_secs(45)) {
            self.reset_while_hidden = true;
            if std::env::var_os("GLINT_TEST_TIMING").is_some() {
                eprintln!("TIMING hidden reset (query was {:?})", self.query);
            }
            if !self.query.is_empty() || self.filter != "all" {
                self.query.clear();
                self.filter = "all";
                self.search(ctx, true);
            }
        }
    }

    pub fn on_settings_changed(&mut self, ctx: &egui::Context, s: &Settings) {
        if Self::is_start(s) {
            self.jobs.call(ctx, Tag::Home, "start_home", json!({}));
        }
        self.search(ctx, true);
    }

    pub fn start(&mut self, ctx: &egui::Context, s: &Settings) {
        if Self::is_start(s) {
            self.jobs.call(ctx, Tag::Home, "start_home", json!({}));
        }
        self.jobs.call(ctx, Tag::Status, "status", json!({}));
        self.search(ctx, true);
    }

    // ------------------------------------------------------------------ async results

    fn poll(&mut self, ctx: &egui::Context, s: &Settings) {
        for (tag, r) in self.jobs.poll() {
            self.on_reply(ctx, s, tag, r);
        }
        if let Some((at, it)) = &self.details_due {
            if Instant::now() >= *at {
                let it = it.clone();
                self.details_due = None;
                let key = s_owned(&it, "key");
                self.details_for = Some(key.clone());
                self.details = None;
                if let Some(d) = test_json("GLINT_TEST_DETAILS").filter(|_| self.sel == 0) {
                    self.details = Some(d);
                } else {
                    self.jobs.call(ctx, Tag::Details(key), "details", json!({ "item": it }));
                }
            } else {
                ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
            }
        }
        if let Some(at) = self.research_at {
            if Instant::now() >= at {
                self.research_at = None;
                self.search(ctx, true);
            } else {
                ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
            }
        }
        if let Some(at) = self.status_at {
            if Instant::now() >= at {
                self.status_at = None;
                self.jobs.call(ctx, Tag::Status, "status", json!({}));
            } else {
                ctx.request_repaint_after(at.saturating_duration_since(Instant::now()));
            }
        }
    }

    fn on_reply(&mut self, ctx: &egui::Context, st: &Settings, tag: Tag, r: Reply) {
        match tag {
            Tag::Search(q) => {
                if let Ok(v) = r {
                    self.last_query = Some(q.clone());
                    // a newer query arrived while this one ran: show this one only if nothing better is coming
                    if q == self.query || !self.pending {
                        self.render(&v, &q, st);
                    }
                }
                self.inflight = false;
                if self.pending {
                    self.pending = false;
                    self.search(ctx, false);
                }
            }
            Tag::Details(key) => {
                if let Ok(d) = r {
                    if self.details_for.as_deref() == Some(&key) && self.cur().map(|x| s(x, "key") == key).unwrap_or(false) {
                        self.details = Some(d);
                    }
                }
            }
            Tag::Status => {
                if let Ok(v) = r {
                    self.vscode = b(&v, "vscode");
                    self.status = Some(v);
                    if !self.tabs(st).iter().any(|t| t.0 == self.filter) {
                        self.filter = "all";
                    }
                }
            }
            Tag::Queries => {
                if let Ok(Value::Array(a)) = r {
                    self.query_history = a.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                }
            }
            Tag::Home => {
                if let Ok(v) = r {
                    self.home = Some(v);
                }
            }
            Tag::AllApps => {
                if let Ok(Value::Array(apps)) = r {
                    let mut groups: std::collections::BTreeMap<String, Vec<Value>> = Default::default();
                    for a in apps {
                        let first = s(&a, "title").chars().next().unwrap_or('#');
                        let l = if first.is_ascii_alphabetic() { first.to_ascii_uppercase().to_string() } else { "#".into() };
                        groups.entry(l).or_default().push(a);
                    }
                    let sections: Vec<Value> = groups.into_iter().map(|(l, items)| json!({"title": l, "items": items})).collect();
                    self.render(&json!({"sections": sections, "mode": "apps", "live": true}), "", st);
                }
            }
            Tag::Activate(it, _mode) => match r {
                Ok(r) => {
                    if let Some(t) = r["toast"].as_str() {
                        self.toast = Some(Toast::new(t, false, 1.8));
                    }
                    if let Some(q) = r["query"].as_str() {
                        self.query = q.to_string();
                        self.move_cursor_end(ctx);
                        self.search(ctx, true);
                        return;
                    }
                    if b(&r, "hide") {
                        self.hide();
                    } else if b(&r, "refresh") {
                        self.details_for = None;
                        self.search(ctx, true);
                        // the player takes a moment to report its new state
                        if s(&it, "kind") == "media" {
                            self.research_at = Some(Instant::now() + Duration::from_millis(450));
                        }
                    }
                }
                Err(e) => {
                    if e != "Cancelled" {
                        self.toast = Some(Toast::new(e, true, 1.8));
                    }
                    self.search(ctx, true);
                }
            },
            Tag::Power => match r {
                Ok(r) => {
                    if b(&r, "hide") {
                        self.hide();
                    }
                }
                Err(e) => self.toast = Some(Toast::new(e, true, 1.8)),
            },
            Tag::QlText(key) => {
                if let Some(ql) = &mut self.ql {
                    if ql.key == key {
                        ql.body = match r {
                            Ok(v) => QlBody::Text(v.as_str().unwrap_or("").to_string()),
                            Err(e) => QlBody::Empty(e),
                        };
                    }
                }
            }
            Tag::QlDetails(key) => {
                if let Some(ql) = &mut self.ql {
                    if ql.key == key {
                        ql.body = QlBody::Info(r.ok());
                    }
                }
            }
            Tag::StartIndexer => match r {
                Ok(_) => {
                    self.toast = Some(Toast::new("Starting the indexer…", false, 1.8));
                    self.status_at = Some(Instant::now() + Duration::from_millis(2500));
                }
                Err(e) => self.toast = Some(Toast::new(e, true, 1.8)),
            },
            Tag::Ignore => {}
        }
    }

    // ------------------------------------------------------------------ search

    fn search(&mut self, ctx: &egui::Context, force: bool) {
        let q = self.query.clone();
        if !force && self.last_query.as_deref() == Some(q.as_str()) {
            return;
        }
        if self.inflight {
            self.pending = true;
            return;
        }
        self.inflight = true;
        // test copies: fixed results from a file (GLINT_TEST_RESULTS), to compare screenshots
        if let Some(v) = test_json("GLINT_TEST_RESULTS").filter(|_| std::env::var("GLINT_TEST_QUERY").ok().as_deref() == Some(q.as_str())) {
            let tx = Tag::Search(q.clone());
            self.on_reply(ctx, &crate::settings::load(), tx, Ok(v));
            return;
        }
        self.jobs.call(ctx, Tag::Search(q.clone()), "search", json!({"q": q, "filter": self.filter}));
    }

    fn render(&mut self, r: &Value, q: &str, st: &Settings) {
        let prev_key = self.cur().map(|x| s_owned(x, "key"));
        self.sections = r["sections"]
            .as_array()
            .map(|a| a.iter().map(|sec| (s_owned(sec, "title"), sec["items"].as_array().cloned().unwrap_or_default())).collect())
            .unwrap_or_default();
        self.mode = r["mode"].as_str().map(String::from);
        self.live = r["live"].as_bool();
        self.ms = r["ms"].as_f64();
        self.flat = self.sections.iter().flat_map(|(_, it)| it.iter().cloned()).collect();
        self.confirm_key = None;
        // keep the selection on the same item while typing refines results, else go to the top
        let keep = if self.rendered_query.as_deref() == Some(q) { prev_key.and_then(|k| self.flat.iter().position(|x| s(x, "key") == k)) } else { None };
        self.sel = keep.unwrap_or(0);
        self.rendered_query = Some(q.to_string());
        if self.sel == 0 {
            self.scroll_top = true;
        } else {
            self.scroll_to_sel = true;
        }
        for k in std::mem::take(&mut self.cover_keys) {
            let _ = k; // forgotten at the next draw (see `media_cover`)
        }
        self.update_preview(st);
    }

    fn cur(&self) -> Option<&Value> {
        self.flat.get(self.sel)
    }

    fn update_sel(&mut self, n: i64, st: &Settings, scroll: bool) {
        if self.flat.is_empty() {
            return;
        }
        let len = self.flat.len() as i64;
        self.sel = (((n % len) + len) % len) as usize;
        if let Some(ck) = &self.confirm_key {
            if self.cur().map(|x| s(x, "key") != ck.as_str()).unwrap_or(true) {
                self.confirm_key = None;
            }
        }
        if scroll {
            if self.sel == 0 {
                self.scroll_top = true;
            } else {
                self.scroll_to_sel = true;
            }
        }
        self.update_preview(st);
    }

    fn update_preview(&mut self, st: &Settings) {
        let home = Self::is_start(st) && self.query.trim().is_empty() && !self.all_apps;
        if home {
            return;
        }
        if !st.show_preview && !Self::is_start(st) {
            return;
        }
        let Some(it) = self.cur().cloned() else {
            self.details_for = None;
            self.details_due = None;
            return;
        };
        if self.details_for.as_deref() != Some(s(&it, "key")) {
            self.details = None;
        }
        if ["file", "folder", "content", "app", "window", "process", "action", "url", "layout", "toggle", "command"].contains(&s(&it, "kind")) {
            self.details_due = Some((Instant::now() + Duration::from_millis(70), it));
        } else {
            self.details_due = None;
        }
    }

    // ------------------------------------------------------------------ actions

    fn details_of(&self, it: &Value) -> Option<&Value> {
        if self.details_for.as_deref() == Some(s(it, "key")) {
            self.details.as_ref()
        } else {
            None
        }
    }

    fn actions_for(&self, it: &Value, s_: &Settings) -> Vec<Act> {
        let d = self.details_of(it);
        let mut base = self.base_actions(it, s_);
        let kind = s(it, "kind");
        if (truthy(it, "path") && ["file", "folder", "content", "clip"].contains(&kind)) || truthy(it, "image") {
            base.insert(1.min(base.len()), act("Quick Look", "eye", "quicklook", &["Shift", "Space"]));
        }
        if kind == "folder" {
            base.insert(2.min(base.len()), act("What takes the space", "hdd", "biggest", &[]));
        }
        if !PINNABLE.contains(&kind) {
            return base;
        }
        let pinned = d.map(|d| b(d, "pinned")).unwrap_or_else(|| b(it, "pinned"));
        let mut extra = vec![
            if pinned { act("Unpin", "pin", "unpin", &["Ctrl", "Shift", "P"]) } else { act("Pin to top", "pin", "pin", &["Ctrl", "Shift", "P"]) },
            act("Add keyword…", "tag", "keyword-prompt", &[]),
        ];
        match d.and_then(|d| d["hotkey"].as_str()).filter(|h| !h.is_empty()) {
            Some(h) => extra.push(act(&format!("Remove shortcut {h}"), "keyboard", "unhotkey", &[])),
            None => extra.push(act("Set a global shortcut…", "keyboard", "hotkey-prompt", &[])),
        }
        if let Some(kw) = d.and_then(|d| d["keywords"].as_array()) {
            for w in kw.iter().filter_map(|x| x.as_str()) {
                extra.push(act(&format!("Remove keyword “{w}”"), "eraser", &format!("unkeyword:{w}"), &[]));
            }
        }
        // keep the dangerous/forget entries last
        let (tail, head): (Vec<Act>, Vec<Act>) = base.into_iter().partition(|a| a.danger || a.mode == "forget");
        head.into_iter().chain(extra).chain(tail).collect()
    }

    fn base_actions(&self, it: &Value, st: &Settings) -> Vec<Act> {
        let path = s(it, "path");
        let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
        let fp = st.folder_opener == "filepilot";
        let recent: Vec<Act> = if b(it, "recent") { vec![act("Remove from Recent", "eraser", "forget", &["Ctrl", "Del"])] } else { vec![] };
        let danger = |mut a: Act| {
            a.danger = true;
            a
        };
        let mut v = Vec::new();
        match s(it, "kind") {
            "app" => {
                v.push(act("Open", "external", "default", &["↵"]));
                if !b(it, "store") {
                    v.push(act("Run as administrator", "admin", "admin", &["Shift", "↵"]));
                }
                if truthy(it, "path") || truthy(it, "lnk") {
                    v.push(act("Open file location", "folder", "reveal", &["Ctrl", "↵"]));
                    v.push(act("Copy path", "copy", "copy-path", &["Ctrl", "C"]));
                }
                if truthy(it, "lnk") {
                    v.push(act("Properties", "props", "properties", &["Alt", "↵"]));
                }
                v.extend(recent);
            }
            "file" | "content" => {
                let shot = truthy(it, "screenshot");
                if shot {
                    v.push(act("Copy image", "copy", "copy-image", &["↵"]));
                }
                v.push(act("Open", "external", "default", if shot { &["Ctrl", "↵"] } else { &["↵"] }));
                v.push(act("Show in folder", "folder", "reveal", &["Ctrl", "↵"]));
                v.push(act("Copy path", "copy", "copy-path", &["Ctrl", "C"]));
                v.push(act("Copy file", "copy", "copy-file", &["Ctrl", "Shift", "C"]));
                v.push(act("Open with…", "grid", "open-with", &["Ctrl", "O"]));
                if RUNNABLE.contains(&ext.as_str()) {
                    v.push(act("Run as administrator", "admin", "admin", &["Shift", "↵"]));
                }
                if self.vscode {
                    v.push(act("Open in VS Code", "code", "vscode", &[]));
                }
                v.push(act("Open terminal here", "terminal", "terminal", &["Ctrl", "T"]));
                v.push(act("Copy name", "copy", "copy-name", &[]));
                v.push(act("Properties", "props", "properties", &["Alt", "↵"]));
                v.push(danger(act("Move to Recycle Bin", "trash", "recycle", &["Shift", "Del"])));
                v.extend(recent);
            }
            "folder" => {
                v.push(act("Open", "external", "default", &["↵"]));
                v.push(act("Show in parent folder", "folder", "reveal", &["Ctrl", "↵"]));
                v.push(act("Open terminal here", "terminal", "terminal", &["Ctrl", "T"]));
                if self.vscode {
                    v.push(act("Open in VS Code", "code", "vscode", &[]));
                }
                if fp {
                    v.push(act("Open in File Explorer", "folder", "explorer", &[]));
                }
                v.push(act("Copy path", "copy", "copy-path", &["Ctrl", "C"]));
                v.push(act("Copy folder", "copy", "copy-file", &["Ctrl", "Shift", "C"]));
                v.push(act("Properties", "props", "properties", &["Alt", "↵"]));
                v.push(danger(act("Move to Recycle Bin", "trash", "recycle", &["Shift", "Del"])));
                v.extend(recent);
            }
            "window" => {
                v.push(act("Switch to window", "window", "default", &["↵"]));
                v.push(act("Close window", "x", "close", &["Ctrl", "W"]));
                v.push(act("Show program in folder", "folder", "reveal", &["Ctrl", "↵"]));
            }
            "process" => v.push(danger(act("End process", "x", "default", &["↵"]))),
            "calc" => v.push(act("Copy result", "copy", "default", &["↵"])),
            "color" => {
                for (i, a) in it["alts"].as_array().cloned().unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).enumerate() {
                    v.push(act(&format!("Copy {a}"), "copy", &if i > 0 { format!("alt:{a}") } else { "default".into() }, if i > 0 { &[] } else { &["↵"] }));
                }
            }
            "command" => {
                v.push(act("Run", "terminal", "default", &["↵"]));
                v.push(act("Run as administrator", "admin", "admin", &["Shift", "↵"]));
            }
            "url" | "web" => v.push(act("Open in browser", "globe", "default", &["↵"])),
            "action" => {
                v.push(act("Run", "bolt", "default", &["↵"]));
                v.extend(recent);
            }
            "toggle" => v.push(act("Apply", if truthy(it, "glyph") { s(it, "glyph") } else { "sliders" }, "default", &["↵"])),
            "info" => {
                if truthy(it, "copy") || truthy(it, "op") {
                    v.push(act(if s(it, "op") == "public_ip" { "Look up and copy" } else { "Copy" }, "copy", "default", &["↵"]));
                }
            }
            "layout" => v.push(act("Apply to window", "layout", "default", &["↵"])),
            "media" => {
                if s(it, "key") == "media:now" {
                    let playing = b(it, "playing");
                    v.push(act(if playing { "Pause" } else { "Play" }, if playing { "pause" } else { "play" }, "default", &["↵"]));
                    v.push(act("Next track", "next", "media-next", &[]));
                    v.push(act("Previous track", "prev", "media-prev", &[]));
                } else {
                    v.push(act(&self.go_label(it, st), if truthy(it, "glyph") { s(it, "glyph") } else { "play" }, "default", &["↵"]));
                }
            }
            "timer" => {
                if truthy(it, "op") {
                    let l = if s(it, "op") == "cancel" { "Cancel it" } else if s(it, "tkind") == "reminder" { "Set reminder" } else { "Start timer" };
                    v.push(act(l, "clock", "default", &["↵"]));
                }
            }
            "tab" => v.push(act("Switch to tab", "window", "default", &["↵"])),
            "script" => {
                v.push(act("Run", "terminal", "default", &["↵"]));
                v.push(act("Run in a terminal", "terminal", "admin", &["Shift", "↵"]));
            }
            "clip" => {
                if !st.clip_paste {
                    v.push(act("Copy to clipboard", "copy", "copy", &["↵"]));
                } else {
                    v.push(act("Paste into window", "clipboard", "default", &["↵"]));
                    v.push(act("Copy only", "copy", "copy", &["Ctrl", "↵"]));
                }
            }
            _ => {}
        }
        v
    }

    fn go_label(&self, it: &Value, st: &Settings) -> String {
        let kind = s(it, "kind");
        if b(it, "confirm") && self.confirm_key.as_deref() == Some(s(it, "key")) {
            return "Press again".into();
        }
        let l: &str = match kind {
            "media" => match s(it, "op") {
                "toggle" => {
                    if b(it, "playing") {
                        "Pause"
                    } else {
                        "Play"
                    }
                }
                "next" => "Next",
                "prev" => "Previous",
                _ => "Play",
            },
            "info" => {
                if s(it, "op") == "public_ip" {
                    "Look up"
                } else if truthy(it, "copy") {
                    "Copy"
                } else {
                    ""
                }
            }
            "clip" => {
                if st.clip_paste {
                    "Paste"
                } else {
                    "Copy"
                }
            }
            "timer" => {
                if s(it, "op") == "cancel" {
                    "Cancel"
                } else if s(it, "tkind") == "reminder" {
                    "Set"
                } else {
                    "Start"
                }
            }
            _ if truthy(it, "screenshot") => "Copy",
            "tab" | "window" => "Switch",
            "script" | "command" | "action" => "Run",
            "calc" | "color" => "Copy",
            "process" => "End",
            "web" => "Search",
            "url" => "Open",
            "hint" => "",
            "toggle" | "layout" => "Apply",
            _ => "Open",
        };
        l.to_string()
    }

    fn run_action(&mut self, ctx: &egui::Context, st: &Settings, it: Value, a: Act) {
        let mut it = it;
        let mut mode = a.mode.clone();
        match mode.as_str() {
            "keyword-prompt" => {
                self.menu = Some(Menu { items: vec![], sel: 0, item: it, kind: MenuKind::Keyword(String::new()), opened: Instant::now(), scroll_to_sel: false });
                return;
            }
            "hotkey-prompt" => {
                self.menu = Some(Menu { items: vec![], sel: 0, item: it, kind: MenuKind::Hotkey("Press the keys, e.g. Ctrl+Alt+T".into()), opened: Instant::now(), scroll_to_sel: false });
                return;
            }
            "quicklook" => {
                self.menu = None;
                self.ql_open(ctx, st);
                return;
            }
            "media-next" | "media-prev" => {
                it["op"] = json!(&mode[6..]);
                mode = "default".into();
            }
            _ => {}
        }
        self.menu = None;
        // dangerous things ask twice: first Enter arms, second runs
        let key = s_owned(&it, "key");
        let needs_confirm = (b(&it, "confirm") && mode == "default") || mode == "recycle";
        let arm_key = format!("{key}|{mode}");
        if needs_confirm && self.confirm_key.as_deref() != Some(&arm_key) && self.confirm_key.as_deref() != Some(&key) {
            self.confirm_key = Some(if mode == "default" { key } else { arm_key });
            if mode == "recycle" {
                self.toast = Some(Toast::new("Press Shift+Delete again to move it to the Recycle Bin", true, 1.8));
            }
            return;
        }
        self.confirm_key = None;
        self.jobs.call(ctx, Tag::Activate(it.clone(), mode.clone()), "activate", json!({"item": it, "mode": mode, "q": self.query}));
    }

    fn activate(&mut self, ctx: &egui::Context, st: &Settings, mode: &str) {
        let Some(it) = self.cur().cloned() else { return };
        if s(&it, "kind") == "hint" {
            return;
        }
        let mut mode = mode.to_string();
        if truthy(&it, "screenshot") && mode == "default" {
            mode = "copy-image".into();
        } else if truthy(&it, "screenshot") && mode == "reveal" {
            mode = "default".into();
        }
        let acts = self.actions_for(&it, st);
        let a = acts.iter().find(|a| a.mode == mode).cloned().or_else(|| if mode == "default" { acts.first().cloned() } else { None });
        if let Some(a) = a {
            self.run_action(ctx, st, it, a);
        }
    }

    fn open_menu(&mut self, st: &Settings) {
        let Some(it) = self.cur().cloned() else { return };
        let items = self.actions_for(&it, st);
        if items.is_empty() {
            return;
        }
        self.menu = Some(Menu { items, sel: 0, item: it, kind: MenuKind::List, opened: Instant::now(), scroll_to_sel: false });
    }

    fn hide(&mut self) {
        self.menu = None;
        host::hide_launcher();
    }

    fn set_filter(&mut self, ctx: &egui::Context, f: &'static str) {
        if self.filter == f {
            return;
        }
        self.filter = f;
        self.search(ctx, true);
    }

    fn cycle_filter(&mut self, ctx: &egui::Context, st: &Settings, dir: i32) {
        let list = self.tabs(st);
        let i = list.iter().position(|t| t.0 == self.filter).unwrap_or(0) as i32;
        let n = list.len() as i32;
        let f = list[((i + dir) % n + n) as usize % n as usize].0;
        self.set_filter(ctx, f);
    }

    fn show_all_apps(&mut self, ctx: &egui::Context) {
        self.all_apps = true;
        self.jobs.call(ctx, Tag::AllApps, "all_apps", json!({}));
    }

    // ------------------------------------------------------------------ Quick Look

    fn ql_open(&mut self, ctx: &egui::Context, st: &Settings) {
        let Some(it) = self.cur() else { return };
        if !(truthy(it, "path") || truthy(it, "image")) {
            return;
        }
        self.menu = None;
        host::quicklook(true);
        self.ql_render(ctx, st);
    }

    fn ql_close(&mut self) {
        self.ql = None;
        host::quicklook(false);
    }

    fn ql_render(&mut self, ctx: &egui::Context, _st: &Settings) {
        let Some(it) = self.cur().cloned() else { return };
        let key = s_owned(&it, "key");
        let p = if s(&it, "kind") == "clip" { s(&it, "image") } else { s(&it, "path") }.to_string();
        let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
        let body = if s(&it, "kind") == "clip" && !truthy(&it, "image") {
            QlBody::Text(s_owned(&it, "text"))
        } else if p.is_empty() {
            QlBody::Empty("Nothing to show".into())
        } else if QL_IMG.contains(&ext.as_str()) {
            if decodable(&ext) { QlBody::Picture(format!("file:{p}"), false) } else { QlBody::Picture(crate::icons::big_url(&p), false) }
        } else if QL_VID.contains(&ext.as_str()) {
            QlBody::Picture(crate::icons::big_url(&p), true)
        } else if QL_AUD.contains(&ext.as_str()) {
            QlBody::Audio(p.clone())
        } else if ext == "pdf" {
            QlBody::Picture(crate::icons::big_url(&p), false)
        } else if QL_TXT.contains(&ext.as_str()) {
            self.jobs.call(ctx, Tag::QlText(key.clone()), "read_text", json!({"path": p}));
            QlBody::Loading
        } else {
            self.jobs.call(ctx, Tag::QlDetails(key.clone()), "details", json!({"item": it}));
            QlBody::Loading
        };
        self.ql = Some(Ql { key, body });
    }

    // ------------------------------------------------------------------ keyboard

    fn query_selection(&self, ctx: &egui::Context) -> (usize, usize) {
        let id = Id::new("glint-q");
        egui::TextEdit::load_state(ctx, id)
            .and_then(|st| st.cursor.char_range())
            .map(|r| {
                let (a, b) = (r.primary.index.0, r.secondary.index.0);
                (a.min(b), a.max(b))
            })
            .unwrap_or((self.query.chars().count(), self.query.chars().count()))
    }

    fn move_cursor_end(&self, ctx: &egui::Context) {
        let id = Id::new("glint-q");
        let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let end = egui::text::CCursor::new(self.query.chars().count());
        st.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
        st.store(ctx, id);
    }

    fn select_all_query(&self, ctx: &egui::Context) {
        let id = Id::new("glint-q");
        let mut st = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let r = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(self.query.chars().count()));
        st.cursor.set_char_range(Some(r));
        st.store(ctx, id);
    }

    /// Handle one key press; true if it was ours (the text box doesn't see it).
    fn key(&mut self, ctx: &egui::Context, st: &Settings, k: Key, m: Modifiers) -> bool {
        let ctrl = m.ctrl || m.command;
        if self.ql.is_some() {
            match k {
                Key::Escape | Key::Space => self.ql_close(),
                Key::ArrowRight | Key::ArrowDown => {
                    self.update_sel(self.sel as i64 + 1, st, true);
                    self.ql_render(ctx, st);
                }
                Key::ArrowLeft | Key::ArrowUp => {
                    self.update_sel(self.sel as i64 - 1, st, true);
                    self.ql_render(ctx, st);
                }
                Key::Enter => {
                    self.ql_close();
                    self.activate(ctx, st, "default");
                }
                _ => {}
            }
            return true;
        }
        if k == Key::Space && m.shift && self.menu.is_none() {
            if let Some(it) = self.cur() {
                if truthy(it, "path") || truthy(it, "image") {
                    self.ql_open(ctx, st);
                    return true;
                }
            }
        }
        if self.all_apps && k == Key::Escape && self.query.is_empty() {
            self.all_apps = false;
            self.search(ctx, true);
            return true;
        }
        // the action menu has its own keys
        if let Some(menu) = &mut self.menu {
            match &mut menu.kind {
                MenuKind::Keyword(_) => {
                    // the keyword box handles typing; Enter/Esc are read there
                    return false;
                }
                MenuKind::Hotkey(text) => {
                    if k == Key::Escape {
                        self.menu = None;
                        return true;
                    }
                    let mut mods = Vec::new();
                    if m.ctrl {
                        mods.push("Ctrl");
                    }
                    if m.alt {
                        mods.push("Alt");
                    }
                    if m.shift {
                        mods.push("Shift");
                    }
                    let name = hotkey_name(k);
                    let combo = format!("{}{}{}", mods.join("+"), if mods.is_empty() { "" } else { "+" }, name);
                    *text = combo.clone();
                    let it = menu.item.clone();
                    self.run_action(ctx, st, it, Act { label: String::new(), icon: String::new(), mode: format!("hotkey:{combo}"), keys: vec![], danger: false });
                    return true;
                }
                MenuKind::List => {
                    let len = menu.items.len();
                    match k {
                        Key::ArrowDown => {
                            menu.sel = (menu.sel + 1) % len;
                            menu.scroll_to_sel = true;
                        }
                        Key::ArrowUp => {
                            menu.sel = (menu.sel + len - 1) % len;
                            menu.scroll_to_sel = true;
                        }
                        Key::Enter => {
                            let (it, a) = (menu.item.clone(), menu.items[menu.sel].clone());
                            self.run_action(ctx, st, it, a);
                        }
                        Key::Escape | Key::ArrowLeft => self.menu = None,
                        Key::K if ctrl => self.menu = None,
                        _ => return false,
                    }
                    return true;
                }
            }
        }
        match k {
            Key::Escape => {
                if self.confirm_key.is_some() {
                    self.confirm_key = None;
                } else if self.filter != "all" {
                    self.set_filter(ctx, "all");
                } else if !self.query.is_empty() {
                    self.query.clear();
                    self.search(ctx, true);
                } else {
                    self.hide();
                }
            }
            Key::ArrowDown => self.update_sel(self.sel as i64 + 1, st, true),
            Key::ArrowUp => {
                // Up on an empty box (or while browsing past queries) recalls earlier searches
                if (self.query.is_empty() || self.history_idx >= 0) && !self.query_history.is_empty() {
                    self.history_idx = (self.history_idx + 1).min(self.query_history.len() as i32 - 1);
                    self.query = self.query_history[self.history_idx as usize].clone();
                    self.move_cursor_end(ctx);
                    self.search(ctx, true);
                } else {
                    self.update_sel(self.sel as i64 - 1, st, true);
                }
            }
            Key::PageDown => {
                let n = (self.sel + 6).min(self.flat.len().saturating_sub(1));
                self.update_sel(n as i64, st, true);
            }
            Key::PageUp => self.update_sel(self.sel.saturating_sub(6) as i64, st, true),
            Key::Tab => {
                let complete = self.cur().map(|it| s_owned(it, "complete")).unwrap_or_default();
                let browsing = matches!(self.mode.as_deref(), Some("path") | Some("biggest"));
                if browsing && !complete.is_empty() && !m.shift {
                    self.query = complete;
                    self.move_cursor_end(ctx);
                    self.search(ctx, true);
                } else {
                    self.cycle_filter(ctx, st, if m.shift { -1 } else { 1 });
                }
            }
            Key::Enter => {
                if m.alt {
                    self.activate(ctx, st, "properties");
                } else if ctrl {
                    let clip = self.cur().map(|x| s(x, "kind") == "clip").unwrap_or(false);
                    self.activate(ctx, st, if clip { "copy" } else { "reveal" });
                } else if m.shift {
                    self.activate(ctx, st, "admin");
                } else {
                    self.activate(ctx, st, "default");
                }
            }
            Key::ArrowRight if !m.any() => {
                let (a, b2) = self.query_selection(ctx);
                if a == b2 && a == self.query.chars().count() && self.cur().is_some() {
                    self.open_menu(st);
                } else {
                    return false;
                }
            }
            Key::K if ctrl => self.open_menu(st),
            Key::Comma if ctrl => host::open_settings(""),
            Key::P if ctrl && !m.shift => {
                let on = !st.show_preview;
                self.jobs.call(ctx, Tag::Ignore, "settings_set", json!({"show_preview": on}));
            }
            Key::P if ctrl && m.shift => {
                if let Some(it) = self.cur().cloned() {
                    if PINNABLE.contains(&s(&it, "kind")) {
                        if let Some(mode) = self.actions_for(&it, st).iter().find(|a| a.mode == "pin" || a.mode == "unpin").map(|a| a.mode.clone()) {
                            self.activate(ctx, st, &mode);
                        }
                    }
                }
            }
            Key::Num1 | Key::Num2 | Key::Num3 | Key::Num4 | Key::Num5 | Key::Num6 | Key::Num7 | Key::Num8 | Key::Num9 if ctrl => {
                let i = (k as usize) - (Key::Num1 as usize);
                if i < self.flat.len() {
                    self.sel = i;
                    self.activate(ctx, st, "default");
                }
            }
            Key::C if ctrl => {
                let (a, b2) = self.query_selection(ctx);
                let Some(it) = self.cur().cloned() else { return false };
                if a != b2 {
                    return false;
                }
                if m.shift {
                    self.activate(ctx, st, "copy-file");
                } else if ["calc", "color", "info"].contains(&s(&it, "kind")) {
                    self.activate(ctx, st, "default");
                } else if s(&it, "kind") == "clip" {
                    self.activate(ctx, st, "copy");
                } else if truthy(&it, "path") || truthy(&it, "lnk") {
                    self.activate(ctx, st, "copy-path");
                }
            }
            Key::O if ctrl => self.activate(ctx, st, "open-with"),
            Key::T if ctrl => self.activate(ctx, st, "terminal"),
            Key::W if ctrl => self.activate(ctx, st, "close"),
            Key::Delete if ctrl && self.cur().map(|x| b(x, "recent")).unwrap_or(false) => self.activate(ctx, st, "forget"),
            Key::Delete if m.shift && self.cur().is_some() => self.activate(ctx, st, "recycle"),
            _ => return false,
        }
        true
    }

    fn handle_keys(&mut self, ui: &Ui, st: &Settings) {
        let ctx = ui.ctx().clone();
        let events = ui.input_mut(|i| std::mem::take(&mut i.events));
        let mut keep = Vec::with_capacity(events.len());
        let hotkey_prompt = matches!(self.menu.as_ref().map(|m| &m.kind), Some(MenuKind::Hotkey(_)));
        let log = std::env::var_os("GLINT_TEST_KEYLOG").is_some();
        for ev in events {
            if log && matches!(ev, egui::Event::Key { .. } | egui::Event::Text(_)) {
                eprintln!("KEY {ev:?}");
            }
            match &ev {
                egui::Event::Key { key, pressed: true, modifiers, .. } => {
                    if !self.key(&ctx, st, *key, *modifiers) {
                        keep.push(ev);
                    }
                }
                egui::Event::Text(_) if hotkey_prompt || self.ql.is_some() => {}
                // our own Ctrl+C handling must not also copy the (empty) selection
                egui::Event::Copy => {
                    let (a, b2) = self.query_selection(&ctx);
                    if a != b2 {
                        keep.push(ev);
                    }
                }
                _ => keep.push(ev),
            }
        }
        ui.input_mut(|i| i.events = keep);
    }

    // ------------------------------------------------------------------ drawing

    pub fn ui(&mut self, ui: &mut Ui, cx: &mut Cx) {
        let ctx = ui.ctx().clone();
        let st = cx.s.clone();
        self.poll(&ctx, &st);
        self.hidden_upkeep(&ctx);
        self.handle_keys(ui, &st);
        if self.select_all {
            self.select_all = false;
            self.select_all_query(&ctx);
        }
        // test hook: GLINT_TEST_OPEN=menu|ql|run[:row] opens the action menu or Quick Look, or runs the row, once results are in
        if let Ok(v) = std::env::var("GLINT_TEST_OPEN") {
            static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !self.flat.is_empty() && self.shown_at.map(|s0| s0.elapsed().as_millis() > 600).unwrap_or(false) && !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                let (what, row) = v.split_once(':').map(|(a, b)| (a.to_string(), b.parse::<i64>().unwrap_or(0))).unwrap_or((v.clone(), 0));
                self.update_sel(row, &st, true);
                match what.as_str() {
                    "ql" => self.ql_open(&ctx, &st),
                    "run" => self.activate(&ctx, &st, "default"),
                    _ => self.open_menu(&st),
                }
            }
            if !DONE.load(std::sync::atomic::Ordering::Relaxed) {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
        let t = cx.t.clone();
        let full = ui.max_rect();
        // the opening animation (.14s pop)
        let k = self.shown_at.map(|s0| (s0.elapsed().as_secs_f32() / 0.14).min(1.0)).unwrap_or(0.0);
        if self.shown_at.is_some() && k < 1.0 {
            ctx.request_repaint();
        }
        let e = ease(k);
        let scale = 0.985 + 0.015 * e;
        let dy = -4.0 * (1.0 - e);
        let c = full.center();
        let tr = egui::emath::TSTransform::new(vec2(c.x * (1.0 - scale), c.y * (1.0 - scale) + dy), scale);
        for layer in [ui.layer_id(), egui::LayerId::new(egui::Order::Foreground, Id::new("glint-menu"))] {
            ctx.set_transform_layer(layer, tr);
        }
        ui.set_opacity(if self.shown_at.is_some() { k.max(0.0) } else { 1.0 });

        let p = ui.painter().clone();
        let shell = full;
        p.rect_filled(shell, cr(8.0), t.bg);
        let inner = shell.shrink(1.0);
        let start = Self::is_start(&st);
        let home = start && self.query.trim().is_empty() && !self.all_apps;
        let show_preview = st.show_preview || start;
        let preview_w = if start { 270.0 } else { 290.0 };

        // vertical layout
        let (bar, tabs, body, foot);
        if start {
            foot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 38.0), inner.right_bottom());
            bar = Rect::from_min_max(pos2(inner.left(), foot.top() - 56.0), pos2(inner.right(), foot.top()));
            tabs = Rect::from_min_max(pos2(inner.left(), bar.top() - 48.0), pos2(inner.right(), bar.top()));
            body = Rect::from_min_max(inner.left_top(), pos2(inner.right(), tabs.top()));
        } else {
            bar = Rect::from_min_size(inner.left_top(), vec2(inner.width(), 62.0));
            tabs = Rect::from_min_max(pos2(inner.left(), bar.bottom()), pos2(inner.right(), bar.bottom() + 39.0));
            foot = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 38.0), inner.right_bottom());
            body = Rect::from_min_max(pos2(inner.left(), tabs.bottom()), pos2(inner.right(), foot.top()));
        }
        let menu_open = self.menu.is_some();
        self.draw_bar(ui, cx, bar, start, home);
        self.draw_tabs(ui, cx, tabs, start);
        let (results, preview) = if show_preview {
            (Rect::from_min_max(body.min, pos2(body.right() - preview_w, body.bottom())), Some(Rect::from_min_max(pos2(body.right() - preview_w, body.top()), body.max)))
        } else {
            (body, None)
        };
        self.draw_results(ui, cx, results, menu_open);
        if let Some(pr) = preview {
            p.line_segment([pos2(pr.left() + 0.5, pr.top()), pos2(pr.left() + 0.5, pr.bottom())], Stroke::new(1.0, t.line));
            let pr = Rect::from_min_max(pos2(pr.left() + 1.0, pr.top()), pr.max);
            if home {
                self.draw_start_side(ui, cx, pr);
            } else {
                self.draw_preview(ui, cx, pr);
            }
        }
        self.draw_footer(ui, cx, foot);
        if self.ql.is_some() {
            self.draw_ql(ui, cx, inner);
        }
        // the border on top of everything, like .shell's
        p.rect_stroke(shell, cr(8.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
        if self.menu.is_some() {
            self.draw_menu(ui, cx, inner);
        }
        if let Some(toast) = &self.toast {
            let lp = ui.painter().clone().with_layer_id(egui::LayerId::new(egui::Order::Tooltip, Id::new("glint-toast")));
            let _ = lp;
            if !toast.draw(ui, &t, cx.a, inner.center().x, inner.bottom() - 52.0, t.bg_solid) {
                self.toast = None;
            }
        }
        // the clock ticks by the minute; timer countdowns by the second
        let counting = self.flat.iter().any(|it| n(it, "due").is_some() && s(it, "op") == "cancel");
        if !self.visible {
        } else if counting {
            ctx.request_repaint_after(Duration::from_millis(1000 - (chrono::Local::now().timestamp_subsec_millis() as u64).min(999)));
        } else if st.show_clock || home {
            let now = chrono::Local::now();
            let ms = 60_000 - (now.timestamp() % 60) as u64 * 1000 - now.timestamp_subsec_millis() as u64;
            ctx.request_repaint_after(Duration::from_millis(ms.max(50)));
        }
    }

    fn draw_bar(&mut self, ui: &mut Ui, cx: &mut Cx, bar: Rect, start: bool, home: bool) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        if start {
            p.line_segment([bar.left_top() + vec2(0.0, 0.5), bar.right_top() + vec2(0.0, 0.5)], Stroke::new(1.0, t.line));
        }
        let cy = bar.center().y + if start { 0.5 } else { 0.0 };
        // search icon (accent while a search runs)
        let busy = self.inflight;
        let ic_col = lerp_color(t.text3, t.accent, anim(ui, Id::new("bar-busy"), busy, 0.15));
        cx.a.glyph(ui, "search", Rect::from_center_size(pos2(bar.left() + 20.0 + 12.0, cy), vec2(20.0, 20.0)), ic_col);
        // right side, from the right edge
        let mut right = bar.right() - 14.0;
        let gear = Rect::from_min_size(pos2(right - 32.0, cy - 16.0), vec2(32.0, 32.0));
        if widgets::icon_button(ui, &t, cx.a, gear, "gear", 18.0, 9.0, t.text3, t.text).on_hover_text("Settings (Ctrl+,)").clicked() {
            host::open_settings("");
        }
        right = gear.left() - 8.0;
        if cx.s.show_clock && !(start && home) {
            let g1 = theme::line(ui, &fmt::clock_time(), fd(15.0, 600.0), t.text, 200.0);
            let g2 = theme::line(ui, &fmt::clock_date_short(), f(11.0, 400.0), t.text3, 200.0);
            let w = g1.size().x.max(g2.size().x);
            let x_right = right - 6.0;
            let h = g1.size().y * 0.92 + g2.size().y;
            let top = cy - h / 2.0;
            p.galley(pos2(x_right - g1.size().x, top), g1.clone(), Color32::WHITE);
            p.galley(pos2(x_right - g2.size().x, top + g1.size().y * 0.92), g2, Color32::WHITE);
            right -= w + 12.0 + 8.0;
        }
        let label = match self.mode.as_deref() {
            Some("command") => "Command",
            Some("kill") => "End process",
            Some("web") => "Web search",
            Some("path") => "Browse",
            Some("clip") => "Clipboard",
            Some("biggest") => "Disk usage",
            Some("tabs") => "Open tabs",
            Some("bookmarks") => "Bookmarks",
            Some("apps") => "All apps",
            _ => "",
        };
        if !label.is_empty() {
            let g = theme::line(ui, label, f(11.5, 600.0), t.accent, 200.0);
            let w = g.size().x + 18.0 + 2.0;
            let r = Rect::from_min_size(pos2(right - w, cy - 11.5), vec2(w, 23.0));
            p.rect_filled(r, cr(11.5), theme::alpha(t.accent, 0.14));
            p.rect_stroke(r, cr(11.5), Stroke::new(1.0, theme::alpha(t.accent, 0.35)), StrokeKind::Inside);
            theme::put(&p, &g, r.left() + 10.0, r.center().y);
            right = r.left() - 8.0;
        }
        // the search box
        // the input had 2px of padding inside it
        let x0 = bar.left() + 20.0 + 24.0 + 12.0 + 2.0;
        let input = Rect::from_min_max(pos2(x0, bar.top()), pos2(right - 4.0, bar.bottom()));
        let font = if start { fd(17.0, 400.0) } else { fd(20.0, 400.0) };
        let (tc, pc) = (t.text, t.text3);
        let mut layouter = move |ui: &Ui, buf: &dyn egui::TextBuffer, _w: f32| {
            let mut j = theme::job(buf.as_str(), font, tc);
            j.wrap.max_width = f32::INFINITY;
            ui.fonts_mut(|f| f.layout_job(j))
        };
        let hint = theme::job("Search apps, files, settings and more…", font, pc);
        let editable = !matches!(self.menu.as_ref().map(|m| &m.kind), Some(MenuKind::Keyword(_)));
        let te = egui::TextEdit::singleline(&mut self.query)
            .id(Id::new("glint-q"))
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .vertical_align(egui::Align::Center)
            .desired_width(input.width())
            .hint_text(hint)
            .layouter(&mut layouter)
            .interactive(editable);
        let resp = ui.put(input, te);
        if editable && !resp.has_focus() {
            resp.request_focus();
        }
        if resp.changed() {
            self.history_idx = -1;
            let ctx = ui.ctx().clone();
            self.search(&ctx, false);
        }
    }

    fn draw_tabs(&mut self, ui: &mut Ui, cx: &mut Cx, r: Rect, start: bool) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let top = if start {
            p.line_segment([r.left_top() + vec2(0.0, 0.5), r.right_top() + vec2(0.0, 0.5)], Stroke::new(1.0, t.line));
            r.top() + 1.0 + 8.0
        } else {
            r.top()
        };
        p.line_segment([pos2(r.left(), r.bottom() - 0.5), pos2(r.right(), r.bottom() - 0.5)], Stroke::new(1.0, t.line));
        let mut x = r.left() + 16.0;
        let font = f(12.5, 500.0);
        let tabs = self.tabs(cx.s);
        let mut clicked = None;
        for (id, label, icon) in tabs {
            let g = theme::line(ui, label, font, Color32::WHITE, 300.0);
            let w = 11.0 * 2.0 + 14.0 + 6.0 + g.size().x + 2.0;
            let tr = Rect::from_min_size(pos2(x, top), vec2(w, 28.0));
            let resp = ui.interact(tr, Id::new(("tab", id)), Sense::click());
            let on = self.filter == id;
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            if on {
                p.rect_filled(tr, cr(8.0), t.sel);
                p.rect_stroke(tr, cr(8.0), Stroke::new(1.0, t.sel_line), StrokeKind::Inside);
            } else if h > 0.0 {
                p.rect_filled(tr, cr(8.0), theme::alpha(t.hover, h));
            }
            let fg = if on { t.text } else { lerp_color(t.text2, t.text, h) };
            cx.a.glyph(ui, icon, Rect::from_min_size(pos2(tr.left() + 12.0, tr.center().y - 7.0), vec2(14.0, 14.0)), theme::alpha(fg, 0.8));
            let g = theme::line(ui, label, font, fg, 300.0);
            theme::put(&p, &g, tr.left() + 12.0 + 14.0 + 6.0, tr.center().y - 1.0);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(id);
            }
            x += w + 4.0;
        }
        if !start {
            // "Tab switch"
            let g = theme::line(ui, "switch", f(11.5, 400.0), t.text3, 200.0);
            let right = r.right() - 14.0;
            theme::put_right(&p, &g, right, top + 14.0);
            let kw = widgets::kbd_width(ui, "Tab");
            kbd(ui, &t, right - g.size().x - 6.0 - kw, top + 14.0, "Tab");
        }
        if let Some(id) = clicked {
            let ctx = ui.ctx().clone();
            self.set_filter(&ctx, id);
        }
    }

    fn draw_results(&mut self, ui: &mut Ui, cx: &mut Cx, area: Rect, menu_open: bool) {
        let t = cx.t.clone();
        let ctx = ui.ctx().clone();
        let st = cx.s.clone();
        let q = self.query.clone();
        if self.flat.is_empty() {
            if q.trim().is_empty() {
                self.draw_tips(ui, cx, area);
            } else {
                self.draw_empty(ui, cx, area, q.trim());
            }
            return;
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        let mut sa = egui::ScrollArea::vertical().id_salt("results").auto_shrink([false, false]);
        if std::mem::take(&mut self.scroll_top) {
            sa = sa.vertical_scroll_offset(0.0);
        }
        let mut action: Option<(usize, &'static str)> = None;
        let mut menu_at: Option<usize> = None;
        let mut hover_sel: Option<usize> = None;
        let mut media_op: Option<(usize, String)> = None;
        let cur = screen_cursor();
        let pointer_moved = cur != self.last_cursor && ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
        self.last_cursor = cur;
        let scroll_to = std::mem::take(&mut self.scroll_to_sel);
        sa.show(&mut child, |ui| {
            ui.add_space(6.0);
            let w = ui.available_width();
            let mut i = 0usize;
            let mut first = true;
            let sections = self.sections.clone();
            for (title, items) in &sections {
                if items.is_empty() {
                    continue;
                }
                // section header
                let hh = if first { 6.0 + 14.6 + 6.0 } else { 12.0 + 14.6 + 6.0 };
                let (hr, _) = ui.allocate_exact_size(vec2(w, hh), Sense::hover());
                let mut j = theme::job(&title.to_uppercase(), f(11.0, 600.0), t.text3);
                j.sections.iter_mut().for_each(|s| s.format.extra_letter_spacing = 0.66);
                let g = theme::layout(ui, j, w, true);
                ui.painter().galley(pos2(hr.left() + 8.0 + 12.0, hr.bottom() - 6.0 - g.size().y), g, Color32::WHITE);
                first = false;
                for it in items {
                    let kind = s(it, "kind");
                    let h = if s(it, "key") == "media:now" {
                        74.0
                    } else if kind == "calc" {
                        62.0
                    } else {
                        50.0
                    };
                    let (rr, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
                    let row = Rect::from_min_max(pos2(rr.left() + 8.0, rr.top()), pos2(rr.right() - 8.0, rr.bottom()));
                    let resp = ui.interact(row, Id::new(("row", i)), Sense::click());
                    if resp.hovered() && pointer_moved && !menu_open && i != self.sel {
                        hover_sel = Some(i);
                    }
                    if resp.clicked() {
                        let ctrl = ctx.input(|i| i.modifiers.ctrl);
                        action = Some((i, if ctrl { "reveal" } else { "default" }));
                    }
                    if resp.secondary_clicked() {
                        menu_at = Some(i);
                    }
                    if scroll_to && i == self.sel {
                        ui.scroll_to_rect(Rect::from_min_max(pos2(row.left(), row.top() - 34.0), pos2(row.right(), row.bottom() + 10.0)), None);
                    }
                    if ui.is_rect_visible(row) {
                        if let Some(op) = self.draw_row(ui, cx, row, it, i, &q, resp.hovered() && !menu_open) {
                            media_op = Some((i, op));
                        }
                    }
                    i += 1;
                }
            }
            ui.add_space(10.0);
        });
        if let Some(i) = hover_sel {
            self.update_sel(i as i64, &st, false);
        }
        if let Some((i, op)) = media_op {
            let mut it = self.flat[i].clone();
            it["op"] = json!(op);
            self.run_action(&ctx, &st, it, act("", "", "default", &[]));
        } else if let Some((i, mode)) = action {
            if !menu_open {
                self.sel = i;
                self.activate(&ctx, &st, mode);
            }
        }
        if let Some(i) = menu_at {
            self.update_sel(i as i64, &st, false);
            self.open_menu(&st);
        }
    }

    /// One result row. Returns a media button press ("prev"/"toggle"/"next").
    #[allow(clippy::too_many_arguments)]
    fn draw_row(&mut self, ui: &mut Ui, cx: &mut Cx, r: Rect, it: &Value, i: usize, q: &str, hovered: bool) -> Option<String> {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let sel = i == self.sel;
        let kind = s(it, "kind");
        let key = s(it, "key");
        let danger = b(it, "confirm") && self.confirm_key.as_deref() == Some(key);
        if sel {
            let (bg, line, bar) = if danger { (theme::alpha(t.danger, 0.18), theme::alpha(t.danger, 0.5), t.danger) } else { (t.sel, t.sel_line, t.accent) };
            p.rect_filled(r, cr(10.0), bg);
            p.rect_stroke(r, cr(10.0), Stroke::new(1.0, line), StrokeKind::Inside);
            p.rect_filled(Rect::from_min_max(pos2(r.left(), r.top() + 12.0), pos2(r.left() + 3.0, r.bottom() - 12.0)), CornerRadius { nw: 0, sw: 0, ne: 3, se: 3 }, bar);
        } else if hovered {
            p.rect_filled(r, cr(10.0), t.hover);
        }
        if key == "media:now" {
            return self.draw_media_card(ui, cx, r, it, sel);
        }
        // icon
        let ico = Rect::from_min_size(pos2(r.left() + 13.0, r.center().y - 16.0), vec2(32.0, 32.0));
        self.draw_icon(ui, cx, ico, it);
        // meta (right side)
        let mut right = r.right() - 13.0;
        let mid = r.center().y;
        let go = self.go_label(it, cx.s);
        if sel {
            if !go.is_empty() {
                let w = widgets::kbd_width(ui, "↵");
                kbd(ui, &t, right - w, mid, "↵");
                let g = theme::line(ui, &go, f(12.0, 400.0), t.text2, 200.0);
                theme::put_right(&p, &g, right - w - 6.0, mid);
                right -= w + 6.0 + g.size().x + 6.0;
            }
        } else {
            if i < 9 {
                let g = theme::line(ui, &format!("Ctrl {}", i + 1), f(11.0, 400.0), t.text3, 100.0);
                let w = g.size().x.max(34.0);
                theme::put_right(&p, &g, right, mid);
                right -= w + 6.0;
            }
            let badge = if b(it, "recent") { Some("Recent") } else { kind_badge(kind) };
            if let (Some(bd), true) = (badge, self.filter == "all") {
                let mut j = theme::job(bd, f(10.5, 600.0), t.text3);
                j.sections[0].format.extra_letter_spacing = 0.21;
                let g = theme::layout(ui, j, 200.0, true);
                let br = Rect::from_min_size(pos2(right - g.size().x - 14.0, mid - 9.5), vec2(g.size().x + 14.0, 19.0));
                p.rect_filled(br, cr(6.0), t.panel2);
                theme::put(&p, &g, br.left() + 7.0, mid);
                right = br.left() - 6.0;
            }
        }
        if (kind == "file" || kind == "folder") && it["share"].is_null() {
            let mut parts = Vec::new();
            if let Some(sz) = n(it, "size") {
                if kind == "file" || sz > 0.0 {
                    parts.push(fmt::size(sz));
                }
            }
            if let Some(mt) = n(it, "mtime") {
                if mt > 0.0 {
                    parts.push(fmt::short_date(mt));
                }
            }
            if !parts.is_empty() {
                let g = theme::line(ui, &parts.join(" · "), f(11.5, 400.0), if sel { t.text2 } else { t.text3 }, 300.0);
                theme::put_right(&p, &g, right, mid);
                right -= g.size().x + 6.0;
            }
        }
        // text
        let x = ico.right() + 12.0;
        let tw = (right - 6.0 - x).max(20.0);
        let sub_text = if danger {
            "Press Enter again to confirm".to_string()
        } else if n(it, "due").is_some() && s(it, "op") == "cancel" {
            format!("{} left · {}", fmt::countdown(n(it, "due").unwrap_or(0.0)), s(it, "subtitle"))
        } else {
            s_owned(it, "subtitle")
        };
        let calc = kind == "calc";
        let title_font = if calc { fd(22.0, 600.0) } else { f(14.5, 500.0) };
        let pinned = b(it, "pinned");
        let mut tx = x;
        let title_job = if calc { theme::job(s(it, "title"), title_font, t.text) } else { highlight(s(it, "title"), q, title_font, &t) };
        let tg = theme::layout(ui, title_job, tw - if pinned { 18.0 } else { 0.0 }, true);
        let sg = (!sub_text.is_empty()).then(|| theme::line(ui, &sub_text, f(12.0, 400.0), if sel { t.text2 } else { t.text3 }, tw));
        let share = n(it, "share");
        let total_h = tg.size().y + sg.as_ref().map(|g| 2.0 + g.size().y).unwrap_or(0.0) + if share.is_some() { 8.0 } else { 0.0 };
        let mut y = (mid - total_h / 2.0).round();
        if pinned {
            cx.a.glyph(ui, "pin", Rect::from_min_size(pos2(tx, y + (tg.size().y - 12.0) / 2.0 + 1.0), vec2(12.0, 12.0)), t.accent);
            tx += 18.0;
        }
        p.galley(pos2(tx, y), tg.clone(), Color32::WHITE);
        y += tg.size().y;
        if let Some(sg) = sg {
            y += 2.0;
            p.galley(pos2(x, y), sg.clone(), Color32::WHITE);
            y += sg.size().y;
        }
        if let Some(sh) = share {
            let bar = Rect::from_min_size(pos2(x, y + 4.0), vec2(tw.min(360.0), 4.0));
            p.rect_filled(bar, cr(3.0), t.panel2);
            let fw = bar.width() * (sh.clamp(0.01, 1.0) as f32);
            gradient(&p, Rect::from_min_size(bar.min, vec2(fw, 4.0)), t.accent, theme::mix(t.accent, 0.6, Color32::WHITE));
        }
        None
    }

    fn draw_icon(&mut self, ui: &mut Ui, cx: &mut Cx, ico: Rect, it: &Value) {
        let t = cx.t;
        let p = ui.painter().clone();
        let kind = s(it, "kind");
        if kind == "color" {
            let c = theme::hex(s(it, "swatch")).unwrap_or(Color32::GRAY);
            p.rect_filled(ico, cr(8.0), c);
            p.rect_stroke(ico, cr(8.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
            return;
        }
        let fallback = if truthy(it, "glyph") { s_owned(it, "glyph") } else { kind_glyph(kind).unwrap_or("file").to_string() };
        if truthy(it, "icon") {
            let key = s_owned(it, "icon");
            if let Some(pic) = cx.a.image(ui.ctx(), &key) {
                paint_contain(ui, &pic.tex, pic.tex.size_vec2(), ico, 0.0);
                return;
            }
            if !cx.a.failed(&key) {
                return; // loading: an empty box, as the page showed while the image came in
            }
            glyph_box(ui, cx, ico, &fallback, false, 8.0, 18.0);
            return;
        }
        let accent = ["calc", "web", "url", "command", "toggle", "info", "layout", "media"].contains(&kind);
        let g = if truthy(it, "glyph") { s_owned(it, "glyph") } else { kind_glyph(kind).unwrap_or("sparkle").to_string() };
        glyph_box(ui, cx, ico, &g, accent, 8.0, 18.0);
    }

    fn draw_media_card(&mut self, ui: &mut Ui, cx: &mut Cx, r: Rect, it: &Value, _sel: bool) -> Option<String> {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let playing = b(it, "playing");
        let cover = Rect::from_min_size(pos2(r.left() + 13.0, r.center().y - 25.0), vec2(50.0, 50.0));
        self.draw_cover(ui, cx, cover, it, 10.0, 20.0);
        // buttons on the right
        let mut out = None;
        let cy = r.center().y;
        let next = Rect::from_center_size(pos2(r.right() - 13.0 - 16.0, cy), vec2(32.0, 32.0));
        let main = Rect::from_center_size(pos2(next.left() - 4.0 - 19.0, cy), vec2(38.0, 38.0));
        let prev = Rect::from_center_size(pos2(main.left() - 4.0 - 16.0, cy), vec2(32.0, 32.0));
        for (rect, op, icon, is_main) in [(prev, "prev", "prev", false), (main, "toggle", if playing { "pause" } else { "play" }, true), (next, "next", "next", false)] {
            let resp = ui.interact(rect, Id::new(("mc", op)), Sense::click());
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            let press = resp.is_pointer_button_down_on();
            let rr = if press { Rect::from_center_size(rect.center(), rect.size() * 0.92) } else { rect };
            if is_main {
                p.circle_filled(rr.center(), rr.width() / 2.0, lerp_color(t.accent, theme::mix(t.accent, 0.85, Color32::WHITE), h));
                cx.a.glyph(ui, icon, Rect::from_center_size(rr.center(), vec2(18.0, 18.0)), Color32::WHITE);
            } else {
                if h > 0.0 {
                    p.circle_filled(rr.center(), rr.width() / 2.0, theme::alpha(t.hover, h));
                }
                cx.a.glyph(ui, icon, Rect::from_center_size(rr.center(), vec2(16.0, 16.0)), lerp_color(t.text2, t.text, h));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                out = Some(op.to_string());
            }
        }
        let x = cover.right() + 14.0;
        let tw = prev.left() - 14.0 - x;
        let tg = theme::line(ui, s(it, "title"), f(14.5, 500.0), t.text, tw);
        let sg = theme::line(ui, s(it, "subtitle"), f(12.0, 400.0), t.text3, tw);
        let total = tg.size().y + 2.0 + sg.size().y + 3.0 + 10.0;
        let mut y = (cy - total / 2.0).round();
        p.galley(pos2(x, y), tg.clone(), Color32::WHITE);
        y += tg.size().y + 2.0;
        p.galley(pos2(x, y), sg.clone(), Color32::WHITE);
        y += sg.size().y + 3.0;
        // the little equalizer
        let time = ui.input(|i| i.time) as f32;
        for k in 0..4 {
            let h = if playing {
                let delay = [0.0, -0.4, -0.7, -0.2][k];
                let ph = ((time - delay) % 2.0 + 2.0) % 2.0;
                let tri = if ph < 1.0 { ph } else { 2.0 - ph };
                3.0 + 7.0 * ease(tri)
            } else {
                3.0
            };
            let bx = x + k as f32 * 5.0;
            p.rect_filled(Rect::from_min_max(pos2(bx, y + 10.0 - h), pos2(bx + 3.0, y + 10.0)), cr(1.0), theme::alpha(t.accent, if playing { 1.0 } else { 0.5 }));
        }
        if playing && self.visible {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        out
    }

    fn draw_cover(&mut self, ui: &mut Ui, cx: &mut Cx, r: Rect, it: &Value, radius: f32, glyph: f32) {
        let t = cx.t;
        let p = ui.painter().clone();
        widgets::shadow(&p, r, radius, Color32::from_black_alpha(64), 4.0, 14.0);
        diag_gradient(&p, r, radius, theme::alpha(t.accent, 0.45), theme::alpha(t.accent, 0.12));
        let key = s_owned(it, "icon");
        if !key.is_empty() {
            if !self.cover_keys.contains(&key) {
                self.cover_keys.push(key.clone());
            }
            if let Some(pic) = cx.a.image(ui.ctx(), &key) {
                paint_cover(ui, &pic.tex, r, radius);
                return;
            }
            if !cx.a.failed(&key) {
                return;
            }
        }
        cx.a.glyph(ui, "play", Rect::from_center_size(r.center(), vec2(glyph, glyph)), Color32::WHITE);
    }

    fn draw_tips(&mut self, ui: &mut Ui, cx: &mut Cx, area: Rect) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();
        // "Get started"
        let mut j = theme::job("GET STARTED", f(11.0, 600.0), t.text3);
        j.sections[0].format.extra_letter_spacing = 0.66;
        let g = theme::layout(ui, j, 400.0, true);
        let x0 = area.left() + 8.0;
        p.galley(pos2(x0 + 12.0, area.top() + 6.0 + 6.0), g.clone(), Color32::WHITE);
        let top = area.top() + 6.0 + 6.0 + g.size().y + 6.0 + 8.0;
        let w = area.width() - 16.0 - 12.0;
        let cw = (w - 8.0) / 2.0;
        let ch = 60.0;
        let mut clicked = None;
        for (i, tip) in TIPS.iter().enumerate() {
            let (col, row) = (i % 2, i / 2);
            let r = Rect::from_min_size(pos2(x0 + 6.0 + col as f32 * (cw + 8.0), top + row as f32 * (ch + 8.0)), vec2(cw, ch));
            if r.top() > area.bottom() {
                break;
            }
            let resp = ui.interact(r, Id::new(("tip", i)), Sense::click());
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            p.rect_filled(r, cr(12.0), lerp_color(t.panel, t.panel2, h));
            p.rect_stroke(r, cr(12.0), Stroke::new(1.0, lerp_color(t.line, t.line2, h)), StrokeKind::Inside);
            let ico = Rect::from_min_size(pos2(r.left() + 15.0, r.center().y - 16.0), vec2(32.0, 32.0));
            glyph_box(ui, cx, ico, tip.icon, true, 8.0, 18.0);
            let tx = ico.right() + 12.0;
            let g1 = theme::line(ui, tip.t1, f(13.0, 500.0), t.text, r.right() - tx - 10.0);
            let g2 = theme::layout(ui, theme::rich_accent(tip.t2, f(12.0, 400.0), t.text3, &t), r.right() - tx - 10.0, true);
            let total = g1.size().y + 2.0 + g2.size().y;
            let y = r.center().y - total / 2.0;
            p.galley(pos2(tx, y), g1.clone(), Color32::WHITE);
            p.galley(pos2(tx, y + g1.size().y + 2.0), g2, Color32::WHITE);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                clicked = Some(i);
            }
        }
        if let Some(i) = clicked {
            let tip = &TIPS[i];
            if !tip.q.is_empty() {
                self.query = tip.q.to_string();
                self.move_cursor_end(&ctx);
                self.search(&ctx, true);
            }
        }
    }

    fn draw_empty(&mut self, ui: &mut Ui, cx: &mut Cx, area: Rect, q: &str) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let off = self.live == Some(false);
        let h3 = theme::wrapped(ui, &format!("No results for “{q}”"), f(15.0, 600.0), t.text, area.width() - 60.0);
        let msg = format!(
            "{}Try fewer words, or press <kbd>Tab</kbd> to search only files, folders or file contents.",
            if off { "The file indexer isn’t running, so only your user folder was searched. " } else { "" }
        );
        let mut j = theme::rich(&msg, f(12.5, 400.0), t.text2, &t);
        j.halign = egui::Align::Center;
        j.sections.iter_mut().for_each(|s| s.format.line_height = Some(18.75));
        let pg = theme::layout(ui, j, 360.0_f32.min(area.width() - 60.0), false);
        let total = 40.0 + 6.0 + 8.0 + h3.size().y + 8.0 + pg.size().y;
        let mut y = area.center().y - total / 2.0;
        cx.a.glyph(ui, "search", Rect::from_center_size(pos2(area.center().x, y + 20.0), vec2(40.0, 40.0)), t.text3);
        y += 40.0 + 6.0 + 8.0;
        p.galley(pos2(area.center().x - h3.size().x / 2.0, y), h3.clone(), Color32::WHITE);
        y += h3.size().y + 8.0;
        p.galley(pos2(area.center().x, y), pg, Color32::WHITE);
    }

    // ------------------------------------------------------------------ preview

    fn draw_preview(&mut self, ui: &mut Ui, cx: &mut Cx, area: Rect) {
        let Some(it) = self.cur().cloned() else { return };
        let t = cx.t.clone();
        let st = cx.s.clone();
        let d = self.details_of(&it).cloned();
        let acts: Vec<Act> = self.actions_for(&it, &st).into_iter().take(5).collect();
        // one scrolling column like `.preview`: content, then the actions pushed to the bottom
        // (margin-top: auto) when there's room, or after the content when there isn't
        let acts_h = if acts.len() > 1 { 6.0 + acts.len() as f32 * 32.0 + (acts.len() as f32 - 1.0) * 4.0 } else { 0.0 };
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(area.intersect(ui.clip_rect()));
        let mut run = None;
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().id_salt(("pv", s(&it, "key"))).auto_shrink([false, false]).show(&mut child, |ui| {
            let top = ui.cursor().top();
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(18.0);
                ui.vertical(|ui| {
                    let w = area.width() - 36.0;
                    ui.set_width(w);
                    self.preview_content(ui, cx, &it, d.as_ref());
                    if acts_h > 0.0 {
                        let used = ui.cursor().top() - top;
                        let room = area.height() - 28.0 - used - acts_h;
                        ui.add_space(14.0 + room.max(0.0));
                        let x0 = ui.cursor().left();
                        for (i, a) in acts.iter().enumerate() {
                            if i == 0 {
                                ui.add_space(6.0);
                            } else {
                                ui.add_space(4.0);
                            }
                            let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
                            let r = Rect::from_min_size(pos2(x0, r.top()), vec2(w, 32.0));
                            let resp = ui.interact(r, Id::new(("pv-act", i)), Sense::click());
                            let h = anim(ui, resp.id, resp.hovered(), 0.12);
                            if h > 0.0 {
                                ui.painter().rect_filled(r, cr(8.0), theme::alpha(t.hover, h));
                            }
                            let fg = lerp_color(t.text2, t.text, h);
                            cx.a.glyph(ui, &a.icon, Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 7.5), vec2(15.0, 15.0)), fg);
                            let kref: Vec<&str> = a.keys.iter().map(|s| s.as_str()).collect();
                            let kw = if kref.is_empty() { 0.0 } else { keys_width(ui, &kref, 3.0) };
                            let g = theme::line(ui, &a.label, f(12.5, 400.0), fg, r.width() - 45.0 - kw);
                            theme::put(ui.painter(), &g, r.left() + 35.0, r.center().y);
                            if !kref.is_empty() {
                                keys(ui, &t, r.right() - 10.0 - kw, r.center().y, &kref, 3.0);
                            }
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                run = Some(a.clone());
                            }
                        }
                    }
                    ui.add_space(14.0);
                });
            });
        });
        if let Some(a) = run {
            self.run_action(&ctx, &st, it, a);
        }
    }

    fn preview_content(&mut self, ui: &mut Ui, cx: &mut Cx, it: &Value, d: Option<&Value>) {
        let t = cx.t.clone();
        let w = ui.available_width();
        let kind = s(it, "kind");
        let gap = 14.0;
        if kind == "clip" {
            if truthy(it, "image") {
                let key = format!("file:{}", s(it, "image"));
                media_box(ui, cx, &key, w, 220.0, false);
                ui.add_space(gap);
            }
            text_block(ui, s(it, "subtitle"), f(12.0, 400.0), t.text3, w, false);
            if s(it, "clipkind") != "Image" && truthy(it, "text") {
                ui.add_space(gap);
                pv_text(ui, cx, s(it, "text"), w, 260.0, false);
            }
            if truthy(it, "image") {
                let key = format!("file:{}", s(it, "image"));
                if let Some(pic) = cx.a.image(ui.ctx(), &key) {
                    ui.add_space(gap);
                    facts(ui, cx, &[("Size".into(), format!("{} × {}", pic.dims[0], pic.dims[1]))], w);
                }
            }
            return;
        }
        if s(it, "key") == "media:now" {
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 156.0), Sense::hover());
            let cover = Rect::from_center_size(r.center(), vec2(156.0, 156.0));
            self.draw_cover(ui, cx, cover, it, 18.0, 36.0);
            ui.add_space(10.0);
            text_block(ui, s(it, "title"), f(15.0, 600.0), t.text, w, true);
            ui.add_space(10.0);
            text_block(ui, s(it, "subtitle"), f(12.0, 400.0), t.text3, w, true);
            return;
        }
        if kind == "calc" {
            ui.add_space(18.0);
            text_block(ui, s(it, "title"), fd(30.0, 650.0), t.text, w, true);
            ui.add_space(4.0 + gap);
            text_block(ui, s(it, "subtitle"), f(12.0, 400.0), t.text3, w, true);
            return;
        }
        if kind == "color" {
            let (r, _) = ui.allocate_exact_size(vec2(w, 110.0), Sense::hover());
            let c = theme::hex(s(it, "swatch")).unwrap_or(Color32::GRAY);
            ui.painter().rect_filled(r, cr(14.0), c);
            ui.painter().rect_stroke(r, cr(14.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
            ui.add_space(gap);
            text_block(ui, s(it, "title"), f(15.0, 600.0), t.text, w, false);
            let alts: Vec<(String, String)> =
                it["alts"].as_array().cloned().unwrap_or_default().iter().enumerate().filter_map(|(i, a)| a.as_str().map(|a| ([("Hex"), ("RGB"), ("HSL")].get(i).copied().unwrap_or("").to_string(), a.to_string()))).collect();
            if !alts.is_empty() {
                ui.add_space(gap);
                facts(ui, cx, &alts, w);
            }
            return;
        }
        // the hero: thumbnail / picture, or the icon
        ui.add_space(6.0);
        let mut drew_big = false;
        if let Some(d) = d {
            let path = s(d, "path");
            match s(d, "media") {
                "image" => {
                    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
                    let key = if decodable(&ext) { format!("file:{path}") } else { crate::icons::thumb_url(path) };
                    drew_big = media_box(ui, cx, &key, w, 220.0, false);
                }
                "video" => drew_big = media_box(ui, cx, &crate::icons::thumb_url(path), w, 220.0, true),
                "audio" => {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
                    let ib = Rect::from_center_size(r.center(), vec2(72.0, 72.0));
                    ui.painter().rect_filled(ib, cr(16.0), t.panel2);
                    ui.painter().rect_stroke(ib, cr(16.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
                    cx.a.glyph(ui, "volume", Rect::from_center_size(ib.center(), vec2(30.0, 30.0)), t.accent);
                    drew_big = true;
                }
                _ => {
                    if truthy(d, "thumb") {
                        drew_big = media_box(ui, cx, s(d, "thumb"), w, 200.0, false);
                    }
                }
            }
        }
        if !drew_big {
            let (r, _) = ui.allocate_exact_size(vec2(w, 72.0), Sense::hover());
            let ib = Rect::from_center_size(r.center(), vec2(72.0, 72.0));
            let fallback = if truthy(it, "glyph") { s_owned(it, "glyph") } else { kind_glyph(kind).unwrap_or("sparkle").to_string() };
            let mut done = false;
            if truthy(it, "icon") {
                let key = s_owned(it, "icon");
                if let Some(pic) = cx.a.image(ui.ctx(), &key) {
                    paint_contain(ui, &pic.tex, pic.tex.size_vec2(), Rect::from_center_size(ib.center(), vec2(64.0, 64.0)), 0.0);
                    done = true;
                } else if !cx.a.failed(&key) {
                    done = true;
                }
            }
            if !done {
                glyph_box(ui, cx, ib, &fallback, false, 16.0, 30.0);
            }
        }
        ui.add_space(10.0);
        text_block(ui, s(it, "title"), f(15.0, 600.0), t.text, w, true);
        let kind_name = match kind {
            "app" => "Application",
            "file" => "File",
            "folder" => "Folder",
            "content" => "Text match",
            "window" => "Open window",
            "process" => "Running process",
            "action" => "Command",
            "web" => "Web search",
            "url" => "Website",
            "command" => "PowerShell command",
            _ => "",
        };
        ui.add_space(10.0);
        text_block(ui, kind_name, f(12.0, 400.0), t.text3, w, true);
        // the body
        if let Some(d) = d {
            if b(d, "missing") {
                ui.add_space(gap);
                text_block(ui, "This file no longer exists.", f(12.0, 400.0), t.danger, w, false);
            }
            let mut fs: Vec<(String, String)> = Vec::new();
            let fact = |fs: &mut Vec<(String, String)>, k: &str, v: String| {
                if !v.is_empty() {
                    fs.push((k.to_string(), v));
                }
            };
            let is_dir = d["is_dir"].as_bool();
            if is_dir == Some(false) {
                fact(&mut fs, "Size", fmt::size(n(d, "size").unwrap_or(0.0)));
            }
            if is_dir == Some(true) {
                if let Some(c) = n(d, "count") {
                    fact(&mut fs, "Items", if c >= 400.0 { "400+".into() } else { format!("{}", c as i64) });
                }
            }
            fact(&mut fs, "Modified", fmt::date(n(d, "modified").unwrap_or(0.0)));
            if let Some(c) = n(d, "created") {
                fact(&mut fs, "Created", fmt::date(c));
            }
            if s(d, "media") == "image" && d["path"].is_string() {
                let ext = s(d, "path").rsplit('.').next().unwrap_or("").to_lowercase();
                if decodable(&ext) {
                    if let Some(pic) = cx.a.image(ui.ctx(), &format!("file:{}", s(d, "path"))) {
                        fact(&mut fs, "Dimensions", format!("{} × {}", pic.dims[0], pic.dims[1]));
                    }
                }
            }
            let ver = &d["version"];
            fact(&mut fs, "Description", s_owned(ver, "description"));
            fact(&mut fs, "Publisher", s_owned(ver, "company"));
            fact(&mut fs, "Version", s_owned(ver, "version"));
            if truthy(d, "target") {
                fact(&mut fs, "Target", self.pretty_path(s(d, "target")));
            }
            fact(&mut fs, "Opens with", s_owned(d, "opens_with"));
            if d["archive"].is_object() {
                fact(&mut fs, "Contains", format!("{} items · {} unpacked", fmt::count(n(&d["archive"], "count").unwrap_or(0.0)), fmt::size(n(&d["archive"], "unpacked").unwrap_or(0.0))));
            }
            fact(&mut fs, "Where", s_owned(d, "pretty"));
            if let Some(kw) = d["keywords"].as_array().filter(|a| !a.is_empty()) {
                fact(&mut fs, "Keyword", kw.iter().filter_map(|x| x.as_str()).collect::<Vec<_>>().join(", "));
            }
            fact(&mut fs, "Shortcut", s_owned(d, "hotkey"));
            if is_dir == Some(true) && n(d, "total").unwrap_or(0.0) > 0.0 {
                fs.insert(0, ("Size".into(), format!("{} inside", fmt::size(n(d, "total").unwrap_or(0.0)))));
            }
            if ["app", "window", "process"].contains(&kind) && truthy(d, "path") {
                fact(&mut fs, "Program", self.pretty_path(s(d, "path")));
            }
            if !fs.is_empty() {
                ui.add_space(gap);
                facts(ui, cx, &fs, w);
            }
            if truthy(&d["snippet"], "text") {
                ui.add_space(gap);
                snippet(ui, cx, &d["snippet"], w);
            } else if truthy(d, "text") {
                ui.add_space(gap);
                pv_text(ui, cx, s(d, "text"), w, 240.0, true);
            }
            for list in [&d["archive"]["entries"], &d["children"]] {
                if let Some(a) = list.as_array().filter(|a| !a.is_empty()) {
                    ui.add_space(gap);
                    children(ui, cx, a, w);
                }
            }
        } else if truthy(it, "subtitle") && !["calc", "color", "clip"].contains(&kind) {
            ui.add_space(gap);
            let k = if ["file", "folder", "content"].contains(&kind) { "Where" } else { "Details" };
            facts(ui, cx, &[(k.into(), s_owned(it, "subtitle"))], w);
        }
    }

    fn pretty_path(&self, p: &str) -> String {
        if let Some(home) = self.status.as_ref().map(|s_| s(s_, "home")).filter(|h| !h.is_empty()) {
            if p.to_lowercase().starts_with(&home.to_lowercase()) {
                return format!("~{}", &p[home.len()..]);
            }
        }
        p.to_string()
    }

    // ------------------------------------------------------------------ Start-menu side

    fn draw_start_side(&mut self, ui: &mut Ui, cx: &mut Cx, area: Rect) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let st = cx.s.clone();
        let inner = Rect::from_min_max(pos2(area.left() + 18.0, area.top() + 14.0), pos2(area.right() - 18.0, area.bottom() - 12.0));
        let home = self.home.clone().unwrap_or(json!({"folders": []}));
        let user = {
            let u = s(&home, "user");
            if !u.is_empty() {
                u.to_string()
            } else {
                self.status.as_ref().map(|x| s_owned(x, "user")).filter(|x| !x.is_empty()).unwrap_or_else(|| "You".into())
            }
        };
        let mut y = inner.top();
        // the user
        let av = Rect::from_min_size(pos2(inner.left(), y), vec2(38.0, 38.0));
        widgets::shadow(&p, av, 19.0, theme::alpha(t.accent, 0.35), 4.0, 14.0);
        diag_gradient(&p, av, 19.0, t.accent, theme::mix(t.accent, 0.55, Color32::BLACK));
        let initial = user.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default();
        let g = theme::line(ui, &initial, fd(18.0, 650.0), Color32::WHITE, 40.0);
        theme::put(&p, &g, av.center().x - g.size().x / 2.0, av.center().y);
        let total = self.status.as_ref().and_then(|x| n(&x["indexer"], "total")).unwrap_or(0.0);
        let sub = if total > 0.0 { format!("{} files ready to search", fmt::count(total)) } else { "Start".into() };
        let g1 = theme::line(ui, &user, f(15.0, 600.0), t.text, inner.width() - 50.0);
        let g2 = theme::line(ui, &sub, f(11.5, 400.0), t.text3, inner.width() - 50.0);
        let ty = av.center().y - (g1.size().y + g2.size().y) / 2.0;
        p.galley(pos2(av.right() + 12.0, ty), g1.clone(), Color32::WHITE);
        p.galley(pos2(av.right() + 12.0, ty + g1.size().y), g2, Color32::WHITE);
        y = av.bottom() + 10.0;
        // the big clock
        let gt = theme::line(ui, &fmt::clock_time(), fd(30.0, 600.0), t.text, inner.width());
        let gd = theme::line(ui, &fmt::clock_date_long(), f(12.5, 400.0), t.text2, inner.width() - 28.0);
        let ch = 10.0 + 31.5 + 4.0 + gd.size().y + 10.0 + 2.0;
        let cr_ = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), ch));
        p.rect_filled(cr_, cr(12.0), t.panel);
        p.rect_stroke(cr_, cr(12.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
        p.galley(pos2(cr_.left() + 15.0, cr_.top() + 11.0 + (31.5 - gt.size().y) / 2.0), gt, Color32::WHITE);
        p.galley(pos2(cr_.left() + 15.0, cr_.top() + 11.0 + 31.5 + 4.0), gd, Color32::WHITE);
        y = cr_.bottom() + 10.0;
        // links
        let folders = home["folders"].as_array().cloned().unwrap_or_default();
        let mut clicked: Option<Value> = None;
        let mut all = false;
        let mut settings = false;
        let link = |ui: &mut Ui, cx: &mut Cx, y: f32, id: Id, icon: &str, label: &str, chev: bool| -> bool {
            let r = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), 31.0));
            let resp = ui.interact(r, id, Sense::click());
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            if h > 0.0 {
                ui.painter().rect_filled(r, cr(8.0), theme::alpha(t.hover, h));
            }
            cx.a.glyph(ui, icon, Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 8.5), vec2(17.0, 17.0)), t.text2);
            let g = theme::line(ui, label, f(13.5, 500.0), t.text, r.width() - 60.0);
            theme::put(ui.painter(), &g, r.left() + 10.0 + 17.0 + 12.0, r.center().y);
            if chev {
                let g = theme::line(ui, "›", f(16.0, 400.0), t.text3, 30.0);
                theme::put_right(ui.painter(), &g, r.right() - 10.0, r.center().y - 1.0);
            }
            resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        };
        for (i, fo) in folders.iter().enumerate() {
            if link(ui, cx, y, Id::new(("st-f", i)), s(fo, "glyph"), s(fo, "title"), false) {
                clicked = Some(fo.clone());
            }
            y += 32.0;
        }
        p.line_segment([pos2(inner.left() + 4.0, y + 4.5), pos2(inner.right() - 4.0, y + 4.5)], Stroke::new(1.0, t.line));
        y += 9.0 + 1.0;
        if link(ui, cx, y, Id::new("st-all"), "grid", "All apps", true) {
            all = true;
        }
        y += 32.0;
        if link(ui, cx, y, Id::new("st-settings"), "gear", "Settings", false) {
            settings = true;
        }
        // power buttons, at the bottom
        let pw_h = 34.0 * 2.0 + 6.0;
        let top = inner.bottom() - pw_h;
        let cw = (inner.width() - 6.0) / 2.0;
        if let Some((_, at)) = &self.armed_power {
            if at.elapsed() > Duration::from_millis(2500) {
                self.armed_power = None;
            } else {
                ctx.request_repaint_after(Duration::from_millis(2500).saturating_sub(at.elapsed()));
            }
        }
        let mut press = None;
        for (i, (id, label, icon, confirm)) in [("lock", "Lock", "lock", false), ("sleep", "Sleep", "moon", false), ("restart", "Restart", "restart", true), ("shutdown", "Shut down", "power", true)].iter().enumerate() {
            let r = Rect::from_min_size(pos2(inner.left() + (i % 2) as f32 * (cw + 6.0), top + (i / 2) as f32 * 40.0), vec2(cw, 34.0));
            let resp = ui.interact(r, Id::new(("pw", *id)), Sense::click());
            let h = anim(ui, resp.id, resp.hovered(), 0.12);
            let armed = self.armed_power.as_ref().map(|a| a.0 == *id).unwrap_or(false);
            p.rect_filled(r, cr(8.0), lerp_color(t.panel, t.hover, h));
            p.rect_stroke(r, cr(8.0), Stroke::new(1.0, if armed { t.danger } else { t.line }), StrokeKind::Inside);
            let fg = if armed { t.danger } else { lerp_color(t.text2, t.text, h) };
            cx.a.glyph(ui, icon, Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 8.5), vec2(17.0, 17.0)), fg);
            let g = theme::line(ui, if armed { "Click again" } else { label }, f(12.5, 500.0), fg, r.width() - 40.0);
            theme::put(&p, &g, r.left() + 10.0 + 17.0 + 8.0, r.center().y);
            if resp.on_hover_text(*label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                press = Some((*id, *confirm, armed));
            }
        }
        let _ = y;
        if let Some((id, confirm, armed)) = press {
            if confirm && !armed {
                self.armed_power = Some((id.to_string(), Instant::now()));
            } else {
                self.armed_power = None;
                self.jobs.call(&ctx, Tag::Power, "activate", json!({"item": {"kind": "action", "id": id, "key": format!("act:{id}")}, "mode": "default", "q": ""}));
            }
        }
        if let Some(fo) = clicked {
            self.run_action(&ctx, &st, fo, act("", "", "default", &[]));
        }
        if all {
            self.show_all_apps(&ctx);
        }
        if settings {
            host::open_settings("");
        }
    }

    // ------------------------------------------------------------------ footer

    fn draw_footer(&mut self, ui: &mut Ui, cx: &mut Cx, r: Rect) {
        let t = cx.t.clone();
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let st = cx.s.clone();
        p.rect_filled(r, CornerRadius { nw: 0, ne: 0, sw: 7, se: 7 }, t.panel);
        p.line_segment([pos2(r.left(), r.top() + 0.5), pos2(r.right(), r.top() + 0.5)], Stroke::new(1.0, t.line));
        let cy = r.center().y + 0.5;
        // hints (right)
        let mut hints: Vec<(String, String)> = Vec::new();
        if self.menu.is_some() {
            hints = vec![("↑+↓".into(), "Choose".into()), ("↵".into(), "Run".into()), ("Esc".into(), "Back".into())];
        } else if let Some(it) = self.cur().cloned() {
            let acts = self.actions_for(&it, &st);
            let go = self.go_label(&it, &st);
            hints.push(("↵".into(), if go.is_empty() { "Open".into() } else { go }));
            let second = acts.iter().find(|a| a.mode == "reveal").or_else(|| acts.iter().find(|a| a.mode == "admin"));
            if let Some(sec) = second.filter(|a| !a.keys.is_empty()) {
                hints.push((sec.keys.join("+"), sec.label.replace("Open file location", "Location").replace("Show in folder", "Folder")));
            }
            if acts.len() > 1 {
                hints.push(("→".into(), "Actions".into()));
            }
            if matches!(self.mode.as_deref(), Some("path") | Some("biggest")) && truthy(&it, "complete") {
                hints.insert(0, ("Tab".into(), if self.mode.as_deref() == Some("biggest") { "Look inside".into() } else { "Complete".into() }));
            }
        } else {
            hints = vec![("Esc".into(), "Close".into())];
        }
        // the menu's "↑↓" is one key cap in the page
        let mut x = r.right() - 14.0;
        let mut items: Vec<(Vec<String>, std::sync::Arc<egui::Galley>)> = Vec::new();
        for (k, l) in &hints {
            let ks: Vec<String> = if k == "↑+↓" { vec!["↑↓".into()] } else { k.split('+').map(String::from).collect() };
            items.push((ks, theme::line(ui, l, f(12.0, 400.0), t.text3, 200.0)));
        }
        for (ks, g) in items.iter().rev() {
            let kref: Vec<&str> = ks.iter().map(|s| s.as_str()).collect();
            let kw = keys_width(ui, &kref, 6.0);
            theme::put_right(&p, g, x, cy);
            keys(ui, &t, x - g.size().x - 6.0 - kw, cy, &kref, 6.0);
            x -= g.size().x + 6.0 + kw + 14.0;
        }
        let status_right = x;
        // status (left)
        let Some(stv) = self.status.clone() else { return };
        let mut sx = r.left() + 16.0;
        let dot = |p: &egui::Painter, x: f32, c: Color32| {
            p.circle_filled(pos2(x + 3.5, cy), 3.5 + 3.0, theme::alpha(c, 0.2));
            p.circle_filled(pos2(x + 3.5, cy), 3.5, c);
        };
        let idx = &stv["indexer"];
        let mut links: Vec<(&str, String)> = Vec::new();
        let text;
        if idx["volumes"].as_array().map(|v| !v.is_empty()).unwrap_or(false) {
            let ready = idx["volumes"].as_array().unwrap().iter().all(|v| b(v, "ready"));
            let nn = fmt::count(n(idx, "total").unwrap_or(0.0));
            if ready {
                dot(&p, sx, t.ok);
                text = format!("{nn} files indexed{}", match (self.ms, self.query.is_empty()) {
                    (Some(ms), false) => format!(" · {} ms", ms as i64),
                    _ => String::new(),
                });
            } else {
                dot(&p, sx, t.warn);
                text = format!("Indexing drives… {nn} so far");
            }
        } else if b(&stv, "indexer_installed") {
            dot(&p, sx, t.warn);
            text = "File indexer not running · ".into();
            links.push(("indexer", "Start it".into()));
        } else {
            dot(&p, sx, t.warn);
            text = "Searching your user folder only".into();
        }
        if let Some(u) = stv["update"].as_str() {
            links.push(("update", format!("Update to {u}")));
        }
        sx += 7.0 + 8.0;
        let g = theme::line(ui, &text, f(12.0, 400.0), t.text3, (status_right - sx).max(10.0));
        sx += theme::put(&p, &g, sx, cy);
        for (i, (id, label)) in links.iter().enumerate() {
            if i > 0 || (*id == "update" && !text.ends_with("· ")) {
                let g = theme::line(ui, " · ", f(12.0, 400.0), t.text3, 40.0);
                sx += theme::put(&p, &g, sx, cy);
            }
            let g = theme::line(ui, label, f(12.0, 600.0), t.accent, 300.0);
            let lr = Rect::from_min_size(pos2(sx, cy - g.size().y / 2.0), g.size());
            let resp = ui.interact(lr, Id::new(("st-link", *id)), Sense::click());
            theme::put(&p, &g, sx, cy);
            if resp.hovered() {
                p.line_segment([pos2(lr.left(), lr.bottom() - 1.0), pos2(lr.right(), lr.bottom() - 1.0)], Stroke::new(1.0, t.accent));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if *id == "indexer" {
                    self.jobs.call(&ctx, Tag::StartIndexer, "install_indexer", json!({}));
                } else {
                    host::open_settings("about");
                }
            }
            sx = lr.right();
        }
    }

    // ------------------------------------------------------------------ action menu

    fn draw_menu(&mut self, ui: &mut Ui, cx: &mut Cx, inner: Rect) {
        let t = cx.t.clone();
        let ctx = ui.ctx().clone();
        let st = cx.s.clone();
        let Some(menu) = &mut self.menu else { return };
        let k = ease((menu.opened.elapsed().as_secs_f32() / 0.12).min(1.0));
        if k < 1.0 {
            ctx.request_repaint();
        }
        let title = match &menu.kind {
            MenuKind::List => s_owned(&menu.item, "title"),
            MenuKind::Keyword(_) => format!("Keyword for {}", s(&menu.item, "title")),
            MenuKind::Hotkey(_) => format!("Global shortcut for {}", s(&menu.item, "title")),
        };
        let head_h = 10.0 + 16.0 + 8.0 + 1.0;
        let body_h = match &menu.kind {
            MenuKind::List => 12.0 + menu.items.len() as f32 * 36.0,
            _ => 10.0 + 40.0 + 10.0 + 16.0 + 12.0,
        };
        let h = (head_h + body_h).min(380.0);
        let rect = Rect::from_min_size(pos2(inner.right() - 14.0 - 300.0, inner.bottom() - 48.0 - h + 6.0 * (1.0 - k)), vec2(300.0, h));
        let mut run: Option<Act> = None;
        let mut close = false;
        let mut keyword_submit: Option<String> = None;
        let area = egui::Area::new(Id::new("glint-menu")).order(egui::Order::Foreground).fixed_pos(rect.min).constrain(false);
        let resp = area.show(&ctx, |ui| {
            ui.set_opacity(k);
            ui.set_width(300.0);
            ui.set_height(h);
            let p = ui.painter().clone();
            shadow(&p, rect, 14.0, t.shadow, 18.0, 50.0);
            p.rect_filled(rect, cr(14.0), t.bg_solid);
            let g = theme::line(ui, &title, f(12.0, 400.0), t.text3, 300.0 - 28.0);
            p.galley(pos2(rect.left() + 14.0, rect.top() + 10.0), g, Color32::WHITE);
            p.line_segment([pos2(rect.left(), rect.top() + head_h - 0.5), pos2(rect.right(), rect.top() + head_h - 0.5)], Stroke::new(1.0, t.line));
            let body = Rect::from_min_max(pos2(rect.left(), rect.top() + head_h), rect.max);
            match &mut menu.kind {
                MenuKind::List => {
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink(1.0)).layout(egui::Layout::top_down(egui::Align::Min)));
                    child.set_clip_rect(body.shrink(1.0));
                    let moved = ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO);
                    let scroll = std::mem::take(&mut menu.scroll_to_sel);
                    // sub-pixel rounding must not make a list that fits show a scrollbar
                    let fits = 12.0 + menu.items.len() as f32 * 36.0 <= body.height() + 1.0;
                    let vis = if fits { egui::scroll_area::ScrollBarVisibility::AlwaysHidden } else { egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded };
                    egui::ScrollArea::vertical().id_salt("menu-list").auto_shrink([false, false]).scroll_bar_visibility(vis).show(&mut child, |ui| {
                        ui.add_space(6.0);
                        let w = ui.available_width();
                        for (i, a) in menu.items.iter().enumerate() {
                            let (rr, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
                            let r = Rect::from_min_max(pos2(rr.left() + 6.0, rr.top()), pos2(rr.right() - 6.0, rr.bottom()));
                            let resp = ui.interact(r, Id::new(("mi", i)), Sense::click());
                            // hovering only moves the highlight; it never scrolls the list
                            if resp.hovered() && moved {
                                menu.sel = i;
                            }
                            if scroll && i == menu.sel {
                                ui.scroll_to_rect(r, None);
                            }
                            if i == menu.sel {
                                ui.painter().rect_filled(r, cr(8.0), t.sel);
                            }
                            let fg = if a.danger { t.danger } else { t.text };
                            cx.a.glyph(ui, &a.icon, Rect::from_min_size(pos2(r.left() + 10.0, r.center().y - 8.0), vec2(16.0, 16.0)), if a.danger { t.danger } else { t.text2 });
                            let kref: Vec<&str> = a.keys.iter().map(|s| s.as_str()).collect();
                            let kw = if kref.is_empty() { 0.0 } else { keys_width(ui, &kref, 3.0) };
                            let g = theme::line(ui, &a.label, f(13.0, 400.0), fg, r.width() - 46.0 - kw);
                            theme::put(ui.painter(), &g, r.left() + 36.0, r.center().y);
                            if !kref.is_empty() {
                                keys(ui, &t, r.right() - 10.0 - kw, r.center().y, &kref, 3.0);
                            }
                            if resp.clicked() {
                                run = Some(a.clone());
                            }
                        }
                        ui.add_space(6.0);
                    });
                }
                MenuKind::Keyword(buf) => {
                    let bx = Rect::from_min_size(pos2(body.left() + 10.0, body.top() + 10.0), vec2(body.width() - 20.0, 40.0));
                    p.rect_filled(bx, cr(10.0), t.panel2);
                    p.rect_stroke(bx, cr(10.0), Stroke::new(1.0, t.sel_line), StrokeKind::Inside);
                    cx.a.glyph(ui, "tag", Rect::from_min_size(pos2(bx.left() + 12.0, bx.center().y - 8.0), vec2(16.0, 16.0)), t.accent);
                    let ph = s(&menu.item, "title").split_whitespace().next().unwrap_or("").to_lowercase().chars().take(6).collect::<String>();
                    let ph = format!("e.g. {}", if ph.is_empty() { "word".into() } else { ph });
                    let font = f(14.0, 500.0);
                    let tc = t.text;
                    let mut layouter = move |ui: &Ui, b: &dyn egui::TextBuffer, _w: f32| ui.fonts_mut(|f| f.layout_job(theme::job(b.as_str(), font, tc)));
                    let ir = Rect::from_min_max(pos2(bx.left() + 38.0, bx.top()), pos2(bx.right() - 12.0, bx.bottom()));
                    let te = egui::TextEdit::singleline(buf)
                        .id(Id::new("glint-kw"))
                        .frame(egui::Frame::NONE)
                        .margin(egui::Margin::ZERO)
                        .vertical_align(egui::Align::Center)
                        .char_limit(24)
                        .hint_text(theme::job(&ph, font, t.text3))
                        .layouter(&mut layouter)
                        .desired_width(ir.width());
                    let r = ui.put(ir, te);
                    if !r.has_focus() {
                        r.request_focus();
                    }
                    let (enter, esc) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Enter), i.consume_key(Modifiers::NONE, Key::Escape)));
                    if esc {
                        close = true;
                    } else if enter {
                        keyword_submit = Some(buf.trim().to_string());
                    }
                    help_line(ui, cx, &t, pos2(body.left() + 14.0, bx.bottom() + 10.0 + 8.0), &[("", "Type it in Glint and this comes first. "), ("↵", " save · "), ("Esc", " cancel")]);
                }
                MenuKind::Hotkey(text) => {
                    let bx = Rect::from_min_size(pos2(body.left() + 10.0, body.top() + 10.0), vec2(body.width() - 20.0, 40.0));
                    p.rect_filled(bx, cr(10.0), t.panel2);
                    p.rect_stroke(bx, cr(10.0), Stroke::new(1.0, t.accent), StrokeKind::Inside);
                    cx.a.glyph(ui, "keyboard", Rect::from_min_size(pos2(bx.left() + 12.0, bx.center().y - 8.0), vec2(16.0, 16.0)), t.text2);
                    let g = theme::line(ui, text, f(14.0, 500.0), t.text, bx.width() - 50.0);
                    theme::put(&p, &g, bx.left() + 38.0, bx.center().y);
                    help_line(ui, cx, &t, pos2(body.left() + 14.0, bx.bottom() + 10.0 + 8.0), &[("", "Works anywhere, even when Glint is closed. "), ("Esc", " cancel")]);
                }
            }
            p.rect_stroke(rect, cr(14.0), Stroke::new(1.0, t.line2), StrokeKind::Inside);
        });
        // a click outside closes it
        let clicked_outside = ctx.input(|i| i.pointer.any_pressed()) && ctx.input(|i| i.pointer.interact_pos()).map(|p| !rect.contains(p)).unwrap_or(false);
        let _ = resp;
        if let Some(w) = keyword_submit {
            let it = menu.item.clone();
            if w.is_empty() {
                self.menu = None;
            } else {
                self.run_action(&ctx, &st, it, Act { label: String::new(), icon: String::new(), mode: format!("keyword:{w}"), keys: vec![], danger: false });
            }
            return;
        }
        if close || clicked_outside {
            self.menu = None;
            return;
        }
        if let Some(a) = run {
            let it = menu.item.clone();
            self.run_action(&ctx, &st, it, a);
        }
    }

    // ------------------------------------------------------------------ Quick Look

    fn draw_ql(&mut self, ui: &mut Ui, cx: &mut Cx, inner: Rect) {
        let t = cx.t.clone();
        let Some(ql) = &self.ql else { return };
        let Some(it) = self.cur().cloned() else { return };
        // it covers everything and eats the clicks
        let _ = ui.interact(inner, Id::new("ql-cover"), Sense::click());
        let p = ui.painter().clone();
        p.rect_filled(inner, cr(7.0), t.bg_solid);
        let head = Rect::from_min_size(inner.min, vec2(inner.width(), 50.0));
        p.line_segment([pos2(head.left(), head.bottom() - 0.5), pos2(head.right(), head.bottom() - 0.5)], Stroke::new(1.0, t.line));
        let cy = head.center().y;
        cx.a.glyph(ui, "eye", Rect::from_min_size(pos2(head.left() + 18.0, cy - 8.0), vec2(16.0, 16.0)), t.text2);
        // keys on the right
        let mut x = head.right() - 18.0;
        for (k, l) in [("Esc", " close"), ("↵", " open · "), ("→", " step · ")].iter() {
            let g = theme::line(ui, l, f(11.5, 400.0), t.text3, 100.0);
            x -= theme::put_right(&p, &g, x, cy);
            let w = widgets::kbd_width(ui, k);
            kbd(ui, &t, x - w, cy, k);
            x -= w;
            if *k == "→" {
                let w2 = widgets::kbd_width(ui, "←");
                x -= 5.0;
                kbd(ui, &t, x - w2, cy, "←");
                x -= w2;
            }
            x -= 5.0;
        }
        let pos = theme::line(ui, &format!("{} / {}", self.sel + 1, self.flat.len()), f(12.0, 400.0), t.text3, 100.0);
        let title = theme::line(ui, s(&it, "title"), f(14.0, 600.0), t.text, (x - 20.0 - (head.left() + 44.0) - pos.size().x - 10.0).max(40.0));
        let tw = theme::put(&p, &title, head.left() + 44.0, cy);
        theme::put(&p, &pos, head.left() + 44.0 + tw + 10.0, cy);
        let body = Rect::from_min_max(pos2(inner.left() + 16.0, head.bottom() + 16.0), pos2(inner.right() - 16.0, inner.bottom() - 16.0));
        match &ql.body {
            QlBody::Loading => {
                let g = theme::line(ui, "Loading…", f(14.0, 400.0), t.text3, 300.0);
                theme::put(&p, &g, body.center().x - g.size().x / 2.0, body.center().y);
            }
            QlBody::Empty(m) => {
                let g = theme::line(ui, m, f(14.0, 400.0), t.text3, body.width());
                theme::put(&p, &g, body.center().x - g.size().x / 2.0, body.center().y);
            }
            QlBody::Text(txt) => {
                let txt = txt.clone();
                p.rect_filled(body, cr(10.0), t.panel);
                p.rect_stroke(body, cr(10.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body.shrink2(vec2(18.0, 16.0))).layout(egui::Layout::top_down(egui::Align::Min)));
                child.set_clip_rect(body.shrink(1.0));
                egui::ScrollArea::vertical().id_salt(("ql-text", s(&it, "key"))).auto_shrink([false, false]).show(&mut child, |ui| {
                    let w = ui.available_width();
                    let mut j = theme::job(&txt, fm(12.5), t.text2);
                    j.sections.iter_mut().for_each(|s| s.format.line_height = Some(19.4));
                    let g = theme::layout(ui, j, w, false);
                    let (r, _) = ui.allocate_exact_size(g.size(), Sense::hover());
                    ui.painter().galley(r.min, g, Color32::WHITE);
                });
            }
            QlBody::Picture(key, play) => {
                let (key, play) = (key.clone(), *play);
                // no thumbnail to show: the info card, like other files
                if cx.a.failed(&key) && !key.starts_with("file:") {
                    if let Some(it) = self.cur().cloned() {
                        let k = s_owned(&it, "key");
                        self.jobs.call(ui.ctx(), Tag::QlDetails(k), "details", json!({"item": it}));
                        if let Some(q) = &mut self.ql {
                            q.body = QlBody::Loading;
                        }
                    }
                    return;
                }
                if let Some(pic) = cx.a.image(ui.ctx(), &key) {
                    let sz = pic.tex.size_vec2();
                    let ppp = ui.ctx().pixels_per_point();
                    // never blow small pictures up past their real size
                    let natural = vec2(pic.dims[0] as f32, pic.dims[1] as f32) / ppp;
                    let limit = body.size().min(natural);
                    let sc = (limit.x / sz.x).min(limit.y / sz.y);
                    let r = Rect::from_center_size(body.center(), sz * sc);
                    widgets::shadow(&p, r, 8.0, Color32::from_black_alpha(90), 10.0, 40.0);
                    egui::Image::new((pic.tex.id(), r.size())).corner_radius(cr(8.0)).paint_at(ui, r);
                    if play {
                        p.circle_filled(r.center(), 30.0, Color32::from_black_alpha(140));
                        cx.a.glyph(ui, "play", Rect::from_center_size(r.center() + vec2(2.0, 0.0), vec2(28.0, 28.0)), Color32::WHITE);
                    }
                } else if cx.a.failed(&key) {
                    let g = theme::line(ui, "Nothing to show", f(14.0, 400.0), t.text3, 300.0);
                    theme::put(&p, &g, body.center().x - g.size().x / 2.0, body.center().y);
                }
            }
            QlBody::Audio(path) => {
                let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                cx.a.glyph(ui, "volume", Rect::from_center_size(body.center() - vec2(0.0, 30.0), vec2(64.0, 64.0)), t.accent);
                let g = theme::line(ui, &format!("{name} · press Enter to play"), f(13.0, 400.0), t.text3, body.width());
                theme::put(&p, &g, body.center().x - g.size().x / 2.0, body.center().y + 30.0);
            }
            QlBody::Info(d) => {
                let d = d.clone();
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Center)));
                child.set_clip_rect(body);
                egui::ScrollArea::vertical().id_salt(("ql-info", s(&it, "key"))).auto_shrink([false, false]).show(&mut child, |ui| {
                    let w = ui.available_width().min(560.0);
                    ui.add_space(body.height() * 0.04 + 4.0);
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(w);
                        self.ql_info(ui, cx, &it, d.as_ref(), w);
                    });
                });
            }
        }
    }

    fn ql_info(&mut self, ui: &mut Ui, cx: &mut Cx, it: &Value, d: Option<&Value>, w: f32) {
        let t = cx.t.clone();
        let mut drew = false;
        if let Some(th) = d.and_then(|d| d["thumb"].as_str()) {
            drew = media_box(ui, cx, th, w, 320.0, false);
        }
        if !drew {
            let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            let ib = Rect::from_center_size(r.center(), vec2(96.0, 96.0));
            let mut done = false;
            if truthy(it, "icon") {
                if let Some(pic) = cx.a.image(ui.ctx(), s(it, "icon")) {
                    paint_contain(ui, &pic.tex, pic.tex.size_vec2(), Rect::from_center_size(ib.center(), vec2(88.0, 88.0)), 0.0);
                    done = true;
                }
            }
            if !done {
                cx.a.glyph(ui, kind_glyph(s(it, "kind")).unwrap_or("file"), Rect::from_center_size(ib.center(), vec2(56.0, 56.0)), t.text2);
            }
        }
        ui.add_space(12.0);
        text_block(ui, s(it, "title"), f(15.0, 600.0), t.text, w, true);
        if let Some(d) = d {
            let mut fs: Vec<(String, String)> = Vec::new();
            if d["is_dir"].as_bool() == Some(false) {
                fs.push(("Size".into(), fmt::size(n(d, "size").unwrap_or(0.0))));
            } else if n(d, "total").unwrap_or(0.0) > 0.0 {
                fs.push(("Size".into(), fmt::size(n(d, "total").unwrap_or(0.0))));
            }
            for (k, v) in [("Modified", fmt::date(n(d, "modified").unwrap_or(0.0))), ("Opens with", s_owned(d, "opens_with")), ("Where", s_owned(d, "pretty"))] {
                if !v.is_empty() {
                    fs.push((k.into(), v));
                }
            }
            ui.add_space(12.0);
            facts(ui, cx, &fs, w);
            if truthy(d, "text") {
                ui.add_space(12.0);
                pv_text(ui, cx, s(d, "text"), w, 220.0, false);
            }
            if let Some(kids) = d["children"].as_array().filter(|a| !a.is_empty()) {
                ui.add_space(12.0);
                let half = (w - 16.0) / 2.0;
                let rows = kids.len().div_ceil(2);
                let (r, _) = ui.allocate_exact_size(vec2(w, rows as f32 * 22.0), Sense::hover());
                for (i, c) in kids.iter().enumerate() {
                    let x = r.left() + (i % 2) as f32 * (half + 16.0);
                    let y = r.top() + (i / 2) as f32 * 22.0 + 11.0;
                    cx.a.glyph(ui, if b(c, "dir") { "folder" } else { "file" }, Rect::from_min_size(pos2(x, y - 7.5), vec2(15.0, 15.0)), t.text2);
                    let g = theme::line(ui, s(c, "name"), f(12.5, 400.0), t.text2, half - 24.0);
                    theme::put(ui.painter(), &g, x + 23.0, y);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------- helpers

fn test_json(var: &str) -> Option<Value> {
    let p = std::env::var(var).ok()?;
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}

fn s_owned(v: &Value, k: &str) -> String {
    s(v, k).to_string()
}

/// The key name the global-shortcut parser understands.
fn hotkey_name(k: Key) -> String {
    match k {
        Key::ArrowUp => "Up".into(),
        Key::ArrowDown => "Down".into(),
        Key::ArrowLeft => "Left".into(),
        Key::ArrowRight => "Right".into(),
        Key::Space => "Space".into(),
        Key::Enter => "Enter".into(),
        Key::Backtick => "Backquote".into(),
        Key::Minus => "Minus".into(),
        Key::Equals | Key::Plus => "Equal".into(),
        Key::Comma => "Comma".into(),
        Key::Period => "Period".into(),
        Key::Slash => "Slash".into(),
        Key::Backslash => "Backslash".into(),
        Key::Semicolon => "Semicolon".into(),
        Key::Quote => "Quote".into(),
        Key::OpenBracket => "BracketLeft".into(),
        Key::CloseBracket => "BracketRight".into(),
        Key::Delete => "Delete".into(),
        Key::Insert => "Insert".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PageUp".into(),
        Key::PageDown => "PageDown".into(),
        Key::Backspace => "Backspace".into(),
        other => other.name().to_string(),
    }
}

/// Mark the typed words in a title (or the letters in order, for initials).
pub fn highlight(text: &str, q: &str, font: Font, t: &Theme) -> LayoutJob {
    let chars: Vec<char> = text.chars().collect();
    let lower: Vec<char> = text.to_lowercase().chars().collect();
    let ql = q.to_lowercase();
    let words: Vec<&str> = ql.split_whitespace().filter(|w| !w.contains(':') && !w.starts_with('*') && !w.starts_with('>')).collect();
    let mut marks = vec![false; chars.len()];
    if !words.is_empty() && lower.len() == chars.len() {
        let mut any = false;
        for w in &words {
            let wc: Vec<char> = w.chars().collect();
            if let Some(i) = (0..lower.len()).find(|&i| lower[i..].starts_with(&wc)) {
                for m in marks.iter_mut().skip(i).take(wc.len()) {
                    *m = true;
                }
                any = true;
            }
        }
        if !any && words.len() == 1 {
            // initials / letters in order
            let wc: Vec<char> = words[0].chars().collect();
            let mut j = 0;
            for i in 0..chars.len() {
                if j < wc.len() && lower[i] == wc[j] {
                    marks[i] = true;
                    j += 1;
                }
            }
            if j < wc.len() {
                marks.iter_mut().for_each(|m| *m = false);
            }
        }
    }
    let mut j = LayoutJob::default();
    let normal = theme::format(font, t.text);
    let marked = theme::format(Font { weight: 650.0, ..font }, t.mark);
    let mut i = 0;
    while i < chars.len() {
        let m = marks[i];
        let start = i;
        while i < chars.len() && marks[i] == m {
            i += 1;
        }
        let seg: String = chars[start..i].iter().collect();
        j.append(&seg, 0.0, if m { marked.clone() } else { normal.clone() });
    }
    j
}

/// A rounded box with a line icon (`.ico.glyphy`, optionally `.accent`).
fn glyph_box(ui: &mut Ui, cx: &mut Cx, r: Rect, icon: &str, accent: bool, radius: f32, size: f32) {
    let t = cx.t;
    let p = ui.painter();
    let (bg, line, fg) = if accent { (theme::alpha(t.accent, 0.14), theme::alpha(t.accent, 0.30), t.accent) } else { (t.panel2, t.line, t.text2) };
    p.rect_filled(r, cr(radius), bg);
    p.rect_stroke(r, cr(radius), Stroke::new(1.0, line), StrokeKind::Inside);
    cx.a.glyph(ui, icon, Rect::from_center_size(r.center(), vec2(size, size)), fg);
}

/// object-fit: contain.
fn paint_contain(ui: &Ui, tex: &egui::TextureHandle, size: egui::Vec2, r: Rect, radius: f32) {
    let sc = (r.width() / size.x).min(r.height() / size.y);
    let rr = Rect::from_center_size(r.center(), size * sc);
    egui::Image::new((tex.id(), rr.size())).corner_radius(cr(radius)).paint_at(ui, rr);
}

/// object-fit: cover, clipped to a rounded box.
fn paint_cover(ui: &Ui, tex: &egui::TextureHandle, r: Rect, radius: f32) {
    let s = tex.size_vec2();
    let (ra, ia) = (r.width() / r.height(), s.x / s.y);
    let uv = if ia > ra {
        let w = ra / ia;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ia / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    };
    egui::Image::new((tex.id(), r.size())).uv(uv).corner_radius(cr(radius)).paint_at(ui, r);
}

fn gradient(p: &egui::Painter, r: Rect, a: Color32, b2: Color32) {
    if r.width() <= 0.0 {
        return;
    }
    let mut m = egui::Mesh::default();
    m.colored_vertex(r.left_top(), a);
    m.colored_vertex(r.right_top(), b2);
    m.colored_vertex(r.right_bottom(), b2);
    m.colored_vertex(r.left_bottom(), a);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(m);
}

/// A rounded box filled with a 135° gradient.
fn diag_gradient(p: &egui::Painter, r: Rect, radius: f32, a: Color32, b2: Color32) {
    // build the rounded outline, colour each point by its position along the diagonal
    let shape = egui::epaint::RectShape::filled(r, cr(radius), Color32::WHITE);
    let mut mesh = egui::Mesh::default();
    let mut tess = egui::epaint::Tessellator::new(1.0, Default::default(), [1, 1], vec![]);
    tess.tessellate_rect(&shape, &mut mesh);
    let span = r.width() + r.height();
    for v in &mut mesh.vertices {
        let k = (((v.pos.x - r.left()) + (v.pos.y - r.top())) / span).clamp(0.0, 1.0);
        let c = a.lerp_to_gamma(b2, k);
        let alpha = v.color.a() as f32 / 255.0;
        v.color = c.gamma_multiply(alpha);
    }
    p.add(mesh);
}

fn text_block(ui: &mut Ui, text: &str, font: Font, color: Color32, w: f32, center: bool) {
    if text.is_empty() {
        return;
    }
    let mut j = theme::job(text, font, color);
    if center {
        j.halign = egui::Align::Center;
    }
    j.sections.iter_mut().for_each(|s| s.format.line_height = Some(font.size * 1.3));
    let g = theme::layout(ui, j, w, false);
    let (r, _) = ui.allocate_exact_size(vec2(w, g.size().y), Sense::hover());
    let x = if center { r.center().x } else { r.left() };
    ui.painter().galley(pos2(x, r.top()), g, Color32::WHITE);
}

/// `.pv-facts`: key on the left, value on the right.
fn facts(ui: &mut Ui, cx: &mut Cx, fs: &[(String, String)], w: f32) {
    let t = cx.t;
    let (top, _) = ui.allocate_exact_size(vec2(w, 10.0), Sense::hover());
    ui.painter().line_segment([pos2(top.left(), top.top() + 0.5), pos2(top.right(), top.top() + 0.5)], Stroke::new(1.0, t.line));
    for (k, v) in fs {
        let kg = theme::line(ui, k, f(12.0, 400.0), t.text3, w);
        let vw = (w - kg.size().x - 12.0).max(40.0);
        let mut j = theme::job(v, f(12.0, 400.0), t.text2);
        j.halign = egui::Align::RIGHT;
        let vg = theme::layout(ui, j, vw, false);
        let h = vg.size().y.max(kg.size().y) + 10.0;
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        ui.painter().galley(pos2(r.left(), r.top() + 5.0), kg, Color32::WHITE);
        ui.painter().galley(pos2(r.right(), r.top() + 5.0), vg, Color32::WHITE);
    }
}

/// `.pv-text`: monospace, boxed, cut off with a fade.
fn pv_text(ui: &mut Ui, cx: &mut Cx, text: &str, w: f32, max_h: f32, fade: bool) {
    let t = cx.t;
    let text: String = text.chars().take(4000).collect();
    let mut j = theme::job(&text, fm(11.5), t.text2);
    j.sections.iter_mut().for_each(|s| s.format.line_height = Some(17.8));
    let g = theme::layout(ui, j, w - 24.0, false);
    let h = (g.size().y + 20.0 + 2.0).min(max_h);
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, cr(10.0), t.panel);
    p.rect_stroke(r, cr(10.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
    let clip = r.shrink(1.0);
    let pos = pos2(r.left() + 12.0, r.top() + 10.0);
    if fade && g.size().y + 22.0 > max_h * 0.75 {
        // mask-image: linear-gradient(black 75%, transparent)
        let solid = r.top() + h * 0.75;
        p.with_clip_rect(Rect::from_min_max(clip.min, pos2(clip.right(), solid)).intersect(ui.clip_rect())).galley(pos, g.clone(), Color32::WHITE);
        let bands = 8;
        let bh = (r.bottom() - solid) / bands as f32;
        for i in 0..bands {
            let band = Rect::from_min_max(pos2(clip.left(), solid + i as f32 * bh), pos2(clip.right(), solid + (i + 1) as f32 * bh));
            let mut pp = p.with_clip_rect(band.intersect(clip).intersect(ui.clip_rect()));
            pp.multiply_opacity(1.0 - (i as f32 + 0.5) / bands as f32);
            pp.galley(pos, g.clone(), Color32::WHITE);
        }
    } else {
        p.with_clip_rect(clip.intersect(ui.clip_rect())).galley(pos, g, Color32::WHITE);
    }
}

/// `.pv-snippet`: the matching passage, words marked (offsets are UTF-8 bytes).
fn snippet(ui: &mut Ui, cx: &mut Cx, sn: &Value, w: f32) {
    let t = cx.t;
    let text = s(sn, "text");
    let font = f(12.5, 400.0);
    let normal = {
        let mut tf = theme::format(font, t.text2);
        tf.line_height = Some(19.4);
        tf
    };
    let mut marked = theme::format(font, t.text);
    marked.background = theme::alpha(t.accent, 0.35);
    marked.line_height = Some(19.4);
    let mut j = LayoutJob::default();
    let mut last = 0usize;
    for m in sn["marks"].as_array().cloned().unwrap_or_default() {
        let (a, b2) = (m[0].as_u64().unwrap_or(0) as usize, m[1].as_u64().unwrap_or(0) as usize);
        if a < last || b2 > text.len() || !text.is_char_boundary(a) || !text.is_char_boundary(b2) {
            continue;
        }
        j.append(&text[last..a], 0.0, normal.clone());
        j.append(&text[a..b2], 0.0, marked.clone());
        last = b2;
    }
    j.append(&text[last.min(text.len())..], 0.0, normal);
    let g = theme::layout(ui, j, w - 24.0, false);
    let h = (g.size().y + 22.0).min(240.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, cr(10.0), t.panel);
    p.rect_stroke(r, cr(10.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
    p.with_clip_rect(r.shrink(1.0).intersect(ui.clip_rect())).galley(pos2(r.left() + 12.0, r.top() + 10.0), g, Color32::WHITE);
}

fn children(ui: &mut Ui, cx: &mut Cx, list: &[Value], w: f32) {
    let t = cx.t;
    for c in list {
        let (r, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        cx.a.glyph(ui, if b(c, "dir") { "folder" } else { "file" }, Rect::from_min_size(pos2(r.left(), r.center().y - 7.0), vec2(14.0, 14.0)), t.text3);
        let g = theme::line(ui, s(c, "name"), f(12.0, 400.0), t.text2, w - 22.0);
        theme::put(ui.painter(), &g, r.left() + 22.0, r.center().y);
    }
}

/// A picture box (`.pv-media`/`.pv-thumb`): full width, up to `max_h`, contained. Returns
/// whether it drew (so the caller can fall back to the icon when the picture fails).
fn media_box(ui: &mut Ui, cx: &mut Cx, key: &str, w: f32, max_h: f32, play: bool) -> bool {
    let t = cx.t;
    let pic = cx.a.image(ui.ctx(), key);
    if pic.is_none() && cx.a.failed(key) {
        return false;
    }
    let (sz, tex) = match &pic {
        Some(p) => (p.tex.size_vec2(), Some(p.tex.clone())),
        None => (vec2(16.0, 9.0), None),
    };
    let h = (w * sz.y / sz.x).min(max_h);
    let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, cr(12.0), if play { Color32::BLACK } else { t.panel2 });
    if let Some(tex) = tex {
        paint_contain(ui, &tex, sz, r.shrink(1.0), 11.0);
    }
    ui.painter().rect_stroke(r, cr(12.0), Stroke::new(1.0, t.line), StrokeKind::Inside);
    if play {
        ui.painter().circle_filled(r.center(), 22.0, Color32::from_black_alpha(140));
        cx.a.glyph(ui, "play", Rect::from_center_size(r.center() + vec2(1.5, 0.0), vec2(20.0, 20.0)), Color32::WHITE);
    }
    true
}

/// "text <kbd>k</kbd> text" on one line (the keyword/shortcut help).
fn help_line(ui: &Ui, _cx: &mut Cx, t: &Theme, at: Pos2, parts: &[(&str, &str)]) {
    let mut x = at.x;
    let cy = at.y;
    for (k, txt) in parts {
        if !k.is_empty() {
            x += kbd(ui, t, x, cy, k);
        }
        let g = theme::line(ui, txt, f(11.5, 400.0), t.text3, 300.0);
        x += theme::put(ui.painter(), &g, x, cy);
    }
}

fn screen_cursor() -> (i32, i32) {
    let mut p = windows::Win32::Foundation::POINT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
    }
    (p.x, p.y)
}
