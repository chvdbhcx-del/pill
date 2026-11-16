// The PC link (docs/pc-link.md): pairing two Pills with a pasted token, and the few messages two paired Pills exchange (clipboard text, hello, unpair).
// A paired PC is a `Device` of kind "pc" that the relay hub connects like a phone; the pairing itself runs in a temporary device of kind "pairing" whose
// keys come from the token, so it gets the same sealed frames, replay window and reconnects for free.
// PRIVACY: the token and clipboard text are never logged, never put in an event and never stored (the token lives in memory for 10 minutes).
use super::reg::{self, Device};
use super::server::{now_ms, Conn, Route, Shared};
use super::{clip, files, v2};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde_json::{json, Value};

/// How long a shown token works, and how long the PC that entered it waits for the other one to answer.
pub const SHOW_TTL_MS: i64 = 10 * 60 * 1000;
const JOIN_TTL_MS: i64 = 2 * 60 * 1000;
/// The id of the temporary device of a pairing under way.
const PAIRING: &str = "pairing";
/// Crockford base32: no I, L, O, U.
const ALPHA: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

// ------------------------------------------------------------------------------------------------ the token

/// Two check characters: a typo is caught before the relay is asked for a room (it allows 20 new rooms an hour).
fn check(s: &str) -> String {
    let c = s.bytes().fold(7u32, |a, b| (a * 33 + ALPHA.iter().position(|x| *x == b).unwrap_or(0) as u32) % 1024);
    format!("{}{}", ALPHA[(c >> 5) as usize] as char, ALPHA[(c & 31) as usize] as char)
}

/// 28 characters: 26 for 128 random bits and 2 check characters.
pub fn new_token() -> String {
    let n = u128::from_be_bytes(reg::random_bytes(16).try_into().expect("16 bytes"));
    let body: String = (0..26).map(|i| ALPHA[((n >> (125 - 5 * i)) & 31) as usize] as char).collect();
    format!("{body}{}", check(&body))
}

/// The token as typed or pasted (`PILL-7K2M-...`, any case, with or without dashes, O/0 and I/L/1 forgiven) in its normal form; `None` when it is not one.
pub fn parse_token(s: &str) -> Option<String> {
    let up: String = s.to_ascii_uppercase().chars().filter(char::is_ascii_alphanumeric).collect();
    let t: String = up.strip_prefix("PILL").unwrap_or(&up).chars().map(|c| match c { 'O' => '0', 'I' | 'L' => '1', c => c }).collect();
    (t.len() == 28 && t.bytes().all(|b| ALPHA.contains(&b)) && check(&t[..26]) == t[26..]).then_some(t)
}

/// `PILL-7K2M-9XQD-...`
pub fn show(t: &str) -> String {
    format!("PILL-{}", t.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("-"))
}

// ------------------------------------------------------------------------------------------------ pairing state

/// One pairing, from "token shown" / "token entered" to its end. It stays (without the secrets) after the end so the UI can say how it went.
pub(super) struct Pairing {
    token: String,
    /// this PC showed the token (it decides), else it entered it
    shown: bool,
    exp: i64,
    secret: [u8; 32],
    pk: [u8; 32],
    /// waiting | request | allowed | done | denied | failed
    state: &'static str,
    /// the other PC: (id, name, os, public key)
    peer: Option<(String, String, String, Vec<u8>)>,
    /// the shown side, after Allow: the keys it stores once the other side proves it has them too
    keys: Option<v2::Keys>,
}

impl Pairing {
    fn active(&self) -> bool {
        matches!(self.state, "waiting" | "request" | "allowed")
    }
}

fn text(v: &Value, max: usize) -> String {
    v.as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Shared {
    fn pg(&self) -> std::sync::MutexGuard<'_, Option<Pairing>> {
        self.pairing.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pair_open(&self, shown: bool, token: String, ttl: i64) {
        let secret: [u8; 32] = reg::random_bytes(32).try_into().expect("32 bytes");
        let room = Device { id: PAIRING.into(), kind: PAIRING.into(), joiner: !shown, keys: Some(v2::pair_room_keys(&token)), ..Default::default() };
        self.registry().set_pairing(Some(room));
        *self.pg() = Some(Pairing { token, shown, exp: now_ms() + ttl, secret, pk: v2::public(secret), state: "waiting", peer: None, keys: None });
        self.relay_sync();
        self.emit("pc:state", self.pc_status());
    }

    /// "Pair a PC": shows a new token (the old one dies).
    pub(super) fn pair_start(&self, ttl: i64) {
        self.pair_open(true, new_token(), ttl);
    }

    /// "Enter a token".
    pub(super) fn pair_join(&self, token: &str, ttl: i64) -> Result<(), String> {
        self.pair_open(false, parse_token(token).ok_or("that is not a Pill token")?, ttl);
        Ok(())
    }

    /// The end of a pairing. `Some(state)` keeps the outcome for the UI, `None` forgets it.
    fn pair_end(&self, state: Option<&'static str>) {
        {
            let mut g = self.pg();
            match (state, g.as_mut()) {
                (Some(s), Some(p)) => {
                    (p.state, p.secret, p.keys) = (s, [0; 32], None);
                    p.token.clear();
                }
                _ => *g = None,
            }
        }
        self.registry().set_pairing(None);
        self.relay_sync(); // the pairing room's connection ends, the new peer's begins
        self.emit("pc:state", self.pc_status());
    }

    /// A token or a wait that ran out. Called from the relay worker's idle tick and by the UI's poll.
    pub fn pair_expire(&self) {
        if self.pg().as_ref().is_some_and(|p| p.active() && now_ms() > p.exp) {
            self.pair_end(Some("failed"));
        }
    }

    /// The shown side's user answered the request.
    fn pair_decide(&self, allow: bool) -> Result<(), String> {
        let (token, secret, pk, peer) = {
            let g = self.pg();
            let p = g.as_ref().filter(|p| p.shown && p.state == "request").ok_or("no request to answer")?;
            (p.token.clone(), p.secret, p.pk, p.peer.clone().ok_or("no request to answer")?)
        };
        if !allow {
            self.send(Some(PAIRING), &json!({ "t": "pair.deny" }));
            self.pair_end(Some("denied"));
            return Ok(());
        }
        let (_, shared) = v2::agree(secret, &peer.3).ok_or("the other PC sent a bad key")?;
        let me = self.registry().me();
        let keys = v2::derive_pc(&shared, &token, &me, &peer.0);
        let ok = json!({ "t": "pair.ok", "id": me, "name": self.cfg().pc_name, "os": "Windows", "proto": 1, "pk": B64.encode(pk), "confirm": v2::proof(&token, "ok", &pk, &peer.3) });
        if let Some(p) = self.pg().as_mut() {
            (p.state, p.keys) = ("allowed", Some(keys));
        }
        self.send(Some(PAIRING), &ok);
        self.emit("pc:state", self.pc_status());
        Ok(())
    }

    fn pair_msg(&self, t: &str, m: &Value) {
        self.pair_expire();
        let me = self.registry().me();
        let (id, pk) = (text(&m["id"], 64), m["pk"].as_str().and_then(super::relay::b64_any).unwrap_or_default());
        let bad = !files::valid_id(&id) || id == me || pk.len() != 32;
        match t {
            // the other PC entered our token: ask the user (the first request holds the card, a retry of the same PC refreshes it)
            "pair.req" if !bad => {
                let mut g = self.pg();
                let Some(p) = g.as_mut().filter(|p| p.shown && matches!(p.state, "waiting" | "request") && p.peer.as_ref().map_or(true, |x| x.0 == id)) else { return };
                (p.state, p.peer) = ("request", Some((id, text(&m["name"], 60), text(&m["os"], 40), pk)));
                drop(g);
                self.emit("pc:state", self.pc_status());
            }
            // the other PC allowed it: check its proof, derive the keys, store the peer, prove ours
            "pair.ok" if !bad => {
                let (token, secret, mine) = {
                    let g = self.pg();
                    let Some(p) = g.as_ref().filter(|p| !p.shown && p.state == "waiting") else { return };
                    (p.token.clone(), p.secret, p.pk)
                };
                let Some((_, shared)) = v2::agree(secret, &pk).filter(|_| v2::proof_ok(&token, "ok", &pk, &mine, m["confirm"].as_str().unwrap_or(""))) else { return self.pair_end(Some("failed")) };
                let keys = v2::derive_pc(&shared, &token, &id, &me);
                self.send(Some(PAIRING), &json!({ "t": "pair.done", "confirm": v2::proof(&token, "done", &mine, &pk) }));
                let (name, os) = (text(&m["name"], 60), text(&m["os"], 40));
                self.registry().pair_pc(&id, &name, &os, keys, true, now_ms());
                if let Some(p) = self.pg().as_mut() {
                    p.peer = Some((id, name, os, pk)); // for "Paired with <name>"
                }
                self.pair_end(Some("done"));
            }
            "pair.deny" if self.pg().as_ref().is_some_and(|p| !p.shown && p.active()) => self.pair_end(Some("denied")),
            "pair.done" => {
                let (token, mine, keys, peer) = {
                    let g = self.pg();
                    let Some(p) = g.as_ref().filter(|p| p.shown && p.state == "allowed") else { return };
                    (p.token.clone(), p.pk, p.keys.clone(), p.peer.clone())
                };
                let (Some(keys), Some(peer)) = (keys, peer) else { return };
                if v2::proof_ok(&token, "done", &peer.3, &mine, m["confirm"].as_str().unwrap_or("")) {
                    self.registry().pair_pc(&peer.0, &peer.1, &peer.2, keys, false, now_ms());
                    self.pair_end(Some("done"));
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------------------------------------------ messages of paired PCs

    /// What this PC says first on a relay connection: the phone `hello`, a PC's `hello`, or the entering side's `pair.req`.
    pub(super) fn hello_for(&self, id: &str) -> Option<Value> {
        let d = self.registry().get(id)?;
        let name = self.cfg().pc_name;
        match d.kind.as_str() {
            "" => Some(self.hello_msg()),
            "pc" => Some(json!({ "t": "hello", "kind": "pc", "id": self.registry().me(), "name": name, "proto": 1, "time": now_ms() })),
            _ => {
                let me = self.registry().me();
                let g = self.pg();
                g.as_ref().filter(|p| !p.shown && p.state == "waiting").map(|p| json!({ "t": "pair.req", "id": me, "name": name, "os": "Windows", "proto": 1, "pk": B64.encode(p.pk) }))
            }
        }
    }

    /// One message from a paired PC (or from the pairing room), whichever route it came by. Everything else a phone may send is ignored.
    pub(super) fn pc_msg(&self, dev: &Device, conn: &Conn, m: &Value) {
        let t = m["t"].as_str().unwrap_or("");
        if dev.kind == PAIRING {
            return self.pair_msg(t, m);
        }
        match t {
            "unpair" => {
                self.revoke(&dev.id); // it unpaired us: the record and the keys go
            }
            _ if dev.revoked_at.is_some() => {} // we unpaired it: nothing it says counts any more
            "ping" => {
                self.send_conn(conn, &json!({ "t": "pong" }));
            }
            "hello" => {
                let name = text(&m["name"], 60);
                if !name.is_empty() && name != dev.name && self.registry().update(&dev.id, |d| d.name = name) {
                    self.emit("pc:state", self.pc_status());
                }
            }
            "clip" if dev.clip_from => self.clip_in_pc(dev, m),
            _ => {}
        }
    }

    /// The paired PCs this PC's clipboard goes to right now: connected, and their `clipTo` switch is on.
    pub(super) fn clip_pcs(&self) -> Vec<Device> {
        self.registry().list().into_iter().filter(|d| d.kind == "pc" && d.clip_to && d.revoked_at.is_none() && self.route_of(&d.id).is_some()).collect()
    }

    /// Sends a local copy to those PCs, stamped (see `clip::State::stamp`). Never a forward of something a peer sent: only the user's own copies get here.
    pub(super) fn clip_to_pcs(&self, pcs: &[Device], t: &str) -> usize {
        if pcs.is_empty() {
            return 0;
        }
        let ts = now_ms();
        let mut m = clip::message(&reg::new_id(), t, ts);
        m["seq"] = json!(self.clipst.stamp(&self.registry().me(), ts));
        let m = m.to_string();
        pcs.iter().filter(|d| self.channels(&d.id).iter().any(|c| c.send_text(&m))).count()
    }

    /// A `clip` from a paired PC whose `clipFrom` switch is on: onto the clipboard (kept out of Windows' history and cloud), and a chip on the island.
    fn clip_in_pc(&self, dev: &Device, m: &Value) {
        let Some(t) = clip::parse(m) else { return };
        if !self.clipst.take_stamped(&t, m["seq"].as_u64().unwrap_or(0), &dev.id) {
            return;
        }
        if !self.opts.clip.set_text(&t, true) {
            self.clipst.undo_incoming(&t);
            return;
        }
        self.emit("pc:clip", json!({ "device": dev.id, "name": dev.name })); // never the text
    }

    // ------------------------------------------------------------------------------------------------ what the commands use

    /// `{enabled, name, peers[], pair}` for the PCs page and the island (event `pc:state`). `pair` is null or
    /// `{mode: show|enter, state: waiting|request|allowed|done|denied|failed, token (only while shown), expires, peer: {name, os} (request, allowed, done)}`.
    pub fn pc_status(&self) -> Value {
        let c = self.cfg();
        let peers: Vec<Value> = self
            .registry()
            .list()
            .iter()
            .filter(|d| d.kind == "pc" && d.revoked_at.is_none())
            .map(|d| {
                let route = match self.route_of(&d.id) {
                    Some(Route::Lan) => "lan",
                    Some(Route::Relay) => "relay",
                    None => "none",
                };
                json!({ "id": d.id, "name": d.name, "os": d.os, "online": route != "none", "route": route, "lastSeen": d.last_seen, "clipTo": d.clip_to, "clipFrom": d.clip_from })
            })
            .collect();
        let pair = self.pg().as_ref().map(|p| {
            let who = p.peer.as_ref().filter(|_| matches!(p.state, "request" | "allowed" | "done")).map(|x| json!({ "name": x.1, "os": x.2 }));
            json!({ "mode": if p.shown { "show" } else { "enter" }, "state": p.state, "token": (p.shown && p.active()).then(|| show(&p.token)), "expires": p.exp, "peer": who })
        });
        json!({ "enabled": c.pc_on, "name": c.pc_name, "peers": peers, "pair": pair })
    }

    /// `start` (show a token) | `join` (enter `token`) | `allow` | `deny` | `cancel` (forget the pairing and its outcome).
    pub fn pc_pair(&self, op: &str, token: Option<&str>) -> Result<Value, String> {
        match op {
            "start" => self.pair_start(SHOW_TTL_MS),
            "join" => self.pair_join(token.unwrap_or(""), JOIN_TTL_MS)?,
            "allow" | "deny" => self.pair_decide(op == "allow")?,
            "cancel" => self.pair_end(None),
            _ => return Err("unknown operation".into()),
        }
        Ok(self.pc_status())
    }

    /// `clipTo` | `clipFrom` (set to `on`) | `unpair`: a connected peer is told at once and forgotten, an absent one when it next connects.
    pub fn pc_peer(&self, id: &str, op: &str, on: bool) -> Result<Value, String> {
        self.registry().get(id).filter(|d| d.kind == "pc" && d.revoked_at.is_none()).ok_or("no such PC")?;
        match op {
            "clipTo" => self.registry().update(id, |d| d.clip_to = on),
            "clipFrom" => self.registry().update(id, |d| d.clip_from = on),
            "unpair" if self.send(Some(id), &json!({ "t": "unpair" })) > 0 => self.revoke(id),
            "unpair" => self.registry().update(id, |d| d.revoked_at = Some(now_ms())),
            _ => return Err("unknown operation".into()),
        };
        self.clip_sync();
        self.emit("pc:state", self.pc_status());
        Ok(self.pc_status())
    }
}
