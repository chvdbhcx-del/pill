// Voice to text (dictation). Owner: voice agent. User request: "делай все оставшееся" (voice-to-text for the Pill replica).
// Flow: Ctrl+Alt+D / chat mic / command bar / pill header button -> start() -> ONE short-lived thread records the DEFAULT
// communications microphone (WASAPI shared, event-driven, converted to 16 kHz mono 16-bit), the same thread then uploads a WAV
// built in memory and puts the transcript on the clipboard (the clipboard listener files it into the history with its normal
// privacy filters), optionally Ctrl+V into the window that had the caret. When idle there is no thread, no audio client, no socket.
// PRIVACY: audio exists only in RAM during the session, is never written to disk or logged; the key is never logged or echoed.
// The ONLY network path is net::request(Feature::Voice): gated by `voice.enabled`, host = exactly the host of `voice.endpoint`.
// Settings: voice.enabled (false), voice.provider (groq|gemini|openai|custom), voice.endpoint, voice.model, voice.language (auto|ru|en),
// voice.paste (false), voice.autostop (false), voice.mode (toggle|hold), voice.key.<provider> (DPAPI blob, same mechanism as ai.apiKeyEnc), hotkey.voice.
// Commands: voice_toggle voice_start voice_stop voice_cancel voice_state voice_test_mic voice_test_service voice_set_key voice_has_key.
// Events: voice:state {state: idle|recording|transcribing|done|error, startedAt?, text?, code?, message?}, voice:level {rms, level, test?}.
use crate::ui::bus::{self, Target};
use crate::net::{self, Abort, Feature, Opts};
use crate::settings;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Listener, Manager};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL};
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eConsole, ERole, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_EVENTCALLBACK,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcessId, WaitForSingleObject};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

pub const DEFAULT_ENDPOINT: &str = "https://api.groq.com/openai/v1";
const DEFAULT_MODEL: &str = "whisper-large-v3-turbo";
const RATE: u32 = 16_000;
const MAX_SECS: u32 = 120;
const MIN_SAMPLES: usize = RATE as usize / 2; // 0.5 s
const SPEECH_RMS: f32 = 0.012; // about -38 dBFS: above typical room noise
const HEARD_RMS: f32 = 0.003; // below this for the whole take = the microphone gave nothing
const AUTOSTOP_MS: f32 = 4000.0;
const LEVEL_EVERY: Duration = Duration::from_millis(40); // 25 Hz

/// The endpoint the upload may go to (net.rs allow-list uses the same function).
pub fn endpoint() -> String {
    let e = settings::get_str("voice.endpoint", "");
    if e.trim().is_empty() { DEFAULT_ENDPOINT.to_string() } else { e }
}

// ---------------------------------------------------------------------------------------------------------------
// Pure parts (unit-tested)
// ---------------------------------------------------------------------------------------------------------------

/// 44-byte RIFF/WAVE header + 16-bit mono PCM.
pub fn wav16(pcm: &[i16], rate: u32) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + pcm.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

/// Streaming area-average resampler (a box filter doubles as the anti-alias low-pass when going 48 kHz -> 16 kHz).
pub struct Resampler {
    ratio: f64, // input samples per output sample
    acc: f64,
    w: f64,
}

impl Resampler {
    pub fn new(from: f64, to: f64) -> Self {
        Resampler { ratio: (from / to).max(1e-6), acc: 0.0, w: 0.0 }
    }
    pub fn push(&mut self, x: f32, out: &mut Vec<f32>) {
        let mut left = 1.0f64;
        while left > 1e-9 {
            let need = self.ratio - self.w;
            if left >= need - 1e-9 {
                self.acc += x as f64 * need;
                out.push((self.acc / self.ratio) as f32);
                left -= need;
                self.acc = 0.0;
                self.w = 0.0;
            } else {
                self.acc += x as f64 * left;
                self.w += left;
                left = 0.0;
            }
        }
    }
}

pub fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// 0..1 on a -50..0 dBFS scale, for the meter.
pub fn level_of(rms: f32) -> f32 {
    ((20.0 * rms.max(1e-6).log10() + 50.0) / 50.0).clamp(0.0, 1.0)
}

/// Auto-stop after `stop_ms` of quiet that follows speech (a take that never had speech is stopped by hand).
pub struct Silence {
    thr: f32,
    stop_ms: f32,
    quiet: f32,
    heard: bool,
}

impl Silence {
    pub fn new(thr: f32, stop_ms: f32) -> Self {
        Silence { thr, stop_ms, quiet: 0.0, heard: false }
    }
    /// One level window (`dt_ms` long, `rms` loud). True = stop now.
    pub fn feed(&mut self, rms: f32, dt_ms: f32) -> bool {
        if rms >= self.thr {
            self.heard = true;
            self.quiet = 0.0;
        } else if self.heard {
            self.quiet += dt_ms;
        }
        self.heard && self.quiet >= self.stop_ms
    }
}

/// multipart/form-data body: text fields first, then one file part.
pub fn multipart(boundary: &str, fields: &[(&str, &str)], file_name: &str, file_ct: &str, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(data.len() + 512);
    for (k, v) in fields {
        b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n").as_bytes());
    }
    b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\nContent-Type: {file_ct}\r\n\r\n").as_bytes());
    b.extend_from_slice(data);
    b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    b
}

/// Host part of a URL (what the settings page shows as "requests go only to").
pub fn host_of(url: &str) -> Option<String> {
    net::parse_url(url).ok().map(|u| u.host)
}

fn clip_chars(s: &str, n: usize) -> String {
    let s: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() > n { s.chars().take(n).collect::<String>() + "…" } else { s }
}

/// Error text for the card: never the key, one line, short.
fn redact(msg: &str, key: Option<&str>) -> String {
    let mut m = msg.to_string();
    if let Some(k) = key.map(str::trim).filter(|k| k.len() >= 6) {
        m = m.replace(k, "***");
    }
    clip_chars(&m, 160)
}

/// Answer of /audio/transcriptions (`{"text": ...}` or plain text) or, for Gemini, of /chat/completions.
fn parse_transcript(status: u16, body: &[u8], gemini: bool, key: Option<&str>) -> Result<String, VErr> {
    let text = String::from_utf8_lossy(body);
    if !(200..300).contains(&status) {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                let v = if let Value::Array(a) = &v { a.first().cloned().unwrap_or(Value::Null) } else { v };
                v.pointer("/error/message").or_else(|| v.get("error")).and_then(|m| m.as_str()).map(str::to_string)
            })
            .unwrap_or_else(|| text.to_string());
        return Err(VErr::new("http", format!("HTTP {status}: {}", redact(&detail, key))));
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => {
            let t = if gemini { v.pointer("/choices/0/message/content").and_then(Value::as_str) } else { v.get("text").and_then(Value::as_str) };
            t.map(|t| t.trim().to_string()).ok_or_else(|| VErr::new("http", "the service answered without text".into()))
        }
        Err(_) => Ok(text.trim().to_string()), // servers that honour response_format=text
    }
}

#[derive(Debug)]
pub struct VErr {
    code: &'static str,
    msg: String,
}
impl VErr {
    fn new(code: &'static str, msg: String) -> Self {
        VErr { code, msg }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Config + key
// ---------------------------------------------------------------------------------------------------------------

struct Cfg {
    endpoint: String, // base URL without trailing slash
    model: String,
    lang: String,
    key: Option<String>,
    gemini: bool,
}

fn provider() -> String {
    let p = settings::get_str("voice.provider", "groq");
    if p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !p.is_empty() { p } else { "custom".into() }
}

/// Voice key of the provider, else the AI key when it belongs to the same host (one Groq/Gemini key serves both).
fn key_for(provider: &str, endpoint: &str) -> Result<Option<String>, VErr> {
    let own = settings::get_str(&format!("voice.key.{provider}"), "");
    if !own.trim().is_empty() {
        return net::secrets::unprotect(&own).map(Some).map_err(|e| VErr::new("no_key", e));
    }
    let ai = settings::get_str("ai.apiKeyEnc", "");
    let same_host = host_of(endpoint).is_some_and(|h| host_of(&settings::get_str("ai.baseUrl", "")).is_some_and(|a| a == h));
    if same_host && !ai.trim().is_empty() {
        return net::secrets::unprotect(&ai).map(Some).map_err(|e| VErr::new("no_key", e));
    }
    Ok(None)
}

fn load_cfg() -> Result<Cfg, VErr> {
    if !net::enabled(Feature::Voice) {
        return Err(VErr::new("off", "Voice to text is switched off (enable it in Settings > Voice to text).".into()));
    }
    let endpoint = net::check_ai_base(&endpoint()).map_err(|e| VErr::new("no_endpoint", e))?;
    let u = net::parse_url(&endpoint).map_err(|e| VErr::new("no_endpoint", e))?;
    let key = key_for(&provider(), &endpoint)?;
    if key.is_none() && !net::is_loopback(&u.host) {
        return Err(VErr::new("no_key", format!("No API key for {}. Add it in Settings > Voice to text.", u.host)));
    }
    let gemini = u.host == "generativelanguage.googleapis.com";
    let m = settings::get_str("voice.model", "");
    let model = if !m.trim().is_empty() { m.trim().to_string() } else if gemini { "gemini-2.5-flash".into() } else { DEFAULT_MODEL.into() };
    let lang = settings::get_str("voice.language", "auto");
    let lang = if matches!(lang.as_str(), "ru" | "en") { lang } else { "auto".into() };
    Ok(Cfg { endpoint, model, lang, key, gemini })
}

fn boundary() -> String {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    format!("----PillVoice{n:x}")
}

/// Upload one WAV, return the transcript. The request goes through net.rs only.
fn transcribe(wav: &[u8], cfg: &Cfg, abort: &Abort) -> Result<String, VErr> {
    let auth = cfg.key.as_ref().map(|k| format!("Bearer {}", k.trim()));
    let mut hdrs: Vec<(&str, &str)> = vec![("Accept", "application/json")];
    if let Some(a) = &auth {
        hdrs.push(("Authorization", a.as_str()));
    }
    let (url, body, ct);
    if cfg.gemini {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let hint = if cfg.lang == "auto" { String::new() } else { format!(" The speech is in {}.", if cfg.lang == "ru" { "Russian" } else { "English" }) };
        let b = json!({ "model": cfg.model, "messages": [{ "role": "user", "content": [
            { "type": "text", "text": format!("Transcribe this audio verbatim. Reply with the transcript only, no comments.{hint}") },
            { "type": "input_audio", "input_audio": { "data": STANDARD.encode(wav), "format": "wav" } } ] }] });
        url = format!("{}/chat/completions", cfg.endpoint);
        body = serde_json::to_vec(&b).unwrap_or_default();
        ct = "application/json".to_string();
    } else {
        let mut fields: Vec<(&str, &str)> = vec![("model", cfg.model.as_str()), ("response_format", "json")];
        if cfg.lang != "auto" {
            fields.push(("language", cfg.lang.as_str()));
        }
        let bd = boundary();
        url = if cfg.endpoint.ends_with("/audio/transcriptions") { cfg.endpoint.clone() } else { format!("{}/audio/transcriptions", cfg.endpoint) };
        body = multipart(&bd, &fields, "audio.wav", "audio/wav", wav);
        ct = format!("multipart/form-data; boundary={bd}");
    }
    hdrs.push(("Content-Type", ct.as_str()));
    let r = net::request(
        &Opts { feature: Feature::Voice, method: "POST", url: &url, headers: &hdrs, body: Some(&body), max_body: 1 << 20, recv_ms: 60_000, decompress: false, abort: Some(abort) },
        None,
    )
    .map_err(|e| if e == "stopped" { VErr::new("cancelled", e) } else { VErr::new("net", redact(&e, cfg.key.as_deref())) })?;
    parse_transcript(r.status, &r.body, cfg.gemini, cfg.key.as_deref())
}

// ---------------------------------------------------------------------------------------------------------------
// WASAPI capture (default communications microphone, shared mode, event-driven)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
struct Fmt {
    channels: usize,
    rate: u32,
    bits: u16,
    float: bool,
}

/// WAVEFORMATEX / WAVEFORMATEXTENSIBLE bytes -> sample layout.
fn parse_format(b: &[u8]) -> Option<Fmt> {
    if b.len() < 18 {
        return None;
    }
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let tag = u16_at(0);
    let sub = if tag == 0xFFFE && b.len() >= 40 { u32::from_le_bytes([b[24], b[25], b[26], b[27]]) } else { tag as u32 };
    let float = match sub {
        1 => false,
        3 => true,
        _ => return None,
    };
    let bits = u16_at(14);
    let channels = u16_at(2) as usize;
    if channels == 0 || !(if float { bits == 32 } else { matches!(bits, 16 | 24 | 32) }) {
        return None;
    }
    Some(Fmt { channels, rate: u32::from_le_bytes([b[4], b[5], b[6], b[7]]), bits, float })
}

/// Interleaved bytes -> mono f32 (channels averaged), appended to `out`.
fn to_mono(data: &[u8], f: &Fmt, out: &mut Vec<f32>) {
    let bytes = f.bits as usize / 8;
    let sample = |s: &[u8]| -> f32 {
        match (f.float, f.bits) {
            (true, _) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (false, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (false, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
        }
    };
    for fr in data.chunks_exact(bytes * f.channels) {
        out.push(fr.chunks_exact(bytes).map(sample).sum::<f32>() / f.channels as f32);
    }
}

struct Ev(HANDLE);
impl Drop for Ev {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

struct Take {
    pcm: Vec<i16>,
    peak: f32,
}

#[derive(Clone, Copy)]
struct RecOpts {
    max_ms: u32,
    autostop: bool,
    test: bool, // levels go to every window and carry test:true
}

fn emit_level(app: &AppHandle, rms: f32, test: bool) {
    let p = json!({ "rms": (rms * 10000.0).round() / 10000.0, "level": (level_of(rms) * 100.0).round() / 100.0, "test": test });
    if test {
        let _ = bus::emit(&app, Target::All, "voice:level", p);
    } else {
        let _ = bus::emit(&app, Target::Label("pill"), "voice:level", p);
    }
}

fn mic_err(what: &str, e: windows::core::Error) -> VErr {
    VErr::new("mic", format!("{what}: {}", e.message().trim()))
}

/// One opened, started capture stream.
struct Cap {
    client: IAudioClient,
    cap: IAudioCaptureClient,
    ev: Ev,
    fmt: Fmt,
}

unsafe fn open_role(en: &IMMDeviceEnumerator, role: ERole) -> Result<Cap, VErr> {
    let mic = mic_err;
    let dev = en.GetDefaultAudioEndpoint(eCapture, role).map_err(|e| mic("no microphone found", e))?;
    let client: IAudioClient = dev
        .Activate(CLSCTX_ALL, None)
        .map_err(|e| mic("cannot open the microphone (Windows privacy settings: allow desktop apps to use the microphone)", e))?;
    let pf = client.GetMixFormat().map_err(|e| mic("mix format", e))?;
    let cb = u16::from_le_bytes([*(pf as *const u8).add(16), *(pf as *const u8).add(17)]) as usize;
    let fmt = parse_format(std::slice::from_raw_parts(pf as *const u8, 18 + cb));
    let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, 1_000_000, 0, pf, None);
    CoTaskMemFree(Some(pf as *const _));
    let Some(fmt) = fmt else { return Err(VErr::new("mic", "unsupported microphone format".into())) };
    init.map_err(|e| mic("cannot start the microphone", e))?;
    let ev = Ev(CreateEventW(None, false, false, PCWSTR::null()).map_err(|e| mic("event", e))?);
    client.SetEventHandle(ev.0).map_err(|e| mic("event", e))?;
    let cap: IAudioCaptureClient = client.GetService().map_err(|e| mic("capture service", e))?;
    client.Start().map_err(|e| mic("start", e))?;
    Ok(Cap { client, cap, ev, fmt })
}

/// The default "communications" microphone (what voice apps use), else the plain default one.
unsafe fn open_capture() -> Result<Cap, VErr> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| mic_err("audio", e))?;
    open_role(&en, eCommunications).or_else(|e| open_role(&en, eConsole).map_err(|_| e))
}

/// One line per take in %APPDATA%\pill\voice.log: why it ended and how long it was. Never any audio or text.
fn vlog(msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(crate::store::dir().join("voice.log")) {
        let _ = writeln!(f, "{} {msg}", crate::store::now_ms());
    }
}

/// Records until `ctl.stop`, `o.max_ms`, or (autostop) 4 s of quiet after speech. Blocking; call from a worker thread.
/// A microphone that is reconfigured or taken over mid-take (headset profile switch, another app opening it) is reopened and the
/// same take goes on; before this the stream error ended the take after about a second.
fn record(app: &AppHandle, ctl: &Ctl, o: RecOpts) -> Result<Take, VErr> {
    let mic = mic_err;
    unsafe {
        let mut c = open_capture()?;
        let mut rs = Resampler::new(c.fmt.rate as f64, RATE as f64);
        let mut reopened = 0u32;
        let (mut mono, mut rs_out): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
        let max_samples = (o.max_ms as usize * RATE as usize) / 1000;
        let mut pcm: Vec<i16> = Vec::with_capacity(RATE as usize * 8);
        let (mut sumsq, mut cnt, mut peak) = (0f64, 0usize, 0f32);
        let mut sil = o.autostop.then(|| Silence::new(SPEECH_RMS, AUTOSTOP_MS));
        let (t0, mut last) = (Instant::now(), Instant::now());
        let mut err: Option<VErr> = None;
        'main: loop {
            if ctl.stop.load(Ordering::SeqCst) || pcm.len() >= max_samples || t0.elapsed() > Duration::from_millis(o.max_ms as u64 + 500) {
                break;
            }
            WaitForSingleObject(c.ev.0, 100);
            let mut lost: Option<VErr> = None;
            loop {
                let n = match c.cap.GetNextPacketSize() {
                    Ok(n) => n,
                    Err(e) => { lost = Some(mic("the microphone disconnected", e)); break; }
                };
                if n == 0 {
                    break;
                }
                let (mut p, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if let Err(e) = c.cap.GetBuffer(&mut p, &mut frames, &mut flags, None, None) {
                    lost = Some(mic("read", e));
                    break;
                }
                mono.clear();
                if flags & 2 != 0 || p.is_null() {
                    mono.resize(frames as usize, 0.0); // AUDCLNT_BUFFERFLAGS_SILENT
                } else {
                    to_mono(std::slice::from_raw_parts(p, frames as usize * c.fmt.bits as usize / 8 * c.fmt.channels), &c.fmt, &mut mono);
                }
                let _ = c.cap.ReleaseBuffer(frames);
                for &x in &mono {
                    sumsq += (x as f64) * (x as f64);
                    cnt += 1;
                    rs.push(x, &mut rs_out);
                }
                pcm.extend(rs_out.drain(..).map(to_i16));
            }
            if let Some(e) = lost {
                // reopen and carry on with the same take (a short pause lets the device settle)
                let _ = c.client.Stop();
                reopened += 1;
                vlog(&format!("stream lost #{reopened} after {} ms: {}", t0.elapsed().as_millis(), e.msg));
                if reopened > 8 {
                    err = Some(e);
                    break 'main;
                }
                std::thread::sleep(Duration::from_millis(250));
                match open_capture() {
                    Ok(n) => {
                        c = n;
                        rs = Resampler::new(c.fmt.rate as f64, RATE as f64);
                    }
                    Err(e2) if reopened >= 8 => {
                        err = Some(e2);
                        break 'main;
                    }
                    Err(_) => {}
                }
                continue;
            }
            let dt = last.elapsed();
            if dt >= LEVEL_EVERY {
                last = Instant::now();
                let rms = if cnt > 0 { (sumsq / cnt as f64).sqrt() as f32 } else { 0.0 };
                (sumsq, cnt) = (0.0, 0);
                peak = peak.max(rms);
                emit_level(app, rms, o.test);
                if sil.as_mut().is_some_and(|s| s.feed(rms, dt.as_secs_f32() * 1000.0)) {
                    break;
                }
            }
        }
        let _ = c.client.Stop();
        if !o.test {
            let why = if ctl.stop.load(Ordering::SeqCst) { "stopped" } else if err.is_some() { "error" } else if pcm.len() >= max_samples { "max length" } else { "silence" };
            vlog(&format!("take ended ({why}) after {} ms, {} ms of audio, {reopened} reopen(s), peak {:.4}", t0.elapsed().as_millis(), pcm.len() * 1000 / RATE as usize, peak));
        }
        match err {
            Some(e) if pcm.len() < MIN_SAMPLES => Err(e),
            _ => Ok(Take { pcm, peak }),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Session state
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum St {
    Idle,
    Recording,
    Transcribing,
    Testing,
}

#[derive(Default)]
struct Ctl {
    stop: AtomicBool,
    cancel: AtomicBool,
    abort: Abort,
}

struct Sess {
    st: St,
    ctl: Option<Arc<Ctl>>,
    started_ms: i64,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static SESS: Mutex<Sess> = Mutex::new(Sess { st: St::Idle, ctl: None, started_ms: 0 });

fn sess() -> std::sync::MutexGuard<'static, Sess> {
    SESS.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit_state(app: &AppHandle, state: &str, extra: Value) {
    let mut p = json!({ "state": state });
    if let (Some(o), Value::Object(x)) = (p.as_object_mut(), extra) {
        o.extend(x);
    }
    let _ = bus::emit(&app, Target::All, "voice:state", p);
}

fn fail(app: &AppHandle, e: VErr) {
    emit_state(app, "error", json!({ "code": e.code, "message": e.msg }));
}

fn esc_key(app: &AppHandle, on: bool) {
    use tauri_plugin_global_shortcut::{GlobalShortcut, GlobalShortcutExt, ShortcutState};
    if app.try_state::<GlobalShortcut<tauri::Wry>>().is_none() {
        return;
    }
    let gs = app.global_shortcut();
    if on {
        let _ = gs.on_shortcut("Escape", |_, _, ev| {
            if ev.state == ShortcutState::Pressed {
                stop();
            }
        });
    } else {
        let _ = gs.unregister("Escape");
    }
}

/// The whole session on one thread: record, then (unless cancelled) upload and deliver.
fn session(app: AppHandle, ctl: Arc<Ctl>) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let started = sess().started_ms;
    emit_state(&app, "recording", json!({ "startedAt": started }));
    esc_key(&app, true);
    let take = record(&app, &ctl, RecOpts { max_ms: MAX_SECS * 1000, autostop: settings::get_bool("voice.autostop", false), test: false });
    esc_key(&app, false);
    let outcome: Result<Option<String>, VErr> = (|| {
        let take = take?;
        if ctl.cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if take.pcm.len() < MIN_SAMPLES {
            return Err(VErr::new("too_short", "The recording is shorter than half a second.".into()));
        }
        if take.peak < HEARD_RMS {
            return Err(VErr::new("silent", "The microphone picked up no sound (check the default microphone and its volume).".into()));
        }
        let cfg = load_cfg()?;
        sess().st = St::Transcribing;
        emit_state(&app, "transcribing", json!({}));
        let wav = wav16(&take.pcm, RATE);
        drop(take);
        let text = transcribe(&wav, &cfg, &ctl.abort)?;
        if ctl.cancel.load(Ordering::SeqCst) {
            return Ok(None);
        }
        if text.is_empty() {
            return Err(VErr::new("empty", "No speech was recognised.".into()));
        }
        Ok(Some(text))
    })();
    match outcome {
        Ok(Some(text)) => {
            deliver(&text);
            emit_state(&app, "done", json!({ "text": text, "chars": text.chars().count() }));
        }
        Ok(None) => emit_state(&app, "idle", json!({})),
        Err(e) if e.code == "cancelled" => emit_state(&app, "idle", json!({})),
        Err(e) => fail(&app, e),
    }
    let mut s = sess();
    s.st = St::Idle;
    s.ctl = None;
}

fn start(app: &AppHandle) -> Result<(), VErr> {
    load_cfg()?; // fail before recording, not after: switch / address / key
    let ctl = Arc::new(Ctl::default());
    {
        let mut s = sess();
        if s.st != St::Idle {
            return Err(VErr::new("busy", "Dictation is already running.".into()));
        }
        s.st = St::Recording;
        s.ctl = Some(ctl.clone());
        s.started_ms = crate::store::now_ms();
    }
    let a = app.clone();
    if std::thread::Builder::new().name("voice".into()).spawn(move || session(a, ctl)).is_err() {
        let mut s = sess();
        s.st = St::Idle;
        s.ctl = None;
        return Err(VErr::new("mic", "cannot start the recording thread".into()));
    }
    Ok(())
}

/// Ask the recording to finish; it then transcribes. No-op while idle / transcribing.
pub fn stop() {
    let s = sess();
    if s.st == St::Recording {
        if let Some(c) = &s.ctl {
            c.stop.store(true, Ordering::SeqCst);
        }
    }
}

fn cancel() {
    let s = sess();
    if let Some(c) = &s.ctl {
        c.cancel.store(true, Ordering::SeqCst);
        c.stop.store(true, Ordering::SeqCst);
        c.abort.abort();
    }
}

/// Hotkey / tile behaviour: idle -> start, recording -> stop. Errors become an error card.
pub fn toggle(app: &AppHandle) {
    let st = sess().st;
    match st {
        St::Idle => {
            if let Err(e) = start(app) {
                fail(app, e);
            }
        }
        St::Recording => stop(),
        _ => {}
    }
}

// Push-to-talk. GateGuard facts: only voice.rs (sync_hotkey handler) calls these; new setting voice.mode "toggle" (default) | "hold",
// read at every key event so no re-registration is needed; rollback = delete this block and restore `toggle(app)` in sync_hotkey.
// User request: "давай делать все", item: push-to-talk.
const HOLD_MIN_MS: i64 = 350; // a shorter hold in "hold" mode is an accidental tap

#[derive(Debug, PartialEq)]
enum HkAct {
    Nothing,
    Start,
    Stop,
    Discard, // stop and throw the take away: no upload, no error card
}

/// Pure decision for a hotkey event. toggle: press flips idle/recording, release is ignored.
/// hold: press starts only from idle (auto-repeat while recording is ignored), release stops, or discards a short tap.
fn hotkey_action(hold: bool, st: St, pressed: bool, held_ms: i64) -> HkAct {
    match (hold, st, pressed) {
        (_, St::Idle, true) => HkAct::Start,
        (false, St::Recording, true) => HkAct::Stop,
        (true, St::Recording, false) if held_ms < HOLD_MIN_MS => HkAct::Discard,
        (true, St::Recording, false) => HkAct::Stop,
        _ => HkAct::Nothing,
    }
}

static HELD: AtomicBool = AtomicBool::new(false); // the hold-mode press that started the current take; its release acts, any other does not

fn hotkey_event(app: &AppHandle, pressed: bool) {
    let hold = settings::get_str("voice.mode", "toggle") == "hold";
    if !hold {
        HELD.store(false, Ordering::SeqCst);
    } else if !pressed && !HELD.swap(false, Ordering::SeqCst) {
        return;
    }
    let (st, started) = {
        let s = sess();
        (s.st, s.started_ms)
    };
    match hotkey_action(hold, st, pressed, crate::store::now_ms() - started) {
        HkAct::Start => match start(app) {
            Ok(()) => HELD.store(hold, Ordering::SeqCst),
            Err(e) => fail(app, e),
        },
        HkAct::Stop => stop(),
        HkAct::Discard => cancel(),
        HkAct::Nothing => {}
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Result delivery: clipboard (+ optional Ctrl+V)
// ---------------------------------------------------------------------------------------------------------------

unsafe fn set_clipboard_text(s: &str) -> bool {
    let mut ok_open = false;
    for _ in 0..10 {
        if OpenClipboard(windows::Win32::Foundation::HWND::default()).is_ok() {
            ok_open = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    if !ok_open {
        return false;
    }
    let _ = EmptyClipboard();
    let bytes: Vec<u8> = s.encode_utf16().chain(Some(0)).flat_map(|u| u.to_le_bytes()).collect();
    let ok = (|| {
        let h: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).ok()?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        SetClipboardData(13, HANDLE(h.0)).ok() // CF_UNICODETEXT; on success the system owns the memory
    })()
    .is_some();
    let _ = CloseClipboard();
    ok
}

unsafe fn paste_ctrl_v() {
    // The hotkey's Ctrl/Alt may still be down for a moment: wait for Alt to go up, or Ctrl+V becomes Ctrl+Alt+V.
    for _ in 0..25 {
        if GetAsyncKeyState(0x12) >= 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    // Our own window in front (chat input): the page inserts the text itself, a Ctrl+V would paste into the wrong place.
    let fg = GetForegroundWindow();
    let mut pid = 0u32;
    GetWindowThreadProcessId(fg, Some(&mut pid));
    if fg.0.is_null() || pid == GetCurrentProcessId() {
        return;
    }
    let key = |vk: u16, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    };
    let seq = [key(0x11, false), key(0x56, false), key(0x56, true), key(0x11, true)];
    let _ = SendInput(&seq, std::mem::size_of::<INPUT>() as i32); // an elevated target silently ignores it (UIPI): the text is on the clipboard anyway
}

fn deliver(text: &str) {
    unsafe {
        if !set_clipboard_text(text) {
            eprintln!("voice: could not open the clipboard");
            return;
        }
        if settings::get_bool("voice.paste", false) {
            std::thread::sleep(Duration::from_millis(80));
            paste_ctrl_v();
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------------------------

fn res(r: Result<(), VErr>, app: &AppHandle) -> Value {
    match r {
        Ok(()) => json!({ "ok": true }),
        Err(e) => {
            fail(app, VErr { code: e.code, msg: e.msg.clone() });
            json!({ "ok": false, "code": e.code, "error": e.msg })
        }
    }
}

#[tauri::command]
pub fn voice_start(app: AppHandle) -> Value {
    let r = start(&app);
    res(r, &app)
}

#[tauri::command]
pub fn voice_toggle(app: AppHandle) -> Value {
    toggle(&app);
    voice_state()
}

#[tauri::command]
pub fn voice_stop() -> Value {
    stop();
    voice_state()
}

#[tauri::command]
pub fn voice_cancel() -> Value {
    cancel();
    voice_state()
}

#[tauri::command]
pub fn voice_state() -> Value {
    let s = sess();
    let name = match s.st {
        St::Idle => "idle",
        St::Recording => "recording",
        St::Transcribing => "transcribing",
        St::Testing => "testing",
    };
    json!({ "state": name, "startedAt": s.started_ms, "enabled": net::enabled(Feature::Voice), "host": host_of(&endpoint()) })
}

/// Settings page: record 2 s from the default microphone and report the level. Nothing is uploaded or kept.
#[tauri::command]
pub async fn voice_test_mic(app: AppHandle) -> Value {
    {
        let mut s = sess();
        if s.st != St::Idle {
            return json!({ "ok": false, "error": "Dictation is running." });
        }
        s.st = St::Testing;
    }
    let r = tauri::async_runtime::spawn_blocking(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let out = record(&app, &Ctl::default(), RecOpts { max_ms: 2000, autostop: false, test: true });
        emit_level(&app, 0.0, true);
        out
    })
    .await;
    sess().st = St::Idle;
    match r {
        Ok(Ok(t)) => json!({ "ok": t.peak >= HEARD_RMS, "peak": t.peak, "level": level_of(t.peak), "error": if t.peak >= HEARD_RMS { Value::Null } else { json!("No sound picked up. Check the default microphone and its volume.") } }),
        Ok(Err(e)) => json!({ "ok": false, "code": e.code, "error": e.msg }),
        Err(_) => json!({ "ok": false, "error": "the test failed" }),
    }
}

/// Settings page: upload one second of silence and report whether the service accepts the request (ms, status).
#[tauri::command]
pub async fn voice_test_service() -> Value {
    let r = tauri::async_runtime::spawn_blocking(|| {
        let cfg = load_cfg()?;
        let wav = wav16(&vec![0i16; RATE as usize], RATE);
        let t0 = Instant::now();
        transcribe(&wav, &cfg, &Abort::new()).map(|_| t0.elapsed().as_millis() as u64)
    })
    .await;
    match r {
        Ok(Ok(ms)) => json!({ "ok": true, "ms": ms, "host": host_of(&endpoint()) }),
        Ok(Err(e)) => json!({ "ok": false, "code": e.code, "error": e.msg }),
        Err(_) => json!({ "ok": false, "error": "the test failed" }),
    }
}

/// Stores the key of a provider encrypted with DPAPI (net::secrets, same as the AI key). Empty = remove.
#[tauri::command]
pub fn voice_set_key(provider: String, key: String) -> Value {
    let p: String = provider.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect();
    if p.is_empty() {
        return json!({ "ok": false, "error": "unknown provider" });
    }
    let k = key.trim();
    let blob = if k.is_empty() {
        String::new()
    } else {
        match net::secrets::protect(k) {
            Ok(b) => b,
            Err(e) => return json!({ "ok": false, "error": e }),
        }
    };
    settings::set(&format!("voice.key.{p}"), json!(blob));
    voice_has_key(Some(p))
}

/// {has, source: "voice" | "ai" | "none"}: "ai" = the AI key of the same host is used.
#[tauri::command]
pub fn voice_has_key(provider: Option<String>) -> Value {
    let p = provider.unwrap_or_else(provider_cur);
    let own = !settings::get_str(&format!("voice.key.{p}"), "").trim().is_empty();
    let ai = !settings::get_str("ai.apiKeyEnc", "").trim().is_empty()
        && host_of(&endpoint()).is_some_and(|h| host_of(&settings::get_str("ai.baseUrl", "")).is_some_and(|a| a == h));
    json!({ "ok": true, "has": own || ai, "source": if own { "voice" } else if ai { "ai" } else { "none" } })
}

fn provider_cur() -> String {
    provider()
}

// ---------------------------------------------------------------------------------------------------------------
// Hotkey + init: registered only while `voice.enabled`; follows hotkey.voice / voice.enabled changes at run time
// ---------------------------------------------------------------------------------------------------------------

static HK: Mutex<(Option<String>, bool)> = Mutex::new((None, false)); // (combo wanted last time, registered)

fn sync_hotkey(app: &AppHandle) {
    use tauri_plugin_global_shortcut::{GlobalShortcut, GlobalShortcutExt, ShortcutState};
    let want = if net::enabled(Feature::Voice) {
        Some(settings::get_str("hotkey.voice", "Ctrl+Alt+D")).filter(|s| !s.trim().is_empty())
    } else {
        None
    };
    let mut hk = HK.lock().unwrap_or_else(|e| e.into_inner());
    if hk.0 == want {
        return;
    }
    if app.try_state::<GlobalShortcut<tauri::Wry>>().is_none() {
        return;
    }
    let gs = app.global_shortcut();
    if let (Some(old), true) = (&hk.0, hk.1) {
        let _ = gs.unregister(old.as_str());
    }
    hk.1 = false;
    if let Some(spec) = &want {
        match gs.on_shortcut(spec.as_str(), |app, _, ev| {
            hotkey_event(app, ev.state == ShortcutState::Pressed);
        }) {
            Ok(_) => hk.1 = true,
            Err(e) => eprintln!("voice: hotkey '{spec}' could not be registered: {e}"),
        }
    }
    hk.0 = want;
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let _ = APP.set(h.clone());
    sync_hotkey(&h);
    let h2 = h.clone();
    // always on the main thread: the shortcut plugin hops there anyway, and two threads must never wait on each other through HK
    h.listen("settings:changed", move |_| {
        let h3 = h2.clone();
        let _ = h2.run_on_main_thread(move || sync_hotkey(&h3));
    });
}

// ---------------------------------------------------------------------------------------------------------------
// Tests (pure parts only: no audio device, no network)
// ---------------------------------------------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_wav_header_and_size() {
        let w = wav16(&[0, 1, -1, 32767], 16000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()) as usize, w.len() - 8);
        assert_eq!(u16::from_le_bytes(w[20..22].try_into().unwrap()), 1); // PCM
        assert_eq!(u16::from_le_bytes(w[22..24].try_into().unwrap()), 1); // mono
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16000);
        assert_eq!(u32::from_le_bytes(w[28..32].try_into().unwrap()), 32000); // byte rate
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 8);
        assert_eq!(w.len(), 44 + 8);
        assert_eq!(i16::from_le_bytes([w[48], w[49]]), -1);
        assert_eq!(i16::from_le_bytes([w[50], w[51]]), 32767);
        assert_eq!(wav16(&[], 16000).len(), 44);
        assert_eq!(wav16(&vec![0; 16000], 16000).len(), 44 + 32000); // the 1 s silent test file
    }

    #[test]
    fn voice_resampler_rates() {
        let mut rs = Resampler::new(48000.0, 16000.0);
        let mut out = Vec::new();
        for _ in 0..4800 {
            rs.push(0.5, &mut out);
        }
        assert!((1599..=1600).contains(&out.len()), "{}", out.len());
        assert!(out.iter().all(|x| (x - 0.5).abs() < 1e-4));
        // 44.1 kHz: ratio 2.75625, non-integer
        let mut rs = Resampler::new(44100.0, 16000.0);
        let mut out = Vec::new();
        for _ in 0..44100 {
            rs.push(-0.25, &mut out);
        }
        assert!((15999..=16000).contains(&out.len()), "{}", out.len());
        assert!(out.iter().all(|x| (x + 0.25).abs() < 1e-4));
        // upsampling 8 kHz -> 16 kHz doubles the count
        let mut rs = Resampler::new(8000.0, 16000.0);
        let mut out = Vec::new();
        for _ in 0..100 {
            rs.push(0.1, &mut out);
        }
        assert_eq!(out.len(), 200);
        // a 2 kHz-aliasing tone at 48 kHz (24 kHz Nyquist alternation) is averaged away
        let mut rs = Resampler::new(48000.0, 16000.0);
        let mut out = Vec::new();
        for i in 0..4800 {
            rs.push(if i % 2 == 0 { 1.0 } else { -1.0 }, &mut out);
        }
        assert!(out.iter().all(|x| x.abs() < 0.4));
    }

    #[test]
    fn voice_sample_helpers() {
        assert_eq!(to_i16(1.5), 32767);
        assert_eq!(to_i16(-1.5), -32767);
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(level_of(0.0), 0.0);
        assert_eq!(level_of(1.0), 1.0);
        assert!((level_of(0.1) - 0.6).abs() < 0.01);
    }

    #[test]
    fn voice_silence_detector() {
        let mut s = Silence::new(0.012, 4000.0);
        for _ in 0..200 {
            assert!(!s.feed(0.0, 40.0), "never stops before any speech");
        }
        assert!(!s.feed(0.1, 40.0)); // speech
        for _ in 0..99 {
            assert!(!s.feed(0.001, 40.0)); // 3.96 s quiet
        }
        assert!(s.feed(0.001, 40.0)); // 4.0 s
        let mut s = Silence::new(0.012, 4000.0);
        s.feed(0.1, 40.0);
        for _ in 0..90 {
            s.feed(0.0, 40.0);
        }
        assert!(!s.feed(0.2, 40.0), "speech resets the quiet counter");
        for _ in 0..99 {
            assert!(!s.feed(0.0, 40.0));
        }
    }

    #[test]
    fn voice_multipart_body() {
        let b = multipart("XX", &[("model", "whisper-1"), ("language", "ru")], "audio.wav", "audio/wav", b"RIFFdata");
        let s = String::from_utf8_lossy(&b).to_string();
        assert!(s.starts_with("--XX\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nwhisper-1\r\n"));
        assert!(s.contains("name=\"language\"\r\n\r\nru\r\n"));
        assert!(s.contains("name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n--XX--\r\n"));
        assert!(s.ends_with("\r\n--XX--\r\n"));
        assert_eq!(s.matches("--XX\r\n").count(), 3);
    }

    #[test]
    fn voice_host_extraction() {
        assert_eq!(host_of("https://api.groq.com/openai/v1").as_deref(), Some("api.groq.com"));
        assert_eq!(host_of("http://127.0.0.1:8000/v1").as_deref(), Some("127.0.0.1"));
        assert_eq!(host_of("HTTPS://Generativelanguage.googleapis.com/v1beta/openai").as_deref(), Some("generativelanguage.googleapis.com"));
        assert_eq!(host_of("api.groq.com"), None);
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("https://user:pw@evil.example/v1"), None);
    }

    #[test]
    fn voice_transcript_parsing_and_redaction() {
        assert_eq!(parse_transcript(200, r#"{"text":"  привет мир \n"}"#.as_bytes(), false, None).unwrap(), "привет мир");
        assert_eq!(parse_transcript(200, b"plain text\n", false, None).unwrap(), "plain text");
        assert_eq!(parse_transcript(200, br#"{"choices":[{"message":{"content":" ok "}}]}"#, true, None).unwrap(), "ok");
        assert!(parse_transcript(200, br#"{"x":1}"#, false, None).is_err());
        let e = parse_transcript(401, br#"{"error":{"message":"Invalid API key sk-secret-123456"}}"#, false, Some("sk-secret-123456")).unwrap_err();
        assert_eq!(e.code, "http");
        assert!(e.msg.starts_with("HTTP 401: Invalid API key"));
        assert!(!e.msg.contains("sk-secret"), "{}", e.msg);
        let long = format!("{{\"error\":{{\"message\":\"{}\"}}}}", "x".repeat(500));
        assert!(parse_transcript(500, long.as_bytes(), false, None).unwrap_err().msg.chars().count() < 190);
    }

    #[test]
    fn voice_format_parse() {
        // WAVEFORMATEX: float32 stereo 48 kHz
        let mut b = vec![0u8; 18];
        b[0..2].copy_from_slice(&3u16.to_le_bytes());
        b[2..4].copy_from_slice(&2u16.to_le_bytes());
        b[4..8].copy_from_slice(&48000u32.to_le_bytes());
        b[14..16].copy_from_slice(&32u16.to_le_bytes());
        let f = parse_format(&b).unwrap();
        assert_eq!(f, Fmt { channels: 2, rate: 48000, bits: 32, float: true });
        let mut m = Vec::new();
        let mut data = Vec::new();
        data.extend_from_slice(&0.5f32.to_le_bytes());
        data.extend_from_slice(&(-0.25f32).to_le_bytes());
        to_mono(&data, &f, &mut m);
        assert_eq!(m, vec![0.125]);
        assert!(parse_format(&b[..10]).is_none());
    }

    #[test]
    fn voice_hotkey_action() {
        use HkAct::*;
        // toggle: press flips, release ignored
        assert_eq!(hotkey_action(false, St::Idle, true, 0), Start);
        assert_eq!(hotkey_action(false, St::Recording, true, 0), Stop);
        assert_eq!(hotkey_action(false, St::Recording, false, 9999), Nothing);
        assert_eq!(hotkey_action(false, St::Transcribing, true, 0), Nothing);
        // hold: press starts, auto-repeat press is ignored, release stops or discards a tap
        assert_eq!(hotkey_action(true, St::Idle, true, 0), Start);
        assert_eq!(hotkey_action(true, St::Recording, true, 100), Nothing);
        assert_eq!(hotkey_action(true, St::Recording, false, HOLD_MIN_MS - 1), Discard);
        assert_eq!(hotkey_action(true, St::Recording, false, HOLD_MIN_MS), Stop);
        assert_eq!(hotkey_action(true, St::Idle, false, 1000), Nothing);
        assert_eq!(hotkey_action(true, St::Transcribing, false, 1000), Nothing);
        assert_eq!(hotkey_action(true, St::Testing, true, 0), Nothing);
    }
}
