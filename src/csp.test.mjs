// CSP check: tauri.conf.json carries a strict Content-Security-Policy, and the front end stays compatible with it:
// no inline <script>, no inline event-handler attributes, no eval / new Function, no network calls from the pages (everything goes through Rust).
// Run: node src/csp.test.mjs
import fs from 'node:fs';

const conf = JSON.parse(fs.readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const csp = conf.app?.security?.csp;
const fail = [];
if (typeof csp !== 'string' || !csp) fail.push('app.security.csp must be a policy string, not null');
else {
  const dir = (n) => csp.split(';').map((d) => d.trim()).find((d) => d.startsWith(n + ' '));
  if (!/default-src 'self'/.test(csp)) fail.push("default-src must be 'self'");
  const script = dir('script-src') || '';
  if (!script || /unsafe-inline|unsafe-eval|\*|https?:/.test(script)) fail.push("script-src must be 'self' only (no unsafe-inline, no unsafe-eval, no hosts)");
  const connect = dir('connect-src') || '';
  if (!/ipc:/.test(connect) || /\*|https?:\/\/(?!ipc\.localhost)/.test(connect)) fail.push('connect-src must allow ipc: / http://ipc.localhost and nothing else');
  for (const d of ['object-src', 'frame-src', 'base-uri']) if (!new RegExp(d + " 'none'").test(csp)) fail.push(d + " must be 'none'");
}

const dirUrl = new URL('./', import.meta.url);
for (const f of fs.readdirSync(dirUrl).filter((n) => /.(html|js)$/.test(n))) {
  const src = fs.readFileSync(new URL(f, dirUrl), 'utf8');
  if (f.endsWith('.html')) {
    if (/<script(?![^>]*\bsrc=)[^>]*>/i.test(src)) fail.push(f + ': inline <script>');
    if (/\son[a-z]+\s*=/i.test(src)) fail.push(f + ': inline event handler attribute');
    if (/<style[\s>]/i.test(src)) fail.push(f + ': inline <style> element');
  }
  if (f.endsWith('.js')) {
    const code = src.replace(/\/\/.*$/gm, '');
    if (/\beval\s*\(|new Function\s*\(/.test(code)) fail.push(f + ': eval / new Function');
    if (/\bfetch\s*\(|new XMLHttpRequest|new WebSocket\s*\(|new EventSource\s*\(|navigator\.sendBeacon/.test(code)) fail.push(f + ': a network call from the page (go through a Rust command)');
    if (/\.setAttribute\(\s*['"]on[a-z]+['"]/.test(code) || /\son(click|load|error|mouse[a-z]*|key[a-z]*)\s*=\s*\\?["']/.test(code)) fail.push(f + ': inline event handler');
    if (/javascript:/.test(code)) fail.push(f + ': javascript: URL');
  }
}
if (fail.length) {
  console.error('CSP check failed:\n  ' + fail.join('\n  '));
  process.exit(1);
}
console.log('csp check: ok');
