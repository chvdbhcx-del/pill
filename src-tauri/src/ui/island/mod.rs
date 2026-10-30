//! The native island (flag `ui.native.pill`): the always-resident `pill` window as a `View`. See docs/native-island.md.
pub mod chrome;
pub mod companions;
pub mod core;
pub mod fonts;
pub mod gpu;
mod icons_data;
pub mod live;
pub mod panel;
pub mod rows;
pub mod shapes;
pub mod sys;
pub mod view;
mod wx_data;

pub use view::IslandView;

#[cfg(test)]
mod snapshots;

/// Helpers for the unit tests of this module tree.
#[cfg(test)]
pub mod tests_support {
    use super::core::Core;
    use super::sys::SysClock;
    use pill_ui_core::shell::types::View;

    /// A real `shell::View` (settings defaults, clock at 12:34).
    pub fn blank_view() -> View {
        let mut c = Core::new("en-US", SysClock::fixed(1_793_363_640_000, 0));
        c.start(serde_json::json!({}), 0.0);
        c.shell.view()
    }
}
