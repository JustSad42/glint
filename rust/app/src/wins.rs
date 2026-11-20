//! Open windows (type to switch to one) and running processes (for "kill").
use crate::{fuzzy, icons};
use serde_json::{json, Value};
use std::path::Path;
use windows::core::BOOL;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId,
    IsIconic, IsWindowVisible, PostMessageW, SetForegroundWindow, ShowWindow, GWL_EXSTYLE, GW_OWNER, SW_RESTORE, SW_SHOW, WM_CLOSE, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW,
};

#[derive(Clone, Debug)]
pub struct Win {
    pub hwnd: isize,
    pub title: String,
    pub exe: String,
    pub pid: u32,
}

pub fn title_of(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

pub fn exe_of(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 1024];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(h);
        if ok {
            String::from_utf16_lossy(&buf[..n as usize])
        } else {
            String::new()
        }
    }
}

unsafe extern "system" fn collect(hwnd: HWND, lp: LPARAM) -> BOOL {
    let out = &mut *(lp.0 as *mut Vec<Win>);
    if !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    if GetWindow(hwnd, GW_OWNER).map(|o| !o.is_invalid()).unwrap_or(false) {
        return true.into();
    }
    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
    if ex & WS_EX_TOOLWINDOW.0 != 0 && ex & WS_EX_APPWINDOW.0 == 0 {
        return true.into();
    }
    let mut cloaked = 0u32;
    if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked as *mut _ as _, 4).is_ok() && cloaked != 0 {
        return true.into();
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return true.into();
    }
    let mut buf = vec![0u16; len as usize + 1];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    if title.is_empty() || title == "Program Manager" {
        return true.into();
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == GetCurrentProcessId() {
        return true.into();
    }
    out.push(Win { hwnd: hwnd.0 as isize, title, exe: String::new(), pid });
    true.into()
}

pub fn list() -> Vec<Win> {
    let mut v: Vec<Win> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut v as *mut _ as isize));
    }
    for w in v.iter_mut() {
        w.exe = exe_of(w.pid);
    }
    v.retain(|w| !w.exe.to_lowercase().ends_with("\\textinputhost.exe"));
    v
}

fn app_name(exe: &str) -> String {
    Path::new(exe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn search(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let mut out: Vec<(i32, Value)> = list()
        .into_iter()
        .filter_map(|w| {
            let name = app_name(&w.exe);
            let s = match (fuzzy::score(q, &w.title), fuzzy::score(q, &name)) {
                (Some(a), Some(b)) => a.max(b - 30),
                (Some(a), None) => a,
                (None, Some(b)) => b - 30,
                (None, None) => return None,
            };
            Some((s, item(&w, s)))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

pub fn item(w: &Win, score: i32) -> Value {
    let name = app_name(&w.exe);
    json!({
        "key": format!("win:{}", w.hwnd), "kind": "window", "title": w.title,
        "subtitle": format!("Switch to window · {name}"), "icon": if w.exe.is_empty() { Value::Null } else { json!(icons::path_url(&w.exe)) },
        "hwnd": w.hwnd, "path": w.exe, "score": score,
    })
}

/// Bring a window to the front (restoring it if minimized).
pub fn activate(hwnd: isize) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else {
            let _ = ShowWindow(h, SW_SHOW);
        }
        force_foreground(h);
    }
}

/// SetForegroundWindow, borrowing the foreground thread's input queue if Windows refuses.
pub fn force_foreground(h: HWND) {
    unsafe {
        if SetForegroundWindow(h).as_bool() && GetForegroundWindow() == h {
            return;
        }
        let fg = GetForegroundWindow();
        let fg_thread = GetWindowThreadProcessId(fg, None);
        let me = GetCurrentThreadId();
        if fg_thread != 0 && fg_thread != me {
            let _ = AttachThreadInput(me, fg_thread, true);
            let _ = BringWindowToTop(h);
            let _ = SetForegroundWindow(h);
            let _ = AttachThreadInput(me, fg_thread, false);
        } else {
            let _ = BringWindowToTop(h);
            let _ = SetForegroundWindow(h);
        }
    }
}

pub fn close(hwnd: isize) {
    unsafe {
        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

// ------------------------------------------------------------------------------------------
// processes

pub struct Proc {
    pub pid: u32,
    pub name: String,
    pub count: usize,
    pub pids: Vec<u32>,
}

pub fn processes() -> Vec<Proc> {
    let mut by_name: std::collections::BTreeMap<String, Proc> = Default::default();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return vec![] };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        let me = GetCurrentProcessId();
        while ok {
            let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            let name = String::from_utf16_lossy(&e.szExeFile[..end]);
            if e.th32ProcessID > 4 && e.th32ProcessID != me && !name.is_empty() {
                let p = by_name.entry(name.to_lowercase()).or_insert(Proc { pid: e.th32ProcessID, name: name.clone(), count: 0, pids: vec![] });
                p.count += 1;
                p.pids.push(e.th32ProcessID);
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    by_name.into_values().collect()
}

/// Processes that must never be offered for killing.
const PROTECTED: &[&str] = &[
    "csrss.exe", "wininit.exe", "winlogon.exe", "services.exe", "lsass.exe", "smss.exe", "svchost.exe", "system", "registry", "dwm.exe", "fontdrvhost.exe",
    "memory compression", "secure system", "lsaiso.exe", "glint.exe", "glint-indexer.exe",
];

pub fn search_processes(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let mut out: Vec<(i32, Value)> = processes()
        .into_iter()
        .filter(|p| !PROTECTED.contains(&p.name.to_lowercase().as_str()))
        .filter_map(|p| {
            let s = if q.trim().is_empty() { 100 } else { fuzzy::score(q, &p.name)? };
            let exe = exe_of(p.pid);
            Some((
                s,
                json!({
                    "key": format!("proc:{}", p.name.to_lowercase()), "kind": "process", "title": format!("Kill {}", p.name),
                    "subtitle": if p.count > 1 { format!("{} processes · Enter twice to end them", p.count) } else { format!("PID {} · Enter twice to end it", p.pid) },
                    "icon": if exe.is_empty() { Value::Null } else { json!(icons::path_url(&exe)) },
                    "pids": p.pids, "path": exe, "score": s, "confirm": true,
                }),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

pub fn kill(pids: &[u32]) -> usize {
    let mut n = 0;
    for &pid in pids {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                if TerminateProcess(h, 1).is_ok() {
                    n += 1;
                }
                let _ = CloseHandle(h);
            }
        }
    }
    n
}
