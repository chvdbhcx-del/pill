//! What the settings window asks of the rest of the app, as data. `command()` + `args()` are exactly the Tauri command name and
//! the camelCase argument object the JS page passes to `invoke` (see docs/native-ui-contract.md), so a native shell can route
//! each call to the same Rust function. The `Ui*` variants are effects the JS does inline (switch the language, open a page,
//! start the wizard): they have no command and are executed by the settings window itself.
use crate::settings::schema::Action;
use serde_json::{json, Map, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum BackendCall {
    // settings.json
    SettingsGet,
    SettingsSet { patch: Map<String, Value> },
    SettingsInfo,
    SettingsExport,
    SettingsImport { json_text: String },
    ProfileSave { name: String },
    ProfileApply { name: String },
    ProfileDelete { name: String },
    // window
    SettingsOpen { page: Option<String> },
    SettingsClose,
    SettingsQuit,
    // system / registry / hotkeys
    AutostartGet,
    AutostartSet { on: bool },
    HotkeySet { action: String, combo: String },
    LauncherList,
    PillMonitors,
    MediaDevices,
    MediaCaptureDevices,
    OpenLink { url: String },
    SoundPlay { tone: String },
    // clipboard
    ClipStatus,
    ClipPause { paused: bool },
    ClipWipe,
    // Everything and the AI setup page
    EverythingStatus,
    EverythingDownload,
    EverythingRunInstaller,
    AisetupCancel,
    AisetupStatus,
    AisetupWhisperInstall { model: String },
    AisetupWhisperSet { on: bool, model: Option<String> },
    AisetupWhisperRemove,
    AisetupOllamaDownload,
    AisetupOllamaRunInstaller,
    AisetupOllamaPull { model: String },
    AisetupAiUse { server: String, model: String },
    // AI chat / voice / weather
    AiConfig,
    AiSetKey { key: String },
    AiModels,
    AiTest,
    NetFeaturesStatus,
    VoiceSetKey { provider: String, key: String },
    VoiceHasKey { provider: Option<String> },
    VoiceTestMic,
    VoiceTestService,
    WeatherSearchCity { query: String, lang: String },
    WeatherSetCity { name: String, lat: f64, lon: f64 },
    WeatherGet { force: bool, tz_minutes: i32 },
    // updates / report
    UpdateCheck,
    UpdateInstall,
    ReportProblem,
    // live-event extras
    AgentsHookSnippets,
    ObsStatus,
    ObsSetPassword { password: String },
    SteamStatus,
    NotifStatus,
    NotifRequest,
    LiveplusIcsStatus,
    MailStatus,
    MailSetPassword { password: String },
    MailTest,
    // effects of the window itself
    UiSetLanguage { lang: String },
    UiRerender,
    UiShowPage { page: String },
    UiStartWizard,
    UiPickFile,
    /// Repaint every control from the current values (the JS `sync()`).
    UiSyncControls,
    /// Rebuild the sidebar (language change, wizard end).
    UiRebuildSidebar,
    /// Draw the wizard step again (language change inside the wizard).
    UiRedrawWizard,
    /// Leave the wizard's chrome (sidebar visible again) without storing the flag.
    UiEndWizardChrome,
    /// The `reduceMotion` setting changed: toggle the window's reduced-motion class.
    UiSetReduceMotion { on: bool },
    UiMinimize,
    UiToggleMaximize,
}

impl BackendCall {
    /// The Tauri command, or "" for a `Ui*` effect.
    pub fn command(&self) -> &'static str {
        use BackendCall::*;
        match self {
            SettingsGet => "settings_get",
            SettingsSet { .. } => "settings_set",
            SettingsInfo => "settings_info",
            SettingsExport => "settings_export",
            SettingsImport { .. } => "settings_import",
            ProfileSave { .. } => "profile_save",
            ProfileApply { .. } => "profile_apply",
            ProfileDelete { .. } => "profile_delete",
            SettingsOpen { .. } => "settings_open",
            SettingsClose => "settings_close",
            SettingsQuit => "settings_quit",
            AutostartGet => "autostart_get",
            AutostartSet { .. } => "autostart_set",
            HotkeySet { .. } => "hotkey_set",
            LauncherList => "launcher_list",
            PillMonitors => "pill_monitors",
            MediaDevices => "media_devices",
            MediaCaptureDevices => "media_capture_devices",
            OpenLink { .. } => "chat_open_link",
            SoundPlay { .. } => "sound_play",
            ClipStatus => "clip_status",
            ClipPause { .. } => "clip_pause",
            ClipWipe => "clip_wipe",
            EverythingStatus => "everything_status",
            EverythingDownload => "everything_download",
            EverythingRunInstaller => "everything_run_installer",
            AisetupCancel => "aisetup_cancel",
            AisetupStatus => "aisetup_status",
            AisetupWhisperInstall { .. } => "aisetup_whisper_install",
            AisetupWhisperSet { .. } => "aisetup_whisper_set",
            AisetupWhisperRemove => "aisetup_whisper_remove",
            AisetupOllamaDownload => "aisetup_ollama_download",
            AisetupOllamaRunInstaller => "aisetup_ollama_run_installer",
            AisetupOllamaPull { .. } => "aisetup_ollama_pull",
            AisetupAiUse { .. } => "aisetup_ai_use",
            AiConfig => "ai_config",
            AiSetKey { .. } => "ai_set_key",
            AiModels => "ai_models",
            AiTest => "ai_test",
            NetFeaturesStatus => "net_features_status",
            VoiceSetKey { .. } => "voice_set_key",
            VoiceHasKey { .. } => "voice_has_key",
            VoiceTestMic => "voice_test_mic",
            VoiceTestService => "voice_test_service",
            WeatherSearchCity { .. } => "weather_search_city",
            WeatherSetCity { .. } => "weather_set_city",
            WeatherGet { .. } => "weather_get",
            UpdateCheck => "update_check",
            UpdateInstall => "update_install",
            ReportProblem => "report_problem",
            AgentsHookSnippets => "agents_hook_snippets",
            ObsStatus => "obs_status",
            ObsSetPassword { .. } => "obs_set_password",
            SteamStatus => "steam_status",
            NotifStatus => "notif_status",
            NotifRequest => "notif_request",
            LiveplusIcsStatus => "liveplus_ics_status",
            MailStatus => "mail_status",
            MailSetPassword { .. } => "mail_set_password",
            MailTest => "mail_test",
            UiSetLanguage { .. } | UiRerender | UiShowPage { .. } | UiStartWizard | UiPickFile | UiSyncControls | UiRebuildSidebar | UiRedrawWizard | UiEndWizardChrome | UiSetReduceMotion { .. } | UiMinimize | UiToggleMaximize => "",
        }
    }

    pub fn is_ui_effect(&self) -> bool {
        self.command().is_empty()
    }

    /// The argument object of the `invoke` (camelCase keys, as Tauri converts them). Null when the command takes none.
    pub fn args(&self) -> Value {
        use BackendCall::*;
        match self {
            SettingsSet { patch } => json!({ "patch": patch }),
            SettingsImport { json_text } => json!({ "jsonText": json_text }),
            ProfileSave { name } | ProfileApply { name } | ProfileDelete { name } => json!({ "name": name }),
            SettingsOpen { page } => json!({ "page": page }),
            AutostartSet { on } => json!({ "on": on }),
            HotkeySet { action, combo } => json!({ "action": action, "combo": combo }),
            OpenLink { url } => json!({ "url": url }),
            SoundPlay { tone } => json!({ "tone": tone }),
            ClipPause { paused } => json!({ "paused": paused }),
            AisetupWhisperInstall { model } => json!({ "model": model }),
            AisetupWhisperSet { on, model } => json!({ "on": on, "model": model }),
            AisetupOllamaPull { model } => json!({ "model": model }),
            AisetupAiUse { server, model } => json!({ "server": server, "model": model }),
            AiSetKey { key } => json!({ "key": key }),
            VoiceSetKey { provider, key } => json!({ "provider": provider, "key": key }),
            VoiceHasKey { provider } => json!({ "provider": provider }),
            WeatherSearchCity { query, lang } => json!({ "query": query, "lang": lang }),
            WeatherSetCity { name, lat, lon } => json!({ "name": name, "lat": lat, "lon": lon }),
            WeatherGet { force, tz_minutes } => json!({ "force": force, "tz": tz_minutes }),
            ObsSetPassword { password } | MailSetPassword { password } => json!({ "password": password }),
            UiSetLanguage { lang } => json!({ "lang": lang }),
            UiShowPage { page } => json!({ "page": page }),
            UiSetReduceMotion { on } => json!({ "on": on }),
            _ => Value::Null,
        }
    }

    /// Convenience: a one-key settings patch.
    pub fn put(key: &str, value: Value) -> BackendCall {
        let mut patch = Map::new();
        patch.insert(key.to_string(), value);
        BackendCall::SettingsSet { patch }
    }
}

/// The call a stateless button action starts. Actions that need a flow (update check/install, wipe confirmation, text
/// fields that carry a value, the "later" button) return None: `flows.rs` / `model.rs` build their calls.
pub fn action_call(action: Action) -> Option<BackendCall> {
    use BackendCall as C;
    Some(match action {
        Action::RunWizard => C::UiStartWizard,
        Action::OpenPage(p) => C::UiShowPage { page: p.to_string() },
        Action::SettingsExport => C::SettingsExport,
        Action::SettingsImport => C::UiPickFile,
        Action::EverythingDownload => C::EverythingDownload,
        Action::EverythingRunInstaller => C::EverythingRunInstaller,
        Action::ReportProblem => C::ReportProblem,
        Action::PlaySignal | Action::UpdateCheck | Action::ClipWipe | Action::AiSetupLater => return None,
        Action::WeatherRefresh => C::WeatherGet { force: true, tz_minutes: 0 },
        Action::AgentsSnippets => C::AgentsHookSnippets,
        Action::AiList => C::AiModels,
        Action::AiTest => C::AiTest,
        Action::AiRemoveKey => C::AiSetKey { key: String::new() },
        Action::VoiceTestMic => C::VoiceTestMic,
        Action::VoiceTestService => C::VoiceTestService,
        Action::MailTest => C::MailTest,
        Action::NotifAllow => C::NotifRequest,
        Action::WeatherSearch | Action::AiSetKey | Action::VoiceSetKey | Action::VoiceRemoveKey | Action::ObsSetPassword | Action::MailSetPassword | Action::ProfileSave => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_and_args_match_the_js_invokes() {
        let c = BackendCall::put("size", json!(120));
        assert_eq!((c.command(), c.args()), ("settings_set", json!({"patch": {"size": 120}})));
        let c = BackendCall::HotkeySet { action: "toggle".into(), combo: "Ctrl+Alt+K".into() };
        assert_eq!((c.command(), c.args()), ("hotkey_set", json!({"action": "toggle", "combo": "Ctrl+Alt+K"})));
        assert_eq!(BackendCall::AutostartSet { on: true }.args(), json!({"on": true}));
        assert_eq!(BackendCall::SettingsImport { json_text: "{}".into() }.args(), json!({"jsonText": "{}"}));
        assert_eq!(BackendCall::WeatherGet { force: true, tz_minutes: 180 }.args(), json!({"force": true, "tz": 180}));
        assert_eq!(BackendCall::AisetupWhisperSet { on: false, model: None }.args(), json!({"on": false, "model": null}));
        assert_eq!(BackendCall::SettingsExport.args(), Value::Null);
        assert_eq!(BackendCall::OpenLink { url: "https://x".into() }.command(), "chat_open_link");
    }

    #[test]
    fn ui_effects_have_no_command() {
        for c in [BackendCall::UiRerender, BackendCall::UiStartWizard, BackendCall::UiSetLanguage { lang: "en".into() }, BackendCall::UiShowPage { page: "ai".into() }, BackendCall::UiPickFile] {
            assert!(c.is_ui_effect(), "{c:?}");
        }
        assert!(!BackendCall::SettingsGet.is_ui_effect());
    }

    #[test]
    fn stateless_buttons_map_and_stateful_ones_do_not() {
        assert_eq!(action_call(Action::EverythingDownload), Some(BackendCall::EverythingDownload));
        assert_eq!(action_call(Action::OpenPage("ai")), Some(BackendCall::UiShowPage { page: "ai".into() }));
        assert_eq!(action_call(Action::AiRemoveKey), Some(BackendCall::AiSetKey { key: String::new() }));
        assert_eq!(action_call(Action::UpdateCheck), None);
        assert_eq!(action_call(Action::ClipWipe), None);
    }
}
