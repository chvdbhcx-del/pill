# Pill Phone over the internet: relay + end-to-end encryption (protocol v2)

Extends `docs/phone-protocol.md`. The v1 messages (`notif.*`, `status`, `file.*`, `reply.result`, `ping`...) are unchanged; v2 only adds a second **transport** (a relay) and **encryption** of everything that crosses it. At home the phone still talks to the PC directly over the LAN as before (fast, no internet). Away from home both sides reach each other through the relay.

## Components

1. **Relay** (`relay/`): a Cloudflare Worker + one Durable Object per room. It forwards opaque frames between exactly two sockets (role `pc` and role `phone`) of the same room and keeps nothing but a verifier of the room's join secret. It never sees plaintext.
2. **PC (Pill)**: keeps one outbound WebSocket to the relay for as long as "Phone link" is ON and at least one phone is paired and `phone.relay` is ON. The LAN server stays as it is.
3. **Phone app**: connects to the PC directly when it can (last known host, 2 s timeout), otherwise to the relay.

## Keys (made at pairing, over the existing LAN `POST /api/pair`)

Pairing still happens at home (the QR page and `/api/pair` are LAN-only). Extra fields:

* request: `"pk": "<phone X25519 public key, base64url, 32 bytes>"`
* response: `"pk": "<PC X25519 public key, base64url>"`, `"relay": {"url":"wss://...","room":"<32 hex>","join":"<base64url 32 bytes>"}` (only if the relay is configured and enabled)

Both sides compute `shared = X25519(own_secret, peer_public)` and then
`okm = HKDF-SHA256(ikm = shared, salt = SHA256("pill-phone-v2" || pairing_code || deviceId), info = "keys", L = 96)`:

| bytes | name | use |
|---|---|---|
| 0..32 | `k_p2c` | phone -> PC frames |
| 32..64 | `k_c2p` | PC -> phone frames |
| 64..96 | `k_room` | `room = hex(HMAC-SHA256(k_room, "room")[0..16])`, `join = base64url(HMAC-SHA256(k_room, "join"))` |

(The relay block in the response is therefore redundant but lets old code stay simple; both sides MUST derive `room`/`join` themselves and compare.) Secret keys never leave their device; the pairing code and `deviceId` are the HKDF salt, so a captured public key exchange alone does not give the keys to a later observer of the relay. Keys are stored: PC in `%LOCALAPPDATA%\Pill\phone\devices.json` (protected with DPAPI, `CryptProtectData`), phone in the Android Keystore-wrapped store. Revoking a device deletes its keys and its room.

## Frames over the relay

Application payload = the v1 JSON message (UTF-8). Each frame is ONE binary WebSocket message:

```
frame   = nonce(24) || XChaCha20-Poly1305(key = k_<dir>, nonce, plaintext = envelope, aad = "pill-v2" || room || dir)
dir     = "p2c" | "c2p"
envelope= {"sid":"<random 8 bytes hex, new per connection>","seq":<u64, starts at 1, +1 per frame>,"m":<the v1 message object>}
```

* The receiver decrypts, rejects a frame whose `(sid, seq)` is not greater than the last accepted one for that `sid` (replay/reorder), and drops frames that fail authentication silently (count them; 50 in a minute closes the connection).
* Random 24-byte nonces: no counter state to persist, no nonce reuse.
* Max plaintext 900 KiB (frame limit of the relay is 1 MiB).
* Libraries: Rust `chacha20poly1305`, `x25519-dalek`, `hkdf`, `hmac`, `sha2`, `windows` DPAPI; Android Tink (`com.google.crypto.tink`: `XChaCha20Poly1305`) + an X25519 implementation (Tink `X25519`/BouncyCastle `X25519`). Do not hand-roll primitives.

## Relay protocol (WebSocket, `wss://<worker>/r/<room>?role=pc|phone`)

1. First client message (text JSON, within 5 s): `{"t":"join","join":"<join secret base64url>","v":2}`. For a room that does not exist yet the relay stores `verifier = SHA-256(join)` (first claim wins) and creates the room; for an existing room it compares `SHA-256(join)` to the verifier in constant time. Wrong -> close 4401.
2. The relay answers `{"t":"joined","peer":true|false}` and later `{"t":"peer","up":true|false}` whenever the other role connects or leaves. A second socket with the same role replaces the first (the old one is closed with 4409).
3. After `joined` every BINARY frame from one role is forwarded unchanged to the other role if present; if absent the frame is dropped (the relay buffers nothing). Text frames other than `join` are ignored.
4. Limits: frame <= 1 MiB (else close 1009), <= 60 frames/s and <= 4 MiB/s per room (excess dropped, 4429 after sustained abuse), idle sockets are kept by WebSocket hibernation; a ping from clients every 25 s keeps NATs open. A room with no activity for 30 days is deleted (alarm). Per source IP: at most 20 room creations per hour.
5. The relay logs nothing about content; `console.log` only counters.

## Application behaviour over the relay

* **Presence**: the PC treats the phone as online when the relay says `peer.up` (and a v1 `hello`/`status` follows); the phone does the same for the PC.
* **Route choice on the phone**: try LAN `ws://<last host>:<port>/ws` (2 s); if it fails try the relay; while on the relay retry the LAN every 60 s and when the Wi-Fi network changes, and switch back when it works. The phone must reconnect with back-off 1..30 s as in v1.
* **Replays**: on each (re)connection the phone replays active notifications exactly as in v1.
* **Files over the relay** (no HTTP side channel there): `file.offer`/`file.accept` as in v1, then data as v1-style messages `{"t":"file.chunk","id":"...","off":<byte offset>,"d":"<base64, <= 96 KiB raw>"}`, flow control `{"t":"file.ack","id","upto":<bytes>}` (sender keeps at most 8 chunks unacked), end `{"t":"file.end","id","size"}`, abort `{"t":"file.abort","id","reason"}`; the receiver verifies `size`. A setting caps the size over the relay (`phone.relayMaxFileMB`, default 100). Downloads PC -> phone over the relay use the same messages in the other direction. Over the LAN nothing changes.
* **Clipboard** (`caps` / `clip`, docs/phone-protocol.md, "Clipboard (v1.2)"): ordinary v1 JSON messages inside the same encrypted envelope; nothing else to add. `caps` is per connection, so the phone sends it again after each relay (re)connect.
* **Cost/privacy note shown in Settings**: "Through the relay your data is end-to-end encrypted; the relay (a Cloudflare Worker you own) sees only that two devices exchange data, when, and how much."

## Settings

PC: `phone.relay` (bool, default ON when a relay URL is set), `phone.relayUrl` (default = the deployed Worker, empty until deployed), `phone.relayMaxFileMB` (100). Phone app: "Use internet relay when away from home" switch (default ON) and the file cap shown read-only.

## Deploying the relay

`relay/` contains the Worker (`wrangler.toml`, `src/index.ts`, tests). Deploy with `npx wrangler login` (once, in a browser) then `npx wrangler deploy` from `relay/`; the URL is `https://pill-relay.<account-subdomain>.workers.dev`. Uses only the free plan features: a Durable Object class with SQLite storage (`new_sqlite_classes`) and the WebSocket Hibernation API. No secrets are needed.

## Changes by the phone side (v2)

The Android app (`android/`, `net/Link.kt`, `protocol/RelayCrypto.kt`, `transfer/Transfers.kt`) implements the text above as written. Where it was silent, the phone does the following; the PC side and the relay may rely on it (or must adapt to it):

* **Encodings.** `room` is the 32-char lowercase hex string; the AAD is the UTF-8 bytes of `"pill-v2" + room + dir` (the hex text, not 16 raw bytes). The HKDF salt is `SHA256(UTF-8("pill-phone-v2" + code + deviceId))` with `code` as shown in the QR (upper case) and `deviceId` the UUID string as sent in `/api/pair`. `info` is the 4 bytes `keys`. All base64url values are written WITHOUT padding and read with or without. `join` in the `join` message is the same string as in the relay block.
* **Relay URL.** `relay.url` is the Worker's base (`wss://host`); the phone connects to `<url>/r/<room>?role=phone` (a url that already contains `/r/` is used as is; `https://` is read as `wss://`; anything else, e.g. `ws://`, makes the phone treat the PC as having no relay).
* **No relay block = LAN only.** If the pair answer has no `pk` or no `relay`, the phone keeps working over the LAN and shows "Internet link: not available, update Pill on your PC". If both are present but the phone's derived `room`/`join` differ, or `pk` is not a valid 32-byte key, pairing FAILS on the phone and nothing is stored (the PC may be left with an unused device record).
* **`hello` over the relay.** The PC must send a v1 `hello` as soon as the relay tells it the phone is there (`joined` with `peer:true`, or `peer` with `up:true`), exactly as it does when a LAN socket opens: the phone replays notifications and the status on `hello`. `hello` may carry an optional `"relayMaxFileMB": <int>` (the `phone.relayMaxFileMB` setting); the phone defaults to 100 and shows the value read-only. If the PC speaks without a `hello`, the phone still counts it as present; 3 s after `peer up` with nothing received it sends a `ping`.
* **Liveness.** Over the relay the phone sends an encrypted `{"t":"ping"}` every 20 s only while the PC is present, and drops the connection after 50 s without any frame from the PC; OkHttp's WebSocket ping (25 s) keeps the NAT mapping open. `joined`/`peer` JSON text frames and close codes `4401` (join rejected, shown as "pair again"), `4409` (replaced) are treated as in the text above.
* **Replay window.** Per connection the receiver remembers the last `seq` of the 16 most recent `sid`s; a frame of an older `sid` that has fallen out of that window would be accepted once. A phone reconnect always uses a new random `sid`.
* **Files over the relay, details.**
  * Phone -> PC: `file.offer` / `file.accept` as in v1 (over the relay the accept is the go-ahead; the phone then sends `file.chunk`s). Receiver -> sender `file.ack {id,upto}` is **cumulative** and sent after each chunk is stored; the phone keeps at most 8 chunks (96 KiB raw) unacknowledged and sends `file.end {id,size}` after the last one. The upload counts as done when `upto == size` is acknowledged or the PC sends `file.done {id,ok:true}` (the PC should send `file.done` after verifying `size`; `ok:false`, `file.abort` or 60 s of silence fail it).
  * PC -> phone: the PC offers with `file.offer`; the phone, when the user (or auto-receive) accepts and the route is the relay, answers `file.accept {id}` and the PC then streams `file.chunk` (contiguous `off`, at most 8 unacknowledged), then `file.end`. The phone acks every chunk, verifies `size`, and answers `file.done {id,ok}` (as in v1). On a gap, duplicate or oversize chunk the phone sends `file.abort {id,reason:"bad_chunk"}`; either side may send `file.abort` at any time.
  * **Pacing.** The relay drops frames beyond 60 frames/s and 4 MiB/s per room, both directions together, and a dropped chunk kills the transfer. Both senders must therefore pace themselves: the phone sends at most ~3 MiB/s (one 96 KiB chunk per 32 ms). The PC should do the same.
  * A file larger than the cap is refused on the phone before the offer (upload) or when accepting (download) with a message to use the home Wi-Fi; a transfer that is running holds the phone on the relay (no LAN switch-back until it ends). Over the LAN nothing changed.
* **LAN switch-back probe.** While on the relay the phone calls `GET /api/state` with its bearer token on the stored LAN address (2 s timeout) every 60 s and when nudged (reconnect button, network callback); on `200` it drops the relay and reconnects over the LAN WebSocket. A network change restarts the whole attempt (LAN first, 2 s, then the relay). The PC must keep that endpoint cheap.
* **Settings.** The phone stores "Use internet relay when away from home" (default ON); OFF means LAN only, and the relay socket is closed at once.
* **Known gap.** A phone revoked on the PC while it is away only learns it on the next LAN contact (the relay room is gone; a rejoin would create an empty one). A `{"t":"revoked"}` frame is not part of v2.

### Fixed-key test vector (made by an independent Python implementation; asserted in `android/app/src/test/java/app/pill/phone/RelayTest.kt`)

```
phone secret  = 0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20 (hex)
pc secret     = 65666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f8081828384
phone pk      = B6N8vBQgk8i3VdwbEOhstCY3StFqqFPtC9_AsrhtHHw
pc pk         = VxR2nRFr92Q2rnS8eT0sMK0ZA8WaxSc4BcfiaYtBDDY
code, deviceId= AB3D9XKQ , 0b6f2f6e-6c7e-4c53-9d2a-1d0c5b7a9e11
shared        = c9ea6a3f79a000b60b076d4afc990b272f3f0b5aaa3f0b8713c209273e363863
salt          = 220b38e089346c3d86a79bffeabea4fd52d25fc5f36f2b72c44eebdbb5b40da8
k_p2c         = 6f29aa994173633730fd4e75b68768a37fd7a566053ee8b9eb547892d25121af
k_c2p         = e305543817a96cab46c969005eabfeb66aa15dd70bcd93a3f31beb9ea743b726
room          = 2fdc4b4165a4e9db4e0f4cf160d6d9e3
join          = NsHWn-LIc5AIRoIZMQEdcxf-AmAm9IdoAFk89Q6YKJY
```

Frames (nonce `a0a1...b7` fixed for the vector only; hex):

```
p2c envelope {"sid":"a1b2c3d4e5f60718","seq":1,"m":{"t":"ping"}}
p2c frame    a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b789056473c7a389b1db80a44d36411304040928e6c57ea0e40e475b3d740962081779a35e543d48cde87dcdde576e3aaa0126dbf10ae6d0b796da57c3f67332161e5f95
c2p envelope {"sid":"0011223344556677","seq":7,"m":{"t":"hello","pcName":"DESKTOP-X","serverVersion":1,"time":1700000000000}}
c2p frame    a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b74b5e5c8e21e8ba96a44f519ba3d061edd5f3647fb9b3930235a33f00b36717d055836d45e5ff3a68a141700eea3f1ca489d8a7ee22a0d6a54cd1d2890388aa9196673df1405638ae86dbf8c917786aa1ff6f810006275a3dea6b165d06d0a2ce70417ef0984529d34fa0d3fa5d71a4ff9e4de3309e246fb72b505748aebd21a2
```


## Changes by the PC side (v2)

What the Pill PC implements (`src-tauri/src/phone/{v2,relay,relay_files,wsclient}.rs`), including the few places where the text above was open. None of it contradicts the phone side's section; the PC reproduces the phone side's independent test vector above (`v2.rs::the_phone_sides_independent_vector`) and passes a second one it was developed against (`the_documented_test_vector`: phone secret = bytes 01..20 hex, PC secret = bytes 21..40 hex, code `AB3D9XKQ`, deviceId `dev-0001`; k_p2c `66b82e9d...f457`, room `9cb6afbf89e807519b65ebc880f4fa66`; both cross-checked with Python `cryptography`).

* **Pairing.** `pk` is accepted as base64url (padding optional; standard base64 also accepted). It is checked BEFORE the one-time code is spent: a key that is not 32 bytes, not base64, or a low-order point (all-zero shared secret) answers `400 bad_request` and the code stays valid. The PC makes a fresh X25519 key pair per pairing and forgets the secret at once; keys are derived even when the relay is off or has no URL (then the answer has `pk` but no `relay`). A pairing without `pk` stores no keys (and removes old ones of that `deviceId`).
* **Encodings** are exactly the phone side's: code upper-cased, `deviceId` as sent (it must match `[A-Za-z0-9_-]{1,64}`), room hex as ASCII in the AAD, envelope field order `sid`, `seq`, `m`.
* **Late relay configuration (optional message).** A phone paired while the relay was off has keys but no relay address. When `phone.relay` is on and a URL is set, the PC sends `{"t":"relay.config","url":"wss://...","room":"<32 hex>","join":"<b64url>"}` right after `hello` on every LAN WebSocket. A phone may store it (the values equal what pairing would have returned); phones that ignore unknown messages are unaffected.
* **`hello`** carries `"relayMaxFileMB"` on both routes and is (re)sent on `joined peer:true` and on every `peer up:true`, even if the PC thought the phone was already there (its earlier socket may have died unnoticed).
* **Route choice on the PC.** A phone may be on the LAN and the relay at once. The PC sends over the LAN socket and falls back to the relay if the LAN write fails; the relay link is standby. `phone_status` / `phone:state` devices have `route` = `lan` | `relay` | `none` and `relay` (bool: the phone has v2 keys). The Phone tab shows "Home network" / "Internet (relay)" / "Offline".
* **Connector.** One WebSocket per phone with keys, only while "Phone link" and `phone.relay` are ON and `phone.relayUrl` is set. URL: `wss://host[:port][/prefix]` (`https://` is read as `wss://`; `ws://` / `http://` only for loopback, for development). Back-off 1, 2, 4 ... 30 s (reset after a joined session), WebSocket ping every 25 s, a path is declared dead after 80 s without any frame if the relay ever answered a ping (the PC never requires a `pong`). 50 frames that fail authentication within 60 s close the connection. Replay protection keeps the last `seq` of the 64 most recent `sid`s and survives reconnects of the relay socket but NOT a restart of Pill (a frame from a forgotten `sid` would be accepted once). Turning a switch off, clearing the URL or revoking the device sends a WebSocket close 1000 and unregisters the phone at once; nothing reconnects.
* **Receiver acks.** The PC acknowledges an incoming upload every 4th chunk and at the last byte (cumulative `file.ack {id, upto}`), not after every chunk: with the relay's limit of 60 frames/s a 3 MiB/s transfer plus one ack per 96 KiB chunk would exceed it. The phone's window of 8 is never starved by this. After verifying `size` the PC sends the final `file.ack {upto:size}` and `file.done {id, ok:true}`; a mismatch or any error is `file.abort {id, reason}` with reason `size_mismatch`, `gap`, `too_large`, `too_big`, `bad_chunk`, `unknown_file`, `busy`, `disk`, `timeout`. An empty file has no chunks: `file.end {size:0}` alone creates it. A transfer with no chunk for 60 s is dropped (partial file deleted); so is every running transfer when the link or the phone goes away.
* **PC -> phone files.** After `file.accept` from the phone (relay route only) the PC streams contiguous 96 KiB chunks, never more than 8 unacknowledged, paced at <= 3 MiB/s, then `file.end {id,size}`, and waits up to 60 s for the phone's `file.done`. `file.abort` from the phone stops it at once. A file over `phone.relayMaxFileMB` is refused by `send_file` while the phone is on the relay (no offer is sent); an upload offer over it is answered with `file.reject {reason:"too_big"}`.
* **Storage.** Keys live in `%LOCALAPPDATA%\Pill\phone\devices.json` as the base64 of a DPAPI blob (current user, fixed entropy `pill.phone.v2.keys`) of the 96 key bytes; a blob that cannot be opened (another Windows user) reads as "no keys", the phone then works over the LAN only until it is paired again. Phones paired before v2 have no keys and keep working over the LAN.
* **Settings.** `phone.relay` (default true; idle without a URL), `phone.relayUrl` (default empty), `phone.relayMaxFileMB` (default 100, 1..4096).
