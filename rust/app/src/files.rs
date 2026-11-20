//! File and folder name search: asks the Glint Indexer service over its pipe, and falls back to
//! the content indexer's own file list (your user folder only) when the service isn't running.
use crate::{content, history, icons, settings};
use glint_core::pipe::Client;
use glint_core::proto::{Hit, Request, SearchReply, StatusReply};
use glint_core::query::{self, Kind};
use glint_core::PIPE_NAME;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::path::Path;

static CLIENT: Lazy<Mutex<Client>> = Lazy::new(|| Mutex::new(Client::new(PIPE_NAME)));

pub fn key(path: &str) -> String {
    format!("file:{}", path.to_lowercase())
}

/// `C:\Users\me\Documents` -> `~\Documents`
pub fn pretty_dir(path: &str) -> String {
    let home = settings::user_home().to_string_lossy().into_owned();
    if path.len() >= home.len() && path[..home.len()].eq_ignore_ascii_case(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

pub fn item(path: &str, dir: bool, score: i32) -> Value {
    item_meta(path, dir, score, None, None)
}

pub fn item_meta(path: &str, dir: bool, score: i32, mtime: Option<u32>, size: Option<u64>) -> Value {
    let p = Path::new(path);
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
    let parent = p.parent().map(|d| pretty_dir(&d.to_string_lossy())).unwrap_or_default();
    json!({
        "key": key(path), "kind": if dir { "folder" } else { "file" }, "title": name, "subtitle": parent,
        "icon": icons::path_url(path), "path": path, "score": score, "mtime": mtime, "size": size,
    })
}

fn ask(req: &Request) -> Result<String, String> {
    let line = serde_json::to_string(req).map_err(|e| e.to_string())?;
    CLIENT.lock().call(&line).map_err(|e| e.to_string())
}

pub fn status() -> Option<StatusReply> {
    ask(&Request::Status).ok().and_then(|s| serde_json::from_str(&s).ok())
}

/// (results, came from the indexer service?)
pub fn search(input: &str, kind: Option<Kind>, limit: usize) -> (Vec<(i32, Value)>, bool) {
    let mut q = query::parse(input);
    if let Some(k) = kind {
        q.kind = k;
    }
    if q.is_empty() {
        return (vec![], true);
    }
    // ask for more than we show, so learned favourites can move up
    q.limit = (limit * 4).max(40);
    let (hits, live) = match ask(&Request::Search { query: q.clone() }).ok().and_then(|s| serde_json::from_str::<SearchReply>(&s).ok()) {
        Some(r) => (r.hits, true),
        None => (content::name_search(&q), false),
    };
    let mut out: Vec<(i32, Value)> = hits
        .into_iter()
        .map(|Hit { path, dir, score, mtime, size }| {
            let s = score + history::boost(&key(&path), input) * 2;
            (s, item_meta(&path, dir, s, mtime, size))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    (out, live)
}
