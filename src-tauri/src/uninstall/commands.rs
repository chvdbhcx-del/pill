//! The uninstaller panel: state of one run and the Tauri commands behind it.
use super::*;

// ---------------------------------------------------------------------------------------------------------------
// State, commands
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
pub(super) struct Flow {
    progs: Vec<Prog>,
    locs: Vec<(String, PathBuf)>, // every installed program's recorded install folder (for the "shared folder" check)
    sel: Option<Prog>,
    cands: Vec<Cand>,
    manual: Vec<String>,
    cut: bool,
    busy: bool,
    last: Value,
    /// The island card ("N leftovers to review") was pressed: the bar page opens the panel the next time it reads the state.
    focus: bool,
}
pub(super) static FLOW: Mutex<Option<Flow>> = Mutex::new(None);
pub(super) static APPX: Mutex<Option<(Instant, Vec<Prog>)>> = Mutex::new(None);
pub(super) static CANCEL: AtomicBool = AtomicBool::new(false);

pub(super) fn flow<R>(f: impl FnOnce(&mut Flow) -> R) -> R {
    let mut g = FLOW.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Flow::default))
}

/// Command bar trigger: `u foo` -> Some("foo"). The trigger word needs a space after it.
pub fn trigger(q: &str) -> Option<&str> {
    const WORDS: &[&str] = &["u", "del", "uninstall", "remove", "удалить", "удали", "удаление", "деинсталляция", "деинсталлировать", "снести"];
    let (w, rest) = q.split_once(char::is_whitespace)?;
    WORDS.contains(&w.to_lowercase().as_str()).then(|| rest.trim())
}

pub struct Row {
    pub kind: &'static str,
    pub id: String,
    pub title: String,
    pub sub: String,
    pub payload: String,
}

pub(super) fn fmt_size(kb: u64) -> String {
    match kb {
        0 => String::new(),
        k if k >= 1 << 20 => format!("{:.1} GB", k as f64 / (1 << 20) as f64),
        k if k >= 1 << 10 => format!("{} MB", k >> 10),
        k => format!("{k} KB"),
    }
}

/// Rows for `u <query>`. Reads the registry (fast) and the Store apps (cached for a minute, the first call takes a second or two).
pub fn rows(query: &str) -> Vec<Row> {
    let appx = {
        let mut g = APPX.lock().unwrap_or_else(|e| e.into_inner());
        if g.as_ref().is_none_or(|(t, _)| t.elapsed() > Duration::from_secs(60)) {
            *g = Some((Instant::now(), appx_list().unwrap_or_default()));
        }
        g.as_ref().map(|x| x.1.clone()).unwrap_or_default()
    };
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let want_risky = words.iter().any(|w| RISKY_WORDS.contains(&w.as_str()));
    let words: Vec<String> = words.into_iter().filter(|w| !RISKY_WORDS.contains(&w.as_str())).collect();
    let sd = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf));
    let mut all = read_programs();
    let locs: Vec<(String, PathBuf)> = all.iter().filter(|p| !p.location.is_empty()).map(|p| (p.sub.clone(), PathBuf::from(p.location.trim_end_matches('\\')))).collect();
    all.extend(appx);
    let list: Vec<(Prog, bool)> = all.into_iter().filter_map(|p| classify(&p, sd.as_deref()).map(|r| (p, r))).filter(|(_, r)| !r || want_risky).collect();
    let mut hits: Vec<(u8, usize)> = list
        .iter()
        .enumerate()
        .filter_map(|(i, (p, _))| {
            let hay = format!("{} {}", p.name, p.publisher).to_lowercase();
            let n = p.name.to_lowercase();
            let rank = if words.is_empty() { 3 } else if n.starts_with(words[0].as_str()) { 0 } else if n.contains(&format!(" {}", words[0])) { 1 } else { 2 };
            words.iter().all(|w| hay.contains(w.as_str())).then_some((rank, i))
        })
        .collect();
    hits.sort_by(|a, b| a.0.cmp(&b.0).then(list[b.1].0.size_kb.cmp(&list[a.1].0.size_kb)).then(list[a.1].0.name.len().cmp(&list[b.1].0.name.len())));
    hits.truncate(8);
    let mut out: Vec<Row> = hits
        .iter()
        .map(|&(_, i)| {
            let (p, risky) = &list[i];
            let sub = [p.publisher.clone(), p.version.clone(), fmt_size(p.size_kb), if p.appx { "Store".into() } else { String::new() }]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            Row { kind: "uninstall", id: format!("uninstall:{i}"), title: p.name.clone(), sub, payload: format!("{i}{}", if *risky { "!" } else { "" }) }
        })
        .collect();
    if out.is_empty() {
        out.push(Row { kind: "hint", id: "hint:uninstall".into(), title: "No installed app matches".into(), sub: "Runtimes and drivers show with the word: all".into(), payload: String::new() });
    }
    flow(|f| {
        if !f.busy {
            f.progs = list.into_iter().map(|x| x.0).collect();
            f.locs = locs;
        }
    });
    out
}

pub(super) fn emit(_app: &AppHandle, v: Value) {
    flow(|f| f.last = v.clone());
    let _ = bus::emit("uninstall:state", v);
}

pub(crate) fn items_json(cands: &[Cand]) -> Vec<Value> {
    cands
        .iter()
        .enumerate()
        .map(|(i, c)| json!({ "id": i, "kind": c.kind, "path": c.shown, "size": c.size, "ticked": c.ticked, "note": c.note, "admin": c.admin, "tier": if c.tier == Tier::Certain { "certain" } else { "probable" } }))
        .collect()
}

/// The review state the page draws: the candidate list with everything needed for the two groups and the administrator section.
pub(super) fn review_json(name: &str, cands: &[Cand], manual: &[String], cut: bool, extra: Value) -> Value {
    let mut v = json!({ "phase": if cands.is_empty() && manual.is_empty() { "done" } else { "review" }, "name": name, "items": items_json(cands), "manual": manual, "cut": cut,
        "needAdmin": cands.iter().filter(|c| c.admin).count(), "files": 0, "keys": 0, "other": 0, "failed": [] });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    v
}

/// Details for the confirm card; also selects the app for the following commands. `name` must match the row (the list may have moved on).
pub fn uninstall_info(id: usize, name: String) -> Value {
    flow(|f| {
        if f.busy {
            return json!({ "ok": false, "error": "busy" });
        }
        let Some(p) = f.progs.get(id).filter(|p| p.name == name).cloned() else { return json!({ "ok": false, "error": "stale" }) };
        let v = json!({ "ok": true, "name": p.name, "publisher": p.publisher, "version": p.version, "location": p.location, "sizeKb": p.size_kb,
            "store": p.appx, "canSilent": can_silent(&p), "risky": is_risky(&p) });
        f.sel = Some(p);
        f.cands.clear();
        v
    })
}

/// ShellExecute the uninstaller (its own window / UAC) and wait for it. Never kills it. Ok(false) = stopped waiting.
pub(super) fn run_and_wait(file: &str, args: &str) -> Result<bool, String> {
    let (f, a): (Vec<u16>, Vec<u16>) = (file.encode_utf16().chain(Some(0)).collect(), args.encode_utf16().chain(Some(0)).collect());
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        hwnd: HWND::default(),
        lpVerb: w!("open"),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: if args.is_empty() { PCWSTR::null() } else { PCWSTR(a.as_ptr()) },
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    unsafe { ShellExecuteExW(&mut sei).map_err(|e| format!("Windows could not start the uninstaller ({e})"))? };
    let h = sei.hProcess;
    if h.is_invalid() {
        return Ok(true);
    }
    let t = Instant::now();
    let mut waited = true;
    while unsafe { WaitForSingleObject(h, 250) }.0 != 0 {
        if CANCEL.load(Ordering::Relaxed) || t.elapsed() > WAIT_MAX {
            waited = false;
            break;
        }
    }
    unsafe {
        let _ = CloseHandle(h);
    }
    Ok(waited)
}

/// Runs a worker-thread body; a panic is caught and its message returned (so `busy` can be reset and the user told).
pub fn catch_body(f: impl FnOnce()) -> Option<String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).err().map(|p| {
        p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown error".into())
    })
}

/// A thread for one step of the flow: whatever happens in `f`, `busy` is reset and the page gets an error instead of a panel that spins forever.
pub(super) fn spawn_guarded(app: AppHandle, name: String, f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        if let Some(msg) = catch_body(f) {
            flow(|fl| fl.busy = false);
            emit(&app, json!({ "phase": "error", "name": name, "message": format!("internal error: {msg}") }));
            notify_ready(&app, &name, "error", 0);
        }
    });
}

pub(super) fn begin() -> Option<Prog> {
    CANCEL.store(false, Ordering::Relaxed);
    flow(|f| if f.busy { None } else { f.sel.clone().inspect(|_| f.busy = true) })
}

pub fn uninstall_start(app: AppHandle, silent: bool) -> Value {
    let Some(p) = begin() else { return json!({ "ok": false, "error": "nothing selected or busy" }) };
    spawn_guarded(app.clone(), p.name.clone(), move || {
        emit(&app, json!({ "phase": "running", "name": p.name }));
        let res: Result<bool, String> = if p.appx {
            if !valid_full_name(&p.sub) {
                Err("bad package name".into())
            } else {
                ps(&format!("Remove-AppxPackage -Package '{}'", p.sub), 300);
                Ok(true)
            }
        } else {
            build_cmd(&p, silent).map_err(String::from).and_then(|(f, a)| run_and_wait(&f, &a))
        };
        match res {
            Err(e) => {
                flow(|f| f.busy = false);
                emit(&app, json!({ "phase": "error", "name": p.name, "message": e }));
                notify_ready(&app, &p.name, "error", 0);
            }
            Ok(waited) => settle_and_scan(&app, &p, waited),
        }
    });
    json!({ "ok": true })
}

/// After the uninstaller process ended: wait for the entry to vanish, then scan; or report that it is still installed.
pub(super) fn settle_and_scan(app: &AppHandle, p: &Prog, waited: bool) {
    let t = Instant::now();
    while waited && still_installed(p) && t.elapsed() < SETTLE_MAX && !CANCEL.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
    }
    if still_installed(p) {
        flow(|f| f.busy = false);
        emit(app, json!({ "phase": "still", "name": p.name }));
        notify_ready(app, &p.name, "still", 0);
    } else if p.appx {
        flow(|f| f.busy = false);
        emit(app, review_json(&p.name, &[], &[], false, json!({})));
        notify_ready(app, &p.name, "done", 0);
    } else {
        emit(app, json!({ "phase": "scanning", "name": p.name, "step": "starting", "found": 0 }));
        let (cands, manual, cut) = scan_all(app, p);
        let v = review_json(&p.name, &cands, &manual, cut, json!({}));
        let found = cands.len();
        CANCEL.store(false, Ordering::Relaxed); // a stopped scan must not make the deletion that follows refuse its size checks
        flow(|f| {
            f.cands = cands;
            f.manual = manual;
            f.cut = cut;
            f.busy = false;
        });
        emit(app, v);
        notify_ready(app, &p.name, if found == 0 { "done" } else { "review" }, found);
    }
}

/// "Check again": the user finished the uninstaller by hand (or it handed over to another process).
pub fn uninstall_scan(app: AppHandle) -> Value {
    let Some(p) = begin() else { return json!({ "ok": false }) };
    spawn_guarded(app.clone(), p.name.clone(), move || settle_and_scan(&app, &p, false));
    json!({ "ok": true })
}

pub fn uninstall_delete(app: AppHandle, ids: Vec<usize>) -> Value {
    CANCEL.store(false, Ordering::Relaxed);
    let Some((p, picked)) = flow(|f| {
        if f.busy {
            return None;
        }
        let p = f.sel.clone()?;
        let mut ids = ids.clone();
        ids.sort_unstable();
        ids.dedup();
        let picked: Vec<Cand> = ids.iter().filter_map(|i| f.cands.get(*i).cloned()).filter(|c| !c.admin).collect(); // unknown indexes and administrator items are dropped
        f.busy = true;
        Some((p, picked))
    }) else {
        return json!({ "ok": false });
    };
    spawn_guarded(app.clone(), p.name.clone(), move || {
        emit(&app, json!({ "phase": "deleting", "name": p.name, "total": picked.len() }));
        let env = Env::real();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
        let (dir, base) = (backup_dir(&env), format!("{}-{stamp}", sanitize(&p.name)));
        let refs: Vec<&Cand> = picked.iter().collect();
        let r = delete_cands(&p, &refs, &env, &RealReg, &stamp, false, &|name, bytes| file_store(&dir, &base, name, bytes), &|shown, ok, err, n, total| {
            let _ = bus::emit("uninstall:item", json!({ "shown": shown, "ok": ok, "error": err, "n": n, "total": total }));
        });
        finish_delete(&app, &p, &r, &dir);
    });
    json!({ "ok": true })
}

/// Drops what was deleted from the list and emits the summary (the rest, administrator items included, stays tickable).
pub(crate) fn finish_delete(app: &AppHandle, p: &Prog, r: &Report, dir: &Path) {
    let gone: std::collections::HashSet<String> = r.done.iter().map(|(_, s)| s.clone()).collect();
    let (cands, manual, cut) = flow(|f| {
        f.busy = false;
        f.cands.retain(|c| !gone.contains(&c.shown));
        (f.cands.clone(), f.manual.clone(), f.cut)
    });
    let count = |kinds: &[&str]| r.done.iter().filter(|(k, _)| kinds.contains(k)).count();
    let failed: Vec<String> = r.failed.iter().map(|(a, b)| format!("{a}: {b}")).collect();
    let v = review_json(
        &p.name,
        &cands,
        &manual,
        cut,
        json!({ "phase": "done", "files": count(&["folder", "file", "shortcut"]), "keys": count(&["key", "value", "autorun"]), "other": count(&["service", "task", "item"]),
            "failed": failed, "skippedAdmin": r.need_admin, "backup": r.backup, "log": r.log, "backupDir": dir.to_string_lossy() }),
    );
    emit(app, v);
    notify_ready(app, &p.name, "done", r.done.len());
}

/// Opens the folder with the .reg backups and logs in Explorer.
pub fn uninstall_open_backups() {
    let dir = backup_dir(&Env::real());
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new(windows_dir().join("explorer.exe")).arg(&dir).creation_flags(NO_WINDOW).spawn();
}

/// Stops waiting for the uninstaller (it keeps running) and forgets the selection.
pub fn uninstall_cancel() {
    CANCEL.store(true, Ordering::Relaxed);
    flow(|f| {
        if !f.busy {
            f.sel = None;
            f.cands.clear();
            f.last = Value::Null;
        }
    });
}

/// The last state event, for a bar that was hidden while the uninstaller ran.
pub fn uninstall_state() -> Value {
    flow(|f| {
        let mut v = f.last.clone();
        if let (Some(o), true) = (v.as_object_mut(), std::mem::take(&mut f.focus)) {
            o.insert("focus".into(), json!(true));
        }
        v
    })
}

/// The island card's button: show the bar (when it is closed) and bring the uninstall panel back where it was.
pub fn uninstall_open_panel(app: AppHandle) {
    flow(|f| f.focus = !f.last.is_null());
    if crate::command::bar_visible(&app) {
        let _ = bus::emit("uninstall:focus", json!({}));
        flow(|f| f.focus = false);
    } else {
        crate::command::show(&app);
    }
}

/// A step finished (leftovers found, summary ready, still installed, error): the island shows a small card, unless the bar is open (its strip tells).
/// The panel itself is never forced open: the user may be searching for something else.
pub(super) fn notify_ready(app: &AppHandle, name: &str, kind: &str, count: usize) {
    if crate::command::bar_visible(app) {
        return;
    }
    let _ = bus::emit("uninstall:ready", json!({ "name": name, "kind": kind, "count": count }));
}

// ---------------------------------------------------------------------------------------------------------------
// Real scan: ties the pure pieces to the disk and the registry
// ---------------------------------------------------------------------------------------------------------------

pub(super) fn lnk_target(p: &Path) -> Option<PathBuf> {
    use windows::core::Interface;
    use windows::Win32::Storage::FileSystem::WIN32_FIND_DATAW;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let w: Vec<u16> = p.to_string_lossy().encode_utf16().chain(Some(0)).collect();
        pf.Load(PCWSTR(w.as_ptr()), STGM_READ).ok()?;
        let mut buf = [0u16; 520];
        let mut fd = WIN32_FIND_DATAW::default();
        link.GetPath(&mut buf, &mut fd, 0).ok()?;
        let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        let s = String::from_utf16_lossy(&buf[..n]);
        (!s.is_empty()).then(|| PathBuf::from(expand_env(&s)))
    }
}

/// The whole deep scan of `p`: (candidates, lines that are only listed, cut off by the caps). `step(stage, found so far)` reports progress.
/// Used by this process and by the elevated one, so both derive the same list.
pub fn scan_all_with(p: &Prog, env: &Env, reg: &dyn Reg, shared: bool, lnk: &dyn Fn(&Path) -> Option<PathBuf>, step: &dyn Fn(&str, usize)) -> (Vec<Cand>, Vec<String>, bool) {
    let mut b = Budget::new(400_000, 60);
    step("files and folders", 0);
    let mut cands = scan_fs(p, env, shared, lnk, &mut b);
    let n = cands.len();
    cands.extend(scan_registry(p, env, reg, &mut b, &|s, c| step(s, n + c)));
    let mut seen = std::collections::HashSet::new();
    cands.retain(|c| seen.insert(cand_key(&c.what)));
    let kind_rank = |k: &str| ["folder", "file", "shortcut", "autorun", "key", "value", "service", "task", "item"].iter().position(|x| *x == k).unwrap_or(9);
    cands.sort_by_key(|c| (c.admin, c.tier != Tier::Certain, kind_rank(c.kind)));
    let drivers = list_drivers(p, env, reg);
    (cands, drivers, b.cut)
}

pub(super) fn scan_all(app: &AppHandle, p: &Prog) -> (Vec<Cand>, Vec<String>, bool) {
    let env = Env::real();
    let install = p.install();
    let shared = install.as_ref().is_some_and(|i| flow(|f| f.locs.iter().any(|(k, l)| *k != p.sub && (depth_under(l, i).is_some() || depth_under(i, l).is_some()))));
    let name = p.name.clone();
    scan_all_with(p, &env, &RealReg, shared, &lnk_target, &|s, c| emit(app, json!({ "phase": "scanning", "name": name, "step": s, "found": c })))
}

pub(super) fn static_kind(k: &str) -> &'static str {
    ["folder", "file", "shortcut", "key", "value", "autorun", "service", "task"].iter().find(|x| **x == k).copied().unwrap_or("item")
}

/// "Scan as administrator" (`mode` scan: finds what only an elevated process can see) or "Remove system leftovers" (`mode` delete: the ticked
/// administrator items). Runs in the background; the outcome arrives as `uninstall:state`.
pub fn uninstall_admin(app: AppHandle, mode: String, ids: Vec<usize>) -> Value {
    let mode = if mode == "delete" { Mode::Delete } else { Mode::Scan };
    let Some((p, keys)) = flow(|f| {
        if f.busy {
            return None;
        }
        let p = f.sel.clone()?;
        let keys: Vec<String> = ids.iter().filter_map(|i| f.cands.get(*i)).filter(|c| c.admin).map(|c| cand_key(&c.what)).collect();
        if mode == Mode::Delete && keys.is_empty() {
            return None;
        }
        f.busy = true;
        Some((p, keys))
    }) else {
        return json!({ "ok": false });
    };
    spawn_guarded(app.clone(), p.name.clone(), move || {
        emit(&app, json!({ "phase": "elevating", "name": p.name, "mode": if mode == Mode::Scan { "scan" } else { "delete" } }));
        let env = Env::real();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
        let (dir, base) = (backup_dir(&env), format!("{}-{stamp}-admin", sanitize(&p.name)));
        let r = run_elevated(&p, mode, keys, &|name, bytes| file_store(&dir, &base, name, bytes));
        let (cands, manual, cut) = flow(|f| (f.cands.clone(), f.manual.clone(), f.cut));
        match r {
            Err(e) => {
                flow(|f| f.busy = false);
                emit(&app, review_json(&p.name, &cands, &manual, cut, json!({ "phase": "review", "adminError": e })));
            }
            Ok(res) if res.get("error").is_some() => {
                flow(|f| f.busy = false);
                emit(&app, review_json(&p.name, &cands, &manual, cut, json!({ "phase": "review", "adminError": res["error"] })));
            }
            Ok(res) if mode == Mode::Scan => {
                let tier_of = |s: &str| if s == "certain" { Tier::Certain } else { Tier::Probable };
                let remote: Vec<Cand> = res["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|j| {
                                let g = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                                let t = tier_of(&g("tier"));
                                Cand { kind: static_kind(&g("kind")), shown: g("shown"), size: j.get("size").and_then(Value::as_u64).unwrap_or(0), tier: t, ticked: t == Tier::Certain, note: g("note"), admin: true, what: What::Remote(g("key")) }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let manual2: Vec<String> = res["manual"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or(manual);
                let all = flow(|f| {
                    f.cands.retain(|c| !c.admin);
                    f.cands.extend(remote);
                    f.manual = manual2.clone();
                    f.busy = false;
                    f.cands.clone()
                });
                emit(&app, review_json(&p.name, &all, &manual2, cut, json!({ "phase": "review", "adminScanned": true, "hiddenOthers": res["hiddenOthers"].as_u64().unwrap_or(0) })));
            }
            Ok(res) => {
                let pairs = |k: &str| -> Vec<(String, String)> {
                    res[k].as_array().map(|a| a.iter().map(|x| (x[0].as_str().unwrap_or("").to_string(), x[1].as_str().unwrap_or("").to_string())).collect()).unwrap_or_default()
                };
                let rep = Report {
                    done: pairs("done").into_iter().map(|(k, s)| (static_kind(&k), s)).collect(),
                    failed: pairs("failed"),
                    backup: res["backup"].as_str().map(str::to_string),
                    log: res["log"].as_str().map(str::to_string),
                    need_admin: 0,
                };
                finish_delete(&app, &p, &rep, &dir);
            }
        }
    });
    json!({ "ok": true })
}
