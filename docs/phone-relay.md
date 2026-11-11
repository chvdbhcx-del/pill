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
* **Cost/privacy note shown in Settings**: "Through the relay your data is end-to-end encrypted; the relay (a Cloudflare Worker you own) sees only that two devices exchange data, when, and how much."

## Settings

PC: `phone.relay` (bool, default ON when a relay URL is set), `phone.relayUrl` (default = the deployed Worker, empty until deployed), `phone.relayMaxFileMB` (100). Phone app: "Use internet relay when away from home" switch (default ON) and the file cap shown read-only.

## Deploying the relay

`relay/` contains the Worker (`wrangler.toml`, `src/index.ts`, tests). Deploy with `npx wrangler login` (once, in a browser) then `npx wrangler deploy` from `relay/`; the URL is `https://pill-relay.<account-subdomain>.workers.dev`. Uses only the free plan features: a Durable Object class with SQLite storage (`new_sqlite_classes`) and the WebSocket Hibernation API. No secrets are needed.
