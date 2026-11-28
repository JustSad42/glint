//! Glint: a fast launcher and search for Windows.
//!   Glint.exe                  open the search window (or settings on first run)
//!   Glint.exe --tray           start in the background (used at sign-in)
//!   Glint.exe --settings       open settings
//!   Glint.exe --quit           close the running Glint (handy when the tray icon is hidden)
//!   Glint.exe --setup          installer: turn on start-with-Windows
//!   Glint.exe --remove-integration   uninstaller: undo --setup
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod apps;
mod audio;
mod browser;
mod calc;
mod clipvault;
mod content;
mod extract;
mod files;
mod fuzzy;
mod history;
mod hook;
mod host;
mod hotkeys;
mod icons;
mod media;
mod pins;
mod preview;
mod quick;
mod scripts;
mod search;
mod settings;
mod shellops;
mod sysinfo;
mod taskbar;
mod timers;
mod ui;
mod updater;
mod winmgmt;
mod wins;

use parking_lot::Mutex;
use serde_json::{json, Value};
use std::time::Duration;

static UPDATE: Mutex<Option<String>> = Mutex::new(None);
/// notices on screen (timers, reminders, script results)
pub static NOTICES: Mutex<Vec<Value>> = Mutex::new(Vec::new());

fn arg_str<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("")
}

/// Show a notice in the bottom-right corner (timer done, reminder, script result).
pub fn notice(mut n: Value) {
    {
        let mut l = NOTICES.lock();
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0) + l.len() as u64;
        n["id"] = json!(id);
        l.push(n);
        while l.len() > 4 {
            l.remove(0);
        }
    }
    host::notices_changed();
}

pub fn dismiss_notice(id: u64) -> Option<Value> {
    let gone = {
        let mut l = NOTICES.lock();
        let i = l.iter().position(|n| n["id"].as_u64() == Some(id));
        i.map(|i| l.remove(i))
    };
    host::notices_changed();
    gone
}

/// Strip what shouldn't be remembered in history.
fn clean_item(item: &Value) -> Value {
    let mut it = item.clone();
    if let Some(o) = it.as_object_mut() {
        for k in ["score", "terms", "recent", "confirm"] {
            o.remove(k);
        }
    }
    it
}

fn glint_command(cmd: &str) -> Value {
    match cmd {
        "settings" => {
            host::open_settings("");
            json!({"hide": true})
        }
        "updates" => {
            host::open_settings("about");
            json!({"hide": true})
        }
        "rebuild" => {
            content::rebuild();
            json!({"hide": false, "toast": "Reading your documents again in the background"})
        }
        "apps" => {
            icons::clear_disk_cache();
            apps::refresh(true);
            json!({"hide": false, "toast": "Refreshing the app list"})
        }
        "history" => {
            history::clear();
            json!({"hide": false, "toast": "History cleared", "refresh": true})
        }
        "terminal" => {
            let _ = shellops::terminal(&settings::user_home().to_string_lossy());
            json!({"hide": true})
        }
        "start" => {
            host::hide_launcher();
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(120));
                hook::open_start_menu();
            });
            json!({"hide": true})
        }
        "quit" => {
            host::quit();
            json!({"hide": true})
        }
        "scripts" => {
            let d = scripts::ensure_dir();
            let _ = shellops::open(&d.to_string_lossy());
            json!({"hide": true})
        }
        _ => json!({"hide": false}),
    }
}

/// Do what the user picked. `mode` is the chosen action (default, reveal, admin, copy-path...).
pub fn activate(item: &Value, mode: &str, query: &str) -> Result<Value, String> {
    let kind = arg_str(item, "kind");
    let key = arg_str(item, "key");
    let path = arg_str(item, "path");
    let done = |toast: Option<&str>, hide: bool| Ok(json!({"hide": hide, "toast": toast}));
    let remember = || history::record(key, query, clean_item(item));
    match (kind, mode) {
        // copying never hides the window, so you can copy several things
        (_, "copy-path") => {
            let p = if path.is_empty() { arg_str(item, "lnk") } else { path };
            shellops::copy_text(p)?;
            done(Some("Path copied"), false)
        }
        (_, "copy-name") => {
            let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            shellops::copy_text(&name)?;
            done(Some("Name copied"), false)
        }
        (_, "copy-file") => {
            shellops::copy_files(&[path.to_string()])?;
            done(Some("Copied — paste it in a folder or chat"), false)
        }
        (_, "pin") => {
            pins::pin(&clean_item(item));
            Ok(json!({"hide": false, "toast": "Pinned — it shows first when Glint opens", "refresh": true}))
        }
        (_, "unpin") => {
            pins::unpin(key);
            Ok(json!({"hide": false, "toast": "Unpinned", "refresh": true}))
        }
        (_, m) if m.starts_with("keyword:") => {
            let w = pins::set_keyword(&m[8..], &clean_item(item))?;
            Ok(json!({"hide": false, "toast": format!("Type \"{w}\" to open it"), "refresh": true}))
        }
        (_, m) if m.starts_with("unkeyword:") => {
            pins::remove_keyword(&m[10..]);
            Ok(json!({"hide": false, "toast": "Keyword removed", "refresh": true}))
        }
        (_, m) if m.starts_with("hotkey:") => {
            let combo = hotkeys::normalize(&m[7..])?;
            pins::set_hotkey(&combo, &clean_item(item));
            hotkeys::apply()?;
            Ok(json!({"hide": false, "toast": format!("{combo} now opens it, from anywhere"), "refresh": true}))
        }
        (_, "unhotkey") => {
            if let Some(c) = pins::hotkey_for(key) {
                pins::remove_hotkey(&c);
                let _ = hotkeys::apply();
            }
            Ok(json!({"hide": false, "toast": "Shortcut removed", "refresh": true}))
        }
        (_, "forget") => {
            history::forget(key);
            Ok(json!({"hide": false, "toast": "Removed from Recent", "refresh": true}))
        }
        ("app", "default") => {
            shellops::shell_execute(&format!("shell:AppsFolder\\{}", arg_str(item, "id")), None, None, None)?;
            remember();
            done(None, true)
        }
        ("app", "admin") => {
            shellops::shell_execute(&format!("shell:AppsFolder\\{}", arg_str(item, "id")), Some("runas"), None, None)?;
            remember();
            done(None, true)
        }
        ("app", "reveal") => {
            let p = if !path.is_empty() { path } else { arg_str(item, "lnk") };
            if p.is_empty() {
                return Err("This app has no file location (it's a Store app)".into());
            }
            shellops::reveal(p)?;
            done(None, true)
        }
        ("app", "properties") => {
            let p = if !arg_str(item, "lnk").is_empty() { arg_str(item, "lnk") } else { path };
            shellops::properties(p)?;
            done(None, true)
        }
        ("file" | "folder" | "content", m) => {
            if !std::path::Path::new(path).exists() {
                history::forget(key);
                return Err("That file doesn't exist anymore".into());
            }
            match m {
                "default" => {
                    shellops::open(path)?;
                    remember();
                }
                "reveal" => shellops::reveal(path)?,
                "admin" => shellops::run_as_admin(path)?,
                "properties" => shellops::properties(path)?,
                "open-with" => shellops::open_with(path)?,
                "terminal" => shellops::terminal(path)?,
                "vscode" => shellops::open_in_vscode(path)?,
                "explorer" => shellops::open_in_explorer(path)?,
                "copy-image" => {
                    clipvault::copy_image(path)?;
                    return Ok(json!({"hide": false, "toast": "Image copied — paste it anywhere"}));
                }
                "biggest" => return Ok(json!({"hide": false, "query": format!("biggest {}\\", path.trim_end_matches('\\'))})),
                "recycle" => {
                    shellops::recycle(path)?;
                    history::forget(key);
                    return Ok(json!({"hide": false, "toast": "Moved to the Recycle Bin", "refresh": true}));
                }
                _ => return Err(format!("Unknown action {m}")),
            }
            done(None, true)
        }
        ("window", "close") => {
            wins::close(item["hwnd"].as_i64().unwrap_or(0) as isize);
            Ok(json!({"hide": false, "toast": "Window closed", "refresh": true}))
        }
        ("window", "reveal") => {
            shellops::reveal(path)?;
            done(None, true)
        }
        ("window", _) => {
            host::hide_launcher();
            wins::activate(item["hwnd"].as_i64().unwrap_or(0) as isize);
            done(None, true)
        }
        ("process", _) => {
            let pids: Vec<u32> = item["pids"].as_array().map(|a| a.iter().filter_map(|p| p.as_u64().map(|x| x as u32)).collect()).unwrap_or_default();
            let n = wins::kill(&pids);
            Ok(json!({"hide": false, "toast": if n > 0 { format!("Ended {n} process{}", if n == 1 { "" } else { "es" }) } else { "Couldn't end it (it may need admin rights)".into() }, "refresh": true}))
        }
        ("calc" | "color", m) => {
            let text = if m.starts_with("alt:") { m[4..].to_string() } else { arg_str(item, "copy").to_string() };
            shellops::copy_text(&text)?;
            done(Some(&format!("Copied {text}")), false)
        }
        ("timer", _) => {
            let msg = timers::run(item)?;
            Ok(json!({"hide": false, "toast": msg, "refresh": true}))
        }
        ("tab", _) => {
            host::hide_launcher();
            browser::activate_tab(item["hwnd"].as_i64().unwrap_or(0) as isize, arg_str(item, "title"), item["pos"].as_u64().unwrap_or(0) as usize)?;
            done(None, true)
        }
        ("script", m) => {
            let (path, args) = (arg_str(item, "path"), arg_str(item, "args"));
            remember();
            if m == "admin" || m == "terminal" {
                scripts::run_in_terminal(path, args)?;
                return done(None, true);
            }
            let name = arg_str(item, "title").to_string();
            scripts::run_hidden(path, args, move |r| {
                let (body, ok) = match r {
                    Ok(t) => (t, true),
                    Err(e) => (e, false),
                };
                notice(json!({"title": name, "body": body, "kind": if ok { "ok" } else { "err" }}));
            });
            done(Some(&format!("Running {}…", arg_str(item, "title"))), true)
        }
        ("toggle", _) => {
            let msg = sysinfo::run(arg_str(item, "op"))?;
            Ok(json!({"hide": false, "toast": msg, "refresh": true}))
        }
        ("info", _) => {
            if arg_str(item, "op") == "public_ip" {
                let ip = sysinfo::public_ip()?;
                shellops::copy_text(&ip)?;
                return done(Some(&format!("Public IP {ip} copied")), false);
            }
            let text = arg_str(item, "copy");
            if text.is_empty() {
                return done(None, false);
            }
            shellops::copy_text(text)?;
            done(Some(&format!("Copied {text}")), false)
        }
        ("layout", _) => {
            host::hide_launcher();
            let op = arg_str(item, "op").to_string();
            std::thread::sleep(Duration::from_millis(60));
            winmgmt::apply(&op)?;
            done(None, true)
        }
        ("media", _) => {
            let msg = media::control(arg_str(item, "op"))?;
            Ok(json!({"hide": false, "toast": msg, "refresh": true}))
        }
        ("clip", m) => {
            let id = arg_str(item, "id");
            if m == "copy" || !settings::load().clip_paste {
                let msg = clipvault::restore(id, false)?;
                return done(Some(&msg), false);
            }
            host::hide_launcher();
            clipvault::restore(id, true)?;
            done(None, true)
        }
        ("url" | "web", _) => {
            shellops::open_url(arg_str(item, "url"))?;
            done(None, true)
        }
        ("command", m) => {
            shellops::run_command(arg_str(item, "command"), m == "admin")?;
            remember();
            done(None, true)
        }
        ("action", _) => {
            remember();
            match actions::run(arg_str(item, "id"))? {
                actions::Outcome::Done(msg) => Ok(json!({"hide": msg.is_none(), "toast": msg})),
                actions::Outcome::Glint(cmd) => Ok(glint_command(cmd)),
            }
        }
        _ => Err("Nothing to do".into()),
    }
}

fn status() -> Value {
    let idx = files::status();
    let c = content::STATUS.lock().clone();
    json!({
        "version": updater::VERSION,
        "indexer": idx,
        "indexer_installed": settings::exe_dir().join("glint-indexer.exe").exists(),
        "content": c,
        "apps": apps::count(),
        "update": *UPDATE.lock(),
        "data_dir": settings::DATA_DIR.to_string_lossy(),
        "home": settings::user_home().to_string_lossy(),
        "installed": updater::is_installed(),
        "vscode": shellops::vscode().is_some(),
        "clipvault": clipvault::available(),
        "user": std::env::var("USERNAME").unwrap_or_default(),
    })
}

pub fn handle(cmd: &str, a: Value) -> Result<Value, String> {
    Ok(match cmd {
        "search" => search::run(arg_str(&a, "q"), if arg_str(&a, "filter").is_empty() { "all" } else { arg_str(&a, "filter") }),
        "details" => search::details(&a["item"]),
        "activate" => activate(&a["item"], if arg_str(&a, "mode").is_empty() { "default" } else { arg_str(&a, "mode") }, arg_str(&a, "q"))?,
        "hide" => {
            host::hide_launcher();
            json!(true)
        }
        "queries" => json!(history::queries()),
        "pins_list" => pins::list(),
        "hotkey_remove" => {
            pins::remove_hotkey(arg_str(&a, "combo"));
            let _ = hotkeys::apply();
            json!(true)
        }
        "quicklook" => {
            host::quicklook(a["on"].as_bool().unwrap_or(false));
            json!(true)
        }
        "all_apps" => json!(apps::all()),
        "read_text" => {
            // Quick Look: the start of a text file, decoded leniently
            use std::io::Read;
            let mut buf = Vec::new();
            std::fs::File::open(arg_str(&a, "path")).map_err(|e| e.to_string())?.take(400_000).read_to_end(&mut buf).map_err(|e| e.to_string())?;
            json!(String::from_utf8_lossy(&buf))
        }
        "start_home" => quick::start_home(),
        "open_scripts" => {
            let d = scripts::ensure_dir();
            shellops::open(&d.to_string_lossy())?;
            json!(true)
        }
        "notices" => json!(*NOTICES.lock()),
        "notice_dismiss" => {
            dismiss_notice(a["id"].as_u64().unwrap_or(0));
            json!(true)
        }
        "notice_snooze" => {
            if let Some(n) = dismiss_notice(a["id"].as_u64().unwrap_or(0)) {
                let mins = a["mins"].as_i64().unwrap_or(5).max(1);
                let label = n["timer"]["label"].as_str().or(n["title"].as_str()).unwrap_or("").to_string();
                let due = chrono::Local::now().timestamp() + mins * 60;
                timers::add(due, &label, "reminder", 0);
            }
            json!(true)
        }
        "keyword_remove" => {
            pins::remove_keyword(arg_str(&a, "word"));
            json!(true)
        }
        "unpin" => {
            pins::unpin(arg_str(&a, "key"));
            json!(true)
        }
        "status" => status(),
        "settings_get" => {
            let mut v = serde_json::to_value(settings::load()).unwrap();
            v["_version"] = json!(updater::VERSION);
            v["_filepilot_found"] = json!(!settings::default_filepilot().is_empty());
            v["_user"] = json!(std::env::var("USERNAME").unwrap_or_default());
            v
        }
        "settings_set" => {
            let old = settings::load();
            let mut cur = serde_json::to_value(&old).unwrap();
            if let (Some(c), Some(n)) = (cur.as_object_mut(), a.as_object()) {
                for (k, v) in n {
                    if !k.starts_with('_') {
                        c.insert(k.clone(), v.clone());
                    }
                }
            }
            let s: settings::Settings = serde_json::from_value(cur).map_err(|e| e.to_string())?;
            settings::save(&s)?;
            hook::apply(&s);
            if s.start_with_windows != old.start_with_windows {
                settings::set_autostart(s.start_with_windows);
            }
            if s.content_roots != old.content_roots || s.content_excludes != old.content_excludes || s.content_max_mb != old.content_max_mb {
                content::restart();
            }
            // the tray icon, the window backdrop and the theme follow in the UI's next pass
            host::settings_changed();
            json!({"ok": true})
        }
        "open_settings" => {
            host::open_settings(arg_str(&a, "section"));
            json!(true)
        }
        "pick_folder" => json!(rfd::FileDialog::new().pick_folder().map(|p| p.to_string_lossy().into_owned())),
        "pick_file" => json!(rfd::FileDialog::new().add_filter("Programs", &["exe"]).pick_file().map(|p| p.to_string_lossy().into_owned())),
        "rebuild" => {
            content::rebuild();
            json!(true)
        }
        "clear_history" => {
            history::clear();
            json!(true)
        }
        "install_indexer" => {
            let exe = settings::exe_dir().join("glint-indexer.exe");
            if !exe.exists() {
                return Err("The indexer isn't installed next to Glint. Reinstall Glint.".into());
            }
            shellops::shell_execute(&exe.to_string_lossy(), Some("runas"), Some("--install"), None)?;
            json!(true)
        }
        "update_check" => updater::check(a["force"].as_bool().unwrap_or(false)),
        "update_install" => {
            let v = updater::install(&|_| {})?;
            json!({"version": v})
        }
        "open_url" => {
            shellops::open_url(arg_str(&a, "url"))?;
            json!(true)
        }
        "open_path" => {
            shellops::open(arg_str(&a, "path"))?;
            json!(true)
        }
        "show" => {
            host::close_settings();
            host::show_launcher();
            json!(true)
        }
        "close_settings" => {
            host::close_settings();
            json!(true)
        }
        "quit" => {
            host::quit();
            json!(true)
        }
        _ => return Err(format!("unknown command {cmd}")),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    if has("--setup") {
        let mut s = settings::load();
        s.start_with_windows = true;
        let _ = settings::save(&s);
        settings::set_autostart(true);
        return;
    }
    if has("--remove-integration") {
        settings::set_autostart(false);
        return;
    }
    // test hook: run one call and print its JSON (used to compare the old and new UI)
    if let Some(i) = args.iter().position(|a| a == "--call") {
        let cmd = args.get(i + 1).cloned().unwrap_or_default();
        let a: Value = args.get(i + 2).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(json!({}));
        if cmd == "icon" {
            // write one icon/thumbnail (by its gicon path) to a PNG file
            let (tx, rx) = crossbeam_channel::bounded(1);
            icons::request(arg_str(&a, "path"), move |png| {
                let _ = tx.send(png);
            });
            if let Ok(Some(png)) = rx.recv_timeout(Duration::from_secs(10)) {
                let _ = std::fs::write(arg_str(&a, "out"), png.as_slice());
            }
            return;
        }
        apps::refresh(true);
        std::thread::sleep(Duration::from_millis(1500));
        match handle(&cmd, a) {
            Ok(v) => println!("{v}"),
            Err(e) => println!("{}", json!({ "error": e })),
        }
        return;
    }
    // the watchdog task starts us every few minutes; stay quit if you quit on purpose
    if has("--watchdog") && settings::quit_marker().exists() {
        return;
    }
    if !has("--watchdog") && !has("--quit") {
        let _ = std::fs::remove_file(settings::quit_marker());
    }
    // with another copy running, hand it our arguments ("--quit", "--settings", or show) and end
    let forward: Vec<String> = args.iter().skip(1).cloned().collect();
    if host::forward_to_running(&forward) {
        return;
    }
    if has("--quit") {
        return; // nothing to quit
    }
    taskbar::restore_after_crash();
    let tray_start = has("--tray");
    let want_settings = has("--settings");

    let s = settings::load();
    hook::apply(&s);
    if s.start_with_windows {
        // put start-with-Windows back if something removed it (or after an update)
        std::thread::spawn(|| {
            if !settings::autostart_ok() {
                settings::set_autostart(true);
            }
        });
    }
    // keyboard hook -> show / Win+E
    let (tx, rx) = crossbeam_channel::unbounded();
    hook::start(tx);
    std::thread::spawn(move || {
        for ev in rx.iter() {
            match ev {
                hook::Event::Show => host::show_launcher(),
                hook::Event::WinE => {
                    if !shellops::launch_filepilot(None) {
                        let _ = std::process::Command::new("explorer.exe").spawn();
                    }
                }
            }
        }
    });
    apps::refresh(true);
    content::start();
    scripts::ensure_dir();
    timers::start(move |t| {
        let title = if t.label.is_empty() { if t.kind == "timer" { "Time's up".to_string() } else { "Reminder".to_string() } } else { t.label.clone() };
        notice(json!({"title": title, "body": timers::describe(&t), "kind": "timer", "timer": t}));
    });
    if s.browser_search {
        browser::refresh(s.browser_history);
    }
    // daily update check
    std::thread::spawn(|| loop {
        std::thread::sleep(Duration::from_secs(90));
        updater::daily_check(&|_, _| {});
        let v = updater::check(false);
        if v["available"].as_bool() == Some(true) {
            *UPDATE.lock() = v["latest"].as_str().map(String::from);
        }
        std::thread::sleep(Duration::from_secs(6 * 3600));
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Glint")
            .with_app_id("com.glint.launcher")
            .with_inner_size([host::W as f32, host::H as f32])
            .with_decorations(false)
            // no caption buttons: with the acrylic backdrop DWM would draw them on the extended frame
            .with_close_button(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_icon(ui::app::app_icon())
            .with_visible(false),
        persist_window: false,
        // no waiting for vsync in SwapBuffers: every pass of the opening would wait a frame
        // (DWM composes the window, so nothing tears); App::ui caps animations instead
        glow_options: eframe::egui_glow::GlowConfiguration { vsync: false, ..Default::default() },
        ..Default::default()
    };
    let _ = eframe::run_native(
        "Glint",
        options,
        Box::new(move |cc| Ok(Box::new(ui::app::App::new(cc, ui::app::Startup { tray_start, want_settings })))),
    );
}
