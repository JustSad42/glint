//! Low-level keyboard hook.
//! - Windows key pressed and released on its own: open Glint instead of Start. Every Win+<key>
//!   shortcut keeps working, because the Win key itself is never blocked; Start is suppressed by
//!   tapping an unassigned key (0xE8) just before Win is released, the trick AutoHotkey uses.
//! - Win+E: open File Pilot instead of File Explorer.
//! - Alt+Space (optional): open Glint.
//! Keys typed into elevated (admin) windows don't reach a normal app's hook, so there Start
//! opens as usual.
use crossbeam_channel::Sender;
use once_cell::sync::OnceCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LMENU,
    VK_LWIN, VK_MENU, VK_RMENU, VK_RWIN, VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, HC_ACTION, KBDLLHOOKSTRUCT,
    LLKHF_ALTDOWN, LLKHF_INJECTED, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

#[derive(Debug, Clone, Copy)]
pub enum Event {
    Show,
    WinE,
}

static TX: OnceCell<Sender<Event>> = OnceCell::new();
static THREAD: AtomicU32 = AtomicU32::new(0);

pub static WIN_KEY: AtomicBool = AtomicBool::new(false);
pub static WIN_E: AtomicBool = AtomicBool::new(false);
pub static ALT_SPACE: AtomicBool = AtomicBool::new(false);

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// another key went down while Win was held
static COMBO: AtomicBool = AtomicBool::new(false);
/// we swallowed a key during this Win press (so Start must be masked)
static SWALLOWED: AtomicBool = AtomicBool::new(false);
static EAT_E_UP: AtomicBool = AtomicBool::new(false);
static EAT_SPACE_UP: AtomicBool = AtomicBool::new(false);
static MASK_ALT_UP: AtomicBool = AtomicBool::new(false);

/// marks the key presses we inject so the hook ignores them
const MARK: usize = 0x474C_4E54;
const VK_MASK: u16 = 0xE8;
const VK_E: u32 = 0x45;

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

/// Tap the unassigned key so Windows sees Win (or Alt) as part of a combination.
fn send_mask() {
    let key = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(VK_MASK), wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MARK } },
    };
    let inputs = [key(KEYBD_EVENT_FLAGS(0)), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

fn emit(e: Event) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(e);
    }
}

unsafe extern "system" fn proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let ours = k.flags.0 & LLKHF_INJECTED.0 != 0 && k.dwExtraInfo == MARK;
        if !ours {
            let msg = wp.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
            let vk = k.vkCode;
            let is_win = vk == VK_LWIN.0 as u32 || vk == VK_RWIN.0 as u32;

            if is_win {
                if down && !WIN_DOWN.swap(true, Ordering::SeqCst) {
                    COMBO.store(false, Ordering::SeqCst);
                    SWALLOWED.store(false, Ordering::SeqCst);
                }
                if up {
                    WIN_DOWN.store(false, Ordering::SeqCst);
                    let alone = !COMBO.load(Ordering::SeqCst) && !held(VK_CONTROL) && !held(VK_SHIFT) && !held(VK_MENU);
                    if alone && WIN_KEY.load(Ordering::SeqCst) {
                        send_mask();
                        emit(Event::Show);
                    } else if SWALLOWED.load(Ordering::SeqCst) {
                        send_mask();
                    }
                }
            } else if WIN_DOWN.load(Ordering::SeqCst) {
                if down {
                    COMBO.store(true, Ordering::SeqCst);
                    if vk == VK_E && WIN_E.load(Ordering::SeqCst) {
                        EAT_E_UP.store(true, Ordering::SeqCst);
                        SWALLOWED.store(true, Ordering::SeqCst);
                        emit(Event::WinE);
                        return LRESULT(1);
                    }
                }
                if up && vk == VK_E && EAT_E_UP.swap(false, Ordering::SeqCst) {
                    return LRESULT(1);
                }
            } else {
                if up && vk == VK_E && EAT_E_UP.swap(false, Ordering::SeqCst) {
                    return LRESULT(1);
                }
                // Alt+Space
                if vk == VK_SPACE.0 as u32 {
                    if down && ALT_SPACE.load(Ordering::SeqCst) && k.flags.0 & LLKHF_ALTDOWN.0 != 0 && !held(VK_CONTROL) {
                        EAT_SPACE_UP.store(true, Ordering::SeqCst);
                        MASK_ALT_UP.store(true, Ordering::SeqCst);
                        emit(Event::Show);
                        return LRESULT(1);
                    }
                    if up && EAT_SPACE_UP.swap(false, Ordering::SeqCst) {
                        return LRESULT(1);
                    }
                }
                // releasing Alt after Alt+Space would otherwise focus the app's menu bar
                if up && (vk == VK_LMENU.0 as u32 || vk == VK_RMENU.0 as u32 || vk == VK_MENU.0 as u32) && MASK_ALT_UP.swap(false, Ordering::SeqCst) {
                    send_mask();
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Install the hook on its own thread (with the message loop a low-level hook needs).
pub fn start(tx: Sender<Event>) {
    let _ = TX.set(tx);
    if THREAD.load(Ordering::SeqCst) != 0 {
        return;
    }
    std::thread::Builder::new()
        .name("keyboard-hook".into())
        .spawn(|| unsafe {
            THREAD.store(GetCurrentThreadId(), Ordering::SeqCst);
            let module = GetModuleHandleW(None).ok().map(|m| HINSTANCE(m.0));
            let hook = match SetWindowsHookExW(WH_KEYBOARD_LL, Some(proc), module, 0) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("keyboard hook failed: {e}");
                    THREAD.store(0, Ordering::SeqCst);
                    return;
                }
            };
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
            THREAD.store(0, Ordering::SeqCst);
        })
        .ok();
}

pub fn stop() {
    let t = THREAD.load(Ordering::SeqCst);
    if t != 0 {
        unsafe {
            let _ = PostThreadMessageW(t, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

pub fn apply(s: &crate::settings::Settings) {
    WIN_KEY.store(s.win_key, Ordering::SeqCst);
    WIN_E.store(s.win_e_filepilot && std::path::Path::new(&s.filepilot_path).exists(), Ordering::SeqCst);
    ALT_SPACE.store(s.alt_space, Ordering::SeqCst);
}

/// Press Ctrl+V in whatever window is in front.
pub fn send_paste() {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MARK } },
    };
    let v = VIRTUAL_KEY(0x56);
    let inputs = [key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)), key(v, KEYBD_EVENT_FLAGS(0)), key(v, KEYEVENTF_KEYUP), key(VK_CONTROL, KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Open the real Start menu (Ctrl+Esc).
pub fn open_start_menu() {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: MARK } },
    };
    let esc = VIRTUAL_KEY(0x1B);
    let inputs = [key(VK_CONTROL, KEYBD_EVENT_FLAGS(0)), key(esc, KEYBD_EVENT_FLAGS(0)), key(esc, KEYEVENTF_KEYUP), key(VK_CONTROL, KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}
