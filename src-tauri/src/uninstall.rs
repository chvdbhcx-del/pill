// Uninstall apps completely from the command bar (`u foo`, `del foo`, `uninstall foo`, `удалить foo`): the app's own uninstaller first,
// then a STRICT leftover scan the user ticks through. Switch: Settings > Modules, `module.uninstall` (default on).
// Safety model: nothing is removed without the confirm card; leftovers go to the Recycle Bin (never a plain delete from here), every HKCU key
// is exported to %LOCALAPPDATA%\Pill\uninstall-backups first; HKLM, services and tasks are only listed. Every deletion path goes through
// `guard` / `key_guard` / `run_guard`, and the page only ever sends indexes into the server-side lists of the last list / scan.
// Commands: uninstall_info uninstall_start uninstall_scan uninstall_delete uninstall_cancel uninstall_state.
// Event (window bar): uninstall:state {phase: running|still|scanning|review|deleting|done|error, name, message, items[], manual[], ...}.
use serde_json::{json, Value};
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use windows::core::{w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, WIN32_ERROR};
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegDeleteValueW, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    KEY_SET_VALUE, REG_SAM_FLAGS,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::Win32::UI::Shell::{ShellExecuteExW, SHFileOperationW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, SHFILEOPSTRUCTW};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const NO_WINDOW: u32 = 0x0800_0000;
const OK: WIN32_ERROR = WIN32_ERROR(0);
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const WAIT_MAX: Duration = Duration::from_secs(45 * 60); // the uninstaller's own window may sit there; it is never killed
const SETTLE_MAX: Duration = Duration::from_secs(90); // after the process exits: some uninstallers hand over to a temp copy and exit at once

/// Folder / key names that never count as "the app's own" (compared normalised, exact equality).
const GENERIC: &[&str] = &[
    "app", "apps", "data", "tools", "common", "microsoft", "windows", "system", "program", "programs", "update", "updates", "install", "setup", "cache", "temp",
    "logs", "software", "files", "local", "roaming", "user", "users", "default", "config", "settings", "shared", "bin", "lib", "plugins", "resources", "assets",
    "commonfiles", "windowsapps", "windowsnt", "packages", "microsoftshared", "startup", "accessories", "classes", "policies", "wow6432node",
];
const CORP: &[&str] = &["inc", "llc", "ltd", "corp", "corporation", "gmbh", "co", "company", "limited", "sa", "ag", "bv", "oy", "plc", "software", "the"];
/// Runtimes, drivers and SDKs: hidden unless the query has one of RISKY_WORDS.
const RISKY_NAMES: &[&str] = &["visual c++", ".net", "webview2", "driver", "redistributable", "runtime", "directx", "vulkan", "physx", "windows sdk", "software development kit"];
const RISKY_WORDS: &[&str] = &["all", "system", "sys", "runtime", "driver", "drivers", "все", "системные"];

// ---------------------------------------------------------------------------------------------------------------
// Names
// ---------------------------------------------------------------------------------------------------------------

/// Case-insensitive, letters and digits only: "Foo-Bar 2" == "foobar2".
pub fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// A normalised name may match something on disk only when it is long enough and not a generic word.
pub fn usable(n: &str) -> bool {
    n.chars().count() >= 3 && !GENERIC.contains(&n)
}

fn is_version(t: &str) -> bool {
    let t = t.trim_start_matches(['v', 'V']);
    t.starts_with(|c: char| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '_' | '+'))
}

/// "7-Zip 23.01 (x64)" -> "7-Zip": trailing version / architecture tokens and parenthesised tails dropped (never below one token).
pub fn short_name(s: &str) -> String {
    let s = s.split(" - ").next().unwrap_or(s);
    let mut t: Vec<&str> = s.split_whitespace().collect();
    while t.len() > 1 {
        let last = t[t.len() - 1];
        if last.ends_with(')') {
            let Some(i) = t.iter().rposition(|x| x.starts_with('(')) else { break };
            if i == 0 {
                break;
            }
            t.truncate(i);
        } else if is_version(last) || matches!(last.to_lowercase().as_str(), "x64" | "x86" | "64-bit" | "32-bit" | "64bit" | "32bit" | "amd64" | "beta") {
            t.pop();
        } else {
            break;
        }
    }
    t.join(" ")
}

/// "Google LLC" -> "google", "Igor Pavlov" -> "igorpavlov".
fn publisher_norm(s: &str) -> String {
    let t: Vec<&str> = s.split_whitespace().collect();
    let keep: Vec<&str> = t.iter().copied().filter(|w| !CORP.contains(&norm(w).as_str())).collect();
    norm(&if keep.is_empty() { t.join(" ") } else { keep.join(" ") })
}

/// Normalised names an app's leftovers may carry: product (name, short name) and publisher. Only `usable` ones.
#[derive(Clone, Debug, Default)]
pub struct Names {
    pub product: Vec<String>,
    pub publisher: Vec<String>,
}

pub fn names_of(p: &Prog) -> Names {
    let mut product: Vec<String> = [norm(&p.name), norm(&short_name(&p.name))].into_iter().filter(|n| usable(n)).collect();
    product.dedup();
    let publisher: Vec<String> = [publisher_norm(&p.publisher)].into_iter().filter(|n| usable(n) && !product.contains(n)).collect();
    Names { product, publisher }
}

// ---------------------------------------------------------------------------------------------------------------
// Installed programs
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    S(String),
    D(u32),
}
/// Values of one Uninstall key, names lower-cased.
pub type Vals = HashMap<String, Val>;

#[derive(Clone, Debug, Default)]
pub struct Prog {
    pub appx: bool,
    pub hklm: bool,
    /// Registry subkey (Reg) or package full name (Appx).
    pub sub: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub location: String,
    pub uninst: String,
    pub quiet: String,
    pub size_kb: u64,
    pub msi: Option<String>,
    pub sys: bool,
    pub parent: bool,
    pub update: bool,
    pub no_remove: bool,
}

fn vs(v: &Vals, k: &str) -> String {
    match v.get(k) {
        Some(Val::S(s)) => s.trim().to_string(),
        Some(Val::D(d)) => d.to_string(),
        None => String::new(),
    }
}
fn vd(v: &Vals, k: &str) -> u32 {
    match v.get(k) {
        Some(Val::D(d)) => *d,
        Some(Val::S(s)) => s.trim().parse().unwrap_or(0),
        None => 0,
    }
}

/// `{8-4-4-4-12}` hex GUID inside `s`, braces included.
pub fn find_guid(s: &str) -> Option<String> {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if b[i] == b'{' && i + 38 <= b.len() && b[i + 37] == b'}' && s.is_char_boundary(i + 38) {
            let g = &s[i..i + 38];
            let ok = g.char_indices().all(|(k, c)| match k {
                0 => c == '{',
                37 => c == '}',
                9 | 14 | 19 | 24 => c == '-',
                _ => c.is_ascii_hexdigit(),
            });
            if ok {
                return Some(g.to_string());
            }
        }
    }
    None
}

/// One Uninstall key -> Prog (None without a DisplayName).
pub fn parse_entry(hklm: bool, sub: &str, v: &Vals) -> Option<Prog> {
    let name = vs(v, "displayname");
    if name.is_empty() {
        return None;
    }
    let uninst = vs(v, "uninstallstring");
    let leaf = sub.rsplit('\\').next().unwrap_or(sub);
    let msi = if uninst.to_lowercase().contains("msiexec") {
        find_guid(&uninst)
    } else if vd(v, "windowsinstaller") == 1 && uninst.is_empty() {
        find_guid(leaf)
    } else {
        None
    };
    let rel = vs(v, "releasetype").to_lowercase();
    let lname = name.to_lowercase();
    Some(Prog {
        appx: false,
        hklm,
        sub: sub.to_string(),
        name,
        publisher: vs(v, "publisher"),
        version: vs(v, "displayversion"),
        location: vs(v, "installlocation").trim_matches('"').to_string(),
        uninst,
        quiet: vs(v, "quietuninstallstring"),
        size_kb: vd(v, "estimatedsize") as u64,
        msi,
        sys: vd(v, "systemcomponent") == 1,
        parent: !vs(v, "parentkeyname").is_empty() || !vs(v, "parentdisplayname").is_empty(),
        update: matches!(rel.as_str(), "update" | "hotfix" | "security update" | "service pack" | "update rollup" | "critical update")
            || lname.contains("(kb")
            || lname.starts_with("security update for")
            || lname.starts_with("update for")
            || lname.starts_with("hotfix for"),
        no_remove: vd(v, "noremove") == 1,
    })
}

fn is_risky(p: &Prog) -> bool {
    let ln = p.name.to_lowercase();
    RISKY_NAMES.iter().any(|k| ln.contains(k)) || (p.appx && (ln.starts_with("microsoft.windows") || ln.starts_with("microsoftwindows") || ln.starts_with("windows.")))
}

/// None = hidden from the list; Some(risky) = listed (runtimes / drivers only when asked for).
pub fn classify(p: &Prog, self_dir: Option<&Path>) -> Option<bool> {
    if p.sys || p.parent || p.update || p.no_remove {
        return None;
    }
    if !p.appx && p.uninst.is_empty() && p.quiet.is_empty() && p.msi.is_none() {
        return None;
    }
    let ln = p.name.to_lowercase();
    if ln == "pill" || ln.starts_with("pill ") {
        return None;
    }
    if let (Some(sd), Some((exe, _))) = (self_dir, split_cmd(&p.uninst)) {
        if depth_under(Path::new(&exe), sd).is_some() {
            return None; // Pill's own uninstaller
        }
    }
    Some(is_risky(p))
}

/// `"C:\x y\unins000.exe" /SILENT` or an unquoted `C:\x y\unins000.exe /SILENT` -> (file, args).
pub fn split_cmd(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(r) = s.strip_prefix('"') {
        let e = r.find('"')?;
        return Some((r[..e].to_string(), r[e + 1..].trim().to_string()));
    }
    // unquoted path with spaces: every ".exe" followed by a blank or the end may end the file name; an existing file wins (like CreateProcess)
    let low = s.to_lowercase();
    let (mut from, mut ends) = (0, Vec::new());
    while let Some(i) = low[from..].find(".exe") {
        let end = from + i + 4;
        if end == s.len() || low[end..].starts_with(char::is_whitespace) {
            ends.push(end);
        }
        from = end;
    }
    if let Some(&end) = ends.iter().find(|e| Path::new(&s[..**e]).is_file()).or(ends.first()) {
        return Some((s[..end].to_string(), s[end..].trim().to_string()));
    }
    match s.split_once(char::is_whitespace) {
        Some((f, a)) => Some((f.to_string(), a.trim().to_string())),
        None => Some((s.to_string(), String::new())),
    }
}

/// (file, args) for ShellExecute. MSI goes through `msiexec /x {GUID}`; `silent` = QuietUninstallString, or msiexec /passive.
pub fn build_cmd(p: &Prog, silent: bool) -> Result<(String, String), &'static str> {
    if let Some(g) = &p.msi {
        return Ok(("msiexec.exe".into(), format!("/x {g}{}", if silent { " /passive /norestart" } else { "" })));
    }
    let s = if silent { &p.quiet } else { &p.uninst };
    split_cmd(s).filter(|(f, _)| !f.is_empty()).ok_or("no uninstall command")
}

pub fn can_silent(p: &Prog) -> bool {
    p.msi.is_some() || !p.quiet.is_empty()
}

// ---------------------------------------------------------------------------------------------------------------
// The guard
// ---------------------------------------------------------------------------------------------------------------

/// Where things live. Built from the environment; tests build one inside a temp directory.
#[derive(Clone, Debug)]
pub struct Env {
    pub profile: PathBuf,
    pub windows: PathBuf,
    pub appdata: PathBuf,
    pub local: PathBuf,
    pub programdata: PathBuf,
    pub pf: PathBuf,
    pub pf86: PathBuf,
    pub desktop: PathBuf,
    pub public_desktop: PathBuf,
    /// ...\Start Menu\Programs of the user and of all users
    pub start_user: PathBuf,
    pub start_common: PathBuf,
    /// Pill's own folders: never touched, nothing inside them either.
    pub pill: Vec<PathBuf>,
}

impl Env {
    pub fn real() -> Env {
        let var = |k: &str, d: &str| PathBuf::from(std::env::var_os(k).filter(|v| !v.is_empty()).unwrap_or_else(|| d.into()));
        let profile = var("USERPROFILE", r"C:\Users\Default");
        let appdata = var("APPDATA", r"C:\Users\Default\AppData\Roaming");
        let local = var("LOCALAPPDATA", r"C:\Users\Default\AppData\Local");
        let programdata = var("ProgramData", r"C:\ProgramData");
        let mut pill = vec![local.join("Pill"), appdata.join("com.pill.dev"), local.join("com.pill.dev")];
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            pill.push(d);
        }
        let start = |b: &Path| b.join(r"Microsoft\Windows\Start Menu\Programs");
        Env {
            windows: var("SystemRoot", r"C:\Windows"),
            pf: var("ProgramFiles", r"C:\Program Files"),
            pf86: var("ProgramFiles(x86)", r"C:\Program Files (x86)"),
            desktop: profile.join("Desktop"),
            public_desktop: var("PUBLIC", r"C:\Users\Public").join("Desktop"),
            start_user: start(&appdata),
            start_common: start(&programdata),
            profile,
            appdata,
            local,
            programdata,
            pill,
        }
    }

    /// Never deletable, and nothing that contains one of them is either.
    fn protected(&self) -> Vec<PathBuf> {
        let mut v = vec![
            self.profile.clone(),
            self.windows.clone(),
            self.windows.join("System32"),
            self.appdata.clone(),
            self.local.clone(),
            self.programdata.clone(),
            self.pf.clone(),
            self.pf86.clone(),
            self.desktop.clone(),
            self.public_desktop.clone(),
            self.start_user.clone(),
            self.start_common.clone(),
            self.local.join("Programs"),
        ];
        for k in ["Documents", "Downloads", "Pictures", "Music", "Videos", "Favorites", "Links", "Contacts", "Saved Games", "OneDrive", "AppData"] {
            v.push(self.profile.join(k));
        }
        v.extend(self.pill.iter().cloned());
        v
    }

    /// Folders a leftover may live in (strictly below).
    fn parents(&self) -> Vec<PathBuf> {
        vec![
            self.appdata.clone(),
            self.local.clone(),
            self.local.join("Programs"),
            self.programdata.clone(),
            self.pf.clone(),
            self.pf86.clone(),
            self.start_user.clone(),
            self.start_common.clone(),
            self.desktop.clone(),
            self.public_desktop.clone(),
        ]
    }

    /// User folders where an app's recorded InstallLocation is not believed (a stray "InstallLocation = Documents").
    fn known_folders(&self) -> Vec<PathBuf> {
        ["Documents", "Downloads", "Pictures", "Music", "Videos", "Desktop", "OneDrive"].iter().map(|k| self.profile.join(k)).collect()
    }
}

fn comps(p: &Path) -> Vec<String> {
    p.components().map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect()
}

/// Number of components `child` has below `parent` (0 = same path), case-insensitive; None when it is not inside.
pub fn depth_under(child: &Path, parent: &Path) -> Option<usize> {
    let (c, p) = (comps(child), comps(parent));
    (c.len() >= p.len() && c[..p.len()] == p[..]).then(|| c.len() - p.len())
}

fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().into_owned();
    match s.strip_prefix(r"\\?\") {
        Some(r) if !r.starts_with("UNC") => PathBuf::from(r),
        _ => p,
    }
}

fn lexical_ok(p: &Path) -> Result<(), &'static str> {
    let s = p.to_string_lossy();
    if s.is_empty() || s.contains('\0') || !p.is_absolute() || s.starts_with(r"\\") {
        return Err("not a plain absolute path");
    }
    if p.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err("path has . or .. parts");
    }
    if p.components().count() <= 2 {
        return Err("drive root");
    }
    Ok(())
}

fn leaf_generic(p: &Path) -> bool {
    let n = norm(&p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    !usable(&n)
}

/// An InstallLocation recorded by an app is trusted as a parent only when it is a real app folder.
fn install_usable(i: &Path, env: &Env) -> bool {
    lexical_ok(i).is_ok()
        && !env.protected().iter().any(|f| depth_under(f, i).is_some())
        && depth_under(i, &env.windows).is_none()
        && !env.known_folders().iter().any(|k| depth_under(i, k).is_some())
        && !leaf_generic(i)
}

/// THE check every file / folder deletion goes through. Ok = `p` is a real, existing, non-link path strictly inside an allowed parent.
pub fn guard(p: &Path, env: &Env, install: Option<&Path>) -> Result<(), &'static str> {
    lexical_ok(p)?;
    if env.protected().iter().any(|f| depth_under(f, p).is_some()) {
        return Err("protected folder or a parent of one");
    }
    if depth_under(p, &env.windows).is_some() {
        return Err("inside Windows");
    }
    if env.pill.iter().any(|d| depth_under(p, d).is_some()) {
        return Err("Pill's own folder");
    }
    let install = install.filter(|i| install_usable(i, env));
    // the closest allowed parent; the first folder below it must carry a specific name (not Microsoft, Temp, Packages, ...)
    let parents = env.parents();
    let par: &Path = match parents.iter().filter(|r| depth_under(p, r).is_some_and(|d| d >= 1)).max_by_key(|r| r.components().count()) {
        Some(r) => {
            let first = p.components().nth(r.components().count()).map(|c| norm(&c.as_os_str().to_string_lossy())).unwrap_or_default();
            if !usable(&first) {
                return Err("generic folder name");
            }
            r
        }
        None => match install.filter(|i| depth_under(p, i).is_some()) {
            Some(i) if depth_under(p, i) != Some(0) || !leaf_generic(p) => i,
            _ => return Err("not inside an allowed folder"),
        },
    };
    // exists, and is what it says: canonical form must be the parent's canonical form plus the same tail (no junction / symlink anywhere below)
    let cp = std::fs::canonicalize(p).map_err(|_| "does not exist")?;
    let cr = std::fs::canonicalize(par).map_err(|_| "allowed folder is not readable")?;
    let tail: PathBuf = p.components().skip(par.components().count()).collect();
    if comps(&plain(cp)) != comps(&plain(cr.join(tail))) {
        return Err("link, junction or alternative name");
    }
    Ok(())
}

/// HKCU subkey (below HKCU) that may be deleted: `Software\<Name>` or `Software\<Publisher>\<Name>`, exact normalised matches only.
pub fn key_guard(sub: &str, n: &Names) -> Result<(), &'static str> {
    let parts: Vec<&str> = sub.split('\\').collect();
    if sub.contains("..") || !parts[0].eq_ignore_ascii_case("software") || !(2..=3).contains(&parts.len()) || parts.iter().any(|x| x.is_empty()) {
        return Err("not an app key");
    }
    if !n.product.contains(&norm(parts[parts.len() - 1])) {
        return Err("name does not match the app");
    }
    if parts.len() == 3 && !n.publisher.contains(&norm(parts[1])) {
        return Err("publisher does not match");
    }
    Ok(())
}

/// A Run value may be deleted when its command line starts inside the app's install folder.
pub fn run_guard(data: &str, install: Option<&Path>, env: &Env) -> Result<(), &'static str> {
    let i = install.filter(|i| install_usable(i, env)).ok_or("no usable install folder")?;
    let (f, _) = split_cmd(&expand_env(data)).ok_or("empty value")?;
    match depth_under(Path::new(&f), i) {
        Some(d) if d >= 1 && !f.contains("..") => Ok(()),
        _ => Err("does not start inside the install folder"),
    }
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => match std::env::var(&after[..j]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &after[j + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------------------------------------------
// Leftover scan
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum What {
    Fs(PathBuf),
    /// Subkey below HKCU
    Key(String),
    /// (Run key below HKCU, value name, value data)
    Run(String, String, String),
}

#[derive(Clone, Debug)]
pub struct Cand {
    pub kind: &'static str, // folder | shortcut | key | autorun
    pub shown: String,
    pub size: u64,
    pub ticked: bool,
    pub note: &'static str,
    pub what: What,
}

/// Caps a scan: entries looked at and wall time.
pub struct Budget {
    until: Instant,
    left: usize,
    pub cut: bool,
}
impl Budget {
    pub fn new(entries: usize, secs: u64) -> Budget {
        Budget { until: Instant::now() + Duration::from_secs(secs), left: entries, cut: false }
    }
    fn tick(&mut self) -> bool {
        if self.left == 0 || Instant::now() > self.until {
            self.cut = true;
            return false;
        }
        self.left -= 1;
        true
    }
}

/// Entries of one folder, links / junctions skipped. (name, path, is_dir)
fn read_dir(dir: &Path, b: &mut Budget) -> Vec<(String, PathBuf, bool)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = Vec::new();
    for e in rd.flatten() {
        if !b.tick() {
            break;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        out.push((e.file_name().to_string_lossy().into_owned(), e.path(), ft.is_dir()));
    }
    out
}

/// Bytes in a folder (links not followed), capped by the budget.
pub fn dir_size(p: &Path, b: &mut Budget) -> u64 {
    let Ok(md) = std::fs::symlink_metadata(p) else { return 0 };
    if !md.is_dir() {
        return md.len();
    }
    let (mut total, mut stack) = (0u64, vec![p.to_path_buf()]);
    while let Some(d) = stack.pop() {
        for (_, path, is_dir) in read_dir(&d, b) {
            if is_dir {
                stack.push(path);
            } else {
                total += std::fs::symlink_metadata(&path).map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

fn fs_cand(kind: &'static str, path: PathBuf, ticked: bool, note: &'static str, b: &mut Budget) -> Cand {
    Cand { kind, shown: path.to_string_lossy().into_owned(), size: dir_size(&path, b), ticked, note, what: What::Fs(path) }
}

/// Files and folders: the recorded install folder, name-equal folders in the usual data roots, shortcuts. `lnk` resolves a .lnk target.
/// `shared` = another installed app records a folder that contains / is contained in the install folder.
pub fn scan_fs(p: &Prog, env: &Env, shared: bool, lnk: &dyn Fn(&Path) -> Option<PathBuf>, b: &mut Budget) -> Vec<Cand> {
    let names = names_of(p);
    let install = (!p.location.is_empty()).then(|| PathBuf::from(p.location.trim_end_matches('\\')));
    let ins = install.as_deref();
    let mut out: Vec<Cand> = Vec::new();
    if let Some(i) = ins.filter(|i| i.is_dir() && guard(i, env, Some(i)).is_ok()) {
        out.push(fs_cand("folder", i.to_path_buf(), !shared, if shared { "shared with another installed app" } else { "install folder" }, b));
    }
    // folders named like the app (ticked) or like its publisher (unticked: the publisher's other apps use it)
    let roots = [env.appdata.clone(), env.local.clone(), env.local.join("Programs"), env.programdata.clone(), env.pf.clone(), env.pf86.clone()];
    for root in &roots {
        for (name, path, is_dir) in read_dir(root, b) {
            let n = norm(&name);
            if !is_dir || !usable(&n) {
                continue;
            }
            if names.product.contains(&n) {
                out.push(fs_cand("folder", path, true, "named like the app", b));
            } else if names.publisher.contains(&n) {
                for (n2, p2, d2) in read_dir(&path, b) {
                    if d2 && names.product.contains(&norm(&n2)) {
                        out.push(fs_cand("folder", p2, true, "named like the app, under the publisher", b));
                    }
                }
                out.push(fs_cand("folder", path, false, "publisher folder, other apps may use it", b));
            }
        }
    }
    // shortcuts: Desktop and Start Menu, one level of subfolders
    for root in [&env.desktop, &env.public_desktop, &env.start_user, &env.start_common] {
        for (name, path, is_dir) in read_dir(root, b) {
            if !is_dir {
                out.extend(shortcut(&name, path, &names, ins, env, lnk, b));
            } else if names.product.contains(&norm(&name)) {
                out.push(fs_cand("folder", path, true, "Start Menu folder named like the app", b));
            } else {
                for (n2, p2, d2) in read_dir(&path, b) {
                    if !d2 {
                        out.extend(shortcut(&n2, p2, &names, ins, env, lnk, b));
                    }
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|c| match &c.what {
        What::Fs(p) => seen.insert(comps(p)),
        _ => true,
    });
    // a candidate inside a ticked candidate is covered by it
    let all: Vec<PathBuf> = out.iter().filter_map(|c| if let (What::Fs(p), true) = (&c.what, c.ticked) { Some(p.clone()) } else { None }).collect();
    out.retain(|c| match &c.what {
        What::Fs(p) => !all.iter().any(|a| depth_under(p, a).is_some_and(|d| d >= 1)),
        _ => true,
    });
    out
}

fn shortcut(name: &str, path: PathBuf, names: &Names, install: Option<&Path>, env: &Env, lnk: &dyn Fn(&Path) -> Option<PathBuf>, b: &mut Budget) -> Option<Cand> {
    let low = name.to_lowercase();
    if !(low.ends_with(".lnk") || low.ends_with(".url")) {
        return None;
    }
    let stem = norm(&name[..name.len() - 4]);
    let by_name = names.product.contains(&stem);
    let by_target = install.filter(|i| install_usable(i, env)).is_some_and(|i| low.ends_with(".lnk") && lnk(&path).is_some_and(|t| depth_under(&t, i).is_some()));
    (by_name || by_target).then(|| fs_cand("shortcut", path, true, if by_target { "points into the install folder" } else { "named like the app" }, b))
}

/// HKCU\Software children matching the app. `children(sub)` lists the subkeys of a key below HKCU (a closure so tests can feed fixtures).
pub fn scan_keys(p: &Prog, children: &dyn Fn(&str) -> Vec<String>) -> Vec<Cand> {
    let names = names_of(p);
    let mut out = Vec::new();
    for c in children("Software") {
        let n = norm(&c);
        if !usable(&n) {
            continue;
        }
        let sub = format!(r"Software\{c}");
        if names.product.contains(&n) {
            out.push(Cand { kind: "key", shown: format!(r"HKCU\{sub}"), size: 0, ticked: true, note: "key named like the app", what: What::Key(sub) });
        } else if names.publisher.contains(&n) {
            for g in children(&sub) {
                if names.product.contains(&norm(&g)) {
                    let s2 = format!(r"{sub}\{g}");
                    out.push(Cand { kind: "key", shown: format!(r"HKCU\{s2}"), size: 0, ticked: true, note: "key under the publisher", what: What::Key(s2) });
                }
            }
        }
    }
    out
}

/// HKCU Run values that start inside the install folder.
pub fn scan_run(p: &Prog, values: &[(String, String)], env: &Env) -> Vec<Cand> {
    let install = (!p.location.is_empty()).then(|| PathBuf::from(p.location.trim_end_matches('\\')));
    values
        .iter()
        .filter(|(_, d)| run_guard(d, install.as_deref(), env).is_ok())
        .map(|(n, d)| Cand {
            kind: "autorun",
            shown: format!(r"HKCU\{RUN_KEY} : {n}"),
            size: 0,
            ticked: true,
            note: "starts the app at sign-in",
            what: What::Run(RUN_KEY.to_string(), n.clone(), d.clone()),
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------------------------
// Deleting: Recycle Bin for files, exported .reg for keys
// ---------------------------------------------------------------------------------------------------------------

/// Sends `p` to the Recycle Bin (never a plain delete) and checks it is gone.
pub fn recycle(p: &Path) -> Result<(), String> {
    let mut from: Vec<u16> = p.to_string_lossy().encode_utf16().collect();
    from.extend([0, 0]);
    // FO_DELETE; FOF_SILENT | FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_NOERRORUI
    let mut op = SHFILEOPSTRUCTW { wFunc: 3, pFrom: PCWSTR(from.as_ptr()), fFlags: (0x4 | 0x10 | 0x40 | 0x400) as u16, ..Default::default() };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.fAnyOperationsAborted.as_bool() {
        return Err(format!("Windows refused (code {r}); it may need administrator rights or a program may still use it"));
    }
    if p.exists() {
        return Err("still there after the delete".into());
    }
    Ok(())
}

fn sanitize(s: &str) -> String {
    let t: String = s.chars().map(|c| if c.is_alphanumeric() || c == '-' { c } else { '_' }).take(40).collect();
    if t.is_empty() {
        "app".into()
    } else {
        t
    }
}

/// Exports each HKCU subkey with reg.exe and merges the results into one .reg file; Ok only when the file exists and is not empty.
pub fn export_keys(subs: &[String], out: &Path) -> Result<(), String> {
    let mut merged: Vec<u16> = vec![0xFEFF];
    for (i, s) in subs.iter().enumerate() {
        let part = out.with_extension(format!("part{i}"));
        let st = std::process::Command::new("reg").args(["export", &format!(r"HKCU\{s}")]).arg(&part).arg("/y").creation_flags(NO_WINDOW).output();
        let bytes = std::fs::read(&part).unwrap_or_default();
        let _ = std::fs::remove_file(&part);
        if !st.is_ok_and(|o| o.status.success()) || bytes.len() < 4 {
            return Err(format!("could not export HKCU\\{s}"));
        }
        let text = String::from_utf16_lossy(&bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>());
        let text = text.trim_start_matches('\u{feff}');
        // every part starts with the "Windows Registry Editor" header line: keep it once
        let body = if i == 0 { text } else { text.split_once('\n').map_or("", |x| x.1) };
        merged.extend(body.encode_utf16());
        merged.extend("\r\n".encode_utf16());
    }
    let bytes: Vec<u8> = merged.iter().flat_map(|u| u.to_le_bytes()).collect();
    std::fs::write(out, &bytes).map_err(|e| e.to_string())?;
    if std::fs::metadata(out).map_or(0, |m| m.len()) < 8 {
        return Err("backup file is empty".into());
    }
    Ok(())
}

pub struct Report {
    pub done: Vec<String>,
    pub failed: Vec<(String, String)>,
    pub backup: Option<PathBuf>,
    pub log: PathBuf,
}

/// Deletes the chosen candidates. Every item is re-checked by the guards; registry items need their backup first.
pub fn delete_cands(p: &Prog, cands: &[&Cand], env: &Env, dir: &Path, stamp: &str) -> Report {
    let names = names_of(p);
    let install = (!p.location.is_empty()).then(|| PathBuf::from(p.location.trim_end_matches('\\')));
    let base = format!("{}-{stamp}", sanitize(&p.name));
    let _ = std::fs::create_dir_all(dir);
    let (log_path, reg_path) = (dir.join(format!("{base}.log")), dir.join(format!("{base}.reg")));
    let mut log = vec![format!("Pill uninstall cleanup: {} ({stamp})", p.name)];
    let (mut done, mut failed) = (Vec::new(), Vec::new());
    let mut subs: Vec<String> = Vec::new();
    for c in cands {
        match &c.what {
            What::Key(s) if key_guard(s, &names).is_ok() => subs.push(s.clone()),
            What::Run(k, _, d) if !subs.contains(k) && run_guard(d, install.as_deref(), env).is_ok() => subs.push(k.clone()),
            _ => {}
        }
    }
    let backup = if subs.is_empty() {
        None
    } else {
        match export_keys(&subs, &reg_path) {
            Ok(()) => {
                log.push(format!("backup: {}", reg_path.display()));
                Some(reg_path.clone())
            }
            Err(e) => {
                log.push(format!("backup failed, registry items skipped: {e}"));
                None
            }
        }
    };
    for c in cands {
        let r: Result<(), String> = match &c.what {
            What::Fs(path) => guard(path, env, install.as_deref()).map_err(String::from).and_then(|_| recycle(path)),
            What::Key(s) => match (key_guard(s, &names), &backup) {
                (Err(e), _) => Err(e.into()),
                (_, None) => Err("no backup, not deleted".into()),
                _ => {
                    let w: Vec<u16> = s.encode_utf16().chain(Some(0)).collect();
                    let e = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(w.as_ptr())) };
                    if e == OK { Ok(()) } else { Err(format!("registry error {}", e.0)) }
                }
            },
            What::Run(k, n, d) => match (run_guard(d, install.as_deref(), env), &backup) {
                (Err(e), _) => Err(e.into()),
                (_, None) => Err("no backup, not deleted".into()),
                _ => Key::open(HKEY_CURRENT_USER, k, KEY_SET_VALUE).ok_or_else(|| "cannot open the Run key".to_string()).and_then(|key| {
                    let w: Vec<u16> = n.encode_utf16().chain(Some(0)).collect();
                    let e = unsafe { RegDeleteValueW(key.0, PCWSTR(w.as_ptr())) };
                    if e == OK { Ok(()) } else { Err(format!("registry error {}", e.0)) }
                }),
            },
        };
        match r {
            Ok(()) => {
                log.push(format!("deleted {} {}", c.kind, c.shown));
                done.push(c.shown.clone());
            }
            Err(e) => {
                log.push(format!("NOT deleted {} {}: {e}", c.kind, c.shown));
                failed.push((c.shown.clone(), e));
            }
        }
    }
    let _ = std::fs::write(&log_path, log.join("\r\n") + "\r\n");
    Report { done, failed, backup, log: log_path }
}

// ---------------------------------------------------------------------------------------------------------------
// Registry access (reads; HKCU writes only in delete_cands)
// ---------------------------------------------------------------------------------------------------------------

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
impl Key {
    fn open(hive: HKEY, sub: &str, access: REG_SAM_FLAGS) -> Option<Key> {
        let w: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
        let mut h = HKEY::default();
        (unsafe { RegOpenKeyExW(hive, PCWSTR(w.as_ptr()), 0, access, &mut h) } == OK).then_some(Key(h))
    }
    fn subkeys(&self, cap: u32) -> Vec<String> {
        let mut out = Vec::new();
        for i in 0..cap {
            let mut buf = [0u16; 512];
            let mut n = buf.len() as u32;
            if unsafe { RegEnumKeyExW(self.0, i, PWSTR(buf.as_mut_ptr()), &mut n, None, PWSTR::null(), None, None) } != OK {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..n as usize]));
        }
        out
    }
    /// REG_SZ / REG_EXPAND_SZ / REG_DWORD values, names lower-cased. Others are skipped.
    fn values(&self) -> Vals {
        let mut out = Vals::new();
        for i in 0..1024u32 {
            let (mut name, mut data) = ([0u16; 512], vec![0u8; 8192]);
            let (mut nn, mut dn, mut ty) = (name.len() as u32, data.len() as u32, 0u32);
            let r = unsafe { RegEnumValueW(self.0, i, PWSTR(name.as_mut_ptr()), &mut nn, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut dn)) };
            if r == WIN32_ERROR(259) {
                break; // ERROR_NO_MORE_ITEMS
            }
            if r != OK {
                continue;
            }
            let k = String::from_utf16_lossy(&name[..nn as usize]).to_lowercase();
            match ty {
                1 | 2 => {
                    let u: Vec<u16> = data[..dn as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                    out.insert(k, Val::S(String::from_utf16_lossy(&u).trim_end_matches('\0').to_string()));
                }
                4 if dn >= 4 => {
                    out.insert(k, Val::D(u32::from_le_bytes([data[0], data[1], data[2], data[3]])));
                }
                _ => {}
            }
        }
        out
    }
}

fn read_programs() -> Vec<Prog> {
    let mut out: Vec<Prog> = Vec::new();
    for (hklm, hive) in [(true, HKEY_LOCAL_MACHINE), (false, HKEY_CURRENT_USER)] {
        for base in [UNINSTALL.to_string(), UNINSTALL.replace("SOFTWARE", r"SOFTWARE\WOW6432Node")] {
            let Some(k) = Key::open(hive, &base, KEY_READ) else { continue };
            for name in k.subkeys(4096) {
                let sub = format!(r"{base}\{name}");
                let Some(vals) = Key::open(hive, &sub, KEY_READ).map(|kk| kk.values()) else { continue };
                if let Some(p) = parse_entry(hklm, &sub, &vals) {
                    if !out.iter().any(|o| o.name == p.name && o.version == p.version && o.location == p.location) {
                        out.push(p);
                    }
                }
            }
        }
    }
    out
}

fn still_installed(p: &Prog) -> bool {
    if p.appx {
        return appx_list().is_none_or(|l| l.iter().any(|a| a.sub == p.sub));
    }
    Key::open(if p.hklm { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER }, &p.sub, KEY_READ).is_some()
}

// ---------------------------------------------------------------------------------------------------------------
// Store apps (current user) through PowerShell
// ---------------------------------------------------------------------------------------------------------------

/// Hidden powershell, stdout back as text; killed after `secs`.
fn ps(script: &str, secs: u64) -> Option<String> {
    use std::io::Read;
    let full = format!("[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    let mut c = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &full])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(NO_WINDOW)
        .spawn()
        .ok()?;
    let mut so = c.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.by_ref().take(4 << 20).read_to_end(&mut b);
        let _ = tx.send(b);
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(b) => {
            let _ = c.wait();
            Some(String::from_utf8_lossy(&b).into_owned())
        }
        Err(_) => {
            let _ = c.kill();
            None
        }
    }
}

/// Package full names are `Name_1.2.3.4_arch__publisherid`; anything else never reaches a command line.
pub fn valid_full_name(s: &str) -> bool {
    !s.is_empty() && s.len() < 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '~'))
}

pub fn parse_appx(json: &str) -> Vec<Prog> {
    let Ok(v) = serde_json::from_str::<Value>(json.trim().trim_start_matches('\u{feff}')) else { return vec![] };
    let arr = match v {
        Value::Array(a) => a,
        o @ Value::Object(_) => vec![o],
        _ => vec![],
    };
    let g = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    arr.iter()
        .filter_map(|o| {
            let full = g(o, "PackageFullName");
            let publisher = g(o, "Publisher");
            let cn = publisher.split(',').find_map(|x| x.trim().strip_prefix("CN=")).unwrap_or("").trim_matches('"').to_string();
            valid_full_name(&full).then(|| Prog { appx: true, sub: full, name: g(o, "Name"), publisher: cn, version: g(o, "Version"), ..Default::default() })
        })
        .filter(|p| !p.name.is_empty())
        .collect()
}

fn appx_list() -> Option<Vec<Prog>> {
    let s = "Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.NonRemovable -and $_.SignatureKind -ne 'System' -and $_.IsResourcePackage -ne $true } | Select-Object Name,PackageFullName,Publisher,Version | ConvertTo-Json -Compress";
    ps(s, 20).map(|o| parse_appx(&o))
}

// ---------------------------------------------------------------------------------------------------------------
// State, commands
// ---------------------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Flow {
    progs: Vec<Prog>,
    locs: Vec<(String, PathBuf)>, // every installed program's recorded install folder (for the "shared folder" check)
    sel: Option<Prog>,
    cands: Vec<Cand>,
    busy: bool,
    last: Value,
}
static FLOW: Mutex<Option<Flow>> = Mutex::new(None);
static APPX: Mutex<Option<(Instant, Vec<Prog>)>> = Mutex::new(None);
static CANCEL: AtomicBool = AtomicBool::new(false);

fn flow<R>(f: impl FnOnce(&mut Flow) -> R) -> R {
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

fn fmt_size(kb: u64) -> String {
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

fn emit(app: &AppHandle, v: Value) {
    flow(|f| f.last = v.clone());
    let _ = app.emit_to("bar", "uninstall:state", v);
}

fn items_json(cands: &[Cand]) -> Vec<Value> {
    cands.iter().enumerate().map(|(i, c)| json!({ "id": i, "kind": c.kind, "path": c.shown, "size": c.size, "ticked": c.ticked, "note": c.note })).collect()
}

/// Details for the confirm card; also selects the app for the following commands. `name` must match the row (the list may have moved on).
#[tauri::command]
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
fn run_and_wait(file: &str, args: &str) -> Result<bool, String> {
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

fn begin() -> Option<Prog> {
    CANCEL.store(false, Ordering::Relaxed);
    flow(|f| if f.busy { None } else { f.sel.clone().inspect(|_| f.busy = true) })
}

#[tauri::command]
pub fn uninstall_start(app: AppHandle, silent: bool) -> Value {
    let Some(p) = begin() else { return json!({ "ok": false, "error": "nothing selected or busy" }) };
    std::thread::spawn(move || {
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
                crate::command::show(&app);
            }
            Ok(waited) => settle_and_scan(&app, &p, waited),
        }
    });
    json!({ "ok": true })
}

/// After the uninstaller process ended: wait for the entry to vanish, then scan; or report that it is still installed.
fn settle_and_scan(app: &AppHandle, p: &Prog, waited: bool) {
    let t = Instant::now();
    while waited && still_installed(p) && t.elapsed() < SETTLE_MAX && !CANCEL.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_secs(1));
    }
    if still_installed(p) {
        flow(|f| f.busy = false);
        emit(app, json!({ "phase": "still", "name": p.name }));
    } else if p.appx {
        flow(|f| f.busy = false);
        emit(app, json!({ "phase": "done", "name": p.name, "done": 0, "failed": [] }));
    } else {
        emit(app, json!({ "phase": "scanning", "name": p.name }));
        let (cands, manual, cut) = scan_all(p);
        let phase = if cands.is_empty() && manual.is_empty() { "done" } else { "review" };
        let v = json!({ "phase": phase, "name": p.name, "items": items_json(&cands), "manual": manual, "cut": cut, "done": 0, "failed": [] });
        flow(|f| {
            f.cands = cands;
            f.busy = false;
        });
        emit(app, v);
    }
    crate::command::show(app);
}

/// "Check again": the user finished the uninstaller by hand (or it handed over to another process).
#[tauri::command]
pub fn uninstall_scan(app: AppHandle) -> Value {
    let Some(p) = begin() else { return json!({ "ok": false }) };
    std::thread::spawn(move || settle_and_scan(&app, &p, false));
    json!({ "ok": true })
}

#[tauri::command]
pub fn uninstall_delete(app: AppHandle, ids: Vec<usize>) -> Value {
    let Some((p, picked)) = flow(|f| {
        if f.busy {
            return None;
        }
        let p = f.sel.clone()?;
        let mut ids = ids.clone();
        ids.sort_unstable();
        ids.dedup();
        let picked: Vec<Cand> = ids.iter().filter_map(|i| f.cands.get(*i).cloned()).collect(); // unknown indexes are dropped
        f.busy = true;
        Some((p, picked))
    }) else {
        return json!({ "ok": false });
    };
    std::thread::spawn(move || {
        emit(&app, json!({ "phase": "deleting", "name": p.name }));
        let env = Env::real();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
        let refs: Vec<&Cand> = picked.iter().collect();
        let r = delete_cands(&p, &refs, &env, &env.local.join(r"Pill\uninstall-backups"), &stamp);
        flow(|f| {
            f.busy = false;
            f.cands.clear();
        });
        let failed: Vec<String> = r.failed.iter().map(|(a, b)| format!("{a}: {b}")).collect();
        emit(&app, json!({ "phase": "done", "name": p.name, "done": r.done.len(), "failed": failed,
            "backup": r.backup.map(|b| b.to_string_lossy().into_owned()), "log": r.log.to_string_lossy() }));
    });
    json!({ "ok": true })
}

/// Stops waiting for the uninstaller (it keeps running) and forgets the selection.
#[tauri::command]
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
#[tauri::command]
pub fn uninstall_state() -> Value {
    flow(|f| f.last.clone())
}

// ---------------------------------------------------------------------------------------------------------------
// Real scan: ties the pure pieces to the disk and the registry
// ---------------------------------------------------------------------------------------------------------------

fn lnk_target(p: &Path) -> Option<PathBuf> {
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

/// (candidates, read-only "remove manually" lines, truncated)
fn scan_all(p: &Prog) -> (Vec<Cand>, Vec<String>, bool) {
    let env = Env::real();
    let mut b = Budget::new(20_000, 12);
    let install = (!p.location.is_empty()).then(|| PathBuf::from(p.location.trim_end_matches('\\')));
    let shared = install.as_ref().is_some_and(|i| flow(|f| f.locs.iter().any(|(k, l)| *k != p.sub && (depth_under(l, i).is_some() || depth_under(i, l).is_some()))));
    let mut cands = scan_fs(p, &env, shared, &lnk_target, &mut b);
    let children = |sub: &str| Key::open(HKEY_CURRENT_USER, sub, KEY_READ).map(|k| k.subkeys(3000)).unwrap_or_default();
    cands.extend(scan_keys(p, &children));
    let run: Vec<(String, String)> = Key::open(HKEY_CURRENT_USER, RUN_KEY, KEY_READ)
        .map(|k| k.values().into_iter().filter_map(|(n, v)| if let Val::S(s) = v { Some((n, s)) } else { None }).collect())
        .unwrap_or_default();
    cands.extend(scan_run(p, &run, &env));
    (cands, scan_manual(p, install.as_deref(), &env), b.cut)
}

/// Things that need administrator rights: only listed, never touched.
fn scan_manual(p: &Prog, install: Option<&Path>, env: &Env) -> Vec<String> {
    let names = names_of(p);
    let mut out = Vec::new();
    for base in ["SOFTWARE", r"SOFTWARE\WOW6432Node"] {
        let Some(k) = Key::open(HKEY_LOCAL_MACHINE, base, KEY_READ) else { continue };
        for c in k.subkeys(3000) {
            let n = norm(&c);
            if !usable(&n) {
                continue;
            }
            if names.product.contains(&n) {
                out.push(format!(r"HKLM\{base}\{c}"));
            } else if names.publisher.contains(&n) {
                let sub = format!(r"{base}\{c}");
                if let Some(kk) = Key::open(HKEY_LOCAL_MACHINE, &sub, KEY_READ) {
                    out.extend(kk.subkeys(500).into_iter().filter(|g| names.product.contains(&norm(g))).map(|g| format!(r"HKLM\{sub}\{g}")));
                }
            }
        }
    }
    if let Some(i) = install.filter(|i| install_usable(i, env)) {
        let needle = i.to_string_lossy().to_lowercase();
        if let Some(k) = Key::open(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Services", KEY_READ) {
            for s in k.subkeys(4000) {
                let img = Key::open(HKEY_LOCAL_MACHINE, &format!(r"SYSTEM\CurrentControlSet\Services\{s}"), KEY_READ).map(|kk| kk.values()).and_then(|v| v.get("imagepath").cloned());
                if matches!(img, Some(Val::S(x)) if x.to_lowercase().contains(&needle)) {
                    out.push(format!("Service: {s}"));
                }
            }
        }
        if let Some(k) = Key::open(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run", KEY_READ) {
            out.extend(k.values().into_iter().filter(|(_, v)| matches!(v, Val::S(s) if expand_env(s).to_lowercase().contains(&needle))).map(|(n, _)| format!("HKLM Run: {n}")));
        }
    }
    let mut b = Budget::new(3000, 3);
    for (name, _, _) in read_dir(&env.windows.join(r"System32\Tasks"), &mut b) {
        if names.product.contains(&norm(&name)) {
            out.push(format!("Scheduled task: {name}"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
