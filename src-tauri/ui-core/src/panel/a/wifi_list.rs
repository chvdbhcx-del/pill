//! The Wi-Fi network list of the Controls tab (opened from the list button of the Wi-Fi tile).
//!
//! Backend: `wifi_scan` -> `{status, networks: [{ssid, bars, secured, supported, connected, known}]}`, `wifi_connect {ssid, password?}` ->
//! `{status: ok | failed | password | unsupported | error}`, `wifi_disconnect`. Scanning happens only while the list is open: once when it opens
//! and every [`POLL_MS`] after that. Nothing is polled otherwise.
//!
//! Picking a network: a saved or open one connects at once; a new secured one opens a password drawer under its row. The password goes to
//! the backend in the one `wifi_connect` call and is not kept here. A connect that failed reopens the drawer (a wrong or changed password).

use super::{BackendCall, Tr};
use serde_json::{json, Value};

/// While the list is open it asks for a fresh scan this often.
pub const POLL_MS: f64 = 5000.0;
/// How long a short message ("Could not connect") stays under the header.
pub const FLASH_MS: f64 = 3500.0;

#[derive(Clone, Debug, PartialEq)]
pub struct WifiNet {
    pub ssid: String,
    pub bars: u8,
    pub secured: bool,
    pub supported: bool,
    pub connected: bool,
    pub known: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    Normal,
    Connected,
    Connecting,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AskView {
    pub placeholder: String,
    pub button: String,
    /// "Wrong password or the network refused" after a failed try.
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WifiRow {
    pub ssid: String,
    pub bars: u8,
    pub secured: bool,
    pub state: RowState,
    pub detail: String,
    /// Enterprise networks and the like: shown dim, Windows' own settings connect them.
    pub dim: bool,
    pub ask: Option<AskView>,
    /// The "Disconnect" button on the connected row.
    pub disconnect: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WifiView {
    pub title: String,
    pub rows: Vec<WifiRow>,
    /// Shown instead of the rows: first scan, no adapter, nothing in range.
    pub empty: Option<String>,
    /// A short message from the last action.
    pub flash: Option<String>,
}

#[derive(Default)]
pub struct WifiList {
    open: bool,
    nets: Vec<WifiNet>,
    loaded: bool,
    error: Option<String>,
    /// (ssid, previous try failed) the password drawer is open for
    ask: Option<(String, bool)>,
    connecting: Option<String>,
    disconnecting: bool,
    flash: Option<(String, f64)>,
    poll: f64,
    mono: f64,
}

impl WifiList {
    pub fn is_open(&self) -> bool {
        self.open
    }
    /// The ssid whose password drawer is open.
    pub fn asking(&self) -> Option<&str> {
        self.ask.as_ref().map(|a| a.0.as_str())
    }

    pub fn open(&mut self) -> Vec<BackendCall> {
        if self.open {
            return vec![];
        }
        *self = WifiList { open: true, mono: self.mono, ..Default::default() };
        vec![BackendCall::bare("wifi_scan")]
    }

    pub fn close(&mut self) {
        self.open = false;
        self.ask = None;
        self.connecting = None;
        self.disconnecting = false;
    }

    fn busy(&self) -> bool {
        self.connecting.is_some() || self.disconnecting
    }

    fn note(&mut self, text: String) {
        self.flash = Some((text, self.mono + FLASH_MS));
    }

    /// A tap on a row.
    pub fn pick(&mut self, ssid: &str, tr: &dyn Tr) -> Vec<BackendCall> {
        if self.busy() {
            return vec![];
        }
        let Some(n) = self.nets.iter().find(|n| n.ssid == ssid).cloned() else { return vec![] };
        if n.connected {
            return vec![];
        }
        if !n.supported {
            self.note(tr.t("Подключите в параметрах Windows", "Connect it in Windows settings"));
            return vec![];
        }
        if n.secured && !n.known {
            self.ask = Some((n.ssid, false));
            return vec![];
        }
        self.ask = None;
        self.connecting = Some(n.ssid.clone());
        vec![BackendCall::new("wifi_connect", json!({ "ssid": n.ssid }))]
    }

    /// The password drawer's Connect / Enter. The text is moved into the call and not kept.
    pub fn submit(&mut self, password: String) -> Vec<BackendCall> {
        let Some((ssid, _)) = self.ask.clone() else { return vec![] };
        if self.busy() || password.is_empty() {
            return vec![];
        }
        self.connecting = Some(ssid.clone());
        vec![BackendCall::new("wifi_connect", json!({ "ssid": ssid, "password": password }))]
    }

    pub fn cancel_ask(&mut self) {
        if !self.busy() {
            self.ask = None;
        }
    }

    pub fn disconnect(&mut self) -> Vec<BackendCall> {
        if self.busy() {
            return vec![];
        }
        self.disconnecting = true;
        vec![BackendCall::bare("wifi_disconnect")]
    }

    pub fn on_result(&mut self, cmd: &str, args: &Value, res: &Value, tr: &dyn Tr) -> Vec<BackendCall> {
        let status = res.get("status").and_then(Value::as_str).unwrap_or("");
        match cmd {
            "wifi_scan" if self.open => {
                self.loaded = true;
                if status == "ok" {
                    self.error = None;
                    self.nets = res.get("networks").and_then(Value::as_array).map(|a| a.iter().map(net).collect()).unwrap_or_default();
                    // an ask for a network that is gone closes
                    if let Some((s, _)) = &self.ask {
                        if !self.nets.iter().any(|n| &n.ssid == s) && !self.busy() {
                            self.ask = None;
                        }
                    }
                } else {
                    self.error = Some(res.get("message").and_then(Value::as_str).unwrap_or("").to_string());
                    self.nets.clear();
                }
                vec![]
            }
            "wifi_connect" => {
                let ssid = args.get("ssid").and_then(Value::as_str).unwrap_or("").to_string();
                self.connecting = None;
                match status {
                    "ok" => {
                        self.ask = None;
                    }
                    "password" | "failed" => {
                        let bad = status == "failed" || res.get("message").and_then(Value::as_str) == Some("bad");
                        self.ask = Some((ssid, bad));
                        if status == "failed" {
                            self.note(tr.t("Не удалось подключиться", "Could not connect"));
                        }
                    }
                    "unsupported" => self.note(tr.t("Подключите в параметрах Windows", "Connect it in Windows settings")),
                    _ => self.note(tr.t("Не удалось подключиться", "Could not connect")),
                }
                if self.open { vec![BackendCall::bare("wifi_scan")] } else { vec![] }
            }
            "wifi_disconnect" => {
                self.disconnecting = false;
                if status != "ok" {
                    self.note(tr.t("Не удалось отключиться", "Could not disconnect"));
                }
                if self.open { vec![BackendCall::bare("wifi_scan")] } else { vec![] }
            }
            _ => vec![],
        }
    }

    /// The re-scan clock: only while the list is open and nothing is connecting (a connect already waits on the adapter).
    pub fn tick(&mut self, dt_ms: f64) -> Vec<BackendCall> {
        self.mono += dt_ms;
        if self.flash.as_ref().is_some_and(|f| f.1 <= self.mono) {
            self.flash = None;
        }
        if !self.open || self.busy() {
            return vec![];
        }
        self.poll += dt_ms;
        if self.poll >= POLL_MS {
            self.poll = 0.0;
            return vec![BackendCall::bare("wifi_scan")];
        }
        vec![]
    }

    pub fn view(&self, tr: &dyn Tr) -> WifiView {
        let rows: Vec<WifiRow> = self
            .nets
            .iter()
            .map(|n| {
                let connecting = self.connecting.as_deref() == Some(n.ssid.as_str());
                let state = if connecting { RowState::Connecting } else if n.connected { RowState::Connected } else { RowState::Normal };
                let detail = match state {
                    RowState::Connecting => tr.t("Подключение…", "Connecting…"),
                    RowState::Connected => tr.t("Подключено", "Connected"),
                    RowState::Normal if !n.supported => tr.t("Через Windows", "Via Windows"),
                    RowState::Normal if n.known => tr.t("Сохранена", "Saved"),
                    RowState::Normal => String::new(),
                };
                let ask = self.ask.as_ref().filter(|a| a.0 == n.ssid).map(|a| AskView {
                    placeholder: tr.t("Пароль сети", "Network password"),
                    button: tr.t("Подключить", "Connect"),
                    note: a.1.then(|| tr.t("Пароль не подошёл. Введите заново", "The password did not work. Enter it again")),
                });
                WifiRow {
                    ssid: n.ssid.clone(),
                    bars: n.bars,
                    secured: n.secured,
                    state,
                    detail,
                    dim: !n.supported,
                    ask,
                    disconnect: (n.connected && !self.disconnecting).then(|| tr.t("Отключить", "Disconnect")),
                }
            })
            .collect();
        let empty = if !rows.is_empty() {
            None
        } else if let Some(e) = &self.error {
            Some(if e.contains("adapter") || e.contains("service") { tr.t("Нет Wi-Fi адаптера", "No Wi-Fi adapter") } else { tr.t("Wi-Fi недоступен", "Wi-Fi is not available") })
        } else if !self.loaded {
            Some(tr.t("Поиск сетей…", "Looking for networks…"))
        } else {
            Some(tr.t("Сети не найдены", "No networks found"))
        };
        WifiView { title: tr.t("Сети Wi-Fi", "Wi-Fi networks"), rows, empty, flash: self.flash.as_ref().map(|f| f.0.clone()) }
    }
}

fn net(v: &Value) -> WifiNet {
    let b = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    WifiNet {
        ssid: v.get("ssid").and_then(Value::as_str).unwrap_or("").to_string(),
        bars: v.get("bars").and_then(Value::as_u64).unwrap_or(0).min(4) as u8,
        secured: b("secured"),
        supported: v.get("supported").and_then(Value::as_bool).unwrap_or(true),
        connected: b("connected"),
        known: b("known"),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{EnTr, RuTr};
    use super::*;

    fn scan(w: &mut WifiList) {
        w.open();
        w.on_result(
            "wifi_scan",
            &Value::Null,
            &json!({ "status": "ok", "networks": [
                { "ssid": "Office", "bars": 4, "secured": true, "supported": true, "connected": true, "known": true },
                { "ssid": "Home", "bars": 3, "secured": true, "supported": true, "connected": false, "known": true },
                { "ssid": "Cafe", "bars": 2, "secured": true, "supported": true, "connected": false, "known": false },
                { "ssid": "Free", "bars": 1, "secured": false, "supported": true, "connected": false, "known": false },
                { "ssid": "Corp", "bars": 3, "secured": true, "supported": false, "connected": false, "known": false }
            ] }),
            &EnTr,
        );
    }

    #[test]
    fn opening_scans_once_and_polling_runs_only_while_open() {
        let mut w = WifiList::default();
        assert!(w.tick(60_000.0).is_empty(), "closed: no scans");
        assert_eq!(w.open(), vec![BackendCall::bare("wifi_scan")]);
        assert!(w.open().is_empty(), "already open");
        assert!(w.tick(4_000.0).is_empty());
        assert_eq!(w.tick(1_000.0), vec![BackendCall::bare("wifi_scan")]);
        w.close();
        assert!(w.tick(60_000.0).is_empty());
        assert_eq!(w.view(&EnTr).empty.as_deref(), Some("Looking for networks…"));
    }

    #[test]
    fn saved_and_open_networks_connect_at_once_new_secured_ones_ask() {
        let mut w = WifiList::default();
        scan(&mut w);
        assert_eq!(w.pick("Home", &EnTr), vec![BackendCall::new("wifi_connect", json!({ "ssid": "Home" }))]);
        assert_eq!(w.view(&EnTr).rows[1].state, RowState::Connecting);
        assert!(w.pick("Free", &EnTr).is_empty(), "one connection at a time");
        w.on_result("wifi_connect", &json!({ "ssid": "Home" }), &json!({ "status": "ok" }), &EnTr);
        assert_eq!(w.pick("Free", &EnTr), vec![BackendCall::new("wifi_connect", json!({ "ssid": "Free" }))]);
        w.on_result("wifi_connect", &json!({ "ssid": "Free" }), &json!({ "status": "ok" }), &EnTr);
        assert!(w.pick("Cafe", &EnTr).is_empty());
        assert_eq!(w.asking(), Some("Cafe"));
        let v = w.view(&EnTr);
        assert!(v.rows[2].ask.is_some() && v.rows[0].ask.is_none());
        assert_eq!(v.rows[0].disconnect.as_deref(), Some("Disconnect"));
    }

    #[test]
    fn the_password_travels_in_the_call_only_and_a_wrong_one_reopens_the_drawer() {
        let mut w = WifiList::default();
        scan(&mut w);
        w.pick("Cafe", &EnTr);
        assert!(w.submit(String::new()).is_empty(), "an empty password is not sent");
        let calls = w.submit("hunter2hunter2".into());
        assert_eq!(calls, vec![BackendCall::new("wifi_connect", json!({ "ssid": "Cafe", "password": "hunter2hunter2" }))]);
        assert!(!format!("{:?}", w.view(&EnTr)).contains("hunter2"), "the view never holds it");
        let next = w.on_result("wifi_connect", &json!({ "ssid": "Cafe" }), &json!({ "status": "failed" }), &EnTr);
        assert_eq!(next, vec![BackendCall::bare("wifi_scan")]);
        let v = w.view(&EnTr);
        assert_eq!(v.flash.as_deref(), Some("Could not connect"));
        assert!(v.rows[2].ask.as_ref().unwrap().note.is_some());
        w.cancel_ask();
        assert_eq!(w.asking(), None);
    }

    #[test]
    fn enterprise_networks_explain_themselves_instead_of_connecting() {
        let mut w = WifiList::default();
        scan(&mut w);
        assert!(w.pick("Corp", &RuTr).is_empty());
        assert!(w.view(&RuTr).flash.unwrap().contains("Windows"));
        assert!(w.view(&EnTr).rows[4].dim);
        // the message goes away by itself
        w.tick(FLASH_MS + 1.0);
        assert_eq!(w.view(&EnTr).flash, None);
    }

    #[test]
    fn disconnect_and_error_states() {
        let mut w = WifiList::default();
        scan(&mut w);
        assert_eq!(w.disconnect(), vec![BackendCall::bare("wifi_disconnect")]);
        assert!(w.view(&EnTr).rows[0].disconnect.is_none(), "the button is gone while it works");
        w.on_result("wifi_disconnect", &Value::Null, &json!({ "status": "ok" }), &EnTr);
        let mut e = WifiList::default();
        e.open();
        e.on_result("wifi_scan", &Value::Null, &json!({ "status": "error", "message": "no Wi-Fi adapter" }), &EnTr);
        assert_eq!(e.view(&EnTr).empty.as_deref(), Some("No Wi-Fi adapter"));
        // a scan answer after the list closed is ignored
        let mut c = WifiList::default();
        c.on_result("wifi_scan", &Value::Null, &json!({ "status": "ok", "networks": [{ "ssid": "x" }] }), &EnTr);
        assert!(c.view(&EnTr).rows.is_empty());
    }
}
