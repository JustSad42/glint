//! Per-app volume and the output device:
//!   vol spotify 30 · mute discord · unmute discord · vol chrome   (apps playing sound now)
//!   audio · audio headphones · output speakers                    (switch the default output)
#![allow(non_snake_case)]
use crate::fuzzy;
use serde_json::{json, Value};
use std::ffi::c_void;
use windows::core::{Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, IAudioSessionControl2, IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, ISimpleAudioVolume,
    MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

fn com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    com();
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() }
}

#[allow(dead_code)]
pub struct Session {
    pub pid: u32,
    pub name: String,
    pub exe: String,
    pub volume: u32,
    pub muted: bool,
    vol: ISimpleAudioVolume,
}

/// Helper processes that play sound for the app that started them.
const HELPERS: &[&str] = &["qtwebengineprocess", "msedgewebview2", "cefsharp.browsersubprocess", "crashpad_handler", "audiodg"];

fn parent_pid(pid: u32) -> Option<u32> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).ok()?;
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        let mut found = None;
        while ok {
            if e.th32ProcessID == pid {
                found = Some(e.th32ParentProcessID);
                break;
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
        found
    }
}

fn stem_of(exe: &str) -> String {
    std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The program behind an audio session: a helper process is named after its parent app.
fn owner_exe(pid: u32) -> String {
    let exe = crate::wins::exe_of(pid);
    let stem = stem_of(&exe).to_lowercase();
    if HELPERS.contains(&stem.as_str()) || stem.ends_with(" helper") || stem.ends_with("helper") {
        if let Some(pp) = parent_pid(pid) {
            let pe = crate::wins::exe_of(pp);
            if !pe.is_empty() {
                return pe;
            }
        }
    }
    exe
}

/// "Google Chrome" from the program's description, else its file name.
fn friendly(exe: &str) -> String {
    if let Some(d) = crate::preview::version_info(exe).and_then(|v| v["description"].as_str().map(String::from)) {
        let d = d.trim().to_string();
        if !d.is_empty() && d.len() <= 32 {
            return d;
        }
    }
    let stem = std::path::Path::new(exe).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut c = stem.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => stem,
    }
}

/// Apps with an audio session on the default output, one per program.
pub fn sessions() -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    let Some(en) = enumerator() else { return out };
    unsafe {
        let Ok(dev) = en.GetDefaultAudioEndpoint(eRender, eConsole) else { return out };
        let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return out };
        let Ok(list) = mgr.GetSessionEnumerator() else { return out };
        let n = list.GetCount().unwrap_or(0);
        for i in 0..n {
            let Ok(ctl) = list.GetSession(i) else { continue };
            let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let pid = c2.GetProcessId().unwrap_or(0);
            if pid == 0 {
                continue; // system sounds
            }
            let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else { continue };
            let exe = owner_exe(pid);
            if exe.is_empty() {
                continue;
            }
            let name = friendly(&exe);
            if out.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
                // several sessions of one program (Chrome, Discord): keep the first; set_app does all
                continue;
            }
            let volume = (vol.GetMasterVolume().unwrap_or(1.0) * 100.0).round() as u32;
            let muted = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            out.push(Session { pid, name, exe, volume, muted, vol });
        }
    }
    out
}

/// Set volume (Some) and/or mute (Some) for every session of the program called `name`.
fn set_app(name: &str, volume: Option<u32>, mute: Option<bool>) -> Result<String, String> {
    let Some(en) = enumerator() else { return Err("No audio output found".into()) };
    let mut n = 0;
    unsafe {
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| e.to_string())?;
        let mgr = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let list = mgr.GetSessionEnumerator().map_err(|e| e.to_string())?;
        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(ctl) = list.GetSession(i) else { continue };
            let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let pid = c2.GetProcessId().unwrap_or(0);
            if pid == 0 || !friendly(&owner_exe(pid)).eq_ignore_ascii_case(name) {
                continue;
            }
            let Ok(vol) = ctl.cast::<ISimpleAudioVolume>() else { continue };
            if let Some(v) = volume {
                let _ = vol.SetMasterVolume(v.min(100) as f32 / 100.0, std::ptr::null());
                if v > 0 {
                    let _ = vol.SetMute(false, std::ptr::null());
                }
            }
            if let Some(m) = mute {
                let _ = vol.SetMute(m, std::ptr::null());
            }
            n += 1;
        }
    }
    if n == 0 {
        return Err(format!("{name} isn't playing sound right now"));
    }
    Ok(match (volume, mute) {
        (Some(v), _) => format!("{name} at {v}%"),
        (_, Some(true)) => format!("{name} muted"),
        _ => format!("{name} unmuted"),
    })
}

pub struct Device {
    pub id: String,
    pub name: String,
    pub default: bool,
}

fn dev_id(d: &IMMDevice) -> Option<String> {
    unsafe {
        let p = d.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        s
    }
}

pub fn devices() -> Vec<Device> {
    let mut out = Vec::new();
    let Some(en) = enumerator() else { return out };
    unsafe {
        let def = en.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| dev_id(&d)).unwrap_or_default();
        let Ok(col) = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..col.GetCount().unwrap_or(0) {
            let Ok(d) = col.Item(i) else { continue };
            let Some(id) = dev_id(&d) else { continue };
            let name = d
                .OpenPropertyStore(STGM_READ)
                .ok()
                .and_then(|ps| ps.GetValue(&PKEY_Device_FriendlyName).ok())
                .map(|v| v.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "Audio device".into());
            out.push(Device { default: id == def, id, name });
        }
    }
    out
}

// The interface Windows' own Sound settings use to change the default device. It isn't in the
// SDK, but has been stable since Windows 7.
#[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows::core::IUnknown {
    fn GetMixFormat(&self, id: PCWSTR, f: *mut *mut c_void) -> HRESULT;
    fn GetDeviceFormat(&self, id: PCWSTR, d: i32, f: *mut *mut c_void) -> HRESULT;
    fn ResetDeviceFormat(&self, id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(&self, id: PCWSTR, a: *mut c_void, b: *mut c_void) -> HRESULT;
    fn GetProcessingPeriod(&self, id: PCWSTR, d: i32, a: *mut i64, b: *mut i64) -> HRESULT;
    fn SetProcessingPeriod(&self, id: PCWSTR, p: *mut i64) -> HRESULT;
    fn GetShareMode(&self, id: PCWSTR, m: *mut c_void) -> HRESULT;
    fn SetShareMode(&self, id: PCWSTR, m: *mut c_void) -> HRESULT;
    fn GetPropertyValue(&self, id: PCWSTR, k: *const c_void, v: *mut c_void) -> HRESULT;
    fn SetPropertyValue(&self, id: PCWSTR, k: *const c_void, v: *const c_void) -> HRESULT;
    fn SetDefaultEndpoint(&self, id: PCWSTR, role: i32) -> HRESULT;
    fn SetEndpointVisibility(&self, id: PCWSTR, visible: i32) -> HRESULT;
}

const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

fn set_default_device(id: &str) -> Result<String, String> {
    com();
    let name = devices().into_iter().find(|d| d.id == id).map(|d| d.name).unwrap_or_else(|| "device".into());
    let w: Vec<u16> = id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let pc: IPolicyConfig = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        for role in [eConsole, eMultimedia, eCommunications] {
            pc.SetDefaultEndpoint(PCWSTR(w.as_ptr()), role.0).ok().map_err(|e| e.to_string())?;
        }
    }
    Ok(format!("Sound now plays on {name}"))
}

fn row(key: &str, title: String, subtitle: String, glyph: &str, op: String, score: i32) -> Value {
    json!({"key": format!("toggle:{key}"), "kind": "toggle", "title": title, "subtitle": subtitle, "glyph": glyph, "op": op, "score": score})
}

pub fn rows(q: &str) -> Vec<Value> {
    let q = q.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let Some(&first) = words.first() else { return vec![] };
    let mut out = Vec::new();
    match first {
        "vol" | "volume" | "mute" | "unmute" if words.len() >= 2 => {
            // the last word may be a number: vol spotify 30
            let num = words.last().and_then(|w| w.trim_end_matches('%').parse::<i64>().ok());
            let name_words = if num.is_some() { &words[1..words.len() - 1] } else { &words[1..] };
            let who = name_words.join(" ");
            if who.is_empty() || who.parse::<i64>().is_ok() {
                return out;
            }
            let all = sessions();
            for s in &all {
                let Some(score) = fuzzy::score(&who, &s.name).max(fuzzy::score(&who, &stem_of(&s.exe)).map(|x| x - 20)) else { continue };
                let now = format!("{} now {}%{}", s.name, s.volume, if s.muted { ", muted" } else { "" });
                let (title, op) = match (first, num) {
                    ("mute", _) => (format!("Mute {}", s.name), format!("app:{}:mute", s.name)),
                    ("unmute", _) => (format!("Unmute {}", s.name), format!("app:{}:unmute", s.name)),
                    (_, Some(n)) => {
                        let rel = words.last().map(|w| w.starts_with('+') || w.starts_with('-')).unwrap_or(false);
                        let t = if rel { s.volume as i64 + n } else { n }.clamp(0, 100);
                        (format!("Set {} to {t}%", s.name), format!("app:{}:{t}", s.name))
                    }
                    _ => (format!("{} volume {}%", s.name, s.volume), format!("app:{}:{}", s.name, if s.muted { "unmute" } else { "mute" })),
                };
                let sub = if num.is_none() && first != "mute" && first != "unmute" {
                    format!("Enter {} it · type a number to set it: vol {} 40", if s.muted { "unmutes" } else { "mutes" }, s.name.to_lowercase())
                } else {
                    now
                };
                let mut r = row(&format!("app-{}", s.name.to_lowercase()), title, sub, "volume", op, 5200 + score.min(500));
                r["icon"] = json!(crate::icons::path_url(&s.exe));
                out.push(r);
            }
            if out.is_empty() && who.chars().count() >= 3 && !who.chars().all(|c| c.is_ascii_digit()) {
                let playing: Vec<String> = all.iter().map(|s| s.name.clone()).collect();
                out.push(json!({"key": "toggle:app-none", "kind": "hint", "title": format!("Nothing called “{who}” is playing sound"),
                    "subtitle": if playing.is_empty() { "No app is playing sound right now".to_string() } else { format!("Playing now: {}", playing.join(", ")) },
                    "glyph": "volume", "score": 4500}));
            }
        }
        "audio" | "output" | "speakers" | "headphones" | "headset" | "playback" => {
            let filter = if first == "audio" || first == "output" || first == "playback" { words[1..].join(" ") } else { q.clone() };
            for d in devices() {
                let score = if filter.is_empty() { Some(100) } else { fuzzy::score(&filter, &d.name).or_else(|| (first == "headphones" || first == "headset" || first == "speakers").then_some(10)) };
                let Some(score) = score else { continue };
                out.push(row(
                    &format!("dev-{}", d.name.to_lowercase()),
                    if d.default { format!("{} (playing now)", d.name) } else { format!("Play sound on {}", d.name) },
                    if d.default { "The current output device".into() } else { "Switch the default output device".into() },
                    if d.name.to_lowercase().contains("head") { "headphones" } else { "volume" },
                    if d.default { String::new() } else { format!("dev:{}", d.id) },
                    5100 + score.min(500) - if d.default { 300 } else { 0 },
                ));
            }
        }
        _ => {}
    }
    out
}

/// Run an `app:` or `dev:` op.
pub fn run(op: &str) -> Result<String, String> {
    if let Some(id) = op.strip_prefix("dev:") {
        return set_default_device(id);
    }
    if let Some(rest) = op.strip_prefix("app:") {
        let (name, what) = rest.rsplit_once(':').ok_or("bad op")?;
        return match what {
            "mute" => set_app(name, None, Some(true)),
            "unmute" => set_app(name, None, Some(false)),
            n => set_app(name, Some(n.parse().map_err(|_| "bad volume")?), None),
        };
    }
    if op.is_empty() {
        return Ok("Already playing here".into());
    }
    Err("Unknown audio action".into())
}

#[allow(dead_code)]
fn session_count() -> usize {
    sessions().len()
}
