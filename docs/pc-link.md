# PC link (Pill to Pill) - protocol, milestone M1

Two Pills on the owner's own PCs pair once and then share clipboard text, through the same relay and the same sealed frames the phone link uses
([phone-relay.md](phone-relay.md)). Code: `src-tauri/src/phone/pc.rs` (pairing, messages, status), `relay.rs` (the role parameter), `reg.rs` (the
`kind` of a device), `clip.rs` (the stamp). Not in M1: files (M2, they will land in the normal Downloads folder and show in the Shelf), a direct
LAN connection (M3), remote commands (never).

## What is reused, what is new

* Transport, keys, frames, replay window, back-off, presence: unchanged. A paired PC is a `Device` of `kind: "pc"` in `devices.json`; the relay hub
  connects it like a phone. The relay server needs no change: it knows only the labels `pc` and `phone` and forwards opaque frames.
* Roles: the two ends of one room must use different labels. The PC that **showed** the pairing token is the relay's `pc` end (it seals with `c2p`,
  opens `p2c`), the one that **entered** it is the `phone` end (`Device::joiner`, seals `p2c`, opens `c2p`). Phones keep `pc` on the PC side.
* `devices.json` is backward compatible: `kind`, `joiner`, `clipTo`, `clipFrom`, `revokedAt` have defaults (phone, off, on, on, none) and are not
  written for a phone, so an old file loads unchanged and an old Pill reading a new file only sees records it ignores. `me` is this PC's id.
* A PC has no bearer token (`tokenHash` is empty, `by_token("")` never matches): it proves itself with its keys only.

## Pairing (relay, one pasted token)

The token `T` is 128 random bits as 26 Crockford base32 characters plus 2 check characters, shown as `PILL-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX`; it is
entered by paste (case, dashes and `O/0 I/L/1` do not matter). It lives 10 minutes, works once and exists only in memory. The check characters catch
a typo before the relay is asked for a room (it allows 20 new rooms per hour per address).

1. **A** (shows the token): `room = hex(HMAC(k_T.room_key, "room")[0..16])`, `join = b64url(HMAC(k_T.room_key, "join"))` with
   `k_T = HKDF-SHA256(ikm = T, salt = SHA256("pill-pc-pair-v1"), "keys", 96)` (the same `Keys` layout as the phone link). A joins that room as `pc`.
2. **B** (enters it) derives the same and joins as `phone`; on every `peer up` it sends `pair.req {id, name, os, proto:1, pk}` (its random X25519 public key).
3. A shows "B wants to pair: Allow / Deny" (island card and Settings). Nothing is stored before Allow. The first request holds the card; a retry
   of the same PC refreshes it. Deny sends `pair.deny`.
4. Allow: A makes its key, `shared = X25519`, `keys = HKDF-SHA256(ikm = shared, salt = SHA256("pill-pc-v1" || T || idA || idB), "keys", 96)`, and
   sends `pair.ok {id, name, os, proto, pk, confirm}` with `confirm = HMAC(k_T.room_key, "ok" || pkA || pkB)`.
5. B checks `confirm` (constant time: A knew `T` and saw both keys), derives the same keys, stores A, and answers `pair.done {confirm}` with
   `HMAC(.., "done" || pkB || pkA)`. A checks it and only then stores B. A wrong proof ends the pairing as failed; neither side stores anything.
6. Both leave the pairing room (it is a temporary device of kind `"pairing"` that is never written to disk) and connect the permanent room that
   comes from `keys.room_key` (`v2::room`, `v2::join`), A as `pc`, B as `phone`.

A shown token that is not used in 10 minutes dies (checked on the next frame and on the relay worker's 25 s tick); the entering side waits 2 minutes
for an answer. Anyone who sees the token inside its 10 minutes can reach the Allow card (not more): treat it like a password, and Allow only a PC
you started yourself. The relay sees the room, the timing and the sizes, never the token or the keys.

## Messages of a paired PC

Inside the sealed envelope `{sid, seq, m}` of the phone link; unknown fields and types are ignored. A PC may send only these; every phone message
(`notif.*`, `status`, `file.*`, ...) from a PC is dropped.

| `t` | fields | meaning |
|---|---|---|
| `hello` | `kind:"pc"`, `id`, `name`, `proto:1`, `time` | sent when the channel comes up (and again on every `peer up`); a changed `name` is stored |
| `ping` / `pong` | - | answer to `ping` |
| `clip` | `id`, `text`, `ts`, `seq` | clipboard text, at most 256 KiB UTF-8; `seq` is the Lamport stamp below |
| `unpair` | - | the sender forgot you: delete the record and the keys |

### Clipboard

On by default for a paired PC, both directions, with two switches per peer in Settings (`clipTo`: my clipboard goes to it, `clipFrom`: its text
is applied here). Nothing is sent, and no clipboard listener exists, while no PC is paired and connected. The rules of the phone link stay: copies of
password managers (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory = 0`, `CanUploadToCloudClipboard = 0`, the deny list) are never read, text
only, 256 KiB, only hashes of the last texts are kept, the text is never echoed back, logged or put in an event (`pc:clip` carries only the name).
A text that arrives is written with the two exclusion formats set to 0, so Windows keeps it out of its history (Win+V) and the cloud clipboard.
`phone.clipboard` (phones) is still off by default and unrelated.

**Which copy wins.** Each Pill keeps a clock and the `(stamp, device id)` of the text it holds. A local copy is stamped `max(clock + 1, now_ms)`
(the wall clock is only a floor, so a restarted PC does not stamp below older copies) and sent to every connected peer with `clipTo` on. An incoming
`clip` raises the clock to its `seq` and is applied only when `(seq, sender id)` is above the held one (the larger id wins a tie). Two simultaneous
copies therefore end with the same text on both PCs, a late older copy never overwrites a newer one, and a duplicate is dropped.

**No loops.** A Pill sends only the user's own copies, never what it applied, and the applied text is also remembered as a hash, so in a full mesh
of any size one copy produces exactly one frame per peer and nothing is forwarded.

### Unpair

On either side. A connected peer is told at once (`unpair`) and both records go. An absent peer: the record stays as a tombstone (`revokedAt`; it is
hidden, ignored, kept for 30 days) and the Pill keeps its relay connection; when the peer next connects, `unpair` is sent and the record goes.
If the peer never returns it stays "paired" on its side but is dead: nothing it sends is accepted.

## Settings and events

`pc.enabled` (default off; pairing switches it on) starts the server. When only the PC link is on the listener binds to this PC only (loopback) and no
firewall rule is asked for; the phone link binds the LAN address as before. Commands: `pc_status`, `pc_pair {op: start|join|allow|deny|cancel, token}`,
`pc_peer {id, op: clipTo|clipFrom|unpair, on}`. Events: `pc:state` (the whole status, see `phone/pc.rs::pc_status`) and `pc:clip {device, name}`.
