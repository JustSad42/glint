//! Built-in commands: system actions, Windows Settings pages, admin tools and Glint itself.
use crate::{fuzzy, history, shellops};
use serde_json::{json, Value};
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
use windows::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};

#[derive(Clone, Copy)]
pub enum Run {
    Uri(&'static str),
    Exe(&'static str, &'static str),
    Lock,
    Sleep,
    Hibernate,
    Shutdown,
    Restart,
    SignOut,
    EmptyBin,
    ToggleTheme,
    /// handled by the app: settings, quit, rebuild...
    Glint(&'static str),
}

pub struct Action {
    pub id: &'static str,
    pub title: &'static str,
    pub sub: &'static str,
    pub words: &'static str,
    pub glyph: &'static str,
    pub confirm: bool,
    pub run: Run,
}

macro_rules! a {
    ($id:expr, $title:expr, $sub:expr, $words:expr, $glyph:expr, $confirm:expr, $run:expr) => {
        Action { id: $id, title: $title, sub: $sub, words: $words, glyph: $glyph, confirm: $confirm, run: $run }
    };
}

pub const ACTIONS: &[Action] = &[
    a!("startmenu", "Start menu", "Open the Windows Start menu", "start menu windows", "grid", false, Run::Glint("start")),
    a!("lock", "Lock", "Lock your PC", "lock screen away", "lock", false, Run::Lock),
    a!("sleep", "Sleep", "Put your PC to sleep", "suspend standby", "moon", false, Run::Sleep),
    a!("hibernate", "Hibernate", "Save your session and power off", "hibernate", "moon", true, Run::Hibernate),
    a!("restart", "Restart", "Restart your PC", "reboot restart", "restart", true, Run::Restart),
    a!("shutdown", "Shut down", "Turn off your PC", "power off shutdown turn off", "power", true, Run::Shutdown),
    a!("signout", "Sign out", "Sign out of Windows", "log off logout sign out", "logout", true, Run::SignOut),
    a!("emptybin", "Empty Recycle Bin", "Permanently delete everything in the Recycle Bin", "trash clear recycle bin empty", "trash", true, Run::EmptyBin),
    a!("recyclebin", "Recycle Bin", "Open the Recycle Bin", "trash deleted files", "trash", false, Run::Uri("shell:RecycleBinFolder")),
    a!("theme", "Toggle dark mode", "Switch Windows between dark and light", "dark mode light mode theme night", "contrast", false, Run::ToggleTheme),
    a!("snip", "Screen snip", "Take a screenshot of part of the screen", "screenshot snipping capture print screen", "crop", false, Run::Uri("ms-screenclip:")),
    a!("taskmgr", "Task Manager", "See and end running apps", "processes performance task manager", "activity", false, Run::Exe("taskmgr.exe", "")),
    a!("settings", "Windows Settings", "Open Settings", "settings preferences control", "gear", false, Run::Uri("ms-settings:")),
    a!("wifi", "Wi-Fi settings", "Settings › Network › Wi-Fi", "wifi wireless network internet", "wifi", false, Run::Uri("ms-settings:network-wifi")),
    a!("network", "Network & internet", "Settings › Network", "ethernet network internet ip", "globe", false, Run::Uri("ms-settings:network")),
    a!("vpn", "VPN settings", "Settings › Network › VPN", "vpn", "shield", false, Run::Uri("ms-settings:network-vpn")),
    a!("bluetooth", "Bluetooth & devices", "Settings › Bluetooth", "bluetooth devices pair headphones", "bluetooth", false, Run::Uri("ms-settings:bluetooth")),
    a!("display", "Display settings", "Settings › Display", "display monitor resolution scale brightness hdr refresh", "monitor", false, Run::Uri("ms-settings:display")),
    a!("nightlight", "Night light", "Settings › Display › Night light", "night light blue light warm", "moon", false, Run::Uri("ms-settings:nightlight")),
    a!("sound", "Sound settings", "Settings › Sound", "sound audio volume speakers microphone output input", "volume", false, Run::Uri("ms-settings:sound")),
    a!("mixer", "Volume mixer", "Per-app volume", "volume mixer app volume", "volume", false, Run::Uri("ms-settings:apps-volume")),
    a!("apps", "Installed apps", "Settings › Apps › Installed apps (uninstall)", "uninstall remove programs apps features", "grid", false, Run::Uri("ms-settings:appsfeatures")),
    a!("defaultapps", "Default apps", "Settings › Apps › Default apps", "default apps browser open with", "grid", false, Run::Uri("ms-settings:defaultapps")),
    a!("startup", "Startup apps", "Settings › Apps › Startup", "startup boot autostart", "rocket", false, Run::Uri("ms-settings:startupapps")),
    a!("update", "Windows Update", "Settings › Windows Update", "windows update updates patch", "download", false, Run::Uri("ms-settings:windowsupdate")),
    a!("storage", "Storage", "Settings › System › Storage", "storage disk space cleanup", "hdd", false, Run::Uri("ms-settings:storagesense")),
    a!("power", "Power & battery", "Settings › System › Power", "power battery energy plan", "battery", false, Run::Uri("ms-settings:powersleep")),
    a!("personalize", "Personalization", "Settings › Personalization", "wallpaper background colors accent personalize", "palette", false, Run::Uri("ms-settings:personalization")),
    a!("taskbarsettings", "Taskbar settings", "Settings › Personalization › Taskbar", "taskbar", "layout", false, Run::Uri("ms-settings:taskbar")),
    a!("mouse", "Mouse settings", "Settings › Bluetooth & devices › Mouse", "mouse pointer cursor speed", "mouse", false, Run::Uri("ms-settings:mousetouchpad")),
    a!("keyboard", "Keyboard & language", "Settings › Time & language › Typing", "keyboard typing language input", "keyboard", false, Run::Uri("ms-settings:typing")),
    a!("notifications", "Notifications", "Settings › System › Notifications", "notifications do not disturb focus", "bell", false, Run::Uri("ms-settings:notifications")),
    a!("datetime", "Date & time", "Settings › Time & language", "date time clock timezone", "clock", false, Run::Uri("ms-settings:dateandtime")),
    a!("privacy", "Privacy & security", "Settings › Privacy & security", "privacy security permissions camera microphone", "shield", false, Run::Uri("ms-settings:privacy")),
    a!("printers", "Printers & scanners", "Settings › Bluetooth & devices", "printer scanner print", "printer", false, Run::Uri("ms-settings:printers")),
    a!("about", "About this PC", "Specs, Windows version, device name", "about system info specs version pc name", "info", false, Run::Uri("ms-settings:about")),
    a!("controlpanel", "Control Panel", "Classic Control Panel", "control panel", "sliders", false, Run::Exe("control.exe", "")),
    a!("devmgmt", "Device Manager", "Drivers and hardware", "device manager drivers hardware", "cpu", false, Run::Exe("mmc.exe", "devmgmt.msc")),
    a!("diskmgmt", "Disk Management", "Partitions and drives", "disk management partition format drive", "hdd", false, Run::Exe("mmc.exe", "diskmgmt.msc")),
    a!("services", "Services", "Windows services", "services", "server", false, Run::Exe("mmc.exe", "services.msc")),
    a!("eventvwr", "Event Viewer", "System logs", "event viewer logs errors", "list", false, Run::Exe("mmc.exe", "eventvwr.msc")),
    a!("regedit", "Registry Editor", "regedit", "registry regedit", "database", false, Run::Exe("regedit.exe", "")),
    a!("envvars", "Environment variables", "Edit PATH and other variables", "environment variables path env", "terminal", false, Run::Exe("rundll32.exe", "sysdm.cpl,EditEnvironmentVariables")),
    a!("terminal", "Terminal", "Open Windows Terminal / PowerShell", "terminal powershell cmd console shell", "terminal", false, Run::Glint("terminal")),
    a!("g-settings", "Glint settings", "Hotkeys, look, content search", "glint settings preferences options config", "glint", false, Run::Glint("settings")),
    a!("g-rebuild", "Rebuild content index", "Read all your documents again", "glint reindex rebuild index content", "refresh", false, Run::Glint("rebuild")),
    a!("g-apps", "Refresh app list", "Look for newly installed apps", "glint refresh apps reload", "refresh", false, Run::Glint("apps")),
    a!("g-history", "Clear Glint history", "Forget what you opened and searched", "glint clear history forget recent", "eraser", true, Run::Glint("history")),
    a!("g-updates", "Check for Glint updates", "", "glint update version", "download", false, Run::Glint("updates")),
    a!("g-quit", "Quit Glint", "Close Glint until you start it again", "glint quit exit close", "power", false, Run::Glint("quit")),
];

pub fn item(a: &Action, score: i32) -> Value {
    json!({
        "key": format!("act:{}", a.id), "kind": "action", "title": a.title, "subtitle": a.sub, "glyph": a.glyph,
        "id": a.id, "confirm": a.confirm, "score": score,
    })
}

pub fn search(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let mut out: Vec<(i32, Value)> = ACTIONS
        .iter()
        .filter_map(|a| {
            let by_title = fuzzy::score(q, a.title);
            // keywords only count as whole-word prefixes, so they don't swamp results
            let by_words = a.words.split(' ').any(|w| w.starts_with(&q.trim().to_lowercase()) && q.trim().len() >= 2).then_some(640);
            let s = by_title.max(by_words)?;
            let s = s - 60 + history::boost(&format!("act:{}", a.id), q);
            Some((s, item(a, s)))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

fn toggle_theme() -> Result<String, String> {
    use winreg::enums::*;
    let key = winreg::RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize", KEY_READ | KEY_WRITE)
        .map_err(|e| e.to_string())?;
    let light: u32 = key.get_value("AppsUseLightTheme").unwrap_or(1);
    let new = if light == 1 { 0u32 } else { 1u32 };
    key.set_value("AppsUseLightTheme", &new).map_err(|e| e.to_string())?;
    key.set_value("SystemUsesLightTheme", &new).map_err(|e| e.to_string())?;
    unsafe {
        let _ = SendMessageTimeoutW(
            HWND(0xffff as *mut _),
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            200,
            None,
        );
    }
    Ok(if new == 0 { "Dark mode on".into() } else { "Light mode on".into() })
}

/// Run a built-in action. Returns a short message to show, or the name of an app command.
pub enum Outcome {
    Done(Option<String>),
    Glint(&'static str),
}

pub fn run(id: &str) -> Result<Outcome, String> {
    let a = ACTIONS.iter().find(|a| a.id == id).ok_or("Unknown action")?;
    let shutdown = |args: &[&str]| std::process::Command::new("shutdown.exe").args(args).spawn().map(|_| ()).map_err(|e| e.to_string());
    match a.run {
        Run::Uri(u) => shellops::open_url(u)?,
        Run::Exe(exe, args) => shellops::shell_execute(exe, None, if args.is_empty() { None } else { Some(args) }, None)?,
        Run::Lock => unsafe { LockWorkStation().map_err(|e| e.to_string())? },
        Run::Sleep => {
            // give the launcher a moment to hide first
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(400));
                unsafe {
                    SetSuspendState(false, false, false);
                }
            });
        }
        Run::Hibernate => shutdown(&["/h"])?,
        Run::Shutdown => shutdown(&["/s", "/t", "0"])?,
        Run::Restart => shutdown(&["/r", "/t", "0"])?,
        Run::SignOut => shutdown(&["/l"])?,
        Run::EmptyBin => {
            // flags: no confirmation, no progress UI, no sound
            let r = unsafe { SHEmptyRecycleBinW(None, windows::core::PCWSTR::null(), 0x1 | 0x2 | 0x4) };
            return Ok(Outcome::Done(Some(if r.is_ok() { "Recycle Bin emptied".into() } else { "Recycle Bin is already empty".into() })));
        }
        Run::ToggleTheme => return Ok(Outcome::Done(Some(toggle_theme()?))),
        Run::Glint(cmd) => return Ok(Outcome::Glint(cmd)),
    }
    Ok(Outcome::Done(None))
}
