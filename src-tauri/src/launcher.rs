// Home launcher: icons of programs / folders / files the user dropped on the Home tab. Owner: launcher lane.
// Part of the Pill replica (user request: "Home launcher, output devices and clipboard/shelf behaviours from the video").
// Storage: launcher_groups / launcher_links (migration V1). One implicit group (the lowest id) holds the Home row;
// `sort` is the position. Targets are canonical paths, opened with ShellExecute (no shell string); icons come from
// the shelf lane's IShellItemImageFactory pipeline (shelf::icon_for).
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// Most links the Home row keeps (a row of 10 fits the video; more would scroll).
const MAX_LINKS: usize = 40;

#[derive(Serialize, Clone)]
pub struct Link {
    id: i64,
    title: String,
    target: String,
    missing: bool,
    is_dir: bool,
    icon: Option<String>,
}

pub fn init(_app: &tauri::App) {
    group_id();
}

/// The one group of the Home row; created on first use.
fn group_id() -> Option<i64> {
    crate::store::with(|c| {
        c.execute("insert into launcher_groups (name, sort) select 'home', 0 where not exists (select 1 from launcher_groups)", [])?;
        c.query_row("select min(id) from launcher_groups", [], |r| r.get::<_, i64>(0))
    })
}

/// canonicalize() gives `\\?\C:\x`; the shell and the UI want `C:\x`.
fn clean(p: &Path) -> Option<String> {
    let s = std::fs::canonicalize(p).ok()?.to_string_lossy().into_owned();
    Some(if let Some(r) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{r}")
    } else {
        s.strip_prefix(r"\\?\").map(str::to_owned).unwrap_or(s)
    })
}

/// Display name: file name without extension (`Chrome.lnk` -> `Chrome`), folders keep their name, drive roots the path.
fn title_of(path: &str) -> String {
    let p = Path::new(path);
    let name = if p.is_dir() { p.file_name() } else { p.file_stem() };
    name.map(|n| n.to_string_lossy().into_owned()).filter(|n| !n.is_empty()).unwrap_or_else(|| path.to_string())
}

/// New order after moving `id` to `new_index` (clamped). Unknown id: order unchanged.
fn reorder(mut ids: Vec<i64>, id: i64, new_index: usize) -> Vec<i64> {
    if let Some(from) = ids.iter().position(|x| *x == id) {
        ids.remove(from);
        ids.insert(new_index.min(ids.len()), id);
    }
    ids
}

fn rows() -> Vec<(i64, String, String)> {
    crate::store::with(|c| {
        let mut st = c.prepare("select id, title, target from launcher_links order by sort, id")?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<rusqlite::Result<Vec<_>>>();
        v
    })
    .unwrap_or_default()
}

/// Microsoft Store apps are not files: their target is the shell parsing name `shell:AppsFolder\<AppUserModelID>`.
const STORE_PREFIX: &str = r"shell:AppsFolder\";

fn is_store(target: &str) -> bool {
    target.len() > STORE_PREFIX.len() && target.len() <= 300 && target.get(..STORE_PREFIX.len()).is_some_and(|p| p.eq_ignore_ascii_case(STORE_PREFIX)) && !target.chars().any(|c| c.is_control())
}

fn links() -> Vec<Link> {
    rows()
        .into_iter()
        .map(|(id, title, target)| {
            if is_store(&target) {
                return Link { id, title, missing: false, is_dir: false, icon: crate::shelf::bar_icon(&target), target };
            }
            let meta = std::fs::metadata(&target).ok();
            Link { id, title, missing: meta.is_none(), is_dir: meta.as_ref().map_or(false, |m| m.is_dir()), icon: meta.and_then(|_| crate::shelf::icon_for(&target)), target }
        })
        .collect()
}

fn changed(app: &AppHandle) -> Vec<Link> {
    let v = links();
    let _ = app.emit_to("pill", "launcher:changed", serde_json::json!({ "count": v.len(), "items": &v }));
    v
}

#[tauri::command(async)]
pub fn launcher_list() -> Vec<Link> {
    links()
}

/// Adds existing files / folders / shortcuts (duplicates and non-existent paths are skipped). Returns how many were added.
#[tauri::command(async)]
pub fn launcher_add(app: AppHandle, paths: Vec<String>) -> usize {
    if !crate::settings::get_bool("module.launcher", true) {
        return 0; // Settings > Modules: the launcher is off
    }
    let Some(gid) = group_id() else { return 0 };
    let mut added = 0;
    for p in paths {
        if p.trim().is_empty() || p.len() > 1024 {
            continue;
        }
        let Some(target) = clean(Path::new(&p)) else { continue };
        let title = title_of(&target);
        let ins = crate::store::with(|c| {
            let n: i64 = c.query_row("select count(*) from launcher_links", [], |r| r.get(0))?;
            let dup: i64 = c.query_row("select count(*) from launcher_links where target = ?1 collate nocase", [&target], |r| r.get(0))?;
            if dup > 0 || n as usize >= MAX_LINKS {
                return Ok(0);
            }
            let next: i64 = c.query_row("select coalesce(max(sort) + 1, 0) from launcher_links", [], |r| r.get(0))?;
            c.execute("insert into launcher_links (group_id, title, target, sort) values (?1, ?2, ?3, ?4)", rusqlite::params![gid, title, target, next])
        });
        added += ins.unwrap_or(0);
    }
    if added > 0 {
        changed(&app);
    }
    added
}

/// Pins a Microsoft Store app (`shell:AppsFolder\...` as the command bar lists it) under the name the user saw. Same limits as `launcher_add`.
pub(crate) fn add_store(app: &AppHandle, target: &str, title: &str) -> usize {
    if !crate::settings::get_bool("module.launcher", true) || !is_store(target) {
        return 0;
    }
    let Some(gid) = group_id() else { return 0 };
    let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();
    let title = if title.trim().is_empty() { "App".to_string() } else { title.trim().to_string() };
    let ins = crate::store::with(|c| {
        let n: i64 = c.query_row("select count(*) from launcher_links", [], |r| r.get(0))?;
        let dup: i64 = c.query_row("select count(*) from launcher_links where target = ?1 collate nocase", [target], |r| r.get(0))?;
        if dup > 0 || n as usize >= MAX_LINKS {
            return Ok(0);
        }
        let next: i64 = c.query_row("select coalesce(max(sort) + 1, 0) from launcher_links", [], |r| r.get(0))?;
        c.execute("insert into launcher_links (group_id, title, target, sort) values (?1, ?2, ?3, ?4)", rusqlite::params![gid, title, target, next])
    });
    let added = ins.unwrap_or(0);
    if added > 0 {
        changed(app);
    }
    added
}

#[tauri::command(async)]
pub fn launcher_remove(app: AppHandle, id: i64) {
    crate::store::with(|c| c.execute("delete from launcher_links where id = ?1", [id]));
    changed(&app);
}

/// Starts the target through ShellExecute("open") with its canonical path as the file argument (never a shell string);
/// the working directory is the target's folder, so programs find their own files.
#[tauri::command(async)]
pub fn launcher_open(id: i64) -> bool {
    let Some(t) = crate::store::with(|c| c.query_row("select target from launcher_links where id = ?1", [id], |r| r.get::<_, String>(0))) else { return false };
    let p = Path::new(&t);
    if !is_store(&t) && !p.exists() {
        return false;
    }
    let dir = if is_store(&t) || p.is_dir() { None } else { p.parent().map(|d| d.to_string_lossy().into_owned()) };
    unsafe {
        let dir_h = dir.as_deref().map(HSTRING::from);
        let dir_p = dir_h.as_ref().map_or(PCWSTR::null(), |h| PCWSTR(h.as_ptr()));
        // values > 32 mean success
        ShellExecuteW(HWND::default(), &HSTRING::from("open"), &HSTRING::from(t.as_str()), PCWSTR::null(), dir_p, SW_SHOWNORMAL).0 as isize > 32
    }
}

/// Moves one link to `new_index` (0-based) and renumbers `sort`.
#[tauri::command(async)]
pub fn launcher_move(app: AppHandle, id: i64, new_index: usize) {
    let ids = reorder(rows().into_iter().map(|r| r.0).collect(), id, new_index);
    crate::store::with(|c| {
        let tx = c.unchecked_transaction()?;
        for (i, lid) in ids.iter().enumerate() {
            tx.execute("update launcher_links set sort = ?1 where id = ?2", rusqlite::params![i as i64, lid])?;
        }
        tx.commit()
    });
    changed(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_moves_and_clamps() {
        assert_eq!(reorder(vec![1, 2, 3, 4], 1, 2), vec![2, 3, 1, 4]);
        assert_eq!(reorder(vec![1, 2, 3, 4], 4, 0), vec![4, 1, 2, 3]);
        assert_eq!(reorder(vec![1, 2, 3], 2, 99), vec![1, 3, 2]); // clamped to the end
        assert_eq!(reorder(vec![1, 2, 3], 7, 0), vec![1, 2, 3]); // unknown id: unchanged
        assert_eq!(reorder(vec![], 1, 0), Vec::<i64>::new());
    }

    #[test]
    fn titles() {
        assert_eq!(title_of(r"C:\Users\x\Chrome.lnk"), "Chrome");
        assert_eq!(title_of(r"C:\Program Files\Steam\steam.exe"), "steam");
        assert_eq!(title_of(r"C:\"), r"C:\"); // drive root keeps its path
        assert_eq!(title_of(r"C:\no\such\archive.tar.gz"), "archive.tar");
    }

    #[test]
    fn clean_rejects_missing_and_strips_prefix() {
        assert!(clean(Path::new(r"C:\definitely\not\here\x.exe")).is_none());
        let c = clean(Path::new(&std::env::temp_dir())).unwrap();
        assert!(!c.starts_with(r"\\?\"));
    }

    #[test]
    fn store_targets_are_recognised_strictly() {
        assert!(is_store(r"shell:AppsFolder\Microsoft.WindowsNotepad_8wekyb3d8bbwe!App"));
        assert!(is_store(r"SHELL:appsfolder\Some.App_abc!App"));
        for bad in ["", r"shell:AppsFolder\", r"shell:AppsFolder", r"C:\Windows\notepad.exe", "shell:Startup\\x", "shell:AppsFolder\\a\nb", "приложение-приложение-приложение"] {
            assert!(!is_store(bad), "{bad:?}");
        }
        assert!(!is_store(&format!(r"shell:AppsFolder\{}", "x".repeat(300)))); // too long
    }
}
