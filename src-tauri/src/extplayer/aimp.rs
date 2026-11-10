// AIMP through its documented remote API (AIMP SDK, "apiRemote"): a hidden window of class "AIMP2_RemoteInfo" takes window messages, and the
// shared memory "AIMP2_RemoteInfo" (2 KB) holds the current track. NOT tested against a real AIMP (none installed here): the constants and the
// TAIMPRemoteFileInfo layout follow the SDK as documented; a mismatch shows up as an empty title, never as a crash (every read is bounds-checked).
use super::{pid_of, send, Info, Kind};
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ};
use windows::Win32::UI::WindowsAndMessaging::FindWindowW;

const WM_USER: u32 = 0x400;
const WM_AIMP_COMMAND: u32 = WM_USER + 0x75;
const WM_AIMP_PROPERTY: u32 = WM_USER + 0x77;
const PROP_PLAYER_POSITION: usize = 0x20;
const PROP_PLAYER_DURATION: usize = 0x30;
const PROP_PLAYER_STATE: usize = 0x40;
const PROP_GET: usize = 0;
const PROP_SET: usize = 1;
const CMD_PLAYPAUSE: usize = 14;
const CMD_NEXT: usize = 17;
const CMD_PREV: usize = 18;
/// AIMP_RA_PLAYER_STATE_PLAYING (0 = stopped, 1 = paused).
const STATE_PLAYING: isize = 2;
const MAP_SIZE: usize = 2048;
/// TAIMPRemoteFileInfo: 5 DWORD, INT64 FileSize, 3 DWORD, 6 DWORD string lengths, 6 reserved DWORD.
const HEADER: usize = 88;

fn window() -> Option<HWND> {
    unsafe { FindWindowW(w!("AIMP2_RemoteInfo"), None) }.ok().filter(|h| !h.0.is_null())
}

/// The track part of TAIMPRemoteFileInfo.
#[derive(Debug, Default, PartialEq)]
pub(super) struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
}

/// Parses the shared-memory block. Strings follow the header as UTF-16 in the order Album, Artist, Date, FileName, Genre, Title.
pub(super) fn parse_track(b: &[u8]) -> Option<Track> {
    if b.len() < HEADER {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let duration_ms = u32_at(16) as u64;
    // lengths (in characters) of Album, Artist, Date, FileName, Genre, Title at offsets 40..64
    let lens: Vec<usize> = (0..6).map(|k| u32_at(40 + 4 * k) as usize).collect();
    let mut at = HEADER;
    let mut parts: Vec<String> = Vec::with_capacity(6);
    for n in lens {
        let end = at.checked_add(n.checked_mul(2)?)?;
        if end > b.len() {
            return None;
        }
        let units: Vec<u16> = b[at..end].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        parts.push(String::from_utf16_lossy(&units));
        at = end;
    }
    let mut it = parts.into_iter();
    let album = it.next()?;
    let artist = it.next()?;
    let title = it.nth(3)?; // skip Date, FileName, Genre
    Some(Track { title, artist, album, duration_ms })
}

fn read_track() -> Option<Track> {
    unsafe {
        let m = OpenFileMappingW(FILE_MAP_READ.0, false, w!("AIMP2_RemoteInfo")).ok()?;
        let view = MapViewOfFile(m, FILE_MAP_READ, 0, 0, MAP_SIZE);
        let out = if view.Value.is_null() {
            None
        } else {
            let t = parse_track(std::slice::from_raw_parts(view.Value as *const u8, MAP_SIZE));
            let _ = UnmapViewOfFile(view);
            t
        };
        let _ = CloseHandle(m);
        out
    }
}

pub(super) fn read() -> Option<Info> {
    let hwnd = window()?;
    let state = send(hwnd, WM_AIMP_PROPERTY, PROP_PLAYER_STATE | PROP_GET, 0)?;
    let position_ms = send(hwnd, WM_AIMP_PROPERTY, PROP_PLAYER_POSITION | PROP_GET, 0).unwrap_or(0).max(0) as u64;
    let track = read_track().unwrap_or_default();
    let duration_ms = if track.duration_ms > 0 { track.duration_ms } else { send(hwnd, WM_AIMP_PROPERTY, PROP_PLAYER_DURATION | PROP_GET, 0).unwrap_or(0).max(0) as u64 };
    Some(Info {
        kind: Kind::Aimp,
        pid: pid_of(hwnd),
        title: track.title,
        artist: track.artist,
        album: track.album,
        playing: state == STATE_PLAYING,
        position_ms: if duration_ms > 0 { position_ms.min(duration_ms) } else { position_ms },
        duration_ms,
    })
}

pub(super) fn control(action: &str) -> bool {
    let cmd = match action {
        "play_pause" => CMD_PLAYPAUSE,
        "next" => CMD_NEXT,
        "prev" => CMD_PREV,
        _ => return false,
    };
    window().and_then(|h| send(h, WM_AIMP_COMMAND, cmd, 0)).is_some()
}

pub(super) fn seek(ms: u64) -> bool {
    window().and_then(|h| send(h, WM_AIMP_PROPERTY, PROP_PLAYER_POSITION | PROP_SET, ms as isize)).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(duration: u32, album: &str, artist: &str, file: &str, title: &str) -> Vec<u8> {
        let mut b = vec![0u8; HEADER];
        b[0..4].copy_from_slice(&(HEADER as u32).to_le_bytes());
        b[4..8].copy_from_slice(&1u32.to_le_bytes()); // Active
        b[16..20].copy_from_slice(&duration.to_le_bytes());
        let strs = [album, artist, "2001", file, "Rock", title];
        for (k, s) in strs.iter().enumerate() {
            b[40 + 4 * k..44 + 4 * k].copy_from_slice(&(s.encode_utf16().count() as u32).to_le_bytes());
        }
        for s in strs {
            b.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
        }
        b.resize(MAP_SIZE, 0);
        b
    }

    #[test]
    fn track_strings_come_out_in_order() {
        let t = parse_track(&block(215_000, "Лучшее", "Кино", r"C:\m\x.flac", "Группа крови")).unwrap();
        assert_eq!(t, Track { title: "Группа крови".into(), artist: "Кино".into(), album: "Лучшее".into(), duration_ms: 215_000 });
    }

    #[test]
    fn empty_and_truncated_blocks_are_safe() {
        assert!(parse_track(&[0u8; 10]).is_none());
        let t = parse_track(&vec![0u8; MAP_SIZE]).unwrap();
        assert_eq!(t, Track::default(), "AIMP idle: all lengths zero");
        // a length that points past the block must not panic
        let mut b = vec![0u8; MAP_SIZE];
        b[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(parse_track(&b).is_none());
    }
}
