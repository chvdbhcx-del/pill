//! Creating the window: the host registration, geometry memory (`settingsWin.*`), the gate used by `settingswin.rs`, the backend facade and the
//! small workers (ticker, file picker).
use super::{Local, SettingsView, LABEL};
use crate::ui::host::{self, Geometry, HostHandle, NativeWindow, Placement, WinEvent};
use crate::ui::view::{Api, Cx, WindowId};
use glasskit::theme::Theme;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Is the native window the one to open (flag on, no `--web-ui`)?
pub fn native_enabled() -> bool {
    crate::ui::flags::native_settings()
}

static OPEN_ID: Mutex<Option<WindowId>> = Mutex::new(None);

struct FacadeApi;

impl Api for FacadeApi {
    fn call(&self, caller: &str, name: &str, args: Value) -> Result<Value, String> {
        crate::ui::api::call(name, args, caller)
    }
}

fn ensure_api() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| host::set_api(Arc::new(FacadeApi)));
}

pub fn system_lang() -> String {
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    let id = unsafe { GetUserDefaultUILanguage() };
    if id & 0x3ff == 0x19 { "ru-RU".into() } else { "en-US".into() }
}

fn num(key: &str) -> Option<f64> {
    let v = crate::settings::get_f64(key, f64::NAN);
    v.is_finite().then_some(v)
}

/// The remembered geometry (position in screen px, size in logical px), if any.
pub fn saved_geometry() -> Option<Geometry> {
    let (x, y) = (num("settingsWin.x")?, num("settingsWin.y")?);
    let (w, h) = pill_ui_core::settings::nav::initial_size(num("settingsWin.w"), num("settingsWin.h"));
    Some(Geometry { x: x as i32, y: y as i32, w: w as f32, h: h as f32, maximized: crate::settings::get_bool("settingsWin.max", false) })
}

fn save_geometry(g: &Geometry) {
    use crate::settings::set;
    set("settingsWin.max", json!(g.maximized));
    if g.maximized {
        return; // keep the last normal geometry
    }
    if g.x > -30000 && g.y > -30000 && g.w > 200.0 {
        set("settingsWin.x", json!(g.x));
        set("settingsWin.y", json!(g.y));
        set("settingsWin.w", json!(g.w.round()));
        set("settingsWin.h", json!(g.h.round()));
    }
}

fn register(host: &HostHandle) {
    let (w, h) = pill_ui_core::settings::nav::initial_size(num("settingsWin.w"), num("settingsWin.h"));
    let mut win = NativeWindow::normal(LABEL)
        .size(w as f32, h as f32)
        .min_size(700.0, 520.0)
        .title("Pill")
        .theme(Theme::settings_dark())
        .on_event(|id, ev| match ev {
            WinEvent::Created => *OPEN_ID.lock().unwrap_or_else(|e| e.into_inner()) = Some(id),
            WinEvent::Closed => {
                let mut g = OPEN_ID.lock().unwrap_or_else(|e| e.into_inner());
                if *g == Some(id) {
                    *g = None;
                }
            }
            WinEvent::Geometry(geo) => {
                save_geometry(&geo);
                if let Some(h) = host::handle() {
                    h.waker(id).post_custom(Local::Geometry { maximized: geo.maximized });
                }
            }
            _ => {}
        });
    if let Some(g) = saved_geometry() {
        win = win.placement(Placement::Restore(g));
    }
    win.register(host, LABEL, |cx: &mut Cx, args: Value| Box::new(SettingsView::new(cx, &args)));
}

/// Opens (or focuses) the native settings window on `page`.
pub fn open_native(page: Option<&str>) {
    ensure_api();
    let host = host::start();
    register(&host);
    host.open(LABEL, json!({ "page": page }));
}

/// Closes the native window if it is open.
pub fn close_native() {
    let id = *OPEN_ID.lock().unwrap_or_else(|e| e.into_inner());
    if let (Some(id), Some(h)) = (id, host::handle()) {
        h.close(id);
    }
}

/// Periodic wake-ups for status polling and the caret; runs only while something on screen needs it.
pub struct Ticker {
    stop: Arc<AtomicBool>,
}

impl SettingsView {
    pub(super) fn ensure_ticker(&mut self, cx: &Cx, period_ms: u64) {
        if self.ticker.is_some() || cx.window.0 == 0 {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (s2, w) = (stop.clone(), cx.waker());
        std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                std::thread::sleep(std::time::Duration::from_millis(period_ms));
                if s2.load(Ordering::Relaxed) {
                    break;
                }
                w.post_custom(Local::Tick);
            }
        });
        self.ticker = Some(Ticker { stop });
    }

    pub(super) fn stop_ticker(&mut self) {
        if let Some(t) = self.ticker.take() {
            t.stop.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn on_tick(&mut self, cx: &mut Cx) {
        use pill_ui_core::settings::calls::BackendCall as C;
        self.data.tick += 1;
        let t = self.data.tick;
        if self.model.current == "live" && !self.wiz.active() {
            let mut calls = Vec::new();
            if t % 2 == 0 {
                calls.push(C::ObsStatus);
            }
            if t % 3 == 0 {
                calls.extend([C::SteamStatus, C::NotifStatus, C::LiveplusIcsStatus, C::MailStatus]);
            }
            self.run(cx, calls);
        }
        cx.request_frame();
    }
}

fn pick_json_file(owner: isize) -> Result<String, String> {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED};
    use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
    use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let spec = [COMDLG_FILTERSPEC { pszName: w!("JSON"), pszSpec: w!("*.json") }];
        dlg.SetFileTypes(&spec).map_err(|e| e.to_string())?;
        dlg.Show(HWND(owner as *mut _)).map_err(|_| "cancelled".to_string())?;
        let item = dlg.GetResult().map_err(|e| e.to_string())?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH).map_err(|e| e.to_string())?;
        let path = name.to_string().map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        if meta.len() > pill_ui_core::settings::flows::MAX_IMPORT_BYTES {
            return Err("big".into());
        }
        std::fs::read_to_string(&path).map_err(|e| e.to_string())
    }
}

impl SettingsView {
    /// The settings import asks for a .json file: the system dialog runs on a worker, the text comes back as `Local::Picked`.
    pub(super) fn pick_file(&mut self, cx: &mut Cx) {
        let owner = host::handle().and_then(|h| h.hwnd(cx.window)).unwrap_or(0);
        let w = cx.waker();
        std::thread::spawn(move || w.post_custom(Local::Picked(pick_json_file(owner))));
    }
}
