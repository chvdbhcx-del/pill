// Post-deploy check: node scripts/smoke.mjs wss://pill-relay.<subdomain>.workers.dev
// Opens a pc and a phone socket in a throwaway room, joins both and echoes one binary frame each way.
import { randomBytes } from "node:crypto";

const base = (process.argv[2] ?? "").replace(/\/+$/, "");
if (!/^wss?:\/\//.test(base)) {
  console.error("usage: node scripts/smoke.mjs <wss-url>");
  process.exit(2);
}
const room = randomBytes(16).toString("hex");
const secret = randomBytes(32).toString("base64url");

const open = (role) =>
  new Promise((resolve, reject) => {
    const ws = new WebSocket(`${base}/r/${room}?role=${role}`);
    ws.binaryType = "arraybuffer";
    const inbox = [];
    let wake;
    ws.onmessage = (e) => (inbox.push(e.data), wake?.());
    ws.onerror = () => reject(new Error(`${role}: connection failed`));
    ws.onclose = (e) => (inbox.push({ close: e.code }), wake?.());
    ws.onopen = () => resolve({
      ws,
      send: (d) => ws.send(d),
      next: (ms = 5000) => new Promise((res, rej) => {
        const t = setTimeout(() => rej(new Error(`${role}: timed out`)), ms);
        const check = () => inbox.length && (clearTimeout(t), res(inbox.shift()));
        wake = check;
        check();
      }),
    });
  });

const expectJson = async (c, role, want) => {
  const m = await c.next();
  if (typeof m !== "string" || JSON.stringify(JSON.parse(m)) !== JSON.stringify(want)) throw new Error(`${role}: expected ${JSON.stringify(want)}, got ${typeof m === "string" ? m : JSON.stringify(m)}`);
};

const pc = await open("pc");
pc.send(JSON.stringify({ t: "join", join: secret, v: 2 }));
await expectJson(pc, "pc", { t: "joined", peer: false });
const phone = await open("phone");
phone.send(JSON.stringify({ t: "join", join: secret, v: 2 }));
await expectJson(phone, "phone", { t: "joined", peer: true });
await expectJson(pc, "pc", { t: "peer", up: true });

const payload = randomBytes(64);
pc.send(payload);
const got = await phone.next();
if (!(got instanceof ArrayBuffer) || !payload.equals(Buffer.from(got))) throw new Error("phone: frame mismatch");
phone.send(payload);
const back = await pc.next();
if (!(back instanceof ArrayBuffer) || !payload.equals(Buffer.from(back))) throw new Error("pc: frame mismatch");

pc.ws.close();
phone.ws.close();
console.log("relay OK: join, peer notifications and a binary frame in both directions");
process.exit(0);
