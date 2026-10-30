// Compares the native pet renderer with the web pets, pixel by pixel.
//
//   cargo build --release --example pets_dump          (in src-tauri, CARGO_TARGET_DIR as you like)
//   node tools/dev/pets-diff.mjs [--exe path\to\pets_dump.exe] [--out dir] [--t 700] [--hooks] [--threshold 2.5] [ids...]
//
// For every pet and every state (calm music thinking done attention file sleep poked) the web pet is built in a private headless Chrome, all its
// animations are paused at t ms (document.getAnimations()[i].currentTime = t) and the pet box is screenshot over black; the example renders the
// same state at the same time through Direct2D / WARP. The table lists the mean absolute difference per pet and state in percent of full scale
// (anti-aliasing alone gives ~0.2 - 0.8), plus the share of pixels that are clearly different. Pets above --threshold get a side-by-side PNG
// (web | native | difference x4) in <out>/diff/. Nothing is written outside --out (default: a temp dir).
import { spawnSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync, existsSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launch } from './headless.mjs';
import { decodePng, encodePng } from './png.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '..', '..');
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); if (i < 0) return d; const v = args[i + 1]; args.splice(i, 2); return v; };
const flag = (n) => { const i = args.indexOf(n); if (i < 0) return false; args.splice(i, 1); return true; };
const exe = opt('--exe', join(process.env.CARGO_TARGET_DIR || join(repo, 'target-agent'), 'release', 'examples', 'pets_dump.exe'));
const out = opt('--out', mkdtempSync(join(tmpdir(), 'pets-diff-')));
const T = Number(opt('--t', '700'));
const hooks = flag('--hooks');
const chars = flag('--chars'); // compare the capsule characters (tail, legs + eyes) instead of the pets
const threshold = Number(opt('--threshold', '2.5'));
const side = opt('--side', 'right');
const BOX = 72, MARGIN = 16, SCALE = 2;
const ids = args;
const STATES = ['calm', 'music', 'thinking', 'done', 'attention', 'file', 'sleep', 'poked'];

// ---- native side
const nat = join(out, 'native'), webd = join(out, 'web'), dif = join(out, 'diff');
[nat, webd, dif].forEach((d) => mkdirSync(d, { recursive: true }));
if (!existsSync(exe)) throw new Error(`${exe} not found: cargo build --release --example pets_dump`);
const r = spawnSync(exe, [nat, ...(chars ? ['--chars'] : []), '--t', String(T), '--scale', String(SCALE), '--box', String(BOX), '--margin', String(MARGIN), '--side', side, ...(hooks ? ['--hooks'] : []), ...ids], { stdio: ['ignore', 'ignore', 'inherit'], windowsHide: true });
if (r.status !== 0) throw new Error('pets_dump failed');

// ---- web side
const b = await launch({ dsf: SCALE, width: 900, height: 700 });
if (chars) {
  // characters.js animates from performance.now(): freeze the clock at 10 s and drive its rAF callback by hand (setState draws at 10 s, then t ms later)
  const cstates = ['calm', 'music', 'thinking', 'done', 'attention', 'file', 'sleep'], crow = [];
  try {
    await b.call('Page.navigate', { url: pathToFileURL(join(here, 'pets-harness.html')).href });
    await new Promise((r) => setTimeout(r, 1500));
    for (const kind of ['tail', 'legs']) {
      const line = { id: kind };
      for (const st of cstates) {
        await b.evaluate(`(async () => {
          const stage = document.getElementById('stage'); stage.innerHTML = '';
          let cb = null; window.__cb = () => cb; window.requestAnimationFrame = (f) => { cb = f; return 1; }; performance.now = () => 10000;
          const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
          svg.setAttribute('viewBox', '0 0 149 92'); svg.setAttribute('width', 149); svg.setAttribute('height', 92); svg.style.cssText = 'position:absolute;left:0;top:0;background:#000';
          stage.append(svg);
          const h = window.PillChars.preview(svg, '${kind}'); h.setState('${st}');
          window.__cb()(10000 + ${T});
          return 0;
        })()`);
        const shot = await b.call('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 149, height: 92, scale: SCALE } }, 20000);
        const webPng = Buffer.from(shot.data, 'base64');
        writeFileSync(join(webd, `chars_${kind}_${st}.png`), webPng);
        const A = decodePng(webPng), Bm = decodePng(readFileSync(join(nat, `chars_${kind}_${st}.png`)));
        let sum = 0; const d = Buffer.alloc(A.w * A.h * 4);
        for (let i = 0; i < A.w * A.h; i++) { const dr = Math.abs(A.px[i * 4] - Bm.px[i * 4]), dg = Math.abs(A.px[i * 4 + 1] - Bm.px[i * 4 + 1]), db = Math.abs(A.px[i * 4 + 2] - Bm.px[i * 4 + 2]); sum += (dr + dg + db) / 3; d[i * 4] = Math.min(255, dr * 4); d[i * 4 + 1] = Math.min(255, dg * 4); d[i * 4 + 2] = Math.min(255, db * 4); d[i * 4 + 3] = 255; }
        const mean = (sum / (A.w * A.h) / 255) * 100; line[st] = mean;
        if (mean > threshold) { const W = A.w * 3, o = Buffer.alloc(W * A.h * 4); for (let y = 0; y < A.h; y++) { A.px.copy(o, (y * W) * 4, y * A.w * 4, (y + 1) * A.w * 4); Bm.px.copy(o, (y * W + A.w) * 4, y * A.w * 4, (y + 1) * A.w * 4); d.copy(o, (y * W + 2 * A.w) * 4, y * A.w * 4, (y + 1) * A.w * 4); } writeFileSync(join(dif, `chars_${kind}_${st}.png`), encodePng(W, A.h, o)); }
      }
      crow.push(line);
    }
  } finally { await b.close(); }
  console.log(`characters-diff  t=${T} ms  (Settings preview card 149 x 92 css px x${SCALE})  out=${out}`);
  console.log('kind'.padEnd(8) + cstates.map((s) => s.padEnd(11)).join(''));
  for (const l of crow) console.log(l.id.padEnd(8) + cstates.map((s) => (l[s].toFixed(2) + (l[s] > threshold ? '*' : '')).padEnd(11)).join(''));
  process.exit(0);
}
const rows = [];
try {
  await b.call('Page.navigate', { url: pathToFileURL(join(here, 'pets-harness.html')).href });
  await new Promise((r) => setTimeout(r, 1500));
  await b.evaluate(`window.requestAnimationFrame = () => 0; document.getElementById('stage').innerHTML = ''; 0`); // the JS tick hooks stay at their initial pose (like --hooks off)
  const all = await b.evaluate('PetsExport.ids()');
  for (const id of all) {
    if (ids.length && !ids.includes(id)) continue;
    const row = { id };
    for (const st of STATES) {
      const dim = await b.evaluate(`(async () => {
        const P = window.PillPets, stage = document.getElementById('stage'); stage.innerHTML = '';
        const def = P.registry['${id}'], bw = ${BOX} * def.ar, bh = ${BOX}, m = ${MARGIN};
        const cell = document.createElement('div'); cell.style.cssText = 'position:absolute;left:0;top:0;background:#000;width:' + (bw + 2 * m) + 'px;height:' + (bh + 2 * m) + 'px;overflow:hidden';
        stage.append(cell);
        const inst = P.create('${id}', { state: '${st === 'poked' ? 'calm' : st}', side: '${side}', size: 'm', idle: false });
        inst.el.style.setProperty('--pet-px', bh + 'px'); inst.el.style.position = 'absolute'; inst.el.style.left = m + 'px'; inst.el.style.top = m + 'px';
        cell.append(inst.el);
        ${st === 'poked' ? 'inst.pet();' : ''}
        void inst.el.offsetWidth;
        for (const a of inst.el.getAnimations({ subtree: true })) { a.pause(); a.currentTime = ${T}; }
        await new Promise((r) => setTimeout(r, 120));
        return { w: bw + 2 * m, h: bh + 2 * m };
      })()`);
      const shot = await b.call('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: dim.w, height: dim.h, scale: SCALE } }, 20000);
      const webPng = Buffer.from(shot.data, 'base64');
      writeFileSync(join(webd, `${id}_${st}.png`), webPng);
      const A = decodePng(webPng), Bm = decodePng(readFileSync(join(nat, `${id}_${st}.png`)));
      if (A.w !== Bm.w || A.h !== Bm.h) { row[st] = { err: `size ${A.w}x${A.h} vs ${Bm.w}x${Bm.h}` }; continue; }
      let sum = 0, bad = 0; const d = Buffer.alloc(A.w * A.h * 4);
      for (let i = 0; i < A.w * A.h; i++) {
        const dr = Math.abs(A.px[i * 4] - Bm.px[i * 4]), dg = Math.abs(A.px[i * 4 + 1] - Bm.px[i * 4 + 1]), db = Math.abs(A.px[i * 4 + 2] - Bm.px[i * 4 + 2]);
        sum += (dr + dg + db) / 3; if (Math.max(dr, dg, db) > 48) bad++;
        d[i * 4] = Math.min(255, dr * 4); d[i * 4 + 1] = Math.min(255, dg * 4); d[i * 4 + 2] = Math.min(255, db * 4); d[i * 4 + 3] = 255;
      }
      const mean = (sum / (A.w * A.h) / 255) * 100, badp = (bad / (A.w * A.h)) * 100;
      row[st] = { mean, bad: badp };
      if (mean > threshold) { // web | native | diff x4
        const W = A.w * 3, o = Buffer.alloc(W * A.h * 4);
        for (let y = 0; y < A.h; y++) { A.px.copy(o, (y * W) * 4, y * A.w * 4, (y + 1) * A.w * 4); Bm.px.copy(o, (y * W + A.w) * 4, y * A.w * 4, (y + 1) * A.w * 4); d.copy(o, (y * W + 2 * A.w) * 4, y * A.w * 4, (y + 1) * A.w * 4); }
        writeFileSync(join(dif, `${id}_${st}.png`), encodePng(W, A.h, o));
      }
    }
    rows.push(row);
    console.error(id);
  }
} finally {
  await b.close();
}
// ---- table
const pad = (s, n) => String(s).padEnd(n);
console.log(`pets-diff  t=${T} ms  hooks=${hooks ? 'on (web static)' : 'off'}  box=${BOX}px x${SCALE}  threshold=${threshold}%   out=${out}`);
console.log(pad('pet', 11) + STATES.map((s) => pad(s, 11)).join('') + 'worst');
const worst = [];
for (const r of rows) {
  let line = pad(r.id, 11), w = 0;
  for (const st of STATES) { const c = r[st]; line += pad(c.err ? 'SIZE' : c.mean.toFixed(2) + (c.mean > threshold ? '*' : ''), 11); if (c.mean) { w = Math.max(w, c.mean); worst.push([c.mean, r.id, st]); } }
  console.log(line + w.toFixed(2));
}
worst.sort((a, c) => c[0] - a[0]);
console.log('worst 10: ' + worst.slice(0, 10).map((x) => `${x[1]}/${x[2]} ${x[0].toFixed(2)}`).join(', '));
writeFileSync(join(out, 'table.json'), JSON.stringify(rows));
