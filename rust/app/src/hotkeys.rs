//! Global shortcuts for pinned/any items ("Ctrl+Alt+T" opens Terminal without opening Glint).
use crate::pins;
use serde_json::Value;
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

/// Register every saved shortcut again. Returns the ones Windows refused (taken by another app).
pub fn apply(app: &AppHandle) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut bad = Vec::new();
    for (combo, _) in pins::hotkeys() {
        match combo.parse::<Shortcut>() {
            Ok(sc) => {
                if gs.register(sc).is_err() {
                    bad.push(combo);
                }
            }
            Err(_) => bad.push(combo),
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{} is already used by another app", bad.join(", ")))
    }
}

pub fn find(sc: &Shortcut) -> Option<Value> {
    pins::hotkeys().into_iter().find(|(c, _)| c.parse::<Shortcut>().ok().as_ref() == Some(sc)).map(|(_, v)| v)
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
    order.parse::<Shortcut>().map_err(|_| format!("{order} can't be used as a shortcut"))?;
    Ok(order)
}
