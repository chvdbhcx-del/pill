// Sound for screen recordings (capture.rs): the PC sound (WASAPI loopback of the default output) and/or the microphone, each written
// to a temporary 16-bit stereo WAV while ffmpeg records the picture, then muxed into the MP4 by the ffmpeg capture.rs already uses.
// Nothing here is downloaded, sent or kept: the WAVs live in the temp folder and are deleted after the mux (or when it fails).
// The loopback delivers nothing in silence, so every track is padded with zeros to the wall clock: both stay in step with the video.
use crate::audio::{parse_format, Fmt};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use windows::Win32::Media::Audio::{
    eCapture, eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator, MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED};

const POLL: Duration = Duration::from_millis(20);
const BUFFERFLAGS_SILENT: u32 = 2;
/// Seconds cut from the start of every track when muxing: ffmpeg needs this long after its spawn to grab the first picture, the sound
/// starts at the spawn. A tuning knob (raise it if the sound runs ahead of the picture, lower it if it lags).
pub const LEAD_IN_SECS: f64 = 0.25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Source {
    /// what Windows plays
    System,
    Mic,
}

/// Running recorders; `finish` stops them and returns the finished WAV files.
pub struct Session {
    stop: Arc<AtomicBool>,
    jobs: Vec<(Source, JoinHandle<Result<PathBuf, String>>)>,
}

/// Starts the wanted recorders (a source that cannot be opened is logged and skipped). `None` when nothing is wanted or nothing opened.
pub fn start(dir: &Path, stem: &str, system: bool, mic: bool) -> Option<Session> {
    let stop = Arc::new(AtomicBool::new(false));
    let mut jobs = Vec::new();
    for (src, on, tag) in [(Source::System, system, "sys"), (Source::Mic, mic, "mic")] {
        if !on {
            continue;
        }
        let (path, st) = (dir.join(format!("{stem}.{tag}.wav")), stop.clone());
        jobs.push((src, std::thread::spawn(move || record(src, &path, &st).map(|_| path))));
    }
    (!jobs.is_empty()).then_some(Session { stop, jobs })
}

impl Session {
    /// Stops, waits for the files to be closed and returns those that hold sound. Failures are logged (no audio, no content).
    pub fn finish(mut self) -> Vec<PathBuf> {
        self.stop.store(true, Ordering::Relaxed);
        std::mem::take(&mut self.jobs)
            .into_iter()
            .filter_map(|(src, j)| match j.join() {
                Ok(Ok(p)) => Some(p),
                Ok(Err(e)) => {
                    eprintln!("capture: {src:?} sound not recorded: {e}");
                    None
                }
                Err(_) => None,
            })
            .collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed); // threads end by themselves; their files are removed by `remove`
    }
}

pub fn remove(files: &[PathBuf]) {
    for f in files {
        let _ = std::fs::remove_file(f);
    }
}

/// ffmpeg arguments that put `audio` WAVs under the copied picture of `video`: one track as it is, two mixed. AAC, `-shortest`.
pub fn mux_args(video: &str, audio: &[String], out: &str) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-loglevel", "error", "-y", "-i", video].iter().map(|s| s.to_string()).collect();
    for f in audio {
        a.extend(["-ss".into(), format!("{LEAD_IN_SECS}"), "-i".into(), f.clone()]);
    }
    match audio.len() {
        0 => {}
        1 => a.extend(["-map", "0:v:0", "-map", "1:a:0"].iter().map(|s| s.to_string())),
        _ => {
            // amix halves every input: volume=2 puts the sum back (the `normalize` option is missing from older ffmpeg builds)
            a.extend(["-filter_complex", "[1:a][2:a]amix=inputs=2:duration=longest,volume=2[a]", "-map", "0:v:0", "-map", "[a]"].iter().map(|s| s.to_string()));
        }
    }
    a.extend(["-c:v", "copy", "-c:a", "aac", "-b:a", "160k", "-shortest", "-movflags", "+faststart", out].iter().map(|s| s.to_string()));
    a
}

/// The 44-byte header of a 16-bit PCM WAV with `data_len` bytes of samples.
pub fn wav_header(rate: u32, channels: u16, data_len: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    let block = channels as u32 * 2;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes());
    h[22..24].copy_from_slice(&channels.to_le_bytes());
    h[24..28].copy_from_slice(&rate.to_le_bytes());
    h[28..32].copy_from_slice(&(rate * block).to_le_bytes());
    h[32..34].copy_from_slice(&(block as u16).to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

/// Frames of silence to add so that `written` frames cover `elapsed` (a lag under 60 ms is left alone: packets arrive in bursts).
pub fn silence_to_add(written: u64, elapsed: Duration, rate: u32) -> u64 {
    let due = (elapsed.as_secs_f64() * rate as f64) as u64;
    if due > written + (rate as u64 * 60 / 1000) { due - written } else { 0 }
}

/// Interleaved device samples -> 16-bit stereo (the first two channels; mono is doubled), appended to `out`.
pub fn to_stereo_i16(data: &[u8], f: &Fmt, out: &mut Vec<i16>) {
    let bytes = f.bits as usize / 8;
    let sample = |s: &[u8]| -> f32 {
        match (f.float, f.bits) {
            (true, _) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (false, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (false, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
        }
    };
    let q = |v: f32| (v.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    for fr in data.chunks_exact(bytes * f.channels) {
        let l = sample(&fr[..bytes]);
        let r = if f.channels > 1 { sample(&fr[bytes..2 * bytes]) } else { l };
        out.push(q(l));
        out.push(q(r));
    }
}

fn write_samples(w: &mut impl Write, s: &[i16]) -> std::io::Result<()> {
    for v in s {
        w.write_all(&v.to_le_bytes())?;
    }
    Ok(())
}

/// Records `src` into `path` until `stop`. Blocking: runs on its own thread.
fn record(src: Source, path: &Path, stop: &AtomicBool) -> Result<(), String> {
    let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {}", err.message().trim());
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(e("audio"))?;
        let dev = en.GetDefaultAudioEndpoint(if src == Source::System { eRender } else { eCapture }, eConsole).map_err(e("no device"))?;
        let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).map_err(e("activate"))?;
        let pf = client.GetMixFormat().map_err(e("mix format"))?;
        let cb = u16::from_le_bytes([*(pf as *const u8).add(16), *(pf as *const u8).add(17)]) as usize;
        let fmt = parse_format(std::slice::from_raw_parts(pf as *const u8, 18 + cb));
        let flags = if src == Source::System { AUDCLNT_STREAMFLAGS_LOOPBACK } else { 0 };
        let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, pf, None);
        CoTaskMemFree(Some(pf as *const _));
        let Some(fmt) = fmt else { return Err("unsupported sample format".into()) };
        init.map_err(e("initialize"))?;
        let cap: IAudioCaptureClient = client.GetService().map_err(e("capture service"))?;
        let file = std::fs::File::create(path).map_err(|x| x.to_string())?;
        let mut w = BufWriter::new(file);
        w.write_all(&wav_header(fmt.rate, 2, 0)).map_err(|x| x.to_string())?;
        client.Start().map_err(e("start"))?;
        let t0 = Instant::now();
        let (mut frames, mut buf) = (0u64, Vec::<i16>::new());
        let mut failure = None;
        loop {
            let stopping = stop.load(Ordering::Relaxed);
            // drain every waiting packet
            loop {
                match cap.GetNextPacketSize() {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(er) => {
                        failure = Some(format!("packet size: {er}"));
                        break;
                    }
                }
                let (mut p, mut n, mut fl) = (std::ptr::null_mut(), 0u32, 0u32);
                if let Err(er) = cap.GetBuffer(&mut p, &mut n, &mut fl, None, None) {
                    failure = Some(format!("get buffer: {er}"));
                    break;
                }
                buf.clear();
                if fl & BUFFERFLAGS_SILENT != 0 || p.is_null() {
                    buf.resize(n as usize * 2, 0);
                } else {
                    to_stereo_i16(std::slice::from_raw_parts(p, n as usize * fmt.bits as usize / 8 * fmt.channels), &fmt, &mut buf);
                }
                let _ = cap.ReleaseBuffer(n);
                write_samples(&mut w, &buf).map_err(|x| x.to_string())?;
                frames += n as u64;
            }
            if failure.is_some() {
                break;
            }
            let pad = silence_to_add(frames, t0.elapsed(), fmt.rate);
            if pad > 0 {
                write_samples(&mut w, &vec![0i16; pad as usize * 2]).map_err(|x| x.to_string())?;
                frames += pad;
            }
            if stopping {
                break;
            }
            std::thread::sleep(POLL);
        }
        let _ = client.Stop();
        w.flush().map_err(|x| x.to_string())?;
        let mut file = w.into_inner().map_err(|x| x.to_string())?;
        file.seek(SeekFrom::Start(0)).map_err(|x| x.to_string())?;
        file.write_all(&wav_header(fmt.rate, 2, (frames * 4).min(u32::MAX as u64 - 64) as u32)).map_err(|x| x.to_string())?;
        match failure {
            Some(f) if frames == 0 => Err(f),
            _ => Ok(()), // ponytail: a stream that died midway keeps what it recorded (the rest of the track is silent)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_header_is_a_valid_16_bit_stereo_header() {
        let h = wav_header(48_000, 2, 1000);
        assert_eq!(&h[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 1036);
        assert_eq!(u32::from_le_bytes(h[24..28].try_into().unwrap()), 48_000);
        assert_eq!(u32::from_le_bytes(h[28..32].try_into().unwrap()), 192_000); // byte rate
        assert_eq!(u16::from_le_bytes(h[32..34].try_into().unwrap()), 4); // block align
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 1000);
    }

    #[test]
    fn silence_fills_only_real_gaps() {
        let ms = Duration::from_millis;
        assert_eq!(silence_to_add(0, ms(1000), 48_000), 48_000);
        assert_eq!(silence_to_add(48_000, ms(1030), 48_000), 0, "a 30 ms lag is jitter");
        assert_eq!(silence_to_add(47_000, ms(1000), 48_000), 0, "21 ms behind");
        assert_eq!(silence_to_add(40_000, ms(1000), 48_000), 8_000);
        assert_eq!(silence_to_add(50_000, ms(1000), 48_000), 0, "ahead of the clock: never negative");
    }

    #[test]
    fn samples_become_16_bit_stereo() {
        let mut out = Vec::new();
        // float stereo: L = 0.5, R = -1.0 (+ a frame beyond full scale that must clamp)
        let f = Fmt { channels: 2, rate: 48_000, bits: 32, float: true };
        let bytes: Vec<u8> = [0.5f32, -1.0, 2.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
        to_stereo_i16(&bytes, &f, &mut out);
        assert_eq!(out, vec![16384, -32767, 32767, 0]);
        // 16-bit mono is doubled
        out.clear();
        let m = Fmt { channels: 1, rate: 16_000, bits: 16, float: false };
        to_stereo_i16(&[0x00, 0x40, 0x00, 0xC0], &m, &mut out);
        assert_eq!(out, vec![16384, 16384, -16384, -16384]);
        // a 5.1 frame keeps its first two channels; a torn trailing frame is ignored
        out.clear();
        let s = Fmt { channels: 6, rate: 48_000, bits: 16, float: false };
        let mut six: Vec<u8> = (1..=6i16).flat_map(|v| (v * 100).to_le_bytes()).collect();
        six.extend([1, 2, 3]);
        to_stereo_i16(&six, &s, &mut out);
        assert_eq!(out, vec![100, 200]);
    }

    #[test]
    fn mux_maps_one_track_or_mixes_two() {
        let one = mux_args("v.mp4", &["a.wav".into()], "o.mp4");
        assert!(one.windows(2).any(|w| w == ["-map", "1:a:0"]) && !one.contains(&"-filter_complex".to_string()));
        assert!(one.windows(2).any(|w| w == ["-c:v", "copy"]) && one.contains(&"-shortest".to_string()));
        assert_eq!(one.last().map(String::as_str), Some("o.mp4"));
        let two = mux_args("v.mp4", &["a.wav".into(), "b.wav".into()], "o.mp4");
        assert_eq!(two.iter().filter(|a| *a == "-i").count(), 3);
        assert!(two.iter().any(|a| a.starts_with("[1:a][2:a]amix=inputs=2")));
        assert!(two.windows(2).any(|w| w == ["-map", "[a]"]));
        // every audio input is trimmed by the lead-in, the video one is not
        assert_eq!(two.iter().filter(|a| *a == "-ss").count(), 2);
    }
}
