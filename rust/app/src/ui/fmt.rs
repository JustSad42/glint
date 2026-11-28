//! Number and date formatting (as the old UI showed them, en-GB style: 16:11, Mon 23 Nov).
use chrono::{Datelike, Local, TimeZone};

pub fn size(n: f64) -> String {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let (mut n, mut i) = (n, 0);
    while n >= 1024.0 && i < u.len() - 1 {
        n /= 1024.0;
        i += 1;
    }
    if n < 10.0 && i > 0 {
        format!("{:.1} {}", n, u[i])
    } else {
        format!("{} {}", n.round() as i64, u[i])
    }
}

pub fn count(n: f64) -> String {
    if n >= 1e6 {
        if n >= 1e7 { format!("{:.0}M", n / 1e6) } else { format!("{:.1}M", n / 1e6) }
    } else if n >= 1e3 {
        if n >= 1e4 { format!("{:.0}K", n / 1e3) } else { format!("{:.1}K", n / 1e3) }
    } else {
        format!("{}", n as i64)
    }
}

fn local(secs: f64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_opt(secs as i64, 0).single()
}

/// "Just now", "5 min ago", "Today, 11:11", "Yesterday, 09:02", "23 Nov 2026"
pub fn date(secs: f64) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let Some(d) = local(secs) else { return String::new() };
    let now = Local::now();
    let diff = (now.timestamp_millis() as f64 / 1000.0) - secs;
    if diff < 60.0 {
        return "Just now".into();
    }
    if diff < 3600.0 {
        return format!("{} min ago", (diff / 60.0).floor() as i64);
    }
    if diff < 86400.0 && d.day() == now.day() {
        return format!("Today, {}", d.format("%H:%M"));
    }
    if diff < 172800.0 {
        return format!("Yesterday, {}", d.format("%H:%M"));
    }
    d.format("%-d %b %Y").to_string()
}

/// File rows: "11:11" today, "24 Oct" this year, "Nov 2025" before.
pub fn short_date(secs: f64) -> String {
    if secs <= 0.0 {
        return String::new();
    }
    let Some(d) = local(secs) else { return String::new() };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if d.year() == now.year() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%b %Y").to_string()
    }
}

pub fn clock_time() -> String {
    Local::now().format("%H:%M").to_string()
}

pub fn clock_date_short() -> String {
    Local::now().format("%a %-d %b").to_string()
}

pub fn clock_date_long() -> String {
    Local::now().format("%A %-d %B %Y").to_string()
}

/// Timer countdown: "4:05" or "1:02:03".
pub fn countdown(due: f64) -> String {
    let now = Local::now().timestamp_millis() as f64 / 1000.0;
    let s = (due - now).round().max(0.0) as i64;
    let (h, m, x) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{x:02}") } else { format!("{m}:{x:02}") }
}
