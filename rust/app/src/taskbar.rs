//! Your taskbar comes up with Glint, like it does with the real Start menu.
//!
//! An auto-hidden taskbar only stays up while it has the focus or the mouse is on it, and Glint
//! needs the focus for typing. So while Glint is open, auto-hide is switched off, and it's
//! switched back on the moment Glint hides. A marker file covers a crash in between: the next
//! start puts auto-hide back.
use crate::settings;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

const ABS_AUTOHIDE: isize = 1;
const ABS_ALWAYSONTOP: isize = 2;

/// Glint switched auto-hide off and owes it back.
static LIFTED: AtomicBool = AtomicBool::new(false);

fn marker() -> std::path::PathBuf {
    settings::DATA_DIR.join("taskbar.flag")
}

fn data() -> APPBARDATA {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    if let Ok(h) = unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd"), None) } {
        d.hWnd = h;
    }
    d
}

fn auto_hide_on() -> bool {
    let mut d = data();
    (unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) } as isize) & ABS_AUTOHIDE != 0
}

fn set_auto_hide(on: bool) {
    let mut d = data();
    d.lParam = LPARAM(if on { ABS_AUTOHIDE } else { ABS_ALWAYSONTOP });
    unsafe {
        SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// Glint is opening: bring the taskbar up (only if it auto-hides and the setting is on).
pub fn lift() {
    if LIFTED.load(Ordering::SeqCst) || !settings::load().show_taskbar || !auto_hide_on() {
        return;
    }
    let _ = std::fs::write(marker(), b"auto-hide switched off while Glint is open");
    LIFTED.store(true, Ordering::SeqCst);
    set_auto_hide(false);
}

/// Glint is hiding (or quitting): let the taskbar hide again.
pub fn lower() {
    if LIFTED.swap(false, Ordering::SeqCst) {
        set_auto_hide(true);
        let _ = std::fs::remove_file(marker());
    }
}

/// At startup: if Glint ended while the taskbar was up, put auto-hide back.
pub fn restore_after_crash() {
    if marker().exists() {
        set_auto_hide(true);
        let _ = std::fs::remove_file(marker());
    }
}
