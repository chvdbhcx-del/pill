// Compares the native renders of the panel-set-B views with the web renders, state by state.
//
//   node tools/dev/tabs-b-web.mjs                                   (web PNGs -> target-agent/snap/web)
//   PILL_SNAP_DIR=target-agent/snap/native cargo test --release tabs::b   (native PNGs, written by kit::snap::shoot)
//   node tools/dev/tabs-b-diff.mjs [--web dir] [--native dir] [--out dir] [--threshold 4] [--only text,chat] [--theme dark] [--scale 1]
//
// Every name present in BOTH folders gets a side-by-side PNG (web | native | difference x4) in <out> (default target-agent/snap/diff) and a row
// in the table: mean absolute channel difference in percent of full scale, the share of clearly different pixels, and a size flag (the two
// pictures must have the same pixel size, else the sizes are listed and the picture is cropped to the common area). Sorted worst first.
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { decodePng, encodePng } from './png.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = resolve(here, '../..');
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); return i < 0 ? d : args[i + 1]; };
const webDir = resolve(opt('--web', join(repo, 'target-agent/snap/web')));
const natDir = resolve(opt('--native', join(repo, 'target-agent/snap/native')));
const outDir = resolve(opt('--out', join(repo, 'target-agent/snap/diff')));
const threshold = Number(opt('--threshold', '4'));
const only = opt('--only', '') ? opt('--only', '').split(',') : null;
const themeF = opt('--theme', ''), scaleF = opt('--scale', '');

const list = (d) => (existsSync(d) ? readdirSync(d).filter((f) => f.endsWith('.png')) : []);
const web = new Set(list(webDir));
const names = list(natDir).filter((f) => web.has(f) && (!only || only.some((o) => f.startsWith(o))) && (!themeF || f.includes(`-${themeF}-`)) && (!scaleF || f.endsWith(`-${scaleF}x.png`)));
if (!names.length) { console.error(`no common PNGs in ${webDir} and ${natDir}`); process.exit(2); }
mkdirSync(outDir, { recursive: true });

// the native PNGs are straight alpha over the panel colour already (kit::snap paints it); flatten anything translucent over black to be safe
const flat = (img) => { for (let i = 0; i < img.px.length; i += 4) { const a = img.px[i + 3] / 255; for (let c = 0; c < 3; c++) img.px[i + c] = Math.round(img.px[i + c] * a); img.px[i + 3] = 255; } return img; };

const rows = [];
for (const f of names) {
  const a = flat(decodePng(readFileSync(join(webDir, f)))), b = flat(decodePng(readFileSync(join(natDir, f))));
  const w = Math.min(a.w, b.w), h = Math.min(a.h, b.h);
  let sum = 0, bad = 0;
  const d = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const i = (y * a.w + x) * 4, j = (y * b.w + x) * 4, o = (y * w + x) * 4;
    let m = 0;
    for (let c = 0; c < 3; c++) { const e = Math.abs(a.px[i + c] - b.px[j + c]); sum += e; m = Math.max(m, e); d[o + c] = Math.min(255, e * 4); }
    d[o + 3] = 255;
    if (m > 24) bad++;
  }
  const mean = (sum / (w * h * 3 * 255)) * 100, pct = (bad / (w * h)) * 100;
  rows.push({ f, mean, pct, size: a.w === b.w && a.h === b.h ? '' : `web ${a.w}x${a.h} native ${b.w}x${b.h}` });
  // side by side
  const sw = w * 3 + 8, sh = h, px = Buffer.alloc(sw * sh * 4, 0);
  for (let y = 0; y < h; y++) {
    for (let x = 0; x < w; x++) {
      const o = (y * sw + x) * 4;
      a.px.copy(px, o, (y * a.w + x) * 4, (y * a.w + x) * 4 + 4);
      b.px.copy(px, o + (w + 4) * 4, (y * b.w + x) * 4, (y * b.w + x) * 4 + 4);
      d.copy(px, o + (2 * w + 8) * 4, (y * w + x) * 4, (y * w + x) * 4 + 4);
    }
  }
  if (mean > threshold / 4 || pct > threshold) writeFileSync(join(outDir, f), encodePng(sw, sh, px));
}
rows.sort((p, q) => q.mean - p.mean);
console.log('mean%  diff%px  name');
for (const r of rows) console.log(`${r.mean.toFixed(2).padStart(5)}  ${r.pct.toFixed(2).padStart(6)}  ${r.f}${r.size ? '   SIZE ' + r.size : ''}`);
console.log(`\n${rows.length} compared, side-by-sides of the ones above the threshold in ${outDir}`);
