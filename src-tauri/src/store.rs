// SQLite store: %APPDATA%\pill\store.db (WAL). Owner: lead. Every lane's tables live HERE so lanes never
// fight over migrations. To add tables: append a new SQL string at the `// @migrations` marker (never edit an
// old one); the index in MIGRATIONS + 1 is PRAGMA user_version. Large blobs (images) go to files under
// %APPDATA%\pill\clips\, the DB only stores the path.
use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

static DB: OnceLock<Mutex<Connection>> = OnceLock::new();

/// %APPDATA%\pill (created on demand).
pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join("pill");
    let _ = std::fs::create_dir_all(&d);
    d
}

const V1: &str = "
create table clip_items (
  id          integer primary key,
  kind        text not null check (kind in ('text','image','files')),
  text        text,
  image_path  text,
  files_json  text,
  hash        text not null,
  source_exe  text,
  size_bytes  integer not null,
  pinned      integer not null default 0 check (pinned in (0,1)),
  created_at  integer not null,
  used_at     integer not null
);
create unique index clip_hash_idx on clip_items(hash);
create index clip_recent_idx on clip_items(pinned desc, used_at desc);

create table shelf_items (
  id integer primary key,
  path text not null,
  name text not null,
  added_at integer not null
);
create index shelf_added_idx on shelf_items(added_at desc);

create table note_folders (
  id integer primary key,
  name text not null,
  parent_id integer references note_folders(id) on delete cascade
);
create table notes (
  id integer primary key,
  folder_id integer references note_folders(id) on delete set null,
  title text not null default '',
  body_html text not null default '',
  pinned integer not null default 0,
  updated_at integer not null
);
create index notes_folder_idx on notes(folder_id, updated_at desc);

create table reminders (
  id integer primary key,
  kind text not null check (kind in ('reminder','timer')),
  text text not null default '',
  due_at integer not null,
  repeat_rule text not null default 'none',
  done integer not null default 0 check (done in (0,1)),
  created_at integer not null
);
create index reminders_due_idx on reminders(done, due_at);

create table launcher_groups ( id integer primary key, name text not null, sort integer not null default 0 );
create table launcher_links (
  id integer primary key,
  group_id integer not null references launcher_groups(id) on delete cascade,
  title text not null,
  target text not null,
  icon_path text,
  hotkey text,
  sort integer not null default 0
);
create index links_group_idx on launcher_links(group_id, sort);

create table color_history ( id integer primary key, hex text not null, picked_at integer not null );
";

const MIGRATIONS: &[&str] = &[
    V1,
    // chat (net/chat agent): AI chat threads and messages, exactly as in architecture.md
    "create table chat_threads ( id integer primary key, title text not null default '', created_at integer not null );
create table chat_messages (
  id integer primary key,
  thread_id integer not null references chat_threads(id) on delete cascade,
  role text not null check (role in ('user','assistant','system')),
  content text not null, attachments_json text, created_at integer not null
);
create index chat_thread_idx on chat_messages(thread_id, id);",
    // reminders/timers (timers.rs): fired = a one-shot already announced, waiting for Done/Snooze; total_s = timer length
    "alter table reminders add column fired integer not null default 0;
alter table reminders add column total_s integer not null default 0;",
    // notification centre history (notifhist.rs): what the island showed; capped and expired there
    "create table notif_history (
  id integer primary key,
  ts integer not null,
  kind text not null default '',
  icon text not null default '',
  tone text not null default '',
  title text not null default '',
  sub text not null default '',
  read integer not null default 0 check (read in (0,1))
);
create index notif_history_ts_idx on notif_history(ts desc);",
    // @migrations
];

/// Open the database and apply pending migrations. Never panics: on failure `with()` just returns None.
pub fn init(_app: &tauri::App) {
    let path = dir().join("store.db");
    let conn = match Connection::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("store: cannot open {}: {e}", path.display());
            return;
        }
    };
    let _ = conn.pragma_update(None, "journal_mode", "WAL");
    let _ = conn.pragma_update(None, "synchronous", "NORMAL"); // safe with WAL: a power cut can lose the last commits, never corrupt the file
    let _ = conn.pragma_update(None, "foreign_keys", "ON");
    migrate(&conn);
    let _ = DB.set(Mutex::new(conn));
}

pub(crate) fn migrate(c: &Connection) {
    let have: usize = c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap_or(0).max(0) as usize;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(have) {
        let script = format!("BEGIN;\n{sql}\nPRAGMA user_version = {};\nCOMMIT;", i + 1);
        if let Err(e) = c.execute_batch(&script) {
            eprintln!("store: migration {} failed: {e}", i + 1);
            let _ = c.execute_batch("ROLLBACK;");
            return;
        }
    }
}

/// Run `f` with the connection. Returns None if the store is unavailable or the query failed (logged, no data in the log).
pub fn with<R>(f: impl FnOnce(&Connection) -> rusqlite::Result<R>) -> Option<R> {
    let m = DB.get()?;
    let c = m.lock().ok()?;
    match f(&c) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("store: query failed: {e}");
            None
        }
    }
}

/// Current time as UTC epoch milliseconds (the unit of every *_at column).
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
