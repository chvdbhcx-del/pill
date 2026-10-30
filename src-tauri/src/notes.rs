// Notes: rows of the `notes` table (store.rs V1). Owner: text/note agent. The page sends sanitized HTML (whitelist in
// tab-note.js) and sanitizes again when it loads a note; this file only stores, lists and deletes (size-capped).
// Commands: note_list, note_get, note_save, note_delete. Folders (note_folders) come in a later slice.
use crate::store;
use serde::Serialize;

const MAX_BODY: usize = 2_000_000; // bytes of HTML per note

pub fn init(_app: &tauri::App) {}

#[derive(Serialize)]
pub struct NoteMeta { id: i64, folder_id: Option<i64>, title: String, pinned: bool, updated_at: i64 }
#[derive(Serialize)]
pub struct Note { id: i64, folder_id: Option<i64>, title: String, body_html: String, pinned: bool, updated_at: i64 }

fn meta(r: &rusqlite::Row) -> rusqlite::Result<NoteMeta> {
    Ok(NoteMeta { id: r.get(0)?, folder_id: r.get(1)?, title: r.get(2)?, pinned: r.get::<_, i64>(3)? != 0, updated_at: r.get(4)? })
}

/// All notes (or one folder's), pinned first, then oldest first so the tab chips keep a stable order.
pub fn note_list(folder_id: Option<i64>) -> Vec<NoteMeta> {
    store::with(|c| {
        let mut st = c.prepare("select id, folder_id, title, pinned, updated_at from notes where (?1 is null or folder_id = ?1) order by pinned desc, id")?;
        let v = st.query_map([folder_id], meta)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).unwrap_or_default()
}

pub fn note_get(id: i64) -> Option<Note> {
    store::with(|c| {
        c.query_row("select id, folder_id, title, body_html, pinned, updated_at from notes where id = ?1", [id], |r| {
            Ok(Note { id: r.get(0)?, folder_id: r.get(1)?, title: r.get(2)?, body_html: r.get(3)?, pinned: r.get::<_, i64>(4)? != 0, updated_at: r.get(5)? })
        })
    })
}

/// Create (id = None) or update a note. `title` is the first line, derived by the page.
pub fn note_save(id: Option<i64>, body_html: String, title: Option<String>) -> Result<NoteMeta, String> {
    if body_html.len() > MAX_BODY { return Err("too_big".into()); }
    let title: String = title.unwrap_or_default().chars().take(120).collect();
    let now = store::now_ms();
    store::with(|c| {
        let id = match id {
            Some(id) => { c.execute("update notes set body_html = ?2, title = ?3, updated_at = ?4 where id = ?1", (id, &body_html, &title, now))?; id }
            None => { c.execute("insert into notes (title, body_html, updated_at) values (?1, ?2, ?3)", (&title, &body_html, now))?; c.last_insert_rowid() }
        };
        c.query_row("select id, folder_id, title, pinned, updated_at from notes where id = ?1", [id], meta)
    }).ok_or_else(|| "store".to_string())
}

pub fn note_delete(id: i64) { store::with(|c| c.execute("delete from notes where id = ?1", [id])); }

// ---- text index for the command bar's note search ----
// One entry per note of the newest 200 (pinned first), stripped to plain text and lowercased once. A keystroke only runs a cheap
// (id, title, updated_at) query and reuses every entry that did not change; note_save moves updated_at, so edits show up at once.
// Kept only while the bar is open (command::hide drops it), like the other in-app rows.

/// A note prepared for searching: `hay` is "title body" lowercased, `body` the stripped text for snippets / previews.
pub(crate) struct NoteDoc { pub id: i64, pub title: String, pub body: String, pub hay: String, updated_at: i64 }

const INDEX_NOTES: usize = 200;
const INDEX_CHARS: usize = 20_000; // per note, like the old per-keystroke scan
static INDEX: std::sync::Mutex<Vec<std::sync::Arc<NoteDoc>>> = std::sync::Mutex::new(Vec::new());

/// Refreshes `cache` against the database and returns the notes in display order (pinned, then newest).
fn refresh(cache: &mut Vec<std::sync::Arc<NoteDoc>>, c: &rusqlite::Connection) -> rusqlite::Result<Vec<std::sync::Arc<NoteDoc>>> {
    let mut st = c.prepare("select id, title, updated_at from notes order by pinned desc, updated_at desc limit ?1")?;
    let heads = st.query_map([INDEX_NOTES as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::with_capacity(heads.len());
    for (id, title, updated_at) in heads {
        if let Some(d) = cache.iter().find(|d| d.id == id && d.updated_at == updated_at && d.title == title) {
            out.push(d.clone());
            continue;
        }
        let html: String = c.query_row("select substr(body_html, 1, ?2) from notes where id = ?1", rusqlite::params![id, INDEX_CHARS as i64], |r| r.get(0))?;
        let body = crate::command::strip_html(&html);
        let hay = format!("{title} {body}").to_lowercase();
        out.push(std::sync::Arc::new(NoteDoc { id, title, body, hay, updated_at }));
    }
    *cache = out.clone();
    Ok(out)
}

pub(crate) fn search_index() -> Vec<std::sync::Arc<NoteDoc>> {
    store::with(|c| refresh(&mut INDEX.lock().unwrap_or_else(|e| e.into_inner()), c)).unwrap_or_default()
}

pub(crate) fn search_cache_drop() {
    *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = Vec::new();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> rusqlite::Connection {
        let c = rusqlite::Connection::open_in_memory().unwrap();
        c.execute_batch("create table notes (id integer primary key, folder_id integer, title text not null default '', body_html text not null default '', pinned integer not null default 0, updated_at integer not null);").unwrap();
        c
    }

    #[test]
    fn index_reuses_unchanged_and_follows_edits() {
        let c = db();
        c.execute("insert into notes (id, title, body_html, updated_at) values (1, 'Alpha', '<p>Hello&nbsp;<b>World</b></p>', 10), (2, '', '<p>Second</p>', 20), (3, 'Pinned', 'x', 5)", []).unwrap();
        c.execute("update notes set pinned = 1 where id = 3", []).unwrap();
        let mut cache = Vec::new();
        let a = refresh(&mut cache, &c).unwrap();
        assert_eq!(a.iter().map(|d| d.id).collect::<Vec<_>>(), [3, 2, 1]); // pinned first, then newest
        assert_eq!(a[2].hay, "alpha hello world");
        let b = refresh(&mut cache, &c).unwrap();
        assert!(a.iter().zip(&b).all(|(x, y)| std::sync::Arc::ptr_eq(x, y))); // nothing changed: same entries, no re-strip
        c.execute("update notes set body_html = '<p>Changed</p>', updated_at = 30 where id = 1", []).unwrap();
        c.execute("delete from notes where id = 2", []).unwrap();
        let d = refresh(&mut cache, &c).unwrap();
        assert_eq!(d.iter().map(|d| d.id).collect::<Vec<_>>(), [3, 1]);
        assert_eq!(d[1].hay, "alpha changed");
        assert!(std::sync::Arc::ptr_eq(&a[0], &d[0])); // the untouched note kept its entry
        assert_eq!(cache.len(), 2); // the cache follows the database: deleted notes are gone
    }

    // Timing, run with --nocapture: 200 notes x 20 000 chars of HTML, the old per-keystroke scan against the cached index.
    #[test]
    fn index_timing() {
        let c = db();
        let html = format!("<p>{}</p>", "lorem <b>ipsum</b> dolor&nbsp;sit amet, ".repeat(500)); // ~20 KB
        for i in 0..200 {
            c.execute("insert into notes (id, title, body_html, updated_at) values (?1, ?2, ?3, ?1)", rusqlite::params![i, format!("note {i}"), &html]).unwrap();
        }
        let w = vec!["needle".to_string()];
        let t = std::time::Instant::now();
        let mut st = c.prepare("select id, title, substr(body_html,1,20000) from notes order by pinned desc, updated_at desc limit 200").unwrap();
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        let old_hits = rows.iter().filter(|(_, t, h)| format!("{t} {}", crate::command::strip_html(h)).to_lowercase().contains(w[0].as_str())).count();
        let old = t.elapsed();
        let mut cache = Vec::new();
        let first = std::time::Instant::now();
        refresh(&mut cache, &c).unwrap();
        let first = first.elapsed();
        let t = std::time::Instant::now();
        let docs = refresh(&mut cache, &c).unwrap();
        let new_hits = docs.iter().filter(|d| d.hay.contains(w[0].as_str())).count();
        let new = t.elapsed();
        assert_eq!(old_hits, new_hits);
        eprintln!("notes search, 200 x 20 KB: old per keystroke {old:?}; index build (once per bar open) {first:?}; cached per keystroke {new:?}");
    }
}
