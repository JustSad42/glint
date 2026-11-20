//! The in-memory file table for one volume, and the ranking.
//!
//! Every file and folder is one 20-byte `Entry` pointing at its parent, with names packed into
//! two big strings (original case for display, lowercase for matching). A search scans the
//! lowercase names in parallel, so a few million entries take a few milliseconds.
use crate::query::{Kind, Query};
use memchr::memmem::Finder;
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

pub const ROOT: u32 = u32::MAX;

pub const F_DIR: u8 = 1;
pub const F_HIDDEN: u8 = 2;
pub const F_SYSTEM: u8 = 4;
/// inside Windows, the recycle bin, node_modules, .git...: almost never what you want
pub const F_NOISE: u8 = 8;
/// inside AppData, Program Files, ProgramData, build folders: sometimes what you want
pub const F_MINOR: u8 = 16;
/// inside C:\Users\<someone>
pub const F_USER: u8 = 32;
pub const F_DEAD: u8 = 64;
const INHERITED: u8 = F_NOISE | F_MINOR | F_USER;

const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_DIRECTORY: u32 = 0x10;

const NOISE_NAMES: &[&str] = &[
    "$recycle.bin",
    "system volume information",
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "__pycache__",
    "winsxs",
    "$windows.~ws",
    "$windows.~bt",
    "$winreagent",
    "$sysreset",
    "windows.old",
    ".gradle",
    ".cargo",
    ".rustup",
    ".nuget",
    "site-packages",
    "ebwebview",
];
const NOISE_TOP: &[&str] = &["windows", "recovery", "$getcurrent", "msocache", "config.msi", "perflogs"];
const MINOR_NAMES: &[&str] = &[
    "appdata",
    "programdata",
    "program files",
    "program files (x86)",
    "target",
    ".vs",
    ".vscode",
    ".idea",
    "obj",
    ".venv",
    "venv",
    ".cache",
    "cache",
    "temp",
    "tmp",
    "dist",
    "build",
];

#[derive(Clone, Copy, Debug)]
struct Entry {
    parent: u32,
    name_off: u32,
    lower_off: u32,
    name_len: u16,
    lower_len: u16,
    flags: u8,
}

/// One record as read from the volume.
#[derive(Clone, Debug)]
pub struct RawRec {
    pub frn: u64,
    pub parent: u64,
    pub name: String,
    pub attrs: u32,
}

pub struct VolumeIndex {
    pub letter: char,
    pub root_frn: u64,
    entries: Vec<Entry>,
    names: String,
    lower: String,
    map: HashMap<u64, u32>,
    dead: usize,
    garbage: usize,
}

/// A ranked match, before its path is built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Scored {
    pub score: i32,
    pub idx: u32,
}

fn is_boundary(b: u8) -> bool {
    matches!(b, b' ' | b'.' | b'_' | b'-' | b'(' | b'[' | b'{' | b'+' | b',' | b'@' | b'#' | b'\'')
}

impl VolumeIndex {
    pub fn new(letter: char, root_frn: u64) -> Self {
        VolumeIndex {
            letter,
            root_frn,
            entries: Vec::new(),
            names: String::new(),
            lower: String::new(),
            map: HashMap::new(),
            dead: 0,
            garbage: 0,
        }
    }

    /// Build from a full enumeration of the volume.
    pub fn from_records(letter: char, root_frn: u64, recs: Vec<RawRec>) -> Self {
        let mut b = Builder::new(letter, root_frn);
        for r in &recs {
            b.push(r);
        }
        b.finish()
    }

    /// Append a name to the string buffers: (name_off, lower_off, name_len, lower_len).
    fn store_name(&mut self, name: &str) -> (u32, u32, u16, u16) {
        let name = &name[..name.floor_char_boundary_compat(name.len().min(u16::MAX as usize))];
        let lower = name.to_lowercase();
        let lower = &lower[..lower.floor_char_boundary_compat(lower.len().min(u16::MAX as usize))];
        let r = (self.names.len() as u32, self.lower.len() as u32, name.len() as u16, lower.len() as u16);
        self.names.push_str(name);
        self.lower.push_str(lower);
        r
    }

    fn push_name(&mut self, name: &str, flags: u8) -> u32 {
        let (name_off, lower_off, name_len, lower_len) = self.store_name(name);
        self.entries.push(Entry { parent: ROOT, name_off, lower_off, name_len, lower_len, flags });
        (self.entries.len() - 1) as u32
    }

    pub fn len(&self) -> usize {
        self.entries.len() - self.dead
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Fraction of the table that is deleted or renamed-away leftovers.
    pub fn waste(&self) -> f64 {
        let total = self.entries.len().max(1) as f64;
        let bytes = self.names.len().max(1) as f64;
        (self.dead as f64 / total).max(self.garbage as f64 / bytes)
    }

    fn name(&self, i: u32) -> &str {
        let e = &self.entries[i as usize];
        &self.names[e.name_off as usize..e.name_off as usize + e.name_len as usize]
    }

    fn lower(&self, i: u32) -> &[u8] {
        let e = &self.entries[i as usize];
        &self.lower.as_bytes()[e.lower_off as usize..e.lower_off as usize + e.lower_len as usize]
    }

    pub fn is_dir(&self, i: u32) -> bool {
        self.entries[i as usize].flags & F_DIR != 0
    }

    pub fn flags(&self, i: u32) -> u8 {
        self.entries[i as usize].flags
    }

    /// Flags that come from this entry's own name and position (not from its parents).
    fn own_flags(&self, i: u32) -> u8 {
        let e = &self.entries[i as usize];
        let lower = std::str::from_utf8(self.lower(i)).unwrap_or("");
        let mut f = 0;
        if NOISE_NAMES.contains(&lower) {
            f |= F_NOISE;
        }
        if MINOR_NAMES.contains(&lower) && e.flags & F_DIR != 0 {
            f |= F_MINOR;
        }
        if e.parent == ROOT {
            if NOISE_TOP.contains(&lower) && e.flags & F_DIR != 0 {
                f |= F_NOISE;
            }
        } else {
            let p = e.parent;
            let pe = &self.entries[p as usize];
            if pe.parent == ROOT && self.lower(p) == b"users" {
                f |= F_USER;
            }
        }
        f
    }

    /// Spread NOISE / MINOR / USER from folders to everything inside them.
    fn compute_inherited(&mut self) {
        let n = self.entries.len();
        // 0 = not done, 1 = done, 2 = on the current chain (guards against corrupt cycles)
        let mut state = vec![0u8; n];
        let mut chain: Vec<u32> = Vec::new();
        for start in 0..n {
            if state[start] != 0 {
                continue;
            }
            chain.clear();
            let mut j = start as u32;
            let mut inherit = 0u8;
            loop {
                if j == ROOT {
                    break;
                }
                match state[j as usize] {
                    1 => {
                        inherit = self.entries[j as usize].flags & INHERITED;
                        break;
                    }
                    2 => break, // cycle
                    _ => {
                        state[j as usize] = 2;
                        chain.push(j);
                        j = self.entries[j as usize].parent;
                    }
                }
            }
            for &k in chain.iter().rev() {
                let own = self.own_flags(k);
                let e = &mut self.entries[k as usize];
                e.flags = (e.flags & !INHERITED) | own | inherit;
                inherit = e.flags & INHERITED;
                state[k as usize] = 1;
            }
        }
    }

    fn refresh_flags(&mut self, i: u32) {
        let parent = self.entries[i as usize].parent;
        let inherit = if parent == ROOT { 0 } else { self.entries[parent as usize].flags & INHERITED };
        let own = self.own_flags(i);
        let e = &mut self.entries[i as usize];
        e.flags = (e.flags & !INHERITED) | own | inherit;
    }

    /// A file or folder was created, renamed or moved.
    pub fn upsert(&mut self, frn: u64, parent_frn: u64, name: &str, attrs: u32) {
        if frn == self.root_frn {
            return;
        }
        let parent = if parent_frn == self.root_frn { ROOT } else { self.map.get(&parent_frn).copied().unwrap_or(ROOT) };
        let base = flags_from_attrs(attrs);
        if let Some(&i) = self.map.get(&frn) {
            if self.name(i) != name {
                self.garbage += self.entries[i as usize].name_len as usize;
                let (name_off, lower_off, name_len, lower_len) = self.store_name(name);
                let e = &mut self.entries[i as usize];
                (e.name_off, e.lower_off, e.name_len, e.lower_len) = (name_off, lower_off, name_len, lower_len);
            }
            let e = &mut self.entries[i as usize];
            e.parent = parent;
            e.flags = (e.flags & INHERITED) | base;
            self.refresh_flags(i);
        } else {
            let i = self.push_name(name, base);
            self.entries[i as usize].parent = parent;
            self.map.insert(frn, i);
            self.refresh_flags(i);
        }
    }

    /// A file or folder was deleted.
    pub fn remove(&mut self, frn: u64) {
        if let Some(i) = self.map.remove(&frn) {
            let e = &mut self.entries[i as usize];
            if e.flags & F_DEAD == 0 {
                e.flags |= F_DEAD;
                self.dead += 1;
                self.garbage += e.name_len as usize;
            }
        }
    }

    pub fn contains(&self, frn: u64) -> bool {
        self.map.contains_key(&frn)
    }

    /// Full path, like `C:\Users\me\file.txt`.
    pub fn path(&self, i: u32) -> String {
        let mut parts: Vec<&str> = Vec::with_capacity(12);
        let mut j = i;
        let mut guard = 0;
        while j != ROOT && guard < 512 {
            parts.push(self.name(j));
            j = self.entries[j as usize].parent;
            guard += 1;
        }
        let mut s = String::with_capacity(3 + parts.iter().map(|p| p.len() + 1).sum::<usize>());
        s.push(self.letter);
        s.push_str(":\\");
        for (k, p) in parts.iter().rev().enumerate() {
            if k > 0 {
                s.push('\\');
            }
            s.push_str(p);
        }
        s
    }

    fn lower_path(&self, i: u32) -> String {
        self.path(i).to_lowercase()
    }

    /// Does a parent folder's name contain `needle`?
    fn ancestor_has(&self, i: u32, needle: &Finder) -> bool {
        let mut j = self.entries[i as usize].parent;
        let mut guard = 0;
        while j != ROOT && guard < 512 {
            if needle.find(self.lower(j)).is_some() {
                return true;
            }
            j = self.entries[j as usize].parent;
            guard += 1;
        }
        false
    }

    /// The best `q.limit` matches, best first.
    pub fn search(&self, q: &Query) -> Vec<Scored> {
        let limit = q.limit.clamp(1, 1000);
        // plain words vs. words containing a backslash (matched against the whole path)
        let plain: Vec<&str> = q.tokens.iter().filter(|t| !t.contains('\\')).map(|s| s.as_str()).collect();
        let paths: Vec<&str> = q.tokens.iter().filter(|t| t.contains('\\')).map(|s| s.as_str()).collect();
        // for a path word, its last part must be in the name
        let mut name_words: Vec<String> = plain.iter().map(|s| s.to_string()).collect();
        for p in &paths {
            if let Some(last) = p.rsplit('\\').find(|s| !s.is_empty()) {
                name_words.push(last.to_string());
            }
        }
        let finders: Vec<Finder> = name_words.iter().map(|t| Finder::new(t.as_bytes()).into_owned()).collect();
        let n_plain = plain.len();
        let exts: Vec<Vec<u8>> = q.exts.iter().map(|e| format!(".{e}").into_bytes()).collect();
        let total_len: usize = name_words.iter().map(|t| t.len()).sum();

        const CHUNK: usize = 1 << 15;
        let heaps: Vec<BinaryHeap<Reverse<Scored>>> = self
            .entries
            .par_chunks(CHUNK)
            .enumerate()
            .map(|(c, chunk)| {
                let mut heap: BinaryHeap<Reverse<Scored>> = BinaryHeap::with_capacity(limit + 1);
                let mut found = vec![usize::MAX; finders.len()];
                for (k, e) in chunk.iter().enumerate() {
                    if e.flags & F_DEAD != 0 {
                        continue;
                    }
                    match q.kind {
                        Kind::Dir if e.flags & F_DIR == 0 => continue,
                        Kind::File if e.flags & F_DIR != 0 => continue,
                        _ => {}
                    }
                    let lower = &self.lower.as_bytes()[e.lower_off as usize..e.lower_off as usize + e.lower_len as usize];
                    if !exts.is_empty() && (e.flags & F_DIR != 0 || !exts.iter().any(|x| lower.ends_with(x))) {
                        continue;
                    }
                    let idx = (c * CHUNK + k) as u32;
                    let mut any = finders.is_empty();
                    for (t, f) in finders.iter().enumerate() {
                        found[t] = f.find(lower).unwrap_or(usize::MAX);
                        any |= found[t] != usize::MAX;
                    }
                    if !any {
                        continue;
                    }
                    // words missing from the name may be in a parent folder's name
                    let mut in_parent = 0;
                    let mut ok = true;
                    for t in 0..n_plain {
                        if found[t] == usize::MAX {
                            if self.ancestor_has(idx, &finders[t]) {
                                in_parent += 1;
                            } else {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if !ok {
                        continue;
                    }
                    if n_plain < finders.len() && (found[n_plain..].iter().any(|&p| p == usize::MAX)) {
                        continue;
                    }
                    if !paths.is_empty() {
                        let lp = self.lower_path(idx);
                        if !paths.iter().all(|p| lp.contains(p)) {
                            continue;
                        }
                    }
                    let score = score_entry(lower, e.flags, &name_words, &found, in_parent, total_len);
                    let s = Scored { score, idx };
                    if heap.len() < limit {
                        heap.push(Reverse(s));
                    } else if let Some(Reverse(min)) = heap.peek() {
                        if s > *min {
                            heap.pop();
                            heap.push(Reverse(s));
                        }
                    }
                }
                heap
            })
            .collect();
        let mut all: Vec<Scored> = heaps.into_iter().flat_map(|h| h.into_iter().map(|Reverse(s)| s)).collect();
        all.sort_by(|a, b| b.cmp(a));
        all.truncate(limit);
        all
    }

    /// Rebuild the string buffers without deleted or renamed-away names.
    pub fn compact(&mut self) {
        let mut names = String::with_capacity(self.names.len() - self.garbage.min(self.names.len()));
        let mut lower = String::with_capacity(self.lower.len());
        let mut remap = vec![ROOT; self.entries.len()];
        let mut entries = Vec::with_capacity(self.entries.len() - self.dead);
        for (i, e) in self.entries.iter().enumerate() {
            if e.flags & F_DEAD != 0 {
                continue;
            }
            remap[i] = entries.len() as u32;
            let mut ne = *e;
            ne.name_off = names.len() as u32;
            ne.lower_off = lower.len() as u32;
            names.push_str(&self.names[e.name_off as usize..e.name_off as usize + e.name_len as usize]);
            lower.push_str(&self.lower[e.lower_off as usize..e.lower_off as usize + e.lower_len as usize]);
            entries.push(ne);
        }
        for e in entries.iter_mut() {
            if e.parent != ROOT {
                e.parent = remap[e.parent as usize];
            }
        }
        for v in self.map.values_mut() {
            *v = remap[*v as usize];
        }
        self.map.retain(|_, v| *v != ROOT);
        self.entries = entries;
        self.names = names;
        self.lower = lower;
        self.dead = 0;
        self.garbage = 0;
    }
}

/// Builds a `VolumeIndex` one record at a time, so a full scan never holds millions of
/// temporary strings.
pub struct Builder {
    v: VolumeIndex,
    parents: Vec<u64>,
}

impl Builder {
    pub fn new(letter: char, root_frn: u64) -> Self {
        Builder { v: VolumeIndex::new(letter, root_frn), parents: Vec::new() }
    }

    pub fn push(&mut self, r: &RawRec) {
        if r.frn == self.v.root_frn || self.v.map.contains_key(&r.frn) {
            return;
        }
        let idx = self.v.push_name(&r.name, flags_from_attrs(r.attrs));
        self.v.map.insert(r.frn, idx);
        self.parents.push(r.parent);
    }

    pub fn finish(self) -> VolumeIndex {
        let Builder { mut v, parents } = self;
        let root = v.root_frn;
        for (i, p) in parents.into_iter().enumerate() {
            v.entries[i].parent = if p == root { ROOT } else { v.map.get(&p).copied().unwrap_or(ROOT) };
        }
        v.compute_inherited();
        v.entries.shrink_to_fit();
        v.names.shrink_to_fit();
        v.lower.shrink_to_fit();
        v.map.shrink_to_fit();
        v
    }
}

fn flags_from_attrs(attrs: u32) -> u8 {
    let mut f = 0;
    if attrs & ATTR_DIRECTORY != 0 {
        f |= F_DIR;
    }
    if attrs & ATTR_HIDDEN != 0 {
        f |= F_HIDDEN;
    }
    if attrs & ATTR_SYSTEM != 0 {
        f |= F_SYSTEM;
    }
    f
}

/// Higher is better. Exact names beat prefixes beat word starts beat anywhere-matches;
/// short names and your own files beat system clutter.
fn score_entry(lower: &[u8], flags: u8, words: &[String], found: &[usize], in_parent: i32, total_len: usize) -> i32 {
    let mut s: i32 = 0;
    let stem_end = if flags & F_DIR != 0 { lower.len() } else { memchr::memrchr(b'.', lower).filter(|&p| p > 0).unwrap_or(lower.len()) };
    let stem = &lower[..stem_end];
    for (t, w) in words.iter().enumerate() {
        let p = found[t];
        if p == usize::MAX {
            continue;
        }
        let w = w.as_bytes();
        s += if lower == w || stem == w {
            420
        } else if p == 0 {
            220
        } else if is_boundary(lower[p - 1]) {
            130
        } else {
            50
        };
    }
    // how much of the name the words cover
    if !lower.is_empty() && total_len > 0 {
        s += ((total_len.min(stem.len().max(1)) * 160) / stem.len().max(1)) as i32;
    }
    s += in_parent * 15;
    if flags & F_NOISE != 0 {
        s -= 700;
    }
    if flags & F_MINOR != 0 {
        s -= 140;
    }
    if flags & F_USER != 0 {
        s += 70;
    }
    if flags & (F_HIDDEN | F_SYSTEM) != 0 {
        s -= 120;
    }
    s -= (lower.len() as i32).min(200) / 3;
    s
}

trait FloorBoundary {
    fn floor_char_boundary_compat(&self, i: usize) -> usize;
}
impl FloorBoundary for str {
    fn floor_char_boundary_compat(&self, mut i: usize) -> usize {
        while i > 0 && !self.is_char_boundary(i) {
            i -= 1;
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parse;

    const ROOTF: u64 = 5;

    fn rec(frn: u64, parent: u64, name: &str, dir: bool) -> RawRec {
        RawRec { frn, parent, name: name.into(), attrs: if dir { ATTR_DIRECTORY } else { 0x20 } }
    }

    fn sample() -> VolumeIndex {
        VolumeIndex::from_records(
            'C',
            ROOTF,
            vec![
                rec(10, ROOTF, "Users", true),
                rec(11, 10, "Me", true),
                rec(12, 11, "Documents", true),
                rec(13, 12, "Budget 2024.xlsx", false),
                rec(14, 12, "notes.txt", false),
                rec(20, ROOTF, "Windows", true),
                rec(21, 20, "System32", true),
                rec(22, 21, "budget.dll", false),
                rec(30, 11, "Projects", true),
                rec(31, 30, "glint", true),
                rec(32, 31, "main.rs", false),
                rec(33, 30, "node_modules", true),
                rec(34, 33, "notes.txt", false),
            ],
        )
    }

    fn top(v: &VolumeIndex, q: &str) -> Vec<String> {
        v.search(&parse(q)).into_iter().map(|s| v.path(s.idx)).collect()
    }

    #[test]
    fn paths_and_flags() {
        let v = sample();
        let i = v.map[&13];
        assert_eq!(v.path(i), r"C:\Users\Me\Documents\Budget 2024.xlsx");
        assert!(v.flags(i) & F_USER != 0);
        assert!(v.flags(v.map[&22]) & F_NOISE != 0);
        assert!(v.flags(v.map[&34]) & F_NOISE != 0);
        assert!(v.flags(v.map[&10]) & F_USER == 0);
    }

    #[test]
    fn ranking_prefers_user_files() {
        let v = sample();
        let r = top(&v, "budget");
        assert_eq!(r[0], r"C:\Users\Me\Documents\Budget 2024.xlsx");
        assert_eq!(r.len(), 2);
        let r = top(&v, "notes");
        assert_eq!(r[0], r"C:\Users\Me\Documents\notes.txt");
    }

    #[test]
    fn words_in_parent_folders() {
        let v = sample();
        assert_eq!(top(&v, "glint main"), vec![r"C:\Users\Me\Projects\glint\main.rs".to_string()]);
        assert_eq!(top(&v, "glint\\main"), vec![r"C:\Users\Me\Projects\glint\main.rs".to_string()]);
        assert!(top(&v, "glint nothere").is_empty());
    }

    #[test]
    fn filters() {
        let v = sample();
        assert_eq!(top(&v, "*.xlsx"), vec![r"C:\Users\Me\Documents\Budget 2024.xlsx".to_string()]);
        assert_eq!(top(&v, "doc folder:"), vec![r"C:\Users\Me\Documents".to_string()]);
        assert!(top(&v, "documents file:").is_empty());
    }

    #[test]
    fn changes() {
        let mut v = sample();
        v.upsert(40, 12, "Résumé.pdf", 0x20);
        assert_eq!(top(&v, "résumé"), vec![r"C:\Users\Me\Documents\Résumé.pdf".to_string()]);
        v.upsert(40, 30, "CV.pdf", 0x20); // renamed + moved
        assert!(top(&v, "résumé").is_empty());
        assert_eq!(top(&v, "cv.pdf"), vec![r"C:\Users\Me\Projects\CV.pdf".to_string()]);
        v.remove(14);
        assert_eq!(top(&v, "notes").len(), 1);
        let before = v.len();
        v.compact();
        assert_eq!(v.len(), before);
        assert_eq!(top(&v, "cv"), vec![r"C:\Users\Me\Projects\CV.pdf".to_string()]);
        assert_eq!(top(&v, "budget")[0], r"C:\Users\Me\Documents\Budget 2024.xlsx");
    }
}
