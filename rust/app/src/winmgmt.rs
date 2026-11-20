//! Move and size the window you were in before opening Glint:
//! halves, thirds, maximize, almost maximize, center, restore, minimize, next monitor.
use crate::{fuzzy, wins};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicIsize, Ordering};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowRect, IsIconic, IsWindow, IsZoomed, SetWindowPos, ShowWindow, SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_MINIMIZE, SW_RESTORE,
};

/// The window that was in front when Glint opened.
pub static PREV: AtomicIsize = AtomicIsize::new(0);

pub fn prev() -> Option<HWND> {
    let h = HWND(PREV.load(Ordering::SeqCst) as *mut _);
    (!h.is_invalid() && unsafe { IsWindow(Some(h)).as_bool() }).then_some(h)
}

/// (id, title, keywords)
const LAYOUTS: &[(&str, &str, &str)] = &[
    ("left", "Left half", "snap left half window"),
    ("right", "Right half", "snap right half window"),
    ("top", "Top half", "snap top half upper window"),
    ("bottom", "Bottom half", "snap bottom half lower window"),
    ("max", "Maximize", "maximize full fill window"),
    ("almost", "Almost maximize", "almost maximize large window"),
    ("center", "Center", "center middle window"),
    ("restore", "Restore", "restore unmaximize window"),
    ("min", "Minimize", "minimize hide window"),
    ("next", "Move to next monitor", "next monitor display screen move window"),
    ("third1", "Left third", "third left window"),
    ("third2", "Center third", "third center middle window"),
    ("third3", "Right third", "third right window"),
    ("twothirds1", "Left two thirds", "two thirds left window"),
    ("twothirds2", "Right two thirds", "two thirds right window"),
];

pub fn search(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let Some(h) = prev() else { return vec![] };
    let title = wins::title_of(h);
    if title.is_empty() {
        return vec![];
    }
    let ql = q.trim().to_lowercase();
    let mut out: Vec<(i32, Value)> = LAYOUTS
        .iter()
        .filter_map(|(id, name, words)| {
            let by_name = fuzzy::score(q, name);
            let by_words = (ql.len() >= 3 && words.split(' ').any(|w| w.starts_with(&ql))).then_some(600);
            let s = by_name.max(by_words)? - 80;
            Some((
                s,
                json!({"key": format!("layout:{id}"), "kind": "layout", "title": name, "subtitle": format!("Window · {title}"), "glyph": "layout", "op": id, "score": s}),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

fn work_area(m: HMONITOR) -> RECT {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    unsafe {
        let _ = GetMonitorInfoW(m, &mut mi);
    }
    mi.rcWork
}

/// The invisible resize borders around a window (Windows 10/11 draws them outside the visible frame).
fn insets(h: HWND) -> (i32, i32, i32, i32) {
    let (mut wr, mut fr) = (RECT::default(), RECT::default());
    unsafe {
        if GetWindowRect(h, &mut wr).is_err()
            || DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut fr as *mut _ as _, std::mem::size_of::<RECT>() as u32).is_err()
        {
            return (0, 0, 0, 0);
        }
    }
    (fr.left - wr.left, fr.top - wr.top, wr.right - fr.right, wr.bottom - fr.bottom)
}

fn place(h: HWND, x: i32, y: i32, w: i32, hgt: i32) {
    unsafe {
        if IsZoomed(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let (l, t, r, b) = insets(h);
        let _ = SetWindowPos(h, None, x - l, y - t, w + l + r, hgt + t + b, SWP_NOZORDER | SWP_NOACTIVATE);
    }
}

unsafe extern "system" fn collect(m: HMONITOR, _: HDC, _: *mut RECT, lp: LPARAM) -> BOOL {
    let v = &mut *(lp.0 as *mut Vec<HMONITOR>);
    v.push(m);
    true.into()
}

pub fn apply(op: &str) -> Result<(), String> {
    let h = prev().ok_or("No window to move: open Glint while the window you want is in front")?;
    let mon = unsafe { MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) };
    let wa = work_area(mon);
    let (x, y, w, hh) = (wa.left, wa.top, wa.right - wa.left, wa.bottom - wa.top);
    match op {
        "left" => place(h, x, y, w / 2, hh),
        "right" => place(h, x + w / 2, y, w - w / 2, hh),
        "top" => place(h, x, y, w, hh / 2),
        "bottom" => place(h, x, y + hh / 2, w, hh - hh / 2),
        "third1" => place(h, x, y, w / 3, hh),
        "third2" => place(h, x + w / 3, y, w / 3, hh),
        "third3" => place(h, x + 2 * w / 3, y, w - 2 * w / 3, hh),
        "twothirds1" => place(h, x, y, 2 * w / 3, hh),
        "twothirds2" => place(h, x + w / 3, y, w - w / 3, hh),
        "almost" => place(h, x + w / 20, y + hh / 20, w * 9 / 10, hh * 9 / 10),
        "center" => {
            let mut fr = RECT::default();
            unsafe {
                let _ = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut fr as *mut _ as _, std::mem::size_of::<RECT>() as u32);
            }
            let (cw, ch) = ((fr.right - fr.left).min(w), (fr.bottom - fr.top).min(hh));
            place(h, x + (w - cw) / 2, y + (hh - ch) / 2, cw, ch);
        }
        "max" => unsafe {
            let _ = ShowWindow(h, SW_MAXIMIZE);
        },
        "restore" => unsafe {
            let _ = ShowWindow(h, SW_RESTORE);
        },
        "min" => unsafe {
            let _ = ShowWindow(h, SW_MINIMIZE);
            return Ok(());
        },
        "next" => {
            let mut mons: Vec<HMONITOR> = Vec::new();
            unsafe {
                let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut mons as *mut _ as isize));
            }
            if mons.len() < 2 {
                return Err("There's only one monitor".into());
            }
            mons.sort_by_key(|m| {
                let r = work_area(*m);
                (r.left, r.top)
            });
            let i = mons.iter().position(|m| *m == mon).unwrap_or(0);
            let to = work_area(mons[(i + 1) % mons.len()]);
            let was_max = unsafe { IsZoomed(h).as_bool() };
            // keep the same relative position and size
            let mut fr = RECT::default();
            unsafe {
                let _ = DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut fr as *mut _ as _, std::mem::size_of::<RECT>() as u32);
            }
            let (tw, th) = (to.right - to.left, to.bottom - to.top);
            let fx = |v: i32| (v - x) as f64 / w.max(1) as f64;
            let fy = |v: i32| (v - y) as f64 / hh.max(1) as f64;
            let nx = to.left + (fx(fr.left) * tw as f64) as i32;
            let ny = to.top + (fy(fr.top) * th as f64) as i32;
            let nw = ((fr.right - fr.left) as f64 / w.max(1) as f64 * tw as f64) as i32;
            let nh = ((fr.bottom - fr.top) as f64 / hh.max(1) as f64 * th as f64) as i32;
            place(h, nx, ny, nw.min(tw), nh.min(th));
            if was_max {
                unsafe {
                    let _ = ShowWindow(h, SW_MAXIMIZE);
                }
            }
        }
        _ => return Err("Unknown layout".into()),
    }
    wins::force_foreground(h);
    Ok(())
}
