//! Turns what the user typed into a file query.
//!   report 2024          both words, in the name or in a parent folder's name
//!   "annual report"      a phrase, spaces included
//!   *.pdf  ext:pdf,docx  only those extensions
//!   folder:  file:       only folders / only files
//!   dm:today  dm:yesterday  dm:week  dm:month  dm:year  dm:3d  dm:12h  dm:2026-11-20  dm:2026-11
//!   size:>1gb  size:<10mb  size:1mb..5mb  size:empty  size:large (>128 MB)  size:huge (>1 GB)
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Any,
    File,
    Dir,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Query {
    /// lowercase words; every one must appear in the name or a parent folder's name
    #[serde(default)]
    pub tokens: Vec<String>,
    /// lowercase extensions without the dot
    #[serde(default)]
    pub exts: Vec<String>,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// modified at or after / before (unix seconds)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm_min: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dm_max: Option<u32>,
    /// size in bytes, inclusive
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_min: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_max: Option<u64>,
}

fn default_limit() -> usize {
    50
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && self.exts.is_empty() && !self.has_filters()
    }

    pub fn has_filters(&self) -> bool {
        self.dm_min.is_some() || self.dm_max.is_some() || self.size_min.is_some() || self.size_max.is_some()
    }
}

/// "1.5gb" -> bytes (1024-based, like Explorer).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let split = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: f64 = num.trim().parse().ok()?;
    let mult: f64 = match unit.trim() {
        "" | "b" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024f64.powi(4),
        _ => return None,
    };
    (n >= 0.0).then(|| (n * mult) as u64)
}

/// size:>1gb  size:<10mb  size:1mb..5mb  size:empty  size:large  size:huge
fn parse_size_filter(v: &str) -> Option<(Option<u64>, Option<u64>)> {
    match v {
        "empty" => return Some((None, Some(0))),
        "tiny" => return Some((None, Some(16 * 1024))),
        "small" => return Some((None, Some(1024 * 1024))),
        "large" | "big" => return Some((Some(128 << 20), None)),
        "huge" => return Some((Some(1 << 30), None)),
        _ => {}
    }
    if let Some((a, b)) = v.split_once("..") {
        return Some((parse_size(a), parse_size(b)));
    }
    if let Some(x) = v.strip_prefix(">=").or_else(|| v.strip_prefix('>')) {
        return Some((Some(parse_size(x)?), None));
    }
    if let Some(x) = v.strip_prefix("<=").or_else(|| v.strip_prefix('<')) {
        return Some((None, Some(parse_size(x)?)));
    }
    // a bare size means "about that big or bigger"
    Some((Some(parse_size(v)?), None))
}

/// dm:today, dm:week, dm:3d, dm:2026-11-20... -> (from, until) in unix seconds, local time.
fn parse_date_filter(v: &str) -> Option<(Option<u32>, Option<u32>)> {
    use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
    let now = Local::now();
    let midnight = |d: NaiveDate| Local.from_local_datetime(&d.and_hms_opt(0, 0, 0)?).earliest().map(|t| t.timestamp() as u32);
    let today = now.date_naive();
    let ago = |secs: i64| Some((now.timestamp() - secs).max(0) as u32);
    Some(match v {
        "today" => (midnight(today), None),
        "yesterday" => (midnight(today - Duration::days(1)), midnight(today)),
        "week" | "thisweek" => (ago(7 * 86400), None),
        "month" | "thismonth" => (ago(30 * 86400), None),
        "year" | "thisyear" => (midnight(NaiveDate::from_ymd_opt(today.year(), 1, 1)?), None),
        _ => {
            // 3d, 12h, 2w, 6m (months), 1y
            if let Some(unit) = v.chars().last() {
                if let Ok(n) = v[..v.len() - unit.len_utf8()].parse::<i64>() {
                    let secs = match unit {
                        'h' => 3600,
                        'd' => 86400,
                        'w' => 7 * 86400,
                        'm' => 30 * 86400,
                        'y' => 365 * 86400,
                        _ => return None,
                    };
                    return Some((ago(n * secs), None));
                }
            }
            // >2026-01-01  <2026-01-01  2026-11-20  2026-11
            if let Some(d) = v.strip_prefix('>') {
                return Some((parse_day(d).and_then(midnight), None));
            }
            if let Some(d) = v.strip_prefix('<') {
                return Some((None, parse_day(d).and_then(midnight)));
            }
            if let Some(d) = parse_day(v) {
                return Some((midnight(d), midnight(d + Duration::days(1))));
            }
            let parts: Vec<&str> = v.split('-').collect();
            if parts.len() == 2 {
                let (y, m): (i32, u32) = (parts[0].parse().ok()?, parts[1].parse().ok()?);
                let start = NaiveDate::from_ymd_opt(y, m, 1)?;
                let end = if m == 12 { NaiveDate::from_ymd_opt(y + 1, 1, 1)? } else { NaiveDate::from_ymd_opt(y, m + 1, 1)? };
                return Some((midnight(start), midnight(end)));
            }
            return None;
        }
    })
}

fn parse_day(s: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// Split on spaces, keeping "quoted phrases" together.
fn split(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in input.chars() {
        match c {
            '"' => {
                if quoted && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                quoted = !quoted;
            }
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn parse(input: &str) -> Query {
    let mut q = Query { limit: default_limit(), ..Default::default() };
    for raw in split(input) {
        let w = raw.to_lowercase().replace('/', "\\");
        if let Some(v) = w.strip_prefix("dm:").or_else(|| w.strip_prefix("modified:")).or_else(|| w.strip_prefix("date:")) {
            if let Some((a, b)) = parse_date_filter(v) {
                q.dm_min = a.or(q.dm_min);
                q.dm_max = b.or(q.dm_max);
                continue;
            }
        }
        if let Some(v) = w.strip_prefix("size:") {
            if let Some((a, b)) = parse_size_filter(v) {
                q.size_min = a.or(q.size_min);
                q.size_max = b.or(q.size_max);
                continue;
            }
        }
        if let Some(list) = w.strip_prefix("ext:") {
            q.exts.extend(list.split([',', ';']).map(|e| e.trim().trim_start_matches('.').to_string()).filter(|e| !e.is_empty()));
            continue;
        }
        if let Some(e) = w.strip_prefix("*.") {
            if !e.is_empty() && !e.contains(['*', '?', '\\']) {
                q.exts.push(e.to_string());
                continue;
            }
        }
        match w.as_str() {
            "folder:" | "folders:" | "dir:" | "kind:folder" => {
                q.kind = Kind::Dir;
                continue;
            }
            "file:" | "files:" | "kind:file" => {
                q.kind = Kind::File;
                continue;
            }
            _ => {}
        }
        let w = w.trim_matches('*').to_string();
        if !w.is_empty() {
            q.tokens.push(w);
        }
    }
    q.exts.sort();
    q.exts.dedup();
    // longest first: the scan rejects most entries on the first (most selective) word
    q.tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
    q.tokens.dedup();
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_and_filters() {
        let q = parse(r#"Report "Annual Plan" *.PDF ext:docx,.txt folder:"#);
        assert_eq!(q.tokens, vec!["annual plan", "report"]);
        assert_eq!(q.exts, vec!["docx", "pdf", "txt"]);
        assert_eq!(q.kind, Kind::Dir);
    }

    #[test]
    fn date_and_size() {
        let q = parse("report dm:today size:>1mb");
        assert_eq!(q.tokens, vec!["report"]);
        assert!(q.dm_min.is_some() && q.dm_max.is_none());
        assert_eq!(q.size_min, Some(1 << 20));
        let q = parse("dm:2026-11-20 size:1kb..2kb");
        assert_eq!(q.dm_max.unwrap() - q.dm_min.unwrap(), 86400);
        assert_eq!((q.size_min, q.size_max), (Some(1024), Some(2048)));
        assert!(!parse("dm:week").is_empty());
        assert!(parse("size:huge").size_min == Some(1 << 30));
        // not a valid filter: stays a word
        assert_eq!(parse("dm:soon").tokens, vec!["dm:soon"]);
    }

    #[test]
    fn slashes_and_stars() {
        let q = parse("src/main *notes*");
        assert_eq!(q.tokens, vec!["src\\main", "notes"]);
        assert!(parse("   ").is_empty());
    }
}
