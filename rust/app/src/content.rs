//! Full-text search over the documents in your user folder (or the folders set in Settings).
//!
//! A background thread at low CPU and disk priority walks the folders, extracts text from
//! documents and code, and keeps a tantivy index in the data folder. After the first pass a file
//! watcher picks up changes within seconds; a full re-check runs every few hours to catch anything
//! the watcher missed. Online-only cloud files are skipped (reading them would download them),
//! as are files that usually hold secrets (.env, keys, password databases).
use crate::{extract, files, icons, settings};
use glint_core::proto::Hit;
use glint_core::query::{Kind, Query};
use notify::{RecursiveMode, Watcher};
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};
use tantivy::collector::TopDocs;
use tantivy::directory::MmapDirectory;
use tantivy::query::{BooleanQuery, FuzzyTermQuery, Occur, Query as TQuery, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Schema, Value as TValue, STORED, STRING, TEXT};
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument, Term};

const SCHEMA_VERSION: u32 = 1;

/// Folder names never walked.
const SKIP_DIRS: &[&str] = &[
    "appdata", "node_modules", ".git", ".svn", ".hg", "target", "$recycle.bin", "__pycache__", ".venv", "venv", ".cache", ".gradle", ".cargo", ".rustup",
    ".nuget", ".vs", ".idea", "obj", ".next", ".nuxt", ".pnpm-store", "site-packages", "bower_components", ".tox", ".mypy_cache", ".pytest_cache",
    "ebwebview", "graphify-out", ".npm", ".yarn", "scoop", "go", ".m2", ".android", ".dotnet", "ntuser.dat",
];

// file attributes
const A_SYSTEM: u32 = 0x4;
const A_REPARSE: u32 = 0x400;
const A_OFFLINE: u32 = 0x1000;
const A_RECALL_ON_OPEN: u32 = 0x40000;
const A_RECALL_ON_DATA: u32 = 0x400000;
const A_CLOUD: u32 = A_OFFLINE | A_RECALL_ON_OPEN | A_RECALL_ON_DATA;

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub enabled: bool,
    pub phase: String,
    pub indexed: usize,
    pub pending: usize,
    pub last_full: u64,
    pub error: Option<String>,
}

pub static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status::default()));
static REBUILD: AtomicBool = AtomicBool::new(false);
static RESTART: AtomicBool = AtomicBool::new(false);

struct Engine {
    index: Index,
    reader: IndexReader,
    path: Field,
    name: Field,
    body: Field,
    mtime: Field,
}

static ENGINE: OnceCell<Engine> = OnceCell::new();

/// Every file under the content folders, for name search when the indexer service is missing.
static NAMES: Lazy<RwLock<Vec<(String, String, bool)>>> = Lazy::new(|| RwLock::new(Vec::new()));

#[derive(Default, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    /// path -> (modified time, size) when indexed
    files: HashMap<String, (u64, u64)>,
}

fn index_dir() -> PathBuf {
    settings::data_file("content-index")
}
fn manifest_path() -> PathBuf {
    settings::data_file("content-manifest.json")
}

fn schema() -> (Schema, Field, Field, Field, Field) {
    let mut sb = Schema::builder();
    let path = sb.add_text_field("path", STRING | STORED);
    let name = sb.add_text_field("name", TEXT);
    let body = sb.add_text_field("body", TEXT);
    let mtime = sb.add_u64_field("mtime", STORED);
    (sb.build(), path, name, body, mtime)
}

fn open_engine() -> Result<Engine, String> {
    let dir = index_dir();
    let (schema, path, name, body, mtime) = schema();
    let open = |dir: &Path| -> tantivy::Result<Index> {
        std::fs::create_dir_all(dir)?;
        Index::open_or_create(MmapDirectory::open(dir).map_err(|e| tantivy::TantivyError::SystemError(e.to_string()))?, schema.clone())
    };
    let index = match open(&dir) {
        Ok(i) => i,
        Err(_) => {
            // incompatible or damaged: start over
            let _ = std::fs::remove_dir_all(&dir);
            let _ = std::fs::remove_file(manifest_path());
            open(&dir).map_err(|e| e.to_string())?
        }
    };
    let reader = index.reader_builder().reload_policy(ReloadPolicy::OnCommitWithDelay).try_into().map_err(|e: tantivy::TantivyError| e.to_string())?;
    Ok(Engine { index, reader, path, name, body, mtime })
}

fn mtime_of(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn excluded(path_lower: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|x| !x.is_empty() && path_lower.contains(x.as_str()))
}

fn skip_dir(e: &walkdir::DirEntry, data_dir: &str, excludes: &[String]) -> bool {
    if e.depth() == 0 {
        return false;
    }
    let name = e.file_name().to_string_lossy().to_lowercase();
    if SKIP_DIRS.contains(&name.as_str()) || (name.starts_with('.') && name.len() > 1) {
        return true;
    }
    if let Ok(m) = e.metadata() {
        let a = m.file_attributes();
        if a & (A_REPARSE | A_CLOUD) != 0 || (a & A_SYSTEM != 0) {
            return true;
        }
    }
    let lower = e.path().to_string_lossy().to_lowercase();
    lower.starts_with(data_dir) || excluded(&lower, excludes)
}

fn background_priority() {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

struct Run {
    writer: IndexWriter,
    manifest: Manifest,
    since_commit: usize,
    last_commit: Instant,
    max_bytes: u64,
}

impl Run {
    fn commit(&mut self, force: bool) {
        if self.since_commit == 0 || (!force && self.since_commit < 300 && self.last_commit.elapsed() < Duration::from_secs(15)) {
            return;
        }
        if let Err(e) = self.writer.commit() {
            STATUS.lock().error = Some(e.to_string());
            return;
        }
        self.since_commit = 0;
        self.last_commit = Instant::now();
        if let Ok(t) = serde_json::to_string(&self.manifest) {
            let _ = settings::write_atomic(&manifest_path(), t.as_bytes());
        }
        STATUS.lock().indexed = self.manifest.files.len();
    }

    fn remove(&mut self, e: &Engine, path: &str) {
        if self.manifest.files.remove(path).is_some() {
            self.writer.delete_term(Term::from_field_text(e.path, path));
            self.since_commit += 1;
        }
    }

    /// (Re)index one file if it changed since it was indexed.
    fn update(&mut self, e: &Engine, path: &Path, m: &std::fs::Metadata) {
        let key = path.to_string_lossy().into_owned();
        let stamp = (mtime_of(m), m.len());
        if self.manifest.files.get(&key) == Some(&stamp) {
            return;
        }
        let max = if extract::ext_of(path) == "pdf" { self.max_bytes.min(20 << 20) } else { self.max_bytes };
        let body = extract::text(path, max).unwrap_or_default();
        self.writer.delete_term(Term::from_field_text(e.path, &key));
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        // name words split on punctuation so "annual_report-2024" matches "annual report"
        let name_words = name.replace(['_', '-', '.'], " ");
        let _ = self.writer.add_document(doc!(e.path => key.clone(), e.name => name_words, e.body => body, e.mtime => stamp.0));
        self.manifest.files.insert(key, stamp);
        self.since_commit += 1;
        self.commit(false);
    }
}

fn wants(path: &Path, m: &std::fs::Metadata, excludes: &[String]) -> bool {
    m.is_file() && m.file_attributes() & A_CLOUD == 0 && extract::indexable(path) && !excluded(&path.to_string_lossy().to_lowercase(), excludes)
}

/// Walk every content folder: index new and changed files, drop deleted ones.
fn full_scan(e: &Engine, run: &mut Run, roots: &[PathBuf], excludes: &[String], want_names: bool) {
    STATUS.lock().phase = "Scanning your files".into();
    let data_dir = settings::DATA_DIR.to_string_lossy().to_lowercase();
    let mut seen: HashSet<String> = HashSet::with_capacity(run.manifest.files.len());
    let mut names = Vec::new();
    let mut todo: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
    for root in roots {
        let walker = walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_entry(|d| !skip_dir(d, &data_dir, excludes));
        for d in walker.flatten() {
            if REBUILD.load(Ordering::SeqCst) || RESTART.load(Ordering::SeqCst) {
                return;
            }
            let p = d.path();
            if want_names && d.depth() > 0 {
                names.push((d.file_name().to_string_lossy().to_lowercase(), p.to_string_lossy().into_owned(), d.file_type().is_dir()));
            }
            if !d.file_type().is_file() {
                continue;
            }
            let Ok(m) = d.metadata() else { continue };
            if !wants(p, &m, excludes) {
                continue;
            }
            let key = p.to_string_lossy().into_owned();
            let stamp = (mtime_of(&m), m.len());
            if run.manifest.files.get(&key) != Some(&stamp) {
                todo.push((p.to_path_buf(), m));
            }
            seen.insert(key);
        }
    }
    if want_names {
        *NAMES.write() = names;
    }
    let gone: Vec<String> = run.manifest.files.keys().filter(|k| !seen.contains(*k)).cloned().collect();
    for k in gone {
        run.remove(e, &k);
    }
    // newest first: what you worked on lately becomes searchable soonest
    todo.sort_by_key(|(_, m)| std::cmp::Reverse(mtime_of(m)));
    let total = todo.len();
    for (i, (p, m)) in todo.into_iter().enumerate() {
        if REBUILD.load(Ordering::SeqCst) || RESTART.load(Ordering::SeqCst) {
            break;
        }
        {
            let mut st = STATUS.lock();
            st.phase = format!("Reading documents ({}/{})", i + 1, total);
            st.pending = total - i;
        }
        run.update(e, &p, &m);
    }
    run.commit(true);
    let mut st = STATUS.lock();
    st.pending = 0;
    st.last_full = crate::history::now();
}

fn handle_change(e: &Engine, run: &mut Run, p: &Path, excludes: &[String]) {
    let key = p.to_string_lossy().into_owned();
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => {}
        Ok(m) => {
            let data_dir = settings::DATA_DIR.to_string_lossy().to_lowercase();
            let lower = key.to_lowercase();
            let in_skipped = lower.starts_with(&data_dir)
                || p.ancestors().skip(1).any(|a| a.file_name().map(|n| {
                    let n = n.to_string_lossy().to_lowercase();
                    SKIP_DIRS.contains(&n.as_str()) || (n.starts_with('.') && n.len() > 1)
                }).unwrap_or(false));
            if !in_skipped && wants(p, &m, excludes) {
                run.update(e, p, &m);
            } else {
                run.remove(e, &key);
            }
        }
        Err(_) => {
            // deleted: the file itself, or a folder and everything indexed inside it
            run.remove(e, &key);
            let prefix = format!("{key}\\");
            let inside: Vec<String> = run.manifest.files.keys().filter(|k| k.starts_with(&prefix)).cloned().collect();
            for k in inside {
                run.remove(e, &k);
            }
        }
    }
}

pub fn start() {
    std::thread::Builder::new()
        .name("content".into())
        .spawn(|| {
            background_priority();
            loop {
                let s = settings::load();
                STATUS.lock().enabled = s.content_search;
                if !s.content_search {
                    STATUS.lock().phase = "Off".into();
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                }
                if let Err(err) = run_once(&s) {
                    STATUS.lock().error = Some(err);
                    std::thread::sleep(Duration::from_secs(60));
                }
            }
        })
        .ok();
}

fn run_once(s: &settings::Settings) -> Result<(), String> {
    if REBUILD.swap(false, Ordering::SeqCst) {
        STATUS.lock().phase = "Starting over".into();
        if let Some(e) = ENGINE.get() {
            let mut w: IndexWriter = e.index.writer_with_num_threads(1, 30_000_000).map_err(|e| e.to_string())?;
            let _ = w.delete_all_documents();
            let _ = w.commit();
        }
        let _ = std::fs::remove_file(manifest_path());
    }
    RESTART.store(false, Ordering::SeqCst);
    let e = match ENGINE.get() {
        Some(e) => e,
        None => {
            let eng = open_engine()?;
            let _ = ENGINE.set(eng);
            ENGINE.get().unwrap()
        }
    };
    let mut manifest: Manifest = std::fs::read_to_string(manifest_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    if manifest.version != SCHEMA_VERSION {
        manifest = Manifest { version: SCHEMA_VERSION, files: HashMap::new() };
    }
    let writer: IndexWriter = e.index.writer_with_num_threads(1, 40_000_000).map_err(|e| e.to_string())?;
    let mut run = Run { writer, manifest, since_commit: 0, last_commit: Instant::now(), max_bytes: s.content_max_mb.max(1) << 20 };
    STATUS.lock().indexed = run.manifest.files.len();
    let roots: Vec<PathBuf> = s.content_roots.iter().map(PathBuf::from).filter(|p| p.is_dir()).collect();
    let excludes: Vec<String> = s.content_excludes.iter().map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect();
    let want_names = files::status().is_none();

    // watch first, so nothing that changes during the scan is missed
    let (tx, rx) = crossbeam_channel::unbounded::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let _ = tx.send(p);
            }
        }
    })
    .map_err(|e| e.to_string())?;
    for r in &roots {
        let _ = watcher.watch(r, RecursiveMode::Recursive);
    }

    full_scan(e, &mut run, &roots, &excludes, want_names);
    STATUS.lock().phase = "Up to date".into();

    let started = Instant::now();
    let mut pending: HashSet<PathBuf> = HashSet::new();
    let mut quiet_since = Instant::now();
    loop {
        if REBUILD.load(Ordering::SeqCst) || RESTART.load(Ordering::SeqCst) || !settings::load().content_search {
            run.commit(true);
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(1500)) {
            Ok(p) => {
                pending.insert(p);
                while let Ok(p) = rx.try_recv() {
                    pending.insert(p);
                }
                quiet_since = Instant::now();
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
            Err(_) => return Ok(()),
        }
        // wait for a moment of quiet (files are often written in several steps)
        if !pending.is_empty() && (quiet_since.elapsed() > Duration::from_secs(2) || pending.len() > 500) {
            STATUS.lock().phase = format!("Updating {} changed files", pending.len());
            let batch: Vec<PathBuf> = pending.drain().collect();
            for p in batch {
                handle_change(e, &mut run, &p, &excludes);
            }
            run.commit(true);
            STATUS.lock().phase = "Up to date".into();
        }
        if started.elapsed() > Duration::from_secs(4 * 3600) {
            run.commit(true);
            return Ok(()); // periodic full re-check
        }
    }
}

/// Throw the index away and read everything again.
pub fn rebuild() {
    REBUILD.store(true, Ordering::SeqCst);
}

/// Settings changed: restart with the new folders.
pub fn restart() {
    RESTART.store(true, Ordering::SeqCst);
}

/// Lowercase words, split the way the index splits them.
pub fn terms(input: &str) -> Vec<String> {
    let Some(e) = ENGINE.get() else {
        return input.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect();
    };
    let mut out = Vec::new();
    if let Ok(mut an) = e.index.tokenizer_for_field(e.body) {
        let mut ts = an.token_stream(input);
        while ts.advance() {
            out.push(ts.token().text.clone());
        }
    }
    out
}

pub fn search(input: &str, limit: usize) -> Vec<(i32, Value)> {
    let Some(e) = ENGINE.get() else { return vec![] };
    let words = terms(input);
    if words.is_empty() || words.iter().map(|w| w.chars().count()).sum::<usize>() < 3 {
        return vec![];
    }
    let n = words.len();
    let clauses: Vec<(Occur, Box<dyn TQuery>)> = words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            // the word being typed matches as a prefix
            let field_q = |f: Field| -> Box<dyn TQuery> {
                let t = Term::from_field_text(f, w);
                if i == n - 1 && w.chars().count() >= 3 {
                    Box::new(FuzzyTermQuery::new_prefix(t, 0, true))
                } else {
                    Box::new(TermQuery::new(t, IndexRecordOption::WithFreqs))
                }
            };
            let either: Box<dyn TQuery> = Box::new(BooleanQuery::new(vec![(Occur::Should, field_q(e.body)), (Occur::Should, field_q(e.name))]));
            (Occur::Must, either)
        })
        .collect();
    let q = BooleanQuery::new(clauses);
    let searcher = e.reader.searcher();
    let Ok(top) = searcher.search(&q, &TopDocs::with_limit(limit)) else { return vec![] };
    top.into_iter()
        .filter_map(|(score, addr)| {
            let d: TantivyDocument = searcher.doc(addr).ok()?;
            let path = d.get_first(e.path)?.as_str()?.to_string();
            if !Path::new(&path).exists() {
                return None;
            }
            let s = 200 + (score * 25.0) as i32;
            let p = Path::new(&path);
            Some((
                s,
                json!({
                    "key": files::key(&path), "kind": "content",
                    "title": p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                    "subtitle": p.parent().map(|d| files::pretty_dir(&d.to_string_lossy())).unwrap_or_default(),
                    "icon": icons::path_url(&path), "path": path, "terms": words, "score": s,
                }),
            ))
        })
        .collect()
}

/// Excerpt around the match, for the preview pane.
pub fn snippet(path: &str, terms: &[String]) -> Value {
    let max = settings::load().content_max_mb.max(1) << 20;
    match extract::text(Path::new(path), max) {
        Some(t) => {
            let (text, marks) = extract::snippet(&t, terms, 220);
            json!({"text": text, "marks": marks})
        }
        None => Value::Null,
    }
}

/// Name search over the walked folders, used only when the indexer service isn't running.
pub fn name_search(q: &Query) -> Vec<Hit> {
    let names = NAMES.read();
    let mut v: Vec<Hit> = names
        .iter()
        .filter(|(name, path, dir)| {
            match q.kind {
                Kind::Dir if !dir => return false,
                Kind::File if *dir => return false,
                _ => {}
            }
            if !q.exts.is_empty() && !q.exts.iter().any(|x| name.ends_with(&format!(".{x}"))) {
                return false;
            }
            let lp = path.to_lowercase();
            q.tokens.iter().all(|t| lp.contains(t.as_str())) && q.tokens.iter().any(|t| name.contains(t.as_str()))
        })
        .take(2000)
        .map(|(name, path, dir)| {
            let starts = q.tokens.iter().any(|t| name.starts_with(t.as_str()));
            Hit { path: path.clone(), dir: *dir, score: if starts { 300 } else { 150 } - (name.len() as i32).min(100), ..Default::default() }
        })
        .collect();
    v.sort_by(|a, b| b.score.cmp(&a.score));
    v.truncate(q.limit);
    v
}
