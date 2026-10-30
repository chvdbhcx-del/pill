// Renders the WEB versions of the panel-set-B tabs (note + timer + reminders, text, chat, agents) and the overlays (AI quick actions, AI setup)
// in a private headless Chrome with a stubbed Tauri runtime, one PNG per state / theme / scale, for the comparison with the native views
// (tools/dev/tabs-b-diff.mjs). Needs Node 22+ and Chrome or Edge (CHROME=path to override). Nothing is written outside --out.
//
//   node tools/dev/tabs-b-web.mjs                              every state, dark + light, 1x + 2x -> target-agent/snap/web
//   node tools/dev/tabs-b-web.mjs --only note,chat --theme dark --scale 1
//   node tools/dev/tabs-b-web.mjs --states chat-streaming,text-qr --panel      (--panel also saves the whole island body as <state>-panel-...)
//   node tools/dev/tabs-b-web.mjs --fixtures ../island-wt27-chat/tools/dev/fixtures/tabs-b      (scenario files of another worktree)
//   node tools/dev/tabs-b-web.mjs --list
//
// Output names: <state>-<dark|light>-<1|2>x.png (the native side: kit::snap::shoot writes the same names). What a PNG shows:
//   views    the tab body #views (648 x 260 css px; 648 x 346 for note and chat, which are 417 tall panels)
//   card     the timer / reminders card of the note tab (.tn-card)
//   panel    ai-actions: the .clip-ai panel over the clipboard tab;  content: ai-setup: Settings' #content column (840 x 760 window)
// Scenario data: tools/dev/fixtures/tabs-b/_defaults.json, overridden per state by every other *.json in --fixtures:
//   {"now": epoch_ms, "scenarios": {"<state>": {"replies": {"cmd": value | {"__error": "text"} | {"__pending": true}}, "events": [{"name","payload","after_ms"}], "text": "..."}}}
import { parseArgs } from 'node:util';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { launch } from './headless.mjs';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../..');
const { values: o } = parseArgs({ options: {
  only: { type: 'string' }, states: { type: 'string' }, theme: { type: 'string', default: 'dark,light' }, scale: { type: 'string', default: '1,2' },
  out: { type: 'string' }, fixtures: { type: 'string' }, lang: { type: 'string', default: 'ru' }, panel: { type: 'boolean', default: false }, list: { type: 'boolean', default: false },
} });
const out = resolve(o.out || join(root, 'target-agent/snap/web'));
const fixDir = resolve(o.fixtures || join(here, 'fixtures/tabs-b'));
const themes = o.theme.split(','), scales = o.scale.split(',').map(Number);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---- what each state shows: which page, which tab, the UI driver, the element that is captured -------------------------------------------
const q = (s) => JSON.stringify(s);
const click = (sel, i = 0) => `(() => { const e = document.querySelectorAll(${q(sel)})[${i}]; if (!e) throw new Error('no ${sel}[${i}]'); e.click(); return 1; })()`;
const setText = (sel, v) => `(() => { const e = document.querySelector(${q(sel)}); if (!e) throw new Error('no ${sel}'); e.value = ${q(v)}; e.dispatchEvent(new Event('input', { bubbles: true })); return 1; })()`;
const P = (tab, shot, drive = async () => {}, extra = {}) => ({ page: 'pill', tab, shot, drive, ...extra });
const chatSend = (b, sc) => async () => { await b.evaluate(setText('.tab-chat textarea', sc.text || 'hi')); await b.evaluate(click('.ch-send')); await sleep(300); };
const STATES = (b, sc) => ({
  'note-formatted': P('note', 'views'),
  'note-empty': P('note', 'views'),
  'note-checklist': P('note', 'views'),
  'timer-idle': P('note', 'card'),
  'timer-running-ring': P('note', 'card'),
  'reminders-2': P('note', 'card'),
  'reminders-empty': P('note', 'card'),
  'custom-editing': P('note', 'card', async () => { await b.evaluate(click('.tn-ctime')); await sleep(250); }),
  'chat-empty': P('chat', 'views'),
  'chat-answer': P('chat', 'views'),
  'chat-streaming': P('chat', 'views', chatSend(b, sc)),
  'chat-typing-dots': P('chat', 'views', chatSend(b, sc)),
  'chat-history-popover': P('chat', 'views', async () => { await b.evaluate(click('.tab-chat .ch-tools .icon-btn', 0)); await sleep(500); }),
  'chat-settings-popover': P('chat', 'views', async () => { await b.evaluate(click('.tab-chat .ch-tools .icon-btn', 1)); await sleep(600); }),
  'chat-offline': P('chat', 'views'),
  'text-tools': P('text', 'views', async () => { await b.evaluate(setText('.tt-area', sc.text || '')); }),
  'text-stats': P('text', 'views', async () => { await b.evaluate(setText('.tt-area', sc.text || '')); await b.evaluate(click('.tt-grid .tt-tool', 18)); await sleep(200); }),
  'text-qr': P('text', 'views', async () => { await b.evaluate(setText('.tt-area', sc.text || '')); await b.evaluate(click('.tt-grid .tt-tool', 17)); await sleep(300); }),
  'text-ai-busy': P('text', 'views', async () => { await b.evaluate(setText('.tt-area', sc.text || '')); await b.evaluate(click('.tt-ai')); await sleep(150); await b.evaluate(click('.tt-aigrid .tt-tool', 0)); await sleep(300); }),
  'agents-list': P('agents', 'views'),
  'agents-empty': P('agents', 'views'),
  'ai-actions-menu': P('clipboard', 'sel:.clip-ai', async () => { await b.evaluate(aiOpen); await sleep(400); }),
  'ai-actions-busy': P('clipboard', 'sel:.clip-ai', async () => { await b.evaluate(aiOpen); await sleep(300); await b.evaluate(click('.clip-ai-opt', 0)); await sleep(500); }),
  'ai-actions-result': P('clipboard', 'sel:.clip-ai', async () => { await b.evaluate(aiOpen); await sleep(300); await b.evaluate(click('.clip-ai-opt', 0)); await sleep(700); }),
  'aisetup-fresh': { page: 'settings', shot: 'sel:#content', drive: async () => {} },
  'aisetup-ready': { page: 'settings', shot: 'sel:#content', drive: async () => {} },
  'aisetup-downloading': { page: 'settings', shot: 'sel:#content', drive: async () => { await b.evaluate(click('.as-scope button.pri', 0)); await sleep(300); } },
  'aisetup-error': { page: 'settings', shot: 'sel:#content', drive: async () => { await b.evaluate(click('.as-scope button.pri', 0)); await sleep(500); } },
});
const aiOpen = `(() => { const r = document.querySelector('#views .view.on'); if (!r) throw new Error('no active view'); window.PillClipAI.open({ root: r, id: 1, onClose() {} }); return 1; })()`;
const NAMES = Object.keys(STATES(null, {}));
const TABOF = (n) => STATES(null, {})[n];

if (o.list) { console.log(NAMES.join('\n')); process.exit(0); }

// ---- scenarios ----------------------------------------------------------------------------------------------------------------------------------
const defFile = existsSync(join(fixDir, '_defaults.json')) ? join(fixDir, '_defaults.json') : join(here, 'fixtures/tabs-b/_defaults.json');
const base = JSON.parse(readFileSync(defFile, 'utf8').replace(/^﻿/, ''));
const scenarios = JSON.parse(JSON.stringify(base.scenarios));
const overrides = {};
if (existsSync(fixDir)) {
  for (const f of readdirSync(fixDir).filter((x) => x.endsWith('.json') && x !== '_defaults.json').sort()) {
    const j = JSON.parse(readFileSync(join(fixDir, f), 'utf8').replace(/^﻿/, ''));
    for (const [k, v] of Object.entries(j.scenarios || {})) overrides[k] = { ...v, _now: j.now };
  }
}
function scenario(name) {
  let s = scenarios[name] || {};
  if (s.alias) s = { ...scenarios[s.alias], ...s };
  const ov = overrides[name] || {};
  return {
    now: ov._now || base.now,
    replies: { ...base.common.replies, ...(s.replies || {}), ...(ov.replies || {}) },
    events: ov.events || s.events || [],
    text: ov.text ?? s.text ?? '',
    storage: { ...(s.storage || {}), ...(ov.storage || {}) },
    fromFixture: !!overrides[name],
  };
}

// ---- run ---------------------------------------------------------------------------------------------------------------------------------------
const want = o.states ? o.states.split(',') : o.only ? NAMES.filter((n) => o.only.split(',').some((p) => n.startsWith(p) || TABOF(n).tab === p || (p === 'overlays' && /^(ai-|aisetup)/.test(n)))) : NAMES;
const unknown = want.filter((n) => !NAMES.includes(n));
if (unknown.length) { console.error('unknown state(s): ' + unknown.join(', ') + '\nknown: ' + NAMES.join(', ')); process.exit(2); }
mkdirSync(out, { recursive: true });
const stub = readFileSync(join(here, 'tabs-b-stub.js'), 'utf8');
const size = { pill: [720, 480], settings: [840, 760] };

const b = await launch({ width: 720, height: 480 });
const failed = [], written = [];
let scriptId = null;
try {
  await b.call('Page.enable'); await b.call('Runtime.enable');
  for (const name of want) for (const theme of themes) {
    const sc = scenario(name), st = STATES(b, sc)[name];
    try {
      const cfg = { now: sc.now, replies: JSON.parse(JSON.stringify(sc.replies)), storage: { 'pill.lang': o.lang, ...sc.storage } };
      cfg.replies.settings_get = { ...cfg.replies.settings_get, theme, language: o.lang };
      if (scriptId) await b.call('Page.removeScriptToEvaluateOnNewDocument', { identifier: scriptId }).catch(() => {});
      scriptId = (await b.call('Page.addScriptToEvaluateOnNewDocument', { source: `const __TB_CFG = ${JSON.stringify(cfg)};\n${stub}` })).identifier;
      const [w, h] = size[st.page];
      await b.call('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: 1, mobile: false });
      await b.call('Emulation.setTimezoneOverride', { timezoneId: base.tz || 'Europe/Moscow' }).catch(() => {});
      await b.call('Emulation.setLocaleOverride', { locale: o.lang === 'ru' ? 'ru-RU' : 'en-US' }).catch(() => {});
      const url = pathToFileURL(join(root, 'src', st.page === 'pill' ? 'pill.html' : 'settings.html')).href + (st.page === 'settings' ? '?page=aisetup' : '');
      await b.call('Page.navigate', { url });
      for (let i = 0; i < 100; i++) { if (await b.evaluate(st.page === 'pill' ? `!!(window.Pill && window.__TB && document.readyState === 'complete')` : `!!(window.__TB && window.PillAiSetupPage && document.readyState === 'complete')`).catch(() => false)) break; await sleep(100); }
      await sleep(400);
      if (st.page === 'pill') {
        await b.evaluate(`Pill.expand(${q(st.tab)})`);
        let ok = false;
        for (let i = 0; i < 60 && !ok; i++) { await sleep(100); ok = await b.evaluate(`Pill.state === 'expanded' && !!document.querySelector('#views .view.on')`); }
        if (!ok) throw new Error(`tab ${st.tab} did not open`);
        const cls = { note: '.tab-note', text: '.tab-text', chat: '.tab-chat', agents: '.ag-root' }[st.tab];
        if (cls && !(await b.evaluate(`(() => { const v = document.querySelector('#views .view.on'); return v.matches(${q(cls)}) || !!v.querySelector(${q(cls)}); })()`))) throw new Error(`wrong view is on (wanted ${cls})`);
        await sleep(700);
      } else await sleep(900);
      await st.drive();
      for (const ev of sc.events) { await sleep(ev.after_ms ?? 100); await b.evaluate(`__TB.emit(${q(ev.name)}, ${JSON.stringify(ev.payload ?? null)})`); }
      await sleep(500);
      const sel = st.shot === 'views' ? '#views' : st.shot === 'card' ? '.tab-note .tn-card' : st.shot.replace(/^sel:/, '');
      const shoot = async (selector, suffix) => {
        for (const s of scales) {
          await b.call('Emulation.setDeviceMetricsOverride', { width: w, height: h, deviceScaleFactor: s, mobile: false });
          await sleep(s === scales[0] ? 120 : 350);
          const r = await b.evaluate(`(() => { const e = document.querySelector(${q(selector)}); if (!e) return null; const r = e.getBoundingClientRect(); return { x: r.x, y: r.y, w: r.width, h: r.height }; })()`);
          if (!r || r.w < 2 || r.h < 2) throw new Error(`element ${selector} has no box`);
          const clip = { x: Math.floor(r.x), y: Math.floor(r.y), width: Math.ceil(r.w + (r.x - Math.floor(r.x))), height: Math.ceil(r.h + (r.y - Math.floor(r.y))), scale: 1 };
          const shot = await b.call('Page.captureScreenshot', { format: 'png', clip, captureBeyondViewport: false }, 20000);
          const file = join(out, `${name}${suffix}-${theme}-${s}x.png`);
          writeFileSync(file, Buffer.from(shot.data, 'base64'));
          written.push(`${name}${suffix}-${theme}-${s}x.png  ${clip.width}x${clip.height} css px`);
        }
      };
      await shoot(sel, '');
      if (o.panel && st.page === 'pill') await shoot('#body', '-panel');
    } catch (e) {
      failed.push(`${name}/${theme}: ${e.message}`);
      console.error('FAILED ' + name + '/' + theme + ': ' + e.message);
    }
  }
} finally { await b.close(); }
console.log(written.join('\n'));
console.log(`\n${written.length} PNG(s) -> ${out}${failed.length ? `\n${failed.length} state(s) could not be driven:\n  ` + failed.join('\n  ') : ''}`);
process.exit(failed.length ? 1 : 0);
