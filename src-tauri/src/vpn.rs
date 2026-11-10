// VPN: starts and stops the user's OWN client (a command line or a client:// link set in Settings > Modules > VPN) and tells the island
// whether it is up. Pill installs no VPN and sends nothing anywhere.
//  * Settings: `module.vpn` (the whole feature: chip, Controls tile, bar command, hotkey), `vpn.own.name`, `vpn.own.connect`,
//    `vpn.own.disconnect`, `vpn.admin` (run the commands elevated through ShellExecute "runas"; links are never elevated).
//  * State: an own client with a name is on while a network adapter whose name or description contains that name is up (so someone else's
//    tunnel or proxy never counts). Without an own client: any up adapter that looks like a VPN. An own client without any name is
//    assumed to be in the state of the last command.
//  * WireGuard / AmneziaWG: `wireguard.exe /installtunnelservice x.conf` connects; with no disconnect command `/uninstalltunnelservice x`
//    is derived. Both need `vpn.admin`.
//  * The island chip is driven by the event `vpn:changed {country: "VPN" | "", on}` (ui/island/core.rs turns it into the chip's settings).
//    The state is read when the island expands, when a vpn.* setting changes and for a few seconds after a toggle: never on a timer.
//  * Commands: vpn_state, vpn_toggle.
use crate::settings;
use crate::ui::bus;
use serde_json::{json, Value};
use std::os::windows::process::CommandExt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};

const NO_WINDOW: u32 = 0x0800_0000;
const IF_PPP: u32 = 23;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();
static LAST_ON: AtomicBool = AtomicBool::new(false); // state assumed for a client without a name
static FOLLOWING: AtomicBool = AtomicBool::new(false);
static LAST_PUBLISH: AtomicI64 = AtomicI64::new(0);

pub fn enabled() -> bool {
    settings::get_bool("module.vpn", true)
}

// ---- settings ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Own {
    pub name: String,
    pub connect: String,
    pub disconnect: String,
    pub admin: bool,
}

/// The own client, if a connect command is set. The name falls back to the tunnel of an `/installtunnelservice` command.
pub fn own_from(name: &str, connect: &str, disconnect: &str, admin: bool) -> Option<Own> {
    let connect = connect.trim();
    if connect.is_empty() {
        return None;
    }
    let name = match name.trim() {
        "" => tunnel_name(connect).unwrap_or_default(),
        n => n.to_string(),
    };
    let disconnect = match disconnect.trim() {
        "" => derive_disconnect(connect).unwrap_or_default(),
        d => d.to_string(),
    };
    Some(Own { name, connect: connect.to_string(), disconnect, admin })
}

fn own() -> Option<Own> {
    own_from(&settings::get_str("vpn.own.name", ""), &settings::get_str("vpn.own.connect", ""), &settings::get_str("vpn.own.disconnect", ""), settings::get_bool("vpn.admin", false))
}

// ---- command lines ----

/// "client://connect", "https://...": a link goes to the shell as it is. A drive path ("C:\x") is not one.
pub fn is_link(s: &str) -> bool {
    let head = s.split_whitespace().next().unwrap_or("");
    head.split_once("://").is_some_and(|(scheme, rest)| scheme.len() >= 2 && !rest.is_empty() && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)))
}

/// (program, raw argument text). A quoted program, an unquoted path up to ".exe" (spaces allowed), or the first word.
pub fn split_cmd(cmd: &str) -> Option<(String, String)> {
    let c = cmd.trim();
    if let Some(rest) = c.strip_prefix('"') {
        let (exe, args) = rest.split_once('"')?;
        return (!exe.is_empty()).then(|| (exe.to_string(), args.trim().to_string()));
    }
    if let Some(i) = c.to_lowercase().find(".exe") {
        let end = i + 4;
        if c.is_char_boundary(end) && c[end..].chars().next().is_none_or(char::is_whitespace) {
            return Some((c[..end].to_string(), c[end..].trim().to_string()));
        }
    }
    let (exe, args) = c.split_once(char::is_whitespace).unwrap_or((c, ""));
    (!exe.is_empty()).then(|| (exe.to_string(), args.trim().to_string()))
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"')
}

fn conf_stem(path: &str) -> Option<String> {
    let p = unquote(path);
    let file = p.rsplit(['\\', '/']).next()?;
    let stem = file.rsplit_once('.').map_or(file, |(s, _)| s);
    (!stem.is_empty()).then(|| stem.to_string())
}

/// `wireguard.exe /installtunnelservice C:\x\home.conf` -> "home".
pub fn tunnel_name(connect: &str) -> Option<String> {
    let (_, args) = split_cmd(connect)?;
    let rest = args.strip_prefix("/installtunnelservice").filter(|r| r.is_empty() || r.starts_with(char::is_whitespace))?;
    conf_stem(rest)
}

/// The matching `/uninstalltunnelservice <tunnel>` command of an `/installtunnelservice` one.
pub fn derive_disconnect(connect: &str) -> Option<String> {
    let (exe, _) = split_cmd(connect)?;
    Some(format!("\"{exe}\" /uninstalltunnelservice {}", tunnel_name(connect)?))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Link(String),
    Exe { exe: String, args: String, admin: bool },
}

/// What running `cmd` means. A link ignores `admin`.
pub fn plan(cmd: &str, admin: bool) -> Option<Plan> {
    let c = cmd.trim();
    if c.is_empty() || c.contains('\0') {
        return None;
    }
    if is_link(c) {
        return Some(Plan::Link(c.to_string()));
    }
    let (exe, args) = split_cmd(c)?;
    Some(Plan::Exe { exe, args, admin })
}

pub trait Launch {
    fn link(&self, url: &str) -> Result<(), String>;
    fn exe(&self, exe: &str, args: &str, admin: bool) -> Result<(), String>;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn shell_execute(verb: &str, file: &str, params: &str, show: i32) -> Result<(), String> {
    let (v, f, p) = (wide(verb), wide(file), wide(params));
    let r = unsafe { ShellExecuteW(HWND::default(), PCWSTR(v.as_ptr()), PCWSTR(f.as_ptr()), if params.is_empty() { PCWSTR::null() } else { PCWSTR(p.as_ptr()) }, PCWSTR::null(), windows::Win32::UI::WindowsAndMessaging::SHOW_WINDOW_CMD(show)) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("could not start (code {}): cancelled or access denied", r.0 as isize))
    }
}

pub struct Real;
impl Launch for Real {
    fn link(&self, url: &str) -> Result<(), String> {
        shell_execute("open", url, "", SW_SHOWNORMAL.0)
    }
    fn exe(&self, exe: &str, args: &str, admin: bool) -> Result<(), String> {
        if admin {
            return shell_execute("runas", exe, args, SW_HIDE.0); // UAC is the user's: Pill never elevates itself
        }
        std::process::Command::new(exe).raw_arg(args).creation_flags(NO_WINDOW).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

pub fn run(l: &dyn Launch, cmd: &str, admin: bool) -> Result<(), String> {
    match plan(cmd, admin).ok_or("empty command")? {
        Plan::Link(u) => l.link(&u),
        Plan::Exe { exe, args, admin } => l.exe(&exe, &args, admin),
    }
}

// ---- state ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ad {
    pub alias: String,
    pub desc: String,
    pub ty: u32,
    pub up: bool,
}

const VPN_WORDS: &[&str] = &["wireguard", "wintun", "tap-windows", "openvpn", "nordlynx", "amnezia", "mullvad", "proton", "vpn", "wg0", "awg"];
const NOT_VPN: &[&str] = &["hyper-v", "vethernet", "wsl", "virtualbox", "vmware", "tailscale", "zerotier", "loopback", "isatap", "teredo", "6to4", "bluetooth", "wi-fi direct"];

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

/// An up adapter that looks like a VPN (no own client set).
fn looks_like_vpn(a: &Ad) -> bool {
    let both = format!("{} {}", a.alias, a.desc).to_lowercase();
    a.up && !NOT_VPN.iter().any(|w| both.contains(w)) && (a.ty == IF_PPP || VPN_WORDS.iter().any(|w| both.contains(w)))
}

/// Is the VPN on? `own_name`: Some(non-empty) = judged only by that name; None = any VPN-looking adapter.
pub fn judge(ads: &[Ad], own_name: Option<&str>) -> bool {
    match own_name {
        Some(n) if !n.is_empty() => ads.iter().any(|a| a.up && (contains_ci(&a.alias, n) || contains_ci(&a.desc, n))),
        _ => ads.iter().any(looks_like_vpn),
    }
}

fn adapters() -> Vec<Ad> {
    let mut out = Vec::new();
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return out;
        }
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
        let text = |w: &[u16]| String::from_utf16_lossy(&w[..w.iter().position(|&c| c == 0).unwrap_or(w.len())]);
        for r in rows {
            out.push(Ad { alias: text(&r.Alias), desc: text(&r.Description), ty: r.Type, up: r.OperStatus.0 == 1 });
        }
        FreeMibTable(t as *const _);
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub enabled: bool,
    pub configured: bool,
    pub on: bool,
    pub name: String,
}

fn current() -> State {
    if !enabled() {
        return State { enabled: false, configured: false, on: false, name: String::new() };
    }
    let own = own();
    let on = match &own {
        Some(o) if o.name.is_empty() => LAST_ON.load(Ordering::Relaxed),
        Some(o) => judge(&adapters(), Some(&o.name)),
        None => judge(&adapters(), None),
    };
    State { enabled: true, configured: own.is_some(), on, name: own.map(|o| o.name).unwrap_or_default() }
}

/// Command `vpn_state`.
pub fn vpn_state() -> Value {
    let s = current();
    json!({ "enabled": s.enabled, "configured": s.configured, "on": s.on, "name": s.name })
}

/// Tells the island chip. The chip shows while the feature is on and there is something to switch or something up.
fn publish() {
    let s = current();
    let show = s.enabled && (s.configured || s.on);
    let _ = bus::emit("vpn:changed", json!({ "country": if show { "VPN" } else { "" }, "on": s.on }));
}

fn publish_soon() {
    let now = crate::store::now_ms();
    if now - LAST_PUBLISH.swap(now, Ordering::Relaxed) < 300 {
        return;
    }
    std::thread::spawn(publish); // bus callbacks must stay short and may not emit
}

pub fn init(app: &tauri::App) {
    let _ = APP.set(app.handle().clone());
    bus::subscribe("pill:expand", |_| publish_soon());
    // settings:changed carries the whole settings object on every write: look at the five VPN keys only
    let last = std::sync::Mutex::new(String::new());
    bus::subscribe("settings:changed", move |p| {
        let sig: String = ["module.vpn", "vpn.own.name", "vpn.own.connect", "vpn.own.disconnect", "vpn.admin"].iter().map(|k| format!("{}|", p.get(k).unwrap_or(&Value::Null))).collect();
        if std::mem::replace(&mut *last.lock().unwrap_or_else(|e| e.into_inner()), sig.clone()) != sig {
            publish_soon();
        }
    });
    publish_soon();
}

// ---- toggle ----

/// Command `vpn_toggle` (chip click, Controls tile, bar row, hotkey). `status`: ok | setup | off | error.
pub fn vpn_toggle() -> Value {
    if !enabled() {
        return json!({ "status": "off" });
    }
    let Some(own) = own() else {
        // nothing to start yet: show where to set the client up
        if let Some(app) = APP.get() {
            let _ = bus::emit("pill:open-tab", json!({ "tab": "settings", "section": "modules" }));
            crate::pill::expand(app, Some("settings".into()), "vpn");
        }
        return json!({ "status": "setup" });
    };
    let was_on = current().on;
    let cmd = if was_on { &own.disconnect } else { &own.connect };
    if let Err(e) = run(&Real, cmd, own.admin) {
        return json!({ "status": "error", "message": e });
    }
    LAST_ON.store(!was_on, Ordering::Relaxed);
    follow(!was_on);
    json!({ "status": "ok", "on": !was_on })
}

/// After a toggle the adapter takes a while: look again after 1.5, 3.5, 6.5, 10.5 and 15.5 s, then stop (never a standing timer).
fn follow(want: bool) {
    if FOLLOWING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        for wait in [1500u64, 2000, 3000, 4000, 5000] {
            std::thread::sleep(std::time::Duration::from_millis(wait));
            publish();
            if current().on == want {
                break;
            }
        }
        FOLLOWING.store(false, Ordering::SeqCst);
    });
}

// ---- command bar ----

/// Every typed word is the start of `vpn` / `впн` / `tunnel` (at least two letters each).
fn is_vpn_query(words: &[String]) -> bool {
    const KEYS: [&str; 5] = ["vpn", "впн", "tunnel", "туннель", "тоннель"];
    !words.is_empty() && words.iter().all(|w| w.chars().count() >= 2 && KEYS.iter().any(|k| k.starts_with(w.as_str())))
}

/// The "Turn VPN on / off" row for words like `vpn`, `впн`: (title, subtitle). Only when an own client is set.
pub fn bar_row(words: &[String], cyr: bool) -> Option<(String, String)> {
    if !is_vpn_query(words) {
        return None;
    }
    let s = current();
    if !s.enabled || !s.configured {
        return None;
    }
    let sub = if s.name.is_empty() { String::new() } else { s.name.clone() };
    let title = match (s.on, cyr) {
        (true, true) => "Выключить VPN",
        (true, false) => "Turn VPN off",
        (false, true) => "Включить VPN",
        (false, false) => "Turn VPN on",
    };
    Some((title.into(), if sub.is_empty() { "VPN".into() } else { sub }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    fn ad(alias: &str, desc: &str, ty: u32, up: bool) -> Ad {
        Ad { alias: alias.into(), desc: desc.into(), ty, up }
    }

    #[test]
    fn links_are_told_from_commands() {
        assert!(is_link("client://connect"));
        assert!(is_link("amneziavpn://connect?x=1"));
        assert!(!is_link(r"C:\Program Files\x.exe /a"));
        assert!(!is_link("rasdial MyVpn"));
        assert!(!is_link("://x"));
    }

    #[test]
    fn command_lines_split_like_a_person_reads_them() {
        assert_eq!(split_cmd(r#""C:\Program Files\WireGuard\wireguard.exe" /installtunnelservice "C:\a b\home.conf""#), Some((r"C:\Program Files\WireGuard\wireguard.exe".into(), r#"/installtunnelservice "C:\a b\home.conf""#.into())));
        assert_eq!(split_cmd(r"C:\Program Files\WireGuard\wireguard.exe /uninstalltunnelservice home"), Some((r"C:\Program Files\WireGuard\wireguard.exe".into(), "/uninstalltunnelservice home".into())));
        assert_eq!(split_cmd("rasdial MyVpn"), Some(("rasdial".into(), "MyVpn".into())));
        assert_eq!(split_cmd("rasdial"), Some(("rasdial".into(), String::new())));
        assert_eq!(split_cmd("  "), None);
    }

    #[test]
    fn wireguard_tunnel_forms() {
        let c = r#""C:\Program Files\WireGuard\wireguard.exe" /installtunnelservice "C:\Users\me\home vpn\de1.conf""#;
        assert_eq!(tunnel_name(c).as_deref(), Some("de1"));
        assert_eq!(derive_disconnect(c).as_deref(), Some(r#""C:\Program Files\WireGuard\wireguard.exe" /uninstalltunnelservice de1"#));
        assert_eq!(tunnel_name("rasdial MyVpn"), None);
        assert_eq!(derive_disconnect("client://connect"), None);
    }

    #[test]
    fn own_client_fills_in_name_and_disconnect() {
        let c = r#"awg.exe /installtunnelservice C:\x\office.conf"#;
        let o = own_from("", c, "", true).unwrap();
        assert_eq!((o.name.as_str(), o.disconnect.as_str()), ("office", "\"awg.exe\" /uninstalltunnelservice office"));
        let o = own_from("Amnezia", "client://connect", "client://disconnect", false).unwrap();
        assert_eq!((o.name.as_str(), o.disconnect.as_str()), ("Amnezia", "client://disconnect"));
        assert_eq!(own_from("x", "  ", "y", false), None);
    }

    struct Rec(RefCell<Vec<String>>);
    impl Launch for Rec {
        fn link(&self, url: &str) -> Result<(), String> {
            self.0.borrow_mut().push(format!("link {url}"));
            Ok(())
        }
        fn exe(&self, exe: &str, args: &str, admin: bool) -> Result<(), String> {
            self.0.borrow_mut().push(format!("exe {exe} | {args} | admin={admin}"));
            Ok(())
        }
    }

    #[test]
    fn admin_applies_to_commands_but_never_to_links() {
        let r = Rec(RefCell::new(vec![]));
        run(&r, "client://connect", true).unwrap();
        run(&r, r#""C:\wg\wireguard.exe" /installtunnelservice C:\wg\a.conf"#, true).unwrap();
        run(&r, "rasdial MyVpn", false).unwrap();
        assert_eq!(*r.0.borrow(), ["link client://connect", r"exe C:\wg\wireguard.exe | /installtunnelservice C:\wg\a.conf | admin=true", "exe rasdial | MyVpn | admin=false"]);
        assert!(run(&r, "  ", false).is_err());
    }

    #[test]
    fn an_own_client_is_judged_by_its_own_name_only() {
        let wg = ad("home", "WireGuard Tunnel", 53, true);
        let amn = ad("AmneziaWG", "AmneziaWG Tunnel", 53, true);
        let proxy_tun = ad("Clash", "Meta Tunnel", 53, true);
        let eth = ad("Ethernet", "Realtek PCIe GbE", 6, true);
        // someone else's tunnel is up, the own client (Amnezia) is not
        assert!(!judge(&[wg.clone(), proxy_tun.clone(), eth.clone()], Some("Amnezia")));
        assert!(judge(&[wg.clone(), amn.clone(), eth.clone()], Some("amnezia")));
        // a down adapter of the own client does not count
        assert!(!judge(&[ad("AmneziaWG", "x", 53, false)], Some("Amnezia")));
        // the description counts too
        assert!(judge(&[ad("Local Area Connection 3", "Amnezia Virtual Adapter", 6, true)], Some("Amnezia")));
    }

    #[test]
    fn without_an_own_client_only_vpn_looking_adapters_count() {
        assert!(judge(&[ad("home", "WireGuard Tunnel", 53, true)], None));
        assert!(judge(&[ad("Work", "WAN Miniport (PPTP)", 23, true)], None));
        assert!(!judge(&[ad("vEthernet (WSL)", "Hyper-V Virtual Ethernet Adapter", 6, true), ad("Tailscale", "Tailscale Tunnel", 53, true), ad("Teredo", "Teredo Tunneling Pseudo-Interface", 131, true)], None));
        assert!(!judge(&[ad("home", "WireGuard Tunnel", 53, false)], None));
        assert!(!judge(&[], None));
    }

    #[test]
    fn bar_words_match_vpn_prefixes() {
        let w = |s: &str| s.split_whitespace().map(str::to_string).collect::<Vec<_>>();
        for q in ["vpn", "vp", "впн", "tunnel", "туннель"] {
            assert!(is_vpn_query(&w(q)), "{q}");
        }
        for q in ["", "v", "vpnx", "chrome", "vpn chrome"] {
            assert!(!is_vpn_query(&w(q)), "{q}");
        }
    }
}
