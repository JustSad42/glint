//! Global shortcuts for pinned/any items ("Ctrl+Alt+T" opens Terminal without opening Glint).
use crate::pins;
use global_hotkey::hotkey::HotKey;
use global_hotkey::GlobalHotKeyManager;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;

/// What's registered now: hotkey id -> (hotkey, item it opens)
static REGISTERED: Mutex<Option<HashMap<u32, (HotKey, Value)>>> = Mutex::new(None);

/// Register every saved shortcut again (from any thread; the UI thread does the work).
pub fn apply() -> Result<(), String> {
    crate::host::apply_hotkeys()
}

/// Register every saved shortcut again. Errors name the ones Windows refused (taken by another app).
pub fn apply_with(m: &GlobalHotKeyManager) -> Result<(), String> {
    let mut reg = REGISTERED.lock();
    if let Some(old) = reg.take() {
        let keys: Vec<HotKey> = old.values().map(|(h, _)| *h).collect();
        let _ = m.unregister_all(&keys);
    }
    let mut now = HashMap::new();
    let mut bad = Vec::new();
    for (combo, item) in pins::hotkeys() {
        match combo.parse::<HotKey>() {
            Ok(hk) => {
                if m.register(hk).is_ok() {
                    now.insert(hk.id(), (hk, item));
                } else {
                    bad.push(combo);
                }
            }
            Err(_) => bad.push(combo),
        }
    }
    *reg = Some(now);
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{} is already used by another app", bad.join(", ")))
    }
}

pub fn find(id: u32) -> Option<Value> {
    REGISTERED.lock().as_ref().and_then(|m| m.get(&id).map(|(_, v)| v.clone()))
}

/// "ctrl+alt+t" -> "Ctrl+Alt+T", rejecting combos without a modifier.
pub fn normalize(combo: &str) -> Result<String, String> {
    let parts: Vec<String> = combo.split('+').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect();
    let mut mods = Vec::new();
    let mut key = None;
    for p in parts {
        match p.to_lowercase().as_str() {
            "ctrl" | "control" => mods.push("Ctrl"),
            "alt" => mods.push("Alt"),
            "shift" => mods.push("Shift"),
            "win" | "super" | "meta" => mods.push("Super"),
            _ => key = Some(p.to_uppercase()),
        }
    }
    let key = key.ok_or("Press a key together with Ctrl, Alt or Shift")?;
    if mods.is_empty() || mods == ["Shift"] {
        return Err("Use Ctrl or Alt in the shortcut, so it doesn't get in the way of typing".into());
    }
    let mut order = ["Ctrl", "Alt", "Shift", "Super"].iter().filter(|m| mods.contains(m)).cloned().collect::<Vec<_>>().join("+");
    order.push('+');
    order.push_str(&key);
    order.parse::<HotKey>().map_err(|_| format!("{order} can't be used as a shortcut"))?;
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::normalize;
    #[test]
    fn combos() {
        assert_eq!(normalize("ctrl+alt+t").unwrap(), "Ctrl+Alt+T");
        assert_eq!(normalize("Alt+Shift+F5").unwrap(), "Alt+Shift+F5");
        assert_eq!(normalize("Ctrl+Space").unwrap(), "Ctrl+SPACE");
        assert!(normalize("Shift+A").is_err());
        assert!(normalize("A").is_err());
    }
}
