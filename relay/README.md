# Pill relay

Cloudflare Worker + Durable Objects that forward opaque, end-to-end encrypted frames between the PC and the phone when they are not on the same network. Protocol: `docs/phone-relay.md`. The relay stores only a SHA-256 verifier of the room's join secret and a last-activity timestamp; it never sees plaintext.

* `src/index.ts`: the Worker (routing), `Room` (one per room, WebSocket Hibernation API, SQLite storage) and `Limiter` (one per source IP, 20 room creations/hour).
* `test/`: vitest tests running in a local workerd (`@cloudflare/vitest-plugin`), no Cloudflare login needed.
* `scripts/smoke.mjs`: post-deploy check.

## Develop

```sh
npm install
npm test            # 17 tests
npm run typecheck
npx wrangler dev    # local server on http://127.0.0.1:8787
node scripts/smoke.mjs ws://127.0.0.1:8787
```

## Deploy (free plan is enough)

```sh
npx wrangler login      # once, opens a browser
npx wrangler deploy     # from relay/
node scripts/smoke.mjs wss://pill-relay.<account-subdomain>.workers.dev
```

No secrets or extra configuration. The first deploy applies the `v1` migration that creates the SQLite-backed `Room` and `Limiter` classes. Put the resulting `wss://` URL into the PC setting `phone.relayUrl`.

## Behaviour summary

| Rule | Value |
|---|---|
| Route | `GET /r/<32 hex>?role=pc\|phone` (WebSocket); `GET /` answers `pill relay`; anything else 404/400/405/426 |
| Join | first text message `{"t":"join","join":"<secret>","v":2}` within 5 s, else close 4408; malformed 4400; wrong secret 4401 |
| First claim | verifier = SHA-256 of the join string as sent, compared in constant time |
| Same role again | old socket closed 4409 |
| Frames | binary only, forwarded to the other role, dropped if absent; > 1 MiB closes 1009 |
| Rate | 60 frames/s and 4 MiB/s per room, excess dropped; 100 drops within 5 s closes 4429 |
| Room creation | 20/hour per `CF-Connecting-IP`, over that closes 4429 |
| Idle | no frames for 30 days: sockets closed, storage wiped (alarm) |
| Text frames `ping` | answered `pong` by the runtime without waking the room |

Hibernation notes: per-socket state (role, join state) lives in `serializeAttachment`, because the `Room` instance can be evicted while sockets stay open. The rate-limit window is deliberately soft in-memory state.

## Free plan limits that matter

* Workers: 100,000 requests/day (each connection upgrade counts), 10 ms CPU per request.
* Durable Objects: 100,000 requests/day; incoming WebSocket messages count 1 request per 20 messages; duration is not billed while a room hibernates.
* SQLite storage: 5 GB total, 100,000 rows written/day (the relay writes one row per hour of activity per room).
* Cloudflare may cap WebSocket messages at 32 MiB; the relay limits frames to 1 MiB itself.
