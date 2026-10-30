// Web | native comparison of the set-A tabs: stacks <web>/<state>-<theme>-<s>x.png over <native>/<tab>/<name>-<theme>-x<s>.png into <out>/<state>-<theme>-<s>x.png
// and prints the mean absolute difference per state (anti-aliasing and font hinting alone give a few percent).
//   node tools/dev/tabs-a-web.mjs --scale 1 && cargo test --release tabs::a   (in src-tauri, CARGO_TARGET_DIR=target-agent)
//   node tools/dev/tabs-a-diff.mjs [--theme dark] [--scale 1] [--web dir] [--native dir] [--out dir]
import { parseArgs } from 'node:util';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { decodePng, encodePng } from './png.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const { values: o } = parseArgs({ options: { theme: { type: 'string', default: 'dark' }, scale: { type: 'string', default: '1' }, web: { type: 'string' }, native: { type: 'string' }, out: { type: 'string' } } });
const web = resolve(o.web || join(root, 'target-agent/snap/web-a')), nat = resolve(o.native || join(root, 'target-agent/tabs-a-snaps')), out = resolve(o.out || join(root, 'target-agent/snap/diff-a'));
mkdirSync(out, { recursive: true });
const TABS = ['home', 'weather', 'controls', 'notifs', 'clipboard', 'shelf'];
const rows = [];
for (const f of readdirSync(web).filter((x) => x.endsWith(`-${o.theme}-${o.scale}x.png`)).sort()) {
  const state = f.slice(0, -`-${o.theme}-${o.scale}x.png`.length);
  const tab = TABS.find((t) => state.startsWith(t + '-'));
  const np = join(nat, tab, `${state.slice(tab.length + 1)}-${o.theme}-x${o.scale}.png`);
  if (!existsSync(np)) { rows.push(`${state.padEnd(28)} (no native PNG ${np})`); continue; }
  const a = decodePng(readFileSync(join(web, f))), b = decodePng(readFileSync(np));
  const w = Math.max(a.w, b.w), h = a.h + b.h + 4, px = Buffer.alloc(w * h * 4, 0x80);
  const put = (img, y0) => { for (let y = 0; y < img.h; y++) img.px.copy(px, ((y0 + y) * w) * 4, y * img.w * 4, (y + 1) * img.w * 4); };
  put(a, 0); put(b, a.h + 4);
  let sum = 0, bad = 0;
  if (a.w === b.w && a.h === b.h) for (let i = 0; i < a.px.length; i += 4) { const d = (Math.abs(a.px[i] - b.px[i]) + Math.abs(a.px[i + 1] - b.px[i + 1]) + Math.abs(a.px[i + 2] - b.px[i + 2])) / 3; sum += d; if (d > 40) bad++; }
  writeFileSync(join(out, f), encodePng(w, h, px));
  rows.push(`${state.padEnd(28)} ${a.w === b.w && a.h === b.h ? `mean diff ${(sum / (a.px.length / 4) / 2.55).toFixed(2)}%  clearly different ${(bad / (a.px.length / 4) * 100).toFixed(1)}%` : `SIZE web ${a.w}x${a.h} native ${b.w}x${b.h}`}`);
}
console.log(rows.join('\n') + `\n-> ${out}`);
