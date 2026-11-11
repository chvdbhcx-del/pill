// Pill Phone relay (protocol: docs/phone-relay.md).
// A Worker routes /r/<room>?role=pc|phone to one Room Durable Object per room, which forwards opaque
// binary frames between the two roles. The relay never sees plaintext and never logs content.
import { DurableObject } from "cloudflare:workers";

interface Env {
  ROOM: DurableObjectNamespace<Room>;
  LIMITER: DurableObjectNamespace<Limiter>;
}

const HOUR = 3_600_000;
const JOIN_MS = 5_000; // time a fresh socket has to send its join message
const IDLE_MS = 30 * 24 * HOUR; // a room without frames for this long is wiped
const MAX_FRAME = 1 << 20; // 1 MiB
const MAX_FPS = 60; // frames per second per room
const MAX_BPS = 4 << 20; // bytes per second per room
const ABUSE_DROPS = 100; // dropped frames within ABUSE_MS close the sender with 4429
const ABUSE_MS = 5_000;
const ROOMS_PER_IP_PER_HOUR = 20;

type Role = "pc" | "phone";

// Per-socket state. With hibernation the Room instance (and its fields) can be thrown away between
// messages while sockets stay open, so anything per socket must live in the attachment (max 2 KiB,
// structured-cloned). Instance fields below are only soft state that may reset at any wake-up.
interface Attachment {
  role: Role;
  ip: string; // only used to charge the room-creation limit; never stored on disk
  t: number; // connect time, for the join deadline
  joined: boolean;
  replaced: boolean; // closed with 4409 by a newer socket: do not announce peer-down
}

export default {
  async fetch(req: Request, env: Env): Promise<Response> {
    const url = new URL(req.url);
    if (url.pathname === "/") {
      return req.method === "GET" ? new Response("pill relay", { headers: { "content-type": "text/plain" } }) : new Response(null, { status: 405 });
    }
    const m = /^\/r\/([0-9a-f]{32})$/.exec(url.pathname);
    if (!m) return new Response(null, { status: 404 });
    const role = url.searchParams.get("role");
    if (role !== "pc" && role !== "phone") return new Response(null, { status: 400 });
    if (req.method !== "GET") return new Response(null, { status: 405 });
    if (req.headers.get("Upgrade")?.toLowerCase() !== "websocket") return new Response(null, { status: 426 });
    return env.ROOM.get(env.ROOM.idFromName(m[1])).fetch(req);
  },
};

async function sha256(s: string): Promise<ArrayBuffer> {
  return crypto.subtle.digest("SHA-256", new TextEncoder().encode(s));
}

export class Room extends DurableObject<Env> {
  // Soft state, reset when the object is evicted/hibernated (fine: rates are per second).
  private recent: { t: number; n: number }[] = []; // frames accepted in the last second
  private dropWindow = { start: 0, count: 0 };
  private lastTouch = 0;
  private stats = { frames: 0, bytes: 0, dropped: 0 };

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    // Answered by the runtime without waking the object, so keep-alives cost nothing.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair("ping", "pong"));
  }

  async fetch(req: Request): Promise<Response> {
    const role = new URL(req.url).searchParams.get("role") as Role;
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);
    // acceptWebSocket (not server.accept) opts in to hibernation; the tag lets us find sockets by role.
    this.ctx.acceptWebSocket(server, [role]);
    const att: Attachment = { role, ip: req.headers.get("CF-Connecting-IP") ?? "unknown", t: Date.now(), joined: false, replaced: false };
    server.serializeAttachment(att);
    await this.schedule(); // join deadline
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, msg: string | ArrayBuffer): Promise<void> {
    const att = ws.deserializeAttachment() as Attachment;
    if (!att.joined) {
      let join: unknown;
      try {
        const j = JSON.parse(typeof msg === "string" ? msg : "");
        if (j.t === "join" && j.v === 2) join = j.join;
      } catch {}
      if (typeof join !== "string" || !/^[A-Za-z0-9_-]{16,128}$/.test(join)) return ws.close(4400, "join expected");
      return this.join(ws, att, join);
    }
    if (typeof msg === "string") return; // only the join message is meaningful as text

    if (msg.byteLength > MAX_FRAME) return ws.close(1009, "frame too large");
    if (!this.allow(msg.byteLength)) return this.abuse(ws);
    this.stats.frames++;
    this.stats.bytes += msg.byteLength;
    await this.touch();
    // Forward to the other role only; with nobody there the frame is dropped (nothing is buffered).
    for (const peer of this.joined(att.role === "pc" ? "phone" : "pc")) {
      try { peer.send(msg); } catch {}
    }
  }

  async webSocketClose(ws: WebSocket, code: number): Promise<void> {
    // The runtime requires us to complete the close handshake; 1005/1006/1015 cannot be echoed.
    try { ws.close([1005, 1006, 1015].includes(code) ? 1000 : code); } catch {}
    this.left(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    this.left(ws);
  }

  // Runs for two reasons: sockets that missed the join deadline, and the 30-day idle wipe.
  async alarm(): Promise<void> {
    const now = Date.now();
    const last = this.ctx.storage.kv.get<number>("last");
    if (last !== undefined && now - last >= IDLE_MS) {
      for (const ws of this.ctx.getWebSockets()) try { ws.close(1001, "room expired"); } catch {}
      await this.ctx.storage.deleteAlarm();
      await this.ctx.storage.deleteAll(); // verifier and timestamp gone: the room id is free again
      console.log(JSON.stringify({ ev: "room_wiped" }));
      return;
    }
    for (const ws of this.ctx.getWebSockets()) {
      const att = ws.deserializeAttachment() as Attachment;
      if (!att.joined && now - att.t >= JOIN_MS) try { ws.close(4408, "join timeout"); } catch {}
    }
    await this.schedule();
  }

  private async join(ws: WebSocket, att: Attachment, secret: string): Promise<void> {
    const hash = await sha256(secret);
    let verifier = this.ctx.storage.kv.get<ArrayBuffer>("v");
    if (!verifier) {
      // New room: first claim wins, but creating one is charged to the caller's IP.
      if (!(await this.env.LIMITER.get(this.env.LIMITER.idFromName(att.ip)).take())) return ws.close(4429, "too many rooms");
      // Re-read: another socket may have claimed the room while we awaited the limiter.
      verifier = this.ctx.storage.kv.get<ArrayBuffer>("v");
      if (!verifier) {
        this.ctx.storage.kv.put("v", hash);
        verifier = hash;
      }
    }
    if (!crypto.subtle.timingSafeEqual(hash, verifier)) return ws.close(4401, "bad join");

    // A newer socket of the same role replaces the old one (e.g. the phone changed network).
    for (const old of this.joined(att.role)) {
      if (old === ws) continue;
      const a = old.deserializeAttachment() as Attachment;
      old.serializeAttachment({ ...a, replaced: true });
      try { old.close(4409, "replaced"); } catch {}
    }
    ws.serializeAttachment({ ...att, joined: true });
    const other = att.role === "pc" ? "phone" : "pc";
    ws.send(JSON.stringify({ t: "joined", peer: this.joined(other).length > 0 }));
    this.notify(other, true);
    await this.touch(true);
    await this.schedule();
  }

  /** Open sockets of a role that completed the join. */
  private joined(role: Role): WebSocket[] {
    return this.ctx.getWebSockets(role).filter((s) => s.readyState === WebSocket.OPEN && (s.deserializeAttachment() as Attachment).joined);
  }

  private notify(role: Role, up: boolean): void {
    for (const s of this.joined(role)) try { s.send(JSON.stringify({ t: "peer", up })); } catch {}
  }

  private left(ws: WebSocket): void {
    const att = ws.deserializeAttachment() as Attachment | null;
    if (!att?.joined || att.replaced) return;
    // Announce peer-down only when no other socket of that role is still there.
    if (this.joined(att.role).every((s) => s === ws)) this.notify(att.role === "pc" ? "phone" : "pc", false);
    console.log(JSON.stringify({ ev: "close", ...this.stats }));
  }

  /** Sliding one-second window over accepted frames: <= 60 frames and <= 4 MiB. */
  private allow(n: number): boolean {
    const now = Date.now();
    this.recent = this.recent.filter((f) => now - f.t < 1000);
    const bytes = this.recent.reduce((s, f) => s + f.n, 0);
    if (this.recent.length >= MAX_FPS || bytes + n > MAX_BPS) return false;
    this.recent.push({ t: now, n });
    return true;
  }

  private abuse(ws: WebSocket): void {
    const now = Date.now();
    this.stats.dropped++;
    if (now - this.dropWindow.start > ABUSE_MS) this.dropWindow = { start: now, count: 0 };
    if (++this.dropWindow.count >= ABUSE_DROPS) ws.close(4429, "rate limit");
  }

  /** Records activity. Throttled to one storage write per hour: 30 days of idleness is coarse anyway. */
  private async touch(force = false): Promise<void> {
    const now = Date.now();
    if (!force && now - this.lastTouch < HOUR) return;
    this.lastTouch = now;
    this.ctx.storage.kv.put("last", now);
  }

  /** One alarm serves both purposes: whichever deadline comes first. */
  private async schedule(): Promise<void> {
    let next = Infinity;
    for (const ws of this.ctx.getWebSockets()) {
      const att = ws.deserializeAttachment() as Attachment;
      if (!att.joined && ws.readyState === WebSocket.OPEN) next = Math.min(next, att.t + JOIN_MS);
    }
    const last = this.ctx.storage.kv.get<number>("last");
    if (last !== undefined) next = Math.min(next, last + IDLE_MS);
    if (next !== Infinity) await this.ctx.storage.setAlarm(next);
  }
}

// Counts room creations per source IP (one object per IP). Only timestamps are kept, and the
// record deletes itself an hour after the last creation.
export class Limiter extends DurableObject<Env> {
  async take(): Promise<boolean> {
    const hits = await this.recentHits();
    if (hits.length >= ROOMS_PER_IP_PER_HOUR) return false;
    hits.push(Date.now());
    await this.ctx.storage.put("hits", hits);
    await this.ctx.storage.setAlarm(hits[0] + HOUR);
    return true;
  }

  async alarm(): Promise<void> {
    const hits = await this.recentHits();
    if (hits.length) await this.ctx.storage.setAlarm(hits[0] + HOUR);
    else await this.ctx.storage.deleteAll();
  }

  private async recentHits(): Promise<number[]> {
    const now = Date.now();
    return ((await this.ctx.storage.get<number[]>("hits")) ?? []).filter((t) => now - t < HOUR);
  }
}
