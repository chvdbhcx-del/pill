// Notification centre history (tab-notifs.js): every event card the island showed, kept locally in store.db (table `notif_history`, store.rs migration 4).
// Commands: notif_history_add / _list / _clear / _mark_read / _remove. Capped at MAX_ROWS entries, entries older than MAX_AGE_MS are dropped on every add / list.
// PRIVACY: local only, never sent anywhere. Settings: history.keep (default true; false = nothing is stored), history.mail (default false:
// entries of kind "mail" / "notif" keep only their title, no sub; a mail title is cut before the " — " that precedes the subject).
use crate::store;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

const MAX_ROWS: i64 = 200;
const MAX_AGE_MS: i64 = 14 * 24 * 3600 * 1000;

pub fn init(_app: &tauri::App) {}

#[derive(Deserialize)]
pub struct NewEntry { kind: Option<String>, icon: Option<String>, tone: Option<String>, title: Option<String>, sub: Option<String> }

#[derive(Serialize, Debug, PartialEq)]
pub struct Entry { id: i64, ts: i64, kind: String, icon: String, tone: String, title: String, sub: String, read: bool }

fn cut(s: &str, max: usize) -> String {
    let flat: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > max { flat.chars().take(max - 1).collect::<String>() + "\u{2026}" } else { flat }
}

/// Short identifier (kind / icon / tone): ascii letters, digits, `-` `_` `:` only.
fn ident(s: &Option<String>, max: usize) -> String {
    s.as_deref().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')).take(max).collect()
}

/// (title, sub) as they are stored. Mail and Windows notifications lose their body unless `keep_mail`.
pub fn sanitize(kind: &str, title: &str, sub: &str, keep_mail: bool) -> (String, String) {
    let mut title = cut(title, 200);
    let mut sub = cut(sub, 400);
    if !keep_mail && matches!(kind, "mail" | "notif") {
        sub.clear();
        if kind == "mail" {
            if let Some(i) = title.find(" \u{2014} ") { title.truncate(i); }
        }
    }
    (title, sub)
}

fn prune(c: &Connection, now: i64) -> rusqlite::Result<()> {
    c.execute("delete from notif_history where ts < ?1", [now - MAX_AGE_MS])?;
    c.execute("delete from notif_history where id not in (select id from notif_history order by ts desc, id desc limit ?1)", [MAX_ROWS])?;
    Ok(())
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry { id: r.get(0)?, ts: r.get(1)?, kind: r.get(2)?, icon: r.get(3)?, tone: r.get(4)?, title: r.get(5)?, sub: r.get(6)?, read: r.get::<_, i64>(7)? != 0 })
}

fn add_at(c: &Connection, now: i64, e: &NewEntry, read: bool, keep_mail: bool) -> rusqlite::Result<Option<Entry>> {
    let kind = ident(&e.kind, 24);
    let (title, sub) = sanitize(&kind, e.title.as_deref().unwrap_or(""), e.sub.as_deref().unwrap_or(""), keep_mail);
    if title.is_empty() { return Ok(None); }
    let (icon, tone) = (ident(&e.icon, 24), ident(&e.tone, 12));
    c.execute("insert into notif_history (ts, kind, icon, tone, title, sub, read) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)", (now, &kind, &icon, &tone, &title, &sub, read as i64))?;
    let id = c.last_insert_rowid();
    prune(c, now)?;
    Ok(Some(Entry { id, ts: now, kind, icon, tone, title, sub, read }))
}

fn list_at(c: &Connection, now: i64) -> rusqlite::Result<Vec<Entry>> {
    prune(c, now)?;
    let mut st = c.prepare("select id, ts, kind, icon, tone, title, sub, read from notif_history order by ts desc, id desc")?;
    let v = st.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
    Ok(v)
}

fn mark_read_in(c: &Connection, ids: Option<&[i64]>) -> rusqlite::Result<usize> {
    match ids {
        None => c.execute("update notif_history set read = 1 where read = 0", []),
        Some(ids) => {
            let mut n = 0;
            for id in ids { n += c.execute("update notif_history set read = 1 where id = ?1 and read = 0", [id])?; }
            Ok(n)
        }
    }
}

/// Store one entry (None when history.keep is off or the title is empty). `read`: the centre was open when it arrived.
pub fn notif_history_add(entry: NewEntry, read: Option<bool>) -> Option<Entry> {
    if !crate::settings::get_bool("history.keep", true) { return None; }
    let keep_mail = crate::settings::get_bool("history.mail", false);
    store::with(|c| add_at(c, store::now_ms(), &entry, read.unwrap_or(false), keep_mail)).flatten()
}

/// Newest first.
pub fn notif_history_list() -> Vec<Entry> {
    store::with(|c| list_at(c, store::now_ms())).unwrap_or_default()
}

pub fn notif_history_clear() { store::with(|c| c.execute("delete from notif_history", [])); }

/// ids = None marks everything read.
pub fn notif_history_mark_read(ids: Option<Vec<i64>>) -> usize {
    store::with(|c| mark_read_in(c, ids.as_deref())).unwrap_or(0)
}

pub fn notif_history_remove(id: i64) { store::with(|c| c.execute("delete from notif_history where id = ?1", [id])); }

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::store::migrate(&c);
        c
    }
    fn ne(kind: &str, title: &str, sub: &str) -> NewEntry {
        NewEntry { kind: Some(kind.into()), icon: Some("bell".into()), tone: Some("green".into()), title: Some(title.into()), sub: Some(sub.into()) }
    }

    #[test]
    fn mail_and_notif_lose_their_body_unless_allowed() {
        assert_eq!(sanitize("mail", "New mail: Ann \u{2014} Salary", "and 2 more", false), ("New mail: Ann".into(), String::new()));
        assert_eq!(sanitize("notif", "Chat", "secret text", false), ("Chat".into(), String::new()));
        assert_eq!(sanitize("mail", "New mail: Ann \u{2014} Salary", "x", true), ("New mail: Ann \u{2014} Salary".into(), "x".into()));
        assert_eq!(sanitize("usb", "Drive E:", "sub", false).1, "sub");
    }

    #[test]
    fn add_list_read_remove_clear() {
        let c = db();
        let a = add_at(&c, 1000, &ne("usb", "Drive E:", ""), false, false).unwrap().unwrap();
        let b = add_at(&c, 2000, &ne("mail", "New mail: A \u{2014} S", "more"), false, false).unwrap().unwrap();
        assert_eq!(b.sub, "");
        assert!(add_at(&c, 3000, &ne("usb", "   ", ""), false, false).unwrap().is_none());
        let l = list_at(&c, 3000).unwrap();
        assert_eq!(l.iter().map(|e| e.id).collect::<Vec<_>>(), vec![b.id, a.id]); // newest first
        assert_eq!(mark_read_in(&c, Some(&[a.id])).unwrap(), 1);
        assert_eq!(mark_read_in(&c, None).unwrap(), 1);
        assert!(list_at(&c, 3000).unwrap().iter().all(|e| e.read));
        c.execute("delete from notif_history where id = ?1", [a.id]).unwrap();
        assert_eq!(list_at(&c, 3000).unwrap().len(), 1);
    }

    #[test]
    fn capped_at_200_and_expires_after_14_days() {
        let c = db();
        for i in 0..230 { add_at(&c, 1_000_000 + i, &ne("x", &format!("n{i}"), ""), false, false).unwrap(); }
        let l = list_at(&c, 1_000_300).unwrap();
        assert_eq!(l.len() as i64, MAX_ROWS);
        assert_eq!(l[0].title, "n229");
        assert_eq!(l.last().unwrap().title, "n30");
        assert_eq!(list_at(&c, 1_000_130 + MAX_AGE_MS).unwrap().len(), 100); // ts 1_000_030..129 age out
        assert!(list_at(&c, 1_000_300 + MAX_AGE_MS).unwrap().is_empty());
    }

    #[test]
    fn identifiers_are_filtered() {
        let e = NewEntry { kind: Some("ma<il>".into()), icon: None, tone: Some("gr een".into()), title: Some("t\n\tx".into()), sub: None };
        let c = db();
        let r = add_at(&c, 5, &e, false, false).unwrap().unwrap();
        assert_eq!((r.kind.as_str(), r.tone.as_str(), r.title.as_str()), ("mail", "green", "t x"));
    }
}
