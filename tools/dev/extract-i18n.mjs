#!/usr/bin/env node
// extract-i18n.mjs - harvest every ru/en string pair of the web UI into a static Rust table.
//
// Why: the web UI spells its translations inline, `t({ ru: 'Открыть', en: 'Open' })` (pill.js `t`, i18n-lite.js `window.t`, `P.t`),
// about 800+ call sites in src/*.js. The native Rust UI keeps the same inline style (`tr.t("Открыть", "Open")`), and this script
// produces the catalogue that proves nothing was forgotten: src-tauri/ui-core/src/i18n/table.rs.
//
// What counts as a pair (found on a real token stream, so strings, comments, regexes and CSS-in-templates never fool it):
//   1. object literal   { ru: <expr>, en: <expr> }   (either order, quoted or bare keys) - covers t({..}), P.t([..{..}..]), name: {..}
//   2. positional helper  L('ru', 'en')               - only for helpers the file DEFINES as `(a, b) => (ru ? a : b)` or `(ru, en) => t({ ru, en })`
//   3. language ternary  isRu() ? 'ru' : 'en'  /  lang === 'ru' ? .. : ..  /  P.lang === 'ru' ? .. : ..
// A side is an `<expr>` made of string / template literals joined by `+`; every non-literal part (a variable, a call, `${n}`)
// becomes a numbered placeholder {0}, {1}, ... (same expression text = same number on both sides). Sides that are not text
// (a bare variable, a ternary / plural expression, locale tags like 'ru-RU') are counted as *dynamic* and listed by --report.
//
// Ids: FNV-1a 64 of  en + "\0" + ru  (UTF-8). Content-addressed on purpose: regenerating after unrelated edits, reordering the
// sources or adding other strings never changes an existing id (sequential ids would shift on every insert). Two different
// pairs colliding is a hard error here. The table is sorted by id (binary search in i18n::lookup).
//
// Usage:  node tools/dev/extract-i18n.mjs            write table.rs
//         node tools/dev/extract-i18n.mjs --check    exit 1 when table.rs is out of date (CI)
//         node tools/dev/extract-i18n.mjs --report   per-file counts + the dynamic / unpaired sites
//         node tools/dev/extract-i18n.mjs --json     machine-readable stats on stdout (used by extract-i18n.test.mjs)
//         --src <dir> --out <file>                    override the input folder / output file
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');

// ------------------------------------------------------------------ lexer
const KEYWORDS_BEFORE_REGEX = new Set(['return', 'typeof', 'case', 'in', 'of', 'delete', 'void', 'throw', 'new', 'else', 'do', 'yield', 'await', 'instanceof']);
const OPS = ['>>>=', '...', '===', '!==', '**=', '<<=', '>>=', '>>>', '&&=', '||=', '??=', '=>', '==', '!=', '<=', '>=', '&&', '||', '??', '?.', '++', '--', '+=', '-=', '*=', '/=', '%=', '&=', '|=', '^=', '<<', '>>', '**'];
const isIdStart = (c) => /[A-Za-z_$\u0080-￿]/.test(c);
const isIdPart = (c) => /[A-Za-z0-9_$\u0080-￿]/.test(c);

function cookEscape(s, i, out) { // s[i] === '\\'; returns the new index
  const c = s[i + 1];
  switch (c) {
    case 'n': out.push('\n'); return i + 2;
    case 't': out.push('\t'); return i + 2;
    case 'r': out.push('\r'); return i + 2;
    case 'b': out.push('\b'); return i + 2;
    case 'f': out.push('\f'); return i + 2;
    case 'v': out.push('\v'); return i + 2;
    case '0': if (!/[0-9]/.test(s[i + 2] || '')) { out.push('\0'); return i + 2; } out.push('0'); return i + 2;
    case 'x': { const h = s.slice(i + 2, i + 4); if (/^[0-9a-fA-F]{2}$/.test(h)) { out.push(String.fromCharCode(parseInt(h, 16))); return i + 4; } out.push('x'); return i + 2; }
    case 'u': {
      if (s[i + 2] === '{') { const e = s.indexOf('}', i + 3); out.push(String.fromCodePoint(parseInt(s.slice(i + 3, e), 16))); return e + 1; }
      const h = s.slice(i + 2, i + 6); if (/^[0-9a-fA-F]{4}$/.test(h)) { out.push(String.fromCharCode(parseInt(h, 16))); return i + 6; }
      out.push('u'); return i + 2;
    }
    case '\r': return s[i + 2] === '\n' ? i + 3 : i + 2; // line continuation
    case '\n': return i + 2;
    default: out.push(c ?? ''); return i + 2;
  }
}

// Returns a token array. Token: {t: 'id'|'str'|'tpl'|'num'|'re'|'p', v, line, s, e}; 'tpl' also has parts: [{text}|{expr, tokens}].
function lex(src, from = 0, until = null, line0 = 1) {
  const toks = [];
  let i = from, line = line0, depth = 0;
  const prevSig = () => toks[toks.length - 1];
  const lineAt = (a, b) => { let n = 0; for (let k = a; k < b; k++) if (src[k] === '\n') n++; return n; };
  while (i < src.length) {
    const c = src[i];
    if (c === '\n') { line++; i++; continue; }
    if (c === ' ' || c === '\t' || c === '\r' || c === '﻿') { i++; continue; }
    if (c === '/' && src[i + 1] === '/') { while (i < src.length && src[i] !== '\n') i++; continue; }
    if (c === '/' && src[i + 1] === '*') { const e = src.indexOf('*/', i + 2); const end = e < 0 ? src.length : e + 2; line += lineAt(i, end); i = end; continue; }
    if (c === '#' && src[i + 1] === '!' && i === 0) { while (i < src.length && src[i] !== '\n') i++; continue; }
    if (c === '"' || c === "'") {
      const out = []; const s0 = i, l0 = line; i++;
      while (i < src.length && src[i] !== c) {
        if (src[i] === '\\') { if (src[i + 1] === '\n') line++; i = cookEscape(src, i, out); } else { if (src[i] === '\n') break; out.push(src[i++]); }
      }
      i++;
      toks.push({ t: 'str', v: out.join(''), line: l0, s: s0, e: i });
      continue;
    }
    if (c === '`') {
      const s0 = i, l0 = line; i++;
      const parts = []; let out = [];
      while (i < src.length && src[i] !== '`') {
        if (src[i] === '\\') { if (src[i + 1] === '\n') line++; i = cookEscape(src, i, out); continue; }
        if (src[i] === '$' && src[i + 1] === '{') {
          if (out.length) { parts.push({ text: out.join('') }); out = []; }
          const es = i + 2;
          const r = lex(src, es, '}', line);
          parts.push({ expr: src.slice(es, r.end).trim().replace(/\s+/g, ' '), tokens: r.toks });
          line = r.line; i = r.end + 1;
          continue;
        }
        if (src[i] === '\n') line++;
        out.push(src[i++]);
      }
      if (out.length) parts.push({ text: out.join('') });
      i++;
      toks.push({ t: 'tpl', parts, line: l0, s: s0, e: i });
      continue;
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1] || ''))) {
      const s0 = i; i++;
      while (i < src.length && /[0-9A-Za-z_.]/.test(src[i])) { if ((src[i] === 'e' || src[i] === 'E') && /[+-]/.test(src[i + 1] || '')) i++; i++; }
      toks.push({ t: 'num', v: src.slice(s0, i), line, s: s0, e: i });
      continue;
    }
    if (isIdStart(c)) {
      const s0 = i; i++;
      while (i < src.length && isIdPart(src[i])) i++;
      toks.push({ t: 'id', v: src.slice(s0, i), line, s: s0, e: i });
      continue;
    }
    if (c === '/') {
      const p = prevSig();
      const division = p && (p.t === 'num' || p.t === 'str' || p.t === 'tpl' || p.t === 're' || (p.t === 'id' && !KEYWORDS_BEFORE_REGEX.has(p.v)) || (p.t === 'p' && (p.v === ')' || p.v === ']')));
      if (!division) {
        const s0 = i; i++; let cls = false;
        while (i < src.length && src[i] !== '\n') {
          if (src[i] === '\\') { i += 2; continue; }
          if (src[i] === '[') cls = true; else if (src[i] === ']') cls = false; else if (src[i] === '/' && !cls) break;
          i++;
        }
        i++; while (i < src.length && /[a-z]/.test(src[i])) i++;
        toks.push({ t: 're', v: src.slice(s0, i), line, s: s0, e: i });
        continue;
      }
    }
    // punctuators
    let op = null;
    for (const o of OPS) if (src.startsWith(o, i)) { op = o; break; }
    if (op === '?.' && /[0-9]/.test(src[i + 2] || '')) op = null; // `a ?.5 : b` is a ternary
    if (!op) op = c;
    if (until === '}' ) {
      if (op === '{') depth++;
      else if (op === '}') { if (depth === 0) return { toks, end: i, line }; depth--; }
    }
    toks.push({ t: 'p', v: op, line, s: i, e: i + op.length });
    i += op.length;
  }
  return { toks, end: i, line };
}

// ------------------------------------------------------------------ value parsing
const OPEN = new Set(['(', '[', '{']), CLOSE = new Set([')', ']', '}']);
const isP = (t, v) => t && t.t === 'p' && t.v === v;

// Reads an expression from toks[i] up to a top-level terminator. Returns {end (index of terminator), elems, dynamic}.
// elems: [{lit: string-with-placeholders-parts} | {expr: source-ish}]; parts of a literal: [{text}|{expr}]
function readExpr(toks, i, stops, src) {
  const start = i; let d = 0;
  while (i < toks.length) {
    const t = toks[i];
    if (t.t === 'p') {
      if (OPEN.has(t.v)) d++;
      else if (CLOSE.has(t.v)) { if (d === 0) break; d--; }
      else if (d === 0 && stops.includes(t.v)) break;
    }
    i++;
  }
  return { start, end: i, toks: toks.slice(start, i) };
}

// Turn the tokens of one side into [{text}|{expr}] parts, or null when it is not text (dynamic).
function sideParts(tk, src) {
  if (!tk.length) return null;
  // split by top-level '+'
  const elems = []; let cur = [], d = 0;
  for (const t of tk) {
    if (t.t === 'p') {
      if (OPEN.has(t.v)) d++; else if (CLOSE.has(t.v)) d--;
      else if (d === 0 && (t.v === '?' || t.v === '||' || t.v === '&&' || t.v === '??' || t.v === ':')) return null; // conditional value: not static text
      else if (d === 0 && t.v === '+') { elems.push(cur); cur = []; continue; }
    }
    cur.push(t);
  }
  elems.push(cur);
  const parts = []; let literal = false;
  for (const el of elems) {
    if (!el.length) return null;
    // a parenthesised pure literal chain: ('a' + 'b')
    if (el.length === 1 && el[0].t === 'str') { parts.push({ text: el[0].v }); if (el[0].v.trim()) literal = true; continue; }
    if (el.length === 1 && el[0].t === 'tpl') {
      for (const p of el[0].parts) {
        if (p.text != null) { parts.push({ text: p.text }); if (p.text.trim()) literal = true; continue; }
        if (p.tokens.some((x) => x.t === 'p' && (x.v === '?' || x.v === '||' || x.v === '&&' || x.v === '??') ) && p.tokens.some((x) => x.t === 'str' || x.t === 'tpl')) return null; // plural / conditional text inside
        parts.push({ expr: p.expr });
      }
      continue;
    }
    // anything else = an expression placeholder; keep its source text for the numbering
    const a = el[0].s, b = el[el.length - 1].e;
    parts.push({ expr: src.slice(a, b).trim().replace(/\s+/g, ' ') });
  }
  return literal ? parts : null;
}

// Render parts of the two sides of a pair into strings with shared {n} numbering.
function renderPair(ruParts, enParts) {
  const idx = new Map();
  const num = (e) => { if (!idx.has(e)) idx.set(e, idx.size); return idx.get(e); };
  const r = (ps) => ps.map((p) => (p.text != null ? p.text : `{${num(p.expr)}}`)).join('');
  const ru = r(ruParts), en = r(enParts);
  return { ru, en, args: idx.size };
}


// Both sides are the same chain of conditions, `c1 ? 'a' : c2 ? 'b' : 'c'` (weather: L(light ? 'Небольшой снег' : 'Снег', light ? 'Light snow' : 'Snow')):
// zip the leaves into ordinary pairs. Returns [{rp, ep}] or null when the shapes differ / a leaf is not text.
function stripParens(tk) {
  while (tk.length > 2 && isP(tk[0], '(') && isP(tk[tk.length - 1], ')')) {
    let d = 0, whole = true;
    for (let k = 0; k < tk.length; k++) { if (OPEN.has(tk[k].v) && tk[k].t === 'p') d++; else if (CLOSE.has(tk[k].v) && tk[k].t === 'p') { d--; if (d === 0 && k < tk.length - 1) { whole = false; break; } } }
    if (!whole) break;
    tk = tk.slice(1, -1);
  }
  return tk;
}
function chain(tk, src) { // -> {sig: string, leaves: tokens[][]} | null
  tk = stripParens(tk);
  let d = 0, q = -1;
  for (let k = 0; k < tk.length; k++) {
    const t = tk[k]; if (t.t !== 'p') continue;
    if (OPEN.has(t.v)) d++; else if (CLOSE.has(t.v)) d--;
    else if (d === 0 && t.v === '?') { q = k; break; }
  }
  if (q < 0) return { sig: '', leaves: [tk] };
  let nest = 0, c = -1; d = 0;
  for (let k = q + 1; k < tk.length; k++) {
    const t = tk[k]; if (t.t !== 'p') continue;
    if (OPEN.has(t.v)) d++; else if (CLOSE.has(t.v)) d--;
    else if (d === 0 && t.v === '?') nest++;
    else if (d === 0 && t.v === ':') { if (nest === 0) { c = k; break; } nest--; }
  }
  if (c < 0) return null;
  const cond = src.slice(tk[0].s, tk[q - 1].e).replace(/\s+/g, ' ');
  const a = chain(tk.slice(q + 1, c), src), b = chain(tk.slice(c + 1), src);
  if (!a || !b) return null;
  return { sig: `(${cond}?${a.sig}:${b.sig})`, leaves: [...a.leaves, ...b.leaves] };
}
function pairsOf(ruTk, enTk, src) {
  const rp = sideParts(ruTk, src), ep = sideParts(enTk, src);
  if (rp && ep) return [{ rp, ep }];
  const a = chain(ruTk, src), b = chain(enTk, src);
  if (!a || !b || !a.sig || a.sig !== b.sig || a.leaves.length !== b.leaves.length) return null;
  const res = [];
  for (let k = 0; k < a.leaves.length; k++) {
    const x = sideParts(a.leaves[k], src), y = sideParts(b.leaves[k], src);
    if (!x || !y) return null;
    res.push({ rp: x, ep: y });
  }
  return res;
}

// ------------------------------------------------------------------ pair finders
// Every token list of a file: the top level plus the `${ ... }` expressions of its template literals (recursively),
// because most pairs live inside html templates: `${esc(t({ ru: 'Закрыть', en: 'Close' }))}`.
function tokenLists(toks, acc = []) {
  acc.push(toks);
  for (const t of toks) if (t.t === 'tpl') for (const p of t.parts) if (p.tokens) tokenLists(p.tokens, acc);
  return acc;
}

function findPairs(topToks, src, file) {
  const out = [];     // {ru, en, args, line, kind}
  const dyn = [];     // {line, kind, why}
  const consumed = new WeakSet();
  const keyOf = (t) => (t && ((t.t === 'id') || (t.t === 'str')) ? (t.t === 'id' ? t.v : t.v) : null);

  // positional helpers defined in this file
  const helpers = new Set();
  const hdef = /(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*\(\s*([A-Za-z_$][\w$]*)\s*,\s*([A-Za-z_$][\w$]*)\s*\)\s*=>\s*(?:\(\s*ru\s*\?\s*\2\s*:\s*\3\s*\)|(?:[A-Za-z_$.]*\bt|window\.t)\(\s*\{\s*ru(?:\s*:\s*\2)?\s*,\s*en(?:\s*:\s*\3)?\s*\}\s*\))/g;
  for (const m of src.matchAll(hdef)) helpers.add(m[1]);

  for (const toks of tokenLists(topToks)) for (let i = 0; i < toks.length; i++) {
    const t = toks[i];
    // ---- 1. object literal { ru: .., en: .. }
    if ((t.t === 'id' || t.t === 'str') && (t.v === 'ru' || t.v === 'en') && isP(toks[i + 1], ':') && !consumed.has(t)) {
      const pv = toks[i - 1];
      if (pv && (isP(pv, '{') || isP(pv, ','))) {
        const a = readExpr(toks, i + 2, [','], src);
        let j = a.end; // terminator
        let pairIdx = -1;
        // sibling member right after: `, en: ..`
        if (isP(toks[j], ',') && keyOf(toks[j + 1]) && isP(toks[j + 2], ':') && keyOf(toks[j + 1]) === (t.v === 'ru' ? 'en' : 'ru') && (toks[j + 1].t === 'id' || toks[j + 1].t === 'str')) pairIdx = j + 1;
        if (pairIdx < 0) { dyn.push({ line: t.line, kind: 'obj', why: 'unpaired ' + t.v }); i = a.end; continue; }
        const b = readExpr(toks, pairIdx + 2, [','], src);
        consumed.add(toks[pairIdx]);
        const ruTk = t.v === 'ru' ? a.toks : b.toks, enTk = t.v === 'ru' ? b.toks : a.toks;
        const ps = pairsOf(ruTk, enTk, src);
        if (!ps) dyn.push({ line: t.line, kind: 'obj', why: 'non-literal ' + (!sideParts(ruTk, src) ? 'ru' : 'en') });
        else for (const { rp, ep } of ps) out.push({ ...renderPair(rp, ep), line: t.line, kind: 'obj' });
        i = a.start; // re-scan the values (a nested pair is possible in theory); progress is guaranteed
        continue;
      }
    }
    // ---- 2. positional helper call L('ru', 'en')
    if (t.t === 'id' && helpers.has(t.v) && isP(toks[i + 1], '(') && !isP(toks[i - 1], '.') && !(toks[i - 1] && toks[i - 1].t === 'id' && toks[i - 1].v === 'function')) {
      const a = readExpr(toks, i + 2, [','], src);
      if (isP(toks[a.end], ',')) {
        const b = readExpr(toks, a.end + 1, [',', ')'], src);
        const ps = pairsOf(a.toks, b.toks, src);
        if (!ps) dyn.push({ line: t.line, kind: 'pos', why: 'non-literal argument' });
        else for (const { rp, ep } of ps) out.push({ ...renderPair(rp, ep), line: t.line, kind: 'pos' });
      }
      continue;
    }
    // ---- 3. language ternary
    if (isP(t, '?')) {
      const p1 = toks[i - 1], p2 = toks[i - 2], p3 = toks[i - 3], p4 = toks[i - 4];
      let cond = false;
      if (isP(p1, ')') && isP(p2, '(') && p3 && p3.t === 'id' && p3.v === 'isRu') cond = true;
      else if (p1 && p1.t === 'str' && p1.v === 'ru' && p2 && p2.t === 'p' && (p2.v === '===' || p2.v === '==') && p3 && p3.t === 'id' && p3.v === 'lang') cond = true;
      if (cond) {
        const a = readExpr(toks, i + 1, [':'], src);
        if (!isP(toks[a.end], ':')) continue;
        const b = readExpr(toks, a.end + 1, [',', ')', ']', '}', ';', ':'], src);
        const ps = pairsOf(a.toks, b.toks, src);
        const locale = a.toks.length === 1 && a.toks[0].t === 'str' && /^[a-z]{2}-[A-Za-z]{2}$/.test(a.toks[0].v);
        if (locale) continue;
        if (!ps) {
          const hasText = a.toks.some((x) => x.t === 'str' || x.t === 'tpl') || b.toks.some((x) => x.t === 'str' || x.t === 'tpl');
          if (hasText) dyn.push({ line: t.line, kind: 'tern', why: 'non-literal branch' });
        } else for (const { rp, ep } of ps) out.push({ ...renderPair(rp, ep), line: t.line, kind: 'tern' });
      }
    }
  }
  return { pairs: out, dynamic: dyn };
}

// ------------------------------------------------------------------ files
function sourcesOf(file, text) {
  if (file.endsWith('.js') || file.endsWith('.mjs')) return [{ code: text, lineOffset: 0 }];
  // html: inline <script> bodies only
  const res = [];
  for (const m of text.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
    if (/\bsrc\s*=/.test(m[1])) continue;
    const before = text.slice(0, m.index + m[0].indexOf('>') + 1);
    res.push({ code: m[2], lineOffset: before.split('\n').length - 1 });
  }
  return res;
}

export function fnv1a64(str) {
  let h = 0xcbf29ce484222325n;
  for (const b of Buffer.from(str, 'utf8')) { h ^= BigInt(b); h = (h * 0x100000001b3n) & 0xffffffffffffffffn; }
  return h;
}
export const idOf = (ru, en) => fnv1a64(en + '\0' + ru);

export function extract(srcDir) {
  const files = fs.readdirSync(srcDir).filter((f) => /\.(js|html)$/.test(f)).sort();
  const perFile = {}; const entries = new Map(); const dynamic = [];
  let sites = 0; const byKind = { obj: 0, pos: 0, tern: 0 };
  for (const f of files) {
    const text = fs.readFileSync(path.join(srcDir, f), 'utf8');
    for (const s of sourcesOf(f, text)) {
      const { toks } = lex(s.code);
      const { pairs, dynamic: dy } = findPairs(toks, s.code, f);
      for (const p of pairs) {
        sites++; byKind[p.kind]++; perFile[f] = (perFile[f] || 0) + 1;
        if (!p.ru.trim() || !p.en.trim()) { dynamic.push({ file: f, line: p.line + s.lineOffset, kind: p.kind, why: 'empty side' }); continue; }
        const id = idOf(p.ru, p.en);
        const prev = entries.get(id);
        if (prev && (prev.ru !== p.ru || prev.en !== p.en)) throw new Error(`id collision ${id.toString(16)}: ${prev.en} / ${p.en}`);
        if (!prev) entries.set(id, { id, ru: p.ru, en: p.en, args: p.args, uses: 1, kinds: new Set([p.kind]) });
        else { prev.uses++; prev.kinds.add(p.kind); }
      }
      for (const d of dy) dynamic.push({ file: f, line: d.line + s.lineOffset, kind: d.kind, why: d.why });
    }
  }
  const list = [...entries.values()].sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return { list, sites, byKind, dynamic, perFile, files };
}

const rs = (s) => '"' + [...s].map((c) => {
  const cp = c.codePointAt(0);
  if (c === '"') return '\\"'; if (c === '\\') return '\\\\'; if (c === '\n') return '\\n'; if (c === '\r') return '\\r'; if (c === '\t') return '\\t';
  if (cp < 0x20 || cp === 0x7f || (cp >= 0x80 && cp < 0xa0) || cp === 0x2028 || cp === 0x2029 || cp === 0xfeff || cp === 0x200b || cp === 0xa0 && false) return `\\u{${cp.toString(16)}}`;
  return c;
}).join('') + '"';

export function render(r) {
  const lines = [];
  lines.push('// @generated by tools/dev/extract-i18n.mjs - do not edit; run `node tools/dev/extract-i18n.mjs` to refresh.');
  lines.push('//! The ru/en catalogue of the web UI: every `t({ ru, en })` pair found in src/*.js and src/*.html, deduplicated.');
  lines.push('//! `id` = FNV-1a 64 of `en + "\\0" + ru` (see i18n::id_of); the table is sorted by id. `{0}`, `{1}` are placeholders for the');
  lines.push('//! non-literal parts of a template / concatenation (fill them with i18n::fmt).');
  lines.push('');
  lines.push('/// Pair occurrences found in the sources (before deduplication), dynamic ones excluded.');
  lines.push(`pub const CALL_SITES: usize = ${r.sites - r.dynamic.filter((d) => d.why === 'empty side').length};`);
  lines.push('/// Occurrences that are not static text (a variable, a plural expression, a conditional): not in the table.');
  lines.push(`pub const DYNAMIC_SITES: usize = ${r.dynamic.filter((d) => d.why !== 'empty side').length};`);
  lines.push('/// Number of rows in `ENTRIES`.');
  lines.push(`pub const UNIQUE: usize = ${r.list.length};`);
  lines.push('');
  lines.push('/// `(id, ru, en)`, sorted by id.');
  lines.push('#[rustfmt::skip]');
  lines.push('pub static ENTRIES: &[(u64, &str, &str)] = &[');
  for (const e of r.list) lines.push(`    (0x${e.id.toString(16).padStart(16, '0')}, ${rs(e.ru)}, ${rs(e.en)}),`);
  lines.push('];');
  lines.push('');
  return lines.join('\n');
}

// ------------------------------------------------------------------ CLI
function main() {
  const argv = process.argv.slice(2);
  const arg = (n, d) => { const k = argv.indexOf(n); return k >= 0 ? argv[k + 1] : d; };
  const srcDir = path.resolve(arg('--src', path.join(ROOT, 'src')));
  const outFile = path.resolve(arg('--out', path.join(ROOT, 'src-tauri', 'ui-core', 'src', 'i18n', 'table.rs')));
  const r = extract(srcDir);
  const text = render(r);
  if (argv.includes('--json')) {
    process.stdout.write(JSON.stringify({ sites: r.sites, byKind: r.byKind, unique: r.list.length, dynamic: r.dynamic.length, perFile: r.perFile, dynamicList: r.dynamic }, null, 1) + '\n');
    return;
  }
  if (argv.includes('--report')) {
    console.log(`files scanned: ${r.files.length}; pair sites: ${r.sites}; unique: ${r.list.length}; dynamic/skipped: ${r.dynamic.length}`);
    for (const [f, n] of Object.entries(r.perFile).sort((a, b) => b[1] - a[1])) console.log(`  ${String(n).padStart(4)}  ${f}`);
    console.log('dynamic / skipped sites:');
    for (const d of r.dynamic) console.log(`  ${d.file}:${d.line}  [${d.kind}] ${d.why}`);
    return;
  }
  if (argv.includes('--check')) {
    const cur = fs.existsSync(outFile) ? fs.readFileSync(outFile, 'utf8').replace(/\r\n/g, '\n') : ''; // git autocrlf may have converted it
    if (cur !== text) { console.error('i18n table is out of date: run node tools/dev/extract-i18n.mjs'); process.exit(1); }
    console.log(`i18n table up to date (${r.list.length} entries)`);
    return;
  }
  fs.mkdirSync(path.dirname(outFile), { recursive: true });
  fs.writeFileSync(outFile, text);
  console.log(`wrote ${path.relative(ROOT, outFile)}: ${r.list.length} entries from ${r.sites} sites (${r.dynamic.length} dynamic skipped)`);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
