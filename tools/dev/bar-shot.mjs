// Renders the WEB command bar (src/bar.html + uninstall-ui.js) in a private headless Chrome with a stubbed window.__TAURI__ and realistic payloads,
// one PNG per scene, for the pixel comparison with the native bar (`PILL_BAR_DUMP=<dir> cargo test --release bar_view`, same scene names).
//   node tools/dev/bar-shot.mjs <outDir> [--segoe]     --segoe: force "Segoe UI" (what the native bar uses until Medeo Sans is bundled there)
// Nothing is written outside <outDir>; no real backend is reached (every invoke is answered by the stub below).
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { launch } from './headless.mjs';

const out = resolve(process.argv[2] || 'bar-shots');
const segoe = process.argv.includes('--segoe');
mkdirSync(out, { recursive: true });
const page = pathToFileURL(resolve(import.meta.dirname, '../../src/bar.html')).href;

const ROWS = [
  { id: 'app:1', kind: 'app', title: 'Firefox', subtitle: '', payload: 'C:\\Users\\a\\Firefox.lnk' },
  { id: 'app:2', kind: 'app', title: 'Fire Safety Notes', subtitle: '', payload: 'C:\\Users\\a\\Fire.lnk' },
  { id: 'folder:1', kind: 'folder', title: 'Fireworks', subtitle: 'C:\\Users\\a\\Pictures\\Fireworks', payload: 'C:\\Users\\a\\Pictures\\Fireworks' },
  { id: 'file:1', kind: 'file', title: 'fire-report.pdf', subtitle: 'C:\\Users\\a\\Documents', payload: 'C:\\Users\\a\\Documents\\fire-report.pdf' },
  { id: 'note:1', kind: 'note', title: 'Fire drill checklist', subtitle: 'yesterday', payload: '1' },
  { id: 'web:1', kind: 'web', title: 'Search Google for "fire"', subtitle: 'Opens your default browser', payload: 'fire' },
];
const UROW = [{ id: 'uninstall:3', kind: 'uninstall', title: 'qBittorrent', subtitle: 'The qBittorrent project \u00b7 4.6.5', payload: '3' }];
const INFO = { ok: true, name: 'qBittorrent', publisher: 'The qBittorrent project', version: '4.6.5', location: 'C:\\Program Files\\qBittorrent', sizeKb: 51200, store: false, canSilent: true, risky: false };
const ITEMS = [
  { id: 0, kind: 'folder', path: 'C:\\Users\\a\\AppData\\Roaming\\qBittorrent', size: 5242880, ticked: true, note: 'install folder', admin: false, tier: 'certain' },
  { id: 1, kind: 'file', path: 'C:\\Users\\a\\AppData\\Local\\qBittorrent\\qBittorrent.ini', size: 2048, ticked: true, note: '', admin: false, tier: 'certain' },
  { id: 2, kind: 'reg', path: 'HKCU\\Software\\qBittorrent', size: 0, ticked: true, note: '', admin: false, tier: 'certain' },
  { id: 3, kind: 'folder', path: 'C:\\Users\\a\\Documents\\torrents shared with the publisher', size: 734003200, ticked: false, note: '', admin: false, tier: 'probable' },
  { id: 4, kind: 'reg', path: 'HKLM\\SOFTWARE\\qBittorrent', size: 0, ticked: false, note: '', admin: true, tier: 'certain' },
];
const ICON = (() => { // the same 32x32 blue tile the native test uses
  const c = [];
  for (let y = 0; y < 32; y++) for (let x = 0; x < 32; x++) {
    const dx = Math.abs(x - 16), dy = Math.abs(y - 16), inside = dx < 13 && dy < 13 && !(dx > 10 && dy > 10), core = dx < 6 && dy < 6;
    c.push(...(core ? [140, 200, 255, 255] : inside ? [40, 110, 230, 255] : [0, 0, 0, 0]));
  }
  return c;
})();

const stub = `(() => {
  const H = {}, ICON = ${JSON.stringify(ICON)}, ROWS = ${JSON.stringify(ROWS)}, UROW = ${JSON.stringify(UROW)}, INFO = ${JSON.stringify(INFO)};
  const cfg = { language: 'en', theme: window.__THEME || 'dark', 'hotkey.command': 'Alt+Shift+Space', 'net.ai': true, 'ai.baseUrl': 'http://127.0.0.1:1/v1', 'ai.model': 'm', reduceMotion: false };
  let q = '';
  const icon = () => { const c = document.createElement('canvas'); c.width = c.height = 32; const g = c.getContext('2d'), d = g.createImageData(32, 32); d.data.set(ICON); g.putImageData(d, 0, 0); return c.toDataURL('image/png'); };
  window.__STATE = { phase: '' };
  window.__TAURI__ = {
    core: { invoke: async (c, a) => {
      if (c === 'settings_get') return cfg;
      if (c === 'cmd_query') { q = a.text; return /^u /i.test(q) ? UROW : /^\\?/.test(q) ? [] : q.trim() ? ROWS : []; }
      if (c === 'cmd_icon') return icon();
      if (c === 'uninstall_info') return INFO;
      if (c === 'uninstall_state') return window.__STATE;
      if (c === 'cmd_ask') return { ok: true, text: 'To list files use:\\n\\n\`\`\`powershell\\nGet-ChildItem -Recurse | Sort-Object Length\\n\`\`\`\\n\\nThe **-Recurse** switch walks *sub-folders*; \`Sort-Object\` orders by size.\\n\\n- first item\\n- second item' };
      return null;
    } },
    event: { listen: async (n, cb) => { (H[n] = H[n] || []).push(cb); return () => {}; } },
  };
  window.__emit = (n, p) => (H[n] || []).forEach((f) => f({ payload: p }));
})();`;

const b = await launch({ url: 'about:blank', width: 560, height: 350 });
await b.call('Page.enable');
await b.call('Emulation.setDeviceMetricsOverride', { width: 560, height: 350, deviceScaleFactor: 1, mobile: false });
await b.call('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function open(theme) {
  await b.call('Page.addScriptToEvaluateOnNewDocument', { source: `window.__THEME = ${JSON.stringify(theme)}; ${stub}${segoe ? ` document.addEventListener('DOMContentLoaded', () => { const s = document.createElement('style'); s.textContent = '* { font-family: "Segoe UI" !important }'; document.head.append(s); });` : ''}` });
  await b.call('Page.navigate', { url: page });
  await wait(900);
  await b.evaluate(`__emit('bar:shown', null); 0`);
  await wait(700);
}
async function type(text) {
  await b.evaluate(`(() => { const q = document.getElementById('q'); q.value = ${JSON.stringify(text)}; q.dispatchEvent(new Event('input')); return 0; })()`);
  await wait(900);
}
const key = (k, extra = {}) => b.evaluate(`window.dispatchEvent(new KeyboardEvent('keydown', Object.assign({ key: ${JSON.stringify(k)}, bubbles: true }, ${JSON.stringify(extra)}))); 0`);
async function shot(name) {
  await wait(700);
  const r = await b.call('Page.captureScreenshot', { format: 'png', clip: { x: 0, y: 0, width: 560, height: 350, scale: 1 }, fromSurface: true });
  writeFileSync(join(out, `${name}.png`), Buffer.from(r.data, 'base64'));
  console.log('wrote', name);
}
const state = (phase, extra = {}) => b.evaluate(`window.__STATE = Object.assign({ phase: ${JSON.stringify(phase)}, name: 'qBittorrent' }, ${JSON.stringify(extra)}); __emit('uninstall:state', window.__STATE); 0`);

try {
  await open('dark');
  await shot('empty');
  await type('fire');
  await key('ArrowDown');
  await shot('results');
  await type('? how do I list files');
  await key('Enter');
  await wait(900);
  await shot('answer');

  await open('dark');
  await type('u qbit');
  await key('Enter');
  await wait(900);
  await shot('u-confirm');
  await state('running');
  await wait(700);
  await state('scanning', { step: 'files and folders', found: 37 });
  await shot('u-scanning');
  await state('review', { items: ITEMS, manual: ['Driver: qbt-tun (listed only)'], cut: false, needAdmin: 1 });
  await shot('u-checklist');
  await state('done', { items: [], files: 3, keys: 1, other: 0, failed: [], backup: 'C:\\Users\\a\\AppData\\Local\\Pill\\backups\\qbt.reg', log: '' });
  await shot('u-summary');
  await state('scanning', { step: 'registry', found: 5 });
  await wait(500);
  await type('fire');
  await shot('u-strip');

  await open('light');
  await type('fire');
  await shot('light');
} finally {
  await b.close();
}
