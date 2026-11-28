//! The window plumbing Tauri used to provide: showing and hiding the launcher, placing it,
//! single instance, and a queue of events for the UI.
//!
//! Window operations always run on the UI thread. Other threads (the keyboard hook, workers
//! running actions, the second instance) post a message to a small message-only window that
//! lives on the UI thread, so a hidden launcher opens at once instead of waiting for egui's
//! next pass.
use crate::{apps, settings, taskbar, winmgmt, wins};
use once_cell::sync::OnceCell;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CLOAK};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What the UI should catch up on in its next pass.
#[derive(Debug, Clone)]
pub enum Event {
    /// the launcher was just shown ("launcher-shown")
    Shown,
    /// the launcher was just hidden ("launcher-hidden")
    Hidden,
    /// open settings at a page ("" = keep the current one)
    OpenSettings(String),
    CloseSettings,
    /// settings were saved; everything re-reads them
    SettingsChanged,
    /// the notice list changed
    Notices,
    /// show or remove the tray icon
    Tray(bool),
    /// register the global shortcuts again; the result goes back on the channel
    ApplyHotkeys(crossbeam_channel::Sender<Result<(), String>>),
    Quit,
}

pub static CTX: OnceCell<egui::Context> = OnceCell::new();
/// the launcher window
static MAIN: AtomicIsize = AtomicIsize::new(0);
static MSGWIN: AtomicIsize = AtomicIsize::new(0);
static UI_THREAD: AtomicU32 = AtomicU32::new(0);
static EVENTS: Mutex<Vec<Event>> = Mutex::new(Vec::new());
pub static SHOWN_AT: Mutex<Option<Instant>> = Mutex::new(None);
pub static QUITTING: AtomicBool = AtomicBool::new(false);
/// Quick Look made the window big; `place` puts it back
pub static QUICKLOOK: AtomicBool = AtomicBool::new(false);
/// bumped on every show, so a late "uncloak" for an older show does nothing
static SHOW_GEN: AtomicU64 = AtomicU64::new(0);

const WM_GLINT_SHOW: u32 = WM_APP + 1;
const WM_GLINT_HIDE: u32 = WM_APP + 2;
const WM_GLINT_QUICKLOOK: u32 = WM_APP + 3;
const WM_GLINT_WAKE: u32 = WM_APP + 4;

pub fn main_hwnd() -> HWND {
    HWND(MAIN.load(Ordering::SeqCst) as *mut _)
}

fn on_ui_thread() -> bool {
    unsafe { GetCurrentThreadId() == UI_THREAD.load(Ordering::SeqCst) }
}

/// A name unique to this data folder, so a test copy (GLINT_DATA) runs next to the real one.
pub fn instance_name() -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    settings::DATA_DIR.to_string_lossy().to_lowercase().hash(&mut h);
    format!("GlintInstance-{:016x}", h.finish())
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ------------------------------------------------------------------ single instance

/// If another Glint already runs (same data folder), hand it our arguments and return true.
pub fn forward_to_running(args: &[String]) -> bool {
    let name = wide(&instance_name());
    unsafe {
        let Ok(target) = FindWindowExW(Some(HWND_MESSAGE), None, PCWSTR(name.as_ptr()), PCWSTR::null()) else { return false };
        if target.is_invalid() {
            return false;
        }
        // the running copy may bring a window to the front: let it
        let _ = AllowSetForegroundWindow(windows::Win32::UI::WindowsAndMessaging::ASFW_ANY);
        let payload = args.join("\u{1}");
        let bytes = payload.as_bytes();
        let cds = COPYDATASTRUCT { dwData: 0x474c4e54, cbData: bytes.len() as u32, lpData: bytes.as_ptr() as *mut _ };
        let mut res = 0usize;
        let _ = SendMessageTimeoutW(target, WM_COPYDATA, WPARAM(0), LPARAM(&cds as *const _ as isize), SMTO_ABORTIFHUNG, 3000, Some(&mut res));
        true
    }
}

/// What a second launch asked for.
fn handle_forwarded(args: &[&str]) {
    if args.contains(&"--quit") {
        quit();
    } else if args.contains(&"--settings") {
        open_settings("");
    } else if !args.contains(&"--tray") {
        show_launcher();
    }
}

// ------------------------------------------------------------------ setup

/// Call once on the UI thread with the launcher's window.
pub fn init(ctx: &egui::Context, main: HWND) {
    let _ = CTX.set(ctx.clone());
    MAIN.store(main.0 as isize, Ordering::SeqCst);
    UI_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    unsafe {
        let class = wide(&instance_name());
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(msg_proc), hInstance: hinst.into(), lpszClassName: PCWSTR(class.as_ptr()), ..Default::default() };
        RegisterClassW(&wc);
        if let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), PCWSTR(class.as_ptr()), w!("Glint"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(hinst.into()), None) {
            MSGWIN.store(h.0 as isize, Ordering::SeqCst);
        }
    }
}

unsafe extern "system" fn msg_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_GLINT_SHOW => {
            do_show();
            LRESULT(0)
        }
        WM_GLINT_HIDE => {
            do_hide();
            LRESULT(0)
        }
        WM_GLINT_QUICKLOOK => {
            do_quicklook(wp.0 != 0);
            LRESULT(0)
        }
        WM_GLINT_WAKE => {
            wake();
            LRESULT(0)
        }
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == 0x474c4e54 && !cds.lpData.is_null() {
                let bytes = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
                let s = String::from_utf8_lossy(bytes).into_owned();
                let args: Vec<&str> = s.split('\u{1}').collect();
                handle_forwarded(&args);
            }
            LRESULT(1)
        }
        _ => DefWindowProcW(h, msg, wp, lp),
    }
}

fn post(msg: u32, wp: usize) {
    let h = HWND(MSGWIN.load(Ordering::SeqCst) as *mut _);
    if !h.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(h), msg, WPARAM(wp), LPARAM(0));
        }
    }
}

fn push(e: Event) {
    EVENTS.lock().push(e);
    if on_ui_thread() {
        wake();
    } else {
        post(WM_GLINT_WAKE, 0);
    }
}

/// Make egui run a pass soon, even with every window hidden.
fn wake() {
    if let Some(ctx) = CTX.get() {
        ctx.request_repaint();
    }
}

pub fn take_events() -> Vec<Event> {
    std::mem::take(&mut *EVENTS.lock())
}

// ------------------------------------------------------------------ public API (any thread)

pub fn show_launcher() {
    dbg_log("show_launcher");
    if on_ui_thread() {
        do_show();
    } else {
        post(WM_GLINT_SHOW, 0);
    }
}

pub fn hide_launcher() {
    if on_ui_thread() {
        do_hide();
    } else {
        post(WM_GLINT_HIDE, 0);
    }
}

pub fn quicklook(on: bool) {
    if on_ui_thread() {
        do_quicklook(on);
    } else {
        post(WM_GLINT_QUICKLOOK, on as usize);
    }
}

pub fn open_settings(section: &str) {
    hide_launcher();
    push(Event::OpenSettings(section.to_string()));
}

pub fn close_settings() {
    push(Event::CloseSettings);
}

pub fn settings_changed() {
    push(Event::SettingsChanged);
}

pub fn notices_changed() {
    push(Event::Notices);
}

pub fn set_tray(on: bool) {
    push(Event::Tray(on));
}

/// Register the saved global shortcuts again (on the UI thread, which owns them).
pub fn apply_hotkeys() -> Result<(), String> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    push(Event::ApplyHotkeys(tx));
    if on_ui_thread() {
        // the UI picks it up in its next pass; nobody waits on the UI thread
        return Ok(());
    }
    rx.recv_timeout(Duration::from_secs(3)).unwrap_or(Ok(()))
}

pub fn quit() {
    QUITTING.store(true, Ordering::SeqCst);
    let _ = std::fs::write(settings::quit_marker(), b"quit by user");
    if on_ui_thread() {
        let _ = taskbar::lower(main_hwnd());
    }
    crate::hook::stop();
    push(Event::Quit);
    // the UI removes the tray icon and exits; don't hang around if it can't
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(3));
        std::process::exit(0);
    });
}

/// Is the launcher open? (Not while the window is only shown, cloaked, to create another window.)
pub fn is_visible() -> bool {
    raw_visible() && !BOOT.load(Ordering::SeqCst)
}

/// Is the launcher's window shown at all (even cloaked)?
pub fn raw_visible() -> bool {
    unsafe { IsWindowVisible(main_hwnd()).as_bool() }
}

/// eframe runs the passes of a hidden window outside its event loop, and a new immediate viewport
/// (Settings, a notice) can't create its window there: egui then panics. So to create one while
/// the launcher is hidden, show the launcher's window cloaked (unseen, not activated) for a pass
/// that comes through a normal redraw, and hide it again afterwards (`boot_end`).
static BOOT: AtomicBool = AtomicBool::new(false);

pub fn boot_show() {
    if raw_visible() {
        return;
    }
    let h = main_hwnd();
    BOOT.store(true, Ordering::SeqCst);
    set_cloaked(h, true);
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
    }
    wake();
}

pub fn boot_end() {
    if BOOT.swap(false, Ordering::SeqCst) {
        let h = main_hwnd();
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
        set_cloaked(h, false);
    }
}

pub fn booting() -> bool {
    BOOT.load(Ordering::SeqCst)
}

// ------------------------------------------------------------------ window work (UI thread)

pub fn set_cloaked(h: HWND, on: bool) {
    let v: i32 = on as i32;
    unsafe {
        let _ = DwmSetWindowAttribute(h, DWMWA_CLOAK, &v as *const _ as _, 4);
    }
}

fn do_show() {
    let h = main_hwnd();
    if h.is_invalid() {
        return;
    }
    unsafe {
        let fg = GetForegroundWindow();
        dbg_log(&format!("do_show visible={} fg_is_main={} boot={}", IsWindowVisible(h).as_bool(), fg == h, BOOT.load(Ordering::SeqCst)));
        if IsWindowVisible(h).as_bool() && fg == h {
            do_hide();
            return;
        }
        // remember the window you came from: layouts move it and clips paste into it
        if !fg.is_invalid() && fg != h {
            winmgmt::PREV.store(fg.0 as isize, Ordering::SeqCst);
        }
    }
    // the taskbar is activated for an instant: don't treat that as Glint losing focus
    *SHOWN_AT.lock() = Some(Instant::now());
    let was_visible = is_visible();
    // a cloaked creation pass becomes a real opening (still cloaked until painted)
    let booted = BOOT.swap(false, Ordering::SeqCst);
    taskbar::prepare(h);
    let before = window_rect(h);
    place(h);
    timing("placed");
    // a quick reopen to the same spot shows the frame it was hidden with straight away, like the old
    // app; fresh content follows a pass later (see `set_warm` for when that frame is still right)
    let warm_ok = WARM.swap(false, Ordering::SeqCst);
    let warm = !was_visible && !booted && before == window_rect(h) && warm_ok;
    timing(&format!("warm={warm} (was_visible={was_visible} booted={booted} same_rect={} warm_ok={warm_ok})", before == window_rect(h)));
    SHOW_WARM.store(warm, Ordering::SeqCst);
    let gen = SHOW_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if !was_visible && !warm {
        // stay out of sight until the UI painted this opening (no flash of the last search)
        set_cloaked(h, true);
        timing("cloaked");
        let hs = h.0 as isize;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(250));
            if SHOW_GEN.load(Ordering::SeqCst) == gen {
                set_cloaked(HWND(hs as *mut _), false);
            }
        });
    }
    no_sysmenu(h);
    timing("sysmenu");
    unsafe {
        // shown without activating: it is on screen sooner; force_foreground activates it right after
        let _ = ShowWindow(h, SW_SHOWNA);
    }
    timing("ShowWindow");
    wins::force_foreground(h);
    timing("foreground");
    taskbar::kick(h);
    EVENTS.lock().push(Event::Shown);
    wake();
    apps::refresh(false);
}

/// A reopen may show the hidden window's last frame at once: the launcher keeps painting while
/// hidden (and does the 45 s query reset in the background), so that frame is the opening's.
static WARM: AtomicBool = AtomicBool::new(false);
/// The current opening showed the last frame straight away (no cloak, no opening animation).
pub static SHOW_WARM: AtomicBool = AtomicBool::new(false);

pub fn set_warm(on: bool) {
    WARM.store(on, Ordering::SeqCst);
}

fn window_rect(h: HWND) -> (i32, i32, i32, i32) {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    (r.left, r.top, r.right, r.bottom)
}

/// The UI painted the opened window: let it be seen.
pub fn reveal() {
    set_cloaked(main_hwnd(), false);
}

fn do_hide() {
    let h = main_hwnd();
    dbg_log("do_hide");
    let lifted = taskbar::lower(h);
    if is_visible() {
        unsafe {
            let _ = ShowWindow(h, SW_HIDE);
        }
        EVENTS.lock().push(Event::Hidden);
        wake();
    }
    if lifted {
        taskbar::hand_back(winmgmt::PREV.load(Ordering::SeqCst));
    }
}

pub struct Mon {
    pub work: RECT,
    pub full: RECT,
    pub scale: f64,
}

pub fn monitor_at(p: POINT, primary: bool) -> Option<Mon> {
    unsafe {
        let hm = if primary { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) } else { MonitorFromPoint(p, MONITOR_DEFAULTTONEAREST) };
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(hm, &mut mi).as_bool() {
            return None;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(hm, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        Some(Mon { work: mi.rcWork, full: mi.rcMonitor, scale: dx as f64 / 96.0 })
    }
}

/// The monitor Glint opens on: under the mouse, or always the main one.
fn target_monitor() -> Option<Mon> {
    let primary = settings::load().monitor == "primary";
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    monitor_at(p, primary)
}

pub const W: f64 = 800.0;
pub const H: f64 = 540.0;
/// the docked "classic Start menu" layout
pub const SW: f64 = 760.0;
pub const SH: f64 = 640.0;

fn set_rect(h: HWND, x: i32, y: i32, ww: i32, wh: i32) {
    unsafe {
        // move first: resizing while still on a monitor with a different scale would be undone
        // by Windows' DPI adjustment when the window crosses over
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        let _ = SetWindowPos(h, None, x, y, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

/// Pop-up: centered, in the upper part (or the middle). Start-menu layout: docked in the
/// bottom-left corner, above the taskbar.
fn place(h: HWND) {
    QUICKLOOK.store(false, Ordering::SeqCst);
    let Some(m) = target_monitor() else { return };
    let s = settings::load();
    let scale = m.scale;
    let start = s.layout == "start";
    let mut wa = m.work;
    // an auto-hidden taskbar brought up with Glint covers the bottom: sit above it
    if start {
        let covered = taskbar::covered_bottom(m.full);
        let overlap = (wa.bottom - (m.full.bottom - covered)).max(0);
        wa.bottom -= overlap;
    }
    let (wa_w, wa_h) = (wa.right - wa.left, wa.bottom - wa.top);
    let (ww, wh) = if start { ((SW * scale) as i32, (SH * scale) as i32) } else { ((W * scale) as i32, (H * scale) as i32) };
    let wh = wh.min(wa_h - (24.0 * scale) as i32);
    let gap = (10.0 * scale) as i32;
    let corner = start && s.start_position != "center" && s.start_position != "bottom";
    let x = if corner { wa.left + gap } else { wa.left + (wa_w - ww) / 2 };
    let top = s.position != "center";
    let y = if start && s.start_position == "center" {
        wa.top + (wa_h - wh) / 2
    } else if start {
        wa.top + wa_h - wh - gap
    } else if top {
        wa.top + (wa_h as f64 * 0.16) as i32
    } else {
        wa.top + (wa_h - wh) / 2
    };
    set_rect(h, x, y, ww, wh);
}

/// Put the hidden launcher where it will open, so the first opening can show its frame at once.
pub fn preplace() {
    let h = main_hwnd();
    if !h.is_invalid() {
        place(h);
    }
}

/// Make the launcher big for Quick Look, or put it back.
fn do_quicklook(on: bool) {
    let h = main_hwnd();
    if !on {
        place(h);
        return;
    }
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    let Some(m) = monitor_at(POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 }, false) else { return };
    let wa = m.work;
    let (wa_w, wa_h) = (wa.right - wa.left, wa.bottom - wa.top);
    let ww = (wa_w as f64 * 0.78) as i32;
    // on a portrait screen don't go taller than a landscape-ish box
    let wh = ((wa_h as f64 * 0.86) as i32).min((ww as f64 * 0.9) as i32);
    QUICKLOOK.store(true, Ordering::SeqCst);
    set_rect(h, wa.left + (wa_w - ww) / 2, wa.top + (wa_h - wh) / 2, ww, wh);
}

/// Find one of our own top-level windows by its title (egui doesn't hand out child viewports' handles).
pub fn find_own_window(title: &str) -> Option<HWND> {
    let t = wide(title);
    let h = unsafe { FindWindowW(PCWSTR::null(), PCWSTR(t.as_ptr())) }.ok()?;
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    (pid == std::process::id()).then_some(h)
}

/// Rounded corners and the system shadow for a frameless window (Windows 11).
pub fn style_frameless(h: HWND, round: bool) {
    use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND};
    use windows::Win32::UI::Controls::MARGINS;
    unsafe {
        let pref = if round { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
        let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
        let m = MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 };
        let _ = DwmExtendFrameIntoClientArea(h, &m);
    }
}

/// The acrylic backdrop behind a translucent window ("Translucent window" setting).
pub fn set_backdrop(h: HWND, acrylic: bool) {
    use windows::Win32::Graphics::Dwm::{DwmExtendFrameIntoClientArea, DWMWA_SYSTEMBACKDROP_TYPE, DWMWA_USE_IMMERSIVE_DARK_MODE};
    use windows::Win32::UI::Controls::MARGINS;
    unsafe {
        // 3 = DWMSBT_TRANSIENTWINDOW (acrylic), 1 = none
        let kind: i32 = if acrylic { 3 } else { 1 };
        let dark: i32 = crate::ui::theme::is_dark(&settings::load()) as i32;
        let _ = DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as _, 4);
        let m = if acrylic { MARGINS { cxLeftWidth: -1, cxRightWidth: -1, cyTopHeight: -1, cyBottomHeight: -1 } } else { MARGINS { cxLeftWidth: 1, cxRightWidth: 1, cyTopHeight: 1, cyBottomHeight: 1 } };
        let _ = DwmExtendFrameIntoClientArea(h, &m);
        let _ = DwmSetWindowAttribute(h, DWMWA_SYSTEMBACKDROP_TYPE, &kind as *const _ as _, 4);
    }
}

/// No system menu, so DWM draws no (greyed) close button on the acrylic frame. winit puts the style
/// back when it changes the window, so this runs before every opening.
fn no_sysmenu(h: HWND) {
    unsafe {
        let st = GetWindowLongPtrW(h, GWL_STYLE);
        if st & WS_SYSMENU.0 as isize != 0 {
            let _ = SetWindowLongPtrW(h, GWL_STYLE, st & !(WS_SYSMENU.0 as isize));
            let _ = SetWindowPos(h, None, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED);
        }
    }
}

/// A see-through window that only paints its cards (the notices): no Windows 11 border, no
/// rounded frame, no system menu (DWM would draw a greyed close button).
pub fn style_bare(h: HWND) {
    use windows::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND};
    unsafe {
        let none: u32 = 0xFFFF_FFFE; // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR, &none as *const _ as _, 4);
        let pref = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &pref as *const _ as _, 4);
    }
    no_sysmenu(h);
}

/// Never in the taskbar or Alt+Tab.
pub fn make_tool_window(h: HWND) {
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize));
    }
}

fn dbg_log(m: &str) {
    if std::env::var_os("GLINT_TEST_KEYLOG").is_some() {
        eprintln!("HOST {m}");
    }
}

pub fn timing(step: &str) {
    if std::env::var_os("GLINT_TEST_TIMING").is_some() {
        if let Some(t) = *SHOWN_AT.lock() {
            eprintln!("TIMING {step} {:.1} ms", t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
