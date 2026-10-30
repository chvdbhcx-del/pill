// Native-UI migration scaffolding: windows built in code, lazy windows, in-process event bus, feature flags.
pub mod api;
pub mod bus;
pub mod flags;
#[allow(dead_code)] // the native window host (Wave 2); the views that run on it arrive in later phases
pub mod host;
pub mod lazy;
#[allow(dead_code)] // the native views (later phases) implement it; nothing in the app does yet
pub mod view;
pub mod windows;
pub mod pets;
pub mod tabs;
#[allow(dead_code)] // the native island (flag ui.native.pill); nothing builds it while the flag is off
pub mod island;
