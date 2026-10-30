// Windows in code, not config: the always-alive `pill` window and the command-bar window `bar`.
// They used to be declared in tauri.conf.json (app.windows); the attributes below are exactly those entries (title, url, inner size, no
// decorations, transparent, always on top, skip taskbar, not resizable, no shadow; `pill` is created unfocused, `bar` hidden). Other modules
// keep finding them by label (pill.rs, command.rs, guard.rs).
use tauri::{AppHandle, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub const PILL: &str = "pill";
pub const BAR: &str = "bar";

/// The island window: always alive, never takes focus (pill::init adds WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW once it exists).
pub fn build_pill(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, PILL, WebviewUrl::App("pill.html".into()))
        .title("pill-main")
        .inner_size(560.0, 120.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .focused(false)
        .build()
}

/// The command bar: created hidden; command.rs places, shows and focuses it.
pub fn build_bar(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    if crate::ui::bar_view::bar_is_native() {
        return Err(tauri::Error::WindowNotFound); // flag ui.native.bar: the bar is a native window, never a WebView
    }
    WebviewWindowBuilder::new(app, BAR, WebviewUrl::App("bar.html".into()))
        .title("pill-bar")
        .inner_size(560.0, 350.0)
        .visible(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .shadow(false)
        .build()
}
