//! A file for everything the app prints to stderr (the ~70 `eprintln!` of the modules and the panic messages): the release build has no
//! console, so without this a failure leaves no trace. `%LOCALAPPDATA%\Pill\logs\pill.log`; at the start it is moved to `pill.log.1` once it
//! passes 1 MiB (one generation is kept). The diagnostic report (`report.rs`) can attach it.
use std::io::Write;
use std::os::windows::io::AsRawHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};

const MAX_BYTES: u64 = 1024 * 1024;

pub fn log_path() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Pill").join("logs").join("pill.log"))
}

/// Rotates when the file is too big, then returns where to append.
fn prepare(path: &std::path::Path, max: u64) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() > max) {
        let old = path.with_extension("log.1");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(path, old)?;
    }
    Ok(())
}

/// Sends stderr to the log file. Failures are ignored: no log is better than no app.
pub fn init() {
    let Some(path) = log_path() else { return };
    if prepare(&path, MAX_BYTES).is_err() {
        return;
    }
    let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    let _ = writeln!(file, "--- pill {} started ---", env!("CARGO_PKG_VERSION"));
    // std's stderr looks the handle up on every write, so replacing it is enough; the file lives as long as the process
    unsafe {
        if SetStdHandle(STD_ERROR_HANDLE, HANDLE(file.as_raw_handle())).is_ok() {
            std::mem::forget(file);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_big_log_is_rotated_and_a_small_one_is_kept() {
        let dir = std::env::temp_dir().join(format!("pill-applog-{}", std::process::id()));
        let p = dir.join("logs").join("pill.log");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, vec![b'x'; 100]).unwrap();
        prepare(&p, 1000).unwrap();
        assert!(p.exists() && !p.with_extension("log.1").exists(), "small: kept");
        prepare(&p, 50).unwrap();
        assert!(!p.exists() && p.with_extension("log.1").exists(), "big: moved to .1");
        std::fs::write(&p, b"y").unwrap();
        std::fs::write(p.with_extension("log.1"), b"older").unwrap();
        prepare(&p, 0).unwrap();
        assert_eq!(std::fs::read(p.with_extension("log.1")).unwrap(), b"y", "only one generation is kept");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
