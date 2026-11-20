//! Put the search window's web view to sleep while it's hidden (WebView2 TrySuspend frees
//! memory and stops its timers) and wake it the moment Glint is shown again.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2_19, ICoreWebView2_3, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL};
use windows_core_62::Interface;

static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// bumped on every show/hide, so a pending suspend is dropped when Glint is shown again
static GEN: AtomicU64 = AtomicU64::new(0);

pub fn schedule_suspend(app: &AppHandle) {
    if !crate::settings::load().suspend_hidden {
        return;
    }
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(5));
        if GEN.load(Ordering::SeqCst) != gen {
            return;
        }
        let a2 = app.clone();
        let _ = app.run_on_main_thread(move || {
            let Some(w) = a2.get_webview_window("main") else { return };
            if w.is_visible().unwrap_or(true) || GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            let _ = w.with_webview(|wv| unsafe {
                let c = wv.controller();
                let _ = c.SetIsVisible(false);
                if let Ok(core) = c.CoreWebView2() {
                    // ask the browser process to trim what it can while nobody looks
                    if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
                        let _ = c19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW);
                    }
                    if let Ok(c3) = core.cast::<ICoreWebView2_3>() {
                        let h = webview2_com::TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
                        if c3.TrySuspend(&h).is_ok() {
                            SUSPENDED.store(true, Ordering::SeqCst);
                        }
                    }
                }
            });
        });
    });
}

/// Call before showing the window.
pub fn resume(w: &WebviewWindow) {
    GEN.fetch_add(1, Ordering::SeqCst);
    if !SUSPENDED.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = w.with_webview(|wv| unsafe {
        let c = wv.controller();
        if let Ok(core) = c.CoreWebView2() {
            if let Ok(c3) = core.cast::<ICoreWebView2_3>() {
                let _ = c3.Resume();
            }
            if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
                let _ = c19.SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL);
            }
        }
        let _ = c.SetIsVisible(true);
    });
}
