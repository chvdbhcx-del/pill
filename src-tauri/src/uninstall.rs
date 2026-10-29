// Uninstall apps completely from the command bar (`u foo`, `del foo`, `uninstall foo`, `удалить foo`): the app's own uninstaller first,
// then a DEEP CLEAN scan the user ticks through. Switch: Settings > Modules, `module.uninstall` (default on).
// Safety model: nothing is removed without the confirm card and the checklist; every item carries evidence and a tier (Certain = pre-ticked,
// Probable = unticked); leftovers go to the Recycle Bin (never a plain delete from here), registry keys / values are backed up to a .reg first.
// Everything that needs administrator rights (HKLM, services, tasks, firewall rules, Program Files, other profiles) is deleted only by an
// elevated copy of Pill (admin.rs) that re-derives the list itself. Every deletion goes through `guard_cand` (deep.rs); the page only ever sends
// indexes into the server-side list of the last scan.
// Commands: uninstall_info uninstall_start uninstall_scan uninstall_delete uninstall_admin uninstall_cancel uninstall_state uninstall_open_backups.
// Event (window bar): uninstall:state {phase: running|still|scanning|review|deleting|elevating|done|error, name, message, items[], manual[], ...}.
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

mod admin;
mod deep;
pub use admin::*;
pub use deep::*;

const NO_WINDOW: u32 = 0x0800_0000;
const OK: WIN32_ERROR = WIN32_ERROR(0);
const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const WAIT_MAX: Duration = Duration::from_secs(45 * 60); // the uninstaller's own window may sit there; it is never killed
const SETTLE_MAX: Duration = Duration::from_secs(90); // after the process exits: some uninstallers hand over to a temp copy and exit at once

/// Folder / key names that never count as "the app's own" (compared normalised, exact equality).
const GENERIC: &[&str] = &[
    "app", "apps", "data", "tools", "common", "microsoft", "windows", "system", "program", "programs", "update", "updates", "install", "setup", "cache", "temp",
    "logs", "software", "files", "local", "roaming", "user", "users", "default", "config", "settings", "shared", "bin", "lib", "plugins", "resources", "assets",
    "commonfiles", "windowsapps", "windowsnt", "packages", "microsoftshared", "startup", "accessories", "classes", "policies", "wow6432node",
    "modifiablewindowsapps", "windowsdefender", "windowsmail", "windowsmediaplayer", "windowsphotoviewer", "windowsportabledevices", "windowssecurity",
    "windowspowershell", "internetexplorer", "dotnet", "referenceassemblies", "msbuild", "uninstallinformation", "microsoftupdatehealthtools",
    "locallow", "documents", "downloads", "desktop", "pictures", "music", "videos", "onedrive", "appdata", "clsid", "interface", "typelib", "appid", "installer",
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
    /// Whole-word sequences of the product name (lower case) for the Probable tier; only distinctive ones (4+ letters in all).
    pub seqs: Vec<Vec<String>>,
}

pub fn names_of(p: &Prog) -> Names {
    let mut product: Vec<String> = [norm(&p.name), norm(&short_name(&p.name))].into_iter().filter(|n| usable(n)).collect();
    product.dedup();
    let publisher: Vec<String> = [publisher_norm(&p.publisher)].into_iter().filter(|n| usable(n) && !product.contains(n)).collect();
    let mut seqs: Vec<Vec<String>> = Vec::new();
    for n in [p.name.as_str(), &short_name(&p.name)] {
        let t = tokens(n);
        let j = t.concat();
        if usable(&j) && j.chars().count() >= 4 && !seqs.contains(&t) {
            seqs.push(t);
        }
    }
    Names { product, publisher, seqs }
}

// ---------------------------------------------------------------------------------------------------------------
// Installed programs
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    S(String),
    /// REG_EXPAND_SZ
    E(String),
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
    /// More product GUIDs than the key name / command lines show (only set from an elevated request).
    pub extra_guids: Vec<String>,
}

fn vs(v: &Vals, k: &str) -> String {
    match v.get(k) {
        Some(Val::S(s)) | Some(Val::E(s)) => s.trim().to_string(),
        Some(Val::D(d)) => d.to_string(),
        None => String::new(),
    }
}
fn vd(v: &Vals, k: &str) -> u32 {
    match v.get(k) {
        Some(Val::D(d)) => *d,
        Some(Val::S(s)) | Some(Val::E(s)) => s.trim().parse().unwrap_or(0),
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
        extra_guids: Vec::new(),
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
    /// Roots of the fixed local drives (`C:\`, `D:\`).
    pub drives: Vec<PathBuf>,
    /// Other users' profile folders that could be listed.
    pub others: Vec<PathBuf>,
}

/// The places of one user profile a deep clean looks into.
#[derive(Clone, Debug)]
pub struct Pd {
    pub root: PathBuf,
    pub roaming: PathBuf,
    pub local: PathBuf,
    pub locallow: PathBuf,
    pub docs: PathBuf,
    pub saved: PathBuf,
    pub desktop: PathBuf,
    pub start: PathBuf,
    pub downloads: PathBuf,
    pub temp: PathBuf,
    pub quick: PathBuf,
}

fn pd_of(root: &Path, roaming: PathBuf, local: PathBuf, desktop: PathBuf, start: PathBuf) -> Pd {
    Pd {
        root: root.to_path_buf(),
        locallow: local.parent().map_or_else(|| root.join(r"AppData\LocalLow"), |a| a.join("LocalLow")),
        docs: root.join("Documents"),
        saved: root.join("Saved Games"),
        downloads: root.join("Downloads"),
        temp: local.join("Temp"),
        quick: roaming.join(r"Microsoft\Internet Explorer\Quick Launch"),
        roaming,
        local,
        desktop,
        start,
    }
}

/// Folder names below `C:\Users` that are not a person's profile.
const NOT_PROFILES: &[&str] = &["public", "default", "default user", "all users", "defaultuser0", "desktop.ini"];

impl Env {
    pub fn real() -> Env {
        let var = |k: &str, d: &str| PathBuf::from(std::env::var_os(k).filter(|v| !v.is_empty()).unwrap_or_else(|| d.into()));
        let profile = var("USERPROFILE", r"C:\Users\Default");
        let appdata = var("APPDATA", r"C:\Users\Default\AppData\Roaming");
        let local = var("LOCALAPPDATA", r"C:\Users\Default\AppData\Local");
        let programdata = var("ProgramData", r"C:\ProgramData");
        let mut pill = vec![local.join("Pill"), appdata.join("com.pill.dev"), local.join("com.pill.dev"), programdata.join("Pill")];
        if let Some(d) = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)) {
            pill.push(d);
        }
        let start = |b: &Path| b.join(r"Microsoft\Windows\Start Menu\Programs");
        let others = profile
            .parent()
            .and_then(|users| std::fs::read_dir(users).ok())
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()))
                    .filter(|e| !NOT_PROFILES.contains(&e.file_name().to_string_lossy().to_lowercase().as_str()))
                    .map(|e| e.path())
                    .filter(|p| depth_under(p, &profile).is_none())
                    .collect()
            })
            .unwrap_or_default();
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
            drives: fixed_drives(),
            others,
        }
    }

    /// The current user's profile first (flag false), then every other one found (flag true).
    pub fn profiles(&self) -> Vec<(Pd, bool)> {
        let mut v = vec![(pd_of(&self.profile, self.appdata.clone(), self.local.clone(), self.desktop.clone(), self.start_user.clone()), false)];
        for o in &self.others {
            let (r, l) = (o.join(r"AppData\Roaming"), o.join(r"AppData\Local"));
            v.push((pd_of(o, r.clone(), l, o.join("Desktop"), r.join(r"Microsoft\Windows\Start Menu\Programs")), true));
        }
        v
    }

    /// Never deletable, and nothing that contains one of them is either.
    fn protected(&self) -> Vec<PathBuf> {
        let mut v = vec![
            self.windows.clone(),
            self.windows.join("System32"),
            self.programdata.clone(),
            self.pf.clone(),
            self.pf86.clone(),
            self.pf.join("Common Files"),
            self.pf86.join("Common Files"),
            self.public_desktop.clone(),
            self.start_common.clone(),
            self.windows.join("Temp"),
        ];
        v.extend(self.drives.iter().cloned());
        for (pd, _) in self.profiles() {
            v.extend([pd.root.clone(), pd.roaming.clone(), pd.local.clone(), pd.local.join("Programs"), pd.locallow.clone(), pd.desktop.clone(), pd.start.clone(), pd.docs.clone(), pd.saved.clone(), pd.downloads.clone(), pd.temp.clone(), pd.quick.clone()]);
            for k in ["Pictures", "Music", "Videos", "Favorites", "Links", "Contacts", "OneDrive", "AppData"] {
                v.push(pd.root.join(k));
            }
        }
        v.extend(self.pill.iter().cloned());
        v
    }

    /// Folders a leftover may live in (strictly below, first folder below must carry a specific name).
    fn parents(&self) -> Vec<PathBuf> {
        let mut v = vec![self.programdata.clone(), self.pf.clone(), self.pf86.clone(), self.start_common.clone(), self.public_desktop.clone()];
        for (pd, _) in self.profiles() {
            v.extend([pd.roaming.clone(), pd.local.clone(), pd.local.join("Programs"), pd.locallow.clone(), pd.start.clone(), pd.desktop.clone(), pd.quick.clone()]);
        }
        v
    }

    /// User folders where an app's recorded InstallLocation is not believed (a stray "InstallLocation = Documents").
    fn known_folders(&self) -> Vec<PathBuf> {
        ["Documents", "Downloads", "Pictures", "Music", "Videos", "Desktop", "OneDrive"].iter().map(|k| self.profile.join(k)).collect()
    }
}

/// Roots of the fixed (not removable, not network) drives.
fn fixed_drives() -> Vec<PathBuf> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    let mask = unsafe { GetLogicalDrives() };
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:\\", (b'A' + i as u8) as char))
        .filter(|root| {
            let w: Vec<u16> = root.encode_utf16().chain(Some(0)).collect();
            unsafe { GetDriveTypeW(PCWSTR(w.as_ptr())) == 3 } // DRIVE_FIXED
        })
        .map(PathBuf::from)
        .collect()
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

/// The strict structural check: `p` is a real, existing, non-link path strictly inside an allowed parent (or the recorded install folder).
/// Deletions use `guard_deep` / `guard_cand` (deep.rs), which add the wider places and the evidence rules on top of this.
#[cfg(test)]
pub fn guard(p: &Path, env: &Env, install: Option<&Path>) -> Result<(), &'static str> {
    deep::guard_core(p, env, install, None)
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    /// Pre-ticked: points into the install folder, carries the product's GUID, or its name equals the app's.
    Certain,
    /// Unticked: the app's full name appears as whole words, or a shared publisher folder.
    Probable,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hive {
    Hkcu,
    Hklm,
}
impl Hive {
    pub fn root(self) -> HKEY {
        match self {
            Hive::Hkcu => HKEY_CURRENT_USER,
            Hive::Hklm => HKEY_LOCAL_MACHINE,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Hive::Hkcu => "HKCU",
            Hive::Hklm => "HKLM",
        }
    }
    fn long(self) -> &'static str {
        match self {
            Hive::Hkcu => "HKEY_CURRENT_USER",
            Hive::Hklm => "HKEY_LOCAL_MACHINE",
        }
    }
}

#[derive(Clone, Debug)]
pub enum What {
    Fs(PathBuf),
    /// Registry key (hive, subkey) and everything below it.
    Key(Hive, String),
    /// Registry value (hive, key, value name).
    Value(Hive, String, String),
    /// Win32 service (own-process / shared-process only; drivers are only listed).
    Service(String),
    /// Scheduled task, as `\Folder\Name`.
    Task(String),
    /// An item the elevated scan found; it can only be deleted by the elevated process, selected by this key.
    Remote(String),
}

#[derive(Clone, Debug)]
pub struct Cand {
    pub kind: &'static str, // folder | file | shortcut | key | value | autorun | service | task | item
    pub shown: String,
    pub size: u64,
    pub tier: Tier,
    pub ticked: bool,
    /// Why it matched.
    pub note: String,
    /// Only an elevated process may delete it.
    pub admin: bool,
    pub what: What,
}

/// Stable identity of an item, the same in the unelevated and the elevated process.
pub fn cand_key(w: &What) -> String {
    match w {
        What::Fs(p) => format!("fs:{}", p.to_string_lossy().to_lowercase()),
        What::Key(h, s) => format!("key:{}\\{}", h.label(), s.to_lowercase()),
        What::Value(h, s, n) => format!("val:{}\\{}|{}", h.label(), s.to_lowercase(), n.to_lowercase()),
        What::Service(n) => format!("svc:{}", n.to_lowercase()),
        What::Task(t) => format!("task:{}", t.to_lowercase()),
        What::Remote(k) => k.clone(),
    }
}

const RUN_LISTS: &[&str] = &[r"microsoft\windows\currentversion\run", r"microsoft\windows\currentversion\runonce"];

impl Cand {
    /// `kind`, the shown path and `admin` follow from the item; Certain items start ticked.
    pub fn new(what: What, tier: Tier, note: &str, size: u64) -> Cand {
        let (kind, shown, admin) = match &what {
            What::Fs(p) => {
                let low = p.to_string_lossy().to_lowercase();
                (if low.ends_with(".lnk") || low.ends_with(".url") { "shortcut" } else if p.is_dir() { "folder" } else { "file" }, p.to_string_lossy().into_owned(), false)
            }
            What::Key(h, s) => ("key", format!("{}\\{s}", h.label()), *h == Hive::Hklm),
            What::Value(h, s, n) => {
                let run = RUN_LISTS.iter().any(|l| s.to_lowercase().ends_with(l));
                (if run { "autorun" } else { "value" }, format!("{}\\{s} : {n}", h.label()), *h == Hive::Hklm)
            }
            What::Service(n) => ("service", format!("Service: {n}"), true),
            What::Task(t) => ("task", format!("Scheduled task: {t}"), true),
            What::Remote(k) => ("item", k.clone(), true),
        };
        Cand { kind, shown, size, tier, ticked: tier == Tier::Certain, note: note.to_string(), admin, what }
    }
}

/// Needs an elevated process to delete: Program Files, ProgramData, Windows, the all-users Start Menu / Desktop, other people's profiles.
fn fs_admin(p: &Path, env: &Env) -> bool {
    [&env.pf, &env.pf86, &env.programdata, &env.windows, &env.public_desktop, &env.start_common].iter().any(|r| depth_under(p, r).is_some())
        || env.others.iter().any(|o| depth_under(p, o).is_some())
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

fn fs_cand(path: PathBuf, tier: Tier, note: &str, env: &Env, b: &mut Budget) -> Cand {
    let size = dir_size(&path, b);
    let admin = fs_admin(&path, env);
    let mut c = Cand::new(What::Fs(path), tier, note, size);
    c.admin = admin;
    c
}

struct Root {
    dir: PathBuf,
    /// installer files are matched here too (Temp, Downloads)
    files: bool,
    /// everything found here is Probable (Downloads)
    probable_only: bool,
}

/// Files and folders: the recorded install folder, name-matching folders in every profile and the shared roots, the top levels of the fixed drives,
/// shortcuts, emptied parent folders. `lnk` resolves a .lnk target. `shared` = another installed app records a folder that contains / is
/// contained in the install folder.
pub fn scan_fs(p: &Prog, env: &Env, shared: bool, lnk: &dyn Fn(&Path) -> Option<PathBuf>, b: &mut Budget) -> Vec<Cand> {
    let names = names_of(p);
    let install = p.install();
    let ins = install.as_deref();
    let mut out: Vec<Cand> = Vec::new();
    if let Some(i) = ins.filter(|i| i.is_dir() && guard_deep(i, env, p).is_ok()) {
        out.push(if shared { fs_cand(i.to_path_buf(), Tier::Probable, "shared with another installed app", env, b) } else { fs_cand(i.to_path_buf(), Tier::Certain, "install folder", env, b) });
    }
    let mut roots: Vec<Root> = Vec::new();
    for (pd, _) in env.profiles() {
        for d in [pd.roaming.clone(), pd.local.clone(), pd.local.join("Programs"), pd.locallow.clone()] {
            roots.push(Root { dir: d, files: false, probable_only: false });
        }
        // places for the user's own files (documents, save games): a name match is only ever Probable, never pre-ticked
        for d in [pd.docs.clone(), pd.saved.clone()] {
            roots.push(Root { dir: d, files: false, probable_only: true });
        }
        roots.push(Root { dir: pd.temp.clone(), files: true, probable_only: false });
        roots.push(Root { dir: pd.downloads.clone(), files: true, probable_only: true });
    }
    for d in [env.programdata.clone(), env.pf.clone(), env.pf86.clone(), env.pf.join("Common Files"), env.pf86.join("Common Files")] {
        roots.push(Root { dir: d, files: false, probable_only: false });
    }
    roots.push(Root { dir: env.windows.join("Temp"), files: true, probable_only: false });
    for root in &roots {
        for (name, path, is_dir) in read_dir(&root.dir, b) {
            if !is_dir {
                if root.files && installer_name(&name, &names) {
                    out.push(fs_cand(path, Tier::Probable, "installer file named like the app", env, b));
                }
                continue;
            }
            let Some((t, note)) = name_tier(&name, &names) else { continue };
            if note != N_PUB {
                let note = if root.probable_only && t == Tier::Certain { "named like the app, in a place for your own files" } else { note };
                out.push(fs_cand(path, if root.probable_only { Tier::Probable } else { t }, note, env, b));
                continue;
            }
            // a folder named like the publisher: the app's own subfolders are Certain; the folder itself only when nothing else is in it
            let kids = read_dir(&path, b);
            let is_app = |n: &str| matches!(name_tier(n, &names), Some((_, x)) if x != N_PUB);
            for (n2, p2, d2) in &kids {
                if *d2 && is_app(n2) {
                    let t2 = name_tier(n2, &names).map_or(Tier::Probable, |x| x.0);
                    out.push(fs_cand(p2.clone(), if root.probable_only { Tier::Probable } else { t2 }, if t2 == Tier::Certain { "named like the app, under the publisher" } else { N_SEQ }, env, b));
                }
            }
            let sole = !kids.is_empty() && kids.iter().all(|(n2, _, d2)| *d2 && is_app(n2));
            out.push(if sole && !root.probable_only {
                fs_cand(path, Tier::Certain, "publisher folder holding only this app", env, b)
            } else {
                fs_cand(path, Tier::Probable, "publisher folder, other apps may use it", env, b)
            });
        }
    }
    // the top level of every fixed drive and one folder below it (D:\Games\<App>, C:\<App>)
    for d in &env.drives {
        for (n1, p1, d1) in read_dir(d, b) {
            if !d1 || DRIVE_PROTECTED.contains(&norm(&n1).as_str()) {
                continue;
            }
            if let Some((t, note)) = name_tier(&n1, &names) {
                out.push(fs_cand(p1, if note == N_PUB { Tier::Probable } else { t }, if note == N_PUB { "publisher folder, other apps may use it" } else { note }, env, b));
                continue;
            }
            for (n2, p2, d2) in read_dir(&p1, b) {
                if let (true, Some((t, note))) = (d2, name_tier(&n2, &names).filter(|x| x.1 != N_PUB)) {
                    out.push(fs_cand(p2, t, note, env, b));
                }
            }
        }
    }
    // folders that held the install folder and are empty now
    if let Some(i) = ins.filter(|i| install_usable(i, env)) {
        for anc in i.ancestors().skip(1).take(2) {
            if anc.is_dir() && is_empty_dir(anc) && guard_deep(anc, env, p).is_ok() {
                out.push(fs_cand(anc.to_path_buf(), Tier::Probable, "emptied folder that held the app", env, b));
            }
        }
    }
    // shortcuts: Desktop, Start Menu, Quick Launch / taskbar pins, up to three folders deep
    let mut lroots: Vec<(PathBuf, bool)> = vec![(env.public_desktop.clone(), false), (env.start_common.clone(), true)];
    for (pd, _) in env.profiles() {
        lroots.extend([(pd.desktop.clone(), false), (pd.start.clone(), true), (pd.quick.clone(), false)]);
    }
    for (root, start_menu) in lroots {
        let mut stack = vec![(root, 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            for (name, path, is_dir) in read_dir(&dir, b) {
                if !is_dir {
                    out.extend(shortcut(&name, path, &names, ins, env, lnk, b));
                } else if start_menu && name_tier(&name, &names).is_some_and(|(t, n)| t == Tier::Certain && n == N_NAME) {
                    out.push(fs_cand(path, Tier::Certain, "Start Menu folder named like the app", env, b));
                } else if depth < 3 {
                    stack.push((path, depth + 1));
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
    let by_target = install.filter(|i| install_usable(i, env)).is_some_and(|i| low.ends_with(".lnk") && lnk(&path).is_some_and(|t| depth_under(&t, i).is_some()));
    if by_target {
        return Some(fs_cand(path, Tier::Certain, "points into the install folder", env, b));
    }
    match name_tier(&name[..name.len() - 4], names) {
        Some((t, n)) if n != N_PUB => Some(fs_cand(path, t, if t == Tier::Certain { "shortcut named like the app" } else { N_SEQ }, env, b)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Deleting: Recycle Bin for files, a .reg backup before any registry item, exact commands for services and tasks
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

fn reg_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

fn hex_bytes(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect::<Vec<_>>().join(",")
}

/// One value in .reg syntax.
fn reg_line(name: &str, ty: u32, data: &[u8]) -> String {
    let n = if name.is_empty() { "@".to_string() } else { format!("\"{}\"", reg_escape(name)) };
    let v = match ty {
        1 if data.len() % 2 == 0 => {
            let u: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            format!("\"{}\"", reg_escape(String::from_utf16_lossy(&u).trim_end_matches('\0')))
        }
        4 if data.len() == 4 => format!("dword:{:08x}", u32::from_le_bytes([data[0], data[1], data[2], data[3]])),
        3 => format!("hex:{}", hex_bytes(data)),
        t => format!("hex({t:x}):{}", hex_bytes(data)),
    };
    format!("{n}={v}\r\n")
}

/// `.reg` text of keys (with everything below them) and single values; imports back with `reg import`. Err when something cannot be read.
pub fn reg_dump(items: &[(Hive, String, Option<String>)]) -> Result<String, String> {
    fn key(out: &mut String, h: Hive, sub: &str, only: Option<&str>, depth: usize, budget: &mut usize) -> Result<(), String> {
        let k = Key::open(h.root(), sub, KEY_READ).ok_or_else(|| format!("cannot read {}\\{sub}", h.label()))?;
        out.push_str(&format!("[{}\\{sub}]\r\n", h.long()));
        for (n, ty, data) in k.raw_values() {
            if only.is_none_or(|o| o.eq_ignore_ascii_case(&n)) {
                out.push_str(&reg_line(&n, ty, &data));
            }
        }
        out.push_str("\r\n");
        if only.is_none() && depth < 16 {
            for c in k.subkeys(20_000) {
                *budget = budget.checked_sub(1).ok_or("key too large to back up")?;
                key(out, h, &format!(r"{sub}\{c}"), None, depth + 1, budget)?;
            }
        }
        Ok(())
    }
    let mut out = String::from("Windows Registry Editor Version 5.00\r\n\r\n");
    let mut budget = 50_000usize;
    for (h, sub, only) in items {
        key(&mut out, *h, sub, only.as_deref(), 0, &mut budget)?;
    }
    Ok(out)
}

/// The text as a UTF-16LE file with BOM, the format regedit writes.
pub fn reg_bytes(text: &str) -> Vec<u8> {
    std::iter::once(0xFEFFu16).chain(text.encode_utf16()).flat_map(u16::to_le_bytes).collect()
}

/// Where backups of the current user go.
pub fn backup_dir(env: &Env) -> PathBuf {
    env.local.join(r"Pill\uninstall-backups")
}

/// Writes `<dir>\<base>.<name>` and checks it is there and not empty. Returns the path.
pub fn file_store(dir: &Path, base: &str, name: &str, bytes: &[u8]) -> Result<String, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{base}.{name}"));
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    if std::fs::metadata(&path).map_or(0, |m| m.len()) == 0 {
        return Err("backup file is empty".into());
    }
    Ok(path.to_string_lossy().into_owned())
}

pub struct Report {
    pub done: Vec<(&'static str, String)>,
    pub failed: Vec<(String, String)>,
    pub backup: Option<String>,
    pub log: Option<String>,
    /// Items left alone because they need administrator rights.
    pub need_admin: usize,
}

fn run_quiet(exe: &str, args: &[&str]) -> Result<(), String> {
    let o = std::process::Command::new(exe).args(args).creation_flags(NO_WINDOW).output().map_err(|e| e.to_string())?;
    if o.status.success() { Ok(()) } else { Err(format!("{exe} failed ({})", o.status.code().unwrap_or(-1))) }
}

fn remove(w: &What) -> Result<(), String> {
    match w {
        What::Fs(p) => recycle(p),
        What::Key(h, sub) => {
            let wide: Vec<u16> = sub.encode_utf16().chain(Some(0)).collect();
            let e = unsafe { RegDeleteTreeW(h.root(), PCWSTR(wide.as_ptr())) };
            if e == OK { Ok(()) } else { Err(format!("registry error {}", e.0)) }
        }
        What::Value(h, sub, name) => {
            let key = Key::open(h.root(), sub, KEY_SET_VALUE).ok_or_else(|| "cannot open the key for writing".to_string())?;
            let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
            let e = unsafe { RegDeleteValueW(key.0, PCWSTR(wide.as_ptr())) };
            if e == OK { Ok(()) } else { Err(format!("registry error {}", e.0)) }
        }
        What::Service(n) => {
            let _ = run_quiet("sc.exe", &["stop", n]); // a stopped service answers with an error: fine
            run_quiet("sc.exe", &["delete", n])
        }
        What::Task(t) => run_quiet("schtasks.exe", &["/delete", "/tn", t, "/f"]),
        What::Remote(_) => Err("only the elevated process can remove this".into()),
    }
}

/// Deletes the chosen candidates. Every item is re-checked by `guard_cand` right before it is touched; registry / service / task items
/// need their backup first (written through `store`, which the elevated process points at its caller). Items that need administrator rights
/// are skipped unless `elevated`.
pub fn delete_cands(
    p: &Prog,
    cands: &[&Cand],
    env: &Env,
    reg: &dyn Reg,
    stamp: &str,
    elevated: bool,
    store: &dyn Fn(&str, &[u8]) -> Result<String, String>,
    progress: &dyn Fn(&str, bool, &str, usize, usize),
) -> Report {
    let mut log = vec![format!("Pill uninstall cleanup: {} ({stamp}){}", p.name, if elevated { " [administrator]" } else { "" })];
    let (mut done, mut failed, mut need_admin) = (Vec::new(), Vec::new(), 0usize);
    let mut todo: Vec<&Cand> = Vec::new();
    for c in cands {
        if c.admin && !elevated {
            need_admin += 1;
            log.push(format!("needs administrator, left alone: {}", c.shown));
        } else {
            todo.push(c);
        }
    }
    // backups first
    let mut items: Vec<(Hive, String, Option<String>)> = Vec::new();
    let mut tasks: Vec<(String, Vec<u8>)> = Vec::new();
    for c in &todo {
        if guard_cand(&c.what, p, env, reg).is_err() {
            continue; // refused below, with its reason
        }
        match &c.what {
            What::Key(h, s) => items.push((*h, s.clone(), None)),
            What::Value(h, s, n) => items.push((*h, s.clone(), Some(n.clone()))),
            What::Service(n) => items.push((Hive::Hklm, format!(r"SYSTEM\CurrentControlSet\Services\{n}"), None)),
            What::Task(t) => {
                if let Ok(b) = std::fs::read(env.windows.join("System32").join("Tasks").join(t.trim_start_matches('\\'))) {
                    tasks.push((t.clone(), b));
                }
            }
            _ => {}
        }
    }
    let mut backup: Option<String> = None;
    let mut backed_up = items.is_empty();
    if !items.is_empty() {
        match reg_dump(&items).and_then(|t| store("reg", &reg_bytes(&t))) {
            Ok(path) => {
                log.push(format!("backup: {path}"));
                backup = Some(path);
                backed_up = true;
            }
            Err(e) => log.push(format!("backup failed, registry items, services and tasks skipped: {e}")),
        }
    }
    let mut task_ok: Vec<String> = Vec::new();
    if backed_up {
        for (i, (t, bytes)) in tasks.iter().enumerate() {
            match store(&format!("task{i}.xml"), bytes) {
                Ok(path) => {
                    log.push(format!("task backup: {path}"));
                    task_ok.push(t.clone());
                }
                Err(e) => log.push(format!("task backup failed ({t}): {e}")),
            }
        }
    }
    // values first (a handler association needs its handler key), files, services, tasks, keys last
    let rank = |c: &Cand| match c.what {
        What::Value(..) => 0,
        What::Fs(_) => 1,
        What::Service(_) => 2,
        What::Task(_) => 3,
        _ => 4,
    };
    todo.sort_by_key(|c| rank(c));
    let total = todo.len();
    for (n, c) in todo.into_iter().enumerate() {
        let r: Result<(), String> = match guard_cand(&c.what, p, env, reg) {
            Err(e) => Err(format!("refused: {e}")),
            Ok(_) => match &c.what {
                What::Fs(_) => remove(&c.what),
                What::Task(t) if !task_ok.contains(t) => Err("no backup, not deleted".into()),
                _ if !backed_up => Err("no backup, not deleted".into()),
                _ => remove(&c.what),
            },
        };
        match r {
            Ok(()) => {
                log.push(format!("deleted {} {}", c.kind, c.shown));
                done.push((c.kind, c.shown.clone()));
                progress(&c.shown, true, "", n + 1, total);
            }
            Err(e) => {
                log.push(format!("NOT deleted {} {}: {e}", c.kind, c.shown));
                progress(&c.shown, false, &e, n + 1, total);
                failed.push((c.shown.clone(), e));
            }
        }
    }
    let log_path = store("log", (log.join("\r\n") + "\r\n").as_bytes()).ok();
    Report { done, failed, backup, log: log_path, need_admin }
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
    /// Every value as (name, type, bytes), names in their own case, for the backup.
    fn raw_values(&self) -> Vec<(String, u32, Vec<u8>)> {
        let mut out = Vec::new();
        let mut i = 0u32;
        while i < 20_000 {
            let (mut name, mut data) = (vec![0u16; 16_384], vec![0u8; 65_536]);
            let (mut nn, mut dn, mut ty) = (name.len() as u32, data.len() as u32, 0u32);
            let r = unsafe { RegEnumValueW(self.0, i, PWSTR(name.as_mut_ptr()), &mut nn, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut dn)) };
            if r == WIN32_ERROR(234) {
                // ERROR_MORE_DATA: grow the data buffer and ask again for the same index
                data = vec![0u8; dn as usize + 2];
                let (mut nn2, mut dn2) = (name.len() as u32, data.len() as u32);
                if unsafe { RegEnumValueW(self.0, i, PWSTR(name.as_mut_ptr()), &mut nn2, None, Some(&mut ty), Some(data.as_mut_ptr()), Some(&mut dn2)) } == OK {
                    out.push((String::from_utf16_lossy(&name[..nn2 as usize]), ty, data[..dn2 as usize].to_vec()));
                }
            } else if r == OK {
                out.push((String::from_utf16_lossy(&name[..nn as usize]), ty, data[..dn as usize].to_vec()));
            } else if r == WIN32_ERROR(259) {
                break;
            }
            i += 1;
        }
        out
    }
    /// REG_SZ / REG_EXPAND_SZ / REG_DWORD values, names lower-cased. Others are skipped.
    fn values(&self) -> Vals {
        let mut out = Vals::new();
        for i in 0..8192u32 {
            let (mut name, mut data) = ([0u16; 2048], vec![0u8; 8192]);
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
                    let t = String::from_utf16_lossy(&u).trim_end_matches('\0').to_string();
                    out.insert(k, if ty == 2 { Val::E(t) } else { Val::S(t) });
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
    manual: Vec<String>,
    cut: bool,
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

pub(crate) fn items_json(cands: &[Cand]) -> Vec<Value> {
    cands
        .iter()
        .enumerate()
        .map(|(i, c)| json!({ "id": i, "kind": c.kind, "path": c.shown, "size": c.size, "ticked": c.ticked, "note": c.note, "admin": c.admin, "tier": if c.tier == Tier::Certain { "certain" } else { "probable" } }))
        .collect()
}

/// The review state the page draws: the candidate list with everything needed for the two groups and the administrator section.
fn review_json(name: &str, cands: &[Cand], manual: &[String], cut: bool, extra: Value) -> Value {
    let mut v = json!({ "phase": if cands.is_empty() && manual.is_empty() { "done" } else { "review" }, "name": name, "items": items_json(cands), "manual": manual, "cut": cut,
        "needAdmin": cands.iter().filter(|c| c.admin).count(), "files": 0, "keys": 0, "other": 0, "failed": [] });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    v
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
        emit(app, review_json(&p.name, &[], &[], false, json!({})));
    } else {
        emit(app, json!({ "phase": "scanning", "name": p.name, "step": "starting", "found": 0 }));
        let (cands, manual, cut) = scan_all(app, p);
        let v = review_json(&p.name, &cands, &manual, cut, json!({}));
        flow(|f| {
            f.cands = cands;
            f.manual = manual;
            f.cut = cut;
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
        let picked: Vec<Cand> = ids.iter().filter_map(|i| f.cands.get(*i).cloned()).filter(|c| !c.admin).collect(); // unknown indexes and administrator items are dropped
        f.busy = true;
        Some((p, picked))
    }) else {
        return json!({ "ok": false });
    };
    std::thread::spawn(move || {
        emit(&app, json!({ "phase": "deleting", "name": p.name, "total": picked.len() }));
        let env = Env::real();
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()).to_string();
        let (dir, base) = (backup_dir(&env), format!("{}-{stamp}", sanitize(&p.name)));
        let refs: Vec<&Cand> = picked.iter().collect();
        let r = delete_cands(&p, &refs, &env, &RealReg, &stamp, false, &|name, bytes| file_store(&dir, &base, name, bytes), &|shown, ok, err, n, total| {
            let _ = app.emit_to("bar", "uninstall:item", json!({ "shown": shown, "ok": ok, "error": err, "n": n, "total": total }));
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
}

/// Opens the folder with the .reg backups and logs in Explorer.
#[tauri::command]
pub fn uninstall_open_backups() {
    let dir = backup_dir(&Env::real());
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::process::Command::new("explorer.exe").arg(&dir).creation_flags(NO_WINDOW).spawn();
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

/// The whole deep scan of `p`: (candidates, lines that are only listed, cut off by the caps). `step(stage, found so far)` reports progress.
/// Used by this process and by the elevated one, so both derive the same list.
pub fn scan_all_with(p: &Prog, env: &Env, reg: &dyn Reg, shared: bool, step: &dyn Fn(&str, usize)) -> (Vec<Cand>, Vec<String>, bool) {
    let mut b = Budget::new(400_000, 60);
    step("files and folders", 0);
    let mut cands = scan_fs(p, env, shared, &lnk_target, &mut b);
    let n = cands.len();
    cands.extend(scan_registry(p, env, reg, &mut b, &|s, c| step(s, n + c)));
    let mut seen = std::collections::HashSet::new();
    cands.retain(|c| seen.insert(cand_key(&c.what)));
    let kind_rank = |k: &str| ["folder", "file", "shortcut", "autorun", "key", "value", "service", "task", "item"].iter().position(|x| *x == k).unwrap_or(9);
    cands.sort_by_key(|c| (c.admin, c.tier != Tier::Certain, kind_rank(c.kind)));
    let drivers = list_drivers(p, env, reg);
    (cands, drivers, b.cut)
}

fn scan_all(app: &AppHandle, p: &Prog) -> (Vec<Cand>, Vec<String>, bool) {
    let env = Env::real();
    let install = p.install();
    let shared = install.as_ref().is_some_and(|i| flow(|f| f.locs.iter().any(|(k, l)| *k != p.sub && (depth_under(l, i).is_some() || depth_under(i, l).is_some()))));
    let name = p.name.clone();
    scan_all_with(p, &env, &RealReg, shared, &|s, c| emit(app, json!({ "phase": "scanning", "name": name, "step": s, "found": c })))
}

fn static_kind(k: &str) -> &'static str {
    ["folder", "file", "shortcut", "key", "value", "autorun", "service", "task"].iter().find(|x| **x == k).copied().unwrap_or("item")
}

/// "Scan as administrator" (`mode` scan: finds what only an elevated process can see) or "Remove system leftovers" (`mode` delete: the ticked
/// administrator items). Runs in the background; the outcome arrives as `uninstall:state`.
#[tauri::command]
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
    std::thread::spawn(move || {
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
                emit(&app, review_json(&p.name, &all, &manual2, cut, json!({ "phase": "review", "adminScanned": true })));
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
        crate::command::show(&app);
    });
    json!({ "ok": true })
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_deep;
