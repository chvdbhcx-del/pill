// JS CPU profile of a Pill page over the WebView2 debug port: busy share of one core and the top functions by self time.
//
//   node tools/dev/prof.mjs pill.html                 profile 4 s
//   node tools/dev/prof.mjs pill.html --ms 10000 --top 20 --port 9333
//
// Leave the app in the state you want to measure (idle, a tab open, music playing) while it runs.
import { parseArgs } from 'node:util';
import { connect, DEFAULT_PORT } from './cdp.mjs';

const { values, positionals } = parseArgs({
  allowPositionals: true,
  options: { ms: { type: 'string' }, top: { type: 'string' }, port: { type: 'string' } },
});
const match = positionals[0];
if (!match) { console.error('usage: node prof.mjs <url-substring> [--ms 4000] [--top 14] [--port N]'); process.exit(2); }
const ms = Number(values.ms) || 4000;
const top = Number(values.top) || 14;

try {
  const page = await connect({ match, port: Number(values.port) || DEFAULT_PORT, timeoutMs: ms + 10000 });
  await page.call('Profiler.enable');
  await page.call('Profiler.setSamplingInterval', { interval: 200 });
  await page.call('Profiler.start');
  await new Promise((r) => setTimeout(r, ms));
  const { profile } = await page.call('Profiler.stop');
  page.close();

  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const self = new Map();
  let total = 0;
  profile.samples.forEach((s, i) => {
    const cf = byId.get(s).callFrame;
    const key = `${cf.functionName || '(anon)'} ${cf.url.split('/').pop()}:${cf.lineNumber + 1}`;
    self.set(key, (self.get(key) || 0) + profile.timeDeltas[i]);
    total += profile.timeDeltas[i];
  });
  const idle = [...self].filter(([k]) => k.startsWith('(idle)')).reduce((a, [, v]) => a + v, 0);
  console.log(`window ${ms} ms, busy ${(((total - idle) / 1000) / ms * 100).toFixed(1)}% of one core`);
  [...self]
    .filter(([k]) => !/^\((idle|program|garbage)/.test(k))
    .sort((a, b) => b[1] - a[1])
    .slice(0, top)
    .forEach(([k, v]) => console.log(`${(v / 1000).toFixed(0).padStart(6)} ms  ${k}`));
} catch (e) {
  console.error(e.message);
  process.exitCode = 1;
}
process.exit();
