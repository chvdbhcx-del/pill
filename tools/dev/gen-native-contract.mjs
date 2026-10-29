// Regenerates docs/native-ui-contract.md from the sources: node tools/dev/gen-native-contract.mjs [repo root]
import fs from 'node:fs';
import { fileURLToPath } from 'node:url';
const root = process.argv[2] || fileURLToPath(new URL('../../', import.meta.url)).replace(/[\/]$/, '');
const R = (p) => fs.readFileSync(root + '/' + p, 'utf8');
const rsFiles = fs.readdirSync(root + '/src-tauri/src').filter((f) => f.endsWith('.rs'));
const jsFiles = fs.readdirSync(root + '/src').filter((f) => f.endsWith('.js') && !/test/.test(f));
const js = Object.fromEntries(jsFiles.map((f) => [f, R('src/' + f)]));
const uses = (lit) => jsFiles.filter((f) => js[f].includes(`'${lit}'`) || js[f].includes(`"${lit}"`) || js[f].includes('`' + lit + '`'));
const BS = String.fromCharCode(92);
const esc = (s) => s.replace(/[|]/g, BS + '|');

// ---- events ----
const ev = {};
const add = (n, f, l, p, t) => ((ev[n] ||= []).push({ f, l, p, t }));
// the argument list of the call whose '(' is at index i (balanced, strings and comments aware)
function args(src, i) {
  const out = []; let depth = 0, cur = '', k = i;
  for (; k < src.length; k++) {
    const ch = src[k];
    if (ch === '"') { let j = k + 1; while (src[j] !== '"') j += src[j] === BS ? 2 : 1; cur += src.slice(k, j + 1); k = j; continue; }
    if (ch === '/' && src[k + 1] === '/') { while (src[k] !== String.fromCharCode(10)) k++; continue; }
    if ('([{'.includes(ch)) { if (depth++ === 0) continue; }
    else if (')]}'.includes(ch)) { if (--depth === 0) { out.push(cur.trim()); break; } }
    else if (ch === ',' && depth === 1) { out.push(cur.trim()); cur = ''; continue; }
    cur += ch;
  }
  return out.map((x) => x.replace(new RegExp(BS + 's+', 'g'), ' '));
}
for (const f of rsFiles) {
  const src = R('src-tauri/src/' + f);
  const lineOf = (idx) => src.slice(0, idx).split(String.fromCharCode(10)).length;
  for (const m of src.matchAll(new RegExp('bus::emit' + BS + '(', 'g'))) {
    const a = args(src, m.index + m[0].length - 1);
    const nm = a[2];
    if (!/^"[a-z]+:[^"]+"$/.test(nm || '')) continue;
    add(nm.slice(1, -1), f, lineOf(m.index), a[3] || '()', a[1] === 'Target::All' ? 'all windows' : a[1].replace(/^Target::Label[(](.*)[)]$/, '$1'));
  }
  for (const m of src.matchAll(new RegExp('(?<![' + BS + 'w.:])(?<!fn )emit' + BS + '(', 'g'))) {
    const a = args(src, m.index + m[0].length - 1);
    const i = a.findIndex((x) => /^"[a-z]+:[^"]+"$/.test(x));
    if (i < 0) continue;
    add(a[i].slice(1, -1), f, lineOf(m.index), a[i + 1] || '()', 'pill');
  }
}
const tgt = (t) => (t === '"pill"' ? 'pill' : t === '"bar"' ? 'bar' : t === 'LABEL' ? 'bar / settings (own window)' : t === 'm.label.as_str()' || t === 'label.as_str()' ? 'capture-N overlay' : t);
let out = '';
out += `# Native UI contract (derived by grep, Phase 0)\n\n`;
out += `Everything the Rust backend and the WebView front end say to each other today; the native UI has to implement the same names and shapes. Generated mechanically from the sources (event producers: \`bus::emit\` call sites and the small local \`emit\` helpers; consumers: string literals in \`src/*.js\`; commands: \`generate_handler!\` in \`main.rs\` plus the \`#[tauri::command]\` signatures). Regenerate rather than hand-edit.\n\n`;
out += `- **Events** (backend to UI) go through \`ui::bus::emit(app, Target, name, payload)\` (\`src-tauri/src/ui/bus.rs\`); native code can \`bus::subscribe(name, f)\` to the same stream and gets the payload as JSON. "Payload as emitted" is the Rust expression at the producer; JSON object keys are the field names.\n`;
out += `- **Commands** (UI to backend) are \`#[tauri::command]\` functions registered in \`generate_handler!\` (\`src-tauri/src/main.rs\`). On the JS side argument names are camelCase (Tauri converts); \`AppHandle\` / window arguments are injected and not listed. Errors: a \`Result\` command rejects the promise with the \`Err\` text; the pages use wrappers that turn that into \`null\`.\n`;
out += `- Window labels: \`pill\` (always alive), \`bar\` (created on demand, \`ui/lazy.rs\`), \`settings\` (on demand, \`settingswin.rs\`), \`capture-N\` (one overlay per monitor, \`capture.rs\`). The first two are built by \`ui/windows.rs\`.\n\n`;
const names = Object.keys(ev).sort();
out += `## Events (${names.length})\n\n| Event | Target | Payload as emitted | Producer | JS consumer |\n|---|---|---|---|---|\n`;
for (const n of names) {
  const es = ev[n];
  const t = [...new Set(es.map((e) => tgt(e.t)))].join(', ');
  const p = [...new Set(es.map((e) => e.p))].map((x) => '`' + esc(x).slice(0, 150) + '`').join('<br>');
  const pr = [...new Set(es.map((e) => `${e.f}:${e.l}`))].join(', ');
  const c = uses(n).join(', ') || '-';
  out += `| \`${n}\` | ${t} | ${p} | ${pr} | ${c}${n === 'settings:changed' ? ' (+ Rust listeners: notif.rs, obs.rs, steam.rs, voice.rs)' : ''} |\n`;
}
out += `\nBuilt-in Tauri events the pages also use: \`tauri://drag-enter\`, \`tauri://drag-over\`, \`tauri://drag-drop\`, \`tauri://drag-leave\` (tab-home.js, tab-shelf.js: files dropped on the pill). The bar page also registers and immediately drops a listener named \`bar:ready-barrier\` (bar-ready.js; never emitted) and calls \`cmd_ready\` once its listeners are in place.\n\n`;

// ---- commands ----
const main = R('src-tauri/src/main.rs');
const block = main.match(/generate_handler!\[([\s\S]*?)\]\)/)[1].replace(/\/\/.*$/gm, '');
const cmds = [...block.matchAll(/\b([a-z_0-9]+)::([a-z_0-9]+)\b/g)].map((m) => ({ mod: m[1], name: m[2] }));
const sigs = {};
for (const c of cmds) {
  const src = R(`src-tauri/src/${c.mod}.rs`);
  const re = new RegExp(String.raw`fn ${c.name}\s*\(([\s\S]*?)\)\s*(?:->\s*([^{]+?))?\s*\{`);
  const m = src.match(re);
  const line = m ? src.slice(0, src.indexOf(m[0])).split('\n').length : 0;
  let args = m ? m[1].replace(/\s+/g, ' ').trim() : '?';
  const parts = [];
  let depth = 0, cur = '';
  for (const ch of args) {
    if ('<([{'.includes(ch)) depth++;
    if ('>)]}'.includes(ch)) depth--;
    if (ch === ',' && depth === 0) { parts.push(cur); cur = ''; } else cur += ch;
  }
  parts.push(cur);
  args = parts.map((a) => a.trim()).filter((a) => a && !/:\s*(tauri::)?(AppHandle|Window|WebviewWindow|Webview)$/.test(a) && !/State</.test(a)).join(', ');
  sigs[c.name] = { mod: c.mod, line, args: args || '-', ret: m && m[2] ? m[2].trim().replace(/\s+/g, ' ') : '()', bad: !m };
}
out += `## Commands (${cmds.length})\n\n| Command | Args | Returns | Producer | JS callers |\n|---|---|---|---|---|\n`;
for (const c of cmds) {
  const s = sigs[c.name];
  const callers = jsFiles.filter((f) => new RegExp(String.raw`['"` + '`' + String.raw`]${c.name}['"` + '`' + String.raw`]`).test(js[f])).join(', ') || '-';
  out += `| \`${c.name}\` | ${s.args === '-' ? '-' : '`' + esc(s.args) + '`'} | \`${esc(s.ret)}\` | ${c.mod}.rs:${s.line} | ${callers} |\n`;
}
const reg = new Set(cmds.map((c) => c.name));
const called = new Set();
for (const f of jsFiles) for (const m of js[f].matchAll(/\b(?:invoke|inv|call)\(\s*['"`]([A-Za-z_0-9]+)['"`]/g)) called.add(m[1]);
const missing = [...called].filter((x) => !reg.has(x));
out += `\nInvoked by the pages but not registered in \`generate_handler!\`: ${missing.length ? missing.map((x) => '\`' + x + '\`').join(', ') : 'none'} (the same check runs in \`node src/contract.test.mjs\`).\n`;
fs.writeFileSync(root + '/docs/native-ui-contract.md', out);
console.log('events', names.length, 'commands', cmds.length, 'unresolved sigs', Object.entries(sigs).filter(([, s]) => s.bad).map(([k]) => k).join(','));
