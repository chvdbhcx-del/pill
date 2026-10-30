// Renders the WEB versions of the panel-set-A tabs (home, weather, controls, notifications, clipboard, shelf) in a private headless Chrome with a stubbed
// Tauri runtime (tabs-a-stub.js), one PNG per state / theme / scale of the tab body #views (648 x 260 css px; 648 x 346 for weather), for the side-by-side
// with the native views (`cargo test --release tabs::a` writes <target>/tabs-a-snaps/<tab>/<state>-<dark|light>-x<1|2>.png from the SAME fixtures).
// Needs Node 22+ and Chrome or Edge (CHROME=path). Nothing is written outside --out.
//
//   node tools/dev/tabs-a-web.mjs [--only home,weather] [--states weather-rain] [--theme dark,light] [--scale 1,2] [--out dir] [--list]
//   output: <out>/<state>-<theme>-<scale>x.png
import { parseArgs } from 'node:util';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launch } from './headless.mjs';
import { encodePng } from './png.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const { values: o } = parseArgs({ options: { only: { type: 'string' }, states: { type: 'string' }, theme: { type: 'string', default: 'dark,light' }, scale: { type: 'string', default: '1,2' }, out: { type: 'string' }, list: { type: 'boolean', default: false } } });
const out = resolve(o.out || join(root, 'target-agent/snap/web-a'));
const themes = o.theme.split(','), scales = o.scale.split(',').map(Number);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const q = JSON.stringify;
const click = (sel, i = 0) => `(() => { const e = document.querySelectorAll(${q(sel)})[${i}]; if (!e) throw new Error('no ${sel}[${i}]'); e.click(); return 1; })()`;

// ---- fixtures: the same data the native tests feed (see src-tauri/src/ui/tabs/a/*/tests.rs) --------------------------------------------------------
const NOW = 1_793_369_109_000; // 2026-10-30 14:05:09 UTC
const MIN = 60_000, HOUR = 60 * MIN;
const pngUrl = (w, h, top, bottom) => {
  const px = Buffer.alloc(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) { const c = y < h / 2 ? top : bottom; px.set([c[0], c[1], c[2], 255], (y * w + x) * 4); }
  return 'data:image/png;base64,' + encodePng(w, h, px).toString('base64');
};
const media = (playing, cover) => ({ has_session: true, playing, title: 'Midnight City', artist: 'M83', album: 'Hurry Up', session_id: 's1', app: 'Spotify.exe', position_ms: 84000, duration_ms: 244000, thumb_id: 7, accent: '#aa3355', ...(cover ? { thumbnail: pngUrl(32, 32, [220, 60, 90], [40, 120, 200]) } : {}) });
const names = ['Notepad', 'Calculator', 'Downloads', 'Terminal', 'Browser'];
const links = (n) => Array.from({ length: n }, (_, i) => ({ id: i + 1, title: names[i % 5], target: `C:\\Apps\\${names[i % 5]}.exe`, missing: false, is_dir: i === 2, icon: null }));
const homeCommon = { media_sessions: [{ id: 's1', app: 'Spotify.exe', title: 'Midnight City', artist: 'M83', playing: true, selected: true }], media_volume_get: { volume: 42, muted: false }, media_devices: [{ id: 'd1', name: 'Speakers (Realtek)', default: true }, { id: 'd2', name: 'Headphones (USB)', default: false }] };
const n = (id, ago, kind, icon, tone, title, sub, read) => ({ id, ts: NOW - ago, kind, icon, tone, title, sub, read });
const history = [
  n(9, 20000, 'download', 'download', 'green', 'report-final.pdf', 'Downloaded · 2.4 MB', false), n(8, 12 * MIN, 'usb', 'usb', '', 'USB drive connected', 'E: · 58 GB', false),
  n(7, 50 * MIN, 'battery', 'battery', 'amber', 'Battery low', '18% · plug in the charger', true), n(6, 3 * HOUR, 'calendar', 'calendar', 'accent', 'Design review', 'Starts in 10 minutes', true),
  n(5, 5 * HOUR, 'mail', 'mail', '', 'Anna Petrova', '', true), n(4, 20 * HOUR, 'agent', 'terminal', '', 'Agent finished', 'build passed, 312 tests', true),
  n(3, 22 * HOUR, 'obs', 'video', 'red', 'Recording stopped', '00:42:10 · saved to Videos', true), n(2, 72 * HOUR, 'notif', 'bell', '', 'Windows update', '', true),
  n(1, 73 * HOUR, 'steam', 'download', 'green', 'Game update installed', 'Hades II · 1.2 GB', true),
];
const clip = (id, kind, preview, pinned = false) => ({ id, kind, preview, truncated: false, fileCount: kind === 'files' ? 3 : 0, size: 1234, pinned, source: 'Code.exe', createdAt: NOW - id * MIN, usedAt: NOW - id * 3 * MIN });
const clipText = [clip(1, 'text', 'fn main() { println!("hello"); }'), clip(2, 'text', 'https://example.com/some/long/link/that/does/not/fit/on/a/single/line/of/the/row'), clip(3, 'files', 'report.pdf, notes.txt +1'), clip(4, 'text', 'Meeting at 15:30, room 4B'), clip(5, 'text', 'Second line\nthird line of a multi-line copy')];
const clipImages = [clip(1, 'image', 'Screenshot 1920x1080'), clip(2, 'text', 'cargo test --release tabs::a'), clip(3, 'image', 'Image 640x480')];
const sh = (id, name, dir, icon) => ({ id, path: `C:/Users/me/${name}`, name, added_at: 1, missing: false, is_dir: dir, size: 1_500_000 * id, icon });
const shelfFiles = [['report.pdf', 0], ['notes.txt', 0], ['Projects', 1], ['archive.zip', 0], ['song.mp3', 0], ['slides.pptx', 0], ['data.csv', 0]].map(([nm, d], i) => sh(i + 1, nm, !!d, null));
const shelfThumbs = [sh(1, 'holiday.jpg', false, pngUrl(48, 48, [230, 90, 70], [70, 130, 230])), sh(2, 'report.pdf', false, null), sh(3, 'Projects', true, null), sh(4, 'poster.png', false, pngUrl(48, 48, [90, 200, 120], [250, 210, 70]))];
const BASE_T = 1_793_368_800_000;
const wx = (sym, icon, temp) => ({ status: 'ok', city: 'Oslo', lat: 59.9, lon: 10.7, fetched: 1, stale: false, warning: null, tz: 0,
  now: { temp, feels: temp - 2.5, symbol: sym, icon, wind: 5.2, dir: 225, humidity: 81, cloud: 90, precip: 0.4, min: temp - 3, max: temp + 3 },
  hourly: Array.from({ length: 8 }, (_, i) => ({ t: BASE_T + i * 3600000, temp: 4 + i * 0.7, symbol: sym, icon, precip: icon === 'rain' && i % 2 === 0 ? 0.4 : 0 })),
  daily: Array.from({ length: 7 }, (_, i) => ({ t: BASE_T + i * 86400000, dow: (5 + i) % 7, min: -3 + i, max: 2 + i * 1.5, symbol: 'x', icon: [icon, 'cloud', 'partly-day', 'rain', 'snow', 'clear-day', 'fog'][i], precip: 0 })),
  sun: { state: 'normal', rise: BASE_T - 32400000, set: BASE_T + 1800000 } });
const sysState = (bt) => ({ ethernet: { available: true, on: true }, wifi: { available: true, on: true, ssid: 'Home-5G' }, bluetooth: { available: true, on: bt, devices: [{ name: 'WH-1000XM5', connected: true, battery: 80 }, { name: 'MX Master 3', connected: true }, { name: 'Old speaker', connected: false }] }, dark: true, volume: 40, muted: false });

const S = (tab, replies, drive, extra = {}) => ({ tab, replies, drive, h: tab === 'weather' ? 346 : 260, ...extra });
const none = async () => {};
const STATES = {
  'home-playing-cover': S('home', { media_state: media(true, true), launcher_list: links(5), ...homeCommon }, none),
  'home-paused': S('home', { media_state: media(false, false), launcher_list: links(3), ...homeCommon }, none),
  'home-idle-empty-launcher': S('home', { media_state: { has_session: false }, launcher_list: [], ...homeCommon }, none),
  'controls-idle': S('controls', { sys_state: sysState(true), sys_set: { status: 'ok' } }, none, { settings: { 'live.sysmon': false } }), // the CPU / RAM meters on top are sysmon-live.js, not part of the tab
  'notifs-few': S('notifs', { notif_history_list: history.slice(0, 5) }, none),
  'notifs-many': S('notifs', { notif_history_list: history }, none),
  'notifs-empty': S('notifs', { notif_history_list: [] }, none),
  'clipboard-text': S('clipboard', { clip_status: { paused: false, count: 5 }, clip_list: clipText }, none),
  'clipboard-images': S('clipboard', { clip_status: { paused: false, count: 3 }, clip_list: clipImages, clip_image: pngUrl(48, 32, [220, 80, 60], [60, 120, 220]) }, none),
  'clipboard-empty': S('clipboard', { clip_status: { paused: false, count: 0 }, clip_list: [] }, none),
  'shelf-empty': S('shelf', { shelf_list: [] }, none),
  'shelf-files': S('shelf', { shelf_list: shelfFiles }, none),
  'shelf-thumbs': S('shelf', { shelf_list: shelfThumbs }, none),
  'weather-clear-day': S('weather', { weather_get: wx('clearsky', 'clear-day', 17) }, none),
  'weather-clear-night': S('weather', { weather_get: wx('clearsky', 'clear-night', 6) }, none),
  'weather-cloudy': S('weather', { weather_get: wx('cloudy', 'cloud', 9) }, none),
  'weather-rain': S('weather', { weather_get: wx('lightrain', 'rain', 4.4) }, none),
  'weather-snow': S('weather', { weather_get: wx('snow', 'snow', -4) }, none),
  'weather-thunder': S('weather', { weather_get: wx('heavyrainandthunder', 'thunder', 21) }, none),
  'weather-fog': S('weather', { weather_get: wx('fog', 'fog', 3) }, none),
  'weather-off': S('weather', { weather_get: null }, none, { settings: { 'net.weather': false } }),
  'weather-error': S('weather', { weather_get: { status: 'error', message: 'HTTP 503' } }, none),
  'weather-city': S('weather', { weather_get: { status: 'nocity' } }, none),
};
const NAMES = Object.keys(STATES);
if (o.list) { console.log(NAMES.join('\n')); process.exit(0); }

const want = o.states ? o.states.split(',') : o.only ? NAMES.filter((s) => o.only.split(',').includes(STATES[s].tab)) : NAMES;
const unknown = want.filter((s) => !STATES[s]);
if (unknown.length) { console.error('unknown: ' + unknown.join(', ')); process.exit(2); }
mkdirSync(out, { recursive: true });
const stub = readFileSync(join(here, 'tabs-a-stub.js'), 'utf8');
const b = await launch({ width: 720, height: 480 });
const failed = [], written = [];
let scriptId = null;
try {
  await b.call('Page.enable'); await b.call('Runtime.enable');
  for (const name of want) for (const theme of themes) {
    const st = STATES[name];
    try {
      const cfg = { now: NOW, replies: { settings_get: { theme, language: 'en', reduceMotion: true, size: 100, clock24: true, lastTab: '', pinned: true, autoHide: false, 'net.weather': true, ...(st.settings || {}) }, settings_info: { version: '0.5.3' }, battery_state: null, pill_monitors: [], ...st.replies }, storage: { 'pill.lang': 'en' } };
      if (scriptId) await b.call('Page.removeScriptToEvaluateOnNewDocument', { identifier: scriptId }).catch(() => {});
      scriptId = (await b.call('Page.addScriptToEvaluateOnNewDocument', { source: `const __TB_CFG = ${JSON.stringify(cfg)};\n${stub}` })).identifier;
      await b.call('Emulation.setDeviceMetricsOverride', { width: 720, height: 480, deviceScaleFactor: 1, mobile: false });
      await b.call('Emulation.setTimezoneOverride', { timezoneId: 'UTC' }).catch(() => {});
      await b.call('Emulation.setLocaleOverride', { locale: 'en-US' }).catch(() => {});
      await b.call('Page.navigate', { url: pathToFileURL(join(root, 'src', 'pill.html')).href });
      for (let i = 0; i < 100; i++) { if (await b.evaluate(`!!(window.Pill && window.__TB && document.readyState === 'complete')`)) break; await sleep(100); }
      await sleep(400);
      await b.evaluate(`Pill.expand(${q(st.tab)})`);
      let ok = false;
      for (let i = 0; i < 60 && !ok; i++) { await sleep(100); ok = await b.evaluate(`Pill.state === 'expanded' && !!document.querySelector('#views .view.on')`); }
      if (!ok) throw new Error(`tab ${st.tab} did not open`);
      await sleep(900);
      await st.drive();
      await sleep(500);
      for (const s of scales) {
        await b.call('Emulation.setDeviceMetricsOverride', { width: 720, height: 480, deviceScaleFactor: s, mobile: false });
        await sleep(s === scales[0] ? 120 : 350);
        const r = await b.evaluate(`(() => { const e = document.querySelector('#views'); const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()`);
        const clip = { x: Math.floor(r.x), y: Math.floor(r.y), width: 648, height: st.h, scale: 1 };
        const shot = await b.call('Page.captureScreenshot', { format: 'png', clip, captureBeyondViewport: false }, 20000);
        const file = `${name}-${theme}-${s}x.png`;
        writeFileSync(join(out, file), Buffer.from(shot.data, 'base64'));
        written.push(file);
      }
    } catch (e) { failed.push(`${name}/${theme}: ${e.message}`); console.error('FAILED ' + name + '/' + theme + ': ' + e.message); }
  }
} finally { await b.close(); }
console.log(`${written.length} PNG(s) -> ${out}${failed.length ? `\n${failed.length} failed:\n  ` + failed.join('\n  ') : ''}`);
process.exit(failed.length ? 1 : 0);
