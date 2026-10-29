//! Controls tab: toggle tiles (Wi-Fi, Ethernet, Bluetooth + paired-device popover, dark theme) and power tiles (press twice).
//! Port of `src/tab-controls.js`.
//!
//! Backend: `sys_state`, `sys_set {key, value}` (keys `wifi`, `ethernet`, `bluetooth`, `dark`, `bluetooth_settings`; status
//! `ok | error | needs_uac`), `sys_power {action, confirmed}` (status `confirm` with `ms`). Event `sys:changed` (the state).
//! No timers while hidden: the model polls `sys_state` every 5 s only while the tab is visible and the island is shown.

use super::anim::Anim;
use super::{bool_of, str_of, BackendCall, Cfg, Fx, FxQueue, Tr};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const POLL_MS: f64 = 5000.0;
/// A Wi-Fi / Ethernet toggle waiting for UAC gives up after this long and re-reads the state.
pub const UAC_WAIT_MS: f64 = 15_000.0;
/// How long a short error text replaces a tile's state line.
pub const FLASH_MS: f64 = 2500.0;
/// Default window for "press again" when the backend does not say (`CONFIRM_MS` is 3000 in `system.rs`).
pub const CONFIRM_MS: f64 = 3000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Nic {
    pub available: bool,
    pub on: bool,
    pub ssid: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BtDevice {
    pub name: String,
    pub connected: bool,
    pub battery: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SysState {
    pub ethernet: Nic,
    pub wifi: Nic,
    pub bt_available: bool,
    pub bt_on: bool,
    pub bt_devices: Vec<BtDevice>,
    pub dark: bool,
}

fn nic(v: &Value) -> Nic {
    Nic { available: bool_of(v, "available"), on: bool_of(v, "on"), ssid: v.get("ssid").and_then(Value::as_str).map(str::to_string) }
}

impl SysState {
    /// `None` when the payload has no `bluetooth` object (the page ignores such answers).
    pub fn from_json(v: &Value) -> Option<SysState> {
        let b = v.get("bluetooth").filter(|b| b.is_object())?;
        Some(SysState {
            ethernet: nic(v.get("ethernet").unwrap_or(&Value::Null)),
            wifi: nic(v.get("wifi").unwrap_or(&Value::Null)),
            bt_available: bool_of(b, "available"),
            bt_on: bool_of(b, "on"),
            bt_devices: b
                .get("devices")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|d| BtDevice { name: str_of(d, "name").to_string(), connected: bool_of(d, "connected"), battery: d.get("battery").and_then(Value::as_i64) }).collect())
                .unwrap_or_default(),
            dark: bool_of(v, "dark"),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// A toggle tile by id: `wifi`, `ethernet`, `bluetooth`, `dark`.
    Toggle(String),
    /// A power tile by action: `lock`, `sleep`, `restart`, `shutdown`.
    Power(String),
    /// The little list button on the Bluetooth tile.
    TogglePopover,
    /// Outside click / Esc.
    ClosePopover,
    /// "Connect in Windows settings" in the popover.
    OpenBtSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TileView {
    pub id: String,
    pub name: String,
    /// `wifi`, `eth`, `bt`, `btoff`, `theme`
    pub icon: &'static str,
    pub state: String,
    /// Lit (`on` and not disabled).
    pub on: bool,
    pub disabled: bool,
    pub busy: bool,
    /// `aria-pressed`
    pub pressed: bool,
    /// The Bluetooth tile carries the device-list button.
    pub list_button: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PowerView {
    pub action: &'static str,
    pub icon: &'static str,
    pub label: String,
    pub armed: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceRow {
    pub name: String,
    pub connected: bool,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Popover {
    pub title: String,
    pub rows: Vec<DeviceRow>,
    /// Shown instead of rows when there are none.
    pub empty: Option<String>,
    pub more: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct View {
    pub tiles: Vec<TileView>,
    pub power: Vec<PowerView>,
    pub popover: Option<Popover>,
}

const POWER: [(&str, &str, &str, &str); 4] =
    [("lock", "lock", "Блокировка", "Lock"), ("sleep", "sleep", "Сон", "Sleep"), ("restart", "restart", "Перезагрузка", "Restart"), ("shutdown", "power", "Выключение", "Shut down")];

pub struct Controls {
    cfg: Cfg,
    mono: f64,
    s: Option<SysState>,
    visible: bool,
    rust_hidden: bool,
    poll: Option<f64>,
    busy: HashMap<String, String>,
    flash: HashMap<String, (String, f64)>,
    /// tile id -> time (ms) when a UAC wait gives up
    uac_until: HashMap<String, f64>,
    armed: Option<String>,
    arm_until: Option<f64>,
    pop: bool,
    ru: bool,
    fx: FxQueue,
}

impl Default for Controls {
    fn default() -> Self {
        Controls::new()
    }
}

impl Controls {
    pub fn new() -> Controls {
        Controls {
            cfg: Cfg::new(),
            mono: 0.0,
            s: None,
            visible: false,
            rust_hidden: false,
            poll: None,
            busy: HashMap::new(),
            flash: HashMap::new(),
            uac_until: HashMap::new(),
            armed: None,
            arm_until: None,
            pop: false,
            ru: false,
            fx: FxQueue::default(),
        }
    }

    pub fn set_settings(&mut self, v: &Value) {
        self.cfg.apply(v);
    }
    pub fn set_os_reduce_motion(&mut self, on: bool) {
        self.cfg.os_reduce = on;
    }
    /// The language of the flash texts written into state lines (`UAC declined` ...), which are produced when an error arrives.
    pub fn set_language_ru(&mut self, ru: bool) {
        self.ru = ru;
    }
    pub fn take_fx(&mut self) -> Vec<Fx> {
        self.fx.take()
    }
    pub fn state(&self) -> Option<&SysState> {
        self.s.as_ref()
    }
    pub fn popover_open(&self) -> bool {
        self.pop
    }
    /// Esc closes the popover first.
    pub fn wants_escape(&self) -> bool {
        self.pop
    }
    pub fn armed(&self) -> Option<&str> {
        self.armed.as_deref()
    }
    fn reduce(&self) -> bool {
        self.cfg.reduce_motion()
    }

    fn apply(&mut self, v: &Value) {
        if let Some(s) = SysState::from_json(v) {
            self.s = Some(s);
            if !self.s.as_ref().map_or(false, |s| s.bt_available) {
                self.close_pop();
            }
        }
    }

    fn refresh(&self) -> Vec<BackendCall> {
        vec![BackendCall::bare("sys_state")]
    }

    fn close_pop(&mut self) {
        if self.pop {
            self.pop = false;
            self.fx.anim(self.reduce(), "bt-pop", Anim::popover_out());
        }
    }

    fn note(&mut self, id: &str, text: String) {
        self.flash.insert(id.to_string(), (text, self.mono + FLASH_MS));
    }

    // ---------------------------------------------------------------- lifecycle and events
    pub fn on_show(&mut self) -> Vec<BackendCall> {
        self.visible = true;
        self.start_poll();
        self.refresh()
    }

    pub fn on_hide(&mut self) {
        self.visible = false;
        self.poll = None;
        self.close_pop();
        self.armed = None;
        self.arm_until = None;
    }

    fn start_poll(&mut self) {
        self.poll = if self.visible && !self.rust_hidden { Some(0.0) } else { None };
    }

    pub fn on_event(&mut self, name: &str, payload: &Value) -> Vec<BackendCall> {
        match name {
            "sys:changed" => {
                self.busy.clear();
                self.uac_until.clear();
                self.apply(payload);
                vec![]
            }
            "pill:hide" => {
                self.rust_hidden = true;
                self.poll = None;
                vec![]
            }
            "pill:show" => {
                self.rust_hidden = false;
                if self.visible {
                    self.start_poll();
                    return self.refresh();
                }
                vec![]
            }
            "settings:changed" => {
                self.cfg.apply(payload);
                vec![]
            }
            _ => vec![],
        }
    }

    pub fn on_action(&mut self, a: Action) -> Vec<BackendCall> {
        match a {
            Action::Toggle(id) => self.toggle(&id),
            Action::Power(a) => self.power(&a),
            Action::TogglePopover => {
                if self.pop {
                    self.close_pop();
                } else if self.s.as_ref().map_or(false, |s| s.bt_available) {
                    self.pop = true;
                    self.fx.anim(self.reduce(), "bt-pop", Anim::popover_in());
                }
                vec![]
            }
            Action::ClosePopover => {
                self.close_pop();
                vec![]
            }
            Action::OpenBtSettings => {
                self.close_pop();
                vec![BackendCall::new("sys_set", json!({ "key": "bluetooth_settings", "value": true }))]
            }
        }
    }

    fn toggle(&mut self, id: &str) -> Vec<BackendCall> {
        let Some(s) = self.s.as_mut() else { return vec![] };
        if self.busy.contains_key(id) {
            return vec![]; // the tile is disabled while busy
        }
        let want = match id {
            "wifi" => !s.wifi.on,
            "ethernet" => !s.ethernet.on,
            "bluetooth" => !s.bt_on,
            "dark" => !s.dark,
            _ => return vec![],
        };
        if id == "ethernet" || id == "wifi" {
            // adapter admin state needs UAC (the app never elevates): no optimistic flip, wait for the result
            self.busy.insert(id.to_string(), if self.ru { "Подтвердите в UAC…".into() } else { "Confirm in UAC…".into() });
            self.fx.anim(self.reduce(), format!("tile:{id}"), Anim::busy_pulse());
        } else if id == "bluetooth" {
            s.bt_on = want; // instant feedback, then reconcile with the real state
        } else {
            s.dark = want;
        }
        vec![BackendCall::new("sys_set", json!({ "key": id, "value": want }))]
    }

    fn power(&mut self, a: &str) -> Vec<BackendCall> {
        self.arm_until = None;
        let confirmed = self.armed.as_deref() == Some(a);
        self.armed = if confirmed { None } else { Some(a.to_string()) };
        if self.armed.is_some() {
            self.fx.anim(self.reduce(), format!("power:{a}"), Anim::arm_countdown());
        }
        vec![BackendCall::new("sys_power", json!({ "action": a, "confirmed": confirmed }))]
    }

    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value) -> Vec<BackendCall> {
        match cmd {
            "sys_state" => {
                self.apply(res);
                vec![]
            }
            "sys_set" => {
                let key = str_of(args, "key").to_string();
                let status = res.get("status").and_then(Value::as_str).unwrap_or("");
                match key.as_str() {
                    "wifi" | "ethernet" => {
                        if status == "needs_uac" {
                            self.uac_until.insert(key, self.mono + UAC_WAIT_MS); // sys:changed arrives when netsh ends
                            return vec![];
                        }
                        self.busy.remove(&key);
                        if res.is_null() || status == "error" {
                            let uac = res.get("message").and_then(Value::as_str).map_or(false, |m| m.contains("UAC"));
                            let t = match (uac, self.ru) {
                                (true, true) => "UAC отклонён",
                                (true, false) => "UAC declined",
                                (false, true) => "Не удалось",
                                (false, false) => "Failed",
                            };
                            self.note(&key, t.to_string());
                        }
                        self.refresh()
                    }
                    "bluetooth" | "dark" => {
                        if res.is_null() || status == "error" {
                            self.note(&key, if self.ru { "Не удалось".into() } else { "Failed".into() });
                        }
                        self.refresh()
                    }
                    _ => vec![],
                }
            }
            "sys_power" => {
                let a = str_of(args, "action").to_string();
                if res.get("status").and_then(Value::as_str) == Some("confirm") {
                    let ms = res.get("ms").and_then(Value::as_f64).filter(|m| *m > 0.0).unwrap_or(CONFIRM_MS);
                    self.armed = Some(a);
                    self.arm_until = Some(self.mono + ms + 50.0);
                } else {
                    self.armed = None;
                }
                vec![]
            }
            _ => vec![],
        }
    }

    /// Timers: the 5 s poll, the UAC give-up, the armed window, flash expiry.
    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        let mut calls = vec![];
        if let Some(p) = self.poll.as_mut() {
            *p += dt_ms;
            if *p >= POLL_MS {
                *p = 0.0;
                calls.push(BackendCall::bare("sys_state"));
            }
        }
        let due: Vec<String> = self.uac_until.iter().filter(|(_, at)| **at <= self.mono).map(|(k, _)| k.clone()).collect();
        for k in due {
            self.uac_until.remove(&k);
            self.busy.remove(&k);
            calls.push(BackendCall::bare("sys_state"));
        }
        if self.arm_until.map_or(false, |at| at <= self.mono) {
            self.arm_until = None;
            self.armed = None;
        }
        let mono = self.mono;
        self.flash.retain(|_, (_, until)| *until > mono);
        calls
    }

    // ---------------------------------------------------------------- view
    fn state_text(&self, tr: &dyn Tr, id: &str, on: bool, disabled: bool, ssid: &Option<String>, fem: bool) -> String {
        if let Some((t, until)) = self.flash.get(id) {
            if *until > self.mono {
                return t.clone();
            }
        }
        if let Some(t) = self.busy.get(id) {
            return t.clone();
        }
        if disabled {
            return tr.t("Нет адаптера", "No adapter");
        }
        if on {
            if let Some(s) = ssid.as_ref().filter(|s| !s.is_empty()) {
                return s.clone();
            }
        }
        match (fem, on) {
            (true, true) => tr.t("Включена", "On"),
            (true, false) => tr.t("Выключена", "Off"),
            (false, true) => tr.t("Включён", "On"),
            (false, false) => tr.t("Выключен", "Off"),
        }
    }

    pub fn view(&self, tr: &dyn Tr) -> View {
        let mut tiles = vec![];
        if let Some(s) = &self.s {
            let mut push = |id: &str, name: String, icon: &'static str, on: bool, disabled: bool, ssid: &Option<String>, fem: bool, list: bool| {
                let busy = self.busy.contains_key(id);
                tiles.push(TileView {
                    id: id.to_string(),
                    name,
                    icon,
                    state: self.state_text(tr, id, on, disabled, ssid, fem),
                    on: on && !disabled,
                    disabled: disabled || busy,
                    busy,
                    pressed: on,
                    list_button: if list { Some(tr.t("Подключённые устройства", "Paired devices")) } else { None },
                });
            };
            if s.wifi.available {
                push("wifi", "Wi-Fi".into(), "wifi", s.wifi.on, !s.wifi.available, &s.wifi.ssid, false, false);
            }
            if s.ethernet.available {
                push("ethernet", "Ethernet".into(), "eth", s.ethernet.on, !s.ethernet.available, &s.ethernet.ssid, false, false);
            }
            if !s.wifi.available && !s.ethernet.available {
                push("wifi", "Wi-Fi".into(), "wifi", s.wifi.on, !s.wifi.available, &s.wifi.ssid, false, false);
            }
            push("bluetooth", "Bluetooth".into(), if s.bt_on { "bt" } else { "btoff" }, s.bt_on, !s.bt_available, &None, false, true);
            push("dark", tr.t("Тёмная тема", "Dark theme"), "theme", s.dark, false, &None, true, false);
        }
        let popover = if self.pop {
            self.s.as_ref().map(|s| Popover {
                title: tr.t("Устройства Bluetooth", "Bluetooth devices"),
                rows: s
                    .bt_devices
                    .iter()
                    .map(|d| DeviceRow {
                        name: d.name.clone(),
                        connected: d.connected,
                        detail: if d.connected { d.battery.map(|b| format!("{b}%")).unwrap_or_else(|| tr.t("Подключено", "Connected")) } else { String::new() },
                    })
                    .collect(),
                empty: if s.bt_devices.is_empty() { Some(if s.bt_on { tr.t("Нет сопряжённых устройств", "No paired devices") } else { tr.t("Bluetooth выключен", "Bluetooth is off") }) } else { None },
                more: tr.t("Подключить в настройках Windows", "Connect in Windows settings"),
            })
        } else {
            None
        };
        View {
            tiles,
            power: POWER
                .iter()
                .map(|(a, icon, ru, en)| {
                    let armed = self.armed.as_deref() == Some(*a);
                    PowerView { action: a, icon, label: if armed { tr.t("Нажмите ещё раз", "Press again") } else { tr.t(ru, en) }, armed }
                })
                .collect(),
            popover,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panel::a::{EnTr, RuTr};

    fn state(wifi: bool, eth_avail: bool, bt_avail: bool) -> Value {
        json!({
            "ethernet": { "available": eth_avail, "on": true },
            "wifi": { "available": wifi, "on": true, "ssid": "Home" },
            "bluetooth": { "available": bt_avail, "on": true, "devices": [{ "name": "Buds", "connected": true, "battery": 80 }, { "name": "Mouse", "connected": true }, { "name": "Old", "connected": false }] },
            "dark": true, "volume": 40, "muted": false
        })
    }

    fn shown() -> Controls {
        let mut c = Controls::new();
        c.on_show();
        c.on_result("sys_state", &Value::Null, &state(true, true, true));
        c.take_fx();
        c
    }

    #[test]
    fn show_refreshes_and_builds_all_tiles() {
        let mut c = Controls::new();
        assert_eq!(c.on_show(), vec![BackendCall::bare("sys_state")]);
        c.on_result("sys_state", &Value::Null, &state(true, true, true));
        let v = c.view(&EnTr);
        let ids: Vec<_> = v.tiles.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["wifi", "ethernet", "bluetooth", "dark"]);
        assert_eq!(v.tiles[0].state, "Home"); // ssid replaces "On"
        assert_eq!(v.tiles[1].state, "On");
        assert_eq!(v.tiles[3].state, "On");
        assert_eq!(c.view(&RuTr).tiles[3].state, "Включена"); // the dark-theme tile is feminine
        assert_eq!(c.view(&RuTr).tiles[1].state, "Включён");
        assert_eq!(v.tiles[2].list_button.as_deref(), Some("Paired devices"));
        assert_eq!(v.tiles[2].icon, "bt");
        assert_eq!(v.power.iter().map(|p| p.action).collect::<Vec<_>>(), ["lock", "sleep", "restart", "shutdown"]);
    }

    #[test]
    fn no_adapters_still_shows_a_disabled_wifi_tile() {
        let mut c = Controls::new();
        c.on_result("sys_state", &Value::Null, &state(false, false, false));
        let v = c.view(&EnTr);
        assert_eq!(v.tiles.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["wifi", "bluetooth", "dark"]);
        assert!(v.tiles[0].disabled && !v.tiles[0].on);
        assert_eq!(v.tiles[0].state, "No adapter");
        assert_eq!(c.view(&RuTr).tiles[1].state, "Нет адаптера");
    }

    #[test]
    fn answers_without_bluetooth_are_ignored() {
        let mut c = shown();
        c.on_result("sys_state", &Value::Null, &Value::Null);
        c.on_result("sys_state", &Value::Null, &json!({ "wifi": {} }));
        assert_eq!(c.view(&EnTr).tiles.len(), 4);
    }

    #[test]
    fn dark_and_bluetooth_flip_at_once_then_reconcile() {
        let mut c = shown();
        let call = c.on_action(Action::Toggle("dark".into()));
        assert_eq!(call, vec![BackendCall::new("sys_set", json!({ "key": "dark", "value": false }))]);
        assert!(!c.state().unwrap().dark);
        let next = c.on_result("sys_set", &json!({ "key": "dark" }), &json!({ "status": "ok" }));
        assert_eq!(next, vec![BackendCall::bare("sys_state")]);
        assert_eq!(c.on_action(Action::Toggle("bluetooth".into()))[0].args, json!({ "key": "bluetooth", "value": false }));
        assert!(!c.state().unwrap().bt_on);
        assert_eq!(c.view(&EnTr).tiles[2].icon, "btoff");
    }

    #[test]
    fn failed_toggle_flashes_text_for_2_5_seconds() {
        let mut c = shown();
        c.on_action(Action::Toggle("dark".into()));
        c.on_result("sys_set", &json!({ "key": "dark" }), &json!({ "status": "error", "message": "x" }));
        assert_eq!(c.view(&EnTr).tiles[3].state, "Failed");
        c.tick(2400.0);
        assert_eq!(c.view(&EnTr).tiles[3].state, "Failed");
        c.tick(200.0);
        assert_ne!(c.view(&EnTr).tiles[3].state, "Failed");
        // no answer at all counts as a failure too
        c.on_action(Action::Toggle("dark".into()));
        c.on_result("sys_set", &json!({ "key": "dark" }), &Value::Null);
        assert_eq!(c.view(&EnTr).tiles[3].state, "Failed");
    }

    #[test]
    fn network_toggles_wait_for_uac_without_flipping() {
        let mut c = shown();
        let call = c.on_action(Action::Toggle("wifi".into()));
        assert_eq!(call, vec![BackendCall::new("sys_set", json!({ "key": "wifi", "value": false }))]);
        assert!(c.state().unwrap().wifi.on); // not flipped
        let t = &c.view(&EnTr).tiles[0];
        assert!(t.busy && t.disabled);
        assert_eq!(t.state, "Confirm in UAC…");
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, .. } if target == "tile:wifi")));
        // a second click while busy does nothing
        assert!(c.on_action(Action::Toggle("wifi".into())).is_empty());
        assert!(c.on_result("sys_set", &json!({ "key": "wifi" }), &json!({ "status": "needs_uac" })).is_empty());
        // no sys:changed for 15 s: give up and re-read
        c.tick(14_000.0); // (the 5 s poll fires once in here)
        let calls = c.tick(1100.0);
        assert!(calls.contains(&BackendCall::bare("sys_state")));
        assert!(!c.view(&EnTr).tiles[0].busy);
    }

    #[test]
    fn sys_changed_clears_busy_and_applies() {
        let mut c = shown();
        c.on_action(Action::Toggle("ethernet".into()));
        c.on_result("sys_set", &json!({ "key": "ethernet" }), &json!({ "status": "needs_uac" }));
        let mut s = state(true, true, true);
        s["ethernet"]["on"] = json!(false);
        c.on_event("sys:changed", &s);
        let t = &c.view(&EnTr).tiles[1];
        assert!(!t.busy && !t.on);
        assert_eq!(t.state, "Off");
        // and the stale UAC timeout no longer fires
        assert_eq!(c.tick(20_000.0).len(), 1); // only the 5 s poll
    }

    #[test]
    fn declined_uac_is_named() {
        let mut c = shown();
        c.set_language_ru(true);
        c.on_action(Action::Toggle("wifi".into()));
        let next = c.on_result("sys_set", &json!({ "key": "wifi" }), &json!({ "status": "error", "message": "UAC was cancelled" }));
        assert_eq!(next, vec![BackendCall::bare("sys_state")]);
        assert_eq!(c.view(&RuTr).tiles[0].state, "UAC отклонён");
        c.on_action(Action::Toggle("wifi".into()));
        c.on_result("sys_set", &json!({ "key": "wifi" }), &json!({ "status": "error", "message": "netsh failed" }));
        assert_eq!(c.view(&RuTr).tiles[0].state, "Не удалось");
    }

    #[test]
    fn power_tiles_need_two_presses_within_the_window() {
        let mut c = shown();
        let first = c.on_action(Action::Power("shutdown".into()));
        assert_eq!(first, vec![BackendCall::new("sys_power", json!({ "action": "shutdown", "confirmed": false }))]);
        assert_eq!(c.armed(), Some("shutdown"));
        assert_eq!(c.view(&EnTr).power[3].label, "Press again");
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "power:shutdown" && anim.dur_ms == 3000)));
        c.on_result("sys_power", &json!({ "action": "shutdown" }), &json!({ "status": "confirm", "ms": 3000 }));
        assert!(c.armed().is_some());
        c.tick(3000.0);
        assert!(c.armed().is_some()); // 3050 ms window
        c.tick(60.0);
        assert!(c.armed().is_none());
        assert_eq!(c.view(&RuTr).power[3].label, "Выключение");
        // second press inside the window confirms and disarms
        c.on_action(Action::Power("lock".into()));
        c.on_result("sys_power", &json!({ "action": "lock" }), &json!({ "status": "confirm", "ms": 3000 }));
        let second = c.on_action(Action::Power("lock".into()));
        assert_eq!(second[0].args, json!({ "action": "lock", "confirmed": true }));
        c.on_result("sys_power", &json!({ "action": "lock" }), &json!({ "status": "ok" }));
        assert!(c.armed().is_none());
        // pressing another tile moves the arm
        c.on_action(Action::Power("sleep".into()));
        c.on_action(Action::Power("restart".into()));
        assert_eq!(c.armed(), Some("restart"));
    }

    #[test]
    fn popover_lists_devices_and_closes_with_an_exit() {
        let mut c = shown();
        assert!(c.view(&EnTr).popover.is_none());
        c.on_action(Action::TogglePopover);
        assert!(c.wants_escape());
        let p = c.view(&EnTr).popover.unwrap();
        assert_eq!(p.title, "Bluetooth devices");
        assert_eq!(p.rows.iter().map(|r| r.detail.as_str()).collect::<Vec<_>>(), ["80%", "Connected", ""]);
        assert!(p.empty.is_none());
        let fx = c.take_fx();
        assert!(fx.iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "bt-pop" && anim.dur_ms == 400)));
        c.on_action(Action::TogglePopover);
        assert!(!c.popover_open());
        assert!(c.take_fx().iter().any(|f| matches!(f, Fx::Anim { target, anim } if target == "bt-pop" && anim.dur_ms == 160)));
        // "connect in settings" closes it and asks the backend
        c.on_action(Action::TogglePopover);
        assert_eq!(c.on_action(Action::OpenBtSettings), vec![BackendCall::new("sys_set", json!({ "key": "bluetooth_settings", "value": true }))]);
        assert!(!c.popover_open());
    }

    #[test]
    fn popover_empty_texts_and_unavailable_adapter() {
        let mut c = Controls::new();
        let mut s = state(true, true, true);
        s["bluetooth"]["devices"] = json!([]);
        c.on_result("sys_state", &Value::Null, &s);
        c.on_action(Action::TogglePopover);
        assert_eq!(c.view(&EnTr).popover.unwrap().empty.as_deref(), Some("No paired devices"));
        s["bluetooth"]["on"] = json!(false);
        c.on_result("sys_state", &Value::Null, &s);
        assert_eq!(c.view(&RuTr).popover.unwrap().empty.as_deref(), Some("Bluetooth выключен"));
        // the adapter disappears: the popover closes by itself
        s["bluetooth"]["available"] = json!(false);
        c.on_result("sys_state", &Value::Null, &s);
        assert!(!c.popover_open());
        assert!(c.on_action(Action::TogglePopover).is_empty() && !c.popover_open());
    }

    #[test]
    fn polls_every_five_seconds_only_while_visible_and_shown() {
        let mut c = shown();
        assert!(c.tick(4900.0).is_empty());
        assert_eq!(c.tick(200.0), vec![BackendCall::bare("sys_state")]);
        assert_eq!(c.tick(5000.0).len(), 1);
        c.on_event("pill:hide", &Value::Null);
        assert!(c.tick(60_000.0).is_empty());
        assert_eq!(c.on_event("pill:show", &Value::Null), vec![BackendCall::bare("sys_state")]);
        assert_eq!(c.tick(5000.0).len(), 1);
        c.on_hide();
        assert!(c.tick(60_000.0).is_empty());
        assert!(c.on_event("pill:show", &Value::Null).is_empty()); // tab not visible: nothing to refresh
    }

    #[test]
    fn leaving_the_tab_disarms_and_closes() {
        let mut c = shown();
        c.on_action(Action::Power("restart".into()));
        c.on_action(Action::TogglePopover);
        c.on_hide();
        assert!(c.armed().is_none() && !c.popover_open());
    }

    #[test]
    fn reduced_motion_pushes_no_cues() {
        let mut c = shown();
        c.set_settings(&json!({ "reduceMotion": true }));
        c.on_action(Action::TogglePopover);
        c.on_action(Action::Power("lock".into()));
        c.on_action(Action::Toggle("wifi".into()));
        assert!(c.take_fx().is_empty());
    }
}
