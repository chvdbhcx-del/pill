// In-page rAF audit of the island's size transitions (expand, collapse, tab switches, event cards).
// For each scenario it samples the island body's rectangle every animation frame and reports:
//   under  = how far the size dipped BELOW the smaller of start/end size (px, the island shrinking under its content)
//   over   = how far it overshot the larger of start/end size
//   shift  = largest frame-to-frame move of the horizontal centre (the island jumping sideways)
//   settle = ms until the size is within 1 px of the final one
//
//   node tools/dev/audit-transitions.mjs
//   node tools/dev/audit-transitions.mjs --max-under-w 14 --max-under-h 14 --max-shift 3 --selector "#body"
//
// Exit code 1 if any scenario undershoots more than the allowed px (width / height) or jumps sideways more than --max-shift,
// 2 if the audit could not run. Run it with the app idle and collapsed; do not touch the mouse meanwhile (hover expands the island).
// The default limits are the measured values of the author's build (no undershoot beyond ~13 px); tune them for yours.
import { parseArgs } from 'node:util';
import { connect, DEFAULT_PORT } from './cdp.mjs';

/** Runs inside the page (serialized with toString): returns one result row per scenario. */
async function pageAudit(opts) {
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const body = document.querySelector(opts.selector);
  if (!body) throw new Error(`selector ${opts.selector} not found in this page`);
  const rect = () => { const r = body.getBoundingClientRect(); return { w: r.width, h: r.height, cx: r.x + r.width / 2 }; };

  async function scenario(name, run, dur) {
    const s0 = rect();
    const frames = [];
    let on = true;
    const t0 = performance.now();
    const loop = () => { frames.push({ t: performance.now() - t0, ...rect() }); if (on) requestAnimationFrame(loop); };
    requestAnimationFrame(loop);
    run();
    await sleep(dur);
    on = false;
    await sleep(40);
    const e = frames[frames.length - 1];
    const min = (k) => Math.min(...frames.map((f) => f[k]));
    const max = (k) => Math.max(...frames.map((f) => f[k]));
    let settle = 0;
    for (const f of frames) if (Math.abs(f.w - e.w) > 1 || Math.abs(f.h - e.h) > 1) settle = f.t;
    let shift = 0;
    for (let i = 1; i < frames.length; i++) shift = Math.max(shift, Math.abs(frames[i].cx - frames[i - 1].cx));
    return {
      name,
      from: `${Math.round(s0.w)}x${Math.round(s0.h)}`,
      to: `${Math.round(e.w)}x${Math.round(e.h)}`,
      underW: Math.max(0, Math.min(s0.w, e.w) - min('w')),
      underH: Math.max(0, Math.min(s0.h, e.h) - min('h')),
      overW: Math.max(0, max('w') - Math.max(s0.w, e.w)),
      overH: Math.max(0, max('h') - Math.max(s0.h, e.h)),
      shift,
      settleMs: Math.round(settle),
    };
  }

  const rows = [];
  Pill.collapse();               // start every run from the collapsed state
  await sleep(1500);
  rows.push(await scenario('expand home', () => Pill.expand('home'), 1400));
  rows.push(await scenario('collapse', () => Pill.collapse(), 1400));
  rows.push(await scenario('expand note', () => Pill.expand('note'), 1400));
  for (const [from, to] of [['note', 'home'], ['home', 'weather'], ['weather', 'chat'], ['chat', 'clipboard']]) {
    rows.push(await scenario(`tab ${from}->${to}`, () => Pill.expand(to), 1000));
  }
  rows.push(await scenario('collapse (click squash)', () => Pill.collapse('click'), 1400));
  rows.push(await scenario('card in', () => Pill.toast({ id: 'audit-card', icon: 'bell', title: 'Test', body: 'Audit of the card transition inside the island', ttlMs: 1800 }), 1400));
  rows.push(await scenario('card out', () => {}, 2600));
  return rows;
}

const { values } = parseArgs({
  options: {
    port: { type: 'string' },
    page: { type: 'string' },
    selector: { type: 'string' },
    'max-under-w': { type: 'string' },
    'max-under-h': { type: 'string' },
    'max-shift': { type: 'string' },
  },
});
const limits = {
  underW: Number(values['max-under-w'] ?? 14),
  underH: Number(values['max-under-h'] ?? 14),
  shift: Number(values['max-shift'] ?? 3),
};

try {
  const page = await connect({ match: values.page ?? 'pill.html', port: Number(values.port) || DEFAULT_PORT, timeoutMs: 90000 });
  const opts = { selector: values.selector ?? '#body' };
  const rows = await page.evaluate(`(${pageAudit.toString()})(${JSON.stringify(opts)})`, 90000);
  page.close();

  const cols = ['name', 'from', 'to', 'underW', 'underH', 'overW', 'overH', 'shift', 'settleMs'];
  const fmt = (v) => (typeof v === 'number' ? v.toFixed(Number.isInteger(v) ? 0 : 1) : String(v));
  const table = rows.map((r) => cols.map((c) => fmt(r[c])));
  const width = cols.map((c, i) => Math.max(c.length, ...table.map((t) => t[i].length)));
  const line = (cells) => cells.map((c, i) => (i === 0 ? c.padEnd(width[i]) : c.padStart(width[i]))).join('  ');
  console.log(line(cols));
  table.forEach((t) => console.log(line(t)));

  const bad = rows.flatMap((r) => [
    r.underW > limits.underW && `${r.name}: width dips ${fmt(r.underW)} px below its content (limit ${limits.underW})`,
    r.underH > limits.underH && `${r.name}: height dips ${fmt(r.underH)} px below its content (limit ${limits.underH})`,
    r.shift > limits.shift && `${r.name}: centre jumps ${fmt(r.shift)} px sideways in one frame (limit ${limits.shift})`,
  ]).filter(Boolean);
  if (bad.length) {
    console.log(`\nFAIL\n${bad.join('\n')}`);
    process.exitCode = 1;
  } else {
    console.log('\nOK: no undershoot or sideways jump beyond the limits');
  }
} catch (e) {
  console.error(e.message);
  process.exitCode = 2;
}
process.exit();
