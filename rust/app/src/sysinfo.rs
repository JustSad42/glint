//! Quick toggles and system facts typed straight into the box:
//!   vol 30 · vol +10 · mute · unmute · wifi on/off · bluetooth on/off
//!   ip · battery · uptime · ram · disk
use serde_json::{json, Value};
use windows::core::PCWSTR;
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetLogicalDrives};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::{GetTickCount64, GlobalMemoryStatusEx, MEMORYSTATUSEX};

fn com() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn endpoint() -> Option<IAudioEndpointVolume> {
    com();
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

/// (volume 0..=100, muted)
pub fn volume() -> Option<(u32, bool)> {
    let ep = endpoint()?;
    unsafe { Some(((ep.GetMasterVolumeLevelScalar().ok()? * 100.0).round() as u32, ep.GetMute().ok()?.as_bool())) }
}

pub fn set_volume(pct: u32) -> Result<String, String> {
    let ep = endpoint().ok_or("No audio output found")?;
    let pct = pct.min(100);
    unsafe {
        ep.SetMasterVolumeLevelScalar(pct as f32 / 100.0, std::ptr::null()).map_err(|e| e.to_string())?;
        if pct > 0 {
            let _ = ep.SetMute(false, std::ptr::null());
        }
    }
    Ok(format!("Volume {pct}%"))
}

pub fn set_mute(on: bool) -> Result<String, String> {
    let ep = endpoint().ok_or("No audio output found")?;
    unsafe { ep.SetMute(on, std::ptr::null()).map_err(|e| e.to_string())? };
    Ok(if on { "Muted".into() } else { "Sound on".into() })
}

fn radio(kind: RadioKind) -> Option<Radio> {
    let _ = Radio::RequestAccessAsync().ok()?.get();
    let all = Radio::GetRadiosAsync().ok()?.get().ok()?;
    (0..all.Size().ok()?).filter_map(|i| all.GetAt(i).ok()).find(|r| r.Kind().ok() == Some(kind))
}

fn radio_kind(name: &str) -> RadioKind {
    if name == "bluetooth" {
        RadioKind::Bluetooth
    } else {
        RadioKind::WiFi
    }
}

pub fn radio_state(name: &str) -> Option<bool> {
    radio(radio_kind(name)).and_then(|r| r.State().ok()).map(|s| s == RadioState::On)
}

pub fn set_radio(name: &str, on: bool) -> Result<String, String> {
    let r = radio(radio_kind(name)).ok_or_else(|| format!("No {} adapter found", label(name)))?;
    let st = r.SetStateAsync(if on { RadioState::On } else { RadioState::Off }).map_err(|e| e.to_string())?.get().map_err(|e| e.to_string())?;
    if st.0 != 1 {
        // 1 = Allowed
        return Err(format!("Windows didn't allow changing {} (turn it on in Settings first, or check airplane mode)", label(name)));
    }
    Ok(format!("{} {}", label(name), if on { "on" } else { "off" }))
}

fn label(name: &str) -> &'static str {
    if name == "bluetooth" {
        "Bluetooth"
    } else {
        "Wi-Fi"
    }
}

/// The address this PC uses on the local network (no packet is sent).
pub fn local_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    Some(s.local_addr().ok()?.ip().to_string())
}

/// Ask api.ipify.org (only when the user presses Enter on the row).
pub fn public_ip() -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(5))).build().into();
    let mut r = agent.get("https://api.ipify.org").call().map_err(|e| format!("Couldn't look it up: {e}"))?;
    let ip = r.body_mut().read_to_string().map_err(|e| e.to_string())?;
    Ok(ip.trim().to_string())
}

fn fmt_dur(secs: u64) -> String {
    let (d, h, m) = (secs / 86400, secs % 86400 / 3600, secs % 3600 / 60);
    if d > 0 {
        format!("{d} d {h} h {m} min")
    } else if h > 0 {
        format!("{h} h {m} min")
    } else {
        format!("{m} min")
    }
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1_073_741_824.0)
}

fn info(key: &str, title: String, subtitle: String, glyph: &str, copy: &str, score: i32) -> Value {
    json!({"key": format!("info:{key}"), "kind": "info", "title": title, "subtitle": subtitle, "glyph": glyph, "copy": copy, "score": score})
}

fn toggle(key: &str, title: String, subtitle: String, glyph: &str, op: &str, score: i32) -> Value {
    json!({"key": format!("toggle:{key}"), "kind": "toggle", "title": title, "subtitle": subtitle, "glyph": glyph, "op": op, "score": score})
}

/// Rows for what was typed, best first. Empty if it isn't one of these.
pub fn rows(q: &str) -> Vec<Value> {
    let q = q.trim().to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let first = words.first().copied().unwrap_or("");
    let rest = words.get(1).copied().unwrap_or("");
    let mut out = Vec::new();
    match first {
        "vol" | "volume" | "sound" => {
            let cur = volume();
            let now = cur.map(|(v, m)| if m { format!("now {v}%, muted") } else { format!("now {v}%") }).unwrap_or_default();
            if let Ok(n) = rest.trim_end_matches('%').parse::<i64>() {
                let base = cur.map(|c| c.0 as i64).unwrap_or(50);
                let target = if rest.starts_with('+') || rest.starts_with('-') { base + n } else { n }.clamp(0, 100);
                out.push(toggle("vol", format!("Set volume to {target}%"), format!("{now} · Enter sets it"), "volume", &format!("vol:{target}"), 5000));
            } else if rest.is_empty() {
                if let Some((v, m)) = cur {
                    out.push(toggle("vol", format!("Volume {v}%{}", if m { " · muted" } else { "" }), "Type a number: vol 30, vol +10, vol -10".into(), "volume", if m { "unmute" } else { "mute" }, 3000));
                    out.last_mut().unwrap()["subtitle"] = json!(format!("Enter {} · type vol 30, vol +10, vol -10", if m { "unmutes" } else { "mutes" }));
                }
            }
        }
        "mute" if rest.is_empty() => out.push(toggle("mute", "Mute sound".into(), "All output on the default device".into(), "volume", "mute", 5000)),
        "unmute" if rest.is_empty() => out.push(toggle("unmute", "Unmute sound".into(), "".into(), "volume", "unmute", 5000)),
        "wifi" | "wi-fi" | "wlan" | "bluetooth" | "bt" => {
            let name = if first == "bluetooth" || first == "bt" { "bluetooth" } else { "wifi" };
            let state = radio_state(name);
            let want = match rest {
                "on" => Some(true),
                "off" => Some(false),
                _ => state.map(|s| !s),
            };
            if state.is_none() && rest.is_empty() {
                out.push(info(name, format!("No {} radio found", label(name)), "Windows doesn't report one on this PC (or it's turned off in Device Manager)".into(), if name == "bluetooth" { "bluetooth" } else { "wifi" }, "", 900));
            } else if let Some(on) = want {
                let now = state.map(|s| if s { "now on" } else { "now off" }).unwrap_or("");
                out.push(toggle(name, format!("Turn {} {}", label(name), if on { "on" } else { "off" }), now.into(), if name == "bluetooth" { "bluetooth" } else { "wifi" }, &format!("{name}:{}", if on { "on" } else { "off" }), if rest.is_empty() { 900 } else { 5000 }));
            }
        }
        "ip" | "myip" => {
            if let Some(ip) = local_ip() {
                out.push(info("lip", ip.clone(), "Local IP address · Enter copies".into(), "globe", &ip, 5000));
            }
            out.push(json!({"key": "info:pip", "kind": "info", "title": "Public IP", "subtitle": "Enter looks it up (asks api.ipify.org) and copies it", "glyph": "globe", "op": "public_ip", "score": 4900}));
        }
        "battery" | "power" => {
            let mut s = SYSTEM_POWER_STATUS::default();
            if unsafe { GetSystemPowerStatus(&mut s) }.is_ok() {
                if s.BatteryFlag & 128 != 0 || s.BatteryLifePercent == 255 {
                    out.push(info("bat", "No battery".into(), if s.ACLineStatus == 1 { "Running on mains power".into() } else { "".into() }, "battery", "", 900));
                } else {
                    let left = if s.BatteryLifeTime != u32::MAX && s.ACLineStatus != 1 { format!(" · {} left", fmt_dur(s.BatteryLifeTime as u64)) } else { String::new() };
                    let charging = if s.ACLineStatus == 1 { " · plugged in" } else { "" };
                    out.push(info("bat", format!("Battery {}%", s.BatteryLifePercent), format!("{charging}{left}").trim_start_matches(" · ").to_string(), "battery", &format!("{}%", s.BatteryLifePercent), 3000));
                }
            }
        }
        "uptime" => {
            let up = fmt_dur(unsafe { GetTickCount64() } / 1000);
            out.push(info("up", format!("Up {up}"), "Since Windows last started · Enter copies".into(), "clock", &up, 5000));
        }
        "ram" | "memory" | "mem" => {
            let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
                let used = m.ullTotalPhys - m.ullAvailPhys;
                out.push(info("ram", format!("Memory {}% used", m.dwMemoryLoad), format!("{} of {} in use · {} free", gb(used), gb(m.ullTotalPhys), gb(m.ullAvailPhys)), "cpu", &format!("{}%", m.dwMemoryLoad), 3000));
            }
        }
        "disk" | "disks" | "space" | "drives" => {
            let mask = unsafe { GetLogicalDrives() };
            for i in 0..26u32 {
                if mask & (1 << i) == 0 {
                    continue;
                }
                let root: Vec<u16> = format!("{}:\\", (b'A' + i as u8) as char).encode_utf16().chain(Some(0)).collect();
                let (mut free, mut total) = (0u64, 0u64);
                if unsafe { GetDiskFreeSpaceExW(PCWSTR(root.as_ptr()), Some(&mut free), Some(&mut total), None) }.is_ok() && total > 0 {
                    let pct = 100 - free * 100 / total;
                    let letter = (b'A' + i as u8) as char;
                    out.push(info(&format!("disk{letter}"), format!("{letter}:  {} free of {}", gb(free), gb(total)), format!("{pct}% used"), "hdd", &gb(free), 3000 - i as i32));
                }
            }
        }
        _ => {}
    }
    out
}

/// Run a toggle row's `op`.
pub fn run(op: &str) -> Result<String, String> {
    if op.starts_with("app:") || op.starts_with("dev:") || op.is_empty() {
        return crate::audio::run(op);
    }
    if let Some(v) = op.strip_prefix("vol:") {
        return set_volume(v.parse().map_err(|_| "bad volume")?);
    }
    match op {
        "mute" => set_mute(true),
        "unmute" => set_mute(false),
        "wifi:on" => set_radio("wifi", true),
        "wifi:off" => set_radio("wifi", false),
        "bluetooth:on" => set_radio("bluetooth", true),
        "bluetooth:off" => set_radio("bluetooth", false),
        _ => Err("Unknown toggle".into()),
    }
}
