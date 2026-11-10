// File transfers of the phone link: name sanitising, unique target files, the transfer table (offers, uploads, downloads).
use serde_json::{json, Value};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_PARALLEL: usize = 2;
const KEEP: usize = 200;
const RESERVED: [&str; 22] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];

/// A file name the phone sent, made safe for Windows: no path parts, no `..`, no control or reserved characters, no alternate data streams
/// (`:` is dropped), no reserved device names, no trailing dots / spaces, at most 150 characters (the extension is kept).
pub fn sanitize_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let mut s: String = last.chars().filter(|c| !c.is_control() && !matches!(*c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\u{202e}')).collect();
    s = s.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if s.chars().count() > 150 {
        let (stem, ext) = match s.rfind('.') {
            Some(i) if s.chars().count() - s[..i].chars().count() <= 16 => (s[..i].to_string(), s[i..].to_string()),
            _ => (s.clone(), String::new()),
        };
        s = stem.chars().take(150usize.saturating_sub(ext.chars().count())).collect::<String>() + &ext;
    }
    let stem = s.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    if s.is_empty() || RESERVED.contains(&stem.as_str()) {
        s = format!("_{s}");
        if s == "_" {
            s = "file".into();
        }
    }
    s
}

/// Creates `dir/name` (or `name (1).ext`, `name (2).ext`, ...) without ever overwriting: the creation itself is the existence check.
pub fn create_unique(dir: &Path, name: &str) -> std::io::Result<(File, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 0..1000 {
        let cand = if n == 0 { name.to_string() } else { format!("{stem} ({n}){ext}") };
        let p = dir.join(&cand);
        match OpenOptions::new().write(true).create_new(true).open(&p) {
            Ok(f) => return Ok((f, p)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "too many files with this name"))
}

/// Programs, scripts and shortcuts that a file from a phone must never start by a click: they are only revealed in Explorer.
pub fn risky_to_open(p: &Path) -> bool {
    const RISKY: &[&str] = &[
        "exe", "msi", "msix", "appx", "bat", "cmd", "com", "scr", "pif", "lnk", "url", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "hta", "jar", "reg", "dll", "cpl", "msc", "gadget", "inf", "sys", "apk", "py", "pyw", "iso", "vhd",
    ];
    p.extension().map_or(true, |e| RISKY.contains(&e.to_string_lossy().to_ascii_lowercase().as_str()))
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

pub fn mime_of(name: &str) -> &'static str {
    match name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("mp4") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("pdf") => "application/pdf",
        Some("txt" | "md" | "log") => "text/plain",
        Some("zip") => "application/zip",
        Some("apk") => "application/vnd.android.package-archive",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    /// phone -> PC
    In,
    /// PC -> phone
    Out,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// an offer waiting for the user's decision (auto-accept off)
    Pending,
    /// accepted, the transfer has not started
    Accepted,
    Active,
    Done,
    Failed,
    Rejected,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Accepted => "accepted",
            Status::Active => "active",
            Status::Done => "done",
            Status::Failed => "failed",
            Status::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Transfer {
    pub id: String,
    pub device: String,
    pub dir: Dir,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub status: Status,
    /// In: where it landed; Out: the file on this PC
    pub path: Option<PathBuf>,
    pub bytes: u64,
    pub at: i64,
}

impl Transfer {
    pub fn public(&self) -> Value {
        json!({
            "id": self.id, "device": self.device, "dir": if self.dir == Dir::In { "in" } else { "out" }, "name": self.name, "size": self.size,
            "mime": self.mime, "status": self.status.as_str(), "path": self.path.as_ref().map(|p| p.to_string_lossy().to_string()),
            "bytes": self.bytes, "at": self.at,
        })
    }
}

#[derive(Default)]
pub struct Transfers {
    m: Mutex<Vec<Transfer>>,
    active: Arc<AtomicUsize>,
}

/// Holds one of the parallel transfer slots until dropped.
pub struct Slot(Arc<AtomicUsize>);
impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    Unknown,
    Busy,
}

impl Transfers {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Transfer>> {
        self.m.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn add(&self, t: Transfer) {
        let mut g = self.lock();
        g.retain(|x| x.id != t.id);
        g.push(t);
        if g.len() > KEEP {
            let drop_n = g.len() - KEEP;
            g.drain(..drop_n);
        }
    }

    pub fn get(&self, id: &str) -> Option<Transfer> {
        self.lock().iter().find(|t| t.id == id).cloned()
    }

    pub fn update(&self, id: &str, f: impl FnOnce(&mut Transfer)) -> Option<Transfer> {
        let mut g = self.lock();
        let t = g.iter_mut().find(|t| t.id == id)?;
        f(t);
        Some(t.clone())
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Transfer> {
        let mut v = self.lock().clone();
        v.reverse();
        v
    }

    /// An accepted upload of this device may start: takes a slot and marks it active.
    pub fn begin_upload(&self, id: &str, device: &str) -> Result<(Transfer, Slot), Refused> {
        let t = self.get(id).filter(|t| t.device == device && t.dir == Dir::In && t.status == Status::Accepted).ok_or(Refused::Unknown)?;
        let slot = self.slot()?;
        Ok((self.update(&t.id, |t| t.status = Status::Active).unwrap_or(t), slot))
    }

    /// A download of an offered PC file by its device.
    pub fn begin_download(&self, id: &str, device: &str) -> Result<(Transfer, Slot), Refused> {
        let t = self.get(id).filter(|t| t.device == device && t.dir == Dir::Out && matches!(t.status, Status::Accepted | Status::Active | Status::Done)).ok_or(Refused::Unknown)?;
        Ok((t, self.slot()?))
    }

    fn slot(&self) -> Result<Slot, Refused> {
        if self.active.fetch_add(1, Ordering::SeqCst) >= MAX_PARALLEL {
            self.active.fetch_sub(1, Ordering::SeqCst);
            return Err(Refused::Busy);
        }
        Ok(Slot(self.active.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitize_name("photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_name("../../Windows/system32/evil.dll"), "evil.dll");
        assert_eq!(sanitize_name("C:\\Users\\x\\a.txt"), "a.txt");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name("..."), "file");
        assert_eq!(sanitize_name(""), "file");
        assert_eq!(sanitize_name("a.txt:hidden:$DATA"), "a.txthidden$DATA", "no alternate data streams");
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("nul.txt"), "_nul.txt");
        assert_eq!(sanitize_name("com1.tar.gz"), "_com1.tar.gz");
        assert_eq!(sanitize_name("a<b>c|d?e*f\"g.txt"), "abcdefg.txt");
        assert_eq!(sanitize_name("report. . "), "report");
        assert_eq!(sanitize_name("a\u{0}b\n.txt"), "ab.txt");
        assert_eq!(sanitize_name("x\u{202e}gpj.exe"), "xgpj.exe", "right-to-left override is dropped");
        let long = format!("{}.mp4", "я".repeat(300));
        let s = sanitize_name(&long);
        assert!(s.chars().count() <= 150 && s.ends_with(".mp4"));
        assert_eq!(sanitize_name("Привет мир.docx"), "Привет мир.docx");
    }

    #[test]
    fn unique_files_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("pill-phone-test-files-{}", crate::phone::reg::new_id()));
        let (_, a) = create_unique(&dir, "x.txt").unwrap();
        let (_, b) = create_unique(&dir, "x.txt").unwrap();
        let (_, c) = create_unique(&dir, "noext").unwrap();
        let (_, d) = create_unique(&dir, "noext").unwrap();
        assert_eq!(a.file_name().unwrap(), "x.txt");
        assert_eq!(b.file_name().unwrap(), "x (1).txt");
        assert_eq!(c.file_name().unwrap(), "noext");
        assert_eq!(d.file_name().unwrap(), "noext (1)");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn tr(id: &str, dev: &str, dir: Dir, st: Status) -> Transfer {
        Transfer { id: id.into(), device: dev.into(), dir, name: "a".into(), size: 1, mime: String::new(), status: st, path: None, bytes: 0, at: 0 }
    }

    #[test]
    fn uploads_need_an_accepted_offer_of_the_same_device_and_two_slots_at_most() {
        let t = Transfers::default();
        t.add(tr("a", "d1", Dir::In, Status::Accepted));
        t.add(tr("b", "d1", Dir::In, Status::Accepted));
        t.add(tr("c", "d1", Dir::In, Status::Accepted));
        t.add(tr("p", "d1", Dir::In, Status::Pending));
        t.add(tr("o", "d2", Dir::In, Status::Accepted));
        assert_eq!(t.begin_upload("zzz", "d1").err(), Some(Refused::Unknown));
        assert_eq!(t.begin_upload("p", "d1").err(), Some(Refused::Unknown), "not accepted yet");
        assert_eq!(t.begin_upload("o", "d1").err(), Some(Refused::Unknown), "somebody else's offer");
        let s1 = t.begin_upload("a", "d1").unwrap();
        let s2 = t.begin_upload("b", "d1").unwrap();
        assert_eq!(t.get("a").unwrap().status, Status::Active);
        assert_eq!(t.begin_upload("c", "d1").err(), Some(Refused::Busy));
        drop(s1);
        assert!(t.begin_upload("c", "d1").is_ok());
        drop(s2);
    }

    #[test]
    fn the_table_is_capped_newest_first() {
        let t = Transfers::default();
        for i in 0..(KEEP + 5) {
            t.add(tr(&format!("t{i}"), "d", Dir::Out, Status::Done));
        }
        let l = t.list();
        assert_eq!(l.len(), KEEP);
        assert_eq!(l[0].id, format!("t{}", KEEP + 4));
        assert!(t.get("t0").is_none());
    }

    #[test]
    fn programs_from_a_phone_are_only_revealed() {
        for p in ["a.exe", "B.PS1", "x.lnk", "noext", "a.tar.msi"] {
            assert!(risky_to_open(Path::new(p)), "{p}");
        }
        for p in ["photo.jpg", "doc.docx", "a.pdf", "clip.mp4"] {
            assert!(!risky_to_open(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn id_shape() {
        assert!(valid_id("3f2a-9B_x") && !valid_id("") && !valid_id("a/b") && !valid_id("..") && !valid_id(&"x".repeat(65)));
    }
}
