//! Typed readers for the uninstall backend's JSON (uninstall.rs): the `uninstall_info` answer for the confirm card, the `uninstall:state` payloads
//! (scan progress, the review / done payload with the candidate list), the `uninstall:item` events and the `uninstall:ready` island payload.
//! Every reader is total: a missing or mistyped field becomes its empty value, exactly like the JS `||` fallbacks.

use serde_json::Value;

fn str_of(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}
fn u64_of(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(0)
}
fn bool_of(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// `uninstall_info` answer: details for the confirm card. `ok == false` carries an `error` (`busy` | `stale`).
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub ok: bool,
    pub error: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub location: String,
    pub size_kb: u64,
    pub store: bool,
    pub can_silent: bool,
    pub risky: bool,
}

impl Info {
    pub fn from_value(v: &Value) -> Info {
        Info {
            ok: bool_of(v, "ok"),
            error: str_of(v, "error"),
            name: str_of(v, "name"),
            publisher: str_of(v, "publisher"),
            version: str_of(v, "version"),
            location: str_of(v, "location"),
            size_kb: u64_of(v, "sizeKb"),
            store: bool_of(v, "store"),
            can_silent: bool_of(v, "canSilent"),
            risky: bool_of(v, "risky"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// points into the install folder, carries the product GUID or is named like the app: pre-ticked
    Certain,
    Probable,
}

/// One leftover candidate (`items_json`).
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// the index the backend uses; the page only ever sends these back, never paths
    pub id: usize,
    pub kind: String,
    /// the path / key as shown (`c.shown`); also what `uninstall:item` reports back
    pub path: String,
    pub size: u64,
    pub ticked: bool,
    pub note: String,
    pub admin: bool,
    pub tier: Tier,
}

impl Item {
    pub fn from_value(v: &Value) -> Item {
        Item {
            id: u64_of(v, "id") as usize,
            kind: str_of(v, "kind"),
            path: str_of(v, "path"),
            size: u64_of(v, "size"),
            ticked: bool_of(v, "ticked"),
            note: str_of(v, "note"),
            admin: bool_of(v, "admin"),
            tier: if v.get("tier").and_then(Value::as_str) == Some("certain") { Tier::Certain } else { Tier::Probable },
        }
    }
}

/// The summary fields of a `done` payload.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Summary {
    pub files: u64,
    pub keys: u64,
    pub other: u64,
    pub failed: Vec<String>,
    pub backup: String,
    pub log: String,
    /// how many ticked items were administrator-only and skipped (0 = none)
    pub need_admin: u64,
}

impl Summary {
    /// `any` of the JS: whether anything happened at all (otherwise "no leftovers found").
    pub fn any(&self) -> bool {
        !self.log.is_empty() || self.files > 0 || self.keys > 0 || self.other > 0 || !self.failed.is_empty()
    }
}

/// A `review` or `done` payload: the candidate list plus flags.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Review {
    pub name: String,
    pub items: Vec<Item>,
    pub manual: Vec<String>,
    pub cut: bool,
    pub admin_error: String,
    pub admin_scanned: bool,
    pub hidden_others: u64,
    pub summary: Summary,
}

impl Review {
    pub fn from_value(v: &Value) -> Review {
        let list = |k: &str| v.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
        Review {
            name: str_of(v, "name"),
            items: list("items").iter().map(Item::from_value).collect(),
            manual: list("manual").iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
            cut: bool_of(v, "cut"),
            admin_error: str_of(v, "adminError"),
            admin_scanned: bool_of(v, "adminScanned"),
            hidden_others: u64_of(v, "hiddenOthers"),
            summary: Summary {
                files: u64_of(v, "files"),
                keys: u64_of(v, "keys"),
                other: u64_of(v, "other"),
                failed: list("failed").iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
                backup: str_of(v, "backup"),
                log: str_of(v, "log"),
                need_admin: u64_of(v, "needAdmin"),
            },
        }
    }
}

/// `scanning` payload.
#[derive(Clone, Debug, PartialEq)]
pub struct Scan {
    pub name: String,
    pub step: String,
    pub found: u64,
}

impl Scan {
    pub fn from_value(v: &Value) -> Scan {
        Scan { name: str_of(v, "name"), step: str_of(v, "step"), found: u64_of(v, "found") }
    }
}

/// `elevating` payload: `mode` is `scan` or `delete`.
pub fn elevating_mode(v: &Value) -> String {
    str_of(v, "mode")
}

/// `uninstall:item` event: one deletion result.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDone {
    pub shown: String,
    pub ok: bool,
    pub error: String,
    pub n: u64,
    pub total: u64,
}

impl ItemDone {
    pub fn from_value(v: &Value) -> ItemDone {
        ItemDone { shown: str_of(v, "shown"), ok: bool_of(v, "ok"), error: str_of(v, "error"), n: u64_of(v, "n"), total: u64_of(v, "total") }
    }
}

/// `uninstall:ready` (all windows, only while the bar is closed): `kind` is `review` | `done` | `still` | `error`.
#[derive(Clone, Debug, PartialEq)]
pub struct Ready {
    pub name: String,
    pub kind: String,
    pub count: u64,
}

impl Ready {
    pub fn from_value(v: &Value) -> Ready {
        Ready { name: str_of(v, "name"), kind: str_of(v, "kind"), count: u64_of(v, "count") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn info_reads_the_confirm_card_fields() {
        let i = Info::from_value(&json!({ "ok": true, "name": "qBittorrent", "publisher": "The qBittorrent project", "version": "4.6.5", "location": "C:\\Program Files\\qBittorrent",
            "sizeKb": 51200, "store": false, "canSilent": true, "risky": false }));
        assert!(i.ok && i.can_silent && !i.risky && !i.store);
        assert_eq!((i.name.as_str(), i.size_kb), ("qBittorrent", 51200));
        let bad = Info::from_value(&json!({ "ok": false, "error": "stale" }));
        assert!(!bad.ok);
        assert_eq!(bad.error, "stale");
        assert!(!Info::from_value(&Value::Null).ok);
    }

    #[test]
    fn review_reads_items_tiers_and_flags() {
        let r = Review::from_value(&json!({ "phase": "review", "name": "App", "needAdmin": 1, "cut": true, "manual": ["Driver X"], "hiddenOthers": 3, "adminScanned": true,
            "items": [
                { "id": 0, "kind": "folder", "path": "C:\\Apps\\App", "size": 2048, "ticked": true, "note": "install folder", "admin": false, "tier": "certain" },
                { "id": 1, "kind": "key", "path": "HKLM\\X", "size": 0, "ticked": false, "note": "", "admin": true, "tier": "probable" } ] }));
        assert_eq!(r.items.len(), 2);
        assert_eq!((r.items[0].tier, r.items[1].tier), (Tier::Certain, Tier::Probable));
        assert!(r.items[1].admin && r.cut && r.admin_scanned);
        assert_eq!((r.hidden_others, r.summary.need_admin), (3, 1));
        assert_eq!(r.manual, vec!["Driver X"]);
        assert!(!r.summary.any());
    }

    #[test]
    fn summary_any_matches_the_js_condition() {
        let done = Review::from_value(&json!({ "phase": "done", "name": "A", "files": 2, "keys": 0, "other": 0, "failed": [], "log": "" }));
        assert!(done.summary.any());
        let only_log = Review::from_value(&json!({ "log": "C:\\x.log" }));
        assert!(only_log.summary.any());
        let failed = Review::from_value(&json!({ "failed": ["a: denied"] }));
        assert!(failed.summary.any());
        assert!(!Review::from_value(&json!({})).summary.any());
    }

    #[test]
    fn scan_item_and_ready() {
        let s = Scan::from_value(&json!({ "phase": "scanning", "name": "A", "step": "files and folders", "found": 12 }));
        assert_eq!((s.found, s.step.as_str()), (12, "files and folders"));
        let d = ItemDone::from_value(&json!({ "shown": "C:\\a", "ok": false, "error": "denied", "n": 2, "total": 5 }));
        assert!(!d.ok && d.error == "denied" && d.total == 5);
        let r = Ready::from_value(&json!({ "name": "A", "kind": "review", "count": 7 }));
        assert_eq!((r.kind.as_str(), r.count), ("review", 7));
        assert_eq!(elevating_mode(&json!({ "mode": "delete" })), "delete");
    }
}
