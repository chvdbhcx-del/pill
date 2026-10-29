// Removing the leftovers that need administrator rights (HKLM keys, services, scheduled tasks, firewall rules, Program Files / ProgramData files,
// other people's profiles). Pill relaunches ITSELF elevated with a fixed argument form `--uninstall-admin <port>:<token>` (Windows shows its own
// permission prompt). `main()` hands that launch to `admin_main` before anything else starts, so the elevated copy is a small headless worker.
//
// Trust model (read this before changing anything):
//  * The unelevated Pill and the worker talk over a loopback socket the unelevated side opened (nothing is written to disk by the worker, so
//    there is no file or junction an unprivileged process could redirect). The worker hands its backups to the unelevated side, which stores them.
//    The listener accepts clients until one presents the session token (a random 128-bit value from BCryptGenRandom); anyone else is dropped.
//  * The request carries an app IDENTITY (name, publisher, install folder) and a list of item KEYS. No GUID, no registry key name: the worker finds
//    the product's GUIDs itself (a dead Uninstall record, or Windows Installer records whose install folder is the validated one). The worker
//    never deletes a path or a key it was given: it validates the identity (`validate_req`), re-derives the candidate list with its own scan (same
//    code, same `guard_cand`), shows a NATIVE dialog listing what it is about to remove, and deletes only the ticked candidates after OK.
//  * The identity is refused when: the name is short or generic, the install folder is not a believable app folder (system roots, deny-listed
//    system trees, a user's known folders, a sync folder, a generic name, not on a fixed drive), or the app is still registered as installed (an
//    Uninstall entry that is alive and shares its GUID / name / folder / product family / DisplayIcon).
//  * Before the permission prompt the unelevated side verifies the exe it is about to elevate: pill.exe is opened at startup with
//    FILE_SHARE_READ only (nobody can write, rename or delete it while Pill runs), hashed from that handle, and the file at the exact path handed
//    to ShellExecute is re-hashed and compared. The worker loads DLLs from System32 only and starts system tools by absolute path.
//  * Residual risk (documented, not removable here): Pill is unsigned and lives in a user-writable folder. Anything that changed pill.exe BEFORE
//    this Pill started is trusted by the hash. Any process of the same user can start `pill.exe --uninstall-admin` itself and run a listener; the
//    permission prompt and the worker's own dialog are then the only gates.
use super::*;
use base64::Engine;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::sync::{Arc, OnceLock};

pub const ARG: &str = "--uninstall-admin";
const MAX_LINE: u64 = 16 << 20;
const KEYS_MAX: usize = 5_000;
const SILENCE_MAX: Duration = Duration::from_secs(60); // no message (not even a heartbeat) from the helper for this long: it is gone

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Scan,
    Delete,
}

/// What the unelevated side sends: an identity and, for a delete, the item keys the user ticked. Nothing else (no GUID, no key name).
#[derive(Clone, Debug)]
pub struct Req {
    pub mode: Mode,
    pub name: String,
    pub publisher: String,
    pub location: String,
    pub keys: Vec<String>,
}

impl Req {
    pub fn of(p: &Prog, mode: Mode, keys: Vec<String>) -> Req {
        Req { mode, name: p.name.clone(), publisher: p.publisher.clone(), location: p.location.clone(), keys }
    }
    pub fn to_json(&self) -> Value {
        json!({ "v": 2, "mode": if self.mode == Mode::Scan { "scan" } else { "delete" }, "name": self.name, "publisher": self.publisher, "location": self.location, "keys": self.keys })
    }
    pub fn from_json(v: &Value) -> Result<Req, &'static str> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        if v.get("v").and_then(Value::as_u64) != Some(2) {
            return Err("unknown request version");
        }
        let mode = match v.get("mode").and_then(Value::as_str) {
            Some("scan") => Mode::Scan,
            Some("delete") => Mode::Delete,
            _ => return Err("unknown mode"),
        };
        let keys = v.get("keys").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default();
        Ok(Req { mode, name: s("name"), publisher: s("publisher"), location: s("location"), keys })
    }
}

fn clean(s: &str, max: usize) -> bool {
    s.chars().count() <= max && !s.chars().any(char::is_control)
}

/// Every Uninstall entry of both hives (HKLM, HKCU, WOW6432Node): (key leaf, values).
fn uninstall_entries(reg: &dyn Reg) -> Vec<(String, Vals)> {
    let mut out = Vec::new();
    for h in [Hive::Hklm, Hive::Hkcu] {
        for base in [UNINSTALL.to_string(), UNINSTALL.replace("SOFTWARE", r"SOFTWARE\WOW6432Node")] {
            for leaf in reg.subkeys(h, &base) {
                let v = reg.values(h, &format!(r"{base}\{leaf}"));
                out.push((leaf, v));
            }
        }
    }
    out
}

fn vstr_pub(v: &Vals, k: &str) -> String {
    match v.get(k) {
        Some(Val::S(s)) | Some(Val::E(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

/// The folder / file paths one Uninstall entry points at: InstallLocation, the uninstaller, the DisplayIcon file.
fn entry_paths(v: &Vals) -> Vec<PathBuf> {
    let loc = vstr_pub(v, "installlocation");
    let unin = split_cmd(&expand_env(&vstr_pub(v, "uninstallstring"))).map(|x| x.0).unwrap_or_default();
    let icon = expand_env(&vstr_pub(v, "displayicon"));
    let icon = icon.trim().trim_matches('"').rsplit_once(',').filter(|(_, n)| n.trim().trim_start_matches('-').chars().all(|c| c.is_ascii_digit())).map_or(icon.trim().trim_matches('"').to_string(), |(a, _)| a.trim_matches('"').to_string());
    [loc.trim_matches('"').trim_end_matches('\\').to_string(), unin, icon].into_iter().filter(|s| !s.is_empty()).map(PathBuf::from).collect()
}

/// Is the entry's program still there? Its uninstaller file exists; msiexec / script entries: their folder still holds something; or its icon file exists.
fn entry_alive(v: &Vals) -> bool {
    let cmd = vstr_pub(v, "uninstallstring");
    let unin = split_cmd(&expand_env(&cmd)).map(|x| x.0).unwrap_or_default();
    let loc = vstr_pub(v, "installlocation");
    let l = Path::new(loc.trim_matches('"').trim_end_matches('\\'));
    let by_cmd = if cmd.to_ascii_lowercase().contains("msiexec") || unin.is_empty() { loc.is_empty() || (l.is_dir() && !is_empty_dir(l)) } else { Path::new(&unin).is_file() };
    by_cmd || entry_paths(v).last().is_some_and(|i| i.is_file() && !vstr_pub(v, "displayicon").is_empty())
}

/// The first folder below a machine-wide or per-user programs root: `C:\Program Files\Foo Editor` for anything inside it. Two paths in the same one
/// belong to the same product family.
fn family_dir(p: &Path, env: &Env) -> Option<PathBuf> {
    let roots = [env.pf.clone(), env.pf86.clone(), env.programdata.clone(), env.local.join("Programs"), env.local.clone(), env.appdata.clone()];
    roots.iter().filter(|r| depth_under(p, r).is_some_and(|d| d >= 1)).max_by_key(|r| r.components().count()).map(|r| p.components().take(r.components().count() + 1).collect())
}

/// Is any registered program this app (same GUID / name / short name) or living in the same folder or product family?
fn still_registered(p: &Prog, install: Option<&Path>, env: &Env, reg: &dyn Reg) -> bool {
    let (names, guids) = (names_of(p), p.guids());
    let fam = install.and_then(|i| family_dir(i, env));
    uninstall_entries(reg).iter().any(|(leaf, v)| {
        if !entry_alive(v) {
            return false; // a dead entry (its program is gone) is a leftover, not an installed app
        }
        let dn = vstr_pub(v, "displayname");
        let same = guids.contains(&leaf.to_uppercase()) || names.product.contains(&norm(&dn)) || names.product.contains(&norm(&short_name(&dn)));
        let paths = entry_paths(v);
        let overlap = install.is_some_and(|i| paths.iter().any(|x| depth_under(x, i).is_some() || depth_under(i, x).is_some()));
        let family = fam.as_ref().is_some_and(|f| paths.iter().filter_map(|x| family_dir(x, env)).any(|g| comps(&g) == comps(f)));
        same || overlap || family
    })
}

/// The product's GUIDs, found by the worker itself: a DEAD Uninstall record that is this app's (name, or its files in the install folder), or a
/// Windows Installer record whose own install folder is the validated one. A request can never supply a GUID.
pub fn derive_guids(p: &Prog, install: Option<&Path>, reg: &dyn Reg) -> Vec<String> {
    let names = names_of(p);
    let mut out: Vec<String> = Vec::new();
    for (leaf, v) in uninstall_entries(reg) {
        if entry_alive(&v) {
            continue;
        }
        let mine = names.product.contains(&norm(&vstr_pub(&v, "displayname"))) || entry_paths(&v).iter().any(|x| install.is_some_and(|i| depth_under(x, i).is_some()));
        if mine {
            out.extend(find_guid(&leaf));
            out.extend(find_guid(&vstr_pub(&v, "uninstallstring")));
        }
    }
    if let Some(i) = install {
        let ud = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData";
        for sid in reg.subkeys(Hive::Hklm, ud) {
            for pk in reg.subkeys(Hive::Hklm, &format!(r"{ud}\{sid}\Products")) {
                let v = reg.values(Hive::Hklm, &format!(r"{ud}\{sid}\Products\{pk}\InstallProperties"));
                if ["installlocation", "uninstallstring", "displayicon"].iter().any(|k| points_into(&vstr_pub(&v, k), Some(i))) {
                    out.extend(unpack_guid(&pk));
                }
            }
        }
    }
    out.iter_mut().for_each(|g| *g = g.to_uppercase());
    out.sort();
    out.dedup();
    out
}

/// Turns a request into the identity the worker scans for, or says why it must not run. The worker calls this before it does anything.
pub fn validate_req(r: &Req, env: &Env, reg: &dyn Reg) -> Result<Prog, &'static str> {
    if r.keys.len() > KEYS_MAX || r.keys.iter().any(|k| k.len() > 700 || !clean(k, 700)) {
        return Err("too many or malformed item keys");
    }
    if !clean(&r.name, 200) || !clean(&r.publisher, 200) || !clean(&r.location, 520) {
        return Err("malformed identity");
    }
    let mut p = Prog { name: r.name.trim().to_string(), publisher: r.publisher.trim().to_string(), location: r.location.trim().to_string(), ..Default::default() };
    let names = names_of(&p);
    // administrator work needs a distinctive name: 4+ letters, not a generic word
    if names.product.is_empty() || !names.product.iter().any(|n| n.chars().count() >= 4) {
        return Err("the app name is too short or too generic");
    }
    let install = p.install();
    if let Some(i) = &install {
        if !install_usable(i, env) || !reparse_free(i) {
            return Err("the install folder is not a believable app folder");
        }
    }
    if still_registered(&p, install.as_deref(), env, reg) {
        return Err("the app is still installed");
    }
    p.extra_guids = derive_guids(&p, install.as_deref(), reg);
    Ok(p)
}

// ---------------------------------------------------------------------------------------------------------------
// The native confirmation of the worker
// ---------------------------------------------------------------------------------------------------------------

/// What the worker's dialog says: the app, a count per kind, the first items it will remove.
pub fn confirm_text(p: &Prog, picked: &[&Cand]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for c in picked {
        match counts.iter_mut().find(|x| x.0 == c.kind) {
            Some(x) => x.1 += 1,
            None => counts.push((c.kind, 1)),
        }
    }
    let kinds = counts.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", ");
    let mut t = format!("Pill will remove these leftovers of \"{}\" as administrator:\n{kinds}\n\n", p.name);
    for c in picked.iter().take(15) {
        let s: String = c.shown.chars().take(110).collect();
        t.push_str(&format!("- {s}\n"));
    }
    if picked.len() > 15 {
        t.push_str(&format!("... and {} more\n", picked.len() - 15));
    }
    t.push_str("\nFiles go to the Recycle Bin; registry keys, services and tasks are backed up first. Press OK to remove, Cancel to stop.");
    t
}

/// The real dialog: OK / Cancel, always on top. Cancel (or any failure) means "do not delete".
pub fn native_confirm(text: &str) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDOK, MB_DEFBUTTON2, MB_ICONWARNING, MB_OKCANCEL, MB_SETFOREGROUND, MB_TOPMOST};
    let (t, c): (Vec<u16>, Vec<u16>) = (text.encode_utf16().chain(Some(0)).collect(), "Pill: remove system leftovers".encode_utf16().chain(Some(0)).collect());
    unsafe { MessageBoxW(HWND::default(), PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OKCANCEL | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND) == IDOK }
}

// ---------------------------------------------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------------------------------------------

pub fn send(s: &mut TcpStream, v: &Value) -> Result<(), String> {
    let mut line = v.to_string();
    line.push('\n');
    s.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Several threads write to one socket (the work and the heartbeat): one message at a time.
fn send_shared(s: &Mutex<TcpStream>, v: &Value) -> Result<(), String> {
    send(&mut s.lock().unwrap_or_else(|e| e.into_inner()), v)
}

pub fn recv(r: &mut BufReader<TcpStream>) -> Result<Value, String> {
    let mut line = String::new();
    let n = r.by_ref().take(MAX_LINE).read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 || !line.ends_with('\n') {
        return Err("connection closed".into());
    }
    serde_json::from_str(&line).map_err(|e| e.to_string())
}

/// Line reader that survives read timeouts without losing a half-received line.
pub struct Lines {
    r: BufReader<TcpStream>,
    buf: String,
}
impl Lines {
    pub fn new(s: TcpStream) -> Lines {
        Lines { r: BufReader::new(s), buf: String::new() }
    }
    /// Ok(None) = nothing complete yet (the read timed out).
    pub fn next(&mut self) -> Result<Option<Value>, String> {
        loop {
            match self.r.by_ref().take(MAX_LINE).read_line(&mut self.buf) {
                Ok(0) => return Err("connection closed".into()),
                Ok(_) if self.buf.ends_with('\n') => {
                    let v = serde_json::from_str(&self.buf).map_err(|e| e.to_string());
                    self.buf.clear();
                    return v.map(Some);
                }
                Ok(_) if self.buf.len() as u64 >= MAX_LINE => return Err("message too long".into()),
                Ok(_) => continue,
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return Ok(None),
                Err(e) => return Err(e.to_string()),
            }
        }
    }
}

pub fn valid_port_token(a: &str) -> Option<(u16, String)> {
    let (port, token) = a.split_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p >= 1024)?;
    (token.len() == 32 && token.chars().all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))).then(|| (port, token.to_string()))
}

/// Names the worker may ask the unelevated side to store (nothing else is ever written for it).
pub fn store_name_ok(n: &str) -> bool {
    n == "reg" || n == "log" || (n.starts_with("task") && n.ends_with(".xml") && n.len() > 8 && n[4..n.len() - 4].chars().all(|c| c.is_ascii_digit()))
}

// ---------------------------------------------------------------------------------------------------------------
// pill.exe is pinned: nobody can swap the file we are going to elevate
// ---------------------------------------------------------------------------------------------------------------

struct Pinned {
    file: std::fs::File, // open for the whole run with FILE_SHARE_READ only: no writer, no rename, no delete
    hash: Mutex<Option<[u8; 32]>>,
}
static PIN: OnceLock<Option<Pinned>> = OnceLock::new();

/// Called once at startup. A failure leaves the admin step unavailable (fail closed).
pub fn pin_exe() {
    PIN.get_or_init(|| {
        let path = std::env::current_exe().ok()?;
        let file = std::fs::OpenOptions::new().read(true).share_mode(1).open(path).ok()?; // FILE_SHARE_READ
        Some(Pinned { file, hash: Mutex::new(None) })
    });
}

/// SHA-256 of a file, read through positional reads so any file pointer stays untouched.
pub fn hash_handle(f: &std::fs::File) -> std::io::Result<[u8; 32]> {
    use sha2::{Digest, Sha256};
    let (mut h, mut buf, mut off) = (Sha256::new(), vec![0u8; 1 << 20], 0u64);
    loop {
        let n = f.seek_read(&mut buf, off)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        off += n as u64;
    }
    Ok(h.finalize().into())
}

/// SHA-256 of the file at `path`, opened for reading with sharing for readers only.
pub fn hash_path(path: &Path) -> std::io::Result<[u8; 32]> {
    hash_handle(&std::fs::OpenOptions::new().read(true).share_mode(1).open(path)?)
}

/// The path the system reports for this running module (not an environment value, not argv[0]).
pub fn module_path() -> Option<PathBuf> {
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    let mut buf = [0u16; 1024];
    let n = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n])))
}

/// `path` is the running module and its content is what was pinned at startup.
pub fn check_exe(expected: &[u8; 32], path: &Path, module: &Path) -> Result<(), &'static str> {
    if comps(path) != comps(module) {
        return Err("the elevation path is not the running program");
    }
    match hash_path(path) {
        Ok(h) if &h == expected => Ok(()),
        Ok(_) => Err("pill.exe does not match the copy that started"),
        Err(_) => Err("pill.exe cannot be read"),
    }
}

/// The exe to hand to the permission prompt, or why not.
fn elevation_exe() -> Result<PathBuf, String> {
    let pin = PIN.get().and_then(Option::as_ref).ok_or("pill.exe was not pinned at startup; restart Pill")?;
    let module = module_path().ok_or("cannot find the running program")?;
    let expected = {
        let mut g = pin.hash.lock().unwrap_or_else(|e| e.into_inner());
        match *g {
            Some(h) => h,
            None => {
                let h = hash_handle(&pin.file).map_err(|e| e.to_string())?;
                *g = Some(h);
                h
            }
        }
    };
    check_exe(&expected, &module, &module).map_err(|e| format!("{e}; restart Pill first"))?;
    Ok(module)
}

// ---------------------------------------------------------------------------------------------------------------
// The elevated worker
// ---------------------------------------------------------------------------------------------------------------

/// Called first thing in `main()`: runs the worker when Pill was started with `--uninstall-admin <port>:<token>`; None for a normal start.
pub fn admin_main() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(ARG) {
        return None;
    }
    {
        // from here on DLLs come from System32 only, never from the app folder or the current directory
        use windows::Win32::System::LibraryLoader::{SetDefaultDllDirectories, LOAD_LIBRARY_SEARCH_SYSTEM32};
        let _ = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    }
    Some(match args.get(2).filter(|_| args.len() == 3).and_then(|a| valid_port_token(a)) {
        Some((port, token)) => match std::panic::catch_unwind(|| worker_with(port, &token, &Env::real(), &RealReg, &native_confirm)) {
            Ok(Ok(())) => 0,
            _ => 2,
        },
        None => 2,
    })
}

pub fn worker_with(port: u16, token: &str, env: &Env, reg: &dyn Reg, confirm: &dyn Fn(&str) -> bool) -> Result<(), String> {
    let s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(600))).ok();
    let w = Arc::new(Mutex::new(s.try_clone().map_err(|e| e.to_string())?));
    send_shared(&w, &json!({ "hello": token }))?;
    let mut r = BufReader::new(s);
    let req = recv(&mut r)?;
    // heartbeat: the parent knows the helper is alive during a long scan or while the dialog is up
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let hb = {
        let (w, done) = (w.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                if send_shared(&w, &json!({ "hb": 1 })).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        })
    };
    let r = RefCell::new(r);
    let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match Req::from_json(&req).and_then(|q| validate_req(&q, env, reg).map(|p| (q, p))) {
        Err(e) => json!({ "error": e }),
        Ok((q, p)) => work(&q, &p, env, reg, &w, &r, confirm),
    })) {
        Ok(v) => v,
        Err(_) => json!({ "error": "the helper hit an internal error" }),
    };
    done.store(true, Ordering::Relaxed);
    let _ = hb.join();
    send_shared(&w, &json!({ "result": result }))
}

/// Is `c` something the elevated process must leave alone? Windows\Temp is writable by every user: a junction could be swapped in under us.
pub fn worker_refuses(c: &Cand, env: &Env) -> bool {
    matches!(&c.what, What::Fs(p) if depth_under(p, &env.windows.join("Temp")).is_some())
}

/// The part of a delete that decides: nothing is removed unless `confirm` says OK for the exact list.
pub fn decide(p: &Prog, picked: &[&Cand], confirm: &dyn Fn(&str) -> bool) -> bool {
    !picked.is_empty() && confirm(&confirm_text(p, picked))
}

fn work(q: &Req, p: &Prog, env: &Env, reg: &dyn Reg, w: &Arc<Mutex<TcpStream>>, r: &RefCell<BufReader<TcpStream>>, confirm: &dyn Fn(&str) -> bool) -> Value {
    // no shortcut resolution (IShellLink) in the elevated process
    let (cands, manual, cut) = scan_all_with(p, env, reg, false, &|_| None, &|_, _| {});
    if q.mode == Mode::Scan {
        let in_other_profile = |c: &Cand| matches!(&c.what, What::Fs(x) if env.others.iter().any(|o| depth_under(x, o).is_some()));
        let hidden = cands.iter().filter(|c| c.admin && in_other_profile(c)).count();
        let items: Vec<Value> = cands
            .iter()
            .filter(|c| c.admin && !in_other_profile(c))
            .map(|c| json!({ "key": cand_key(&c.what), "kind": c.kind, "shown": c.shown, "size": c.size, "tier": if c.tier == Tier::Certain { "certain" } else { "probable" }, "note": c.note }))
            .collect();
        return json!({ "items": items, "manual": manual, "cut": cut, "hiddenOthers": hidden });
    }
    // delete: only what the user ticked AND this scan found again
    let picked: Vec<&Cand> = cands.iter().filter(|c| c.admin && q.keys.contains(&cand_key(&c.what)) && !worker_refuses(c, env)).collect();
    if !decide(p, &picked, confirm) {
        return json!({ "error": if picked.is_empty() { "nothing to remove" } else { "cancelled in the administrator dialog" } });
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
    let store = |name: &str, bytes: &[u8]| -> Result<String, String> {
        send_shared(w, &json!({ "store": { "name": name, "b64": base64::engine::general_purpose::STANDARD.encode(bytes) } }))?;
        loop {
            let mut rd = r.borrow_mut();
            let mut line = String::new();
            if rd.by_ref().take(MAX_LINE).read_line(&mut line).map_err(|e| e.to_string())? == 0 {
                return Err("connection closed".into());
            }
            let ack: Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
            if ack.get("ok").is_none() {
                continue;
            }
            return if ack.get("ok").and_then(Value::as_bool) == Some(true) { Ok(ack.get("path").and_then(Value::as_str).unwrap_or("").to_string()) } else { Err(ack.get("error").and_then(Value::as_str).unwrap_or("not stored").to_string()) };
        }
    };
    let rep = delete_cands(p, &picked, env, reg, &stamp, true, &store, &|_, _, _, _, _| {});
    json!({ "done": rep.done.iter().map(|(k, s)| json!([k, s])).collect::<Vec<_>>(), "failed": rep.failed.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "backup": rep.backup, "log": rep.log })
}

// ---------------------------------------------------------------------------------------------------------------
// The unelevated side
// ---------------------------------------------------------------------------------------------------------------

/// 128 random bits from the system generator (BCryptGenRandom), as 32 hex digits. An error means: do not elevate.
pub fn token() -> Result<String, String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut b = [0u8; 16];
    let st = unsafe { BCryptGenRandom(None, &mut b, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    if st.0 != 0 {
        return Err("the system random generator failed".into());
    }
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

fn runas(file: &Path, params: &str) -> Result<windows::Win32::Foundation::HANDLE, String> {
    let (f, a): (Vec<u16>, Vec<u16>) = (file.to_string_lossy().encode_utf16().chain(Some(0)).collect(), params.encode_utf16().chain(Some(0)).collect());
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: PCWSTR(a.as_ptr()),
        nShow: 0, // SW_HIDE: the worker has no window of its own (its dialog is a message box)
        ..Default::default()
    };
    match unsafe { ShellExecuteExW(&mut sei) } {
        Ok(()) => Ok(sei.hProcess),
        Err(e) if (e.code().0 as u32) & 0xFFFF == 1223 => Err("cancelled".into()), // the user said No to the permission prompt
        Err(e) => Err(format!("Windows could not start the elevated helper ({e})")),
    }
}

/// Starts the worker, serves it (request in, backups stored, result out) and returns its result object.
pub fn run_elevated(p: &Prog, mode: Mode, keys: Vec<String>, store: &dyn Fn(&str, &[u8]) -> Result<String, String>) -> Result<Value, String> {
    let exe = elevation_exe()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let tok = token()?;
    let h = runas(&exe, &format!("{ARG} {port}:{tok}"))?; // blocks on the Windows permission prompt
    let alive = || h.is_invalid() || unsafe { WaitForSingleObject(h, 0) }.0 == 258; // WAIT_TIMEOUT: still running
    let result = serve(&listener, &tok, p, mode, keys, store, 60, &alive);
    unsafe {
        if !h.is_invalid() {
            let _ = WaitForSingleObject(h, 10_000);
            let _ = CloseHandle(h);
        }
    }
    result
}

/// The unelevated end of the conversation: accepts clients until one presents the token (others are dropped), sends the request, stores what the
/// helper hands over, returns its result. Gives up when the helper goes silent (heartbeats keep it alive) or its process is gone.
pub fn serve(
    listener: &TcpListener,
    tok: &str,
    p: &Prog,
    mode: Mode,
    keys: Vec<String>,
    store: &dyn Fn(&str, &[u8]) -> Result<String, String>,
    accept_secs: u64,
    alive: &dyn Fn() -> bool,
) -> Result<Value, String> {
    listener.set_nonblocking(true).ok();
    let t = Instant::now();
    let (mut w, mut rd) = loop {
        match listener.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false).ok();
                s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let (Ok(w), Ok(c)) = (s.try_clone(), s.try_clone()) else { continue };
                let mut lines = Lines::new(c);
                // a stranger (wrong token, no hello, garbage) is dropped and the wait goes on
                if matches!(lines.next(), Ok(Some(h)) if h.get("hello").and_then(Value::as_str) == Some(tok)) {
                    w.set_read_timeout(Some(Duration::from_secs(2))).ok();
                    break (w, lines);
                }
            }
            Err(_) if t.elapsed() < Duration::from_secs(accept_secs) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err("the elevated helper did not connect".into()),
        }
    };
    send(&mut w, &Req::of(p, mode, keys).to_json())?;
    let mut last = Instant::now();
    loop {
        let Some(m) = rd.next()? else {
            if !alive() {
                return Err("the elevated helper stopped".into());
            }
            if last.elapsed() > SILENCE_MAX {
                return Err("the elevated helper stopped answering".into());
            }
            continue;
        };
        last = Instant::now();
        if let Some(st) = m.get("store") {
            let name = st.get("name").and_then(Value::as_str).unwrap_or("");
            let reply = match (store_name_ok(name), base64::engine::general_purpose::STANDARD.decode(st.get("b64").and_then(Value::as_str).unwrap_or(""))) {
                (true, Ok(bytes)) => match store(name, &bytes) {
                    Ok(path) => json!({ "ok": true, "path": path }),
                    Err(e) => json!({ "ok": false, "error": e }),
                },
                _ => json!({ "ok": false, "error": "refused" }),
            };
            send(&mut w, &reply)?;
        } else if let Some(r) = m.get("result") {
            return Ok(r.clone());
        }
    }
}
