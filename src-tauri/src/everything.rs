// Everything (voidtools) for the command-bar file search: an opt-in download of the OFFICIAL installer and es.exe (Settings > File search).
// Flow (all on clicks, modelled on aisetup.rs): Download = installer + es.exe zip from www.voidtools.com over https (host allowlist in aisetup::HOSTS,
// redirects re-checked), size + SHA-256 pinned below, es.exe unpacked into %APPDATA%\pill\ai\everything\; "Run installer" is a SEPARATE click that
// hands the installer to the shell, so Windows shows its own UAC prompt (Everything needs its service to index NTFS). Pill never elevates itself.
// Commands: everything_status, everything_download (progress: aisetup:progress ids "everything-setup" / "everything-es"), everything_run_installer.
// command.rs find_es() also looks in this folder.
use crate::aisetup::{self, Item};
use serde_json::{json, Value};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use tauri::AppHandle;

const VERSION: &str = "1.4.1.1032"; // the stable Everything line (1.5 is still an alpha)
const ES_VERSION: &str = "1.1.0.38";

// Pinned from the files on voidtools.com (the installer's SHA-256 also equals the one in voidtools' own Everything-1.4.1.1032.sha256).
static SETUP: Item = Item {
    id: "setup",
    url: "https://www.voidtools.com/Everything-1.4.1.1032.x64-Setup.exe",
    file: "Everything-1.4.1.1032.x64-Setup.exe",
    size: 1_998_736,
    sha256: "c42efad041d4c0bb4d4ac97ae7cbe89f153ec1fe078772392e749c7f5d5282d3",
};
static ES_ZIP: Item = Item {
    id: "es",
    url: "https://www.voidtools.com/ES-1.1.0.38.x64.zip",
    file: "ES-1.1.0.38.x64.zip",
    size: 117_393,
    sha256: "5e0c70cbf4f694080c34aa7c6c745e606c16fe76a4b5423b93ebf9dc34274c99",
};
const MAX_ES: u64 = 4 << 20; // es.exe is ~256 KB

pub(crate) fn dir() -> PathBuf {
    aisetup::ai_dir().join("everything")
}
fn es_exe() -> PathBuf {
    dir().join("es.exe")
}
fn setup_path() -> PathBuf {
    dir().join(SETUP.file)
}
/// A file that was size-checked and SHA-256 verified before its rename is complete when its length matches.
fn file_ok(p: &Path, it: &Item) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.len() == it.size)
}

fn is_mz(p: &Path) -> bool {
    let mut b = [0u8; 2];
    File::open(p).and_then(|mut f| f.read_exact(&mut b)).is_ok() && &b == b"MZ"
}

/// Unpack es.exe (any folder inside the archive, nothing else) to `out`. Unsafe entry names reject the archive; the result must be a PE file.
fn extract_es<R: Read + Seek>(src: R, out: &Path) -> Result<(), String> {
    let mut ar = zip::ZipArchive::new(src).map_err(|e| format!("not a valid zip: {e}"))?;
    let tmp = out.with_extension("exe.new");
    let mut found = false;
    for i in 0..ar.len() {
        let mut f = ar.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let rel = aisetup::safe_name(f.name()).ok_or_else(|| format!("unsafe path in the archive: {}", f.name()))?;
        if !f.is_file() || !rel.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("es.exe")) {
            continue;
        }
        if f.size() > MAX_ES {
            return Err("es.exe is larger than expected".into());
        }
        let size = f.size();
        let mut o = File::create(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        let n = std::io::copy(&mut (&mut f).take(MAX_ES + 1), &mut o).map_err(|e| format!("cannot unpack es.exe: {e}"))?;
        if n != size {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("es.exe: unpacked {n} bytes, expected {size}"));
        }
        found = true;
        break;
    }
    if !found {
        return Err("es.exe is not in the archive".into());
    }
    if !is_mz(&tmp) {
        let _ = std::fs::remove_file(&tmp);
        return Err("the unpacked es.exe is not a Windows program".into());
    }
    let _ = std::fs::remove_file(out);
    std::fs::rename(&tmp, out).map_err(|e| format!("cannot install es.exe: {e}"))
}

/// What the Settings card draws.
#[tauri::command(async)]
pub fn everything_status() -> Value {
    json!({
        "installed": crate::command::find_everything_gui().is_some(),
        "es": crate::command::find_es().is_some(),
        "running": crate::command::everything_running(),
        "setupReady": file_ok(&setup_path(), &SETUP) && is_mz(&setup_path()),
        "esOwn": es_exe().is_file(),
        "version": VERSION, "esVersion": ES_VERSION,
        "size": SETUP.size + ES_ZIP.size, "host": "www.voidtools.com",
        "dir": dir().to_string_lossy(),
    })
}

/// Download the installer and es.exe (what is missing), verify, unpack es.exe. The installer is NOT started. Progress ids: everything-setup / everything-es.
#[tauri::command]
pub fn everything_download(app: AppHandle) -> Value {
    aisetup::spawn_job(app, "everything-setup", |app, abort| {
        std::fs::create_dir_all(dir()).map_err(|e| format!("cannot create {}: {e}", dir().display()))?;
        let setup = setup_path();
        if !(file_ok(&setup, &SETUP) && is_mz(&setup)) {
            aisetup::download(app, "everything-setup", SETUP.id, SETUP.url, &setup, Some(SETUP.size), Some(SETUP.sha256), SETUP.size, abort)?;
            if !is_mz(&setup) {
                let _ = std::fs::remove_file(&setup);
                return Err("the downloaded file is not a Windows program".into());
            }
        }
        if !es_exe().is_file() {
            let zip = dir().join(ES_ZIP.file);
            aisetup::download(app, "everything-es", ES_ZIP.id, ES_ZIP.url, &zip, Some(ES_ZIP.size), Some(ES_ZIP.sha256), ES_ZIP.size, abort)?;
            let r = File::open(&zip).map_err(|e| e.to_string()).and_then(|f| extract_es(f, &es_exe()));
            let _ = std::fs::remove_file(&zip);
            r?;
            crate::command::forget_es();
        }
        Ok(setup.to_string_lossy().into_owned())
    })
}

/// Second explicit click: start the downloaded installer through the shell (its own window and UAC prompt).
#[tauri::command]
pub fn everything_run_installer() -> Value {
    let p = setup_path();
    if !(file_ok(&p, &SETUP) && is_mz(&p)) {
        return aisetup::err("Download the installer first.");
    }
    aisetup::run_installer(&p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    #[test]
    fn everything_pins_are_official_https_voidtools() {
        for it in [&SETUP, &ES_ZIP] {
            let p = crate::net::parse_url(it.url).unwrap();
            assert!(p.https && aisetup::host_list().iter().any(|h| crate::net::host_matches(h, &p.host)), "{}", it.url);
            assert_eq!(p.host, "www.voidtools.com");
            assert_eq!(it.sha256.len(), 64, "{}", it.id);
            assert!(it.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(it.url.ends_with(it.file) && it.file.contains(if it.id == "setup" { VERSION } else { ES_VERSION }));
            assert!((50_000..50_000_000).contains(&it.size), "{}", it.id);
        }
        assert_eq!(es_exe().file_name().unwrap(), "es.exe");
        assert_eq!(es_exe().parent().unwrap().file_name().unwrap(), "everything");
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (n, b) in entries {
            zw.start_file(*n, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored)).unwrap();
            zw.write_all(b).unwrap();
        }
        zw.finish().unwrap().into_inner()
    }

    #[test]
    fn es_unpack_takes_only_a_real_es_exe() {
        let dir = std::env::temp_dir().join(format!("pill-es-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("es.exe");
        let pe: &[u8] = b"MZ\x90\x00 fake";
        extract_es(Cursor::new(zip_of(&[("readme.txt", b"x"), ("sub/ES.EXE", pe)])), &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), pe);
        assert!(!out.with_extension("exe.new").exists());
        for bad in [zip_of(&[("../es.exe", pe)]), zip_of(&[("other.exe", pe)]), zip_of(&[("es.exe", b"not a program")]), b"not a zip".to_vec()] {
            let _ = std::fs::remove_file(&out);
            assert!(extract_es(Cursor::new(bad), &out).is_err());
            assert!(!out.exists());
            assert!(!out.with_extension("exe.new").exists());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
