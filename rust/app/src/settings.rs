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

pub fn set_autostart(on: bool) {
    use winreg::enums::*;
    let Ok((key, _)) = winreg::RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_KEY) else { return };
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value("Glint", &format!("\"{}\" --tray", exe.display()));
        }
    } else {
        let _ = key.delete_value("Glint");
    }
}
