//! What a caller of the indexer may be told. The service reads every drive's file table as SYSTEM,
//! which ignores file permissions, so it must not repeat other people's file names to them.

/// Folders under the profiles folder that every user may see.
const SHARED: [&str; 4] = ["public", "default", "default user", "all users"];

/// May someone whose profile folder is `profile` (`None` if unknown) see `path`? Everything outside
/// the profiles folder (`profiles`, e.g. `C:\\Users`) is visible; inside it, their own folder and the
/// shared ones. Paths compare without case.
pub fn visible(path: &str, profiles: &str, profile: Option<&str>) -> bool {
    let path = path.to_lowercase();
    let root = format!("{}\\", profiles.trim_end_matches('\\').to_lowercase());
    let Some(inside) = path.strip_prefix(&root) else { return true };
    let first = inside.split('\\').next().unwrap_or("");
    if first.is_empty() || SHARED.contains(&first) {
        return true;
    }
    // loose files directly in the profiles folder (desktop.ini)
    if !inside.contains('\\') && first.ends_with(".ini") {
        return true;
    }
    profile.is_some_and(|p| {
        let p = p.trim_end_matches('\\').to_lowercase();
        path == p || path.starts_with(&format!("{p}\\"))
    })
}

#[cfg(test)]
mod tests {
    use super::visible;

    #[test]
    fn other_peoples_profiles_are_hidden() {
        let see = |p: &str| visible(p, r"C:\Users", Some(r"C:\Users\Ann"));
        assert!(see(r"C:\Windows\notepad.exe"));
        assert!(see(r"D:\Users\Bob\x.txt"), "another drive isn't the profiles folder");
        assert!(see(r"c:\users\ann\Documents\tax.pdf"));
        assert!(see(r"C:\Users\Ann"));
        assert!(see(r"C:\Users\Public\Music\a.mp3"));
        assert!(see(r"C:\Users"));
        assert!(!see(r"C:\Users\Bob"));
        assert!(!see(r"C:\USERS\Bob\Desktop\diary.txt"));
        assert!(!see(r"C:\Users\Annabel\x"), "a name that only starts the same");
        assert!(!visible(r"C:\Users\Ann\x", r"C:\Users", None), "unknown callers see no profile");
    }
}
