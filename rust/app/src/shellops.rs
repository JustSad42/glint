//! Things done to files: open, reveal, run as admin, copy, recycle, open a terminal there...
use crate::settings;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::core::PCWSTR;
use windows::Win32::UI::Shell::{
    ShellExecuteExW, SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_WANTNUKEWARNING, FOF_SILENT, FO_DELETE, SEE_MASK_INVOKEIDLIST, SEE_MASK_NOASYNC,
    SHELLEXECUTEINFOW, SHFILEOPSTRUCTW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// ShellExecuteEx with an optional verb ("runas", "properties", ...).
pub fn shell_execute(file: &str, verb: Option<&str>, params: Option<&str>, dir: Option<&str>) -> Result<(), String> {
    let f = wide(file);
    let v = verb.map(wide);
    let p = params.map(wide);
    let d = dir.map(wide);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | if verb == Some("properties") { SEE_MASK_INVOKEIDLIST } else { Default::default() },
        lpVerb: v.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: p.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        lpDirectory: d.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| {
        // the user pressing "No" on the UAC prompt isn't an error worth showing
        if e.code().0 as u32 == 0x800704C7 {
            "Cancelled".to_string()
        } else {
            e.message().to_string()
        }
    })
}

fn filepilot() -> Option<PathBuf> {
    let s = settings::load();
    let p = PathBuf::from(&s.filepilot_path);
    (!s.filepilot_path.is_empty() && p.exists()).then_some(p)
}

pub fn launch_filepilot(arg: Option<&str>) -> bool {
    let Some(fp) = filepilot() else { return false };
    let mut c = Command::new(fp);
    if let Some(a) = arg {
        c.arg(a);
    }
    c.spawn().is_ok()
}

/// Open a folder in the chosen file manager.
pub fn open_folder(path: &str) -> Result<(), String> {
    if settings::load().folder_opener == "filepilot" && launch_filepilot(Some(path)) {
        return Ok(());
    }
    shell_execute(path, None, None, None)
}

pub fn open(path: &str) -> Result<(), String> {
    if Path::new(path).is_dir() {
        return open_folder(path);
    }
    let dir = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned());
    shell_execute(path, None, None, dir.as_deref())
}

/// Show a file selected in its folder.
pub fn reveal(path: &str) -> Result<(), String> {
    if settings::load().folder_opener == "filepilot" {
        if let Some(parent) = Path::new(path).parent() {
            if launch_filepilot(Some(&parent.to_string_lossy())) {
                return Ok(());
            }
        }
    }
    Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn open_in_explorer(path: &str) -> Result<(), String> {
    Command::new("explorer.exe").arg(path).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn run_as_admin(path: &str) -> Result<(), String> {
    let dir = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned());
    shell_execute(path, Some("runas"), None, dir.as_deref())
}

pub fn properties(path: &str) -> Result<(), String> {
    shell_execute(path, Some("properties"), None, None)
}

pub fn open_with(path: &str) -> Result<(), String> {
    Command::new("rundll32.exe").raw_arg(format!("shell32.dll,OpenAs_RunDLL {path}")).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn copy_text(s: &str) -> Result<(), String> {
    clipboard_win::set_clipboard_string(s).map_err(|e| e.to_string())
}

pub fn copy_files(paths: &[String]) -> Result<(), String> {
    use clipboard_win::Setter;
    let _clip = clipboard_win::Clipboard::new_attempts(10).map_err(|e| e.to_string())?;
    clipboard_win::raw::empty().map_err(|e| e.to_string())?;
    clipboard_win::formats::FileList.write_clipboard(paths).map_err(|e| e.to_string())
}

/// Move to the Recycle Bin (undoable).
pub fn recycle(path: &str) -> Result<(), String> {
    let mut from: Vec<u16> = path.encode_utf16().collect();
    from.push(0);
    from.push(0); // double-null terminated list
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        // NUKEWARNING: ask before a delete that can't be recycled (network or removable drive)
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING).0 as u16,
        ..Default::default()
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r == 0 && !op.fAnyOperationsAborted.as_bool() {
        Ok(())
    } else {
        Err(format!("Couldn't move it to the Recycle Bin (error {r})"))
    }
}

fn windows_terminal() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Microsoft\WindowsApps\wt.exe");
    p.exists().then_some(p)
}

/// A terminal opened in `dir`.
pub fn terminal(dir: &str) -> Result<(), String> {
    let dir = if Path::new(dir).is_dir() { dir.to_string() } else { Path::new(dir).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default() };
    if let Some(wt) = windows_terminal() {
        return Command::new(wt).arg("-d").arg(&dir).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    Command::new("powershell.exe").arg("-NoExit").current_dir(&dir).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Run a command line in a new PowerShell window that stays open.
pub fn run_command(cmd: &str, admin: bool) -> Result<(), String> {
    let home = settings::user_home().to_string_lossy().into_owned();
    let ps_args = format!("-NoExit -Command {cmd}");
    if admin {
        return shell_execute("powershell.exe", Some("runas"), Some(&ps_args), Some(&home));
    }
    if let Some(wt) = windows_terminal() {
        return Command::new(wt).args(["-d", &home, "powershell.exe", "-NoExit", "-Command", cmd]).spawn().map(|_| ()).map_err(|e| e.to_string());
    }
    Command::new("powershell.exe").args(["-NoExit", "-Command", cmd]).current_dir(&home).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn vscode() -> Option<PathBuf> {
    let p = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Programs\Microsoft VS Code\Code.exe");
    if p.exists() {
        return Some(p);
    }
    let p = PathBuf::from(std::env::var_os("ProgramFiles")?).join(r"Microsoft VS Code\Code.exe");
    p.exists().then_some(p)
}

pub fn open_in_vscode(path: &str) -> Result<(), String> {
    let code = vscode().ok_or("VS Code isn't installed")?;
    Command::new(code).arg(path).creation_flags(CREATE_NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub fn open_url(url: &str) -> Result<(), String> {
    shell_execute(url, None, None, None)
}

/// Expand %VARS% and a leading ~ in a typed path.
pub fn expand_path(input: &str) -> String {
    let mut s = input.trim().trim_matches('"').to_string();
    if s == "~" || s.starts_with("~\\") || s.starts_with("~/") {
        s = format!("{}{}", settings::user_home().display(), &s[1..]);
    }
    let mut out = String::new();
    let mut rest = s.as_str();
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(b) = after.find('%') {
            let name = &after[..b];
            match std::env::var(name) {
                Ok(v) if !name.is_empty() => out.push_str(&v),
                _ => {
                    out.push('%');
                    out.push_str(name);
                    out.push('%');
                }
            }
            rest = &after[b + 1..];
        } else {
            out.push_str(&rest[a..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out.replace('/', "\\")
}
