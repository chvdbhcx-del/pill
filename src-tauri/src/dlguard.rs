// Guards for "is this file in Downloads really a finished download": the browser temp names, the user's own list of extensions to ignore
// (settings `live.dlIgnoreExt`, e.g. "log, bak, iso") and the "someone still has it open for writing" test. Used by liveplus.rs (the
// "Downloaded" card) and livesys.rs (the progress chip). A download that waits in a browser's "Save as" dialog (Yandex Browser keeps a
// placeholder with the final name, or a stalled temp file) is not finished: size stable is not enough, the file must also be free and
// its temp twin (`<name>.crdownload`, `<name>.part`, ...) must be gone.
use std::collections::HashSet;
use std::path::Path;

pub const TEMP_EXT: &[&str] = &[".crdownload", ".part", ".partial", ".download", ".tmp", ".opdownload"];
const MAX_EXTRA: usize = 40;

/// "log, .BAK;iso  *.zip" -> [".log", ".bak", ".iso", ".zip"]: lower case, a leading dot, no duplicates, only plain extension characters.
pub fn parse_ext_list(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in s.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let t = t.trim().trim_start_matches('*').trim_start_matches('.').to_lowercase();
        if t.is_empty() || t.len() > 16 || !t.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        let e = format!(".{t}");
        if !out.contains(&e) && out.len() < MAX_EXTRA {
            out.push(e);
        }
    }
    out
}

/// The user's extra extensions from the settings.
pub fn extra_from_settings() -> Vec<String> {
    parse_ext_list(&crate::settings::get_str("live.dlIgnoreExt", ""))
}

/// A browser temp name or an extension the user asked to ignore.
pub fn is_temp(name: &str, extra: &[String]) -> bool {
    let low = name.to_lowercase();
    TEMP_EXT.iter().any(|e| low.ends_with(e)) || extra.iter().any(|e| low.ends_with(e.as_str()))
}

/// Another listed file is the temp twin of `name` (`report.pdf` + `report.pdf.crdownload`): the browser has not finished it yet.
pub fn has_temp_twin<'a>(name: &str, listing: impl IntoIterator<Item = &'a str>) -> bool {
    let low = name.to_lowercase();
    let twins: HashSet<String> = TEMP_EXT.iter().map(|e| format!("{low}{e}")).collect();
    listing.into_iter().any(|n| twins.contains(&n.to_lowercase()))
}

/// True while another process has the file open for writing (the browser is still saving it). Opening it while denying write sharing
/// fails with a sharing violation exactly then; a file that cannot be opened for another reason counts as free.
#[cfg(windows)]
pub fn open_for_writing(p: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    const ERROR_SHARING_VIOLATION: i32 = 32;
    match std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ).open(p) {
        Ok(_) => false,
        Err(e) => e.raw_os_error() == Some(ERROR_SHARING_VIOLATION),
    }
}

#[cfg(not(windows))]
pub fn open_for_writing(_p: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_list_is_cleaned() {
        assert_eq!(parse_ext_list("log, .BAK;iso  *.zip, log"), [".log", ".bak", ".iso", ".zip"]);
        assert!(parse_ext_list("").is_empty());
        assert!(parse_ext_list("../x, a/b, c:d, ?").is_empty(), "only plain extension characters");
        assert_eq!(parse_ext_list(&(0..60).map(|i| format!("e{i}")).collect::<Vec<_>>().join(",")).len(), MAX_EXTRA);
    }

    #[test]
    fn temp_names_and_user_extensions() {
        let extra = parse_ext_list("bak");
        for n in ["a.crdownload", "B.PART", "c.tmp", "d.partial", "e.opdownload", "f.BAK"] {
            assert!(is_temp(n, &extra), "{n}");
        }
        assert!(!is_temp("a.pdf", &extra));
        assert!(!is_temp("f.bak", &[]), "not ignored until the user adds it");
    }

    #[test]
    fn a_file_with_a_temp_twin_is_not_finished() {
        let names = ["Setup.exe", "setup.exe.CRDOWNLOAD", "other.zip"];
        assert!(has_temp_twin("setup.exe", names));
        assert!(!has_temp_twin("other.zip", names));
        assert!(!has_temp_twin("Setup.exe.crdownload", names), "a temp file has no twin of its own");
    }

    #[cfg(windows)]
    #[test]
    fn a_file_open_for_writing_is_held() {
        use std::io::Write;
        let p = std::env::temp_dir().join(format!("pill-dlguard-{}.bin", std::process::id()));
        let mut w = std::fs::File::create(&p).unwrap(); // File::create shares read + write with others, but is a writer itself
        w.write_all(b"x").unwrap();
        assert!(open_for_writing(&p), "a writer is still open");
        drop(w);
        assert!(!open_for_writing(&p), "free once closed");
        let _ = std::fs::remove_file(&p);
    }
}
