import { env, exports } from "cloudflare:workers";
import { evictDurableObject, runDurableObjectAlarm, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";

const DAY = 86_400_000;
const newRoom = () => crypto.randomUUID().replaceAll("-", "");
const newSecret = () => btoa(crypto.randomUUID() + crypto.randomUUID()).replaceAll("=", "").slice(0, 43);
let ipCounter = 0;
const newIp = () => `10.0.${(ipCounter >> 8) & 255}.${++ipCounter & 255}`; // a fresh IP per test: separate creation budgets

type Ev = { msg: string | ArrayBuffer } | { close: number };

/** A WebSocket client with an event queue, so tests read like a conversation. */
class Client {
  private queue: Ev[] = [];
  private waiter?: (e: Ev) => void;
  closeCode?: number;
  private constructor(readonly ws: WebSocket) {
    ws.addEventListener("message", (e) => this.push({ msg: e.data as string | ArrayBuffer }));
    ws.addEventListener("close", (e) => {
      this.closeCode = e.code;
      this.push({ close: e.code });
    });
  }
  private push(e: Ev) {
    if (this.waiter) {
      const w = this.waiter;
      this.waiter = undefined;
      w(e);
    } else this.queue.push(e);
  }
  static async open(room: string, role: string, ip: string): Promise<Client> {
    const res = await (exports as any).default.fetch(`https://relay.test/r/${room}?role=${role}`, {
      headers: { Upgrade: "websocket", "CF-Connecting-IP": ip },
    });
    expect(res.status).toBe(101);
    res.webSocket!.binaryType = "arraybuffer";
    res.webSocket!.accept();
    return new Client(res.webSocket!);
  }
  /** Opens and joins; returns the client and the `joined` reply. */
  static async join(room: string, role: string, secret: string, ip: string) {
    const c = await Client.open(room, role, ip);
    c.sendJson({ t: "join", join: secret, v: 2 });
    return { c, reply: await c.nextJson() };
  }
  send(d: string | ArrayBuffer | Uint8Array) {
    this.ws.send(d);
  }
  sendJson(o: unknown) {
    this.ws.send(JSON.stringify(o));
  }
  next(ms = 2000): Promise<Ev> {
    const q = this.queue.shift();
    if (q) return Promise.resolve(q);
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error("timed out waiting for an event")), ms);
      this.waiter = (e) => (clearTimeout(t), resolve(e));
    });
  }
  async nextJson(): Promise<any> {
    const e = await this.next();
    if (!("msg" in e) || typeof e.msg !== "string") throw new Error("expected text, got " + JSON.stringify(e));
    return JSON.parse(e.msg);
  }
  async nextBinary(): Promise<Uint8Array> {
    const e = await this.next();
    if (!("msg" in e) || typeof e.msg === "string") throw new Error("expected binary, got " + JSON.stringify(e));
    return new Uint8Array(e.msg);
  }
  async nextClose(): Promise<number> {
    const e = await this.next();
    if (!("close" in e)) throw new Error("expected close");
    return e.close;
  }
  /** Asserts that nothing arrives for a short while. */
  async silent(ms = 150) {
    await new Promise((r) => setTimeout(r, ms));
    expect(this.queue).toEqual([]);
  }
  /** Waits for ms and returns everything received meanwhile. */
  async drain(ms = 300): Promise<Ev[]> {
    await new Promise((r) => setTimeout(r, ms));
    return this.queue.splice(0);
  }
}

/** pc + phone joined to a fresh room, join notifications consumed. */
async function pair() {
  const room = newRoom(), secret = newSecret(), ip = newIp();
  const pc = (await Client.join(room, "pc", secret, ip)).c;
  const phone = (await Client.join(room, "phone", secret, ip)).c;
  expect(await pc.nextJson()).toEqual({ t: "peer", up: true });
  return { room, secret, ip, pc, phone };
}

const roomStub = (room: string) => env.ROOM.get(env.ROOM.idFromName(room));
const bytes = (n: number, fill = 7) => new Uint8Array(n).fill(fill);

describe("routing", () => {
  it("answers health with nothing but the name", async () => {
    const r = await (exports as any).default.fetch("https://relay.test/");
    expect(r.status).toBe(200);
    expect(await r.text()).toBe("pill relay");
  });

  it("rejects bad paths, rooms, roles and non-upgrades", async () => {
    const f = (p: string, init?: RequestInit) => (exports as any).default.fetch("https://relay.test" + p, init);
    const up = { headers: { Upgrade: "websocket" } };
    expect((await f("/nope")).status).toBe(404);
    expect((await f("/r/abc?role=pc", up)).status).toBe(404);
    expect((await f(`/r/${newRoom().toUpperCase()}?role=pc`, up)).status).toBe(404);
    expect((await f(`/r/${newRoom()}?role=admin`, up)).status).toBe(400);
    expect((await f(`/r/${newRoom()}`, up)).status).toBe(400);
    expect((await f(`/r/${newRoom()}?role=pc`)).status).toBe(426);
    expect((await f(`/r/${newRoom()}?role=pc`, { method: "POST" })).status).toBe(405);
  });
});

describe("join", () => {
  it("first claim wins and tells whether the peer is present", async () => {
    const room = newRoom(), secret = newSecret(), ip = newIp();
    const a = await Client.join(room, "pc", secret, ip);
    expect(a.reply).toEqual({ t: "joined", peer: false });
    const b = await Client.join(room, "phone", secret, ip);
    expect(b.reply).toEqual({ t: "joined", peer: true });
    expect(await a.c.nextJson()).toEqual({ t: "peer", up: true });
  });

  it("closes 4401 on a wrong secret and keeps the room usable", async () => {
    const room = newRoom(), secret = newSecret(), ip = newIp();
    await Client.join(room, "pc", secret, ip);
    const bad = await Client.open(room, "phone", ip);
    bad.sendJson({ t: "join", join: newSecret(), v: 2 });
    expect(await bad.nextClose()).toBe(4401);
    expect((await Client.join(room, "phone", secret, ip)).reply.peer).toBe(true);
  });

  it("closes 4400 on anything that is not a join, 4408 after the deadline", async () => {
    const ip = newIp();
    for (const first of ['{"t":"hello"}', "garbage", '{"t":"join","join":"short","v":2}', '{"t":"join","join":"' + newSecret() + '","v":1}']) {
      const c = await Client.open(newRoom(), "pc", ip);
      c.send(first);
      expect(await c.nextClose()).toBe(4400);
    }
    const binary = await Client.open(newRoom(), "pc", ip);
    binary.send(bytes(10));
    expect(await binary.nextClose()).toBe(4400);

    // Deadline: age the socket's attachment and fire the alarm instead of waiting 5 s.
    const room = newRoom();
    const idle = await Client.open(room, "pc", ip);
    await runInDurableObject(roomStub(room), (_o, state) => {
      for (const ws of state.getWebSockets()) ws.serializeAttachment({ ...ws.deserializeAttachment(), t: Date.now() - 6000 });
    });
    expect(await runDurableObjectAlarm(roomStub(room))).toBe(true);
    expect(await idle.nextClose()).toBe(4408);
  });

  it("charges the room-creation limit per IP: 20 per hour", async () => {
    const ip = newIp();
    for (let i = 0; i < 20; i++) expect((await Client.join(newRoom(), "pc", newSecret(), ip)).reply.t).toBe("joined");
    const over = await Client.open(newRoom(), "pc", ip);
    over.sendJson({ t: "join", join: newSecret(), v: 2 });
    expect(await over.nextClose()).toBe(4429);
    // Other IPs are unaffected, and joining an existing room from the limited IP is not a creation.
    expect((await Client.join(newRoom(), "pc", newSecret(), newIp())).reply.t).toBe("joined");
    const room = newRoom(), secret = newSecret(), other = newIp();
    await Client.join(room, "pc", secret, other);
    expect((await Client.join(room, "phone", secret, ip)).reply).toEqual({ t: "joined", peer: true });
  });

  it("lets the limiter forget an IP once its hits are an hour old", async () => {
    const ip = newIp();
    const stub = env.LIMITER.get(env.LIMITER.idFromName(ip));
    for (let i = 0; i < 20; i++) expect(await stub.take()).toBe(true);
    expect(await stub.take()).toBe(false);
    await runInDurableObject(stub, async (_o, state) => {
      await state.storage.put("hits", Array.from({ length: 20 }, () => Date.now() - 2 * 3_600_000));
    });
    expect(await stub.take()).toBe(true); // old hits no longer count
    await runInDurableObject(stub, (_o, state) => state.storage.put("hits", [Date.now() - 2 * 3_600_000]));
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    await runInDurableObject(stub, async (_o, state) => expect(await state.storage.get("hits")).toBeUndefined());
  });
});

describe("forwarding", () => {
  it("passes binary frames unchanged both ways and ignores text", async () => {
    const { pc, phone } = await pair();
    const frame = Uint8Array.from({ length: 300 }, (_, i) => i % 256);
    pc.send(frame);
    expect(await phone.nextBinary()).toEqual(frame);
    phone.send(bytes(5, 9));
    expect(await pc.nextBinary()).toEqual(bytes(5, 9));
    pc.sendJson({ t: "hello" }); // text after join is ignored, not forwarded
    await phone.silent();
  });

  it("drops frames when the other role is absent and buffers nothing", async () => {
    const room = newRoom(), secret = newSecret(), ip = newIp();
    const pc = (await Client.join(room, "pc", secret, ip)).c;
    pc.send(bytes(10));
    await new Promise((r) => setTimeout(r, 100));
    const phone = (await Client.join(room, "phone", secret, ip)).c;
    await phone.silent();
  });

  it("announces peer down and up again", async () => {
    const { pc, phone } = await pair();
    phone.ws.close(1000, "bye");
    expect(await pc.nextJson()).toEqual({ t: "peer", up: false });
    pc.send(bytes(3)); // nobody there now
    await pc.silent();
  });

  it("keeps working after the Durable Object hibernated and woke", async () => {
    const { room, pc, phone } = await pair();
    await evictDurableObject(roomStub(room)); // sockets stay open, in-memory state is gone
    pc.send(bytes(4, 1));
    expect(await phone.nextBinary()).toEqual(bytes(4, 1));
    phone.ws.close(1000, "bye");
    expect(await pc.nextJson()).toEqual({ t: "peer", up: false }); // role comes from the socket attachment
  });
});

describe("replacement", () => {
  it("closes the old socket of a role with 4409 without flapping the peer", async () => {
    const { room, secret, ip, pc, phone } = await pair();
    const pc2 = await Client.join(room, "pc", secret, ip);
    expect(pc2.reply).toEqual({ t: "joined", peer: true });
    expect(await pc.nextClose()).toBe(4409);
    expect(await phone.nextJson()).toEqual({ t: "peer", up: true }); // announced by the new join, no "down" before it
    expect(await phone.drain(100)).toEqual([]);
    phone.send(bytes(2));
    expect(await pc2.c.nextBinary()).toEqual(bytes(2));
    await pc.silent(50);
  });
});

describe("limits", () => {
  it("closes 1009 above 1 MiB and accepts exactly 1 MiB", async () => {
    const { pc, phone } = await pair();
    pc.send(bytes(1 << 20));
    expect((await phone.nextBinary()).length).toBe(1 << 20);
    pc.send(bytes((1 << 20) + 1));
    expect(await pc.nextClose()).toBe(1009);
    expect(await phone.nextJson()).toEqual({ t: "peer", up: false });
  });

  it("drops beyond 60 frames per second, then closes 4429 for sustained abuse", async () => {
    const { pc, phone } = await pair();
    for (let i = 0; i < 100; i++) pc.send(bytes(8));
    const got = (await phone.drain(400)).filter((e) => "msg" in e);
    expect(got.length).toBe(60);
    for (let i = 0; i < 200; i++) pc.send(bytes(8));
    expect(await pc.nextClose()).toBe(4429);
  });

  it("drops beyond 4 MiB per second", async () => {
    const { pc, phone } = await pair();
    for (let i = 0; i < 6; i++) pc.send(bytes(1 << 20));
    const got = (await phone.drain(800)).filter((e) => "msg" in e);
    expect(got.length).toBe(4);
    expect(pc.closeCode).toBeUndefined(); // a few drops are not abuse
  });
});

describe("idle alarm", () => {
  it("wipes a room idle for 30 days and frees its id", async () => {
    const { room, secret, ip, pc, phone } = await pair();
    const stub = roomStub(room);

    // Recent activity: the alarm does nothing but re-arm.
    await runInDurableObject(stub, (_o, state) => state.storage.kv.put("last", Date.now() - 10 * DAY));
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    pc.send(bytes(1));
    expect(await phone.nextBinary()).toEqual(bytes(1));
    await runInDurableObject(stub, async (_o, state) => {
      expect(state.storage.kv.get("v")).toBeDefined();
      expect(await state.storage.getAlarm()).toBeGreaterThan(Date.now() + 19 * DAY);
    });

    await runInDurableObject(stub, (_o, state) => state.storage.kv.put("last", Date.now() - 31 * DAY));
    expect(await runDurableObjectAlarm(stub)).toBe(true);
    expect(await pc.nextClose()).toBe(1001);
    expect(await phone.nextClose()).toBe(1001);
    await runInDurableObject(stub, (_o, state) => {
      expect(state.storage.kv.get("v")).toBeUndefined();
      expect(state.storage.kv.get("last")).toBeUndefined();
    });
    // The old secret no longer matters: the room is claimable again.
    expect((await Client.join(room, "pc", newSecret(), ip)).reply).toEqual({ t: "joined", peer: false });
    expect(secret).toBeTruthy();
  });

  it("stores only the verifier and a timestamp", async () => {
    const { room } = await pair();
    await runInDurableObject(roomStub(room), (_o, state) => {
      const keys = [...state.storage.kv.list()].map(([k]) => k).sort();
      expect(keys).toEqual(["last", "v"]);
    });
  });
});
