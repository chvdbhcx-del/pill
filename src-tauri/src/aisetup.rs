// First-run AI setup wizard (Settings > "AI setup", page id `aisetup`). Owner: aisetup agent.
// User request: "при установке предлагай скачать все для ии, whisper и локальную модельку, или подключать облачную".
// Callers: src/ai-setup.js (commands below), main.rs (aisetup::init + generate_handler), settingswin.rs PAGES ("aisetup").
// Local whisper: whisper.cpp release zip (GitHub, pinned tag + SHA-256) + one ggml model (Hugging Face, pinned commit + SHA-256)
//   -> %APPDATA%\pill\ai\whisper\{bin,models}; Pill runs whisper-server.exe hidden on 127.0.0.1, in a kill-on-close job object.
// Local chat: detects Ollama / LM Studio / llama.cpp / FreeLLMAPI on loopback, downloads the official OllamaSetup.exe on a click,
//   runs it ONLY on a second click, pulls a model through Ollama's own /api/pull.
// PRIVACY / NETWORK: nothing happens until the user clicks a button. Every download goes through net::request_checked with the
//   host patterns in HOSTS (each redirect hop re-checked, https only); loopback calls only to 127.0.0.1. No telemetry.
// Settings read: aiSetupDone, voice.enabled, voice.provider, voice.endpoint, net.ai, ai.baseUrl, ai.model,
//   aisetup.whisper.{enabled,model,port}. Settings written (through settings::apply_patch, so settings:changed fires):
//   aiSetupDone, voice.enabled, voice.provider=custom, voice.endpoint, voice.model=whisper-1, ai.baseUrl, ai.model, ai.preset, net.ai,
//   aisetup.whisper.{enabled,model,port}.
// Commands: aisetup_status aisetup_whisper_install aisetup_whisper_set aisetup_whisper_remove aisetup_ollama_download
//   aisetup_ollama_run_installer aisetup_ollama_pull aisetup_ai_use aisetup_cancel.
// Event: aisetup:progress {id: whisper|ollama-setup|ollama-pull, item, done, total, phase: download|verify|unpack|start|pull|done|error|cancelled, message}.
use crate::net::{self, Abort, Feature, Opts};
use crate::settings;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::ffi::c_void;
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const NO_WINDOW: u32 = 0x0800_0000; // CREATE_NO_WINDOW

// ---------------------------------------------------------------------------------------------------------------
// Catalog (pinned: a changed upstream file fails the SHA-256 check instead of being installed)
// ---------------------------------------------------------------------------------------------------------------

/// Hosts a download may touch, redirects included. `*` = [a-z0-9.-] only (net::host_matches).
const HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
    "huggingface.co",
    "cdn-lfs*.huggingface.co",
    "cdn-lfs*.hf.co",
    "*.cdn.hf.co", // Hugging Face's current CDN (us.aws.cdn.hf.co), seen in the live redirect chain
    "cas-bridge.xethub.hf.co",
    "ollama.com",
    "www.voidtools.com", // Everything + es.exe (everything.rs); a redirect to any other host is refused
    "voidtools.com",
];

pub(crate) struct Item {
    pub(crate) id: &'static str,
    pub(crate) url: &'static str,
    pub(crate) file: &'static str,
    pub(crate) size: u64,
    pub(crate) sha256: &'static str,
}

const WHISPER_TAG: &str = "v1.9.2";
static WHISPER_ZIP: Item = Item {
    id: "engine",
    url: "https://github.com/ggml-org/whisper.cpp/releases/download/v1.9.2/whisper-bin-x64.zip",
    file: "whisper-bin-x64.zip",
    size: 8_194_445,
    sha256: "49dcc16de826f20bd53d44f947a1ae49dfa81f86cad67a64d80820cb192d674a", // GitHub release API `digest` of the asset
};
// ggerganov/whisper.cpp @ 5359861c739e955e79d9a303bcbc70fb988958b1; sha256 = the LFS oid shown by the HF API (x-linked-etag)
static WHISPER_MODELS: [Item; 3] = [
    Item {
        id: "base",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-base.bin",
        file: "ggml-base.bin",
        size: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Item {
        id: "small",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin",
        file: "ggml-small.bin",
        size: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
    Item {
        id: "turbo",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q5_0.bin",
        file: "ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        sha256: "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2",
    },
];

const OLLAMA_SETUP_URL: &str = "https://ollama.com/download/OllamaSetup.exe";
const OLLAMA_SETUP_APPROX: u64 = 1_700_000_000; // measured 1_661_645_848 for v0.40.10; only for the progress bar
const OLLAMA_API: &str = "http://127.0.0.1:11434";
/// (name in the Ollama library, approximate download size in bytes) - names checked on ollama.com/library.
static OLLAMA_MODELS: [(&str, u64); 4] = [("llama3.2:3b", 2_000_000_000), ("qwen3:4b", 2_500_000_000), ("gemma3:4b", 3_400_000_000), ("qwen3:1.7b", 1_400_000_000)];

/// (id, name, OpenAI-compatible base URL, chat tab preset)
static SERVERS: [(&str, &str, &str, &str); 4] = [
    ("ollama", "Ollama", "http://127.0.0.1:11434/v1", "ollama"),
    ("freellm", "FreeLLMAPI", "http://127.0.0.1:3001/v1", "freellm"),
    ("lmstudio", "LM Studio", "http://127.0.0.1:1234/v1", "custom"),
    ("llamacpp", "llama.cpp", "http://127.0.0.1:8080/v1", "custom"),
];

// ---------------------------------------------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------------------------------------------

pub(crate) fn ai_dir() -> PathBuf {
    crate::store::dir().join("ai")
}
fn whisper_dir() -> PathBuf {
    ai_dir().join("whisper")
}
fn bin_dir() -> PathBuf {
    whisper_dir().join("bin")
}
fn bin_exe() -> PathBuf {
    bin_dir().join("whisper-server.exe")
}
fn models_dir() -> PathBuf {
    whisper_dir().join("models")
}
fn downloads_dir() -> PathBuf {
    ai_dir().join("downloads")
}
fn ollama_setup_path() -> PathBuf {
    downloads_dir().join("OllamaSetup.exe")
}
fn model_installed(m: &Item) -> bool {
    std::fs::metadata(models_dir().join(m.file)).is_ok_and(|md| md.len() == m.size) // size was checked + SHA-256 verified before the rename
}
fn selected_model() -> &'static Item {
    let id = settings::get_str("aisetup.whisper.model", "base");
    WHISPER_MODELS.iter().find(|m| m.id == id).unwrap_or(&WHISPER_MODELS[0])
}

// ---------------------------------------------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------------------------------------------

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn is_reserved_stem(stem: &str) -> bool {
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit())
}

/// Archive entry name -> normalised relative path with '/' separators, or None when it could escape the target folder:
/// "..", absolute paths, drive letters / stream colons, control or wildcard characters, Windows device names, trailing dots/spaces.
pub fn safe_name(name: &str) -> Option<String> {
    let n = name.replace('\\', "/");
    if n.is_empty() || n.starts_with('/') || n.bytes().any(|b| b < 0x20 || b":<>\"|?*".contains(&b)) {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for c in n.split('/') {
        if c.is_empty() || c == "." {
            continue;
        }
        if c == ".." || c.ends_with('.') || c.ends_with(' ') || is_reserved_stem(&c.split('.').next().unwrap_or("").to_ascii_lowercase()) {
            return None;
        }
        parts.push(c);
    }
    if parts.is_empty() { None } else { Some(parts.join("/")) }
}

/// Is this archive entry one we install (the server and the DLLs next to it)? Returns its file name.
fn wanted(rel: &str) -> Option<&str> {
    let base = rel.rsplit('/').next()?;
    let low = base.to_ascii_lowercase();
    (low == "whisper-server.exe" || low.ends_with(".dll")).then_some(base)
}

const MAX_ENTRY: u64 = 64 << 20;
const MAX_TOTAL: u64 = 256 << 20;

/// Unpack whisper-server.exe + *.dll (flat) into `out`. Any unsafe entry name rejects the whole archive. Entries are written to
/// `<out>.new` first and swapped in at the end, so a failed unpack never leaves a half-installed folder.
pub fn extract_server<R: Read + Seek>(src: R, out: &Path) -> Result<usize, String> {
    let mut ar = zip::ZipArchive::new(src).map_err(|e| format!("not a valid zip: {e}"))?;
    let tmp = out.with_file_name(format!("{}.new", out.file_name().and_then(|n| n.to_str()).unwrap_or("bin")));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot create {}: {e}", tmp.display()))?;
    let mut run = || -> Result<usize, String> {
        let (mut total, mut count, mut server) = (0u64, 0usize, false);
        for i in 0..ar.len() {
            let mut f = ar.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
            let rel = safe_name(f.name()).ok_or_else(|| format!("unsafe path in the archive: {}", f.name()))?;
            if !f.is_file() {
                continue;
            }
            let Some(base) = wanted(&rel) else { continue };
            let size = f.size();
            total += size;
            if size > MAX_ENTRY || total > MAX_TOTAL {
                return Err("the archive is larger than expected".into());
            }
            let mut o = File::create(tmp.join(base)).map_err(|e| format!("cannot write {base}: {e}"))?;
            let n = std::io::copy(&mut (&mut f).take(MAX_ENTRY + 1), &mut o).map_err(|e| format!("cannot unpack {base}: {e}"))?;
            if n != size {
                return Err(format!("{base}: unpacked {n} bytes, expected {size}"));
            }
            server |= base.eq_ignore_ascii_case("whisper-server.exe");
            count += 1;
        }
        if server { Ok(count) } else { Err("whisper-server.exe is not in the archive".into()) }
    };
    match run() {
        Ok(n) => {
            let _ = std::fs::remove_dir_all(out);
            std::fs::rename(&tmp, out).map_err(|e| format!("cannot install into {}: {e}", out.display()))?;
            Ok(n)
        }
        Err(e) => {
            let _ = std::fs::remove_dir_all(&tmp);
            Err(e)
        }
    }
}

#[derive(Debug, PartialEq)]
struct Pull {
    status: String,
    done: u64,
    total: u64,
    error: Option<String>,
}

/// One NDJSON line of Ollama's /api/pull stream: {"status":"pulling 4f2b..","digest":"..","total":N,"completed":M} or {"error":".."}.
fn parse_pull_line(line: &str) -> Option<Pull> {
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    Some(Pull {
        status: v.get("status").and_then(Value::as_str).unwrap_or("").to_string(),
        done: v.get("completed").and_then(Value::as_u64).unwrap_or(0),
        total: v.get("total").and_then(Value::as_u64).unwrap_or(0),
        error: v.get("error").and_then(Value::as_str).map(str::to_string),
    })
}

fn endpoint_for(port: u16) -> String {
    format!("http://127.0.0.1:{port}/v1")
}

fn free_port(start: u16) -> Option<u16> {
    (start..start.saturating_add(50)).find(|p| std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, *p)).is_ok())
}

// ---------------------------------------------------------------------------------------------------------------
// Jobs + progress
// ---------------------------------------------------------------------------------------------------------------

static JOB: Mutex<Option<(String, Abort)>> = Mutex::new(None);

fn job_lock() -> std::sync::MutexGuard<'static, Option<(String, Abort)>> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(app: &AppHandle, id: &str, item: &str, done: u64, total: u64, phase: &str, message: &str) {
    let _ = app.emit("aisetup:progress", json!({ "id": id, "item": item, "done": done, "total": total, "phase": phase, "message": message }));
}

/// Runs `f` on its own thread as the one active job; the outcome arrives as a progress event (done / error / cancelled).
pub(crate) fn spawn_job(app: AppHandle, id: &'static str, f: impl FnOnce(&AppHandle, &Abort) -> Result<String, String> + Send + 'static) -> Value {
    let abort = Abort::new();
    {
        let mut j = job_lock();
        if let Some((cur, _)) = j.as_ref() {
            return json!({ "ok": false, "error": format!("Another step is still running ({cur}). Wait for it or cancel it.") });
        }
        *j = Some((id.to_string(), abort.clone()));
    }
    let spawned = std::thread::Builder::new().name("aisetup".into()).spawn({
        let app = app.clone();
        move || {
            let r = f(&app, &abort);
            *job_lock() = None;
            match r {
                Ok(msg) => emit(&app, id, "", 0, 0, "done", &msg),
                Err(e) if e == "cancelled" => emit(&app, id, "", 0, 0, "cancelled", ""),
                Err(e) => emit(&app, id, "", 0, 0, "error", &e),
            }
        }
    });
    if spawned.is_err() {
        *job_lock() = None;
        return json!({ "ok": false, "error": "cannot start the worker thread" });
    }
    json!({ "ok": true })
}

#[tauri::command]
pub fn aisetup_cancel() -> Value {
    if let Some((_, a)) = job_lock().as_ref() {
        a.abort();
    }
    json!({ "ok": true })
}

pub(crate) fn host_list() -> Vec<String> {
    HOSTS.iter().map(|s| s.to_string()).collect()
}

fn part_path(dest: &Path) -> PathBuf {
    let mut n = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    n.push(".part");
    dest.with_file_name(n)
}

/// GET `url` into `dest` through the checked, redirect-aware client: `.part` file, running SHA-256, size and hash verified before
/// the rename. `est` is the progress total when the real size is unknown. Err("cancelled") = the user pressed Cancel.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download(app: &AppHandle, job: &str, item: &str, url: &str, dest: &Path, size: Option<u64>, sha: Option<&str>, est: u64, abort: &Abort) -> Result<String, String> {
    if let Some(d) = dest.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    }
    let part = part_path(dest);
    let file = File::create(&part).map_err(|e| format!("cannot write {}: {e}", part.display()))?;
    struct St {
        file: File,
        h: Sha256,
        done: u64,
        last: Instant,
        err: Option<String>,
    }
    let mut st = St { file, h: Sha256::new(), done: 0, last: Instant::now(), err: None };
    let total = size.unwrap_or(est);
    emit(app, job, item, 0, total, "download", "");
    let hosts = host_list();
    let res = {
        let mut cb = |chunk: &[u8]| -> bool {
            if let Err(e) = st.file.write_all(chunk) {
                st.err = Some(format!("cannot write the file (disk full?): {e}"));
                return false;
            }
            st.h.update(chunk);
            st.done += chunk.len() as u64;
            if st.last.elapsed() >= Duration::from_millis(200) {
                st.last = Instant::now();
                emit(app, job, item, st.done, total, "download", "");
            }
            true
        };
        let mut od: Option<&mut dyn FnMut(&[u8]) -> bool> = Some(&mut cb);
        // ponytail: no resume; a cancelled or failed download starts over. max_body is a hard stop at 3 GiB.
        net::request_checked(
            &Opts { feature: Feature::Setup, method: "GET", url, headers: &[("Accept", "*/*")], body: None, max_body: 3 << 30, recv_ms: 60_000, decompress: false, abort: Some(abort) },
            &mut od,
            &hosts,
        )
    };
    let St { file, h, done, err, .. } = st;
    let fail = |e: String| -> Result<String, String> {
        let _ = std::fs::remove_file(&part);
        Err(e)
    };
    let r = match res {
        Ok(r) => r,
        Err(e) => return fail(if e == "stopped" || abort.is_set() { "cancelled".into() } else { e }),
    };
    if let Some(e) = err {
        return fail(e);
    }
    if !(200..300).contains(&r.status) {
        return fail(format!("the server answered HTTP {} for {}", r.status, item));
    }
    emit(app, job, item, done, total, "verify", "");
    if let Some(s) = size {
        if done != s {
            return fail(format!("incomplete download: {done} of {s} bytes (connection dropped?)"));
        }
    } else if done == 0 {
        return fail("the server sent an empty file".into());
    }
    let got = hex(&h.finalize());
    if let Some(e) = sha {
        if !got.eq_ignore_ascii_case(e) {
            return fail(format!("SHA-256 mismatch for {item}: the file differs from the pinned one, it was deleted"));
        }
    }
    let _ = file.sync_all();
    drop(file);
    let _ = std::fs::remove_file(dest);
    std::fs::rename(&part, dest).map_err(|e| format!("cannot move the file into place: {e}"))?;
    Ok(got)
}

// ---------------------------------------------------------------------------------------------------------------
// whisper-server process (hidden, loopback only, dies with Pill)
// ---------------------------------------------------------------------------------------------------------------

struct Server {
    child: Child,
    port: u16,
}
static SERVER: Mutex<Option<Server>> = Mutex::new(None);
static KILL_JOB: OnceLock<isize> = OnceLock::new();

fn server_lock() -> std::sync::MutexGuard<'static, Option<Server>> {
    SERVER.lock().unwrap_or_else(|e| e.into_inner())
}

/// One kill-on-close job object for the whole run: when Pill exits or crashes the OS kills whisper-server with it.
fn kill_job() -> isize {
    *KILL_JOB.get_or_init(|| unsafe {
        let Ok(job) = CreateJobObjectW(None, PCWSTR::null()) else { return 0 };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let _ = SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const c_void, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        job.0 as isize
    })
}

fn running_port() -> Option<u16> {
    let mut g = server_lock();
    let alive = g.as_mut().map(|s| (matches!(s.child.try_wait(), Ok(None)), s.port));
    match alive {
        Some((true, port)) => Some(port),
        Some((false, _)) => {
            *g = None;
            None
        }
        None => None,
    }
}

fn stop_server() {
    if let Some(mut s) = server_lock().take() {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
}

/// Start whisper-server for the selected model (no-op when it already runs). The port is the stored one (8179), or the next free one.
fn start_server() -> Result<u16, String> {
    if let Some(p) = running_port() {
        return Ok(p);
    }
    let exe = bin_exe();
    if !exe.exists() {
        return Err("whisper-server.exe is not installed".into());
    }
    let model = selected_model();
    if !model_installed(model) {
        return Err(format!("the model {} is not installed", model.file));
    }
    let want = settings::get_f64("aisetup.whisper.port", 8179.0).clamp(1024.0, 65000.0) as u16;
    let port = free_port(want).ok_or("no free local port for the speech server")?;
    let log = File::create(whisper_dir().join("server.log")).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
    let mut g = server_lock();
    let mut child = Command::new(&exe)
        .current_dir(bin_dir())
        .args(["-m"])
        .arg(models_dir().join(model.file))
        .args(["--host", "127.0.0.1", "--port", &port.to_string(), "--inference-path", "/v1/audio/transcriptions", "-l", "auto"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .creation_flags(NO_WINDOW)
        .spawn()
        .map_err(|e| format!("cannot start whisper-server.exe: {e}"))?;
    let job = kill_job();
    if job != 0 {
        unsafe {
            let _ = AssignProcessToJobObject(HANDLE(job as *mut c_void), HANDLE(child.as_raw_handle()));
        }
    }
    std::thread::sleep(Duration::from_millis(700));
    if let Ok(Some(st)) = child.try_wait() {
        return Err(format!("whisper-server.exe exited at once ({st}); see {}", whisper_dir().join("server.log").display()));
    }
    *g = Some(Server { child, port });
    Ok(port)
}

fn patch(app: &AppHandle, kv: &[(&str, Value)]) {
    let mut m = Map::new();
    for (k, v) in kv {
        m.insert((*k).to_string(), v.clone());
    }
    let _ = settings::apply_patch(app, m);
}

/// Switch to `model`, (re)start the server and point the voice feature at it.
fn enable_whisper(app: &AppHandle, model: &str) -> Result<u16, String> {
    stop_server();
    settings::set("aisetup.whisper.model", json!(model));
    let port = start_server()?;
    patch(
        app,
        &[
            ("aisetup.whisper.enabled", json!(true)),
            ("aisetup.whisper.port", json!(port)),
            ("voice.enabled", json!(true)),
            ("voice.provider", json!("custom")),
            ("voice.endpoint", json!(endpoint_for(port))),
            ("voice.model", json!("whisper-1")),
            ("aiSetupDone", json!(true)),
        ],
    );
    Ok(port)
}

fn disable_whisper(app: &AppHandle) {
    stop_server();
    let ours = settings::get_str("voice.endpoint", "") == endpoint_for(settings::get_f64("aisetup.whisper.port", 8179.0) as u16);
    let mut kv = vec![("aisetup.whisper.enabled", json!(false))];
    if ours {
        kv.push(("voice.enabled", json!(false))); // its server is gone: dictation would only fail
    }
    patch(app, &kv);
}

// ---------------------------------------------------------------------------------------------------------------
// Commands: whisper
// ---------------------------------------------------------------------------------------------------------------

pub(crate) fn err(msg: &str) -> Value {
    json!({ "ok": false, "error": msg })
}

/// Download (what is missing of) engine + model, unpack, start the server, switch the voice settings. Progress: id "whisper".
#[tauri::command]
pub fn aisetup_whisper_install(app: AppHandle, model: String) -> Value {
    let Some(m) = WHISPER_MODELS.iter().find(|m| m.id == model) else { return err("unknown model") };
    spawn_job(app, "whisper", move |app, abort| {
        if !bin_exe().exists() {
            stop_server();
            let zip = downloads_dir().join(WHISPER_ZIP.file);
            download(app, "whisper", WHISPER_ZIP.id, WHISPER_ZIP.url, &zip, Some(WHISPER_ZIP.size), Some(WHISPER_ZIP.sha256), WHISPER_ZIP.size, abort)?;
            emit(app, "whisper", WHISPER_ZIP.id, 0, 0, "unpack", "");
            let r = std::fs::File::open(&zip).map_err(|e| e.to_string()).and_then(|f| extract_server(f, &bin_dir()));
            let _ = std::fs::remove_file(&zip);
            r?;
        }
        if !model_installed(m) {
            download(app, "whisper", m.id, m.url, &models_dir().join(m.file), Some(m.size), Some(m.sha256), m.size, abort)?;
        }
        emit(app, "whisper", m.id, 0, 0, "start", "");
        enable_whisper(app, m.id).map(endpoint_for)
    })
}

/// Switch the local speech server on (optionally with another installed model) or off.
#[tauri::command(async)]
pub fn aisetup_whisper_set(app: AppHandle, on: bool, model: Option<String>) -> Value {
    if !on {
        disable_whisper(&app);
        return json!({ "ok": true });
    }
    let m = match model.as_deref() {
        Some(id) => match WHISPER_MODELS.iter().find(|m| m.id == id) {
            Some(m) => m,
            None => return err("unknown model"),
        },
        None => selected_model(),
    };
    if !bin_exe().exists() || !model_installed(m) {
        return err("Download the engine and this model first.");
    }
    match enable_whisper(&app, m.id) {
        Ok(port) => json!({ "ok": true, "port": port }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

/// Stop the server and delete engine + models (%APPDATA%\pill\ai\whisper).
#[tauri::command(async)]
pub fn aisetup_whisper_remove(app: AppHandle) -> Value {
    if job_lock().is_some() {
        return err("A download is running.");
    }
    disable_whisper(&app);
    match std::fs::remove_dir_all(whisper_dir()) {
        Ok(()) => json!({ "ok": true }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => json!({ "ok": true }),
        Err(e) => json!({ "ok": false, "error": format!("cannot delete {}: {e}", whisper_dir().display()) }),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Commands: Ollama + other local servers
// ---------------------------------------------------------------------------------------------------------------

/// GET {base}/models on a loopback server -> (up, model ids). 401/403 counts as up (wants a key).
fn probe(base: &str) -> (bool, Vec<String>) {
    // A live loopback server accepts in milliseconds; WinHTTP would wait 8 s on a port that hangs instead of refusing.
    let port = base.split("127.0.0.1:").nth(1).and_then(|r| r.split('/').next()).and_then(|p| p.parse::<u16>().ok());
    if port.is_some_and(|p| std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], p)), Duration::from_millis(300)).is_err()) {
        return (false, Vec::new());
    }
    let url = format!("{base}/models");
    let o = Opts { feature: Feature::Setup, method: "GET", url: &url, headers: &[("Accept", "application/json")], body: None, max_body: 1 << 20, recv_ms: 2500, decompress: false, abort: None };
    match net::request_checked(&o, &mut None, &["127.0.0.1".to_string()]) {
        Ok(r) if r.status / 100 == 2 => {
            let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            let mut m: Vec<String> = v["data"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(str::to_string).collect();
            m.sort();
            m.dedup();
            m.truncate(300);
            (true, m)
        }
        Ok(r) => (r.status == 401 || r.status == 403, Vec::new()),
        Err(_) => (false, Vec::new()),
    }
}

fn ollama_exe_present() -> bool {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).is_some_and(|d| {
        let o = d.join("Programs").join("Ollama");
        o.join("ollama app.exe").exists() || o.join("ollama.exe").exists()
    })
}

/// Everything the wizard page draws: catalog with sizes and sources, what is installed, what is running locally.
#[tauri::command(async)]
pub fn aisetup_status() -> Value {
    let handles: Vec<_> = SERVERS
        .iter()
        .map(|&(id, name, url, _)| {
            std::thread::spawn(move || {
                let (up, models) = probe(url);
                json!({ "id": id, "name": name, "url": url, "up": up, "models": models })
            })
        })
        .collect();
    let servers: Vec<Value> = handles.into_iter().filter_map(|h| h.join().ok()).collect();
    let ollama_up = servers.iter().any(|s| s["id"] == "ollama" && s["up"] == true);
    let models: Vec<Value> = WHISPER_MODELS
        .iter()
        .map(|m| json!({ "id": m.id, "file": m.file, "size": m.size, "host": "huggingface.co", "installed": model_installed(m), "verified": true }))
        .collect();
    json!({
        "done": settings::get_bool("aiSetupDone", false),
        "dir": ai_dir().to_string_lossy(),
        "busy": job_lock().as_ref().map(|(id, _)| id.clone()),
        "whisper": {
            "engine": { "tag": WHISPER_TAG, "size": WHISPER_ZIP.size, "host": "github.com", "installed": bin_exe().exists(), "verified": true },
            "models": models,
            "enabled": settings::get_bool("aisetup.whisper.enabled", false),
            "running": running_port(),
            "model": selected_model().id,
            "endpoint": settings::get_str("voice.endpoint", ""),
            "voiceOn": settings::get_bool("voice.enabled", false),
        },
        "ollama": {
            "installed": ollama_up || ollama_exe_present(),
            "up": ollama_up,
            "installer": { "host": "ollama.com", "size": OLLAMA_SETUP_APPROX, "ready": ollama_setup_path().exists(), "verified": false },
            "catalog": OLLAMA_MODELS.iter().map(|(n, s)| json!({ "name": n, "size": s })).collect::<Vec<_>>(),
        },
        "servers": servers,
        "ai": { "on": settings::get_bool("net.ai", false), "baseUrl": settings::get_str("ai.baseUrl", ""), "model": settings::get_str("ai.model", "") },
    })
}

/// Download the official OllamaSetup.exe (ollama.com -> GitHub release CDN). It is NOT started here. Progress: id "ollama-setup".
#[tauri::command]
pub fn aisetup_ollama_download(app: AppHandle) -> Value {
    spawn_job(app, "ollama-setup", |app, abort| {
        let dest = ollama_setup_path();
        download(app, "ollama-setup", "installer", OLLAMA_SETUP_URL, &dest, None, None, OLLAMA_SETUP_APPROX, abort)?;
        let mut mz = [0u8; 2];
        let ok = File::open(&dest).and_then(|mut f| f.read_exact(&mut mz)).is_ok() && &mz == b"MZ";
        if !ok {
            let _ = std::fs::remove_file(&dest);
            return Err("the downloaded file is not a Windows program".into());
        }
        Ok(dest.to_string_lossy().into_owned())
    })
}

/// Second explicit click: start the downloaded installer through the shell (its own window, its own UAC prompt if it wants one).
#[tauri::command]
pub fn aisetup_ollama_run_installer() -> Value {
    run_installer(&ollama_setup_path())
}

/// ShellExecute "open" of a downloaded installer (it shows its own UAC prompt); never started without a click.
pub(crate) fn run_installer(p: &Path) -> Value {
    if !p.exists() {
        return err("Download the installer first.");
    }
    let wide: Vec<u16> = p.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    let r = unsafe { ShellExecuteW(HWND::default(), w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 { json!({ "ok": true }) } else { err("Windows could not start the installer.") }
}

/// Ask the local Ollama to download `model` (a name from the catalog). Streamed progress, id "ollama-pull".
#[tauri::command]
pub fn aisetup_ollama_pull(app: AppHandle, model: String) -> Value {
    let Some(&(name, size)) = OLLAMA_MODELS.iter().find(|(n, _)| *n == model) else { return err("unknown model") };
    spawn_job(app, "ollama-pull", move |app, abort| {
        let body = json!({ "model": name, "stream": true }).to_string();
        let url = format!("{OLLAMA_API}/api/pull");
        let (mut buf, mut ok, mut fail): (Vec<u8>, bool, Option<String>) = (Vec::new(), false, None);
        let mut last = Instant::now() - Duration::from_secs(1);
        let res = {
            let mut cb = |chunk: &[u8]| -> bool {
                buf.extend_from_slice(chunk);
                while let Some(i) = buf.iter().position(|b| *b == b'\n') {
                    let line: Vec<u8> = buf.drain(..=i).collect();
                    let Some(p) = parse_pull_line(&String::from_utf8_lossy(&line)) else { continue };
                    if let Some(e) = p.error {
                        fail = Some(e);
                        return false;
                    }
                    ok |= p.status == "success";
                    if last.elapsed() >= Duration::from_millis(200) || p.status == "success" {
                        last = Instant::now();
                        emit(app, "ollama-pull", name, p.done, if p.total > 0 { p.total } else { size }, "pull", &p.status);
                    }
                }
                true
            };
            let mut od: Option<&mut dyn FnMut(&[u8]) -> bool> = Some(&mut cb);
            net::request_checked(
                &Opts { feature: Feature::Setup, method: "POST", url: &url, headers: &[("Content-Type", "application/json"), ("Accept", "application/x-ndjson")], body: Some(body.as_bytes()), max_body: 512 << 20, recv_ms: 300_000, decompress: false, abort: Some(abort) },
                &mut od,
                &["127.0.0.1".to_string()],
            )
        };
        let r = match res {
            Ok(r) => r,
            Err(e) if e == "stopped" || abort.is_set() => return Err("cancelled".into()),
            Err(e) => return Err(format!("Ollama is not answering ({e}). Start Ollama and try again.")),
        };
        if let Some(e) = fail {
            return Err(e);
        }
        if !(200..300).contains(&r.status) {
            let msg = serde_json::from_slice::<Value>(&r.body).ok().and_then(|v| v["error"].as_str().map(str::to_string)).unwrap_or_default();
            return Err(format!("Ollama answered HTTP {} {msg}", r.status));
        }
        if ok { Ok(name.to_string()) } else { Err("the download ended before Ollama reported success".into()) }
    })
}

/// Point the chat at a detected local server: `server` is an id from SERVERS, `model` one of its models. Turns the AI feature on.
#[tauri::command]
pub fn aisetup_ai_use(app: AppHandle, server: String, model: String) -> Value {
    let Some(&(_, _, url, preset)) = SERVERS.iter().find(|s| s.0 == server) else { return err("unknown server") };
    let model = model.trim();
    if model.is_empty() || model.len() > 200 || model.chars().any(|c| c.is_control()) {
        return err("choose a model");
    }
    patch(&app, &[("ai.baseUrl", json!(url)), ("ai.model", json!(model)), ("ai.preset", json!(preset)), ("net.ai", json!(true)), ("aiSetupDone", json!(true))]);
    json!({ "ok": true })
}

// ---------------------------------------------------------------------------------------------------------------
// Startup: restart the speech server when it was set up, and show the wizard once if the user has not decided yet
// ---------------------------------------------------------------------------------------------------------------

pub fn init(app: &tauri::App) {
    let h = app.handle().clone();
    let _ = std::thread::Builder::new().name("aisetup-init".into()).spawn(move || {
        if settings::get_bool("aisetup.whisper.enabled", false) && bin_exe().exists() && model_installed(selected_model()) {
            let prev = settings::get_f64("aisetup.whisper.port", 8179.0) as u16;
            match start_server() {
                Ok(port) if port != prev => {
                    // the stored port was busy: follow it, but only when voice.endpoint is still the one this wizard wrote
                    let moved = settings::get_str("voice.endpoint", "") == endpoint_for(prev);
                    let mut kv = vec![("aisetup.whisper.port", json!(port))];
                    if moved {
                        kv.push(("voice.endpoint", json!(endpoint_for(port))));
                    }
                    patch(&h, &kv);
                }
                Ok(_) => {}
                Err(e) => eprintln!("aisetup: {e}"),
            }
        }
        std::thread::sleep(Duration::from_millis(3300));
        let configured = settings::get_bool("voice.enabled", false) && settings::get_bool("net.ai", false);
        if !settings::get_bool("aiSetupDone", false) && !configured && !settings::first_run_pending() { // a new install gets the first-run wizard, which ends by pointing here
            crate::settingswin::open(&h, Some("aisetup"));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn aisetup_safe_names() {
        assert_eq!(safe_name("Release/whisper-server.exe").as_deref(), Some("Release/whisper-server.exe"));
        assert_eq!(safe_name("Release\\ggml.dll").as_deref(), Some("Release/ggml.dll"));
        assert_eq!(safe_name("./a//b.dll").as_deref(), Some("a/b.dll"));
        for bad in ["../evil.dll", "a/../../evil.dll", "/etc/passwd", "\\\\srv\\share\\x.dll", "C:/x.dll", "a/b.dll:stream", "a/con.dll", "NUL", "x/com1.txt", "a/b.", "", "///", "a\0b"] {
            assert!(safe_name(bad).is_none(), "{bad:?}");
        }
        assert_eq!(wanted("Release/whisper-server.exe"), Some("whisper-server.exe"));
        assert_eq!(wanted("Release/GGML-base.DLL"), Some("GGML-base.DLL"));
        assert_eq!(wanted("Release/whisper-cli.exe"), None);
    }

    #[test]
    fn aisetup_download_hosts_and_pins() {
        let allowed = |h: &str| HOSTS.iter().any(|p| net::host_matches(p, h));
        let mut urls = vec![WHISPER_ZIP.url, OLLAMA_SETUP_URL];
        urls.extend(WHISPER_MODELS.iter().map(|m| m.url));
        for u in urls {
            let p = net::parse_url(u).unwrap();
            assert!(p.https && allowed(&p.host), "{u}");
        }
        for it in std::iter::once(&WHISPER_ZIP).chain(WHISPER_MODELS.iter()) {
            assert_eq!(it.sha256.len(), 64, "{}", it.id);
            assert!(it.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(it.url.ends_with(it.file));
        }
        for ok in ["github.com", "release-assets.githubusercontent.com", "cdn-lfs.huggingface.co", "cdn-lfs-us-1.huggingface.co", "us.aws.cdn.hf.co", "cas-bridge.xethub.hf.co", "ollama.com", "www.voidtools.com", "voidtools.com"] {
            assert!(allowed(ok), "{ok}");
        }
        for bad in ["evilgithub.com", "github.com.evil.io", "evilhuggingface.co", "cdn-lfs.huggingface.co.evil.io", "api.github.com", "hf.co", "evil.cdn.hf.co.x.io", "cdn-lfs/x.huggingface.co", "127.0.0.1", "ollama.com.evil.io", "evilvoidtools.com", "www.voidtools.com.evil.io", "cdn.voidtools.com"] {
            assert!(!allowed(bad), "{bad}");
        }
    }

    #[test]
    fn aisetup_extract_server_zip() {
        use zip::write::SimpleFileOptions;
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        let build = |names: &[&str]| {
            let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for n in names {
                zw.start_file(*n, opt).unwrap();
                zw.write_all(b"MZdata").unwrap();
            }
            zw.finish().unwrap().into_inner()
        };
        let base = std::env::temp_dir().join(format!("pill-aisetup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let out = base.join("bin");
        let n = extract_server(Cursor::new(build(&["Release/whisper-server.exe", "Release/ggml.dll", "Release/main.exe", "Release/readme.txt"])), &out).unwrap();
        assert_eq!(n, 2);
        assert!(out.join("whisper-server.exe").exists() && out.join("ggml.dll").exists() && !out.join("main.exe").exists());
        assert!(!base.join("bin.new").exists());
        // a traversal entry rejects the whole archive and leaves the installed folder alone
        let e = extract_server(Cursor::new(build(&["Release/whisper-server.exe", "../evil.dll"])), &out).unwrap_err();
        assert!(e.contains("unsafe path"), "{e}");
        assert!(out.join("whisper-server.exe").exists() && !base.join("evil.dll").exists() && !base.join("bin.new").exists());
        // no server inside
        assert!(extract_server(Cursor::new(build(&["Release/ggml.dll"])), &out).unwrap_err().contains("whisper-server.exe"));
        assert!(extract_server(Cursor::new(b"not a zip".to_vec()), &out).is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Deflate path with the real release zip: set PILL_TEST_WHISPER_ZIP to a downloaded whisper-bin-x64.zip to run it.
    #[test]
    fn aisetup_extract_real_zip_if_given() {
        let Some(p) = std::env::var_os("PILL_TEST_WHISPER_ZIP") else { return };
        let out = std::env::temp_dir().join(format!("pill-aisetup-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let n = extract_server(File::open(p).unwrap(), &out.join("bin")).unwrap();
        assert!(n >= 2 && out.join("bin").join("whisper-server.exe").metadata().unwrap().len() == 725_504);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// Live redirect chains of the three download sources (PILL_TEST_NET=1): each must reach a 200 on an allowed host; the body is cut
    /// after the first chunk, except the 8 MB engine zip, which is hashed completely against the pinned SHA-256.
    #[test]
    fn aisetup_live_redirect_chains() {
        if std::env::var_os("PILL_TEST_NET").is_none() {
            return;
        }
        let hosts = host_list();
        for (url, full) in [(WHISPER_ZIP.url, true), (WHISPER_MODELS[0].url, false), (OLLAMA_SETUP_URL, false)] {
            let (mut h, mut n) = (Sha256::new(), 0usize);
            let mut cb = |c: &[u8]| {
                h.update(c);
                n += c.len();
                full
            };
            let mut od: Option<&mut dyn FnMut(&[u8]) -> bool> = Some(&mut cb);
            let r = net::request_checked(&Opts { feature: Feature::Setup, method: "GET", url, headers: &[], body: None, max_body: 3 << 30, recv_ms: 60_000, decompress: false, abort: None }, &mut od, &hosts).unwrap();
            assert_eq!(r.status, 200, "{url}");
            assert!(n > 0, "{url}");
            if full {
                assert_eq!(n as u64, WHISPER_ZIP.size);
                assert_eq!(hex(&h.finalize()), WHISPER_ZIP.sha256);
            }
        }
    }

    #[test]
    fn aisetup_pull_lines_and_helpers() {
        assert_eq!(parse_pull_line(r#"{"status":"pulling manifest"}"#), Some(Pull { status: "pulling manifest".into(), done: 0, total: 0, error: None }));
        let p = parse_pull_line(r#"{"status":"pulling 4f2b","digest":"sha256:4f2b","total":2019377376,"completed":123456}"#).unwrap();
        assert_eq!((p.done, p.total), (123456, 2019377376));
        assert_eq!(parse_pull_line(r#"{"error":"pull model manifest: file does not exist"}"#).unwrap().error.as_deref(), Some("pull model manifest: file does not exist"));
        assert!(parse_pull_line("not json").is_none());
        assert_eq!(hex(&[0, 15, 255]), "000fff");
        assert_eq!(endpoint_for(8179), "http://127.0.0.1:8179/v1");
        // a busy port is skipped
        let l = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let busy = l.local_addr().unwrap().port();
        assert_ne!(free_port(busy), Some(busy));
        assert!(OLLAMA_MODELS.iter().all(|(n, s)| n.chars().all(|c| c.is_ascii_alphanumeric() || ".:-".contains(c)) && *s > 0));
    }
}
