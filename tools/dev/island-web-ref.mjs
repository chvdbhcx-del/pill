// Reference snapshots of the WEB island (src/pill.html with its real JS/CSS) in private headless Chrome, for comparing the native (Direct2D) island side by side.
//
//   node tools/dev/island-web-ref.mjs [scenario...]        render all scenarios or the named ones
//   node tools/dev/island-web-ref.mjs --list                print the scenario names
//   node tools/dev/island-web-ref.mjs --out some\dir        output dir (default: <repo>/target-agent/web-ref)
//
// Per scenario it writes <name>.png (transparent), <name>.grey.png (composited on #6b6f76), <name>.rects.json (getBoundingClientRect of the island's
// elements, css px), <name>.styles.json (computed font / colour of the main text elements) and <name>.log.json (invoke calls the page made, JS errors),
// plus README.txt (payloads, settings, key numbers). Only headless Chrome via CDP is used (no pill.exe, no screen capture, no input simulation).
//
// How: src/ is served by a tiny local http server; a script injected with Page.addScriptToEvaluateOnNewDocument stubs window.__TAURI__
// (core.invoke answered from a table, event.listen registry fed by window.__emit) and freezes Date at Fri 30 Oct 2026 12:34:00 local time.
// Time-based animation (liquid edge phase, equaliser bars, pulsing dot, pet idle motion) still runs on the real clock: those pixels differ per run.
import { createServer } from 'node:http';
import { mkdirSync, readFileSync, writeFileSync, existsSync } from 'node:fs';
import { dirname, extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { launch } from './headless.mjs';
import { decodePng, encodePng } from './png.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const repo = join(here, '..', '..');
const src = join(repo, 'src');

// =====================================================================================================================================================
// SCENARIO TABLE (the one place to edit).  NOW = the frozen clock (local time).
//   settings : patch over BASE (settings.rs defaults, pet 'none', weather on)      invokes : patch over INVOKES (answers of window.__TAURI__.core.invoke)
//   events   : [{ name, payload }] or [{ name, js }] (js = expression evaluated IN the page, e.g. the canvas cover), emitted in order after the page settled
//   viewport : [w, h] css px (window frame: 720x120 collapsed, 720x200 with an event card, 720x480 expanded)    dsf : deviceScaleFactor
//   navLang : navigator.language the page sees (default en-US: the UI strings are English; 'auto' language follows it)   settle   : ms of real time to wait after the last event    reducedMedia : emulate prefers-reduced-motion: reduce
// =====================================================================================================================================================
const NOW = new Date(2026, 9, 30, 12, 34, 0).getTime(); // Fri 30 Oct 2026 12:34:00 local
const WX = { temp: 18.4, icon: 'partly-day', text: 'Partly cloudy' };
// settings.rs defaults() + the keys the page reads that Rust only adds on demand. Deviations from the Rust defaults: pet 'none' (the task: no pet), net.weather true (shows the weather).
const BASE = {
  size: 100, theme: 'dark', clock24: true, autoHide: false, monitor: 'primary', reduceMotion: false, showInCapture: true, hideInApps: [],
  'hotkey.toggle': 'Ctrl+Alt+I', 'hotkey.command': 'Alt+Shift+Space', 'hotkey.clipboard': 'Ctrl+Alt+V',
  language: 'auto', shape: 'notch', character: 'liquid', pet: 'none', 'pet.side': 'right', 'pet.size': 'm', hideInFullscreen: true, autoHideKeepDesktop: false, updateAuto: true, whatsNew: true,
  underTaskbar: true, passThrough: false, passThroughOpacity: 50, offsetX: 0, musicPill: 'title', 'audio.reactive': true, 'sound.reminders': true, 'sound.reminderTone': 'chime', firstRunDone: true,
  'module.media': true, 'module.shelf': true, 'module.launcher': true, 'module.fileSearch': true, 'module.clipboard': true, 'module.text': true, 'module.chat': true, 'module.note': true,
  'module.weather': true, 'module.controls': true, 'module.uninstall': true, 'module.notifs': true,
  'live.volume': true, 'live.music': true, 'live.micCam': true, 'live.downloads': true, 'live.recording': true, 'live.notifications': true, 'live.reminders': true, 'live.keyboardLayout': true,
  'live.copy': true, 'live.obs': false, 'live.agents': true, 'live.steam': true, 'live.sysmon': true, 'live.dlDone': true, 'live.shelf': true, 'live.usb': true, 'live.battery': true, 'live.calendar': false,
  'capture.format': 'mp4', 'capture.fps': 30, 'capture.shotAction': 'both', 'history.keep': true, 'quiet.on': false,
  'net.weather': true, // not a Rust default: Settings > Weather switch (pill.js wxWanted). Without it the page ignores the weather.
};
const INVOKES = {
  battery_state: { has_battery: false, percent: 0, charging: false },
  weather_get: { status: 'ok', now: WX },
  media_state: null, media_sessions: [], media_devices: [], media_volume_get: { volume: 40, muted: false },
  clip_list: [], clip_status: { paused: false }, shelf_list: [], launcher_list: [], note_list: [], remind_list: [], timer_state: null, chat_threads: [], notif_history_list: [],
  pill_monitors: ['Primary'], agents_list: [], agents_tools: [], capture_settings: { recording: false }, net_features_status: {}, ai_config: {}, sysmon_get: null, livesys_privacy: null,
};

// 64 x 64 cover, exactly as specified: r = min(255, 40 + 3x), g = min(255, 90 + 2y), b = max(0, 200 - x - y), a = 255
const COVER_JS = `(() => { const c = document.createElement('canvas'); c.width = c.height = 64; const x = c.getContext('2d'), d = x.createImageData(64, 64);
  for (let y = 0; y < 64; y++) for (let X = 0; X < 64; X++) { const i = (y * 64 + X) * 4; d.data[i] = Math.min(255, 40 + 3 * X); d.data[i + 1] = Math.min(255, 90 + 2 * y); d.data[i + 2] = Math.max(0, 200 - X - y); d.data[i + 3] = 255; }
  x.putImageData(d, 0, 0); return c.toDataURL('image/png'); })()`;
const MEDIA = { has_session: true, playing: true, title: 'Midnight City', artist: 'M83', album: "Hurry Up, We're Dreaming", session_id: 'spotify', app: 'Spotify.exe', position_ms: 61000, duration_ms: 243000, thumb_id: 1, accent: '#2a7bc8' };
const mediaEvent = { name: 'media:changed', js: `Object.assign(${JSON.stringify(MEDIA)}, { thumbnail: ${COVER_JS} })` };
const wxEvent = { name: 'weather:current', payload: WX };
const REC = { name: 'rec:state', payload: { recording: true, startedAt: NOW - 65000, mode: 'screen', fps: 30 } };
const CARD = [720, 200];
const SHOT_PATH = String.raw`C:\Users\demo\Pictures\Pill\shot-0042.png`;

const SCENARIOS = {
  clock_weather: { settings: {}, events: [wxEvent] },
  clock_date: { settings: { showDate: true }, events: [wxEvent] }, // the date piece (Fri 30 Oct) of the clock row
  clock_weather_capsule: { settings: { shape: 'capsule' }, events: [wxEvent] }, // pill.js' own default shape; settings.rs default is 'notch'
  music: { settings: {}, events: [wxEvent, mediaEvent] }, // musicPill 'title' (settings.rs default): equaliser + title, no cover in the capsule
  music_mini: { settings: { musicPill: 'mini' }, events: [wxEvent, mediaEvent] }, // cover + title + prev / play / next
  recording: { settings: {}, events: [wxEvent, REC] },
  uninstall: { settings: {}, viewport: CARD, settle: 1300, events: [wxEvent, { name: 'uninstall:ready', payload: { name: 'Contoso Editor', kind: 'app', count: 3 } }] }, // kind 'app' is not one of review/done/still/error: the page shows its fallback text
  toast: { settings: {}, viewport: CARD, settle: 1300, events: [wxEvent, { name: 'capture:done', payload: { kind: 'screenshot', path: SHOT_PATH, copied: true, w: 1920, h: 1080 } }] },
  toast_ru: { settings: {}, navLang: 'ru-RU', viewport: CARD, settle: 1300, events: [wxEvent, { name: 'capture:done', payload: { kind: 'screenshot', path: SHOT_PATH, copied: true, w: 1920, h: 1080 } }] }, // navigator.language ru-RU: Russian UI strings
  notification: { settings: {}, viewport: CARD, settle: 1300, events: [wxEvent, { name: 'live:notification', payload: { app: 'Slack', title: 'Anna K.', body: 'Are we still on for the 3 pm review? I pushed the new build.', id: 'n1' } }] },
  expanded: { settings: {}, viewport: [720, 480], settle: 1200, events: [wxEvent, { name: 'pill:expand', payload: { tab: 'home', via: 'test' } }] },
  music_expanded: { settings: {}, viewport: [720, 480], settle: 1400, events: [wxEvent, mediaEvent, { name: 'pill:expand', payload: { tab: 'home', via: 'test' } }] }, // Home tab with the player card (cover, seek bar)
  light: { settings: { theme: 'light' }, events: [wxEvent] },
  scale2: { settings: {}, dsf: 2, events: [wxEvent] },
  reduced: { settings: { reduceMotion: true }, reducedMedia: true, events: [wxEvent, mediaEvent] },
  pet_cat: { settings: { pet: 'cat' }, settle: 1400, events: [wxEvent] },
  pet_eyes: { settings: { pet: 'eyes' }, settle: 1400, events: [wxEvent] }, // the 'eyes' pet (pets-chars.js): only the eyes
  music_pet: { settings: { pet: 'cat' }, settle: 1600, events: [wxEvent, mediaEvent] }, // the pet is music-tinted (accent colour of the cover)
  char_legs: { settings: { character: 'legs' }, settle: 1400, events: [wxEvent] }, // the `legs` edge character: eyes in the row + legs under the island
  char_tail: { settings: { character: 'tail' }, settle: 1400, events: [wxEvent] },
};
// =====================================================================================================================================================

// elements whose rectangle is recorded (css px, viewport coordinates): selector -> {x,y,w,h} (several matches -> array)
const RECT_SELECTORS = ['#stage', '#body', '#clip', '#live', '#live-in', '.lp', '.lp-clock', '.lp-l', '.lp-r', '.lp-time', '.lp-time .d', '.lp-date', '.lp-ico', '.lp-wx', '.lp-wx svg', '.lp-aux', '.lp-batt',
  '.lp-music', '.lp-cover', '.lp-eq', '.lp-eq i', '.lp-name', '.lp-btns button', '.lp-rec', '.rec-dot', '.rec-t', '.rec-sp', '.rec-fmt', '.rec-area', '.rec-stop',
  '.tc', '.tc .ti', '.tc .ti svg', '.tc .tx', '.tc .tt', '.tc .ts', '.tc .msg', '.lx-n', '.lx-ico', '.lx-nt', '.lx-nb', '.tc .ta', '.tc .ab',
  '#liq', '#pet', '.pet', '.pet-rig', '#chars', '#panel', '#head', '#tabs', '.tab', '.tab svg', '.tab .tl', '#hdr', '.hbtn', '.hbtn svg', '#pet-slot', '#views', '.view.on', '.th-player', '.th-art', '.th-title', '.th-artist', '.th-tile', '.th-vol'];
// elements whose computed text style is recorded
const STYLE_SELECTORS = ['#body', '.lp-time', '.lp-date', '.lp-aux', '.lp-name', '.rec-t', '.rec-stop', '.rec-fmt', '.tc', '.tc .tt', '.tc .ts', '.tc .ab', '.tc .ab.white', '.lx-ico', '.lx-nt', '.lx-nb', '.tab', '.tab .tl', '.hbtn', '.th-title', '.th-artist', '.th-src', '.th-tile'];
const STYLE_PROPS = ['fontFamily', 'fontSize', 'fontWeight', 'fontStyle', 'letterSpacing', 'lineHeight', 'color', 'textTransform', 'backgroundColor', 'borderRadius', 'boxShadow', 'opacity', 'zoom', 'transform'];

// ---------------------------------------------------------------------------------------------------------------------------------- args
const args = process.argv.slice(2);
const opt = (n, d) => { const i = args.indexOf(n); if (i < 0) return d; const v = args[i + 1]; args.splice(i, 2); return v; };
const outDir = resolve(opt('--out', join(repo, 'target-agent', 'web-ref')));
if (args.includes('--list')) { console.log(Object.keys(SCENARIOS).join('\n')); process.exit(0); }
const names = args.length ? args : Object.keys(SCENARIOS);
for (const n of names) if (!SCENARIOS[n]) { console.error(`unknown scenario "${n}". Known: ${Object.keys(SCENARIOS).join(' ')}`); process.exit(2); }
mkdirSync(outDir, { recursive: true });

// ---------------------------------------------------------------------------------------------------------------------------------- static server for src/
const MIME = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.ttf': 'font/ttf', '.woff2': 'font/woff2', '.png': 'image/png', '.svg': 'image/svg+xml', '.json': 'application/json' };
const server = createServer((req, res) => {
  const p = normalize(join(src, decodeURIComponent(new URL(req.url, 'http://x').pathname)));
  if (!p.startsWith(src + sep) || !existsSync(p)) { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'content-type': MIME[extname(p)] || 'application/octet-stream', 'cache-control': 'no-store' });
  res.end(readFileSync(p));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const origin = `http://127.0.0.1:${server.address().port}`;

// ---------------------------------------------------------------------------------------------------------------------------------- the injected stub
const stubSource = (cfg) => `(() => {
  const CFG = ${JSON.stringify(cfg)};
  const RealDate = Date;
  class FrozenDate extends RealDate { constructor(...a) { if (a.length === 0) super(CFG.now); else super(...a); } static now() { return CFG.now; } }
  window.Date = FrozenDate;
  for (const k of ['language']) Object.defineProperty(Navigator.prototype, k, { get: () => CFG.navLang, configurable: true });
  Object.defineProperty(Navigator.prototype, 'languages', { get: () => [CFG.navLang], configurable: true });
  const lis = {}, settings = Object.assign({}, CFG.settings);
  window.__calls = []; window.__errs = [];
  window.addEventListener('error', (e) => window.__errs.push('error: ' + e.message + ' @' + (e.filename || '').split('/').pop() + ':' + e.lineno));
  window.addEventListener('unhandledrejection', (e) => window.__errs.push('rejection: ' + (e.reason && e.reason.message || e.reason)));
  const ce = console.error.bind(console); console.error = (...a) => { window.__errs.push('console.error: ' + a.map((x) => (x && x.message) || String(x)).join(' ').slice(0, 300)); ce(...a); };
  window.__emit = (name, payload) => { for (const cb of (lis[name] || []).slice()) { try { cb({ event: name, id: 0, payload }); } catch (e) { window.__errs.push('listener ' + name + ': ' + e.message); } } };
  const invoke = async (cmd, a) => {
    window.__calls.push(a === undefined ? cmd : cmd + ' ' + JSON.stringify(a).slice(0, 120));
    if (cmd === 'settings_get') return Object.assign({}, settings);
    if (cmd === 'settings_set') { Object.assign(settings, (a && a.patch) || {}); setTimeout(() => window.__emit('settings:changed', Object.assign({}, settings)), 0); return Object.assign({}, settings); }
    if (cmd in CFG.invokes) return JSON.parse(JSON.stringify(CFG.invokes[cmd]));
    return null;
  };
  window.__TAURI__ = {
    core: { invoke },
    event: { listen: async (name, cb) => { (lis[name] || (lis[name] = [])).push(cb); return () => { lis[name] = (lis[name] || []).filter((f) => f !== cb); }; }, emit: async (name, payload) => window.__emit(name, payload) },
  };
})();`;

// ---------------------------------------------------------------------------------------------------------------------------------- in-page measurements
const measureSource = `(() => {
  const RS = ${JSON.stringify(RECT_SELECTORS)}, SS = ${JSON.stringify(STYLE_SELECTORS)}, SP = ${JSON.stringify(STYLE_PROPS)};
  const r2 = (n) => Math.round(n * 100) / 100;
  const rc = (e) => { const r = e.getBoundingClientRect(); return { x: r2(r.x), y: r2(r.y), w: r2(r.width), h: r2(r.height) }; };
  const rects = {}, styles = {}, hide = !document.documentElement.classList.contains('expanded'), shown = (e) => { const r = e.getBoundingClientRect(); return (r.width > 0 || r.height > 0) && !(hide && e.closest('#panel')); }; // collapsed: the panel exists but is clipped away, its boxes are not on screen
  for (const s of RS) { const es = [...document.querySelectorAll(s)]; if (!es.length) continue; const vis = es.filter(shown); if (!vis.length) continue; rects[s] = vis.length === 1 ? rc(vis[0]) : vis.map(rc); }
  const one = (e) => { const cs = getComputedStyle(e), o = {}; for (const p of SP) o[p] = cs[p]; const t = (e.textContent || '').trim().replace(/\\s+/g, ' '); if (t && t.length < 100) o.text = t; if (e.tagName === 'BUTTON' || e.className) o.cls = String(e.className).slice(0, 60); return o; };
  for (const s of SS) { const es = [...document.querySelectorAll(s)].filter(shown); if (!es.length) continue; styles[s] = es.length === 1 ? one(es[0]) : es.slice(0, 12).map(one); }
  const b = document.getElementById('body');
  rects.__meta = { innerWidth: innerWidth, innerHeight: innerHeight, dpr: devicePixelRatio, dataset: Object.assign({}, document.documentElement.dataset), htmlClass: document.documentElement.className, bodyBorderRadius: b ? getComputedStyle(b).borderRadius : null, bodyInline: b ? { w: b.style.width, h: b.style.height, left: b.style.left, r: b.style.borderRadius } : null, now: new Date().toString(), lang: window.Pill && Pill.lang };
  return { rects, styles, errs: window.__errs, calls: window.__calls };
})()`;

// ---------------------------------------------------------------------------------------------------------------------------------- run
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const GREY = [0x6b, 0x6f, 0x76];
const b = await launch({ width: 900, height: 700 });
const summary = [];
try {
  await b.call('Page.enable');
  for (const name of names) {
    const sc = SCENARIOS[name], [w, h] = sc.viewport || [720, 120], dsf = sc.dsf || 1;
    const t0 = Date.now();
    const cfg = { now: NOW, navLang: sc.navLang || 'en-US', settings: { ...BASE, ...sc.settings }, invokes: { ...INVOKES, ...(sc.invokes || {}) } };
    const { identifier } = await b.call('Page.addScriptToEvaluateOnNewDocument', { source: stubSource(cfg) });
    await b.call('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: dsf, mobile: false });
    await b.call('Emulation.setDefaultBackgroundColorOverride', { color: { r: 0, g: 0, b: 0, a: 0 } });
    await b.call('Emulation.setEmulatedMedia', { media: '', features: [{ name: 'prefers-reduced-motion', value: sc.reducedMedia ? 'reduce' : 'no-preference' }] });
    await b.call('Page.navigate', { url: `${origin}/pill.html` });
    for (let i = 0; i < 100; i++) { if (await b.evaluate(`document.readyState === 'complete' && !!window.Pill`).catch(() => false)) break; await sleep(100); }
    await b.evaluate(`document.fonts.ready.then(() => 1)`);
    await sleep(700); // settings_get / weather_get / pets start()
    for (const ev of sc.events || []) {
      await b.evaluate(ev.js ? `window.__emit(${JSON.stringify(ev.name)}, ${ev.js})` : `window.__emit(${JSON.stringify(ev.name)}, ${JSON.stringify(ev.payload)})`);
      await sleep(120);
    }
    await sleep(sc.settle || 900);
    await b.evaluate(`document.fonts.ready.then(() => 1)`);
    const m = await b.evaluate(measureSource);
    const shot = await b.call('Page.captureScreenshot', { format: 'png', fromSurface: true, captureBeyondViewport: false, clip: { x: 0, y: 0, width: w, height: h, scale: 1 } }, 20000);
    const png = Buffer.from(shot.data, 'base64');
    const img = decodePng(png), grey = Buffer.alloc(img.w * img.h * 4);
    let opaque = 0, partial = 0;
    for (let i = 0; i < img.w * img.h; i++) {
      const a = img.px[i * 4 + 3] / 255;
      if (a > 0.99) opaque++; else if (a > 0) partial++;
      for (let c = 0; c < 3; c++) grey[i * 4 + c] = Math.round(img.px[i * 4 + c] * a + GREY[c] * (1 - a));
      grey[i * 4 + 3] = 255;
    }
    writeFileSync(join(outDir, `${name}.png`), png);
    writeFileSync(join(outDir, `${name}.grey.png`), encodePng(img.w, img.h, grey));
    writeFileSync(join(outDir, `${name}.rects.json`), JSON.stringify(m.rects, null, 1));
    writeFileSync(join(outDir, `${name}.styles.json`), JSON.stringify(m.styles, null, 1));
    writeFileSync(join(outDir, `${name}.log.json`), JSON.stringify({ errors: m.errs, invokeCalls: m.calls }, null, 1));
    await b.call('Page.removeScriptToEvaluateOnNewDocument', { identifier });
    await b.call('Page.navigate', { url: 'about:blank' });
    const bd = m.rects['#body'];
    summary.push(name);
    console.log(`${name.padEnd(22)} ${img.w}x${img.h}  body ${bd ? `${bd.x},${bd.y} ${bd.w}x${bd.h}` : 'MISSING'}  opaque ${opaque}px partial ${partial}px  errors ${m.errs.length}${m.errs.length ? ' [' + m.errs[0].slice(0, 90) + ']' : ''}  ${((Date.now() - t0) / 1000).toFixed(1)}s`);
  }
} finally {
  await b.close();
  server.close();
}

// ---------------------------------------------------------------------------------------------------------------------------------- README.txt (always regenerated from the table + the rects files on disk)
const L = [];
L.push('WEB ISLAND REFERENCE SNAPSHOTS  (tools/dev/island-web-ref.mjs)', '');
L.push('Source: src/pill.html + its real JS / CSS in private headless Chrome, served over http from src/. window.__TAURI__ is stubbed; Date is frozen at Fri 30 Oct 2026 12:34:00 local time.');
L.push('Per scenario: <name>.png (transparent), <name>.grey.png (on #6b6f76), <name>.rects.json (getBoundingClientRect, css px, viewport coords), <name>.styles.json (computed text styles), <name>.log.json (invoke calls + page errors).');
L.push('Viewport = window frame in css px: 720x120 collapsed, 720x200 while an event card is up, 720x480 expanded. deviceScaleFactor 1 unless stated. PNG size = viewport x dsf.');
L.push('NOT deterministic (real clock): liquid-edge wave phase, equaliser bar heights, pulsing rec dot, pet idle animation, spring settle (we wait settle ms after the last event).');
L.push('', 'BASE SETTINGS (answer of settings_get; = settings.rs defaults() with pet "none", firstRunDone true, net.weather true; per-scenario patches below):', JSON.stringify(BASE), '');
L.push('INVOKE TABLE (everything else answers null):', JSON.stringify(INVOKES), '');
L.push('Frozen clock: Date = ' + new Date(NOW).toString() + '  (ms ' + NOW + '). Weather: ' + JSON.stringify(WX) + ' (event weather:current AND weather_get -> {status:"ok", now:{...}}).');
L.push('Cover image (media thumbnail): 64x64, per pixel r=min(255,40+3x) g=min(255,90+2y) b=max(0,200-x-y) a=255, sent as a PNG data URL in media:changed.thumbnail.', 'media:changed payload (+ thumbnail): ' + JSON.stringify(MEDIA), '');
L.push('SCENARIOS');
for (const [n, sc] of Object.entries(SCENARIOS)) {
  L.push(`- ${n}: ${n}.png / ${n}.grey.png   viewport ${(sc.viewport || [720, 120]).join('x')}  dsf ${sc.dsf || 1}  settle ${sc.settle || 900} ms${sc.reducedMedia ? '  prefers-reduced-motion: reduce emulated' : ''}`);
  L.push(`    settings patch: ${JSON.stringify(sc.settings)}${sc.invokes ? '   invoke patch: ' + JSON.stringify(sc.invokes) : ''}`);
  for (const ev of sc.events || []) L.push(`    event ${ev.name}: ${ev.payload ? JSON.stringify(ev.payload) : ev.name === 'media:changed' ? 'media payload above (+ generated cover)' : '(js)'}`);
}
L.push('', 'NOTES', '- Language: navigator.language is forced to en-US (UI strings English; scenario toast_ru uses ru-RU). Settings language stays "auto".', '- Default musicPill is "title" (equaliser + title, NO cover in the capsule); the cover shows in music_mini (musicPill "mini") and on the Home tab (music_expanded).', '- uninstall payload kind "app" is none of review/done/still/error: live-ext.js shows its fallback sub line "It did not work" (green check tone) - that is what the web renders for this payload.', '- Collapsed scenarios omit the (clipped, invisible) panel elements from rects / styles; expanded scenarios include tabs / header buttons.', '- Media seek position = payload position_ms + real elapsed time (performance.now is not frozen): music_expanded shows ~1:02, not 1:01.', '', 'MEASURED GEOMETRY (css px, viewport coords; full lists in <name>.rects.json; read from the files on disk):');
const rd = (n) => { try { return JSON.parse(readFileSync(join(outDir, `${n}.rects.json`), 'utf8')); } catch { return null; } };
const f = (r) => (r ? (Array.isArray(r) ? r : [r]).map((q) => `${q.x},${q.y} ${q.w}x${q.h}`).join(' | ') : '-');
for (const n of Object.keys(SCENARIOS)) {
  const r = rd(n);
  if (!r) continue;
  L.push(`${n}: body ${f(r['#body'])}  radius "${r.__meta.bodyBorderRadius}"`);
  const pick = ['.lp-time', '.lp-date', '.lp-ico', '.lp-aux', '.lp-cover', '.lp-eq', '.lp-name', '.rec-dot', '.rec-t', '.rec-stop', '.tc', '.tc .ti', '.tc .tt', '.tc .ts', '.lx-nt', '.lx-nb', '.tc .ab', '.tab', '.hbtn', '#pet', '.pet', '#chars', '#liq'];
  for (const s of pick) if (r[s]) L.push(`    ${s.padEnd(10)} ${f(r[s])}`);
}
writeFileSync(join(outDir, 'README.txt'), L.join('\n') + '\n');
console.log(`done: ${summary.length} scenario(s) -> ${outDir}`);
process.exit(0);
