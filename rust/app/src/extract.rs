//! Plain text out of documents, for content search and previews.
//! Text and code files are read directly; Word/Excel/PowerPoint/OpenDocument/EPUB are zip
//! archives of XML; PDFs go through pdf-extract.
use std::io::Read;
use std::path::Path;

pub const TEXT_EXTS: &[&str] = &[
    "txt", "md", "markdown", "rst", "log", "csv", "tsv", "json", "jsonc", "json5", "yaml", "yml", "toml", "ini", "cfg", "conf", "xml", "html", "htm",
    "css", "scss", "less", "js", "mjs", "cjs", "ts", "tsx", "jsx", "py", "pyw", "rs", "go", "java", "kt", "kts", "c", "h", "cpp", "hpp", "cc", "hh",
    "cxx", "cs", "fs", "vb", "rb", "php", "pl", "sh", "bash", "zsh", "ps1", "psm1", "psd1", "bat", "cmd", "sql", "lua", "swift", "dart", "vue",
    "svelte", "r", "m", "tex", "bib", "srt", "vtt", "ass", "gradle", "properties", "gitignore", "dockerfile", "makefile", "cmake", "iss", "nsi",
    "ahk", "au3", "reg", "graphql", "proto", "zig", "nim", "ex", "exs", "erl", "hs", "clj", "scala", "groovy", "tf", "hcl", "org", "adoc", "rtf",
];
pub const DOC_EXTS: &[&str] = &["docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "pdf"];

/// Never index these: they tend to hold secrets.
const SECRET_EXTS: &[&str] = &["env", "pem", "key", "pfx", "p12", "kdbx", "ppk", "asc", "gpg"];
const SECRET_NAMES: &[&str] = &[".env", "id_rsa", "id_ed25519", "credentials", ".npmrc", ".pypirc", ".netrc", "secrets.json"];

pub fn ext_of(p: &Path) -> String {
    match p.extension() {
        Some(e) => e.to_string_lossy().to_lowercase(),
        None => p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default(),
    }
}

pub fn is_secret(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    SECRET_NAMES.contains(&name.as_str()) || name.starts_with(".env.") || SECRET_EXTS.contains(&ext_of(p).as_str())
}

pub fn indexable(p: &Path) -> bool {
    let e = ext_of(p);
    (TEXT_EXTS.contains(&e.as_str()) || DOC_EXTS.contains(&e.as_str())) && !is_secret(p)
}

/// Cap on how much text one file contributes.
const MAX_CHARS: usize = 1_000_000;

fn decode(bytes: &[u8]) -> Option<String> {
    if bytes.starts_with(&[0xFF, 0xFE]) || bytes.starts_with(&[0xFE, 0xFF]) {
        let le = bytes[0] == 0xFF;
        let units: Vec<u16> =
            bytes[2..].chunks_exact(2).map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) }).collect();
        return Some(String::from_utf16_lossy(&units));
    }
    let head = &bytes[..bytes.len().min(8192)];
    if head.iter().filter(|&&b| b == 0).count() > head.len() / 50 + 1 {
        return None; // binary
    }
    let b = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    Some(String::from_utf8_lossy(b).into_owned())
}

/// Strip XML tags, turning paragraph/cell/row ends into spaces or newlines.
fn xml_text(xml: &str, out: &mut String) {
    let mut in_tag = false;
    let mut tag = String::new();
    for c in xml.chars() {
        if out.len() >= MAX_CHARS {
            return;
        }
        match c {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' if in_tag => {
                in_tag = false;
                let t = tag.as_str();
                if t.starts_with("/w:p") || t.starts_with("/a:p") || t.starts_with("/text:p") || t.starts_with("/p") || t.starts_with("br") || t.starts_with("/h") || t.starts_with("/row") || t.starts_with("/table:table-row") {
                    out.push('\n');
                } else if t.starts_with("/c") || t.starts_with("w:tab") || t.starts_with("/table:table-cell") || t.starts_with("/si") || t.starts_with("/td") {
                    out.push(' ');
                }
            }
            _ if in_tag => {
                if tag.len() < 24 {
                    tag.push(c);
                }
            }
            _ => out.push(c),
        }
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

fn zip_text(path: &Path, ext: &str) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let mut z = zip::ZipArchive::new(f).ok()?;
    let mut names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|e| e.name().to_string())).collect();
    let wanted = |n: &str| -> bool {
        match ext {
            "docx" => n == "word/document.xml" || n.starts_with("word/footnotes") || n.starts_with("word/header") || n.starts_with("word/footer"),
            "xlsx" => n == "xl/sharedStrings.xml" || (n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml")),
            "pptx" => n.starts_with("ppt/slides/slide") && n.ends_with(".xml"),
            "odt" | "ods" | "odp" => n == "content.xml",
            "epub" => n.ends_with(".xhtml") || n.ends_with(".html") || n.ends_with(".htm"),
            _ => false,
        }
    };
    names.retain(|n| wanted(n));
    // slides and sheets in order: slide2 before slide10
    names.sort_by_key(|n| {
        let digits: String = n.chars().filter(|c| c.is_ascii_digit()).collect();
        (digits.parse::<u64>().unwrap_or(0), n.clone())
    });
    let mut out = String::new();
    for n in names {
        let Ok(mut e) = z.by_name(&n) else { continue };
        if e.size() > 64 * 1024 * 1024 {
            continue;
        }
        let mut xml = String::new();
        if e.read_to_string(&mut xml).is_ok() {
            xml_text(&xml, &mut out);
            out.push('\n');
        }
        if out.len() >= MAX_CHARS {
            break;
        }
    }
    Some(unescape(&out))
}

fn pdf_text(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    // pdf-extract panics on some malformed files; contain it
    let r = std::panic::catch_unwind(move || pdf_extract::extract_text_from_mem(&bytes).ok());
    r.ok().flatten()
}

fn strip_rtf(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    let mut depth_skip = 0i32;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut word = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphabetic() {
                        word.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_digit() || n == '-' {
                        chars.next();
                    } else {
                        break;
                    }
                }
                if chars.peek() == Some(&' ') {
                    chars.next();
                }
                if word == "par" || word == "line" {
                    out.push('\n');
                }
                if word.is_empty() {
                    if let Some(n) = chars.next() {
                        if n == '\\' || n == '{' || n == '}' {
                            out.push(n);
                        }
                    }
                }
                if matches!(word.as_str(), "fonttbl" | "colortbl" | "stylesheet" | "info" | "pict") {
                    depth_skip = 1;
                }
            }
            '{' => {
                if depth_skip > 0 {
                    depth_skip += 1;
                }
            }
            '}' => {
                if depth_skip > 0 {
                    depth_skip -= 1;
                }
            }
            _ if depth_skip > 0 => {}
            '\r' | '\n' => {}
            _ => out.push(c),
        }
    }
    out
}

/// The text of a file, or None if it isn't a type we read (or can't be read).
pub fn text(path: &Path, max_bytes: u64) -> Option<String> {
    let ext = ext_of(path);
    let size = std::fs::metadata(path).ok()?.len();
    if size > max_bytes {
        return None;
    }
    let mut t = if TEXT_EXTS.contains(&ext.as_str()) {
        if size > 8 * 1024 * 1024 {
            return None;
        }
        let s = decode(&std::fs::read(path).ok()?)?;
        if ext == "rtf" {
            strip_rtf(&s)
        } else {
            s
        }
    } else if ext == "pdf" {
        pdf_text(path)?
    } else if DOC_EXTS.contains(&ext.as_str()) {
        zip_text(path, &ext)?
    } else {
        return None;
    };
    if t.len() > MAX_CHARS {
        let mut end = MAX_CHARS;
        while !t.is_char_boundary(end) {
            end -= 1;
        }
        t.truncate(end);
    }
    Some(t)
}

/// A short excerpt around the first place any of `terms` appears, with match ranges
/// (byte offsets in the returned string) for highlighting.
pub fn snippet(text: &str, terms: &[String], radius: usize) -> (String, Vec<(usize, usize)>) {
    let lower = text.to_lowercase();
    // lowercase can change byte lengths for some scripts; only trust offsets when it doesn't
    let same = lower.len() == text.len();
    let first = terms.iter().filter(|t| !t.is_empty()).filter_map(|t| lower.find(t.as_str())).min();
    let Some(pos) = first.filter(|_| same) else {
        let mut end = text.len().min(radius * 2);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        return (text[..end].trim().to_string(), vec![]);
    };
    let mut start = pos.saturating_sub(radius);
    while !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (pos + radius).min(text.len());
    while !text.is_char_boundary(end) {
        end += 1;
    }
    // start at a word/line boundary
    if start > 0 {
        if let Some(sp) = text[start..pos].find([' ', '\n']) {
            start += sp + 1;
        }
    }
    let raw = &text[start..end];
    let piece: String = raw.chars().map(|c| if c == '\r' || c == '\t' { ' ' } else { c }).collect();
    let pl = piece.to_lowercase();
    let mut marks = vec![];
    for t in terms.iter().filter(|t| !t.is_empty()) {
        let mut from = 0;
        while let Some(i) = pl[from..].find(t.as_str()) {
            marks.push((from + i, from + i + t.len()));
            from += i + t.len();
        }
    }
    marks.sort();
    let prefix = if start > 0 { "…" } else { "" };
    let suffix = if end < text.len() { "…" } else { "" };
    let shift = prefix.len();
    (format!("{prefix}{piece}{suffix}"), marks.into_iter().map(|(a, b)| (a + shift, b + shift)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_and_snippets() {
        let mut out = String::new();
        xml_text("<w:p><w:r><w:t>Hello</w:t></w:r></w:p><w:p><w:t>World &amp; co</w:t></w:p>", &mut out);
        assert_eq!(unescape(&out).trim(), "Hello\nWorld & co");
        let (s, m) = snippet("The quick brown fox jumps over the lazy dog", &["fox".into()], 10);
        assert!(s.contains("fox"));
        assert_eq!(&s[m[0].0..m[0].1], "fox");
    }

    #[test]
    fn secrets_and_types() {
        assert!(is_secret(Path::new(r"C:\x\.env")));
        assert!(is_secret(Path::new(r"C:\x\.env.local")));
        assert!(is_secret(Path::new(r"C:\x\server.pem")));
        assert!(!indexable(Path::new(r"C:\x\.env")));
        assert!(indexable(Path::new(r"C:\x\notes.md")));
        assert!(indexable(Path::new(r"C:\x\report.docx")));
        assert!(!indexable(Path::new(r"C:\x\movie.mp4")));
        assert_eq!(decode(b"\xEF\xBB\xBFhi").unwrap(), "hi");
        assert!(decode(&[0u8; 100]).is_none());
        assert!(strip_rtf(r"{\rtf1\ansi{\fonttbl\f0 Arial;}\f0 Hello\par World}").contains("Hello\nWorld"));
    }
}
