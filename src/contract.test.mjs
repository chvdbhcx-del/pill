// Contract check: every command the front-end invokes must be registered in generate_handler! (src-tauri/src/main.rs).
// Run: node src/contract.test.mjs  (docs/CONTRACT.md lists the commands and events)
import fs from 'node:fs';

const main = fs.readFileSync(new URL('../src-tauri/src/main.rs', import.meta.url), 'utf8');
const block = main.match(/generate_handler!\[([\s\S]*?)\]\)/);
if (!block) throw new Error('generate_handler! not found in main.rs');
const registered = new Set([...block[1].replace(/\/\/.*$/gm, '').matchAll(/\b[a-z_0-9]+::([a-z_0-9]+)\b/g)].map((m) => m[1]));
if (registered.size < 50) throw new Error('too few commands parsed from generate_handler!: ' + registered.size);

const dir = new URL('./', import.meta.url);
const missing = [];
let calls = 0;
for (const f of fs.readdirSync(dir).filter((n) => n.endsWith('.js'))) {
  const src = fs.readFileSync(new URL(f, dir), 'utf8');
  for (const m of src.matchAll(/\b(?:invoke|inv)\(\s*['"`]([A-Za-z_0-9]+)['"`]/g)) {
    calls++;
    if (!registered.has(m[1])) missing.push(`${f}: ${m[1]}`);
  }
}
if (missing.length) {
  console.error('commands invoked but not registered in generate_handler!:\n  ' + missing.join('\n  '));
  process.exit(1);
}
console.log(`contract check: ok (${calls} invoke calls, ${registered.size} registered commands)`);
