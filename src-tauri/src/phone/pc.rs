// The PC link (docs/pc-link.md): two Pills pair with a pasted token, then exchange clipboard text. A paired PC is a `Device` of kind "pc" on the relay hub;
// a pairing under way is a temporary device of kind "pairing" keyed by the token. PRIVACY: the token and clipboard text are never logged, evented or stored.
use super::reg::{self, Device};
use super::server::{now_ms, Conn, Route, Shared};
use super::{clip, files, v2};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use serde_json::{json, Value};

/// How long a shown token works, and how long the PC that entered one waits for an answer.
pub const SHOW_TTL_MS: i64 = 10 * 60 * 1000;
const JOIN_TTL_MS: i64 = 2 * 60 * 1000;
const PAIRING: &str = "pairing";
/// Crockford base32: no I, L, O, U.
const ALPHA: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Two check characters: a typo is caught before the relay is asked for a room (it allows 20 an hour).
fn check(s: &str) -> String {
    let c = s.bytes().fold(7u32, |a, b| (a * 33 + ALPHA.iter().position(|x| *x == b).unwrap_or(0) as u32) % 1024);
    format!("{}{}", ALPHA[(c >> 5) as usize] as char, ALPHA[(c & 31) as usize] as char)
}

/// 26 characters for 128 random bits and 2 check characters.
pub fn new_token() -> String {
    let n = u128::from_be_bytes(reg::random_bytes(16).try_into().expect("16 bytes"));
    let body: String = (0..26).map(|i| ALPHA[((n >> (125 - 5 * i)) & 31) as usize] as char).collect();
    format!("{body}{}", check(&body))
}

/// The token as typed (`PILL-7K2M-...`, any case, dashes optional, O/0 and I/L/1 forgiven) in its normal form.
pub fn parse_token(s: &str) -> Option<String> {
    let up: String = s.to_ascii_uppercase().chars().filter(char::is_ascii_alphanumeric).collect();
    let t: String = up.strip_prefix("PILL").unwrap_or(&up).chars().map(|c| match c { 'O' => '0', 'I' | 'L' => '1', c => c }).collect();
    (t.len() == 28 && t.bytes().all(|b| ALPHA.contains(&b)) && check(&t[..26]) == t[26..]).then_some(t)
}

pub fn show(t: &str) -> String {
    format!("PILL-{}", t.as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c)).collect::<Vec<_>>().join("-"))
}

/// One pairing, from "token shown / entered" on. It stays (without secrets) after its end so the UI can say how it went.
#[derive(Clone)]
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
    /// the shown side after Allow: stored once the other side proves it has them too
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

    /// A copy of the pairing when it satisfies `ok` (no lock is held while the caller works with it).
    fn pairing_if(&self, ok: impl Fn(&Pairing) -> bool) -> Option<Pairing> {
        self.pg().clone().filter(ok)
    }

    pub(super) fn pc_emit(&self) {
        self.emit("pc:state", self.pc_status());
    }

    /// Shows (`shown`) or enters a token; any earlier pairing is replaced.
    pub(super) fn pair_open(&self, shown: bool, token: String, ttl: i64) {
        let secret: [u8; 32] = reg::random_bytes(32).try_into().expect("32 bytes");
        let room = Device { id: PAIRING.into(), kind: PAIRING.into(), joiner: !shown, keys: Some(v2::pair_room_keys(&token)), ..Default::default() };
        self.registry().set_pairing(Some(room));
        *self.pg() = Some(Pairing { token, shown, exp: now_ms() + ttl, secret, pk: v2::public(secret), state: "waiting", peer: None, keys: None });
        self.relay_sync();
        self.pc_emit();
    }

    /// `Some(state)` keeps the outcome (without secrets) for the UI, `None` forgets the pairing.
    fn pair_end(&self, state: Option<&'static str>) {
        let mut g = self.pg();
        match (state, g.as_mut()) {
            (Some(s), Some(p)) => (p.state, p.secret, p.keys, p.token) = (s, [0; 32], None, String::new()),
            _ => *g = None,
        }
        drop(g);
        self.registry().set_pairing(None);
        self.relay_sync(); // the pairing room's connection ends, the new peer's begins
        self.pc_emit();
    }

    /// A token or a wait that ran out (called from the relay worker's tick and the UI's poll).
    pub fn pair_expire(&self) {
        if self.pairing_if(|p| p.active() && now_ms() > p.exp).is_some() {
            self.pair_end(Some("failed"));
        }
    }

    fn pair_decide(&self, allow: bool) -> Result<(), String> {
        let p = self.pairing_if(|p| p.shown && p.state == "request").ok_or("no request to answer")?;
        let peer = p.peer.ok_or("no request to answer")?;
        if !allow {
            self.send(Some(PAIRING), &json!({ "t": "pair.deny" }));
            return Ok(self.pair_end(Some("denied")));
        }
        let (_, shared) = v2::agree(p.secret, &peer.3).ok_or("the other PC sent a bad key")?;
        let me = self.registry().me();
        let keys = v2::derive_pc(&shared, &p.token, &me, &peer.0);
        if let Some(g) = self.pg().as_mut() {
            (g.state, g.keys) = ("allowed", Some(keys));
        }
        let confirm = v2::proof(&p.token, "ok", &p.pk, &peer.3);
        self.send(Some(PAIRING), &json!({ "t": "pair.ok", "id": me, "name": self.cfg().pc_name, "os": "Windows", "proto": 1, "pk": B64.encode(p.pk), "confirm": confirm }));
        self.pc_emit();
        Ok(())
    }

    fn pair_msg(&self, t: &str, m: &Value) {
        self.pair_expire();
        let (id, pk) = (text(&m["id"], 64), m["pk"].as_str().and_then(super::relay::b64_any).unwrap_or_default());
        let bad = !files::valid_id(&id) || id == self.registry().me() || pk.len() != 32;
        let (confirm, who) = (m["confirm"].as_str().unwrap_or(""), (id.clone(), text(&m["name"], 60), text(&m["os"], 40), pk.clone()));
        match t {
            // the other PC entered our token: ask the user (the first request holds the card, a retry of the same PC refreshes it)
            "pair.req" if !bad => {
                let mut g = self.pg();
                let Some(p) = g.as_mut().filter(|p| p.shown && matches!(p.state, "waiting" | "request") && p.peer.as_ref().map_or(true, |x| x.0 == id)) else { return };
                (p.state, p.peer) = ("request", Some(who));
                drop(g);
                self.pc_emit();
            }
            // the other PC allowed it: check its proof, derive the keys, store it, prove ours
            "pair.ok" if !bad => {
                let Some(p) = self.pairing_if(|p| !p.shown && p.state == "waiting") else { return };
                let Some((_, shared)) = v2::agree(p.secret, &pk).filter(|_| v2::proof_ok(&p.token, "ok", &pk, &p.pk, confirm)) else { return self.pair_end(Some("failed")) };
                self.send(Some(PAIRING), &json!({ "t": "pair.done", "confirm": v2::proof(&p.token, "done", &p.pk, &pk) }));
                self.registry().pair_pc(&id, &who.1, &who.2, v2::derive_pc(&shared, &p.token, &id, &self.registry().me()), true, now_ms());
                if let Some(g) = self.pg().as_mut() {
                    g.peer = Some(who); // for "Paired with <name>"
                }
                self.pair_end(Some("done"));
            }
            "pair.deny" if self.pairing_if(|p| !p.shown && p.active()).is_some() => self.pair_end(Some("denied")),
            "pair.done" => {
                let Some(p) = self.pairing_if(|p| p.shown && p.state == "allowed") else { return };
                let (Some(keys), Some(peer)) = (p.keys, p.peer) else { return };
                if v2::proof_ok(&p.token, "done", &peer.3, &p.pk, confirm) {
                    self.registry().pair_pc(&peer.0, &peer.1, &peer.2, keys, false, now_ms());
                    self.pair_end(Some("done"));
                }
            }
            _ => {}
        }
    }

    /// What this PC says first on a relay connection: the phone `hello`, a PC's `hello`, or the entering side's `pair.req`.
    pub(super) fn hello_for(&self, id: &str) -> Option<Value> {
        let (kind, name, me) = (self.registry().get(id)?.kind, self.cfg().pc_name, || self.registry().me());
        match kind.as_str() {
            "" => Some(self.hello_msg()),
            "pc" => Some(json!({ "t": "hello", "kind": "pc", "id": me(), "name": name, "proto": 1, "time": now_ms() })),
            _ => self.pairing_if(|p| !p.shown && p.state == "waiting").map(|p| json!({ "t": "pair.req", "id": me(), "name": name, "os": "Windows", "proto": 1, "pk": B64.encode(p.pk) })),
        }
    }

    /// One message from a paired PC or the pairing room. A PC may send only these; everything a phone may send is ignored.
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
                    self.pc_emit();
                }
            }
            "clip" if dev.clip_from => self.clip_in(dev, m),
            _ => {}
        }
    }

    /// The paired, not unpaired PCs.
    fn pcs(&self) -> Vec<Device> {
        self.registry().list().into_iter().filter(|d| d.kind == "pc" && d.revoked_at.is_none()).collect()
    }

    /// The PCs this clipboard goes to right now: connected, `clipTo` on.
    pub(super) fn clip_pcs(&self) -> Vec<Device> {
        self.pcs().into_iter().filter(|d| d.clip_to && self.route_of(&d.id).is_some()).collect()
    }

    /// Sends a local copy (never a forward of a peer's text), stamped like `clip::State::stamp`.
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

    /// `{enabled, name, peers[], pair}` for the PCs section and the island (event `pc:state`); `pair` is null or
    /// `{mode: show|enter, state, token (only while shown), expires, peer: {name, os}}`.
    pub fn pc_status(&self) -> Value {
        let c = self.cfg();
        let peers: Vec<Value> = self.pcs().iter().map(|d| {
            let route = match self.route_of(&d.id) { Some(Route::Lan) => "lan", Some(Route::Relay) => "relay", None => "none" };
            json!({ "id": d.id, "name": d.name, "os": d.os, "online": route != "none", "route": route, "lastSeen": d.last_seen, "clipTo": d.clip_to, "clipFrom": d.clip_from })
        }).collect();
        let pair = self.pg().as_ref().map(|p| {
            let who = p.peer.as_ref().filter(|_| matches!(p.state, "request" | "allowed" | "done")).map(|x| json!({ "name": x.1, "os": x.2 }));
            json!({ "mode": if p.shown { "show" } else { "enter" }, "state": p.state, "token": (p.shown && p.active()).then(|| show(&p.token)), "expires": p.exp, "peer": who })
        });
        json!({ "enabled": c.pc_on, "name": c.pc_name, "peers": peers, "pair": pair })
    }

    /// `start` (show a token) | `join` (enter `token`) | `allow` | `deny` | `cancel` (forget the pairing and its outcome).
    pub fn pc_pair(&self, op: &str, token: Option<&str>) -> Result<Value, String> {
        match op {
            "start" => self.pair_open(true, new_token(), SHOW_TTL_MS),
            "join" => self.pair_open(false, parse_token(token.unwrap_or("")).ok_or("that is not a Pill token")?, JOIN_TTL_MS),
            "allow" | "deny" => self.pair_decide(op == "allow")?,
            "cancel" => self.pair_end(None),
            _ => return Err("unknown operation".into()),
        }
        Ok(self.pc_status())
    }

    /// `clipTo` | `clipFrom` (set to `on`) | `unpair`: a connected peer is told at once and forgotten, an absent one when it next connects.
    pub fn pc_peer(&self, id: &str, op: &str, on: bool) -> Result<Value, String> {
        if !self.pcs().iter().any(|d| d.id == id) {
            return Err("no such PC".into());
        }
        match op {
            "clipTo" => self.registry().update(id, |d| d.clip_to = on),
            "clipFrom" => self.registry().update(id, |d| d.clip_from = on),
            "unpair" if self.send(Some(id), &json!({ "t": "unpair" })) > 0 => self.revoke(id),
            "unpair" => self.registry().update(id, |d| d.revoked_at = Some(now_ms())),
            _ => return Err("unknown operation".into()),
        };
        self.clip_sync();
        self.pc_emit();
        Ok(self.pc_status())
    }
}
