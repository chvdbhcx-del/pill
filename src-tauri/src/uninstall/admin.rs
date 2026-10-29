// Removing the leftovers that need administrator rights (HKLM keys, services, scheduled tasks, firewall rules, Program Files / ProgramData files,
// other people's profiles). Pill relaunches ITSELF elevated with a fixed argument form `--uninstall-admin <port>:<token>` (Windows shows its own
// permission prompt). `main()` hands that launch to `admin_main` before anything else starts, so the elevated copy is a small headless worker.
//
// Trust model (read this before changing anything):
//  * The unelevated Pill and the worker talk over a loopback socket the unelevated side opened (nothing is written to disk by the worker, so
//    there is no file or junction an unprivileged process could redirect). The worker hands its backups to the unelevated side, which stores them.
//  * The request carries an app IDENTITY (name, publisher, install folder, GUIDs) and a list of item KEYS. The worker never deletes a path or a key it
//    was given: it validates the identity (`validate_req`), re-derives the candidate list with its own scan (same code, same `guard_cand`), and deletes
//    only candidates whose key is in the list. A forged request can therefore only name an identity; whatever that identity's evidence finds is the
//    most it can touch.
//  * The identity is refused when: the name is short or generic, the install folder is not a believable app folder (a system root, a parent of one,
//    inside Windows, a user's known folder, generic name), or the app is still registered as installed (any Uninstall entry with its GUID / name /
//    a folder overlapping the install folder).
//  * Residual risk (documented, not removable here): any process of the same user can start `pill.exe --uninstall-admin` itself and run a listener;
//    the permission prompt is the only gate, and it names Pill. Before elevating, the unelevated side refuses when pill.exe was replaced after Pill
//    started. A forged identity is limited as described above.
use super::*;
use base64::Engine;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};

pub const ARG: &str = "--uninstall-admin";
const MAX_LINE: u64 = 16 << 20;
const KEYS_MAX: usize = 5_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Scan,
    Delete,
}

/// What the unelevated side sends: an identity and, for a delete, the item keys the user ticked.
#[derive(Clone, Debug)]
pub struct Req {
    pub mode: Mode,
    pub name: String,
    pub publisher: String,
    pub location: String,
    pub sub: String,
    pub hklm: bool,
    pub guids: Vec<String>,
    pub keys: Vec<String>,
}

impl Req {
    pub fn of(p: &Prog, mode: Mode, keys: Vec<String>) -> Req {
        Req { mode, name: p.name.clone(), publisher: p.publisher.clone(), location: p.location.clone(), sub: p.sub.clone(), hklm: p.hklm, guids: p.guids(), keys }
    }
    pub fn to_json(&self) -> Value {
        json!({ "v": 1, "mode": if self.mode == Mode::Scan { "scan" } else { "delete" }, "name": self.name, "publisher": self.publisher, "location": self.location,
            "sub": self.sub, "hklm": self.hklm, "guids": self.guids, "keys": self.keys })
    }
    pub fn from_json(v: &Value) -> Result<Req, &'static str> {
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let list = |k: &str| -> Vec<String> { v.get(k).and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default() };
        if v.get("v").and_then(Value::as_u64) != Some(1) {
            return Err("unknown request version");
        }
        let mode = match v.get("mode").and_then(Value::as_str) {
            Some("scan") => Mode::Scan,
            Some("delete") => Mode::Delete,
            _ => return Err("unknown mode"),
        };
        Ok(Req { mode, name: s("name"), publisher: s("publisher"), location: s("location"), sub: s("sub"), hklm: v.get("hklm").and_then(Value::as_bool).unwrap_or(false), guids: list("guids"), keys: list("keys") })
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

/// Is any registered program this app (same GUID / name) or living in the same folder?
fn still_registered(p: &Prog, install: Option<&Path>, reg: &dyn Reg) -> bool {
    let (names, guids) = (names_of(p), p.guids());
    uninstall_entries(reg).iter().any(|(leaf, v)| {
        let dn = norm(&vstr_pub(v, "displayname"));
        let loc = vstr_pub(v, "installlocation");
        let cmd = vstr_pub(v, "uninstallstring");
        let unin = split_cmd(&expand_env(&cmd)).map(|x| x.0).unwrap_or_default();
        // alive: its uninstaller file exists; for msiexec / script-style entries, its install folder still holds something
        let alive = if cmd.to_lowercase().contains("msiexec") || unin.is_empty() {
            let l = Path::new(loc.trim_end_matches('\\'));
            loc.is_empty() || (l.is_dir() && !is_empty_dir(l))
        } else {
            Path::new(&unin).is_file()
        };
        let same = guids.contains(&leaf.to_uppercase()) || (!dn.is_empty() && names.product.contains(&dn));
        let overlap = install.is_some_and(|i| {
            let l = Path::new(loc.trim_end_matches('\\'));
            (!loc.is_empty() && (depth_under(l, i).is_some() || depth_under(i, l).is_some())) || (!unin.is_empty() && depth_under(Path::new(&unin), i).is_some())
        });
        // a dead entry (its uninstaller is gone) is a leftover, not an installed app
        alive && (same || overlap)
    })
}

fn vstr_pub(v: &Vals, k: &str) -> String {
    match v.get(k) {
        Some(Val::S(s)) | Some(Val::E(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

/// Turns a request into the identity the worker scans for, or says why it must not run. The worker calls this before it does anything.
pub fn validate_req(r: &Req, env: &Env, reg: &dyn Reg) -> Result<Prog, &'static str> {
    if r.keys.len() > KEYS_MAX || r.keys.iter().any(|k| k.len() > 700 || !clean(k, 700)) {
        return Err("too many or malformed item keys");
    }
    if !clean(&r.name, 200) || !clean(&r.publisher, 200) || !clean(&r.location, 520) || !clean(&r.sub, 520) || r.guids.len() > 8 {
        return Err("malformed identity");
    }
    if r.guids.iter().any(|g| find_guid(g).as_deref() != Some(g.as_str())) {
        return Err("malformed GUID");
    }
    let mut p = Prog { name: r.name.trim().to_string(), publisher: r.publisher.trim().to_string(), location: r.location.trim().to_string(), sub: r.sub.clone(), hklm: r.hklm, ..Default::default() };
    p.extra_guids = r.guids.clone();
    let names = names_of(&p);
    // administrator work needs a distinctive name: 4+ letters, not a generic word
    if names.product.is_empty() || !names.product.iter().any(|n| n.chars().count() >= 4) {
        return Err("the app name is too short or too generic");
    }
    let install = p.install();
    if let Some(i) = &install {
        if !install_usable(i, env) {
            return Err("the install folder is not a believable app folder");
        }
    }
    if still_registered(&p, install.as_deref(), reg) {
        return Err("the app is still installed");
    }
    Ok(p)
}

// ---------------------------------------------------------------------------------------------------------------
// Wire format
// ---------------------------------------------------------------------------------------------------------------

pub fn send(s: &mut TcpStream, v: &Value) -> Result<(), String> {
    let mut line = v.to_string();
    line.push('\n');
    s.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

pub fn recv(r: &mut BufReader<TcpStream>) -> Result<Value, String> {
    let mut line = String::new();
    let n = r.by_ref().take(MAX_LINE).read_line(&mut line).map_err(|e| e.to_string())?;
    if n == 0 || !line.ends_with('\n') {
        return Err("connection closed".into());
    }
    serde_json::from_str(&line).map_err(|e| e.to_string())
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
// The elevated worker
// ---------------------------------------------------------------------------------------------------------------

/// Called first thing in `main()`: runs the worker when Pill was started with `--uninstall-admin <port>:<token>`; None for a normal start.
pub fn admin_main() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some(ARG) {
        return None;
    }
    Some(match args.get(2).filter(|_| args.len() == 3).and_then(|a| valid_port_token(a)) {
        Some((port, token)) if worker_with(port, &token, &Env::real(), &RealReg).is_ok() => 0,
        _ => 2,
    })
}

pub fn worker_with(port: u16, token: &str, env: &Env, reg: &dyn Reg) -> Result<(), String> {
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(600))).ok();
    send(&mut s, &json!({ "hello": token }))?;
    let mut r = BufReader::new(s.try_clone().map_err(|e| e.to_string())?);
    let req = recv(&mut r)?;
    let result = match Req::from_json(&req).and_then(|q| validate_req(&q, env, reg).map(|p| (q, p))) {
        Err(e) => json!({ "error": e }),
        Ok((q, p)) => work(&q, &p, env, reg, &mut s, &mut r),
    };
    send(&mut s, &json!({ "result": result }))
}

fn work(q: &Req, p: &Prog, env: &Env, reg: &dyn Reg, s: &mut TcpStream, r: &mut BufReader<TcpStream>) -> Value {
    let (cands, manual, cut) = scan_all_with(p, env, reg, false, &|_, _| {});
    if q.mode == Mode::Scan {
        let items: Vec<Value> = cands
            .iter()
            .filter(|c| c.admin)
            .map(|c| json!({ "key": cand_key(&c.what), "kind": c.kind, "shown": c.shown, "size": c.size, "tier": if c.tier == Tier::Certain { "certain" } else { "probable" }, "note": c.note }))
            .collect();
        return json!({ "items": items, "manual": manual, "cut": cut });
    }
    // delete: only what the user ticked AND this scan found again
    let picked: Vec<&Cand> = cands.iter().filter(|c| c.admin && q.keys.contains(&cand_key(&c.what))).collect();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
    let sock = std::cell::RefCell::new((s.try_clone().expect("socket"), r.get_ref().try_clone().expect("socket")));
    let store = |name: &str, bytes: &[u8]| -> Result<String, String> {
        let mut g = sock.borrow_mut();
        send(&mut g.0, &json!({ "store": { "name": name, "b64": base64::engine::general_purpose::STANDARD.encode(bytes) } }))?;
        let mut rd = BufReader::new(g.1.try_clone().map_err(|e| e.to_string())?);
        let ack = recv(&mut rd)?;
        if ack.get("ok").and_then(Value::as_bool) == Some(true) { Ok(ack.get("path").and_then(Value::as_str).unwrap_or("").to_string()) } else { Err(ack.get("error").and_then(Value::as_str).unwrap_or("not stored").to_string()) }
    };
    let rep = delete_cands(p, &picked, env, reg, &stamp, true, &store, &|_, _, _, _, _| {});
    json!({ "done": rep.done.iter().map(|(k, s)| json!([k, s])).collect::<Vec<_>>(), "failed": rep.failed.iter().map(|(a, b)| json!([a, b])).collect::<Vec<_>>(),
        "backup": rep.backup, "log": rep.log })
}

// ---------------------------------------------------------------------------------------------------------------
// The unelevated side
// ---------------------------------------------------------------------------------------------------------------

fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let part = || {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher(); // keyed from the OS random source
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        h.finish()
    };
    format!("{:016x}{:016x}", part(), part())
}

fn process_start() -> Option<std::time::SystemTime> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).ok()? };
    let t = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64; // 100 ns since 1601
    t.checked_sub(116_444_736_000_000_000).map(|x| std::time::UNIX_EPOCH + Duration::from_nanos(x * 100))
}

/// pill.exe changed after this process started: do not hand a permission prompt to a file somebody swapped in.
fn exe_replaced() -> bool {
    let Ok(exe) = std::env::current_exe() else { return true };
    let Ok(m) = std::fs::metadata(&exe).and_then(|m| m.modified()) else { return true };
    process_start().is_none_or(|st| m > st + Duration::from_secs(2))
}

fn runas(file: &Path, params: &str) -> Result<windows::Win32::Foundation::HANDLE, String> {
    let (f, a): (Vec<u16>, Vec<u16>) = (file.to_string_lossy().encode_utf16().chain(Some(0)).collect(), params.encode_utf16().chain(Some(0)).collect());
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(f.as_ptr()),
        lpParameters: PCWSTR(a.as_ptr()),
        nShow: 0, // SW_HIDE: the worker has no window
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
    if exe_replaced() {
        return Err("pill.exe changed after Pill started; restart Pill first".into());
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let tok = token();
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let h = runas(&exe, &format!("{ARG} {port}:{tok}"))?; // blocks on the Windows permission prompt
    let result = serve(&listener, &tok, p, mode, keys, store, 60);
    unsafe {
        if !h.is_invalid() {
            let _ = WaitForSingleObject(h, 10_000);
            let _ = CloseHandle(h);
        }
    }
    result
}

/// The unelevated end of the conversation: accepts the helper, checks its token, sends the request, stores what it hands over, returns its result.
pub fn serve(listener: &TcpListener, tok: &str, p: &Prog, mode: Mode, keys: Vec<String>, store: &dyn Fn(&str, &[u8]) -> Result<String, String>, accept_secs: u64) -> Result<Value, String> {
    listener.set_nonblocking(true).ok();
    let t = Instant::now();
    let stream = loop {
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(_) if t.elapsed() < Duration::from_secs(accept_secs) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return Err("the elevated helper did not connect".into()),
        }
    };
    stream.set_nonblocking(false).ok();
    stream.set_read_timeout(Some(Duration::from_secs(600))).ok();
    let mut w = stream.try_clone().map_err(|e| e.to_string())?;
    let mut rd = BufReader::new(stream);
    let hello = recv(&mut rd)?;
    if hello.get("hello").and_then(Value::as_str) != Some(tok) {
        return Err("the helper did not present the session token".into());
    }
    send(&mut w, &Req::of(p, mode, keys).to_json())?;
    let result = loop {
        let m = recv(&mut rd)?;
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
            break r.clone();
        }
    };
    Ok(result)
}

