//! Your own commands: every script in `%LOCALAPPDATA%\Glint\scripts` shows up by name.
//! Typing words after its name passes them as arguments ("deploy staging").
//! A comment on the first line (`# ...`, `REM ...`, `:: ...`, `// ...`) becomes its description.
//! Enter runs it hidden and shows its last line of output; Shift+Enter runs it in a terminal.
use crate::{fuzzy, settings};
use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const EXTS: &[&str] = &["ps1", "bat", "cmd", "py", "js", "exe", "ahk", "vbs"];
const NO_WINDOW: u32 = 0x0800_0000;

pub fn dir() -> PathBuf {
    settings::data_file("scripts")
}

/// Make the folder (with a short example) the first time it's opened.
pub fn ensure_dir() -> PathBuf {
    let d = dir();
    if !d.exists() {
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(
            d.join("hello.ps1"),
            "# Example Glint script: type \"hello world\" in Glint\r\nparam([string]$who = \"there\")\r\nWrite-Output \"Hello, $who! Edit or delete me in the scripts folder.\"\r\n",
        );
    }
    d
}

fn describe(p: &Path) -> String {
    let Ok(s) = std::fs::read_to_string(p) else { return String::new() };
    let first = s.lines().map(str::trim).find(|l| !l.is_empty() && !l.eq_ignore_ascii_case("@echo off") && !l.starts_with("#!")).unwrap_or("");
    for pre in ["#", "REM ", "rem ", "::", "//", "'", ";"] {
        if let Some(r) = first.strip_prefix(pre) {
            return r.trim().to_string();
        }
    }
    String::new()
}

fn list() -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir()) else { return vec![] };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| EXTS.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false))
        .collect()
}

pub fn search(q: &str, limit: usize) -> Vec<(i32, Value)> {
    let q = q.trim();
    if q.is_empty() {
        return vec![];
    }
    let (head, args) = q.split_once(' ').map(|(a, b)| (a, b.trim())).unwrap_or((q, ""));
    let mut out: Vec<(i32, Value)> = list()
        .into_iter()
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            // with arguments, the first word must be the script's name (or its start)
            let s = if args.is_empty() { fuzzy::score(q, &name)? } else { fuzzy::score(head, &name).filter(|&s| s >= 840)? };
            let desc = describe(&p);
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            let sub = match (desc.is_empty(), args.is_empty()) {
                (_, false) => format!("Run with: {args}"),
                (false, true) => desc,
                (true, true) => format!("Your script · .{ext}"),
            };
            Some((
                s,
                json!({
                    "key": format!("script:{}", name.to_lowercase()), "kind": "script", "title": name, "subtitle": sub,
                    "glyph": "terminal", "path": p.to_string_lossy(), "args": args, "score": s,
                }),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out.truncate(limit);
    out
}

fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn command_for(path: &str, args: &[String]) -> Command {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut c = match ext.as_str() {
        "ps1" => {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", path]);
            c
        }
        "bat" | "cmd" => {
            let mut c = Command::new("cmd.exe");
            c.args(["/d", "/c", path]);
            c
        }
        "py" => {
            let mut c = Command::new("py");
            c.arg(path);
            c
        }
        "js" => {
            let mut c = Command::new("node");
            c.arg(path);
            c
        }
        "vbs" => {
            let mut c = Command::new("cscript.exe");
            c.args(["//nologo", path]);
            c
        }
        _ => Command::new(path),
    };
    c.args(args);
    if let Some(d) = Path::new(path).parent() {
        c.current_dir(d);
    }
    c
}

/// Run hidden; `done` gets the outcome text when it finishes.
pub fn run_hidden(path: &str, args: &str, done: impl FnOnce(Result<String, String>) + Send + 'static) {
    let path = path.to_string();
    let args = split_args(args);
    std::thread::spawn(move || {
        let name = Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let r = command_for(&path, &args).creation_flags(NO_WINDOW).stdin(Stdio::null()).output();
        done(match r {
            Err(e) => Err(format!("{name}: couldn't start ({e})")),
            Ok(o) => {
                let text = String::from_utf8_lossy(if o.status.success() || o.stderr.is_empty() { &o.stdout } else { &o.stderr }).into_owned();
                let last = text.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").chars().take(160).collect::<String>();
                if o.status.success() {
                    Ok(if last.is_empty() { format!("{name} finished") } else { last })
                } else {
                    Err(if last.is_empty() { format!("{name} failed (exit {})", o.status.code().unwrap_or(-1)) } else { format!("{name}: {last}") })
                }
            }
        });
    });
}

/// Run in a terminal window that stays open, to watch it.
pub fn run_in_terminal(path: &str, args: &str) -> Result<(), String> {
    let args = split_args(args);
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let quoted: Vec<String> = args.iter().map(|a| format!("'{}'", a.replace('\'', "''"))).collect();
    let inner = match ext.as_str() {
        "ps1" => format!("& '{}' {}", path.replace('\'', "''"), quoted.join(" ")),
        _ => format!("& {}", {
            let c = command_for(path, &args);
            let prog = c.get_program().to_string_lossy().into_owned();
            let a: Vec<String> = c.get_args().map(|x| format!("'{}'", x.to_string_lossy().replace('\'', "''"))).collect();
            format!("'{}' {}", prog, a.join(" "))
        }),
    };
    let mut c = Command::new("powershell.exe");
    c.args(["-NoExit", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &inner]);
    if let Some(d) = Path::new(path).parent() {
        c.current_dir(d);
    }
    c.creation_flags(0x0000_0010).spawn().map(|_| ()).map_err(|e| e.to_string())
}
