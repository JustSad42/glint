//! Turns what the user typed into a file query.
//!   report 2024          both words, in the name or in a parent folder's name
//!   "annual report"      a phrase, spaces included
//!   *.pdf  ext:pdf,docx  only those extensions
//!   folder:  file:       only folders / only files
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
}

fn default_limit() -> usize {
    50
}

impl Query {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && self.exts.is_empty()
    }
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
    fn slashes_and_stars() {
        let q = parse("src/main *notes*");
        assert_eq!(q.tokens, vec!["src\\main", "notes"]);
        assert!(parse("   ").is_empty());
    }
}
