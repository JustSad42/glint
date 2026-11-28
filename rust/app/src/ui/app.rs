//! The eframe app: the launcher is the root window; settings and notices are extra windows.
use super::assets::Assets;
use super::launcher::{Cx, Launcher};
use super::notice::{self, NoticeAction};
use super::settings_ui::{self, SettingsWin};
use super::theme::{self, Theme};
use crate::host::{self, Event};
use crate::settings::{self, Settings};
use egui::{Context, ViewportBuilder, ViewportCommand, ViewportId};
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};

pub struct Startup {
    pub tray_start: bool,
    pub want_settings: bool,
}

pub struct App {
    settings: Settings,
    theme: Theme,
    assets: Assets,
    launcher: Launcher,
    settings_win: Option<SettingsWin>,
    settings_styled: bool,
    /// the Settings / notice windows exist (see `host::boot_show`)
    settings_made: bool,
    notice_made: bool,
    notice_seen: HashMap<u64, Instant>,
    notice_hwnd: Option<(isize, usize)>,
    tray: Option<tray_icon::TrayIcon>,
    hotkeys: Option<global_hotkey::GlobalHotKeyManager>,
    main: HWND,
    startup: Option<Startup>,
    /// 1: painting the opened window now; 2: reveal it at the start of the next frame
    reveal: u8,
    /// start of the last pass (frame cap for animations, vsync is off)
    last_pass: Instant,
    had_focus: bool,
    translucent: bool,
    hidden_at: Option<Instant>,
    trimmed: bool,
}

fn hwnd_of(cc: &eframe::CreationContext<'_>) -> HWND {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match cc.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut _),
        _ => HWND::default(),
    }
}

pub fn app_icon() -> egui::IconData {
    let img = image::load_from_memory(include_bytes!("../../icons/32x32.png")).map(|i| i.to_rgba8()).ok();
    match img {
        Some(i) => egui::IconData { width: i.width(), height: i.height(), rgba: i.into_raw() },
        None => egui::IconData::default(),
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, startup: Startup) -> App {
        let ctx = cc.egui_ctx.clone();
        let main = hwnd_of(cc);
        // stay out of sight while eframe paints (and force-shows) the first frame
        host::set_cloaked(main, true);
        host::make_tool_window(main);
        host::style_frameless(main, true);
        theme::install_fonts(&ctx);
        let s = settings::load();
        let translucent = s.blur;
        host::set_backdrop(main, translucent);
        let theme = Theme::from_settings(&s, translucent);
        theme.apply(&ctx);
        host::init(&ctx, main);
        let mut app = App {
            settings: s.clone(),
            theme,
            assets: Assets::default(),
            launcher: Launcher::new(),
            settings_win: None,
            settings_styled: false,
            settings_made: false,
            notice_made: false,
            notice_seen: HashMap::new(),
            notice_hwnd: None,
            tray: None,
            hotkeys: None,
            main,
            startup: Some(startup),
            reveal: 0,
            last_pass: Instant::now(),
            had_focus: false,
            translucent,
            hidden_at: None,
            trimmed: false,
        };
        if s.show_tray {
            app.tray = make_tray();
        }
        app.hotkeys = global_hotkey::GlobalHotKeyManager::new().ok();
        global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(|e: global_hotkey::GlobalHotKeyEvent| {
            if e.state == global_hotkey::HotKeyState::Pressed {
                if let Some(item) = crate::hotkeys::find(e.id) {
                    std::thread::spawn(move || {
                        let _ = crate::activate(&item, "default", "");
                    });
                }
            }
        }));
        if let Some(m) = &app.hotkeys {
            let _ = crate::hotkeys::apply_with(m);
        }
        app.launcher.start(&ctx, &s);
        app
    }

    fn reload_settings(&mut self, ctx: &Context) {
        let s = settings::load();
        if s.blur != self.translucent {
            self.translucent = s.blur;
            host::set_backdrop(self.main, s.blur);
        }
        self.theme = Theme::from_settings(&s, self.translucent);
        self.theme.apply(ctx);
        if s.show_tray != self.settings.show_tray {
            self.tray = if s.show_tray { make_tray() } else { None };
        }
        self.settings = s;
    }

    fn handle_events(&mut self, ctx: &Context) {
        for e in host::take_events() {
            match e {
                Event::Shown => {
                    timing("shown event");
                    self.reload_settings(ctx);
                    self.launcher.on_shown(ctx, &self.settings);
                    if host::SHOW_WARM.load(Ordering::SeqCst) {
                        self.launcher.on_shown_warm();
                        self.reveal = 0;
                    } else {
                        self.reveal = 1;
                    }
                    self.had_focus = false;
                    self.hidden_at = None;
                    self.trimmed = false;
                }
                Event::Hidden => {
                    self.launcher.on_hidden();
                    // paint the hidden window once more without menu, Quick Look or toast, and again
                    // after the 45 s query reset; the next opening shows that frame first
                    host::set_warm(true);
                    ctx.request_repaint();
                    ctx.request_repaint_after(Duration::from_millis(45_500));
                    self.hidden_at = Some(Instant::now());
                }
                Event::OpenSettings(sec) => {
                    match &mut self.settings_win {
                        Some(w) => {
                            if !sec.is_empty() {
                                w.go(ctx, &sec);
                            }
                            let id = settings_id();
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Minimized(false));
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Visible(true));
                            ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                        }
                        None => {
                            self.settings_win = Some(SettingsWin::new(ctx, &sec));
                            self.settings_styled = false;
                        }
                    }
                }
                Event::CloseSettings => self.settings_win = None,
                Event::SettingsChanged => {
                    self.reload_settings(ctx);
                    self.launcher.on_settings_changed(ctx, &self.settings);
                    if let Some(w) = &mut self.settings_win {
                        w.reload(ctx);
                    }
                }
                Event::Notices => {}
                Event::Tray(on) => self.tray = if on { make_tray() } else { None },
                Event::ApplyHotkeys(tx) => {
                    let r = match &self.hotkeys {
                        Some(m) => crate::hotkeys::apply_with(m),
                        None => Err("Global shortcuts aren't available".into()),
                    };
                    let _ = tx.send(r);
                }
                Event::Quit => {
                    self.tray = None;
                    let _ = crate::taskbar::lower(self.main);
                    std::process::exit(0);
                }
            }
        }
    }
}

fn settings_id() -> ViewportId {
    ViewportId::from_hash_of("glint-settings")
}

fn notice_id() -> ViewportId {
    ViewportId::from_hash_of("glint-notice")
}

fn make_tray() -> Option<tray_icon::TrayIcon> {
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &MenuItem::with_id("open", "Open Glint", true, None),
        &MenuItem::with_id("settings", "Settings", true, None),
        &MenuItem::with_id("rebuild", "Rebuild content index", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id("quit", "Quit Glint", true, None),
    ]);
    MenuEvent::set_event_handler(Some(|e: MenuEvent| match e.id.0.as_str() {
        "open" => host::show_launcher(),
        "settings" => host::open_settings(""),
        "rebuild" => crate::content::rebuild(),
        "quit" => host::quit(),
        _ => {}
    }));
    tray_icon::TrayIconEvent::set_event_handler(Some(|e: tray_icon::TrayIconEvent| {
        if let tray_icon::TrayIconEvent::Click { button: tray_icon::MouseButton::Left, button_state: tray_icon::MouseButtonState::Up, .. } = e {
            host::show_launcher();
        }
    }));
    let icon = app_icon();
    let icon = tray_icon::Icon::from_rgba(icon.rgba, icon.width, icon.height).ok()?;
    tray_icon::TrayIconBuilder::new()
        .with_id("glint")
        .with_tooltip("Glint: press the Windows key to search")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_icon(icon)
        .build()
        .ok()
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // test hook: GLINT_TEST_FPS=1 prints how many passes ran in each second (idle should be ~0)
        if std::env::var_os("GLINT_TEST_FPS").is_some() {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            static SEC: AtomicU64 = AtomicU64::new(0);
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            if SEC.swap(now, Ordering::Relaxed) != now {
                eprintln!("FPS {} {} {:?}", now, N.swap(0, Ordering::Relaxed), ctx.repaint_causes());
            }
            N.fetch_add(1, Ordering::Relaxed);
        }
        self.handle_events(ctx);
        // notices that aren't timers go away on their own after 9 s
        let now_ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let expired: Vec<u64> = crate::NOTICES.lock().iter().filter(|n| n["kind"].as_str() != Some("timer")).filter_map(|n| n["id"].as_u64()).filter(|id| now_ms.saturating_sub(*id) > 9000).collect();
        for id in expired {
            crate::dismiss_notice(id);
        }
        let any_notice = !crate::NOTICES.lock().is_empty();
        if any_notice {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        // a hidden launcher still needs a pass to open (or keep) the other windows
        // (only until they exist: eframe runs the root's ui by itself while a child window is visible,
        // and asking every frame would never let it go idle)
        let settings_pending = self.settings_win.is_some() && !self.settings_styled;
        let notice_pending = any_notice && self.notice_hwnd.is_none();
        if !host::is_visible() && (settings_pending || notice_pending) {
            ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::RequestPaintWhileHidden);
            ctx.request_repaint();
        }
        // "Sleep while hidden": give memory back a few seconds after hiding
        if let Some(h) = self.hidden_at {
            if !self.trimmed && self.settings.suspend_hidden {
                if h.elapsed() > Duration::from_secs(5) {
                    self.trimmed = true;
                    unsafe {
                        use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
                        use windows::Win32::System::Threading::GetCurrentProcess;
                        let _ = EmptyWorkingSet(GetCurrentProcess());
                    }
                } else {
                    ctx.request_repaint_after(Duration::from_secs(5).saturating_sub(h.elapsed()));
                }
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // decided before this pass may hide the launcher (a hiding pass still counts as visible)
        let draw_launcher = host::is_visible();
        // vsync is off: while something animates (it asked for the next frame right away), keep to
        // ~144 fps instead of rendering flat out. Not while opening, which should be quick.
        const FRAME: Duration = Duration::from_millis(7);
        if self.reveal == 0 && ctx.requested_repaint_last_pass_for(&egui::ViewportId::ROOT) {
            let e = self.last_pass.elapsed();
            if e < FRAME {
                std::thread::sleep(FRAME - e);
            }
        }
        self.last_pass = Instant::now();
        // the first frame was painted under a cloak: hide properly, then open as asked
        if let Some(st) = &self.startup {
            if ctx.cumulative_frame_nr() >= 1 {
                unsafe {
                    let _ = ShowWindow(self.main, SW_HIDE);
                }
                host::set_cloaked(self.main, false);
                let st = self.startup.take().unwrap();
                if st.want_settings || !self.settings.first_run_done {
                    let first = !self.settings.first_run_done;
                    let mut s2 = self.settings.clone();
                    s2.first_run_done = true;
                    let _ = settings::save(&s2);
                    self.settings = s2;
                    // test hook: GLINT_TEST_PAGE picks the page (screenshots)
                    let page = std::env::var("GLINT_TEST_PAGE").unwrap_or_default();
                    host::open_settings(if first { "welcome" } else { &page });
                } else if !st.tray_start {
                    host::show_launcher();
                } else {
                    // started hidden: be in place and painted, so the first opening is instant too
                    host::preplace();
                    host::set_warm(true);
                    ctx.request_repaint();
                }
            } else {
                let _ = st;
                ctx.request_repaint();
            }
        }
        if self.reveal == 2 {
            self.reveal = 0;
            timing("reveal");
            host::reveal();
            self.launcher.on_revealed();
            ctx.request_repaint();
        } else if self.reveal == 1 {
            timing("first paint pass");
            self.reveal = 2;
            ctx.request_repaint();
        }
        if self.assets.poll(&ctx) {
            ctx.request_repaint();
        }
        let visible = host::is_visible();
        if visible {
            // clicking elsewhere hides the launcher (not in the first moments: the taskbar
            // is activated for an instant while Glint opens)
            let focused = ctx.input(|i| i.viewport().focused);
            if focused == Some(true) {
                self.had_focus = true;
            }
            let recent = host::SHOWN_AT.lock().map(|t| t.elapsed() < Duration::from_millis(350)).unwrap_or(false);
            let keep_open = std::env::var_os("GLINT_TEST_NOHIDE").is_some();
            if self.had_focus && focused == Some(false) && !recent && !keep_open {
                self.had_focus = false;
                host::hide_launcher();
            }
            if ctx.input(|i| i.viewport().close_requested()) && !host::QUITTING.load(Ordering::SeqCst) {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                host::hide_launcher();
            }
        }
        // the launcher is drawn in every pass, hidden or not: eframe paints the hidden window too (it
        // doesn't know it is hidden), and that frame is what a quick reopen shows first (host::set_warm).
        // While hidden its repeating animations (clock, equalizer) don't ask for more passes.
        self.launcher.visible = draw_launcher;
        {
            let App { launcher, theme, assets, settings, .. } = self;
            let mut cx = Cx { t: theme, a: assets, s: settings };
            egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
                launcher.ui(ui, &mut cx);
            });
        }
        self.settings_window(&ctx);
        self.notice_window(&ctx);
        // the windows that needed a shown root exist now: hide it again
        if host::booting() {
            let settings_pending = self.settings_win.is_some() && !self.settings_made;
            let notice_pending = !crate::NOTICES.lock().is_empty() && !self.notice_made;
            if !settings_pending && !notice_pending {
                host::boot_end();
            } else {
                ctx.request_repaint();
            }
        }
    }
}

impl App {
    fn settings_window(&mut self, ctx: &Context) {
        if self.settings_win.is_none() {
            self.settings_made = false;
            return;
        }
        // a new window can only be made in a pass of a shown root window (see host::boot_show)
        if !self.settings_made && !host::raw_visible() {
            host::boot_show();
            return;
        }
        let Some(win) = &mut self.settings_win else { return };
        let builder = ViewportBuilder::default()
            .with_title(settings_ui::TITLE)
            .with_inner_size([960.0, 680.0])
            .with_min_inner_size([760.0, 520.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_icon(app_icon());
        let theme = &self.theme;
        let assets = &mut self.assets;
        let mut close = false;
        ctx.show_viewport_immediate(settings_id(), builder, |ui, _class| {
            if ui.ctx().input(|i| i.viewport().close_requested()) {
                close = true;
            }
            egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
                win.ui(ui, theme, assets);
            });
        });
        self.settings_made = true;
        if !self.settings_styled {
            if let Some(h) = host::find_own_window(settings_ui::TITLE) {
                host::style_frameless(h, true);
                center_on_cursor_monitor(h);
                self.settings_styled = true;
            }
        }
        if close || win.close {
            self.settings_win = None;
        }
    }

    fn notice_window(&mut self, ctx: &Context) {
        let list = crate::NOTICES.lock().clone();
        if list.is_empty() {
            self.notice_hwnd = None;
            self.notice_made = false;
            return;
        }
        if !self.notice_made && !host::raw_visible() {
            host::boot_show();
            return;
        }
        let builder = ViewportBuilder::default()
            .with_title(notice::TITLE)
            .with_inner_size([notice::W, notice::height(4)])
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(false)
            .with_taskbar(false)
            .with_active(false)
            .with_close_button(false)
            .with_minimize_button(false)
            .with_maximize_button(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop);
        let theme = &self.theme;
        let assets = &mut self.assets;
        let seen = &mut self.notice_seen;
        let mut actions = Vec::new();
        ctx.show_viewport_immediate(notice_id(), builder, |ui, _class| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show_inside(ui, |ui| {
                actions = notice::ui(ui, theme, assets, &list, seen);
            });
        });
        self.notice_made = true;
        // place it in the corner of the main screen, sized to the cards, without taking focus
        let need = (list.len(), list.len());
        if self.notice_hwnd.map(|(_, n)| n) != Some(need.0) {
            if let Some(h) = host::find_own_window(notice::TITLE) {
                host::make_tool_window(h);
                host::style_bare(h);
                if let Some(m) = host::monitor_at(Default::default(), true) {
                    let sc = m.scale;
                    let (ww, wh) = ((notice::W as f64 * sc) as i32, (notice::height(list.len()) as f64 * sc) as i32);
                    let gap = (8.0 * sc) as i32;
                    unsafe {
                        use windows::Win32::UI::WindowsAndMessaging::*;
                        let _ = SetWindowPos(h, Some(HWND_TOPMOST), m.work.right - ww - gap, m.work.bottom - wh - gap, ww, wh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
                    }
                }
                self.notice_hwnd = Some((h.0 as isize, need.0));
            }
        }
        for a in actions {
            match a {
                NoticeAction::Dismiss(id) => {
                    crate::dismiss_notice(id);
                }
                NoticeAction::Snooze(id, mins) => {
                    let _ = crate::handle("notice_snooze", serde_json::json!({"id": id, "mins": mins}));
                }
            }
        }
    }
}

fn center_on_cursor_monitor(h: HWND) {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::*;
    let mut p = POINT::default();
    let mut r = RECT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
        let _ = GetWindowRect(h, &mut r);
    }
    if let Some(m) = host::monitor_at(p, settings::load().monitor == "primary") {
        let (w, hh) = ((960.0 * m.scale) as i32, (680.0 * m.scale) as i32);
        let x = m.work.left + (m.work.right - m.work.left - w) / 2;
        let y = m.work.top + (m.work.bottom - m.work.top - hh) / 2;
        unsafe {
            // move first: crossing to a screen with another scale rescales the window (WM_DPICHANGED),
            // so the size is set once it is there
            let _ = SetWindowPos(h, None, x, y, 0, 0, SWP_NOZORDER | SWP_NOSIZE);
            let _ = SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER);
            let _ = windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow(h);
        }
    }
}

/// test hook: GLINT_TEST_TIMING=1 prints how long after the show request each opening step ran
fn timing(step: &str) {
    if std::env::var_os("GLINT_TEST_TIMING").is_some() {
        if let Some(t) = *host::SHOWN_AT.lock() {
            eprintln!("TIMING {step} {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
