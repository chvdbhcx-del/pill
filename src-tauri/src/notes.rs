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
#[tauri::command(rename_all = "snake_case")]
pub fn note_list(folder_id: Option<i64>) -> Vec<NoteMeta> {
    store::with(|c| {
        let mut st = c.prepare("select id, folder_id, title, pinned, updated_at from notes where (?1 is null or folder_id = ?1) order by pinned desc, id")?;
        let v = st.query_map([folder_id], meta)?.collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }).unwrap_or_default()
}

#[tauri::command]
pub fn note_get(id: i64) -> Option<Note> {
    store::with(|c| {
        c.query_row("select id, folder_id, title, body_html, pinned, updated_at from notes where id = ?1", [id], |r| {
            Ok(Note { id: r.get(0)?, folder_id: r.get(1)?, title: r.get(2)?, body_html: r.get(3)?, pinned: r.get::<_, i64>(4)? != 0, updated_at: r.get(5)? })
        })
    })
}

/// Create (id = None) or update a note. `title` is the first line, derived by the page.
#[tauri::command(rename_all = "snake_case")]
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

#[tauri::command]
pub fn note_delete(id: i64) { store::with(|c| c.execute("delete from notes where id = ?1", [id])); }
