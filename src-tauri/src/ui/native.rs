//! One place that brings the native UI host up. Every native surface (island, command bar, settings, capture overlay) calls [`ensure_host`]
//! before it creates its window; the first call does the work, later calls (from any thread) return the same handle.
//!
//! The work: `ui::api::init` (the facade learns the app), the bundled fonts (Medeo Sans) for all native text, the backend facade as the
//! host's `Api`, then the host thread itself. Fonts and the facade are set BEFORE the host thread starts, so no window ever sees the defaults.
use crate::ui::host::{self, HostHandle};
use crate::ui::view::Api;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use tauri::AppHandle;

/// The real backend for native views: `ui::api::call` (plain function calls into the backend modules).
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
