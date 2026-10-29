// Now playing, media controls, master volume, battery. Owner: media agent (lane L2).
// GSMTC sessions with event subscriptions (no polling); one worker thread coalesces events and emits `media:changed`.
// The page interpolates the timeline between events.
use crate::ui::bus::{self, Target};
use base64::Engine;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows::core::Interface;
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode,
};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager_,
    GlobalSystemMediaTransportControlsSessionMediaProperties as Props,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStream};
use windows::Win32::Foundation::BOOL;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, eConsole, eRender, EDataFlow, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

// ---------- state ----------

static ACTIVE: AtomicBool = AtomicBool::new(true);
static EMITTED_THUMB: AtomicU64 = AtomicU64::new(0);
static SELECTED: Mutex<Option<String>> = Mutex::new(None);
static TX: OnceLock<Mutex<Sender<Msg>>> = OnceLock::new();
static MGR: OnceLock<Manager_> = OnceLock::new();

enum Msg {
    /// Some session property changed: re-read and emit.
    Refresh,
    /// The session list changed: re-subscribe, then refresh.
    Resub,
}

#[derive(Serialize, Default)]
pub struct MediaState {
    has_session: bool,
    playing: bool,
    title: String,
    artist: String,
    album: String,
    session_id: String,
    app: String,
    /// Position at the moment of reading (already advanced by the time since the player reported it).
    position_ms: u64,
    duration_ms: u64,
    /// Changes whenever the cover changes; the page echoes it back as `known`.
    thumb_id: u64,
    /// data-URL, only sent when `thumb_id` differs from what the page already has.
    thumbnail: Option<String>,
    /// Average cover colour as #rrggbb.
    accent: Option<String>,
}

#[derive(Serialize)]
pub struct SessionInfo {
    id: String,
    app: String,
    title: String,
    artist: String,
    playing: bool,
    selected: bool,
}

#[derive(Default)]
struct ThumbCache {
    key: String,
    id: u64,
    data: Option<String>,
    accent: Option<String>,
    hash: u64,
}
static THUMB: Mutex<ThumbCache> = Mutex::new(ThumbCache { key: String::new(), id: 0, data: None, accent: None, hash: 0 });
/// Set by MediaPropertiesChanged and the retry timer: re-read the thumbnail on the next refresh.
static FORCE_THUMB: AtomicBool = AtomicBool::new(false);

fn com() {
    // Ignore the result: S_FALSE / RPC_E_CHANGED_MODE just mean COM is already initialised on this thread.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
}

fn mgr() -> windows::core::Result<Manager_> {
    com();
    if let Some(m) = MGR.get() {
        return Ok(m.clone());
    }
    let m = Manager_::RequestAsync()?.get()?;
    let _ = MGR.set(m.clone());
    Ok(m)
}

fn notify(m: Msg) {
    if let Some(tx) = TX.get() {
        let _ = tx.lock().map(|t| t.send(m));
    }
}

/// Pause/resume event emission (the page calls this on pill:hide / pill:show). No timer thread exists;
/// while paused the worker only drains its channel.
pub fn set_active(on: bool) {
    ACTIVE.store(on, Ordering::Relaxed);
    wake();
    if on {
        notify(Msg::Refresh);
    }
}

static PLAYING: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<()>, std::sync::Condvar) = (Mutex::new(()), std::sync::Condvar::new());

fn wake() {
    WAKE.1.notify_all();
}

/// True while the pill is visible and the selected session is playing (audio.rs captures only then).
pub fn want_capture() -> bool {
    ACTIVE.load(Ordering::Relaxed) && PLAYING.load(Ordering::Relaxed) && enabled()
}

/// Settings `module.media` (the Media player module) and `live.music` (the music widget + reactive edge) both on.
fn enabled() -> bool {
    crate::settings::get_bool("module.media", true) && crate::settings::get_bool("live.music", true)
}

/// settings.rs calls this after every patch: wake the worker / the audio thread when a switch that gates them changed.
pub fn on_settings(patch: &serde_json::Map<String, serde_json::Value>) {
    if ["module.media", "live.music", "audio.reactive"].iter().any(|k| patch.contains_key(*k)) {
        wake();
        if enabled() {
            notify(Msg::Refresh);
        }
    }
}

/// Block until `want_capture` may have changed, or `timeout` passes.
pub fn wait_change(timeout: Duration) {
    let g = WAKE.0.lock().unwrap_or_else(|e| e.into_inner());
    let _ = WAKE.1.wait_timeout(g, timeout);
}

#[tauri::command]
pub fn media_pause_updates() {
    set_active(false);
}

#[tauri::command]
pub fn media_resume_updates() {
    set_active(true);
}

// ---------- reading ----------

/// The selected session if it still exists, else the system's current one.
fn pick(m: &Manager_) -> windows::core::Result<Session> {
    let sel = SELECTED.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some(id) = sel {
        for s in m.GetSessions()? {
            if s.SourceAppUserModelId()?.to_string() == id {
                return Ok(s);
            }
        }
    }
    m.GetCurrentSession()
}

/// "Spotify.exe" -> "Spotify"; "Microsoft.ZuneMusic_8wekyb3d8bbwe!App" -> "App".
fn app_name(aumid: &str) -> String {
    let s = aumid.rsplit('!').next().unwrap_or(aumid);
    let s = s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s);
    s.rsplit('.').next().unwrap_or(s).to_string()
}

/// Average of a 16x16 downscale, lifted if too dark to read as an accent.
fn accent_of(stream: &IRandomAccessStream) -> Option<String> {
    stream.Seek(0).ok()?;
    let dec = BitmapDecoder::CreateAsync(stream).ok()?.get().ok()?;
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(16).ok()?;
    t.SetScaledHeight(16).ok()?;
    let px = dec
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Straight,
            &t,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?
        .DetachPixelData()
        .ok()?;
    let n = (px.len() / 4) as u64;
    if n == 0 {
        return None;
    }
    let (mut b, mut g, mut r) = (0u64, 0u64, 0u64);
    for p in px.chunks_exact(4) {
        b += p[0] as u64;
        g += p[1] as u64;
        r += p[2] as u64;
    }
    let (mut r, mut g, mut b) = ((r / n) as f32, (g / n) as f32, (b / n) as f32);
    let m = r.max(g).max(b);
    if m > 0.0 && m < 110.0 {
        let k = 110.0 / m;
        (r, g, b) = (r * k, g * k, b * k);
    }
    Some(format!("#{:02x}{:02x}{:02x}", r as u8, g as u8, b as u8))
}

struct Thumb {
    url: String,
    accent: Option<String>,
    hash: u64,
}

const MAX_THUMB: usize = 8 * 1024 * 1024;

/// Delays (ms) of the extra thumbnail re-reads after a track change; none without a title yet.
fn retry_delays(has_title: bool) -> &'static [u64] {
    if has_title { &[400, 1200] } else { &[] }
}

/// Trust a usable declared mime, otherwise sniff the magic bytes.
fn pick_mime(declared: &str, b: &[u8]) -> &'static str {
    match declared {
        "image/png" => return "image/png",
        "image/jpeg" | "image/jpg" => return "image/jpeg",
        "image/gif" => return "image/gif",
        "image/bmp" => return "image/bmp",
        "image/webp" => return "image/webp",
        _ => {}
    }
    if b.starts_with(&[0xFF, 0xD8]) { "image/jpeg" }
    else if b.starts_with(b"GIF8") { "image/gif" }
    else if b.starts_with(b"BM") { "image/bmp" }
    else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" { "image/webp" }
    else { "image/png" }
}

fn data_url(declared_mime: &str, b: &[u8]) -> String {
    format!("data:{};base64,{}", pick_mime(declared_mime, b), base64::engine::general_purpose::STANDARD.encode(b))
}

fn load_thumb(props: &Props) -> windows::core::Result<Thumb> {
    let stream = props.Thumbnail()?.OpenReadAsync()?.get()?;
    let mime = stream.ContentType().map(|m| m.to_string()).unwrap_or_default();
    // Size() can be 0 / unknown for some publishers, so read in chunks until the stream ends.
    let reader = DataReader::CreateDataReader(&stream)?;
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let n = reader.LoadAsync(64 * 1024)?.get()? as usize;
        if n == 0 {
            break;
        }
        let at = buf.len();
        buf.resize(at + n, 0);
        reader.ReadBytes(&mut buf[at..])?;
        if buf.len() > MAX_THUMB {
            eprintln!("[media] thumbnail too large: >{} bytes, mime '{}'", MAX_THUMB, mime);
            return Err(windows::core::Error::empty());
        }
    }
    if buf.is_empty() {
        eprintln!("[media] thumbnail stream empty, declared size {:?}, mime '{}'", stream.Size().ok(), mime);
        return Err(windows::core::Error::empty());
    }
    let accent = accent_of(&stream.cast::<IRandomAccessStream>()?);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&buf, &mut h);
    Ok(Thumb { url: data_url(&mime, &buf), accent, hash: std::hash::Hasher::finish(&h) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_only_with_title() {
        assert_eq!(retry_delays(true), &[400, 1200]);
        assert!(retry_delays(false).is_empty());
    }

    #[test]
    fn data_url_mime() {
        assert!(data_url("image/jpeg", b"x").starts_with("data:image/jpeg;base64,"));
        // empty or unknown declared type: sniffed from magic bytes, png as last resort
        assert!(data_url("", &[0xFF, 0xD8, 1]).starts_with("data:image/jpeg;base64,"));
        assert!(data_url("application/octet-stream", b"RIFF\0\0\0\0WEBPx").starts_with("data:image/webp;base64,"));
        assert!(data_url("", b"zz").starts_with("data:image/png;base64,"));
        assert_eq!(data_url("image/png", b"hi"), "data:image/png;base64,aGk=");
    }
}

fn read_media(known: Option<u64>) -> windows::core::Result<MediaState> {
    let s = pick(&mgr()?)?;
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let info = s.GetPlaybackInfo()?;
    let playing = info.PlaybackStatus()? == Status::Playing;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let album = props.AlbumTitle().map(|a| a.to_string()).unwrap_or_default();
    let session_id = s.SourceAppUserModelId()?.to_string();

    // Timeline: position is as of LastUpdatedTime, so advance it while playing.
    let (mut position_ms, mut duration_ms) = (0u64, 0u64);
    if let Ok(t) = s.GetTimelineProperties() {
        let start = t.StartTime().map(|d| d.Duration).unwrap_or(0);
        let end = t.EndTime().map(|d| d.Duration).unwrap_or(0);
        let pos = t.Position().map(|d| d.Duration).unwrap_or(0);
        duration_ms = ((end - start).max(0) / 10_000) as u64;
        position_ms = ((pos - start).max(0) / 10_000) as u64;
        if playing {
            if let (Ok(upd), Ok(now)) = (t.LastUpdatedTime(), SystemTime::now().duration_since(UNIX_EPOCH)) {
                // DateTime ticks are 100 ns since 1601-01-01; 116444736000000000 ticks to the Unix epoch.
                let upd_ms = (upd.UniversalTime - 116_444_736_000_000_000) / 10_000;
                let late = (now.as_millis() as i64 - upd_ms).clamp(0, 6 * 3600 * 1000);
                position_ms += late as u64;
            }
        }
        if duration_ms > 0 {
            position_ms = position_ms.min(duration_ms);
        }
    }

    let mut c = THUMB.lock().unwrap_or_else(|e| e.into_inner());
    let key = format!("{session_id}\u{1}{title}\u{1}{artist}\u{1}{album}");
    if c.key != key {
        c.key = key;
        c.data = None;
        c.accent = None;
        c.hash = 0;
        c.id += 1;
    }
    // Re-read when we have no cover yet or a properties event / retry asked for it. The first image can be
    // a placeholder (app icon) that the real artwork replaces later under the SAME title, so never trust the cache alone.
    if FORCE_THUMB.swap(false, Ordering::Relaxed) || c.data.is_none() {
        match load_thumb(&props) {
            Ok(t) if t.hash != c.hash => {
                c.data = Some(t.url);
                c.accent = t.accent;
                c.hash = t.hash;
                c.id += 1;
            }
            Ok(_) => {}
            Err(e) => eprintln!("[media] thumbnail read failed: {e}"),
        }
    }
    let thumbnail = if known == Some(c.id) { None } else { c.data.clone() };
    Ok(MediaState {
        has_session: true,
        playing,
        title,
        artist,
        album,
        app: app_name(&session_id),
        session_id,
        position_ms,
        duration_ms,
        thumb_id: c.id,
        thumbnail,
        accent: c.accent.clone(),
    })
}

// ---------- worker: subscriptions + emit ----------

struct Sub {
    s: Session,
    tokens: [EventRegistrationToken; 3],
}

fn resub(m: &Manager_, tx: &Sender<Msg>, subs: &mut Vec<Sub>) {
    for sub in subs.drain(..) {
        let _ = sub.s.RemoveMediaPropertiesChanged(sub.tokens[0]);
        let _ = sub.s.RemovePlaybackInfoChanged(sub.tokens[1]);
        let _ = sub.s.RemoveTimelinePropertiesChanged(sub.tokens[2]);
    }
    let Ok(list) = m.GetSessions() else { return };
    for s in list {
        let (a, b, c) = (tx.clone(), tx.clone(), tx.clone());
        let t0 = s.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
            // Artwork may have arrived or changed: force a thumbnail re-read on the next refresh.
            FORCE_THUMB.store(true, Ordering::Relaxed);
            let _ = a.send(Msg::Refresh);
            Ok(())
        }));
        let t1 = s.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
            let _ = b.send(Msg::Refresh);
            Ok(())
        }));
        let t2 = s.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
            let _ = c.send(Msg::Refresh);
            Ok(())
        }));
        if let (Ok(t0), Ok(t1), Ok(t2)) = (t0, t1, t2) {
            subs.push(Sub { s, tokens: [t0, t1, t2] });
        }
    }
}

fn worker(app: tauri::AppHandle, tx: Sender<Msg>, rx: Receiver<Msg>) {
    let Ok(m) = mgr() else { return };
    let (a, b) = (tx.clone(), tx.clone());
    let _ = m.SessionsChanged(&TypedEventHandler::new(move |_, _| {
        let _ = a.send(Msg::Resub);
        Ok(())
    }));
    let _ = m.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
        let _ = b.send(Msg::Resub);
        Ok(())
    }));
    let mut subs = Vec::new();
    let mut last_key = String::new();
    resub(&m, &tx, &mut subs);
    while let Ok(first) = rx.recv() {
        // Coalesce bursts (players fire several events per track change).
        let mut resubscribe = matches!(first, Msg::Resub);
        while let Ok(n) = rx.recv_timeout(Duration::from_millis(80)) {
            resubscribe |= matches!(n, Msg::Resub);
        }
        if resubscribe {
            resub(&m, &tx, &mut subs);
        }
        // module.media off: no reads, no media:changed events, no audio capture (live.music only gates the widget and audio)
        if !ACTIVE.load(Ordering::Relaxed) || !crate::settings::get_bool("module.media", true) {
            continue;
        }
        let st = read_media(Some(EMITTED_THUMB.load(Ordering::Relaxed))).unwrap_or_default();
        if PLAYING.swap(st.playing, Ordering::Relaxed) != st.playing {
            wake();
        }
        EMITTED_THUMB.store(st.thumb_id, Ordering::Relaxed);
        let _ = bus::emit(&app, Target::Label("pill"), "media:changed", &st);
        // New track: browsers publish the artwork after the title (until then Windows may hand out the
        // app icon), so re-read the thumbnail a couple of times.
        let key = format!("{}\u{1}{}\u{1}{}", st.session_id, st.title, st.artist);
        if key != last_key {
            last_key = key;
            let delays = retry_delays(!st.title.is_empty());
            if !delays.is_empty() {
                std::thread::spawn(move || {
                    for d in delays {
                        std::thread::sleep(Duration::from_millis(*d));
                        FORCE_THUMB.store(true, Ordering::Relaxed);
                        notify(Msg::Refresh);
                    }
                });
            }
        }
    }
}

pub fn init(app: &tauri::App) {
    let (tx, rx) = channel();
    let _ = TX.set(Mutex::new(tx.clone()));
    let handle = app.handle().clone();
    std::thread::spawn(move || worker(handle, tx, rx));
}

// ---------- commands ----------

// async: runs on the runtime pool, so the blocking WinRT .get() never stalls the UI thread.
#[tauri::command]
pub async fn media_state(known: Option<u64>) -> MediaState {
    let st = read_media(known).unwrap_or_default();
    // The page now holds this cover; later events can omit it.
    if st.thumbnail.is_some() {
        EMITTED_THUMB.store(st.thumb_id, Ordering::Relaxed);
    }
    st
}

#[tauri::command]
pub async fn media_sessions() -> Vec<SessionInfo> {
    let run = || -> windows::core::Result<Vec<SessionInfo>> {
        let m = mgr()?;
        let cur = pick(&m).and_then(|s| s.SourceAppUserModelId()).map(|h| h.to_string()).unwrap_or_default();
        let mut out = Vec::new();
        for s in m.GetSessions()? {
            let id = s.SourceAppUserModelId()?.to_string();
            let (title, artist) = s
                .TryGetMediaPropertiesAsync()
                .and_then(|o| o.get())
                .map(|p| (p.Title().map(|t| t.to_string()).unwrap_or_default(), p.Artist().map(|t| t.to_string()).unwrap_or_default()))
                .unwrap_or_default();
            let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|p| p == Status::Playing).unwrap_or(false);
            out.push(SessionInfo { app: app_name(&id), selected: id == cur, id, title, artist, playing });
        }
        Ok(out)
    };
    run().unwrap_or_default()
}

/// Pick a player by session id; None/empty goes back to the system's current session.
#[tauri::command]
pub async fn media_select(id: Option<String>) {
    *SELECTED.lock().unwrap_or_else(|e| e.into_inner()) = id.filter(|s| !s.is_empty());
    notify(Msg::Refresh);
}

/// action: "play_pause" | "next" | "prev"
#[tauri::command]
pub async fn media_control(action: String) {
    let _ = (|| -> windows::core::Result<()> {
        let s = pick(&mgr()?)?;
        match action.as_str() {
            "play_pause" => s.TryTogglePlayPauseAsync()?.get()?,
            "next" => s.TrySkipNextAsync()?.get()?,
            "prev" => s.TrySkipPreviousAsync()?.get()?,
            _ => false,
        };
        Ok(())
    })();
}

#[tauri::command]
pub async fn media_seek(ms: u64) {
    let _ = (|| -> windows::core::Result<()> {
        let s = pick(&mgr()?)?;
        let start = s.GetTimelineProperties()?.StartTime()?.Duration;
        s.TryChangePlaybackPositionAsync(start + ms as i64 * 10_000)?.get()?;
        Ok(())
    })();
    notify(Msg::Refresh);
}

// ---------- master volume (Core Audio) ----------

fn endpoint() -> windows::core::Result<IAudioEndpointVolume> {
    endpoint_for(eRender, "sound.volumeDevice")
}

/// Volume endpoint of `flow`: the device picked in Settings > Sound (`key` = sound.volumeDevice / sound.micDevice, "" = the
/// Windows default), falling back to the default one when the picked device is gone. system.rs uses it for the microphone.
pub fn endpoint_for(flow: EDataFlow, key: &str) -> windows::core::Result<IAudioEndpointVolume> {
    use windows::core::{HSTRING, PCWSTR};
    com();
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let id = crate::settings::get_str(key, "");
        if !id.is_empty() {
            let h = HSTRING::from(id);
            if let Ok(ep) = en.GetDevice(PCWSTR(h.as_ptr())).and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)) {
                return Ok(ep);
            }
        }
        let dev = en.GetDefaultAudioEndpoint(flow, eConsole)?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
    }
}

#[derive(Serialize, Default)]
pub struct Volume {
    volume: u32,
    muted: bool,
}

#[tauri::command]
pub async fn media_volume_get() -> Volume {
    endpoint()
        .and_then(|ep| unsafe {
            Ok(Volume { volume: (ep.GetMasterVolumeLevelScalar()? * 100.0).round() as u32, muted: ep.GetMute()?.as_bool() })
        })
        .unwrap_or_default()
}

#[tauri::command]
pub async fn media_volume_set(v: u32) {
    if let Ok(ep) = endpoint() {
        unsafe {
            let _ = ep.SetMasterVolumeLevelScalar(v.min(100) as f32 / 100.0, std::ptr::null());
        }
    }
}

#[tauri::command]
pub async fn media_mute(muted: bool) {
    if let Ok(ep) = endpoint() {
        unsafe {
            let _ = ep.SetMute(BOOL::from(muted), std::ptr::null());
        }
    }
}

// ---------- output devices (Core Audio) ----------

#[derive(Serialize)]
pub struct Device {
    id: String,
    name: String,
    /// The current default (console) render endpoint.
    default: bool,
}

fn pwstr_string(p: windows::core::PWSTR) -> String {
    unsafe {
        let s = p.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const std::ffi::c_void));
        s
    }
}

/// Active render endpoints with their friendly names; the default one is flagged.
fn devices() -> windows::core::Result<Vec<Device>> {
    devices_of(eRender)
}

fn devices_of(flow: EDataFlow) -> windows::core::Result<Vec<Device>> {
    use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
    use windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE;
    use windows::Win32::System::Com::STGM_READ;
    com();
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let def = en.GetDefaultAudioEndpoint(flow, eConsole).and_then(|d| d.GetId()).map(pwstr_string).unwrap_or_default();
        let col = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let mut out = Vec::new();
        for i in 0..col.GetCount()? {
            let d = col.Item(i)?;
            let id = pwstr_string(d.GetId()?);
            let name = d.OpenPropertyStore(STGM_READ).and_then(|s| s.GetValue(&PKEY_Device_FriendlyName)).map(|v| v.to_string()).unwrap_or_default();
            out.push(Device { default: id == def, name: if name.is_empty() { id.clone() } else { name }, id });
        }
        Ok(out)
    }
}

// RISK (read before touching): Windows has NO public API to change the default playback device. The route used by
// SoundSwitch, EarTrumpet, nircmd and others is the undocumented IPolicyConfig COM interface of the class
// CPolicyConfigClient. It is declared by hand here: the IID and the vtable order (IUnknown + 12 methods, slot 11 =
// SetDefaultEndpoint) are the ones those projects use on Windows 10/11. Microsoft may change it without notice, and a wrong
// vtable would be a crash in our own process (not detectable at compile time; this file has only been compile-checked).
// Mitigations: only ids of currently active render endpoints are accepted, nothing is called until the user picks a
// device, and the setting `audio.policyConfig` = false switches the call off (then ms-settings:sound opens instead).
#[allow(non_snake_case, dead_code)]
mod policy {
    use std::ffi::c_void;
    use windows::core::{HRESULT, PCWSTR};
    use windows::Win32::Media::Audio::ERole;

    #[windows::core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
    pub unsafe trait IPolicyConfig: windows::core::IUnknown {
        unsafe fn GetMixFormat(&self, d: PCWSTR, f: *mut *mut c_void) -> HRESULT;
        unsafe fn GetDeviceFormat(&self, d: PCWSTR, default: i32, f: *mut *mut c_void) -> HRESULT;
        unsafe fn ResetDeviceFormat(&self, d: PCWSTR) -> HRESULT;
        unsafe fn SetDeviceFormat(&self, d: PCWSTR, a: *mut c_void, b: *mut c_void) -> HRESULT;
        unsafe fn GetProcessingPeriod(&self, d: PCWSTR, default: i32, a: *mut i64, b: *mut i64) -> HRESULT;
        unsafe fn SetProcessingPeriod(&self, d: PCWSTR, a: *mut i64) -> HRESULT;
        unsafe fn GetShareMode(&self, d: PCWSTR, m: *mut c_void) -> HRESULT;
        unsafe fn SetShareMode(&self, d: PCWSTR, m: *mut c_void) -> HRESULT;
        unsafe fn GetPropertyValue(&self, d: PCWSTR, fx: i32, key: *const c_void, v: *mut c_void) -> HRESULT;
        unsafe fn SetPropertyValue(&self, d: PCWSTR, fx: i32, key: *const c_void, v: *mut c_void) -> HRESULT;
        pub unsafe fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: ERole) -> HRESULT;
        unsafe fn SetEndpointVisibility(&self, d: PCWSTR, visible: i32) -> HRESULT;
    }

    /// CPolicyConfigClient
    pub const CLSID: windows::core::GUID = windows::core::GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
}

fn set_default(id: &str) -> windows::core::Result<()> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Media::Audio::{eCommunications, eMultimedia};
    com();
    unsafe {
        let pc: policy::IPolicyConfig = CoCreateInstance(&policy::CLSID, None, CLSCTX_ALL)?;
        let h = HSTRING::from(id);
        // the Sound control panel sets all three roles
        for role in [eConsole, eMultimedia, eCommunications] {
            pc.SetDefaultEndpoint(PCWSTR(h.as_ptr()), role).ok()?;
        }
    }
    Ok(())
}

fn open_sound_settings() {
    use windows::core::{HSTRING, PCWSTR};
    unsafe {
        let _ = windows::Win32::UI::Shell::ShellExecuteW(
            windows::Win32::Foundation::HWND::default(),
            &HSTRING::from("open"),
            &HSTRING::from("ms-settings:sound"),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

#[tauri::command]
pub async fn media_devices() -> Vec<Device> {
    devices().unwrap_or_default()
}

/// Active microphones (capture endpoints) for Settings > Sound.
#[tauri::command]
pub async fn media_capture_devices() -> Vec<Device> {
    devices_of(eCapture).unwrap_or_default()
}

/// Returns "ok" (default device changed, or it already was), "settings" (the change was not done in-app, the Windows
/// sound settings were opened instead; the page explains why) or "unknown" (id is not an active render endpoint).
#[tauri::command]
pub async fn media_set_default_device(id: String) -> &'static str {
    let Some(d) = devices().ok().and_then(|v| v.into_iter().find(|d| d.id == id)) else { return "unknown" };
    if d.default {
        return "ok";
    }
    if crate::settings::get_bool("audio.policyConfig", false) && set_default(&d.id).is_ok() {
        return "ok";
    }
    open_sound_settings();
    "settings"
}

// ---------- battery ----------

#[derive(Serialize)]
pub struct Battery {
    /// false on a desktop PC without a battery.
    has_battery: bool,
    percent: u8,
    charging: bool,
    ac: bool,
}

#[tauri::command]
pub fn battery_state() -> Battery {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return Battery { has_battery: false, percent: 0, charging: false, ac: true };
    }
    // BatteryFlag: 128 = no system battery, 255 = unknown; 8 = charging.
    Battery {
        has_battery: s.BatteryFlag & 128 == 0 && s.BatteryFlag != 255,
        percent: if s.BatteryLifePercent <= 100 { s.BatteryLifePercent } else { 0 },
        charging: s.BatteryFlag & 8 != 0,
        ac: s.ACLineStatus == 1,
    }
}
