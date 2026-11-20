//! Settings and the data folder (%LOCALAPPDATA%\Glint, or GLINT_DATA for test copies).
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub static DATA_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let dir = std::env::var_os("GLINT_DATA").map(PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Glint")
    });
    let _ = std::fs::create_dir_all(&dir);
    dir
});

pub fn data_file(name: &str) -> PathBuf {
    DATA_DIR.join(name)
}

/// Next to the running .exe (the install folder).
pub fn exe_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
}

pub fn user_home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Users\Default"))
}

pub fn default_filepilot() -> String {
    let p = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default().join(r"Voidstar\FilePilot\FPilot.exe");
    if p.exists() {
        p.to_string_lossy().into_owned()
    } else {
        String::new()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    /// pressing and releasing the Windows key alone opens Glint instead of Start
    pub win_key: bool,
    /// Alt+Space also opens Glint
    pub alt_space: bool,
    /// Win+E opens File Pilot instead of File Explorer
    pub win_e_filepilot: bool,
    pub filepilot_path: String,
    /// "explorer" or "filepilot": what opens folders from Glint
    pub folder_opener: String,
    pub start_with_windows: bool,
    /// tray icon; when off Glint runs fully hidden
    pub show_tray: bool,
    pub check_updates: bool,
    /// "system" | "dark" | "light"
    pub theme: String,
    /// accent color, CSS hex
    pub accent: String,
    /// "top" | "center"
    pub position: String,
    pub blur: bool,
    pub show_preview: bool,
    pub content_search: bool,
    pub content_roots: Vec<String>,
    /// folder names or path fragments to leave out of content search
    pub content_excludes: Vec<String>,
    pub content_max_mb: u64,
    pub web_engine: String,
    pub web_fallback: bool,
    pub max_files: usize,
    /// now-playing card and play/pause/next rows
    pub media_controls: bool,
    /// search ClipVault's clipboard history
    pub clipvault: bool,
    /// Enter on a clip pastes it into the window you came from (not just copies it)
    pub clip_paste: bool,
    /// which screen Glint opens on: "cursor" (the one under the mouse) or "primary"
    pub monitor: String,
    /// "popup" (floating search) or "start" (docked above the taskbar like the classic Start menu)
    pub layout: String,
    /// where the Start-menu layout opens: "left" (bottom-left corner), "bottom" (bottom center) or "center"
    pub start_position: String,
    /// the name shown in the Start-menu layout (empty: your Windows user name)
    pub display_name: String,
    /// time and date in the search bar
    pub show_clock: bool,
    /// search browser bookmarks and open tabs
    pub browser_search: bool,
    /// ...and browser history
    pub browser_history: bool,
    /// put the search window's web view to sleep while it's hidden (less memory)
    pub suspend_hidden: bool,
    pub first_run_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let fp = default_filepilot();
        Settings {
            win_key: true,
            alt_space: false,
            win_e_filepilot: !fp.is_empty(),
            folder_opener: if fp.is_empty() { "explorer".into() } else { "filepilot".into() },
            filepilot_path: fp,
            start_with_windows: true,
            show_tray: true,
            check_updates: true,
            theme: "system".into(),
            accent: "#7c6cff".into(),
            position: "top".into(),
            blur: true,
            show_preview: true,
            content_search: true,
            content_roots: vec![user_home().to_string_lossy().into_owned()],
            content_excludes: vec![],
            content_max_mb: 30,
            web_engine: "google".into(),
            web_fallback: true,
            max_files: 8,
            media_controls: true,
            clipvault: true,
            clip_paste: true,
            monitor: "cursor".into(),
            layout: "popup".into(),
            start_position: "left".into(),
            display_name: String::new(),
            show_clock: true,
            browser_search: true,
            browser_history: true,
            suspend_hidden: true,
            first_run_done: false,
        }
    }
}

static CACHE: Mutex<Option<Settings>> = Mutex::new(None);

pub fn load() -> Settings {
    let mut c = CACHE.lock();
    if let Some(s) = c.as_ref() {
        return s.clone();
    }
    let s: Settings = std::fs::read_to_string(data_file("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    *c = Some(s.clone());
    s
}

pub fn save(s: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(s).map_err(|e| e.to_string())?;
    write_atomic(&data_file("settings.json"), text.as_bytes()).map_err(|e| e.to_string())?;
    *CACHE.lock() = Some(s.clone());
    Ok(())
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Start with Windows, hidden. Three layers, so one being switched off or delayed doesn't matter:
/// - scheduled task "Glint" at sign-in (runs at once; Run-key apps are held back for a while),
/// - scheduled task "Glint Watchdog": every 5 minutes, starts Glint hidden if it isn't running
///   (a crash doesn't leave you without search). If Glint is running, the new copy just exits.
///   Quitting Glint on purpose leaves a marker, so the watchdog stays out of it until next sign-in.
/// - the classic Run key, which is also what Settings > Startup apps shows.
pub fn set_autostart(on: bool) {
    use winreg::enums::*;
    if let Ok((key, _)) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) {
        if on {
            if let Ok(exe) = std::env::current_exe() {
                let _ = key.set_value("Glint", &format!("\"{}\" --tray", exe.display()));
            }
        } else {
            let _ = key.delete_value("Glint");
        }
    }
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let exe = exe.to_string_lossy().into_owned();
            let _ = create_task(TASK, &exe, "--tray", "", None);
            let _ = create_task(WATCHDOG, &exe, "--tray --watchdog", "PT2M", Some("PT5M"));
        }
    } else {
        for t in [TASK, WATCHDOG] {
            let _ = schtasks(&["/Delete", "/TN", t, "/F"]);
        }
    }
}

const TASK: &str = "Glint";
const WATCHDOG: &str = "Glint Watchdog";

/// Is everything for start-with-Windows in place? (the tasks can be deleted by hand)
pub fn autostart_ok() -> bool {
    schtasks(&["/Query", "/TN", TASK]).is_ok() && schtasks(&["/Query", "/TN", WATCHDOG]).is_ok()
}

fn schtasks(args: &[&str]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("schtasks.exe").args(args).creation_flags(0x0800_0000).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn create_task(name: &str, exe: &str, args: &str, delay: &str, repeat: Option<&str>) -> Result<(), String> {
    let user = format!("{}\\{}", std::env::var("USERDOMAIN").unwrap_or_default(), std::env::var("USERNAME").unwrap_or_default());
    let delay = if delay.is_empty() { String::new() } else { format!("<Delay>{delay}</Delay>") };
    let repeat = repeat.map(|r| format!("<Repetition><Interval>{r}</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition>")).unwrap_or_default();
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Author>Glint</Author><Description>Starts Glint hidden when you sign in.</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId>{delay}{repeat}</LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>false</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings><StopOnIdleEnd>false</StopOnIdleEnd><RestartOnIdle>false</RestartOnIdle></IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>5</Priority>
  </Settings>
  <Actions Context="Author"><Exec><Command>{exe}</Command><Arguments>{args}</Arguments></Exec></Actions>
</Task>"#,
        user = xml_escape(&user),
        exe = xml_escape(exe),
        args = xml_escape(args),
    );
    let path = std::env::temp_dir().join(format!("glint-task-{}.xml", std::process::id()));
    // schtasks wants UTF-16 with a BOM for an encoding="UTF-16" file
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    let r = schtasks(&["/Create", "/TN", name, "/XML", &path.to_string_lossy(), "/F"]);
    let _ = std::fs::remove_file(&path);
    r
}

/// Set when you quit Glint yourself; the watchdog leaves it alone until the next normal start.
pub fn quit_marker() -> PathBuf {
    data_file("quit.flag")
}
