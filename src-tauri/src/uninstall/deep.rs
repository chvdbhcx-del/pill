// Deep clean: evidence-based matching (Certain / Probable), the registry / service / task / firewall readers and the single check
// (`guard_cand`) every deletion goes through. Pure functions over the `Reg` trait and an `Env`, so every rule is tested on fixtures.
use super::*;

// Evidence strings (shown in the checklist; the page translates them).
pub const N_INSIDE: &str = "inside the install folder";
pub const N_NAME: &str = "named like the app";
pub const N_SEQ: &str = "contains the app's full name";
pub const N_PUB: &str = "named like the publisher";
const N_REG_PUB_ONLY: &str = "publisher key holding only this app";
const N_REG_UNDER_PUB: &str = "key under the publisher";
const N_DEAD_UNINSTALL: &str = "uninstall entry of this product, its uninstaller is gone";
const N_APPPATHS: &str = "App Paths entry in the install folder";
const N_COM: &str = "COM class or handler in the install folder";
const N_GUID: &str = "carries the product GUID";
const N_AUTORUN: &str = "starts the app at sign-in";
const N_HISTORY: &str = "install folder path in a history list";
const N_ASSOC: &str = "association with a removed handler";
const N_MSI: &str = "Windows Installer record of this product";
const N_SERVICE: &str = "service runs a file in the install folder";
const N_TASK: &str = "scheduled task starts a file in the install folder";
const N_TASK_NAME: &str = "task named like the app";
const N_FW: &str = "firewall rule for a program in the install folder";

// ---------------------------------------------------------------------------------------------------------------
// Registry reader
// ---------------------------------------------------------------------------------------------------------------

/// Read access to the registry; the real one reads, tests feed fixtures. Value names come back lower-cased, the default value as "".
pub trait Reg {
    fn subkeys(&self, h: Hive, sub: &str) -> Vec<String>;
    fn values(&self, h: Hive, sub: &str) -> Vals;
    fn exists(&self, h: Hive, sub: &str) -> bool;
}

pub struct RealReg;
impl Reg for RealReg {
    fn subkeys(&self, h: Hive, sub: &str) -> Vec<String> {
        Key::open(h.root(), sub, KEY_READ).map(|k| k.subkeys(20_000)).unwrap_or_default()
    }
    fn values(&self, h: Hive, sub: &str) -> Vals {
        Key::open(h.root(), sub, KEY_READ).map(|k| k.values()).unwrap_or_default()
    }
    fn exists(&self, h: Hive, sub: &str) -> bool {
        Key::open(h.root(), sub, KEY_READ).is_some()
    }
}

fn vstr(v: &Vals, k: &str) -> String {
    match v.get(k) {
        Some(Val::S(s)) | Some(Val::E(s)) => s.trim().to_string(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Names, tiers, GUIDs
// ---------------------------------------------------------------------------------------------------------------

/// Words of a name: split at anything that is not a letter or digit and at lower->UPPER camel boundaries, lower-cased.
pub fn tokens(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut prev_lower) = (Vec::new(), String::new(), false);
    for c in s.chars() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.extend(c.to_lowercase());
        prev_lower = c.is_lowercase();
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn has_seq(hay: &[String], seq: &[String]) -> bool {
    !seq.is_empty() && hay.windows(seq.len()).any(|w| w == seq)
}

/// How well a file / folder / key name matches the app. Product name equal = Certain; the app's full name as whole words inside a longer name
/// (never a substring of a word, never a generic word, at least 4 letters) = Probable; the publisher's own name = Probable (callers decide).
pub fn name_tier(raw: &str, n: &Names) -> Option<(Tier, &'static str)> {
    let k = norm(raw);
    if !usable(&k) {
        return None;
    }
    if n.product.contains(&k) {
        return Some((Tier::Certain, N_NAME));
    }
    if n.publisher.contains(&k) {
        return Some((Tier::Probable, N_PUB));
    }
    let t = tokens(raw);
    n.seqs.iter().any(|s| has_seq(&t, s)).then_some((Tier::Probable, N_SEQ))
}

/// `Foo-Setup-1.2.3.exe` / `foo_installer_x64.msi` for the app Foo: product name, a setup marker, then only version / arch words.
pub fn installer_name(file: &str, n: &Names) -> bool {
    const EXT: &[&str] = &[".exe", ".msi", ".msix", ".msixbundle", ".appx"];
    const FILLER: &[&str] = &["setup", "install", "installer", "installation", "x64", "x86", "win", "win32", "win64", "windows", "amd64", "msi", "exe", "full", "offline", "online", "web", "stable", "release", "en", "us", "v", "ru"];
    let low = file.to_lowercase();
    let Some(ext) = EXT.iter().find(|e| low.ends_with(*e)) else { return false };
    let t = tokens(&file[..file.len() - ext.len()]);
    if !t.iter().any(|w| matches!(w.as_str(), "setup" | "install" | "installer" | "installation")) {
        return false;
    }
    (1..=t.len()).any(|k| {
        let head = t[..k].concat();
        n.product.contains(&head) && usable(&head) && t[k..].iter().all(|w| FILLER.contains(&w.as_str()) || w.chars().all(|c| c.is_ascii_digit()) || (w.starts_with('v') && w.len() > 1 && w[1..].chars().all(|c| c.is_ascii_digit())) || (w.starts_with('v') && w.len() > 1 && w[1..].chars().all(|c| c.is_ascii_digit())))
    })
}

/// Windows Installer's "packed" GUID: `{AABBCCDD-EEFF-0011-2233-445566778899}` -> `DDCCBBAAFFEE1100332255447766998 8`-style.
pub fn pack_guid(g: &str) -> Option<String> {
    let d: String = g.trim_matches(|c| c == '{' || c == '}').chars().filter(|c| *c != '-').collect();
    if d.len() != 32 || !d.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let rev = |s: &str| s.chars().rev().collect::<String>();
    let swap: String = d[16..].as_bytes().chunks(2).map(|c| format!("{}{}", c[1] as char, c[0] as char)).collect();
    Some(format!("{}{}{}{}", rev(&d[..8]), rev(&d[8..12]), rev(&d[12..16]), swap).to_uppercase())
}

/// True when the command line / path `s` starts a file inside the install folder.
pub fn points_into(s: &str, install: Option<&Path>) -> bool {
    let Some(i) = install else { return false };
    let s = expand_env(s.trim());
    let s = s.trim_start_matches(r"\??\").to_string();
    let head = split_cmd(&s).map(|x| x.0).unwrap_or_default();
    // the command's file (quoted, or ending in .exe) or the whole text (an unquoted path with spaces, a .dll, a registry path value)
    [head.as_str(), s.trim_matches('"')].iter().any(|f| !f.contains("..") && depth_under(Path::new(f.trim_matches('"')), i).is_some())
}

impl Prog {
    /// The recorded install folder, as a path.
    pub fn install(&self) -> Option<PathBuf> {
        (!self.location.is_empty()).then(|| PathBuf::from(self.location.trim_end_matches('\\')))
    }
    /// GUIDs that belong to this product (upper case): ProductCode from the key name, the MSI command line, the uninstall strings.
    pub fn guids(&self) -> Vec<String> {
        let mut v: Vec<String> = [self.msi.clone(), find_guid(self.sub.rsplit('\\').next().unwrap_or("")), find_guid(&self.uninst), find_guid(&self.quiet)]
            .into_iter()
            .flatten()
            .map(|g| g.to_uppercase())
            .collect();
        v.extend(self.extra_guids.iter().map(|g| g.to_uppercase()));
        v.sort();
        v.dedup();
        v
    }
}

/// Install folder as evidence: only when it is a believable app folder.
fn evid_install(p: &Prog, env: &Env) -> Option<PathBuf> {
    p.install().filter(|i| install_usable(i, env))
}

// ---------------------------------------------------------------------------------------------------------------
// Registry evidence
// ---------------------------------------------------------------------------------------------------------------

const CV: &str = r"microsoft\windows\currentversion";
const PATH_LISTS: &[(&str, bool)] = &[
    // (key below `software`, the value NAME is the path; otherwise the value DATA is)
    (r"microsoft\windows\currentversion\explorer\featureusage\appswitched", true),
    (r"microsoft\windows\currentversion\explorer\featureusage\applaunch", true),
    (r"microsoft\windows\currentversion\explorer\featureusage\showjumpview", true),
    (r"classes\local settings\software\microsoft\windows\shell\muicache", true),
    (r"microsoft\windows\shellnoroam\muicache", true),
    (r"microsoft\windows nt\currentversion\appcompatflags\compatibility assistant\store", true),
    (r"microsoft\windows nt\currentversion\appcompatflags\layers", true),
    (r"microsoft\windows\currentversion\explorer\runmru", false),
    (r"microsoft\windows\currentversion\installer\folders", true),
];
const FIREWALL_RULES: &str = r"system\currentcontrolset\services\sharedaccess\parameters\firewallpolicy\firewallrules";
const SERVICES: &str = r"SYSTEM\CurrentControlSet\Services";
const SHELL_PARENTS: &[&str] = &["*", "directory", "directory\\background", "folder", "allfilesystemobjects", "drive"];

fn is_guid(s: &str) -> bool {
    s.len() == 38 && find_guid(s).is_some()
}

fn split_sub(sub: &str) -> Option<Vec<String>> {
    if sub.is_empty() || sub.len() > 500 || sub.contains("..") || sub.contains('\0') {
        return None;
    }
    let p: Vec<String> = sub.split('\\').map(str::to_string).collect();
    (!p.iter().any(|x| x.is_empty())).then_some(p)
}

/// "Software\Wow6432Node\X" -> ("software", index of the first part after the optional wow6432node).
fn sw_root(lp: &[String]) -> Option<usize> {
    if lp.first().map(String::as_str) != Some("software") {
        return None;
    }
    Some(if lp.get(1).map(String::as_str) == Some("wow6432node") { 2 } else { 1 })
}

fn sole_content(h: Hive, sub: &str, n: &Names, reg: &dyn Reg) -> bool {
    reg.values(h, sub).is_empty() && reg.subkeys(h, sub).iter().all(|c| matches!(name_tier(c, n), Some((_, t)) if t != N_PUB))
}

fn clsid_ok(h: Hive, classes: &str, guid: &str, install: Option<&Path>, guids: &[String], reg: &dyn Reg) -> bool {
    let sub = format!(r"{classes}\CLSID\{guid}");
    if guids.contains(&guid.to_uppercase()) {
        return reg.exists(h, &sub);
    }
    ["InprocServer32", "LocalServer32"].iter().any(|k| {
        let v = reg.values(h, &format!(r"{sub}\{k}"));
        points_into(&vstr(&v, ""), install) || points_into(&vstr(&v, "codebase"), install)
    })
}

/// Evidence that a registry KEY belongs to the app. The same call decides what the scan offers and what a delete may touch.
fn key_evidence(h: Hive, sub: &str, p: &Prog, env: &Env, reg: &dyn Reg) -> Result<(Tier, &'static str), &'static str> {
    let parts = split_sub(sub).ok_or("not an app key")?;
    let lp: Vec<String> = parts.iter().map(|x| x.to_lowercase()).collect();
    let k = sw_root(&lp).ok_or("not below Software")?;
    if parts.len() <= k {
        return Err("a Software root");
    }
    let (names, install, guids) = (names_of(p), evid_install(p, env), p.guids());
    let rest = &parts[k..];
    let lr = &lp[k..];
    match lr[0].as_str() {
        "classes" => {
            // Software\Classes[\WOW6432Node]\...
            let ci = if lr.get(1).map(String::as_str) == Some("wow6432node") { 2 } else { 1 };
            let classes = parts[..k + ci].join("\\");
            let r = &rest[ci..];
            let lc = &lr[ci..];
            match lc.len() {
                0 => Err("a Classes root"),
                1 => {
                    // a ProgID / protocol handler: not a GUID, not an extension, not a structural key
                    if lc[0].starts_with('.') || is_guid(&r[0]) || matches!(lc[0].as_str(), "clsid" | "appid" | "typelib" | "interface" | "installer" | "mime" | "local settings" | "*" | "directory" | "folder" | "drive" | "allfilesystemobjects" | "wow6432node") {
                        return Err("not a handler key");
                    }
                    let cmd = |k: &str| vstr(&reg.values(h, &format!(r"{sub}\{k}")), "");
                    let verbs = reg.subkeys(h, &format!(r"{sub}\shell"));
                    let hit = points_into(&cmd(r"shell\open\command"), install.as_deref())
                        || points_into(&cmd("DefaultIcon"), install.as_deref())
                        || verbs.iter().any(|v| points_into(&cmd(&format!(r"shell\{v}\command")), install.as_deref()))
                        || find_guid(&cmd("CLSID")).is_some_and(|g| clsid_ok(h, &classes, &g, install.as_deref(), &guids, reg));
                    if hit { Ok((Tier::Certain, N_COM)) } else { Err("no evidence") }
                }
                2 if matches!(lc[0].as_str(), "clsid" | "appid" | "typelib" | "interface") && is_guid(&r[1]) => {
                    let g = r[1].to_uppercase();
                    if guids.contains(&g) && reg.exists(h, sub) {
                        Ok((Tier::Certain, N_GUID))
                    } else if lc[0] == "clsid" && clsid_ok(h, &classes, &g, install.as_deref(), &guids, reg) {
                        Ok((Tier::Certain, N_COM))
                    } else {
                        Err("no evidence")
                    }
                }
                3 if lc[0] == "installer" && matches!(lc[1].as_str(), "products" | "features") => {
                    if h == Hive::Hklm && guids.iter().filter_map(|g| pack_guid(g)).any(|pk| pk.eq_ignore_ascii_case(&r[2])) && reg.exists(h, sub) {
                        Ok((Tier::Certain, N_MSI))
                    } else {
                        Err("no evidence")
                    }
                }
                _ => {
                    // Classes\<parent>\shellex\ContextMenuHandlers\<h>  and  Classes\<parent>\shell\<verb>
                    let (par_len, rest_l) = if lc.len() > 2 && lc[0] == "directory" && lc[1] == "background" { (2, &lc[2..]) } else { (1, &lc[1..]) };
                    let par = lc[..par_len].join("\\");
                    if !SHELL_PARENTS.contains(&par.as_str()) {
                        return Err("not a handler key");
                    }
                    match rest_l {
                        [a, b, handler] if a == "shellex" && b == "contextmenuhandlers" => {
                            let def = vstr(&reg.values(h, sub), "");
                            let by_clsid = find_guid(&def).is_some_and(|g| clsid_ok(h, &classes, &g, install.as_deref(), &guids, reg));
                            let by_name = name_tier(handler, &names).is_some_and(|(t, n)| t == Tier::Certain && n == N_NAME);
                            if by_clsid { Ok((Tier::Certain, N_COM)) } else if by_name { Ok((Tier::Certain, N_NAME)) } else { Err("no evidence") }
                        }
                        [a, _verb] if a == "shell" => {
                            if points_into(&vstr(&reg.values(h, &format!(r"{sub}\command")), ""), install.as_deref()) { Ok((Tier::Certain, N_COM)) } else { Err("no evidence") }
                        }
                        _ => Err("not a handler key"),
                    }
                }
            }
        }
        "microsoft" => {
            let cv: Vec<&str> = CV.split('\\').collect();
            if lr.len() < 5 || lr[1..3] != [cv[1].to_string(), cv[2].to_string()] {
                return Err("not an app key");
            }
            match (lr[3].as_str(), lr.len()) {
                ("uninstall", 5) => {
                    let v = reg.values(h, sub);
                    if v.is_empty() && !reg.exists(h, sub) {
                        return Err("no such key");
                    }
                    let dead = split_cmd(&expand_env(&vstr(&v, "uninstallstring"))).map_or(true, |(f, _)| !Path::new(&f).is_file());
                    let by_guid = guids.contains(&rest[4].to_uppercase());
                    let by_name = norm(&vstr(&v, "displayname")).is_empty() == false && names.product.contains(&norm(&vstr(&v, "displayname")));
                    let by_path = ["installlocation", "uninstallstring", "displayicon"].iter().any(|k| points_into(&vstr(&v, k), install.as_deref()));
                    if dead && (by_guid || by_name || by_path) { Ok((Tier::Certain, N_DEAD_UNINSTALL)) } else { Err("no evidence") }
                }
                ("app paths", 5) => {
                    let v = reg.values(h, sub);
                    if points_into(&vstr(&v, ""), install.as_deref()) || points_into(&vstr(&v, "path"), install.as_deref()) { Ok((Tier::Certain, N_APPPATHS)) } else { Err("no evidence") }
                }
                ("installer", 8) if h == Hive::Hklm && lr[4] == "userdata" && lr[6] == "products" && lr[5].starts_with("s-1-") => {
                    if guids.iter().filter_map(|g| pack_guid(g)).any(|pk| pk.eq_ignore_ascii_case(&rest[7])) && reg.exists(h, sub) { Ok((Tier::Certain, N_MSI)) } else { Err("no evidence") }
                }
                _ => Err("not an app key"),
            }
        }
        _ => {
            // Software\<Name>  /  Software\<Publisher>\<Name>
            match rest.len() {
                1 => match name_tier(&rest[0], &names) {
                    Some((t, n)) if n != N_PUB => Ok((t, n)),
                    Some(_) if sole_content(h, sub, &names, reg) => Ok((Tier::Certain, N_REG_PUB_ONLY)),
                    _ => Err("name does not match the app"),
                },
                2 => {
                    if !names.publisher.contains(&norm(&rest[0])) {
                        return Err("publisher does not match");
                    }
                    match name_tier(&rest[1], &names) {
                        Some((t, n)) if n != N_PUB => Ok((t, if t == Tier::Certain { N_REG_UNDER_PUB } else { n })),
                        _ => Err("name does not match the app"),
                    }
                }
                _ => Err("not an app key"),
            }
        }
    }
}

/// Evidence that a registry VALUE belongs to the app (a Run entry, a path in a history list, a handler association, a Windows Installer record).
fn value_evidence(h: Hive, sub: &str, name: &str, p: &Prog, env: &Env, reg: &dyn Reg) -> Result<(Tier, &'static str), &'static str> {
    let parts = split_sub(sub).ok_or("not an app value")?;
    let lp: Vec<String> = parts.iter().map(|x| x.to_lowercase()).collect();
    let nm = name.to_lowercase();
    if name.is_empty() || name.contains('\0') {
        return Err("no value name");
    }
    let vals = reg.values(h, sub);
    let data = vals.get(&nm).cloned().ok_or("no such value")?;
    let install = evid_install(p, env);
    let data_s = match &data {
        Val::S(s) | Val::E(s) => s.clone(),
        _ => String::new(),
    };
    let joined = lp.join("\\");
    if joined == FIREWALL_RULES && h == Hive::Hklm {
        let app = data_s.split('|').find_map(|f| f.strip_prefix("App=")).unwrap_or("");
        return if points_into(app, install.as_deref()) { Ok((Tier::Certain, N_FW)) } else { Err("no evidence") };
    }
    let k = sw_root(&lp).ok_or("not below Software")?;
    let lr: Vec<&str> = lp[k..].iter().map(String::as_str).collect();
    let cvs: Vec<&str> = CV.split('\\').collect();
    // Run / RunOnce
    if lr.len() == 4 && lr[..3] == cvs[..] && matches!(lr[3], "run" | "runonce") {
        return if points_into(&data_s, install.as_deref()) { Ok((Tier::Certain, N_AUTORUN)) } else { Err("no evidence") };
    }
    // path-keyed history lists
    let rel = lr.join("\\");
    if let Some((_, by_name)) = PATH_LISTS.iter().find(|(l, _)| *l == rel) {
        let s = if *by_name { name } else { data_s.as_str() };
        return if points_into(s, install.as_deref()) { Ok((Tier::Certain, N_HISTORY)) } else { Err("no evidence") };
    }
    // Windows Installer component records: the value NAME is the packed ProductCode
    if h == Hive::Hklm && lr.len() == 8 && lr[..3] == cvs[..] && lr[3] == "installer" && lr[4] == "userdata" && lr[5].starts_with("s-1-") && lr[6] == "components" {
        return if p.guids().iter().filter_map(|g| pack_guid(g)).any(|pk| pk.eq_ignore_ascii_case(name)) { Ok((Tier::Certain, N_MSI)) } else { Err("no evidence") };
    }
    // .ext\OpenWithProgids: the value NAME is a ProgID of this app
    if lr.len() >= 3 && lr[0] == "classes" && lr[lr.len() - 2].starts_with('.') && lr[lr.len() - 1] == "openwithprogids" {
        let classes = parts[..k + 1].join("\\");
        let progid_key = format!(r"{classes}\{name}");
        return match key_evidence(h, &progid_key, p, env, reg) {
            Ok((Tier::Certain, _)) => Ok((Tier::Certain, N_ASSOC)),
            _ => Err("no evidence"),
        };
    }
    Err("not an app value")
}

fn valid_service_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ' | '$'))
}

fn service_evidence(name: &str, p: &Prog, env: &Env, reg: &dyn Reg) -> Result<(Tier, &'static str), &'static str> {
    if !valid_service_name(name) {
        return Err("bad service name");
    }
    let v = reg.values(Hive::Hklm, &format!(r"{SERVICES}\{name}"));
    let ty = match v.get("type") {
        Some(Val::D(d)) => *d,
        _ => return Err("no such service"),
    };
    if ty & 0x0F != 0 || ty & 0x30 == 0 {
        return Err("a driver, not a service"); // kernel / file system drivers are only listed
    }
    if points_into(&vstr(&v, "imagepath"), evid_install(p, env).as_deref()) { Ok((Tier::Certain, N_SERVICE)) } else { Err("no evidence") }
}

/// Task paths look like `\Vendor\Task Name`: letters, digits, space and a few marks only, no `..`.
fn valid_task_path(s: &str) -> bool {
    s.starts_with('\\') && s.len() < 260 && !s.contains("..") && !s.contains("\\\\") && s.chars().all(|c| c.is_alphanumeric() || matches!(c, '\\' | ' ' | '_' | '-' | '.' | '{' | '}' | '(' | ')' | '+' | '#' | ','))
}

/// Text of a Task Scheduler XML file (UTF-16 with BOM, or UTF-8).
pub fn task_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        String::from_utf16_lossy(&bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())
    } else {
        String::from_utf8_lossy(bytes).trim_start_matches('\u{feff}').to_string()
    }
}

/// Every `<Command>` of a task's XML.
pub fn task_commands(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<Command>") {
        let after = &rest[i + 9..];
        let Some(j) = after.find("</Command>") else { break };
        out.push(after[..j].replace("&quot;", "\"").replace("&amp;", "&").replace("&apos;", "'").trim().to_string());
        rest = &after[j..];
    }
    out
}

fn task_evidence(path: &str, p: &Prog, env: &Env) -> Result<(Tier, &'static str), &'static str> {
    if !valid_task_path(path) {
        return Err("bad task path");
    }
    let file = env.windows.join("System32").join("Tasks").join(path.trim_start_matches('\\'));
    let bytes = std::fs::read(&file).map_err(|_| "task is not readable")?;
    let install = evid_install(p, env);
    if task_commands(&task_text(&bytes)).iter().any(|c| points_into(c, install.as_deref())) {
        return Ok((Tier::Certain, N_TASK));
    }
    let leaf = path.rsplit('\\').next().unwrap_or("");
    match name_tier(leaf, &names_of(p)) {
        Some((t, n)) if n == N_NAME && t == Tier::Certain => Ok((Tier::Probable, N_TASK_NAME)),
        _ => Err("no evidence"),
    }
}

/// THE check behind every deletion: Ok only when the item is structurally allowed AND still carries evidence for this app.
pub fn guard_cand(w: &What, p: &Prog, env: &Env, reg: &dyn Reg) -> Result<(Tier, &'static str), &'static str> {
    match w {
        What::Fs(path) => {
            guard_deep(path, env, p)?;
            if evid_install(p, env).is_some_and(|i| depth_under(path, &i).is_some()) {
                return Ok((Tier::Certain, N_INSIDE));
            }
            let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok(name_tier(&leaf, &names_of(p)).unwrap_or((Tier::Probable, N_NAME)))
        }
        What::Key(h, sub) => key_evidence(*h, sub, p, env, reg),
        What::Value(h, sub, name) => value_evidence(*h, sub, name, p, env, reg),
        What::Service(n) => service_evidence(n, p, env, reg),
        What::Task(t) => task_evidence(t, p, env),
        What::Remote(_) => Err("not verified in this process"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Registry / service / task scans
// ---------------------------------------------------------------------------------------------------------------

fn push(out: &mut Vec<Cand>, w: What, p: &Prog, env: &Env, reg: &dyn Reg) {
    if let Ok((tier, note)) = guard_cand(&w, p, env, reg) {
        out.push(Cand::new(w, tier, note, 0));
    }
}

/// Registry candidates of both hives, services and firewall rules (HKLM is read-only here). `step` reports the stage.
pub fn scan_registry(p: &Prog, env: &Env, reg: &dyn Reg, b: &mut Budget, step: &dyn Fn(&str, usize)) -> Vec<Cand> {
    let names = names_of(p);
    let guids = p.guids();
    let packed: Vec<String> = guids.iter().filter_map(|g| pack_guid(g)).collect();
    let mut out: Vec<Cand> = Vec::new();
    for h in [Hive::Hkcu, Hive::Hklm] {
        step(if h == Hive::Hkcu { "registry (user)" } else { "registry (system)" }, out.len());
        let (sw, wow) = if h == Hive::Hkcu { ("Software", r"Software\Wow6432Node") } else { ("SOFTWARE", r"SOFTWARE\WOW6432Node") };
        for base in [sw, wow] {
            // Software\<Name> and Software\<Publisher>\<Name>
            for c in reg.subkeys(h, base) {
                if !b.tick() {
                    break;
                }
                if name_tier(&c, &names).is_none() {
                    continue;
                }
                let sub = format!(r"{base}\{c}");
                push(&mut out, What::Key(h, sub.clone()), p, env, reg);
                if names.publisher.contains(&norm(&c)) {
                    for g in reg.subkeys(h, &sub) {
                        push(&mut out, What::Key(h, format!(r"{sub}\{g}")), p, env, reg);
                    }
                }
            }
            // Uninstall remnants, App Paths
            for tail in ["Uninstall", "App Paths"] {
                let b2 = format!(r"{base}\Microsoft\Windows\CurrentVersion\{tail}");
                for c in reg.subkeys(h, &b2) {
                    if !b.tick() {
                        break;
                    }
                    push(&mut out, What::Key(h, format!(r"{b2}\{c}")), p, env, reg);
                }
            }
        }
        // Classes
        let mut progids: Vec<String> = Vec::new();
        for classes in [format!(r"{sw}\Classes"), format!(r"{sw}\Classes\WOW6432Node")] {
            let wow_c = classes.ends_with("WOW6432Node");
            if !wow_c {
                for c in reg.subkeys(h, &classes) {
                    if !b.tick() {
                        break;
                    }
                    let sub = format!(r"{classes}\{c}");
                    let before = out.len();
                    push(&mut out, What::Key(h, sub), p, env, reg);
                    if out.len() > before && !c.starts_with('.') {
                        progids.push(c);
                    }
                }
            }
            for c in reg.subkeys(h, &format!(r"{classes}\CLSID")) {
                if !b.tick() {
                    break;
                }
                push(&mut out, What::Key(h, format!(r"{classes}\CLSID\{c}")), p, env, reg);
            }
            for g in &guids {
                for t in ["AppID", "TypeLib", "Interface"] {
                    push(&mut out, What::Key(h, format!(r"{classes}\{t}\{g}")), p, env, reg);
                }
            }
            for pk in &packed {
                for t in ["Products", "Features"] {
                    push(&mut out, What::Key(h, format!(r"{classes}\Installer\{t}\{pk}")), p, env, reg);
                }
            }
            if !wow_c {
                for par in ["*", "Directory", r"Directory\Background", "Folder", "AllFileSystemObjects", "Drive"] {
                    for hd in reg.subkeys(h, &format!(r"{classes}\{par}\shellex\ContextMenuHandlers")) {
                        push(&mut out, What::Key(h, format!(r"{classes}\{par}\shellex\ContextMenuHandlers\{hd}")), p, env, reg);
                    }
                    for v in reg.subkeys(h, &format!(r"{classes}\{par}\shell")) {
                        push(&mut out, What::Key(h, format!(r"{classes}\{par}\shell\{v}")), p, env, reg);
                    }
                }
            }
        }
        if !progids.is_empty() {
            let classes = format!(r"{sw}\Classes");
            for c in reg.subkeys(h, &classes).into_iter().filter(|c| c.starts_with('.')) {
                let sub = format!(r"{classes}\{c}\OpenWithProgids");
                for (n, _) in reg.values(h, &sub) {
                    if progids.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                        push(&mut out, What::Value(h, sub.clone(), n), p, env, reg);
                    }
                }
            }
        }
        // Run, history lists
        let mut lists: Vec<String> = vec![format!(r"{sw}\Microsoft\Windows\CurrentVersion\Run"), format!(r"{sw}\Microsoft\Windows\CurrentVersion\RunOnce"), format!(r"{wow}\Microsoft\Windows\CurrentVersion\Run")];
        lists.extend(PATH_LISTS.iter().map(|(l, _)| format!(r"{sw}\{l}")));
        for sub in lists {
            for (n, _) in reg.values(h, &sub) {
                if !b.tick() {
                    break;
                }
                push(&mut out, What::Value(h, sub.clone(), n), p, env, reg);
            }
        }
    }
    // Windows Installer records (HKLM; only readable as administrator, the elevated scan sees them)
    step("Windows Installer records", out.len());
    let ud = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Installer\UserData";
    for sid in reg.subkeys(Hive::Hklm, ud) {
        for pk in &packed {
            push(&mut out, What::Key(Hive::Hklm, format!(r"{ud}\{sid}\Products\{pk}")), p, env, reg);
        }
        let comps = format!(r"{ud}\{sid}\Components");
        for c in reg.subkeys(Hive::Hklm, &comps) {
            if !b.tick() {
                break;
            }
            let sub = format!(r"{comps}\{c}");
            let v = reg.values(Hive::Hklm, &sub);
            for pk in &packed {
                if v.contains_key(&pk.to_lowercase()) {
                    push(&mut out, What::Value(Hive::Hklm, sub.clone(), pk.to_lowercase()), p, env, reg);
                }
            }
        }
    }
    step("firewall rules", out.len());
    let fw = FIREWALL_RULES.replace("system\\currentcontrolset", "SYSTEM\\CurrentControlSet");
    for (n, _) in reg.values(Hive::Hklm, &fw) {
        if !b.tick() {
            break;
        }
        push(&mut out, What::Value(Hive::Hklm, fw.clone(), n), p, env, reg);
    }
    step("services", out.len());
    for s in reg.subkeys(Hive::Hklm, SERVICES) {
        if !b.tick() {
            break;
        }
        push(&mut out, What::Service(s), p, env, reg);
    }
    step("scheduled tasks", out.len());
    out.extend(scan_tasks(p, env, b));
    out
}

/// Scheduled tasks whose action starts a file of the app (task files are read from System32\Tasks; a task the user cannot read is not seen).
pub fn scan_tasks(p: &Prog, env: &Env, b: &mut Budget) -> Vec<Cand> {
    let root = env.windows.join("System32").join("Tasks");
    let mut out = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        for (name, path, is_dir) in read_dir(&d, b) {
            if is_dir {
                if depth < 3 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            let rel = path.strip_prefix(&root).map(|r| format!(r"\{}", r.to_string_lossy())).unwrap_or(name);
            if let Ok((tier, note)) = task_evidence(&rel, p, env) {
                out.push(Cand::new(What::Task(rel), tier, note, 0));
            }
        }
    }
    out
}

/// Kernel drivers and file-system drivers whose image is in the install folder: listed only, never removed.
pub fn list_drivers(p: &Prog, env: &Env, reg: &dyn Reg) -> Vec<String> {
    let install = evid_install(p, env);
    reg.subkeys(Hive::Hklm, SERVICES)
        .into_iter()
        .filter(|s| {
            let v = reg.values(Hive::Hklm, &format!(r"{SERVICES}\{s}"));
            matches!(v.get("type"), Some(Val::D(t)) if t & 0x0B != 0) && points_into(&vstr(&v, "imagepath"), install.as_deref())
        })
        .map(|s| format!("Driver service: {s}"))
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Files: the wider guard
// ---------------------------------------------------------------------------------------------------------------

/// Top-level folder names of a drive that are never an app's.
pub const DRIVE_PROTECTED: &[&str] = &[
    "windows", "programfiles", "programfilesx86", "programdata", "users", "recovery", "systemvolumeinformation", "recyclebin", "perflogs", "boot", "documentsandsettings",
    "efi", "msocache", "configmsi", "winagent", "windowsold", "sources", "system", "bootmgr", "swsetup", "drivers", "intel", "amd", "nvidia",
];

pub fn is_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut r| r.next().is_none())
}

/// Like `guard`, plus the wider places a deep clean looks in: other drives' top levels (two deep), Documents / Saved Games / Temp / Common Files /
/// Windows\Temp (a name must match the app), Downloads (an installer file named like the app), other users' profiles, and the emptied parent
/// folders of the install folder. All of them still refuse links, protected folders and generic names.
pub fn guard_deep(path: &Path, env: &Env, p: &Prog) -> Result<(), &'static str> {
    let install = p.install();
    let first = guard_core(path, env, install.as_deref(), None);
    let Err(e) = first else { return Ok(()) };
    if !matches!(e, "not inside an allowed folder" | "generic folder name" | "inside Windows") {
        return Err(e);
    }
    guard_core(path, env, install.as_deref(), Some(p))
}

/// The leaf (and, at depth two, its folder) of `p` below `par` must carry the app's name.
fn named_leaf(path: &Path, par: &Path, max_depth: usize, names: &Names) -> bool {
    let d = depth_under(path, par).unwrap_or(0);
    let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    (1..=max_depth).contains(&d) && name_tier(&leaf, names).is_some()
}

pub(super) fn guard_core(path: &Path, env: &Env, install: Option<&Path>, deep: Option<&Prog>) -> Result<(), &'static str> {
    lexical_ok(path)?;
    if env.protected().iter().any(|f| depth_under(f, path).is_some()) {
        return Err("protected folder or a parent of one");
    }
    let win_temp = env.windows.join("Temp");
    let in_win_temp = deep.is_some() && depth_under(path, &win_temp).is_some_and(|d| d >= 1);
    if depth_under(path, &env.windows).is_some() && !in_win_temp {
        return Err("inside Windows");
    }
    if env.pill.iter().any(|d| depth_under(path, d).is_some()) {
        return Err("Pill's own folder");
    }
    let install = install.filter(|i| install_usable(i, env));
    let parents = env.parents();
    let regular = parents.iter().filter(|r| depth_under(path, r).is_some_and(|d| d >= 1)).max_by_key(|r| r.components().count());
    let mut generic = false;
    let mut chosen: Option<PathBuf> = None;
    if let Some(r) = regular {
        let first = path.components().nth(r.components().count()).map(|c| norm(&c.as_os_str().to_string_lossy())).unwrap_or_default();
        if usable(&first) {
            chosen = Some(r.clone());
        } else {
            generic = true;
        }
    }
    if chosen.is_none() {
        if let Some(i) = install.filter(|i| depth_under(path, i).is_some()) {
            if depth_under(path, i) != Some(0) || !leaf_generic(path) {
                chosen = Some(i.to_path_buf());
            }
        }
    }
    if chosen.is_none() {
        let Some(p) = deep else {
            return Err(if generic { "generic folder name" } else { "not inside an allowed folder" });
        };
        let names = names_of(p);
        // places where the NAME has to match
        let mut named: Vec<(PathBuf, usize, bool)> = Vec::new(); // (parent, max depth, installer files only)
        for (pd, _) in env.profiles() {
            named.push((pd.docs.clone(), 2, false));
            named.push((pd.saved.clone(), 2, false));
            named.push((pd.temp.clone(), 1, false));
            named.push((pd.downloads.clone(), 1, true));
        }
        for pf in [&env.pf, &env.pf86] {
            named.push((pf.join("Common Files"), 2, false));
        }
        named.push((win_temp.clone(), 1, false));
        let leaf = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        chosen = named
            .iter()
            .filter(|(np, _, _)| depth_under(path, np).is_some_and(|d| d >= 1))
            .max_by_key(|(np, _, _)| np.components().count())
            .and_then(|(np, md, inst)| {
                let ok = if *inst { depth_under(path, np) == Some(1) && installer_name(&leaf, &names) } else { named_leaf(path, np, *md, &names) };
                ok.then(|| np.clone())
            });
        if chosen.is_none() {
            // a drive's top level and one folder below it
            for d in &env.drives {
                if let Some(dep) = depth_under(path, d).filter(|d| (1..=2).contains(d)) {
                    let c1 = path.components().nth(d.components().count()).map(|c| norm(&c.as_os_str().to_string_lossy())).unwrap_or_default();
                    if !DRIVE_PROTECTED.contains(&c1.as_str()) && named_leaf(path, d, dep, &names) {
                        chosen = Some(d.clone());
                    }
                }
            }
        }
        if chosen.is_none() {
            // the emptied folder(s) that held the install folder
            if let Some(i) = p.install().filter(|i| install_usable(i, env)) {
                if depth_under(&i, path).is_some_and(|d| (1..=2).contains(&d)) && is_empty_dir(path) {
                    chosen = path.parent().map(Path::to_path_buf);
                }
            }
        }
        if chosen.is_none() {
            return Err(if generic { "generic folder name" } else { "not inside an allowed folder" });
        }
    }
    let par = chosen.ok_or("not inside an allowed folder")?;
    let cp = std::fs::canonicalize(path).map_err(|_| "does not exist")?;
    let cr = std::fs::canonicalize(&par).map_err(|_| "allowed folder is not readable")?;
    let tail: PathBuf = path.components().skip(par.components().count()).collect();
    if comps(&plain(cp)) != comps(&plain(cr.join(tail))) {
        return Err("link, junction or alternative name");
    }
    Ok(())
}
