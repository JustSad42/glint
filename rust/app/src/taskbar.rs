//! Your taskbar comes up with Glint, like it does with the real Start menu, without touching
//! auto-hide (so no window ever gets resized).
//!
//! An auto-hidden taskbar slides up when it's activated, and stays up while the window in front
//! belongs to it. So: Glint's window is made an owned window of the taskbar while it's open, the
//! taskbar is activated for an instant, and then Glint takes the focus. When Glint hides, the
//! ownership is dropped and the taskbar hides as usual.
use crate::{settings, wins};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_GETTASKBARPOS, ABM_SETSTATE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, SetWindowLongPtrW, GWLP_HWNDPARENT};

const ABS_AUTOHIDE: isize = 1;
const ABE_BOTTOM: u32 = 3;

/// The taskbar was brought up for the open Glint window.
static LIFTED: AtomicBool = AtomicBool::new(false);

fn tray() -> Option<HWND> {
    unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd"), None) }.ok()
}

fn data() -> APPBARDATA {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    if let Some(h) = tray() {
        d.hWnd = h;
    }
    d
}

fn auto_hide_on() -> bool {
    let mut d = data();
    (unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) } as isize) & ABS_AUTOHIDE != 0
}

/// Glint is opening: hand the window to the taskbar (only if it auto-hides and the setting is
/// on). Call before placing the window, so it can leave room; then `kick` once it has focus.
pub fn prepare(main: HWND) {
    if !settings::load().show_taskbar || !auto_hide_on() {
        return;
    }
    let Some(t) = tray() else { return };
    unsafe {
        SetWindowLongPtrW(main, GWLP_HWNDPARENT, t.0 as isize);
    }
    LIFTED.store(true, Ordering::SeqCst);
}

/// Glint has the focus: activate the taskbar for an instant (it slides up) and take the focus
/// straight back. Plain calls only, while Glint is the foreground app: borrowing other threads'
/// input queues here lets the previous app grab the focus back a second later.
pub fn kick(main: HWND) {
    if !LIFTED.load(Ordering::SeqCst) {
        return;
    }
    let Some(t) = tray() else { return };
    let (main_raw, t_raw) = (main.0 as isize, t.0 as isize);
    // off the UI thread: it waits a moment for the taskbar to take the focus
    std::thread::spawn(move || unsafe {
        use std::time::{Duration, Instant};
        use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};
        let (main, t) = (HWND(main_raw as *mut _), HWND(t_raw as *mut _));
        if GetForegroundWindow() != main {
            return; // never got the focus; don't make it worse
        }
        let _ = SetForegroundWindow(t);
        // the switch happens on Explorer's side a moment later; asking for the focus back before
        // that is a no-op and the taskbar keeps it
        let start = Instant::now();
        while GetForegroundWindow() != t && start.elapsed() < Duration::from_millis(250) {
            std::thread::sleep(Duration::from_millis(4));
        }
        if !LIFTED.load(Ordering::SeqCst) {
            return; // Glint was closed meanwhile
        }
        let _ = SetForegroundWindow(main);
        if GetForegroundWindow() != main {
            wins::force_foreground(main);
        }
    });
}

/// Glint is hiding: let the taskbar hide again. Returns whether it had been brought up.
pub fn lower(main: HWND) -> bool {
    if LIFTED.swap(false, Ordering::SeqCst) {
        unsafe {
            SetWindowLongPtrW(main, GWLP_HWNDPARENT, 0);
        }
        true
    } else {
        false
    }
}

/// After Glint hid: Windows hands the focus to the next window in line, which is the taskbar
/// (it's right under Glint), and a focused taskbar never hides. Give the focus back to the
/// window you came from instead, unless you're using the taskbar with the mouse right now.
pub fn hand_back(prev: isize) {
    let Some(t) = tray() else { return };
    let t_raw = t.0 as isize;
    std::thread::spawn(move || unsafe {
        use std::time::{Duration, Instant};
        use windows::Win32::Foundation::POINT;
        use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowRect, IsWindow, IsWindowVisible, SetForegroundWindow};
        let t = HWND(t_raw as *mut _);
        let start = Instant::now();
        while start.elapsed() < Duration::from_millis(200) {
            if GetForegroundWindow() == t {
                let mut p = POINT::default();
                let mut r = RECT::default();
                let _ = GetCursorPos(&mut p);
                let _ = GetWindowRect(t, &mut r);
                if p.x >= r.left && p.x < r.right && p.y >= r.top && p.y < r.bottom {
                    return; // you clicked something on the taskbar
                }
                let back = HWND(prev as *mut _);
                let target = if prev != 0 && IsWindow(Some(back)).as_bool() && IsWindowVisible(back).as_bool() {
                    back
                } else {
                    FindWindowW(windows::core::w!("Progman"), None).unwrap_or_default()
                };
                if !target.0.is_null() {
                    let _ = SetForegroundWindow(target);
                    if GetForegroundWindow() == t {
                        wins::force_foreground(target);
                    }
                }
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });
}

/// While the taskbar is up: the strip it covers at the bottom of the monitor `mon` (physical
/// pixels), so the Start-menu layout can sit above it. 0 when it isn't up or isn't there.
pub fn covered_bottom(mon: RECT) -> i32 {
    if !LIFTED.load(Ordering::SeqCst) {
        return 0;
    }
    let mut d = data();
    if unsafe { SHAppBarMessage(ABM_GETTASKBARPOS, &mut d) } == 0 || d.uEdge != ABE_BOTTOM {
        return 0;
    }
    let r = d.rc;
    let on_this = r.left < mon.right && r.right > mon.left && r.bottom >= mon.bottom - 4 && r.top < mon.bottom;
    if on_this {
        r.bottom - r.top
    } else {
        0
    }
}

/// Glint 1.3.0 switched auto-hide off while open and left a marker; if it ended while the
/// taskbar was up, put auto-hide back.
pub fn restore_after_crash() {
    let marker = settings::DATA_DIR.join("taskbar.flag");
    if marker.exists() {
        let mut d = data();
        d.lParam = windows::Win32::Foundation::LPARAM(ABS_AUTOHIDE);
        unsafe {
            SHAppBarMessage(ABM_SETSTATE, &mut d);
        }
        let _ = std::fs::remove_file(marker);
    }
}
