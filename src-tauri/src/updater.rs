// Cloud updates from GitHub Releases (tauri-plugin-updater, minisign-signed latest.json). Owner: updates lane.
// The network is touched only when the user asks (update_check / update_install) or `updateAuto` is on (default ON: one check ~20 s after launch, installing is always a click).
use crate::ui::bus;
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{AppHandle};
use tauri_plugin_updater::{Update, UpdaterExt};

static FOUND: Mutex<Option<Update>> = Mutex::new(None);

/// -> { status: "none" | "available" | "error", version?, notes?, error? }
async fn check(app: &AppHandle) -> Value {
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => return json!({ "status": "error", "error": e.to_string() }),
    };
    match updater.check().await {
        Ok(Some(u)) => {
            let r = json!({ "status": "available", "version": u.version, "notes": u.body });
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = Some(u);
            r
        }
        Ok(None) => {
            *FOUND.lock().unwrap_or_else(|e| e.into_inner()) = None;
            json!({ "status": "none" })
        }
        Err(e) => json!({ "status": "error", "error": e.to_string() }),
    }
}

pub async fn update_check(app: AppHandle) -> Value {
    check(&app).await
}

/// Downloads the update found by the last `update_check`, installs it and restarts the app.
pub async fn update_install(app: AppHandle) -> Value {
    let Some(u) = FOUND.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return json!({ "status": "error", "error": "no update checked" }) };
    match u.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => app.restart(),
        Err(e) => json!({ "status": "error", "error": e.to_string() }),
    }
}

pub fn init(app: &tauri::App) {
    let app = app.handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(20)); // after the island is up
        if crate::settings::get_bool("updateAuto", true) {
            let r = tauri::async_runtime::block_on(check(&app));
            if r["status"] == "available" {
                let _ = bus::emit("update:available", r);
            }
        }
    });
}
