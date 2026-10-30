// The native UI: host (windows, GPU, input), views (island, tabs, bar, settings, capture), the command facade and the in-process event bus.
pub mod api;
#[allow(dead_code)] // test-only accessors
pub mod bar_view;
pub mod bus;
pub mod capture_view;
#[allow(dead_code)]
pub mod host;
pub mod native;
#[allow(dead_code)]
pub mod view;
pub mod pets;
pub mod tabs;
pub mod settings_view;
#[allow(dead_code)]
pub mod island;
