//! Pinned results (always first when they match, and on the empty screen) and keywords
//! ("vs" -> Visual Studio Code, "dl" -> Downloads).
use crate::settings;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default, Serialize, Deserialize)]
struct Data {
    #[serde(default)]
    pinned: Vec<Value>,
    /// keyword -> item
    #[serde(default)]
    keywords: BTreeMap<String, Value>,
}

static DATA: Lazy<Mutex<Data>> = Lazy::new(|| {
    Mutex::new(std::fs::read_to_string(settings::data_file("pins.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
});

fn save(d: &Data) {
    if let Ok(t) = serde_json::to_string_pretty(d) {
        let _ = settings::write_atomic(&settings::data_file("pins.json"), t.as_bytes());
    }
}

fn key_of(v: &Value) -> &str {
    v["key"].as_str().unwrap_or("")
}

pub fn pin(item: &Value) {
    let mut d = DATA.lock();
    let k = key_of(item).to_string();
    d.pinned.retain(|x| key_of(x) != k);
    d.pinned.push(item.clone());
    save(&d);
}

pub fn unpin(key: &str) {
    let mut d = DATA.lock();
    d.pinned.retain(|x| key_of(x) != key);
    save(&d);
}

pub fn is_pinned(key: &str) -> bool {
    DATA.lock().pinned.iter().any(|x| key_of(x) == key)
}

pub fn pinned() -> Vec<Value> {
    DATA.lock().pinned.clone()
}

/// Lowercase, no spaces: keywords are typed as one word.
pub fn clean_keyword(k: &str) -> String {
    k.trim().to_lowercase().split_whitespace().collect::<Vec<_>>().join("")
}

pub fn set_keyword(word: &str, item: &Value) -> Result<String, String> {
    let w = clean_keyword(word);
    if w.is_empty() {
        return Err("Type a keyword".into());
    }
    let mut d = DATA.lock();
    d.keywords.insert(w.clone(), item.clone());
    save(&d);
    Ok(w)
}

pub fn remove_keyword(word: &str) {
    let mut d = DATA.lock();
    d.keywords.remove(&clean_keyword(word));
    save(&d);
}

pub fn keywords_for(key: &str) -> Vec<String> {
    DATA.lock().keywords.iter().filter(|(_, v)| key_of(v) == key).map(|(k, _)| k.clone()).collect()
}

/// Items whose keyword is what was typed (exact first, then keywords starting with it).
pub fn keyword_hits(q: &str) -> Vec<Value> {
    let ql = clean_keyword(q);
    if ql.is_empty() {
        return vec![];
    }
    let d = DATA.lock();
    let mut out: Vec<(i32, Value)> = d
        .keywords
        .iter()
        .filter(|(k, _)| k.starts_with(&ql))
        .map(|(k, v)| {
            let mut it = v.clone();
            let s = if *k == ql { 20_000 } else { 6_000 - k.len() as i32 };
            it["score"] = json!(s);
            it["keyword"] = json!(k);
            (s, it)
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.into_iter().map(|(_, v)| v).collect()
}

/// For the settings page.
pub fn list() -> Value {
    let d = DATA.lock();
    json!({
        "pinned": d.pinned.iter().map(|v| json!({"key": key_of(v), "title": v["title"], "kind": v["kind"]})).collect::<Vec<_>>(),
        "keywords": d.keywords.iter().map(|(k, v)| json!({"keyword": k, "key": key_of(v), "title": v["title"], "kind": v["kind"]})).collect::<Vec<_>>(),
    })
}
