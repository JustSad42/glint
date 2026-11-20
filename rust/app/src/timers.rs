//! Timers and reminders:
//!   timer 10m tea · timer 1h30m · timer 90s · timer 25      (minutes when no unit)
//!   remind 3pm call mum · remind 15:30 standup · remind in 20m stretch · remind tomorrow 9am bins
//!   timers                                                  (what's running; Enter cancels)
//! They survive restarts (timers.json). When one is due, `on_fire` is called (it shows Glint's
//! notice popup) and a sound plays.
use crate::settings;
use chrono::{Datelike, Duration as CDuration, Local, NaiveTime, TimeZone};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Timer {
    pub id: u64,
    /// unix seconds
    pub due: i64,
    pub label: String,
    /// "timer" or "reminder"
    pub kind: String,
    /// for timers, how long it was set for (for the notice text)
    #[serde(default)]
    pub secs: i64,
}

static LIST: Lazy<Mutex<Vec<Timer>>> = Lazy::new(|| Mutex::new(load()));
static ON_FIRE: OnceCell<Box<dyn Fn(Timer) + Send + Sync>> = OnceCell::new();

fn file() -> std::path::PathBuf {
    settings::data_file("timers.json")
}

fn load() -> Vec<Timer> {
    std::fs::read_to_string(file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(list: &[Timer]) {
    let _ = std::fs::write(file(), serde_json::to_string_pretty(list).unwrap_or_default());
}

fn now() -> i64 {
    Local::now().timestamp()
}

/// Start the clock thread. Timers that came due while Glint wasn't running fire at once.
pub fn start(on_fire: impl Fn(Timer) + Send + Sync + 'static) {
    if ON_FIRE.set(Box::new(on_fire)).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("timers".into())
        .spawn(|| loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let due: Vec<Timer> = {
                let mut l = LIST.lock();
                let t = now();
                let (due, keep): (Vec<Timer>, Vec<Timer>) = l.drain(..).partition(|x| x.due <= t);
                *l = keep;
                if !due.is_empty() {
                    save(&l);
                }
                due
            };
            for t in due {
                chime();
                if let Some(f) = ON_FIRE.get() {
                    f(t);
                }
            }
        })
        .ok();
}

fn chime() {
    use windows::core::w;
    use windows::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_NODEFAULT};
    unsafe {
        let _ = PlaySoundW(w!("Notification.Reminder"), None, SND_ALIAS | SND_ASYNC | SND_NODEFAULT);
    }
}

pub fn list() -> Vec<Timer> {
    let mut l = LIST.lock().clone();
    l.sort_by_key(|t| t.due);
    l
}

pub fn add(due: i64, label: &str, kind: &str, secs: i64) -> Timer {
    let mut l = LIST.lock();
    let id = l.iter().map(|t| t.id).max().unwrap_or(0).max(now() as u64) + 1;
    let t = Timer { id, due, label: label.trim().to_string(), kind: kind.into(), secs };
    l.push(t.clone());
    save(&l);
    t
}

pub fn cancel(id: u64) -> bool {
    let mut l = LIST.lock();
    let n = l.len();
    l.retain(|t| t.id != id);
    save(&l);
    l.len() != n
}

/// "10m", "1h30m", "90s", "2h", "25" (minutes), "1.5h" -> seconds
pub fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return None;
    }
    if let Ok(n) = s.parse::<f64>() {
        return (n > 0.0).then_some((n * 60.0) as i64);
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut any = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            i += 1;
            continue;
        }
        let mut unit = String::new();
        while i < chars.len() && chars[i].is_alphabetic() {
            unit.push(chars[i]);
            i += 1;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        let mult = match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1.0,
            "m" | "min" | "mins" | "minute" | "minutes" => 60.0,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600.0,
            "d" | "day" | "days" => 86400.0,
            _ => return None,
        };
        total += n * mult;
        any = true;
        if unit.is_empty() {
            i += 1;
        }
    }
    if !num.is_empty() {
        return None;
    }
    (any && total > 0.0).then_some(total as i64)
}

/// "3pm", "3:30pm", "15:30", "9" (with am/pm or 24h) -> time of day
fn parse_clock(s: &str) -> Option<NaiveTime> {
    let s = s.trim().to_lowercase().replace('.', ":");
    let (body, pm, am) = if let Some(b) = s.strip_suffix("pm") {
        (b.trim().to_string(), true, false)
    } else if let Some(b) = s.strip_suffix("am") {
        (b.trim().to_string(), false, true)
    } else {
        (s.clone(), false, false)
    };
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if !(am || pm) && !body.contains(':') {
        return None; // a bare number is a duration, not a time
    }
    let h = match (h, am, pm) {
        (12, true, _) => 0,
        (12, _, true) => 12,
        (h, _, true) => h + 12,
        (h, _, _) => h,
    };
    NaiveTime::from_hms_opt(h, m, 0)
}

/// A reminder: "3pm call", "in 20m stretch", "tomorrow 9am bins", "15:30 standup".
/// Returns (due unix seconds, label).
pub fn parse_reminder(rest: &str) -> Option<(i64, String)> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    let now_l = Local::now();
    let mut i = 0;
    let mut day = 0i64;
    if words[0].eq_ignore_ascii_case("in") {
        let d = parse_duration(words.get(1)?)?;
        return Some((now_l.timestamp() + d, words[2..].join(" ")));
    }
    if words[0].eq_ignore_ascii_case("tomorrow") {
        day = 1;
        i = 1;
    } else if words[0].eq_ignore_ascii_case("today") {
        i = 1;
    }
    if words[i..].first().map(|w| w.eq_ignore_ascii_case("at")).unwrap_or(false) {
        i += 1;
    }
    // "3 pm" as two words
    let (time, used) = match (words.get(i), words.get(i + 1)) {
        (Some(a), Some(b)) if b.eq_ignore_ascii_case("pm") || b.eq_ignore_ascii_case("am") => (parse_clock(&format!("{a}{b}"))?, 2),
        (Some(a), _) => match parse_clock(a) {
            Some(t) => (t, 1),
            None => {
                // "in" is optional for durations: "remind 20m stretch"
                let d = parse_duration(a)?;
                return Some((now_l.timestamp() + d, words[i + 1..].join(" ")));
            }
        },
        _ => return None,
    };
    let date = now_l.date_naive() + CDuration::days(day);
    let mut due = Local.from_local_datetime(&date.and_time(time)).single()?.timestamp();
    if due <= now_l.timestamp() && day == 0 {
        due += 86400; // that time has passed today: tomorrow
    }
    let _ = now_l.weekday();
    Some((due, words[i + used..].join(" ")))
}

fn fmt_dur(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    match (h, m, s) {
        (0, 0, s) => format!("{s} s"),
        (0, m, 0) => format!("{m} min"),
        (0, m, s) => format!("{m} min {s} s"),
        (h, 0, _) => format!("{h} h"),
        (h, m, _) => format!("{h} h {m} min"),
    }
}

fn fmt_when(due: i64) -> String {
    let d = Local.timestamp_opt(due, 0).single().unwrap_or_else(Local::now);
    let today = Local::now().date_naive();
    let t = d.format("%H:%M").to_string();
    if d.date_naive() == today {
        format!("today {t}")
    } else if d.date_naive() == today + CDuration::days(1) {
        format!("tomorrow {t}")
    } else {
        d.format("%a %d %b %H:%M").to_string()
    }
}

pub fn describe(t: &Timer) -> String {
    if t.kind == "timer" {
        format!("{} timer", fmt_dur(t.secs))
    } else {
        format!("Reminder · {}", fmt_when(t.due))
    }
}

fn row(key: &str, title: String, subtitle: String, extra: Value, score: i32) -> Value {
    let mut v = json!({"key": key, "kind": "timer", "title": title, "subtitle": subtitle, "glyph": "clock", "score": score});
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v
}

/// Running timers as rows (for the empty box and "timers").
pub fn active_rows() -> Vec<Value> {
    list()
        .into_iter()
        .map(|t| {
            let title = if t.label.is_empty() { describe(&t) } else { t.label.clone() };
            row(&format!("timer:{}", t.id), title, format!("{} · ends {} · Enter cancels", describe(&t), fmt_when(t.due)), json!({"op": "cancel", "id": t.id, "due": t.due}), 3000)
        })
        .collect()
}

pub fn rows(q: &str) -> Vec<Value> {
    let q = q.trim();
    let lower = q.to_lowercase();
    let (cmd, rest) = lower.split_once(' ').map(|(a, _)| (a.to_string(), q[a.len()..].trim().to_string())).unwrap_or((lower.clone(), String::new()));
    match cmd.as_str() {
        "timer" | "timers" if rest.is_empty() => {
            let mut out = active_rows();
            if out.is_empty() {
                out.push(json!({"key": "timer:hint", "kind": "hint", "title": "Start a timer", "subtitle": "timer 10m tea · timer 1h30m · timer 90s", "glyph": "clock", "score": 3000}));
            }
            out
        }
        "timer" | "countdown" => {
            let (d, label) = match rest.split_once(' ') {
                Some((a, b)) => (a, b),
                None => (rest.as_str(), ""),
            };
            let Some(secs) = parse_duration(d) else { return vec![] };
            let due = now() + secs;
            vec![row(
                "timer:new",
                format!("Start a {} timer{}", fmt_dur(secs), if label.is_empty() { String::new() } else { format!(": {label}") }),
                format!("Rings at {} · Enter starts it", fmt_when(due)),
                json!({"op": "add", "due": due, "label": label, "tkind": "timer", "secs": secs}),
                5600,
            )]
        }
        "remind" | "remindme" | "reminder" => {
            let Some((due, label)) = parse_reminder(&rest) else {
                return vec![json!({"key": "timer:rhint", "kind": "hint", "title": "Set a reminder", "subtitle": "remind 3pm call · remind in 20m stretch · remind tomorrow 9am bins", "glyph": "bell", "score": 3000})];
            };
            vec![row(
                "timer:remind",
                format!("Remind me{}", if label.is_empty() { String::new() } else { format!(": {label}") }),
                format!("{} · Enter sets it", fmt_when(due)),
                json!({"op": "add", "due": due, "label": label, "tkind": "reminder", "secs": 0}),
                5600,
            )]
        }
        _ => vec![],
    }
}

/// Activate a timer row.
pub fn run(item: &Value) -> Result<String, String> {
    match item["op"].as_str().unwrap_or("") {
        "add" => {
            let t = add(item["due"].as_i64().ok_or("bad timer")?, item["label"].as_str().unwrap_or(""), item["tkind"].as_str().unwrap_or("timer"), item["secs"].as_i64().unwrap_or(0));
            Ok(if t.kind == "timer" { format!("{} started", describe(&t)) } else { format!("Reminder set for {}", fmt_when(t.due)) })
        }
        "cancel" => {
            if cancel(item["id"].as_u64().unwrap_or(0)) {
                Ok("Cancelled".into())
            } else {
                Err("It already ended".into())
            }
        }
        _ => Err("Nothing to do".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durations() {
        assert_eq!(parse_duration("10m"), Some(600));
        assert_eq!(parse_duration("1h30m"), Some(5400));
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("25"), Some(1500));
        assert_eq!(parse_duration("1.5h"), Some(5400));
        assert_eq!(parse_duration("tea"), None);
    }
    #[test]
    fn reminders() {
        let n = Local::now().timestamp();
        let (d, l) = parse_reminder("in 20m stretch").unwrap();
        assert!((d - n - 1200).abs() < 3 && l == "stretch");
        let (d, l) = parse_reminder("3pm call mum").unwrap();
        assert!(d > n && d - n <= 86400 && l == "call mum");
        let (d, l) = parse_reminder("tomorrow 9am bins").unwrap();
        assert!(d > n && l == "bins");
        let (_, l) = parse_reminder("15:30 standup").unwrap();
        assert_eq!(l, "standup");
        let (_, l) = parse_reminder("3 pm call").unwrap();
        assert_eq!(l, "call");
        assert!(parse_reminder("nothing here").is_none());
    }
}
