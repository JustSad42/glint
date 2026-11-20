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
