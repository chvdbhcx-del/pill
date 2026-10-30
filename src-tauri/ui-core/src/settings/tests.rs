//! Schema integrity and coverage tests of the whole settings module: structure, options, ranges, references, wording in both
//! languages, `set` -> `BackendCall` for every row, and the comparison with the Rust side (`settings.rs` defaults,
//! `settingswin.rs` hotkey table) which keeps users' settings.json working: no key renamed or invented.
use super::calls::BackendCall;
use super::hotkey;
use super::model::SettingsModel;
use super::schema::{schema, Action, Commit, Control, Row};
use super::search;
use super::tr::{Lang, Tr};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashSet};

fn repr(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        o => o.to_string(),
    }
}

fn complete(t: Tr) -> bool {
    t.is_complete()
}

fn same_value(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() < 1e-9,
        _ => a == b,
    }
}

// ------------------------------------------------------------------------------------------------ structure

#[test]
fn ids_and_keys_are_unique() {
    let mut ids = HashSet::new();
    let mut keys = HashSet::new();
    for (p, r) in schema().all_rows() {
        assert!(ids.insert(r.id), "duplicate row id {} on {}", r.id, p.id);
        if r.is_stored() {
            assert!(keys.insert(r.key), "duplicate key {}", r.key);
            assert_eq!(r.id, r.key, "a stored row's id is its key");
            assert!(!r.key.contains(char::is_whitespace) && !r.key.is_empty());
        } else {
            assert!(r.id.contains('.'), "{}: ids of unstored rows are <page>.<name>", r.id);
        }
    }
    let mut pages = HashSet::new();
    for p in &schema().pages {
        assert!(pages.insert(p.id), "duplicate page {}", p.id);
    }
    assert_eq!(schema().pages.len(), 15);
}

#[test]
fn every_text_has_both_languages() {
    for p in &schema().pages {
        assert!(complete(p.title), "page {}", p.id);
        assert!(p.head.is_blank() || complete(p.head), "head {}", p.id);
        assert!(p.footnote.is_blank() || complete(p.footnote), "footnote {}", p.id);
        for g in &p.groups {
            assert!(g.title.is_blank() || complete(g.title), "group title on {}", p.id);
            assert!(g.footer.is_blank() || complete(g.footer), "group footer on {}", p.id);
            assert!(!g.rows.is_empty(), "empty group on {}", p.id);
            for r in &g.rows {
                assert!(complete(r.label), "label of {}", r.id);
                assert!(r.hint.is_blank() || complete(r.hint), "hint of {}", r.id);
                match &r.control {
                    Control::Segmented { options, .. } | Control::Select { options, .. } => {
                        for o in options {
                            assert!(complete(o.label), "option {:?} of {}", o.value, r.id);
                        }
                    }
                    Control::Button { label, .. } => assert!(complete(*label), "button of {}", r.id),
                    Control::Text { placeholder, .. } | Control::Lines { placeholder, .. } => assert!(placeholder.is_blank() || complete(*placeholder), "placeholder of {}", r.id),
                    _ => {}
                }
                for k in r.search_keywords {
                    assert!(!k.trim().is_empty() && *k == k.trim(), "keyword {k:?} of {}", r.id);
                }
            }
        }
    }
}

#[test]
fn options_are_unique_and_defaults_are_options() {
    for (_, r) in schema().all_rows() {
        let opts = match &r.control {
            Control::Segmented { options, .. } | Control::Select { options, .. } => options,
            _ => continue,
        };
        assert!(!opts.is_empty(), "{} has no options", r.id);
        let mut seen = HashSet::new();
        for o in opts {
            assert!(seen.insert(repr(&o.value)), "duplicate option {:?} in {}", o.value, r.id);
        }
        let dynamic = matches!(&r.control, Control::Segmented { dynamic, .. } | Control::Select { dynamic, .. } if *dynamic != super::schema::Dynamic::None);
        if !dynamic || r.is_stored() {
            assert!(seen.contains(&repr(&r.default)), "default {:?} of {} is not one of its options", r.default, r.id);
        }
    }
}

#[test]
fn ranges_and_steps_are_sane() {
    let mut sliders = 0;
    for (_, r) in schema().all_rows() {
        match &r.control {
            Control::Slider { min, max, step, .. } => {
                sliders += 1;
                assert!(min < max && *step > 0.0 && *step <= max - min, "{}", r.id);
                let steps = (max - min) / step;
                assert!((steps - steps.round()).abs() < 1e-9, "{}: the range is not a whole number of steps", r.id);
                let d = r.default.as_f64().unwrap_or_else(|| panic!("{} default is not a number", r.id));
                assert!(d >= *min && d <= *max, "{} default outside its range", r.id);
                let on_step = (d - min) / step;
                assert!((on_step - on_step.round()).abs() < 1e-9, "{} default is off its step", r.id);
            }
            Control::Number { min, max } => {
                assert!(min < max, "{}", r.id);
                let d = r.default.as_i64().unwrap_or_else(|| panic!("{} default is not an integer", r.id));
                assert!(d >= *min && d <= *max, "{}", r.id);
            }
            Control::Toggle => assert!(r.default.is_boolean(), "{} default must be a bool", r.id),
            Control::Time => assert!(super::model::valid_time(r.default.as_str().unwrap_or("")), "{}", r.id),
            Control::Weekdays => assert!(r.default.as_array().is_some_and(|a| a.len() == 7)),
            Control::Hotkey { action } => {
                assert_eq!(r.key, hotkey::key_of(action));
                assert!(hotkey::action(action).is_some(), "{action}");
            }
            _ => {}
        }
    }
    assert_eq!(sliders, 5, "size, opacityCol, opacityExp, offsetX, passThroughOpacity and no others");
}

#[test]
fn slider_ranges_match_the_web_window() {
    let sl = |id: &str| match &schema().row(id).unwrap().control {
        Control::Slider { min, max, step, .. } => (*min, *max, *step),
        c => panic!("{id}: {c:?}"),
    };
    assert_eq!(sl("size"), (80.0, 200.0, 5.0));
    assert_eq!(sl("opacityCol"), (0.0, 60.0, 5.0));
    assert_eq!(sl("opacityExp"), (0.0, 60.0, 5.0));
    assert_eq!(sl("offsetX"), (-50.0, 50.0, 1.0));
    assert_eq!(sl("passThroughOpacity"), (0.0, 80.0, 5.0));
}

#[test]
fn frame_rate_options_are_exactly_the_web_ones() {
    let opts = |id: &str| match &schema().row(id).unwrap().control {
        Control::Segmented { options, .. } => options.iter().map(|o| repr(&o.value)).collect::<Vec<_>>(),
        c => panic!("{c:?}"),
    };
    assert_eq!(opts("fps"), ["auto", "30", "60", "120", "144", "240"]);
    assert_eq!(opts("edgeIdleFps"), ["15", "30", "60"]);
    assert_eq!(schema().row("fps").unwrap().default, json!("auto"));
    assert_eq!(schema().row("edgeIdleFps").unwrap().default, json!(30));
    assert_eq!(opts("capture.fps"), ["30", "60"]);
    assert_eq!(opts("language"), ["auto", "ru", "en"]);
    assert_eq!(opts("theme"), ["dark", "light", "auto"]);
    assert_eq!(opts("shape"), ["notch", "capsule"]);
    assert_eq!(opts("musicPill"), ["title", "mini", "lyrics"]);
    assert_eq!(opts("sound.reminderTone"), ["chime", "bell", "beep"]);
    assert_eq!(opts("clipboard.maxItems"), ["50", "200", "500", "1000"]);
    assert_eq!(opts("clipboard.retentionDays"), ["7", "30", "90", "365"]);
    assert_eq!(opts("capture.format"), ["mp4", "gif"]);
    assert_eq!(opts("capture.encoder"), ["auto", "nvenc", "x264"]);
    assert_eq!(opts("capture.shotAction"), ["both", "copy", "save"]);
    assert_eq!(opts("weather.units"), ["metric", "imperial"]);
    assert_eq!(opts("voice.mode"), ["toggle", "hold"]);
    assert_eq!(opts("voice.language"), ["auto", "ru", "en"]);
    assert_eq!(opts("pet.side"), ["left", "right"]);
    assert_eq!(opts("pet.size"), ["s", "m", "l"]);
    assert_eq!(opts("clock24"), ["false", "true"]);
}

#[test]
fn conditions_reference_existing_keys_and_rows_stay_reachable() {
    let sc = schema();
    for (_, r) in sc.all_rows() {
        let mut keys = Vec::new();
        r.visible_if.keys(&mut keys);
        r.enabled_if.keys(&mut keys);
        for k in keys {
            let target = sc.row_for_key(k).unwrap_or_else(|| panic!("{}: condition on unknown key {k}", r.id));
            assert_ne!(target.id, r.id, "{} depends on itself", r.id);
            assert!(matches!(target.default, Value::Bool(_) | Value::String(_)), "{k}: conditions use bool or string keys");
        }
    }
    // with the defaults no row is hidden except the ones whose condition is false on purpose
    let m = SettingsModel::new("en");
    let hidden: Vec<&str> = sc.all_rows().filter(|(_, r)| !m.row_state(r).visible).map(|(_, r)| r.id).collect();
    assert!(hidden.is_empty(), "rows hidden by default: {hidden:?}");
}

#[test]
fn custom_widgets_and_actions_are_a_closed_vocabulary() {
    // the renderer must implement exactly these; adding one means adding it here and to the renderer
    let known: BTreeSet<&str> = [
        "character.reactions", "general.backup", "hotkeys.launcher", "modules.profiles", "search.everything", "ai.providers", "aisetup.intro", "aisetup.whisper", "aisetup.chat",
        "voice.providers", "weather.city", "updates.whatsnew",
    ]
    .into_iter()
    .collect();
    let mut used = BTreeSet::new();
    let mut actions = BTreeSet::new();
    for (_, r) in schema().all_rows() {
        match &r.control {
            Control::Custom(c) => {
                used.insert(*c);
            }
            Control::Button { action, .. } => {
                actions.insert(format!("{action:?}"));
            }
            Control::Text { commit: Commit::Backend(a), .. } => {
                actions.insert(format!("{a:?}"));
            }
            _ => {}
        }
    }
    assert_eq!(used, known);
    for a in ["RunWizard", "ReportProblem", "ClipWipe", "WeatherRefresh", "PlaySignal", "AgentsSnippets", "AiList", "AiTest", "UpdateCheck", "VoiceTestMic", "MailTest", "NotifAllow", "AiSetupLater"] {
        assert!(actions.contains(a), "{a} is no longer used");
    }
}

#[test]
fn secrets_and_buttons_never_own_a_settings_key() {
    for (_, r) in schema().all_rows() {
        match &r.control {
            Control::Text { secret: true, commit, kind, .. } => {
                assert!(!r.is_stored(), "{}: a secret field must not write settings.json", r.id);
                assert!(matches!(commit, Commit::Backend(_)) && *kind == super::schema::TextKind::Secret);
            }
            Control::Text { secret: false, commit, .. } => assert!(matches!(commit, Commit::Settings) && r.is_stored(), "{}", r.id),
            Control::Button { .. } => assert!(!r.is_stored() || r.key == "aiSetupDone", "{}", r.id),
            Control::BackendToggle(_) => assert!(!r.is_stored(), "{}", r.id),
            _ => {}
        }
        assert!(!super::schema::schema().row(r.id).is_none());
        let lower = r.key.to_ascii_lowercase();
        assert!(!lower.ends_with("enc") && !lower.contains("password") && !lower.contains("apikey"), "{} looks like a secret key", r.key);
    }
}

#[test]
fn icons_and_nav_cover_every_page() {
    for p in &schema().pages {
        assert!(super::nav::ICONS.contains(&p.icon), "{}", p.id);
    }
}

#[test]
fn no_row_needs_a_restart() {
    // the backend applies every patch at once; the field exists for the native shell, nothing sets it today
    assert!(schema().all_rows().all(|(_, r)| !r.restart_needed));
}

// ------------------------------------------------------------------------------------------------ rows per page

#[test]
fn rows_per_page() {
    let counts: BTreeMap<&str, usize> = schema().pages.iter().map(|p| (p.id, p.rows().count())).collect();
    eprintln!("rows per page: {counts:?}");
    let expect = [
        ("character", 5), ("general", 4), ("appearance", 18), ("behavior", 11), ("hotkeys", 17), ("modules", 14), ("live", 30), ("clipboard", 15), ("search", 2),
        ("sound", 7), ("weather", 4), ("ai", 7), ("aisetup", 6), ("voice", 11), ("updates", 5),
    ];
    for (id, n) in expect {
        assert_eq!(counts[id], n, "row count of {id}");
    }
    assert_eq!(schema().all_rows().count(), expect.iter().map(|e| e.1).sum::<usize>());
}

// ------------------------------------------------------------------------------------------------ set() -> BackendCall

fn settable(r: &Row) -> bool {
    r.is_stored() && !r.key.starts_with("hotkey.") && !matches!(r.control, Control::Button { .. })
}

#[test]
fn setting_a_rows_default_round_trips_through_settings_set() {
    let mut checked = 0;
    for (_, r) in schema().all_rows().filter(|(_, r)| settable(r)) {
        let mut m = SettingsModel::new("en");
        m.begin(Map::new(), None);
        let calls = m.try_set(r.key, r.default.clone()).unwrap_or_else(|e| panic!("{}: {:?} refused: {e:?}", r.key, r.default));
        let BackendCall::SettingsSet { patch } = &calls[0] else { panic!("{}: first call is {:?}", r.key, calls[0]) };
        assert!(patch.get(r.key).is_some_and(|v| same_value(v, &r.default)), "{}: patch {patch:?} vs default {:?}", r.key, r.default);
        assert_eq!(calls[0].command(), "settings_set");
        assert!(m.get(r.key) == m.values()[r.key] || same_value(&m.get(r.key), &r.default), "{}: applied locally", r.key);
        checked += 1;
    }
    assert!(checked >= 90, "only {checked} rows checked");
}

#[test]
fn every_option_is_accepted_and_foreign_values_are_refused() {
    for (_, r) in schema().all_rows().filter(|(_, r)| settable(r)) {
        let opts = match &r.control {
            Control::Segmented { options, dynamic: super::schema::Dynamic::None } | Control::Select { options, dynamic: super::schema::Dynamic::None } => options,
            _ => continue,
        };
        let mut m = SettingsModel::new("en");
        m.begin(Map::new(), None);
        for o in opts {
            let patch = match m.try_set(r.key, o.value.clone()).unwrap().remove(0) {
                BackendCall::SettingsSet { patch } => patch,
                c => panic!("{c:?}"),
            };
            assert_eq!(patch[r.key], o.value, "{}", r.key);
            // the same value as text (a renderer that only has strings) maps to the typed option
            let txt = json!(repr(&o.value));
            if r.key != "language" {
                assert_eq!(m.normalize(r.key, &txt).unwrap()[r.key], o.value, "{} as text", r.key);
            }
        }
        assert!(m.try_set(r.key, json!("__not_an_option__")).is_err(), "{}", r.key);
    }
}

#[test]
fn every_toggle_flips_with_one_settings_set() {
    let mut n = 0;
    for (_, r) in schema().all_rows().filter(|(_, r)| matches!(r.control, Control::Toggle)) {
        let mut m = SettingsModel::new("en");
        m.begin(Map::new(), None);
        let flipped = !r.default.as_bool().unwrap();
        assert_eq!(m.set(r.key, json!(flipped)), vec![BackendCall::put(r.key, json!(flipped))], "{}", r.key);
        assert_eq!(m.get(r.key), json!(flipped));
        assert!(m.try_set(r.key, json!(1)).is_err());
        n += 1;
    }
    assert!(n >= 50);
}

// ------------------------------------------------------------------------------------------------ against the Rust side

/// Every `json!({ ... })` literal of `settings::defaults()` merged (the function is the source of truth for the file).
fn rust_defaults() -> Map<String, Value> {
    let src = include_str!("../../../src/settings.rs");
    let start = src.find("pub(crate) fn defaults()").expect("defaults()");
    let end = start + src[start..].find("static EXISTED").expect("end of defaults()");
    let body = &src[start..end];
    let mut out = Map::new();
    let mut rest = body;
    while let Some(i) = rest.find("json!(") {
        let lit = &rest[i + "json!(".len()..];
        // match the braces of the object literal
        let (mut depth, mut in_str, mut esc, mut stop) = (0i32, false, false, 0usize);
        for (pos, c) in lit.char_indices() {
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        stop = pos + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        assert!(stop > 0, "unbalanced json! literal");
        let cleaned: String = lit[..stop].lines().map(|l| if let Some(c) = l.find("//") { if l[..c].matches('"').count() % 2 == 0 { &l[..c] } else { l } } else { l }).collect::<Vec<_>>().join("\n");
        let v: Value = serde_json::from_str(&cleaned).unwrap_or_else(|e| panic!("defaults literal: {e}\n{cleaned}"));
        out.extend(v.as_object().unwrap().clone());
        rest = &lit[stop..];
    }
    out
}

/// Stored keys the schema shows whose default does not live in `settings.rs::defaults()`: each module reads them with its own
/// default (the same value the page shows). Documented here so a new key cannot slip in unnoticed.
const UI_ONLY: &[(&str, &str)] = &[
    ("opacityCol", "pill.js S: default 0"),
    ("opacityExp", "pill.js S: default 0"),
    ("showTime", "pill.js S: default true"),
    ("showDate", "pill.js S: default false"),
    ("showTemp", "pill.js S: default true"),
    ("showWxIcon", "pill.js S: default true"),
    ("showSeconds", "pill.js S: default false"),
    ("showBattery", "pill.js S: default true"),
    ("fps", "pill.js S: default 'auto' (frame-rate cap)"),
    ("edgeIdleFps", "pill.js S: default 30 (calm-edge rate)"),
    ("everything.autostart", "command.rs: default true"),
    ("net.weather", "net.rs: opt-in, default off"),
    ("net.ai", "chat.rs: opt-in, default off"),
    ("weather.units", "net.rs: default metric"),
    ("weather.city", "net.rs: default empty (set by weather_set_city)"),
    ("sound.volumeDevice", "media.rs: default '' (same as Windows)"),
    ("sound.micDevice", "media.rs: default '' (same as Windows)"),
    ("clipboard.clickPaste", "clipboard.rs / tab-clipboard.js: default true"),
    ("clipboard.maxItems", "clipboard.rs: default 1000"),
    ("clipboard.retentionDays", "clipboard.rs: default 30"),
    ("clipboard.maxImageMb", "clipboard.rs: default 512"),
    ("clipboard.skipSecrets", "clipboard.rs: default true"),
    ("clipboard.denyList", "clipboard.rs: built-in list of password managers"),
    ("capture.folder", "capture.rs: default '' (Pictures\\Screenshots\\Pill)"),
    ("capture.videoFolder", "capture.rs: default '' (Videos\\Pill)"),
    ("live.obs.port", "obs.rs: default 4455"),
    ("ai.provider", "settings.js provId(): derived from ai.baseUrl when unset"),
    ("ai.baseUrl", "chat.rs: default ''"),
    ("ai.model", "chat.rs: default ''"),
    ("voice.enabled", "voice.rs: opt-in, default off"),
    ("voice.mode", "voice.rs: default 'toggle'"),
    ("voice.provider", "voice.rs: default 'groq'"),
    ("voice.endpoint", "voice.rs: default = the Groq URL"),
    ("voice.model", "voice.rs: default '' (the provider's model)"),
    ("voice.language", "voice.rs: default 'auto'"),
    ("voice.paste", "voice.rs: default false"),
    ("voice.autostop", "voice.rs: default false"),
    ("hotkey.chat", "settingswin::ACTIONS default"),
    ("hotkey.voice", "settingswin::ACTIONS default"),
    ("hotkey.record", "settingswin::ACTIONS default"),
    ("hotkey.shotArea", "settingswin::ACTIONS default"),
    ("hotkey.shotFull", "settingswin::ACTIONS default"),
    ("hotkey.ocr", "settingswin::ACTIONS default"),
    ("hotkey.uninstall", "JS-only row: settingswin::ACTIONS has no 'uninstall' (hotkey_set answers reason 'action')"),
    ("hotkey.pastePlain", "settingswin::ACTIONS default"),
    ("hotkey.mic", "settingswin::ACTIONS default"),
    ("hotkey.pipette", "settingswin::ACTIONS default"),
    ("hotkey.qr", "settingswin::ACTIONS default"),
    ("hotkey.passThrough", "settingswin::ACTIONS default"),
    ("hotkey.vpn", "settingswin::ACTIONS default"),
];

/// Keys of `settings.rs::defaults()` the schema has no row for, with the reason. The test lists them and fails when the
/// set changes, so a new Rust-side key has to be looked at.
const NOT_EXPOSED: &[(&str, &str)] = &[
    ("firstRunDone", "written by the wizard (calls::put), not a row"),
    ("aisetup.whisper.enabled", "managed by the AI setup page through aisetup_whisper_set"),
    ("aisetup.whisper.model", "managed by the AI setup page through aisetup_whisper_set"),
    ("aisetup.whisper.port", "managed by aisetup.rs (next free port)"),
    ("module.uninstall", "the uninstall module has no switch in the web settings (always on)"),
];

#[test]
fn every_schema_key_is_a_rust_default_or_documented_ui_only() {
    let rust = rust_defaults();
    assert!(rust.len() > 80, "parsed only {} defaults", rust.len());
    let ui_only: BTreeMap<&str, &str> = UI_ONLY.iter().copied().collect();
    assert_eq!(ui_only.len(), UI_ONLY.len(), "duplicate UI_ONLY entries");
    let mut stored: BTreeSet<&str> = BTreeSet::new();
    for (_, r) in schema().all_rows().filter(|(_, r)| r.is_stored()) {
        stored.insert(r.key);
        assert!(rust.contains_key(r.key) || ui_only.contains_key(r.key), "key {} is neither in settings.rs defaults nor documented UI-only", r.key);
        assert!(!(rust.contains_key(r.key) && ui_only.contains_key(r.key)), "{} is in settings.rs, remove it from UI_ONLY", r.key);
    }
    for k in ui_only.keys() {
        assert!(stored.contains(k), "UI_ONLY lists {k} but no row owns it any more");
    }
}

#[test]
fn schema_defaults_equal_the_rust_defaults() {
    let rust = rust_defaults();
    let mut compared = 0;
    for (_, r) in schema().all_rows().filter(|(_, r)| r.is_stored()) {
        if let Some(v) = rust.get(r.key) {
            assert!(same_value(&r.default, v), "default of {}: schema {:?} vs settings.rs {:?}", r.key, r.default, v);
            compared += 1;
        }
    }
    assert!(compared > 60, "only {compared} defaults compared");
}

#[test]
fn rust_keys_the_schema_does_not_expose() {
    let rust = rust_defaults();
    let stored: HashSet<&str> = schema().all_rows().filter(|(_, r)| r.is_stored()).map(|(_, r)| r.key).collect();
    let missing: BTreeSet<&str> = rust.keys().map(String::as_str).filter(|k| !stored.contains(k)).collect();
    eprintln!("settings.rs keys the schema does not expose: {missing:?}");
    let expected: BTreeSet<&str> = NOT_EXPOSED.iter().map(|e| e.0).collect();
    assert_eq!(missing, expected, "a key of settings.rs defaults() gained or lost its row: update NOT_EXPOSED");
}

#[test]
fn hotkey_table_matches_settingswin() {
    let src = include_str!("../../../src/settingswin.rs");
    let start = src.find("pub const ACTIONS").expect("ACTIONS");
    let end = start + src[start..].find("];").unwrap();
    let rust: BTreeMap<String, String> = src[start..end]
        .lines()
        .filter_map(|l| {
            let l = l.trim().strip_prefix("(\"")?;
            let (id, rest) = l.split_once("\", \"")?;
            let (combo, _) = rest.split_once('"')?;
            Some((id.to_string(), combo.to_string()))
        })
        .collect();
    assert_eq!(rust.len(), 15, "settingswin::ACTIONS parsed");
    for a in hotkey::ACTIONS.iter() {
        match rust.get(a.id) {
            Some(c) => assert_eq!(c, a.default, "default combo of {}", a.id),
            None => assert_eq!(a.id, "uninstall", "{} is not in settingswin::ACTIONS", a.id),
        }
    }
    for id in rust.keys() {
        assert!(hotkey::action(id).is_some(), "settingswin has {id}, the page does not show it");
    }
    // settings.rs repeats three of them as plain defaults
    let d = rust_defaults();
    for (k, id) in [("hotkey.toggle", "toggle"), ("hotkey.command", "command"), ("hotkey.clipboard", "clipboard")] {
        assert_eq!(d[k], json!(hotkey::action(id).unwrap().default));
    }
}

#[test]
fn settings_open_accepts_the_same_pages_as_settingswin() {
    let src = include_str!("../../../src/settingswin.rs");
    let line = src.lines().find(|l| l.starts_with("const PAGES:")).expect("PAGES");
    let rust: BTreeSet<&str> = line.split('"').skip(1).step_by(2).collect();
    let ours: BTreeSet<&str> = super::nav::OPENABLE.iter().copied().collect();
    assert_eq!(ours, rust);
}

#[test]
fn network_features_stay_opt_in() {
    for k in ["net.ai", "net.weather", "live.mail", "live.calendar", "live.obs", "voice.enabled", "history.mail", "capture.revealAfterSave"] {
        assert_eq!(schema().default_of(k), Some(&json!(false)), "{k} must default to off");
    }
    // the profile deny-list of settings.rs: these never travel in a profile, so they must not be plain module/live toggles by accident
    for k in ["live.mail", "live.calendar"] {
        assert!(schema().row_for_key(k).is_some());
    }
}

// ------------------------------------------------------------------------------------------------ search over the real schema

#[test]
fn every_row_is_found_by_its_own_label_in_both_languages() {
    for lang in Lang::ALL {
        let idx = search::build_index(lang);
        for (p, r) in schema().all_rows() {
            let q = r.label.get(lang);
            let hits = search::search(&idx, q, 1000);
            assert!(hits.iter().any(|e| e.row_id == r.id), "{lang:?}: '{q}' ({}) is not found on {}", r.id, p.id);
        }
    }
}

#[test]
fn keywords_make_rows_findable_in_english_and_russian() {
    let ru = search::build_index(Lang::Ru);
    let en = search::build_index(Lang::En);
    for (q, id) in [("autostart", "general.autostart"), ("автозапуск", "general.autostart"), ("fullscreen", "hideInFullscreen"), ("nvidia", "showInCapture"), ("ollama", "ai.provider"), ("webcal", "live.calendar"), ("imap", "live.mail")] {
        assert!(search::search(&ru, q, 8).iter().any(|e| e.row_id == id), "ru index: {q}");
        assert!(search::search(&en, q, 8).iter().any(|e| e.row_id == id), "en index: {q}");
    }
}

#[test]
fn every_control_family_has_feedback_or_is_documented_static() {
    for (_, r) in schema().all_rows() {
        let fb = super::anim::feedback(&r.control);
        let static_ok = matches!(r.control, Control::Text { .. } | Control::Lines { .. } | Control::Number { .. } | Control::Time | Control::Select { .. } | Control::Custom(_) | Control::Color);
        assert!(fb.is_some() || static_ok, "{}: no animation kind for {:?}", r.id, r.control);
    }
    let _ = Action::RunWizard;
}
