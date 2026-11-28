//! Updates come from this app's GitHub releases; the shared `app-update` crate does the work.
use crate::settings;
use serde_json::Value;

pub const REPO: &str = "JustSad42/glint";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

fn app() -> app_update::App {
    app_update::App {
        repo: REPO,
        id: "Glint",
        version: VERSION,
        state_file: settings::data_file("update_state.json"),
        uninstaller: settings::exe_dir().join("unins000.exe"),
    }
}

pub fn is_installed() -> bool {
    app().is_installed()
}

/// For the window. Uses a result less than 6 hours old unless `force`.
pub fn check(force: bool) -> Value {
    app().check(force)
}

/// Background: at most once a day, tell the user (once per version) that an update is out.
pub fn daily_check(notify: &dyn Fn(&str, &str)) {
    if settings::load().check_updates {
        app().daily_check(&|v| notify("Update available", &format!("Glint {v} is out. Open Glint settings to update.")));
    }
}

/// Downloads, verifies and starts the newest installer.
pub fn install(log: &dyn Fn(&str)) -> Result<String, String> {
    app().install(log)
}
