// Music-reactive edge: WASAPI loopback of the default render device -> 32 spectrum bands for the pill page.
// Owner: media agent (lane L2).
// PRIVACY: captured samples live only in a 2048-sample in-memory ring, are never stored, logged or sent anywhere.
// Only 32 level numbers (+ one rms value and a timestamp) leave Rust, as the `audio:bands` event (~60 Hz) to the local window "pill".
// The capture runs only while the pill is visible AND the selected media session is playing AND setting
// `audio.reactive` (default true) is on; otherwise the thread is parked and no audio client is open.
use crate::ui::bus;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};

const N: usize = 2048; // 23 Hz bins at 48 kHz; window 43 ms, hop FRAME_EVERY = 16 ms (~62 % overlap)
const B: usize = 32;
const F_LO: f32 = 40.0;
const F_HI: f32 = 14000.0;
const NOISE_FLOOR: f32 = 0.0005; // rms, about -66 dBFS
const SILENCE_AFTER: Duration = Duration::from_millis(1500);
const FRAME_EVERY: Duration = Duration::from_millis(16); // ~60 Hz
const POLL: Duration = Duration::from_millis(16); // capture drain granularity (the shared buffer holds 200 ms); sleeps never overshoot the frame deadline
const BUFFERFLAGS_SILENT: u32 = 2;
// Time constants matching the old per-33-ms-frame feel: attack 0.7, release 0.15 per 33 ms.
const TAU_ATTACK_MS: f32 = 27.4; // -33 / ln(1 - 0.7)
const TAU_RELEASE_MS: f32 = 203.0; // -33 / ln(1 - 0.15)

// ---------- pure DSP (unit-tested) ----------

/// In-place iterative radix-2 FFT; `re.len()` must be a power of two.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * std::f32::consts::PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for s in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (s + k, s + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

/// Log-spaced band edges in Hz, F_LO..F_HI.
fn band_edges() -> [f32; B + 1] {
    let mut e = [0.0; B + 1];
    for (i, v) in e.iter_mut().enumerate() {
        *v = F_LO * (F_HI / F_LO).powf(i as f32 / B as f32);
    }
    e
}

/// Per band: (lo, hi) in fractional FFT bins, kept inside 1..N/2-1.
fn band_bins(rate: f32) -> [(f32, f32); B] {
    let hz = rate / N as f32;
    let e = band_edges();
    let max = (N / 2 - 1) as f32;
    let mut out = [(1.0, 1.0); B];
    for b in 0..B {
        out[b] = ((e[b] / hz).clamp(1.0, max), (e[b + 1] / hz).clamp(1.0, max));
    }
    out
}

/// Magnitude (full-scale sine = 1.0) -> 0..1 on a -60..0 dB scale.
fn db_level(mag: f32) -> f32 {
    ((20.0 * mag.max(1e-9).log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// Time-based fast attack, slow release: alpha = 1 - exp(-dt/tau), so the feel is independent of the frame rate.
fn smooth(prev: f32, new: f32, dt_ms: f32) -> f32 {
    let tau = if new > prev { TAU_ATTACK_MS } else { TAU_RELEASE_MS };
    prev + (new - prev) * (1.0 - (-dt_ms.max(0.0) / tau).exp())
}

/// Ring (`head` = next write = oldest sample) -> `out` in time order, oldest first.
fn unwind(ring: &[f32], head: usize, out: &mut [f32]) {
    let k = ring.len() - head;
    out[..k].copy_from_slice(&ring[head..]);
    out[k..].copy_from_slice(&ring[..head]);
}

/// All buffers are allocated once; `frame` allocates nothing.
struct Analyzer {
    win: Vec<f32>,
    bins: [(f32, f32); B],
    ring: Vec<f32>, // last N samples, zero-filled until the first N arrive
    head: usize,
    re: Vec<f32>,
    im: Vec<f32>,
    mag: Vec<f32>,
    lv: [f32; B],
}

impl Analyzer {
    fn new(rate: f32) -> Self {
        let win = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
        Analyzer {
            win,
            bins: band_bins(rate),
            ring: vec![0.0; N],
            head: 0,
            re: vec![0.0; N],
            im: vec![0.0; N],
            mag: vec![0.0; N / 2],
            lv: [0.0; B],
        }
    }

    /// Every sample lands in the ring, none is skipped between frames.
    fn push(&mut self, s: &[f32]) {
        for &x in s.iter().skip(s.len().saturating_sub(N)) {
            self.ring[self.head] = x;
            self.head = (self.head + 1) % N;
        }
    }

    fn reset(&mut self) {
        self.ring.fill(0.0);
        self.head = 0;
        self.lv = [0.0; B];
    }

    /// Spectrum of the most recent N samples: band levels smoothed over `dt_ms`, and raw rms.
    fn frame(&mut self, dt_ms: f32) -> ([f32; B], f32) {
        unwind(&self.ring, self.head, &mut self.re);
        let rms = (self.re.iter().map(|x| x * x).sum::<f32>() / N as f32).sqrt();
        for (x, w) in self.re.iter_mut().zip(&self.win) {
            *x *= w;
        }
        self.im.fill(0.0);
        fft(&mut self.re, &mut self.im);
        // Hann window + one-sided spectrum: a full-scale sine has |X| = N/4.
        for k in 0..N / 2 {
            self.mag[k] = self.re[k].hypot(self.im[k]) / (N as f32 / 4.0);
        }
        let mag = &self.mag;
        for (b, &(lo, hi)) in self.bins.iter().enumerate() {
            let m = if hi - lo < 1.0 {
                // narrower than one bin: interpolate at the band centre so low bands stay distinct
                let c = (lo + hi) / 2.0;
                let k = c as usize;
                let f = c - k as f32;
                mag[k] * (1.0 - f) + mag[(k + 1).min(N / 2 - 1)] * f
            } else {
                (lo.ceil() as usize..=hi as usize).map(|k| mag[k.min(N / 2 - 1)]).fold(0.0, f32::max)
            };
            self.lv[b] = smooth(self.lv[b], db_level(m), dt_ms);
        }
        (self.lv, rms)
    }
}

// ---------- sample formats ----------

#[derive(Debug, PartialEq)]
struct Fmt {
    channels: usize,
    rate: u32,
    bits: u16,
    float: bool,
}

/// Parse a WAVEFORMATEX / WAVEFORMATEXTENSIBLE blob (little-endian bytes).
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
    let ok = if float { bits == 32 } else { matches!(bits, 16 | 24 | 32) };
    let channels = u16_at(2) as usize;
    if !ok || channels == 0 {
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

// ---------- capture ----------

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    running: bool,
    device: String,
    sample_rate: u32,
    reactive: bool,
}

static STATUS: Mutex<Option<(String, u32)>> = Mutex::new(None);
/// A non-zero frame went out and no zero frame followed yet.
static DIRTY: AtomicBool = AtomicBool::new(false);

pub fn audio_status() -> Status {
    let s = STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    Status {
        running: s.is_some(),
        device: s.as_ref().map(|x| x.0.clone()).unwrap_or_default(),
        sample_rate: s.map(|x| x.1).unwrap_or(0),
        reactive: reactive(),
    }
}

fn reactive() -> bool {
    crate::settings::get_bool("audio.reactive", true)
}

fn wanted() -> bool {
    reactive() && crate::media::want_capture()
}

/// `t` is a monotonic timestamp in ms (since the first emit of the process), for page-side interpolation.
#[derive(Serialize)]
struct Frame {
    b: [f32; B],
    rms: f32,
    t: u64,
}

static T0: OnceLock<Instant> = OnceLock::new();

fn emit(_app: &tauri::AppHandle, b: &[f32; B], rms: f32) {
    let zero = b.iter().all(|v| *v == 0.0);
    DIRTY.store(!zero, Ordering::Relaxed);
    let t = T0.get_or_init(Instant::now).elapsed().as_millis() as u64;
    let mut b = *b;
    b.iter_mut().for_each(|v| *v = (*v * 100.0).round() / 100.0);
    let _ = bus::emit("audio:bands", &Frame { b, rms: (rms * 1000.0).round() / 1000.0, t });
}

fn device_id(dev: &IMMDevice) -> String {
    unsafe {
        match dev.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

fn default_device() -> windows::core::Result<IMMDevice> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        en.GetDefaultAudioEndpoint(eRender, eConsole)
    }
}

/// Returns Ok when capture is no longer wanted or the default device changed; Err when the stream died.
fn run_capture(app: &tauri::AppHandle) -> Result<(), String> {
    let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
    unsafe {
        let dev = default_device().map_err(e("default device"))?;
        let id = device_id(&dev);
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(e("activate"))?;
        let pf = client.GetMixFormat().map_err(e("mix format"))?;
        let cb = u16::from_le_bytes([*(pf as *const u8).add(16), *(pf as *const u8).add(17)]) as usize;
        let fmt = parse_format(std::slice::from_raw_parts(pf as *const u8, 18 + cb));
        // 200 ms shared-mode buffer; loopback needs no event callback, we poll.
        let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, pf, None);
        CoTaskMemFree(Some(pf as *const _));
        let Some(fmt) = fmt else { return Err("unsupported mix format (need float32 or int16/24/32 PCM)".into()) };
        init.map_err(e("initialize loopback"))?;
        let cap: IAudioCaptureClient = client.GetService().map_err(e("capture service"))?;
        client.Start().map_err(e("start"))?;
        *STATUS.lock().unwrap_or_else(|x| x.into_inner()) = Some((id.clone(), fmt.rate));

        let mut an = Analyzer::new(fmt.rate as f32);
        let mut mono: Vec<f32> = Vec::new();
        let mut last_data = Instant::now();
        let mut next_frame = Instant::now();
        let mut prev_frame = next_frame;
        let mut next_check = Instant::now() + Duration::from_secs(1);
        let mut silent_since: Option<Instant> = None;
        let mut sent_zero = false;
        let stale_zeros = vec![0.0f32; fmt.rate as usize / 10 + 1]; // up to 100 ms of zeros
        let res = loop {
            // Wake at most every POLL, but never sleep past the frame deadline (std sleep is high-resolution on Windows).
            std::thread::sleep(POLL.min(next_frame.saturating_duration_since(Instant::now())));
            let mut fail = None;
            loop {
                let n = match cap.GetNextPacketSize() {
                    Ok(n) => n,
                    Err(er) => { fail = Some(format!("packet size: {er}")); break; }
                };
                if n == 0 {
                    break;
                }
                let (mut p, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                if let Err(er) = cap.GetBuffer(&mut p, &mut frames, &mut flags, None, None) {
                    fail = Some(format!("get buffer: {er}"));
                    break;
                }
                mono.clear();
                if flags & BUFFERFLAGS_SILENT != 0 || p.is_null() {
                    mono.resize(frames as usize, 0.0);
                } else {
                    to_mono(std::slice::from_raw_parts(p, frames as usize * fmt.bits as usize / 8 * fmt.channels), &fmt, &mut mono);
                }
                let _ = cap.ReleaseBuffer(frames);
                an.push(&mono);
                last_data = Instant::now();
            }
            if let Some(f) = fail {
                break Err(f);
            }
            let now = Instant::now();
            if now >= next_check {
                next_check = now + Duration::from_secs(1);
                // The loopback follows the device it was opened on: reopen when the default output changes.
                if !wanted() || default_device().map(|d| device_id(&d)).unwrap_or_default() != id {
                    break Ok(());
                }
            }
            if now < next_frame {
                continue;
            }
            // Fixed deadlines: no drift; if we fell a whole period behind, resync instead of bursting.
            next_frame += FRAME_EVERY;
            if next_frame < now {
                next_frame = now + FRAME_EVERY;
            }
            let dt_ms = (now.duration_since(prev_frame).as_secs_f32() * 1000.0).clamp(1.0, 100.0);
            prev_frame = now;
            if last_data.elapsed() > Duration::from_millis(120) {
                // loopback delivers nothing in silence: slide in zeros for the time that passed
                an.push(&stale_zeros[..(fmt.rate as f32 * dt_ms / 1000.0) as usize]);
            }
            let (b, rms) = an.frame(dt_ms);
            if rms < NOISE_FLOOR {
                let since = *silent_since.get_or_insert(now);
                if sent_zero {
                    continue;
                }
                if now.duration_since(since) >= SILENCE_AFTER {
                    an.reset();
                    emit(app, &[0.0; B], 0.0);
                    sent_zero = true;
                    continue;
                }
            } else {
                silent_since = None;
                sent_zero = false;
            }
            emit(app, &b, rms);
        };
        let _ = client.Stop();
        res
    }
}

fn thread(app: tauri::AppHandle) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut fails = 0u32;
    loop {
        if !wanted() {
            fails = 0;
            crate::media::wait_change(Duration::from_secs(2));
            continue;
        }
        let r = run_capture(&app);
        *STATUS.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if DIRTY.load(Ordering::Relaxed) {
            emit(&app, &[0.0; B], 0.0); // let the page calm the edge
        }
        match r {
            Ok(()) => fails = 0,
            Err(err) => {
                fails += 1;
                eprintln!("[audio] capture stopped: {err} (attempt {fails})");
                if fails >= 5 {
                    eprintln!("[audio] giving up for 30 s");
                    crate::media::wait_change(Duration::from_secs(30));
                    fails = 0;
                } else {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }
    }
}

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    std::thread::spawn(move || thread(h));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(f: f32, rate: f32, a: f32) -> Vec<f32> {
        (0..N).map(|i| a * (2.0 * std::f32::consts::PI * f * i as f32 / rate).sin()).collect()
    }

    #[test]
    fn fft_peak_bin_and_amplitude() {
        // bin 64 of a 48 kHz / 2048 FFT is exactly 1500 Hz
        let mut re = sine(1500.0, 48000.0, 0.5);
        let mut im = vec![0.0; N];
        fft(&mut re, &mut im);
        let mag: Vec<f32> = (0..N / 2).map(|k| re[k].hypot(im[k])).collect();
        let peak = mag.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(peak, 64);
        assert!((mag[64] - 0.5 * N as f32 / 2.0).abs() < 2.0);
    }

    #[test]
    fn analyzer_peak_in_right_band() {
        let mut an = Analyzer::new(48000.0);
        an.push(&sine(1000.0, 48000.0, 0.5));
        let (b, rms) = an.frame(16.0);
        let top = b.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        let e = band_edges();
        assert!(e[top] <= 1000.0 && 1000.0 < e[top + 1], "band {top} [{} {}]", e[top], e[top + 1]);
        assert!((rms - 0.5 / 2f32.sqrt()).abs() < 0.01);
        let (z, r0) = Analyzer::new(48000.0).frame(16.0);
        assert!(z.iter().all(|v| *v == 0.0) && r0 == 0.0);
    }

    #[test]
    fn ring_keeps_latest_n_in_order() {
        // chunks of an odd size wrap the ring; every sample must survive, oldest first
        let mut an = Analyzer::new(48000.0);
        let all: Vec<f32> = (0..N * 2 + 123).map(|i| i as f32).collect();
        for c in all.chunks(777) {
            an.push(c);
        }
        let mut out = vec![0.0; N];
        unwind(&an.ring, an.head, &mut out);
        assert_eq!(out, all[all.len() - N..]);
        // one push larger than N keeps only its tail
        an.push(&all);
        unwind(&an.ring, an.head, &mut out);
        assert_eq!(out, all[all.len() - N..]);
        // a short push is zero-padded in front
        an.reset();
        an.push(&[1.0, 2.0]);
        unwind(&an.ring, an.head, &mut out);
        assert!(out[..N - 2].iter().all(|v| *v == 0.0) && out[N - 2..] == [1.0, 2.0]);
    }

    #[test]
    fn band_mapping_monotonic() {
        for rate in [44100.0, 48000.0, 96000.0] {
            let bins = band_bins(rate);
            for w in bins.windows(2) {
                assert!(w[0].0 <= w[1].0 && w[0].1 <= w[1].1);
            }
            assert!(bins.iter().all(|b| b.0 >= 1.0 && b.0 <= b.1 && b.1 <= (N / 2 - 1) as f32));
        }
        let e = band_edges();
        assert!((e[0] - F_LO).abs() < 1e-3 && (e[B] - F_HI).abs() < 1.0);
    }

    #[test]
    fn band_bins_follow_sample_rate() {
        // 48 kHz / 2048 = 23.4375 Hz per bin: 40 Hz -> bin 1.7066, 14 kHz -> bin 597.33
        let e = band_edges();
        for rate in [44100.0f32, 48000.0, 96000.0] {
            let hz = rate / N as f32;
            for (b, &(lo, hi)) in band_bins(rate).iter().enumerate() {
                assert!((lo - (e[b] / hz).max(1.0)).abs() < 1e-3 && (hi - (e[b + 1] / hz).max(1.0)).abs() < 1e-3);
            }
        }
        let bins = band_bins(48000.0);
        assert!((bins[0].0 - 40.0 / 23.4375).abs() < 1e-3);
        assert!((bins[B - 1].1 - 14000.0 / 23.4375).abs() < 0.01);
    }

    #[test]
    fn db_scale() {
        assert_eq!(db_level(1.0), 1.0);
        assert_eq!(db_level(0.0), 0.0);
        assert_eq!(db_level(0.0005), 0.0);
        assert!((db_level(0.0316228) - 0.5).abs() < 0.01); // -30 dB
    }

    #[test]
    fn smoothing_attack_fast_release_slow() {
        // 33 ms reproduces the old per-frame constants
        assert!((smooth(0.0, 1.0, 33.0) - 0.7).abs() < 2e-3);
        assert!((smooth(1.0, 0.0, 33.0) - 0.85).abs() < 2e-3);
        assert_eq!(smooth(0.4, 0.4, 16.0), 0.4);
        assert_eq!(smooth(0.3, 1.0, 0.0), 0.3);
    }

    #[test]
    fn smoothing_is_frame_rate_independent() {
        // 1 s toward a constant target gives the same level at 60 Hz, 30 Hz and 250 Hz
        let run = |from: f32, to: f32, dt: f32| {
            let mut v = from;
            for _ in 0..(1000.0 / dt).round() as usize {
                v = smooth(v, to, dt);
            }
            v
        };
        for (from, to) in [(0.0, 1.0), (1.0, 0.0)] {
            let a = run(from, to, 16.0);
            assert!((a - run(from, to, 33.0)).abs() < 2e-3 && (a - run(from, to, 4.0)).abs() < 2e-3);
        }
        // release is slower than attack at equal dt
        assert!(1.0 - smooth(1.0, 0.0, 16.0) < smooth(0.0, 1.0, 16.0));
    }

    #[test]
    fn formats_and_mono_mix() {
        // 48 kHz stereo int16 WAVEFORMATEX
        let mut h = vec![1, 0, 2, 0];
        h.extend(48000u32.to_le_bytes());
        h.extend(192000u32.to_le_bytes());
        h.extend([4, 0, 16, 0, 0, 0]);
        let f = parse_format(&h).unwrap();
        assert_eq!(f, Fmt { channels: 2, rate: 48000, bits: 16, float: false });
        let mut out = vec![];
        to_mono(&[0x00, 0x40, 0x00, 0xC0], &f, &mut out); // +0.5 and -0.5
        assert_eq!(out, vec![0.0]);
        h[0] = 3; // float tag with 16 bits is not a valid combination
        assert!(parse_format(&h).is_none());
        let ff = Fmt { channels: 1, rate: 48000, bits: 32, float: true };
        let mut o = vec![];
        to_mono(&0.25f32.to_le_bytes(), &ff, &mut o);
        assert_eq!(o, vec![0.25]);
    }
}
