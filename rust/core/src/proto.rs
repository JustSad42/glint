//! Messages between the launcher and the indexer service: one JSON object per line.
use crate::query::Query;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Request {
    Search {
        #[serde(flatten)]
        query: Query,
    },
    Status,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Hit {
    pub path: String,
    pub dir: bool,
    pub score: i32,
    /// modified (unix seconds) and size, when the indexer knows them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchReply {
    pub hits: Vec<Hit>,
    /// how long the search itself took
    pub micros: u64,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct VolumeStatus {
    pub letter: char,
    pub entries: usize,
    pub ready: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// how long the last full scan took
    #[serde(default)]
    pub scan_ms: u64,
    /// sizes and dates filled in for every file
    #[serde(default)]
    pub meta_ready: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StatusReply {
    pub version: String,
    pub volumes: Vec<VolumeStatus>,
    pub total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let r = Request::Search { query: crate::query::parse("abc *.pdf") };
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains(r#""op":"search""#));
        let back: Request = serde_json::from_str(&s).unwrap();
        match back {
            Request::Search { query } => assert_eq!(query.exts, vec!["pdf"]),
            _ => panic!(),
        }
        let st: Request = serde_json::from_str(r#"{"op":"status"}"#).unwrap();
        assert!(matches!(st, Request::Status));
    }
}
