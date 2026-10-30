// Native-UI migration scaffolding: windows built in code, lazy windows, in-process event bus, feature flags.
pub mod bus;
pub mod flags;
pub mod lazy;
#[allow(dead_code)] // the native views (later phases) implement it; nothing in the app does yet
pub mod view;
pub mod windows;
