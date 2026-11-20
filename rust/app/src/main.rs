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
mod calc;
mod clipvault;
mod content;
mod extract;
mod files;
mod fuzzy;
mod history;
mod hook;
mod icons;
mod media;
mod pins;
mod preview;
mod search;
mod settings;
mod shellops;
mod sysinfo;
mod updater;
mod winmgmt;
mod wins;

use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::http::Response;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::window::{Effect, EffectsBuilder};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

static APP: OnceCell<AppHandle> = OnceCell::new();
static SHOWN_AT: Mutex<Option<Instant>> = Mutex::new(None);
static UPDATE: Mutex<Option<String>> = Mutex::new(None);
static QUITTING: AtomicBool = AtomicBool::new(false);

const W: f64 = 800.0;
const H: f64 = 540.0;

fn arg_str<'a>(a: &'a Value, k: &str) -> &'a str {
    a[k].as_str().unwrap_or("")
}

fn main_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window("main")
}

fn create_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let s = settings::load();
    let mut b = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Glint")
        .inner_size(W, H)
        .decorations(false)
        .transparent(true)
        .resizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .visible(false)
        .focused(false)
        .shadow(true);
    if s.blur {
        b = b.effects(EffectsBuilder::new().effect(Effect::Acrylic).build());
    }
    let w = b.build()?;
    let app2 = app.clone();
    w.on_window_event(move |e| match e {
        WindowEvent::Focused(false) => {
            let recent = SHOWN_AT.lock().map(|t| t.elapsed() < Duration::from_millis(350)).unwrap_or(false);
            if !recent {
                hide_launcher(&app2);
            }
        }
        WindowEvent::CloseRequested { api, .. } => {
            if !QUITTING.load(Ordering::SeqCst) {
                api.prevent_close();
                hide_launcher(&app2);
            }
        }
        _ => {}
    });
    Ok(w)
}

/// Center on the monitor under the mouse, in its upper part (or the middle).
fn place(app: &AppHandle, w: &WebviewWindow) {
    let pos = app.cursor_position().ok();
    let mon = pos.and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten()).or_else(|| w.current_monitor().ok().flatten());
    let Some(m) = mon else { return };
    let scale = m.scale_factor();
    let wa = m.work_area();
    let (ww, wh) = ((W * scale) as i32, (H * scale) as i32);
    let x = wa.position.x + (wa.size.width as i32 - ww) / 2;
    let top = settings::load().position != "center";
    let y = if top { wa.position.y + (wa.size.height as f64 * 0.16) as i32 } else { wa.position.y + (wa.size.height as i32 - wh) / 2 };
    // move first: resizing while still on a monitor with a different scale would be undone
    // by Windows' DPI adjustment when the window crosses over
    let _ = w.set_position(PhysicalPosition::new(x, y));
    let _ = w.set_size(PhysicalSize::new(ww as u32, wh as u32));
    let _ = w.set_position(PhysicalPosition::new(x, y));
}

pub fn show_launcher(app: &AppHandle) {
    let w = match main_window(app) {
        Some(w) => w,
        None => match create_main(app) {
            Ok(w) => w,
            Err(_) => return,
        },
    };
    if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
        hide_launcher(app);
        return;
    }
    // remember the window you came from: layouts move it and clips paste into it
    unsafe {
        let fg = windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow();
        let ours = w.hwnd().map(|h| h.0 as isize == fg.0 as isize).unwrap_or(false);
        if !fg.is_invalid() && !ours {
            winmgmt::PREV.store(fg.0 as isize, Ordering::SeqCst);
        }
    }
    place(app, &w);
    *SHOWN_AT.lock() = Some(Instant::now());
    let _ = w.emit("launcher-shown", json!({}));
    let _ = w.show();
    let _ = w.set_focus();
    if let Ok(h) = w.hwnd() {
        wins::force_foreground(windows::Win32::Foundation::HWND(h.0 as _));
    }
    apps::refresh(false);
}

pub fn hide_launcher(app: &AppHandle) {
    if let Some(w) = main_window(app) {
        if w.is_visible().unwrap_or(false) {
            let _ = w.emit("launcher-hidden", json!({}));
            let _ = w.hide();
        }
    }
}

fn open_settings(app: &AppHandle, section: &str) {
    hide_launcher(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if !section.is_empty() {
            let _ = w.emit("goto", section);
        }
        return;
    }
    let url = if section.is_empty() { "settings.html".to_string() } else { format!("settings.html#{section}") };
    let _ = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App(url.into()))
        .title("Glint Settings")
        .inner_size(960.0, 680.0)
        .min_inner_size(760.0, 520.0)
        .decorations(false)
        .center()
        .shadow(true)
        .build();
}

fn quit(app: &AppHandle) {
    QUITTING.store(true, Ordering::SeqCst);
    hook::stop();
    app.exit(0);
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

fn glint_command(app: &AppHandle, cmd: &str) -> Value {
    match cmd {
        "settings" => {
            open_settings(app, "");
            json!({"hide": true})
        }
        "updates" => {
            open_settings(app, "about");
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
            hide_launcher(app);
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(120));
                hook::open_start_menu();
            });
            json!({"hide": true})
        }
        "quit" => {
            quit(app);
            json!({"hide": true})
        }
        _ => json!({"hide": false}),
    }
}

/// Do what the user picked. `mode` is the chosen action (default, reveal, admin, copy-path...).
fn activate(app: &AppHandle, item: &Value, mode: &str, query: &str) -> Result<Value, String> {
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
            hide_launcher(app);
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
            hide_launcher(app);
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
            hide_launcher(app);
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
                actions::Outcome::Glint(cmd) => Ok(glint_command(app, cmd)),
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
    })
}

fn handle(app: &AppHandle, cmd: &str, a: Value) -> Result<Value, String> {
    Ok(match cmd {
        "search" => search::run(arg_str(&a, "q"), if arg_str(&a, "filter").is_empty() { "all" } else { arg_str(&a, "filter") }),
        "details" => search::details(&a["item"]),
        "activate" => activate(app, &a["item"], if arg_str(&a, "mode").is_empty() { "default" } else { arg_str(&a, "mode") }, arg_str(&a, "q"))?,
        "hide" => {
            hide_launcher(app);
            json!(true)
        }
        "queries" => json!(history::queries()),
        "pins_list" => pins::list(),
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
            if s.show_tray != old.show_tray {
                if s.show_tray {
                    let _ = setup_tray(app);
                } else {
                    let _ = app.remove_tray_by_id("glint");
                }
            }
            if s.blur != old.blur {
                // the window effect is set when the window is made
                if let Some(w) = main_window(app) {
                    let _ = w.destroy();
                }
                let _ = create_main(app);
            }
            let _ = app.emit("settings-changed", serde_json::to_value(&s).unwrap());
            json!({"ok": true})
        }
        "open_settings" => {
            open_settings(app, arg_str(&a, "section"));
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
            if let Some(w) = app.get_webview_window("settings") {
                let _ = w.close();
            }
            let h = app.clone();
            let _ = app.run_on_main_thread(move || show_launcher(&h));
            json!(true)
        }
        "close_settings" => {
            if let Some(w) = app.get_webview_window("settings") {
                let _ = w.close();
            }
            json!(true)
        }
        "quit" => {
            quit(app);
            json!(true)
        }
        _ => return Err(format!("unknown command {cmd}")),
    })
}

#[tauri::command]
async fn call(app: AppHandle, cmd: String, args: Value) -> Result<Value, String> {
    // file work and COM calls stay off the UI thread
    tauri::async_runtime::spawn_blocking(move || handle(&app, &cmd, args)).await.map_err(|e| e.to_string())?
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    if app.tray_by_id("glint").is_some() {
        return Ok(());
    }
    let open = MenuItem::with_id(app, "open", "Open Glint", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let rebuild = MenuItem::with_id(app, "rebuild", "Rebuild content index", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit Glint", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &settings_i, &rebuild, &sep, &quit_i])?;
    let mut b = TrayIconBuilder::with_id("glint")
        .tooltip("Glint: press the Windows key to search")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "open" => show_launcher(app),
            "settings" => open_settings(app, ""),
            "rebuild" => content::rebuild(),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_launcher(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
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
    let tray_start = has("--tray");
    // with another copy running, the single-instance plugin hands it "--quit" and this process
    // ends; otherwise there is nothing to quit
    let quit_only = has("--quit");
    let want_settings = has("--settings");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _| {
            if argv.iter().any(|a| a == "--quit") {
                quit(app);
            } else if argv.iter().any(|a| a == "--settings") {
                open_settings(app, "");
            } else if !argv.iter().any(|a| a == "--tray") {
                show_launcher(app);
            }
        }))
        .register_asynchronous_uri_scheme_protocol("gicon", |_ctx, req, responder| {
            let path = req.uri().path().to_string();
            icons::request(&path, move |png| {
                let resp = match png {
                    Some(p) => Response::builder()
                        .header("Content-Type", "image/png")
                        .header("Cache-Control", "max-age=3600")
                        .header("Access-Control-Allow-Origin", "*")
                        .body(p.as_ref().clone()),
                    None => Response::builder().status(404).header("Access-Control-Allow-Origin", "*").body(Vec::new()),
                };
                responder.respond(resp.unwrap());
            });
        })
        .invoke_handler(tauri::generate_handler![call])
        .setup(move |app| {
            let handle = app.handle().clone();
            if quit_only {
                handle.exit(0);
                return Ok(());
            }
            let _ = APP.set(handle.clone());
            let s = settings::load();
            hook::apply(&s);

            // keyboard hook -> show / Win+E
            let (tx, rx) = crossbeam_channel::unbounded();
            hook::start(tx);
            let h2 = handle.clone();
            std::thread::spawn(move || {
                for ev in rx.iter() {
                    match ev {
                        hook::Event::Show => {
                            let h3 = h2.clone();
                            let _ = h2.run_on_main_thread(move || show_launcher(&h3));
                        }
                        hook::Event::WinE => {
                            if !shellops::launch_filepilot(None) {
                                let _ = std::process::Command::new("explorer.exe").spawn();
                            }
                        }
                    }
                }
            });

            create_main(&handle)?;
            if s.show_tray {
                setup_tray(&handle)?;
            }
            apps::refresh(true);
            content::start();

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

            if want_settings || !s.first_run_done {
                let mut s2 = s.clone();
                s2.first_run_done = true;
                let _ = settings::save(&s2);
                open_settings(&handle, if s.first_run_done { "" } else { "welcome" });
            } else if !tray_start {
                show_launcher(&handle);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Glint")
        .run(|_app, e| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = e {
                // closing the settings window must not end the app
                if code.is_none() && !QUITTING.load(Ordering::SeqCst) {
                    api.prevent_exit();
                }
            }
        });
}
