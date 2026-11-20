//! Learns what you open, so it ranks higher next time.
//! - frecency: how often and how recently each item was opened
//! - picks: which item you chose after typing a given query ("ch" -> Chrome)
//! - queries: recent queries, recalled with the Up arrow
use crate::settings;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Use {
    pub count: u32,
    pub last: u64,
    /// enough to show the item again on the empty screen
    #[serde(default)]
    pub item: serde_json::Value,
}

#[derive(Default, Serialize, Deserialize)]
struct Data {
    uses: HashMap<String, Use>,
    picks: HashMap<String, HashMap<String, u32>>,
    queries: Vec<String>,
}

static DATA: Lazy<Mutex<Data>> = Lazy::new(|| {
    Mutex::new(std::fs::read_to_string(settings::data_file("history.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default())
});

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn save(d: &Data) {
    if let Ok(t) = serde_json::to_string(d) {
        let _ = settings::write_atomic(&settings::data_file("history.json"), t.as_bytes());
    }
}

/// Record that `key` was opened after typing `query`.
pub fn record(key: &str, query: &str, item: serde_json::Value) {
    let mut d = DATA.lock();
    let u = d.uses.entry(key.to_string()).or_default();
    u.count += 1;
    u.last = now();
    if !item.is_null() {
        u.item = item;
    }
    let q = query.trim().to_lowercase();
    if !q.is_empty() && q.len() <= 40 {
        *d.picks.entry(q.clone()).or_default().entry(key.to_string()).or_default() += 1;
        d.queries.retain(|x| x != &q);
        d.queries.insert(0, q);
        d.queries.truncate(50);
    }
    // keep the file small
    if d.uses.len() > 3000 {
        let mut v: Vec<(String, u64)> = d.uses.iter().map(|(k, u)| (k.clone(), u.last)).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1));
        for (k, _) in v.into_iter().skip(2500) {
            d.uses.remove(&k);
        }
    }
    if d.picks.len() > 3000 {
        let keys: Vec<String> = d.picks.keys().take(500).cloned().collect();
        for k in keys {
            d.picks.remove(&k);
        }
    }
    save(&d);
}

/// Forget an item (it was deleted, or the user removed it from Recent).
pub fn forget(key: &str) {
    let mut d = DATA.lock();
    d.uses.remove(key);
    for m in d.picks.values_mut() {
        m.remove(key);
    }
    save(&d);
}

pub fn clear() {
    let mut d = DATA.lock();
    *d = Data::default();
    save(&d);
}

/// 0..=400: frequent and recent items score higher; a week-old open counts half.
fn frecency_of(u: &Use, now: u64) -> i32 {
    let age_days = now.saturating_sub(u.last) as f64 / 86400.0;
    let recency = 0.5f64.powf(age_days / 7.0);
    let f = (u.count as f64).ln_1p() * 60.0 * (0.35 + 0.65 * recency);
    f.min(400.0) as i32
}

/// Boost for `key` given the current query.
pub fn boost(key: &str, query: &str) -> i32 {
    let d = DATA.lock();
    let now = now();
    let mut b = d.uses.get(key).map(|u| frecency_of(u, now)).unwrap_or(0);
    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        if let Some(n) = d.picks.get(&q).and_then(|m| m.get(key)) {
            b += 300 + (*n as i32).min(10) * 30;
        } else {
            // picks made after typing a longer query that starts with this one ("chr" when typing "ch")
            let partial = d.picks.iter().filter(|(pq, _)| pq.starts_with(&q)).filter_map(|(_, m)| m.get(key)).sum::<u32>();
            b += (partial as i32).min(8) * 25;
        }
    }
    b
}

/// Most used recently, for the empty screen.
pub fn recent(n: usize) -> Vec<(String, Use)> {
    let d = DATA.lock();
    let now = now();
    let mut v: Vec<(String, Use)> = d.uses.iter().filter(|(_, u)| !u.item.is_null()).map(|(k, u)| (k.clone(), u.clone())).collect();
    v.sort_by(|a, b| {
        let fa = frecency_of(&a.1, now) as f64 + 0.5f64.powf(now.saturating_sub(a.1.last) as f64 / 86400.0) * 80.0;
        let fb = frecency_of(&b.1, now) as f64 + 0.5f64.powf(now.saturating_sub(b.1.last) as f64 / 86400.0) * 80.0;
        fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
    });
    v.truncate(n);
    v
}

pub fn queries() -> Vec<String> {
    DATA.lock().queries.clone()
}
