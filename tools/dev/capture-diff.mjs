// Compares the native capture overlay with the web one (src/capture.html) on the same synthetic desktop.
//
//   set PILL_CAPTURE_DUMP=<dir>  &  cargo test --release capture_view::tests::web_parity_scenes     (writes frame.png + w-<scene>.png)
//   node tools/dev/capture-diff.mjs <dir> [scene ...]
//
// The web page runs in a private headless Chrome at 960 x 540 with `window.__TAURI__` stubbed: `capture_session` / `capture_frame` answer
// from frame.png, everything else is logged. Each scene replays the same pointer / keyboard script as the Rust test through CDP input events,
// waits for the CSS animations to settle and screenshots the page. Output: <dir>/web-<scene>.png, <dir>/diff-<scene>.png (web | native | diff x4)
// and a table of the mean absolute difference. Nothing is written outside <dir>.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launch } from './headless.mjs';
import { decodePng, encodePng } from './png.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const page = pathToFileURL(join(here, '..', '..', 'src', 'capture.html')).href;
const [dir, ...only] = process.argv.slice(2);
if (!dir) throw new Error('usage: node capture-diff.mjs <dump dir> [scene ...]');
const W = 960, H = 540;
const frame = decodePng(readFileSync(join(dir, 'frame.png')));
const rgba = Buffer.alloc(W * H * 4);
for (let i = 0; i < W * H; i++) { rgba[i * 4] = frame.px[i * 4]; rgba[i * 4 + 1] = frame.px[i * 4 + 1]; rgba[i * 4 + 2] = frame.px[i * 4 + 2]; rgba[i * 4 + 3] = 255; }

const stub = (mode) => `
  window.__calls = [];
  const frame = Uint8Array.from(atob('${rgba.toString('base64')}'), (c) => c.charCodeAt(0)).buffer;
  window.__TAURI__ = {
    core: { invoke: async (c, a) => {
      window.__calls.push(c);
      if (c === 'capture_session') return { id: 1, mode: '${mode}', w: ${W}, h: ${H}, x: 0, y: 0, scale: 1, focus: true };
      if (c === 'capture_frame') return frame;
      return null;
    } },
    event: { listen: () => {} },
  };`;

const ev = (b, type, x, y, extra = {}) => b.call('Input.dispatchMouseEvent', { type, x, y, button: 'left', buttons: type === 'mouseReleased' ? 0 : 1, clickCount: 1, ...extra });
const mv = (b, x, y) => b.call('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y, button: 'none', buttons: 0 });
const down = (b, x, y) => ev(b, 'mousePressed', x, y);
const up = (b, x, y) => ev(b, 'mouseReleased', x, y);
async function drag(b, [x0, y0], [x1, y1]) {
  await mv(b, x0, y0); await down(b, x0, y0);
  for (let i = 1; i <= 6; i++) { await mv(b, x0 + ((x1 - x0) * i) / 6, y0 + ((y1 - y0) * i) / 6); }
  await up(b, x1, y1);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const clickBtn = async (b, sel) => {
  const [x, y] = await b.evaluate(`(() => { const r = document.querySelector('${sel}').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()`);
  await mv(b, x, y); await down(b, x, y); await up(b, x, y);
  return [x, y];
};
const SEL = (a) => `#bar [data-a="${a}"]`;

const scenes = {
  idle: { mode: 'area', run: async () => {} },
  adjusting: { mode: 'area', run: async (b) => { await drag(b, [200, 120], [520, 380]); } },
  hover: { mode: 'area', run: async (b) => { await drag(b, [200, 120], [520, 380]); await sleep(500); const [x, y] = await b.evaluate(`(() => { const r = document.querySelector('${SEL('rect')}').getBoundingClientRect(); return [r.x + r.width / 2, r.y + r.height / 2]; })()`); await mv(b, x, y); } },
  markup: { mode: 'area', run: async (b) => {
    await drag(b, [120, 100], [760, 400]); await sleep(500);
    await clickBtn(b, SEL('rect')); await drag(b, [150, 120], [300, 200]);
    await clickBtn(b, SEL('arrow')); await drag(b, [320, 130], [460, 230]);
    await clickBtn(b, SEL('pen'));
    await mv(b, 480, 140); await down(b, 480, 140);
    for (let i = 0; i < 30; i++) { const t = i / 29; await mv(b, 480 + 120 * t, 140 + 40 * Math.sin(t * 9)); }
    await up(b, 600, 140);
    await clickBtn(b, SEL('blur')); await drag(b, [150, 240], [330, 300]);
    await clickBtn(b, SEL('text')); await mv(b, 250, 330); await down(b, 250, 330); await up(b, 250, 330);
    await b.call('Input.insertText', { text: 'Hello, markup' });
    await b.call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
    await b.call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
    await mv(b, 900, 520);
  } },
  loupe: { mode: 'color', run: async (b) => { await mv(b, 300, 200); } },
};

const diffRow = [];
for (const [name, sc] of Object.entries(scenes)) {
  if (only.length && !only.includes(name)) continue;
  const nat = join(dir, `w-${name}.png`);
  if (!existsSync(nat)) { console.log(`skip ${name}: no ${nat}`); continue; }
  const b = await launch({ width: W, height: H });
  try {
    await b.call('Page.addScriptToEvaluateOnNewDocument', { source: stub(sc.mode) });
    await b.call('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: 1, mobile: false });
    await b.call('Page.navigate', { url: page });
    await sleep(1200);
    await sc.run(b);
    await sleep(900);
    const shot = await b.call('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: W, height: H, scale: 1 } }, 20000);
    const webPng = Buffer.from(shot.data, 'base64');
    writeFileSync(join(dir, `web-${name}.png`), webPng);
    const A = decodePng(webPng), N = decodePng(readFileSync(nat));
    if (A.w !== N.w || A.h !== N.h) { diffRow.push([name, `size ${A.w}x${A.h} vs ${N.w}x${N.h}`]); continue; }
    let sum = 0, bad = 0;
    const o = Buffer.alloc(A.w * 3 * A.h * 4);
    for (let y = 0; y < A.h; y++) for (let x = 0; x < A.w; x++) {
      const i = (y * A.w + x) * 4; let d = 0;
      for (let c = 0; c < 3; c++) d += Math.abs(A.px[i + c] - N.px[i + c]);
      sum += d / 3; if (d / 3 > 24) bad++;
      for (let c = 0; c < 4; c++) { o[(y * A.w * 3 + x) * 4 + c] = A.px[i + c]; o[(y * A.w * 3 + A.w + x) * 4 + c] = N.px[i + c]; }
      const dv = Math.min(255, (d / 3) * 4);
      const k = (y * A.w * 3 + 2 * A.w + x) * 4; o[k] = dv; o[k + 1] = dv; o[k + 2] = dv; o[k + 3] = 255;
    }
    writeFileSync(join(dir, `diff-${name}.png`), encodePng(A.w * 3, A.h, o));
    diffRow.push([name, `${((sum / (A.w * A.h) / 255) * 100).toFixed(2)} % mean, ${((bad / (A.w * A.h)) * 100).toFixed(2)} % of pixels differ by > 24`]);
  } finally { await b.close(); }
}
for (const [n, r] of diffRow) console.log(n.padEnd(12), r);
