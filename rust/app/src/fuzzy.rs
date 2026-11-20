//! Fuzzy matching for short names (apps, windows, actions).
//! Exact > prefix > word start > all words > initials ("vsc" -> Visual Studio Code)
//! > anywhere > letters in order.

/// Words of `s`: split on spaces/punctuation and camelCase humps.
pub fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for c in s.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur).to_lowercase());
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur).to_lowercase());
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        cur.push(c);
    }
    if !cur.is_empty() {
        out.push(cur.to_lowercase());
    }
    out
}

/// 0..=1000, or None if `query` doesn't match `target` at all.
pub fn score(query: &str, target: &str) -> Option<i32> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let t = target.to_lowercase();
    let len_pen = (t.chars().count() as i32 - q.chars().count() as i32).clamp(0, 60);
    if t == q {
        return Some(1000);
    }
    if t.starts_with(&q) {
        return Some(920 - len_pen);
    }
    let tw = words(target);
    if let Some(i) = tw.iter().position(|w| w.starts_with(&q)) {
        return Some(840 - (i as i32).min(6) * 12 - len_pen / 2);
    }
    let qw: Vec<&str> = q.split_whitespace().collect();
    if qw.len() > 1 {
        let mut used = vec![false; tw.len()];
        let all = qw.iter().all(|w| {
            if let Some(i) = tw.iter().enumerate().position(|(i, x)| !used[i] && x.starts_with(w)) {
                used[i] = true;
                true
            } else {
                false
            }
        });
        if all {
            return Some(800 - len_pen / 2);
        }
    }
    let q_compact: String = q.chars().filter(|c| !c.is_whitespace()).collect();
    if q_compact.chars().count() >= 2 {
        let initials: String = tw.iter().filter_map(|w| w.chars().next()).collect();
        if initials.starts_with(&q_compact) {
            return Some(780 - (initials.len() as i32 - q_compact.len() as i32).clamp(0, 10) * 6);
        }
    }
    if let Some(p) = t.find(&q) {
        return Some(620 - (p as i32).min(40) * 3 - len_pen / 3);
    }
    // letters in order, preferring tight, early, word-start matches
    let qn = q_compact.chars().count();
    if qn < 3 {
        return None;
    }
    let tc: Vec<char> = t.chars().collect();
    let orig: Vec<char> = target.chars().collect();
    let boundary = |p: usize| -> bool {
        p == 0
            || !tc[p - 1].is_alphanumeric()
            || (orig.len() == tc.len() && orig[p].is_uppercase() && orig[p - 1].is_lowercase())
    };
    let mut pos = 0usize;
    let mut first = None;
    let mut gaps = 0i32;
    let mut starts = 0i32;
    let mut runs = 0i32;
    let mut runs_at_boundary = 0i32;
    let mut last: Option<usize> = None;
    for qc in q_compact.chars() {
        let mut found = false;
        while pos < tc.len() {
            if tc[pos] == qc {
                if first.is_none() {
                    first = Some(pos);
                }
                if boundary(pos) {
                    starts += 1;
                }
                if last.map(|l| l + 1 != pos).unwrap_or(true) {
                    runs += 1;
                    if boundary(pos) {
                        runs_at_boundary += 1;
                    }
                }
                last = Some(pos);
                found = true;
                pos += 1;
                break;
            }
            if first.is_some() {
                gaps += 1;
            }
            pos += 1;
        }
        if !found {
            return None;
        }
    }
    // scattered letters ("code" in "Cloudflare One Client") are noise, not a match:
    // the letters must form at most two runs, or every run must start a word
    if starts == 0 || (runs > 2 && runs_at_boundary < runs) || gaps > qn as i32 * 3 {
        return None;
    }
    let s = 420 - gaps.min(40) * 6 - (first.unwrap_or(0) as i32).min(20) * 3 + starts * 25 - len_pen;
    Some(s.clamp(60, 560))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_of_match_kinds() {
        let exact = score("code", "Code").unwrap();
        let prefix = score("vis", "Visual Studio Code").unwrap();
        let word = score("studio", "Visual Studio Code").unwrap();
        let acro = score("vsc", "Visual Studio Code").unwrap();
        let inside = score("udio", "Visual Studio Code").unwrap();
        let loose = score("vstcd", "Visual Studio Code").unwrap_or(0);
        assert!(exact > prefix && prefix > word && word > acro && acro > inside && inside >= loose, "{exact} {prefix} {word} {acro} {inside} {loose}");
        assert!(score("xyz", "Visual Studio Code").is_none());
        assert!(score("code", "Cloudflare One Client").is_none());
        assert!(score("code", "Recovery Drive").is_none());
    }

    #[test]
    fn camel_case_and_multiword() {
        assert_eq!(words("FilePilot-v2 beta"), vec!["file", "pilot", "v2", "beta"]);
        assert!(score("pilot", "FilePilot").unwrap() >= 800);
        assert!(score("stu vis", "Visual Studio Code").unwrap() >= 780);
    }
}

/// Damerau–Levenshtein distance (adjacent swaps count as one edit), stopping early past `max`.
fn edits(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for i in 0..=n {
        d[i][0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        let mut row_min = usize::MAX;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return None;
        }
    }
    (d[n][m] <= max).then_some(d[n][m])
}

/// A forgiving match for typos ("spotfy", "dicord", "chorme"), used only when `score` finds
/// nothing. Compares the query with each word of the target, and with that word's start.
pub fn typo(query: &str, target: &str) -> Option<i32> {
    let q: Vec<char> = query.trim().to_lowercase().chars().collect();
    if q.len() < 4 || q.iter().any(|c| c.is_whitespace()) {
        return None;
    }
    let max = if q.len() >= 7 { 2 } else { 1 };
    let mut best: Option<usize> = None;
    for (i, w) in words(target).iter().enumerate() {
        let w: Vec<char> = w.chars().collect();
        let whole = edits(&q, &w, max);
        // "spotif" vs "spotify": compare with the word cut to about the query's length
        let cut = if w.len() > q.len() + 1 { edits(&q, &w[..q.len().min(w.len())], max) } else { None };
        if let Some(d) = whole.into_iter().chain(cut).min() {
            let d = d + usize::from(i > 0);
            best = Some(best.map_or(d, |b| b.min(d)));
        }
    }
    best.map(|d| 560 - d as i32 * 70)
}

#[cfg(test)]
mod typo_tests {
    use super::*;
    #[test]
    fn typos() {
        assert!(typo("spotfy", "Spotify").is_some());
        assert!(typo("dicord", "Discord").is_some());
        assert!(typo("chorme", "Google Chrome").is_some());
        assert!(typo("visula", "Visual Studio Code").is_some());
        assert!(typo("word", "Discord").is_none());
        assert!(typo("abc", "abd").is_none());
    }
}
