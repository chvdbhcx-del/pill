// Winamp (and WACUP, same window class) through its classic IPC: window class "Winamp v1.x", WM_USER with the IPC code in lParam, WM_COMMAND
// for the buttons. The track name is taken from the main window's caption ("12. Artist - Title - Winamp"), which avoids reading the player's
// memory. NOT tested against a real Winamp (none installed here); the IPC numbers are those of the public wa_ipc.h.
use super::{pid_of, send, Info, Kind};
use windows::core::w;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowTextW};

const WM_USER: u32 = 0x400;
const WM_COMMAND: u32 = 0x111;
const IPC_ISPLAYING: isize = 104; // 1 playing, 3 paused, 0 stopped
const IPC_GETOUTPUTTIME: isize = 105; // wParam 0: position ms, 1: length s
const IPC_JUMPTOTIME: isize = 106; // wParam ms
const CMD_PREV: usize = 40044;
const CMD_PLAY: usize = 40045;
const CMD_PAUSE: usize = 40046; // toggles pause / resume
const CMD_NEXT: usize = 40048;

fn window() -> Option<HWND> {
    unsafe { FindWindowW(w!("Winamp v1.x"), None) }.ok().filter(|h| !h.0.is_null())
}

fn caption(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..n.min(buf.len())])
}

/// "12. Artist - Title - Winamp [Paused]" -> (artist, title). Without a " - " in the name the whole text is the title.
/// None when no track is loaded (the caption is just the program name).
pub(super) fn parse_caption(cap: &str) -> Option<(String, String)> {
    let mut s = cap.trim();
    for tail in [" [Paused]", " [Stopped]"] {
        s = s.strip_suffix(tail).unwrap_or(s);
    }
    let s = ["- Winamp", "- WACUP"].iter().find_map(|t| s.strip_suffix(t)).unwrap_or(s).trim_end();
    let s = s.strip_suffix('*').unwrap_or(s).trim(); // "*" = buffering a stream
    // leading playlist number "12. "
    let digits = s.chars().take_while(|c| c.is_ascii_digit()).count();
    let s = if digits > 0 { s[digits..].strip_prefix(". ").unwrap_or(s) } else { s };
    let s = s.trim();
    if s.is_empty() || s.starts_with("Winamp") || s.starts_with("WACUP") {
        return None;
    }
    Some(match s.split_once(" - ") {
        Some((a, t)) => (a.trim().to_string(), t.trim().to_string()),
        None => (String::new(), s.to_string()),
    })
}

pub(super) fn read() -> Option<Info> {
    let hwnd = window()?;
    let state = send(hwnd, WM_USER, 0, IPC_ISPLAYING)?;
    let (artist, title) = parse_caption(&caption(hwnd)).unwrap_or_default();
    let (mut position_ms, mut duration_ms) = (0u64, 0u64);
    if state != 0 {
        // -1 when nothing is playing
        position_ms = send(hwnd, WM_USER, 0, IPC_GETOUTPUTTIME).filter(|v| *v >= 0).unwrap_or(0) as u64;
        duration_ms = send(hwnd, WM_USER, 1, IPC_GETOUTPUTTIME).filter(|v| *v > 0).unwrap_or(0) as u64 * 1000;
    }
    Some(Info {
        kind: Kind::Winamp,
        pid: pid_of(hwnd),
        title,
        artist,
        album: String::new(),
        playing: state == 1,
        position_ms: if duration_ms > 0 { position_ms.min(duration_ms) } else { position_ms },
        duration_ms,
    })
}

pub(super) fn control(action: &str) -> bool {
    let Some(hwnd) = window() else { return false };
    let cmd = match action {
        // Winamp's pause button toggles; a stopped player needs play
        "play_pause" => match send(hwnd, WM_USER, 0, IPC_ISPLAYING) {
            Some(0) => CMD_PLAY,
            Some(_) => CMD_PAUSE,
            None => return false,
        },
        "next" => CMD_NEXT,
        "prev" => CMD_PREV,
        _ => return false,
    };
    send(hwnd, WM_COMMAND, cmd, 0).is_some()
}

pub(super) fn seek(ms: u64) -> bool {
    window().and_then(|h| send(h, WM_USER, ms as usize, IPC_JUMPTOTIME)).map_or(false, |r| r >= 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &str, t: &str) -> Option<(String, String)> {
        Some((a.to_string(), t.to_string()))
    }

    #[test]
    fn captions_become_artist_and_title() {
        assert_eq!(parse_caption("12. Кино - Группа крови - Winamp"), p("Кино", "Группа крови"));
        assert_eq!(parse_caption("1. Daft Punk - Around the World - Winamp [Paused]"), p("Daft Punk", "Around the World"));
        assert_eq!(parse_caption("3. Radio Paradise - Winamp"), p("", "Radio Paradise"), "no artist separator");
        assert_eq!(parse_caption("7. A - B - C - WACUP"), p("A", "B - C"), "the first separator splits");
        assert_eq!(parse_caption("5. Some Stream * - Winamp"), p("", "Some Stream"), "buffering mark");
    }

    #[test]
    fn an_idle_caption_has_no_track() {
        assert_eq!(parse_caption("Winamp"), None);
        assert_eq!(parse_caption("Winamp 5.666"), None);
        assert_eq!(parse_caption("WACUP"), None);
        assert_eq!(parse_caption(""), None);
        assert_eq!(parse_caption("- Winamp"), None);
    }
}
