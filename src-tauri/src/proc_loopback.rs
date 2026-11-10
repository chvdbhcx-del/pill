// WASAPI process loopback: the sound of ONE player (and its child processes) instead of the whole default render device.
// Windows 10 2004+ / Windows 11: ActivateAudioInterfaceAsync("VAD\Process_Loopback") with AUDIOCLIENT_ACTIVATION_PARAMS in a VT_BLOB PROPVARIANT.
// Unlike a device client this one has no mix format (GetMixFormat is E_NOTIMPL): the caller names the format, WASAPI converts into it.
// Why: mixers (SteelSeries Sonar, Voicemeeter, Equalizer APO routers...) route the player to a virtual device, so the loopback of the
// *default* device is empty and the music-reactive edge stays flat.
// PRIVACY: samples are only handed to audio.rs's in-memory analyzer; nothing is stored or sent.
use windows::core::{implement, Interface, HRESULT, IUnknown, PROPVARIANT};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::{
    ActivateAudioInterfaceAsync, IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient, AUDCLNT_SHAREMODE_SHARED,
    AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_EVENTCALLBACK, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, AUDIOCLIENT_ACTIVATION_PARAMS, AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
    AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS, PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE, VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
    WAVEFORMATEX,
};
use windows::Win32::System::Threading::{CreateEventW, SetEvent, WaitForSingleObject};

/// The format we ask WASAPI for: float32 stereo at 48 kHz (the analyzer's native one).
pub const CHANNELS: usize = 2;
pub const RATE: u32 = 48_000;

pub fn wave_format() -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: 3, // WAVE_FORMAT_IEEE_FLOAT
        nChannels: CHANNELS as u16,
        nSamplesPerSec: RATE,
        nAvgBytesPerSec: RATE * CHANNELS as u32 * 4,
        nBlockAlign: CHANNELS as u16 * 4,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// Record `pid` and everything it starts (a browser's audio service is a child of the browser).
pub fn activation_params(pid: u32) -> AUDIOCLIENT_ACTIVATION_PARAMS {
    let mut p = AUDIOCLIENT_ACTIVATION_PARAMS::default();
    p.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
    p.Anonymous.ProcessLoopbackParams =
        AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS { TargetProcessId: pid, ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE };
    p
}

/// A PROPVARIANT of type VT_BLOB, laid out by hand (x64: vt at 0, BLOB.cbSize at 8, BLOB.pBlobData at 16): the typed wrapper would need
/// an owned allocation that PropVariantClear frees, and this one only points at the caller's stack for the duration of the call.
#[repr(C)]
struct BlobVariant {
    vt: u16,
    reserved: [u16; 3],
    cb: u32,
    pad: u32,
    data: *const u8,
}

const VT_BLOB: u16 = 65;

fn blob_variant(data: *const u8, cb: u32) -> BlobVariant {
    BlobVariant { vt: VT_BLOB, reserved: [0; 3], cb, pad: 0, data }
}

#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Done(isize); // the event handle; `#[implement]` objects answer IAgileObject, which ActivateAudioInterfaceAsync requires

impl IActivateAudioInterfaceCompletionHandler_Impl for Done_Impl {
    fn ActivateCompleted(&self, _op: Option<&IActivateAudioInterfaceAsyncOperation>) -> windows::core::Result<()> {
        unsafe { SetEvent(HANDLE(self.0 as _)) }
    }
}

/// An opened process-loopback stream. `event` is signalled when data is ready (the caller waits on it instead of sleeping).
pub struct Stream {
    pub client: IAudioClient,
    pub cap: IAudioCaptureClient,
    pub event: HANDLE,
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
            let _ = CloseHandle(self.event);
        }
    }
}

/// Activates and starts the capture of `pid`'s sound. Err: unsupported Windows build, the process is gone, or WASAPI refused the format.
pub fn open(pid: u32) -> Result<Stream, String> {
    let e = |what: &'static str| move |err: windows::core::Error| format!("{what}: {err}");
    unsafe {
        let params = activation_params(pid);
        let pv = blob_variant(&params as *const _ as *const u8, std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32);
        let done = CreateEventW(None, false, false, None).map_err(e("event"))?;
        let handler: IActivateAudioInterfaceCompletionHandler = Done(done.0 as isize).into();
        let op = ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&pv as *const BlobVariant as *const PROPVARIANT),
            &handler,
        );
        let waited = match &op {
            Ok(_) => WaitForSingleObject(done, 5000),
            Err(_) => WAIT_OBJECT_0,
        };
        if waited == WAIT_OBJECT_0 {
            let _ = CloseHandle(done); // on a timeout the handler may still fire: leak the handle rather than signal a recycled one
        }
        let op = op.map_err(e("activate"))?;
        if waited != WAIT_OBJECT_0 {
            return Err("activate: timed out".into());
        }
        let (mut hr, mut iface) = (HRESULT(0), None::<IUnknown>);
        op.GetActivateResult(&mut hr, &mut iface).map_err(e("activate result"))?;
        hr.ok().map_err(e("activate result"))?;
        let client: IAudioClient = iface.ok_or("activate: no interface")?.cast().map_err(e("audio client"))?;
        let fmt = wave_format();
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                2_000_000, // 200 ms
                0,
                &fmt,
                None,
            )
            .map_err(e("initialize"))?;
        let event = CreateEventW(None, false, false, None).map_err(e("event"))?;
        let stream = (|| -> windows::core::Result<Stream> {
            client.SetEventHandle(event)?;
            let cap: IAudioCaptureClient = client.GetService()?;
            client.Start()?;
            Ok(Stream { client: client.clone(), cap, event })
        })();
        stream.map_err(|err| {
            let _ = CloseHandle(event);
            format!("start: {err}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Media::Audio::AUDIOCLIENT_ACTIVATION_TYPE;

    #[test]
    fn params_target_the_process_tree() {
        let p = activation_params(4242);
        assert_eq!(p.ActivationType, AUDIOCLIENT_ACTIVATION_TYPE(1), "AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK");
        let l = unsafe { p.Anonymous.ProcessLoopbackParams };
        assert_eq!(l.TargetProcessId, 4242);
        assert_eq!(l.ProcessLoopbackMode.0, 0, "INCLUDE_TARGET_PROCESS_TREE");
        assert_eq!(std::mem::size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>(), 12);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn blob_variant_has_the_propvariant_layout() {
        assert_eq!(std::mem::size_of::<BlobVariant>(), std::mem::size_of::<PROPVARIANT>());
        let b = [7u8; 12];
        let v = blob_variant(b.as_ptr(), 12);
        let raw = &v as *const BlobVariant as *const u8;
        unsafe {
            assert_eq!(*(raw as *const u16), 65, "vt = VT_BLOB");
            assert_eq!(*(raw.add(8) as *const u32), 12, "cbSize");
            assert_eq!(*(raw.add(16) as *const *const u8), b.as_ptr(), "pBlobData");
        }
    }

    #[test]
    fn format_is_consistent_float_stereo() {
        let f = wave_format();
        let (tag, ch, rate, avg, align, bits) = (f.wFormatTag, f.nChannels, f.nSamplesPerSec, f.nAvgBytesPerSec, f.nBlockAlign, f.wBitsPerSample);
        assert_eq!((tag, ch, rate, bits), (3, 2, 48_000, 32));
        assert_eq!(align, ch * bits / 8);
        assert_eq!(avg, rate * align as u32);
    }
}
