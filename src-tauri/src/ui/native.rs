//! One place that brings the native UI host up. Every native surface (island, command bar, settings, capture overlay) calls [`ensure_host`]
//! before it creates its window; the first call does the work, later calls (from any thread) return the same handle.
//!
//! The work: `ui::api::init` (the facade learns the app), the bundled fonts (Medeo Sans) for all native text, the backend facade as the
//! host's `Api`, then the host thread itself. Fonts and the facade are set BEFORE the host thread starts, so no window ever sees the defaults.
//! With every native flag off nothing here is called and the app is exactly the WebView one.
use crate::ui::host::{self, HostHandle};
use crate::ui::view::Api;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use tauri::AppHandle;

/// The real backend for native views: `ui::api::call` (the same functions the `#[tauri::command]` wrappers call).
pub struct FacadeApi;

impl Api for FacadeApi {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        crate::ui::api::call(name, args, caller)
    }
}

static HOST: OnceLock<HostHandle> = OnceLock::new();

/// Idempotent: facade init + fonts + `set_api` + `host::start`. Safe to call from every surface and from any thread.
pub fn ensure_host(app: &AppHandle) -> HostHandle {
    HOST.get_or_init(|| {
        crate::ui::api::init(app);
        host::set_fonts(&crate::ui::island::fonts::FONTS);
        host::set_api(Arc::new(FacadeApi));
        host::start()
    })
    .clone()
}

/// Starts the host at application start when any native surface is on, so the first window of the run does not pay for it (and so a
/// surface that only has the facade at hand later, like the settings page opener, finds a running host).
pub fn start_if_needed(app: &AppHandle) {
    if crate::ui::flags::any_native() {
        ensure_host(app);
    }
}
