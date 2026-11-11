// File transfer through the relay (docs/phone-relay.md "Files over the relay"): `file.chunk` / `file.ack` / `file.end` / `file.abort` as v1-style
// messages inside the encrypted frames, both directions, streamed to / from disk (never the whole file in memory), at most 8 chunks in flight.
// The offer / accept handshake, the name sanitising, the Downloads\Pill target and the transfer table are the LAN ones (files.rs, server.rs).
use super::files::{self, Dir, Refused, Slot, Status, Transfer};
use super::reg::Device;
use super::relay::b64_any;
use super::server::{Conn, Route, Shared};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Raw bytes per chunk (the spec's limit: 96 KiB).
pub const CHUNK: usize = 96 * 1024;
/// Chunks a sender may have unacknowledged.
pub const WINDOW: u64 = 8;
/// A receiver acknowledges at least every this many chunks (and at the end).
const ACK_EVERY: u32 = 4;
/// The relay drops more than 4 MiB/s per room: a sender stays well below it.
const PACE_BYTES_PER_SEC: f64 = 3.0 * 1024.0 * 1024.0;
/// A transfer with no progress for this long is dropped.
const STALL: Duration = Duration::from_secs(60);

struct Rx {
    dev: String,
    conn: u64,
    file: File,
    path: PathBuf,
    size: u64,
    next: u64,
    unacked: u32,
    last_emit: Instant,
    last_rx: Instant,
    _slot: Slot,
}

struct Tx {
    dev: String,
    conn: u64,
    cancel: AtomicBool,
    acked: Mutex<u64>,
    cv: Condvar,
}

#[derive(Default)]
pub struct State {
    rx: Mutex<HashMap<String, Rx>>,
    tx: Mutex<HashMap<String, Arc<Tx>>>,
}

fn clip(v: &Value) -> String {
    v.as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(64).collect()
}

impl Shared {
    fn rf_abort_msg(&self, conn: &Conn, id: &str, reason: &str) {
        self.send_conn(conn, &json!({ "t": "file.abort", "id": id, "reason": reason }));
    }

    /// Drops a receiving transfer: closes and deletes the partial file, marks it failed, tells the UI (and the phone when asked).
    fn rf_fail_rx(&self, rx: &mut HashMap<String, Rx>, id: &str, tell: Option<(&Conn, &str)>) {
        if let Some(st) = rx.remove(id) {
            drop(st.file);
            let _ = std::fs::remove_file(&st.path);
        }
        if let Some(t) = self.transfers.update(id, |t| t.status = Status::Failed) {
            self.emit("phone:files", json!({ "kind": "failed", "transfer": t.public() }));
        }
        if let Some((c, why)) = tell {
            self.rf_abort_msg(c, id, why);
        }
    }

    /// `file.chunk` from a phone: appends to the file of an accepted offer.
    pub(super) fn rf_chunk(&self, dev: &Device, conn: &Conn, m: &Value) {
        if conn.route != Route::Relay {
            return;
        }
        let id = clip(&m["id"]);
        if !files::valid_id(&id) {
            return;
        }
        let (Some(off), Some(d)) = (m["off"].as_u64(), m["d"].as_str()) else { return self.rf_abort_msg(conn, &id, "bad_chunk") };
        let mut rx = self.rf.rx.lock().unwrap_or_else(|e| e.into_inner());
        let Some(raw) = b64_any(d).filter(|r| r.len() <= CHUNK) else { return self.rf_fail_rx(&mut rx, &id, Some((conn, "bad_chunk"))) };
        if !rx.contains_key(&id) {
            if off != 0 {
                return self.rf_abort_msg(conn, &id, "unknown_file");
            }
            let c = self.cfg();
            let (t, slot) = match self.transfers.begin_upload(&id, &dev.id) {
                Ok(x) => x,
                Err(Refused::Unknown) => return self.rf_abort_msg(conn, &id, "unknown_file"),
                Err(Refused::Busy) => return self.rf_abort_msg(conn, &id, "busy"),
            };
            if t.size > c.max_bytes || t.size > c.relay_max_bytes {
                return self.rf_fail_rx(&mut rx, &id, Some((conn, "too_big")));
            }
            let Ok((file, path)) = files::create_unique(&c.downloads, &t.name) else { return self.rf_fail_rx(&mut rx, &id, Some((conn, "disk"))) };
            self.emit("phone:files", json!({ "kind": "start", "transfer": self.transfers.get(&id).map(|t| t.public()) }));
            rx.insert(id.clone(), Rx { dev: dev.id.clone(), conn: conn.id, file, path, size: t.size, next: 0, unacked: 0, last_emit: Instant::now(), last_rx: Instant::now(), _slot: slot });
        }
        let Some(st) = rx.get_mut(&id) else { return };
        if st.dev != dev.id || st.conn != conn.id {
            return;
        }
        if off < st.next {
            return; // a chunk we already have
        }
        if off > st.next {
            return self.rf_fail_rx(&mut rx, &id, Some((conn, "gap")));
        }
        if st.next + raw.len() as u64 > st.size {
            return self.rf_fail_rx(&mut rx, &id, Some((conn, "too_large")));
        }
        if st.file.write_all(&raw).is_err() {
            return self.rf_fail_rx(&mut rx, &id, Some((conn, "disk")));
        }
        st.next += raw.len() as u64;
        st.unacked += 1;
        st.last_rx = Instant::now();
        if st.unacked >= ACK_EVERY || st.next == st.size {
            st.unacked = 0;
            self.send_conn(conn, &json!({ "t": "file.ack", "id": id, "upto": st.next }));
        }
        if st.last_emit.elapsed() > Duration::from_millis(300) {
            st.last_emit = Instant::now();
            let n = st.next;
            if let Some(t) = self.transfers.update(&id, |t| t.bytes = n) {
                self.emit("phone:files", json!({ "kind": "progress", "transfer": t.public() }));
            }
        }
    }

    /// `file.end` from a phone: checks the size and finishes the file.
    pub(super) fn rf_end(&self, dev: &Device, conn: &Conn, m: &Value) {
        if conn.route != Route::Relay {
            return;
        }
        let id = clip(&m["id"]);
        let size = m["size"].as_u64();
        let mut rx = self.rf.rx.lock().unwrap_or_else(|e| e.into_inner());
        let done = match rx.remove(&id) {
            Some(st) if st.dev == dev.id => {
                if st.next != st.size || size != Some(st.size) {
                    rx.insert(id.clone(), st);
                    return self.rf_fail_rx(&mut rx, &id, Some((conn, "size_mismatch")));
                }
                drop(st.file);
                Some((st.path, st.size))
            }
            Some(st) => {
                rx.insert(id, st);
                return;
            }
            // an empty file has no chunks: the end message alone creates it
            None => {
                let c = self.cfg();
                match self.transfers.get(&id) {
                    Some(t) if t.dir == Dir::In && t.device == dev.id && t.status == Status::Accepted && t.size == 0 && size == Some(0) => match (self.transfers.begin_upload(&id, &dev.id), files::create_unique(&c.downloads, &t.name)) {
                        (Ok(_), Ok((_f, path))) => Some((path, 0)),
                        _ => return self.rf_fail_rx(&mut rx, &id, Some((conn, "disk"))),
                    },
                    _ => return self.rf_abort_msg(conn, &id, "unknown_file"),
                }
            }
        };
        drop(rx);
        let Some((path, n)) = done else { return };
        if let Some(t) = self.transfers.update(&id, |t| {
            t.status = Status::Done;
            t.bytes = n;
            t.path = Some(path.clone());
        }) {
            self.bump(&dev.id, |s| {
                s.files_in += 1;
                s.bytes_in += n;
            });
            self.emit("phone:files", json!({ "kind": "done", "transfer": t.public() }));
        }
        self.send_conn(conn, &json!({ "t": "file.ack", "id": id, "upto": n })); // the final ack is the receipt
    }

    /// `file.ack` from a phone: frees the window of a file we are sending.
    pub(super) fn rf_ack(&self, dev: &Device, conn: &Conn, m: &Value) {
        let id = clip(&m["id"]);
        let tx = self.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).get(&id).cloned();
        if let (Some(tx), Some(upto)) = (tx.filter(|t| t.dev == dev.id && t.conn == conn.id), m["upto"].as_u64()) {
            let mut a = tx.acked.lock().unwrap_or_else(|e| e.into_inner());
            *a = (*a).max(upto);
            tx.cv.notify_all();
        }
    }

    /// `file.abort` from a phone, for either direction.
    pub(super) fn rf_abort(&self, dev: &Device, _conn: &Conn, m: &Value) {
        let id = clip(&m["id"]);
        {
            let mut rx = self.rf.rx.lock().unwrap_or_else(|e| e.into_inner());
            if rx.get(&id).is_some_and(|s| s.dev == dev.id) {
                self.rf_fail_rx(&mut rx, &id, None);
            }
        }
        if let Some(tx) = self.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).get(&id).filter(|t| t.dev == dev.id) {
            tx.cancel.store(true, Ordering::SeqCst);
            tx.cv.notify_all();
        }
    }

    /// `file.accept` from a phone over the relay: it wants the PC file it was offered, so stream it.
    pub(super) fn rf_accept(&self, dev: &Device, conn: &Conn, m: &Value) {
        if conn.route != Route::Relay {
            return;
        }
        let id = clip(&m["id"]);
        let Some(t) = self.transfers.get(&id).filter(|t| t.dir == Dir::Out && t.device == dev.id && t.status == Status::Accepted) else { return };
        let (Some(me), Some(conn)) = (self.me_arc(), self.channels(&dev.id).into_iter().find(|c| c.id == conn.id)) else { return };
        let slot = match self.transfers.begin_download(&id, &dev.id) {
            Ok((_, s)) => s,
            Err(_) => return self.rf_abort_msg(&conn, &id, "busy"),
        };
        let tx = Arc::new(Tx { dev: dev.id.clone(), conn: conn.id, cancel: AtomicBool::new(false), acked: Mutex::new(0), cv: Condvar::new() });
        self.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), tx.clone());
        self.transfers.update(&id, |t| t.status = Status::Active);
        let spawned = std::thread::Builder::new().name("phone-relay-send".into()).spawn(move || {
            let r = me.rf_stream(&conn, &t, &tx);
            me.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).remove(&t.id);
            drop(slot);
            if let Err(why) = r {
                me.rf_abort_msg(&conn, &t.id, why);
                if let Some(t) = me.transfers.update(&t.id, |t| t.status = Status::Failed) {
                    me.emit("phone:files", json!({ "kind": "failed", "transfer": t.public() }));
                }
            }
        });
        if spawned.is_err() {
            self.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            self.transfers.update(&id, |t| t.status = Status::Failed);
        }
    }

    /// The sending loop of one PC -> phone file. Ok = the phone confirmed (`file.done`) or the file is at least fully sent and acknowledged.
    fn rf_stream(&self, conn: &Conn, t: &Transfer, tx: &Tx) -> Result<(), &'static str> {
        let mut f = t.path.as_ref().and_then(|p| File::open(p).ok()).ok_or("file_gone")?;
        let size = f.metadata().map_err(|_| "file_gone")?.len();
        if size != t.size {
            return Err("changed");
        }
        self.emit("phone:files", json!({ "kind": "start", "transfer": t.public() }));
        let (mut off, mut buf, mut last_emit, started) = (0u64, vec![0u8; CHUNK], Instant::now(), Instant::now());
        let mut progress_at = Instant::now();
        let mut seen_ack = 0u64;
        while off < size {
            // flow control: at most WINDOW chunks unacknowledged
            let mut a = tx.acked.lock().unwrap_or_else(|e| e.into_inner());
            while off.saturating_sub(*a) >= WINDOW * CHUNK as u64 {
                if tx.cancel.load(Ordering::SeqCst) || self.stopping() || progress_at.elapsed() > STALL {
                    return Err(if tx.cancel.load(Ordering::SeqCst) { "cancelled" } else { "timeout" });
                }
                a = tx.cv.wait_timeout(a, Duration::from_millis(500)).unwrap_or_else(|e| e.into_inner()).0;
                if *a > seen_ack {
                    seen_ack = *a;
                    progress_at = Instant::now();
                }
            }
            drop(a);
            if tx.cancel.load(Ordering::SeqCst) || self.stopping() {
                return Err("cancelled");
            }
            let want = (size - off).min(CHUNK as u64) as usize;
            f.read_exact(&mut buf[..want]).map_err(|_| "file_gone")?;
            let msg = format!("{{\"t\":\"file.chunk\",\"id\":\"{}\",\"off\":{off},\"d\":\"{}\"}}", t.id, base64::engine::general_purpose::STANDARD.encode(&buf[..want]));
            if !conn.send_text(&msg) {
                return Err("send_failed");
            }
            off += want as u64;
            progress_at = Instant::now();
            // pacing: never faster than the relay tolerates
            let due = Duration::from_secs_f64(off as f64 / PACE_BYTES_PER_SEC);
            if let Some(wait) = due.checked_sub(started.elapsed()) {
                std::thread::sleep(wait);
            }
            if last_emit.elapsed() > Duration::from_millis(300) {
                last_emit = Instant::now();
                if let Some(t) = self.transfers.update(&t.id, |t| t.bytes = off) {
                    self.emit("phone:files", json!({ "kind": "progress", "transfer": t.public() }));
                }
            }
        }
        if !conn.send_text(&json!({ "t": "file.end", "id": t.id, "size": size }).to_string()) {
            return Err("send_failed");
        }
        // the phone answers `file.done` (handled with the LAN downloads); wait for the verdict
        let t0 = Instant::now();
        loop {
            match self.transfers.get(&t.id).map(|t| t.status) {
                Some(Status::Done) => return Ok(()),
                Some(Status::Failed) | None => return Ok(()), // the verdict is already recorded elsewhere
                _ => {}
            }
            if tx.cancel.load(Ordering::SeqCst) || self.stopping() {
                return Err("cancelled");
            }
            if t0.elapsed() > STALL {
                return Err("timeout");
            }
            let a = tx.acked.lock().unwrap_or_else(|e| e.into_inner());
            let _ = tx.cv.wait_timeout(a, Duration::from_millis(200));
        }
    }

    /// Fails the relay transfers of a channel that went away.
    pub(super) fn rf_conn_gone(&self, conn: &Conn) {
        self.rf_gone(|dev, c| dev == conn.device && c == conn.id);
    }

    /// Fails the relay transfers of a device (revoked).
    pub(super) fn rf_device_gone(&self, device: &str) {
        self.rf_gone(|dev, _| dev == device);
    }

    fn rf_gone(&self, which: impl Fn(&str, u64) -> bool) {
        {
            let mut rx = self.rf.rx.lock().unwrap_or_else(|e| e.into_inner());
            let ids: Vec<String> = rx.iter().filter(|(_, s)| which(&s.dev, s.conn)).map(|(id, _)| id.clone()).collect();
            for id in ids {
                self.rf_fail_rx(&mut rx, &id, None);
            }
        }
        for tx in self.rf.tx.lock().unwrap_or_else(|e| e.into_inner()).values().filter(|t| which(&t.dev, t.conn)) {
            tx.cancel.store(true, Ordering::SeqCst);
            tx.cv.notify_all();
        }
    }

    /// Housekeeping tick of a relay connection: receiving transfers that stalled are dropped.
    pub(super) fn rf_sweep(&self, device: &str) {
        let mut rx = self.rf.rx.lock().unwrap_or_else(|e| e.into_inner());
        let stale: Vec<String> = rx.iter().filter(|(_, s)| s.dev == device && s.last_rx.elapsed() > STALL).map(|(id, _)| id.clone()).collect();
        for id in stale {
            self.rf_fail_rx(&mut rx, &id, None);
            self.send(Some(device), &json!({ "t": "file.abort", "id": id, "reason": "timeout" }));
        }
    }
}
