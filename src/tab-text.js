// Text tab: text area + counter on the left, "Tools | AI" and a grid of text tools on the right.
// Pure tool functions first (testable in node: see tab-text.test.mjs), UI below. No Rust: everything runs in the page.
// Owner: text/note agent. Request: text, note, timer and reminder tabs for the Pill replica.
(() => {
  'use strict';

  // ================= pure tools =================
  const EN_LO = "qwertyuiop[]asdfghjkl;'zxcvbnm,./`", RU_LO = 'йцукенгшщзхъфывапролджэячсмить' + 'бю.ё';
  const EN_UP = 'QWERTYUIOP{}ASDFGHJKL:"ZXCVBNM<>?~', RU_UP = 'ЙЦУКЕНГШЩЗХЪФЫВАПРОЛДЖЭЯЧСМИТЬБЮ,Ё';
  const EN = EN_LO + EN_UP + '@#$^&', RU = RU_LO + RU_UP + '"№;:?';
  const en2ru = {}, ru2en = {};
  for (let i = 0; i < EN.length; i++) { en2ru[EN[i]] = RU[i]; ru2en[RU[i]] = EN[i]; }

  /** RU<->EN keyboard layout swap; direction is detected (more Cyrillic letters than Latin = RU->EN). */
  function layoutFix(s) {
    const cyr = (s.match(/[а-яё]/gi) || []).length, lat = (s.match(/[a-z]/gi) || []).length;
    const map = cyr > lat ? ru2en : en2ru;
    return [...s].map((c) => map[c] ?? c).join('');
  }

  const T_RU = { а: 'a', б: 'b', в: 'v', г: 'g', д: 'd', е: 'e', ё: 'yo', ж: 'zh', з: 'z', и: 'i', й: 'y', к: 'k', л: 'l', м: 'm', н: 'n', о: 'o', п: 'p', р: 'r', с: 's', т: 't', у: 'u', ф: 'f', х: 'kh', ц: 'ts', ч: 'ch', ш: 'sh', щ: 'shch', ъ: '', ы: 'y', ь: '', э: 'e', ю: 'yu', я: 'ya' };
  const T_LAT = [['shch', 'щ'], ['sch', 'щ'], ['yo', 'ё'], ['zh', 'ж'], ['kh', 'х'], ['ts', 'ц'], ['ch', 'ч'], ['sh', 'ш'], ['yu', 'ю'], ['ya', 'я'],
    ['a', 'а'], ['b', 'б'], ['c', 'к'], ['d', 'д'], ['e', 'е'], ['f', 'ф'], ['g', 'г'], ['h', 'х'], ['i', 'и'], ['j', 'й'], ['k', 'к'], ['l', 'л'], ['m', 'м'],
    ['n', 'н'], ['o', 'о'], ['p', 'п'], ['q', 'к'], ['r', 'р'], ['s', 'с'], ['t', 'т'], ['u', 'у'], ['v', 'в'], ['w', 'в'], ['x', 'кс'], ['y', 'ы'], ['z', 'з']];
  const isUp = (c) => c !== undefined && c !== c.toLowerCase() && c === c.toUpperCase();
  const cap = (x, whole) => (whole ? x.toUpperCase() : x.charAt(0).toUpperCase() + x.slice(1));

  /** Transliteration: Cyrillic -> Latin, otherwise Latin -> Cyrillic (greedy digraphs, case kept). */
  function translit(s) {
    const chars = [...s];
    if (/[а-яё]/i.test(s)) {
      return chars.map((c, i) => {
        const lo = c.toLowerCase(), v = T_RU[lo];
        if (v === undefined) return c;
        if (!isUp(c)) return v;
        return cap(v, isUp(chars[i + 1]) || isUp(chars[i - 1]) && !/[a-zа-яё]/i.test(chars[i + 1] ?? ''));
      }).join('');
    }
    let out = '', i = 0;
    while (i < s.length) {
      const rest = s.slice(i).toLowerCase();
      const hit = T_LAT.find(([k]) => rest.startsWith(k));
      if (!hit) { out += s[i]; i++; continue; }
      const [k, v] = hit, src = s.slice(i, i + k.length);
      out += isUp(src[0]) ? cap(v, k.length > 1 && isUp(src[1]) || isUp(s[i + k.length])) : v;
      i += k.length;
    }
    return out;
  }

  const titleCase = (s) => s.toLowerCase().replace(/\p{L}[\p{L}\p{N}'’]*/gu, (w) => w.charAt(0).toUpperCase() + w.slice(1));
  const sentenceCase = (s) => s.toLowerCase().replace(/(^\s*|[.!?…]\s+|\n\s*)(\p{L})/gu, (m, a, b) => a + b.toUpperCase());
  const joinLines = (s) => s.replace(/\s*[\r\n]+\s*/g, ' ').trim();
  const extraSpaces = (s) => s.split(/\r\n|\r|\n/).map((l) => l.replace(/[ \t ]{2,}/g, ' ').trim()).join('\n');
  const trimText = (s) => s.split(/\r\n|\r|\n/).map((l) => l.trim()).join('\n').trim();
  const jsonPretty = (s) => JSON.stringify(JSON.parse(s), null, 2);
  const jsonMin = (s) => JSON.stringify(JSON.parse(s));
  const urlEnc = (s) => encodeURIComponent(s);
  const urlDec = (s) => decodeURIComponent(s.trim().replace(/\+/g, ' '));
  const dedupeLines = (s) => [...new Set(s.split(/\r\n|\r|\n/))].join('\n');
  /** Sort lines A-Z (numbers natural); a second press on already sorted text reverses the order. */
  function sortLines(s) {
    const col = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
    const a = s.split(/\r\n|\r|\n/), up = [...a].sort(col.compare);
    return (a.every((x, i) => x === up[i]) ? up.reverse() : up).join('\n');
  }

  function b64enc(s) {
    const bytes = new TextEncoder().encode(s);
    let bin = '';
    for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
    return btoa(bin);
  }
  function b64dec(s) {
    let c = s.replace(/\s+/g, '').replace(/-/g, '+').replace(/_/g, '/');
    if (!/^[A-Za-z0-9+/]*={0,2}$/.test(c)) throw new Error('not base64');
    c = c.replace(/=+$/, ''); c += '='.repeat((4 - (c.length % 4)) % 4);
    const bin = atob(c), bytes = Uint8Array.from(bin, (ch) => ch.charCodeAt(0));
    return new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  }

  function stats(s) {
    const chars = [...s].length, nospace = [...s.replace(/\s/g, '')].length;
    const words = s.trim() ? s.trim().split(/\s+/).length : 0;
    const lines = s === '' ? 0 : s.split(/\r\n|\r|\n/).length;
    const sentences = (s.match(/[^.!?…]+[.!?…]+|[^.!?…]+$/g) || []).filter((x) => x.trim()).length;
    return { chars, nospace, words, lines, sentences };
  }

  // ---- QR code: byte mode, error correction L, versions 1-10 (up to 271 UTF-8 bytes), mask chosen by penalty ----
  // ponytail: ECC L only and 271 bytes max; extend the two tables to go bigger.
  const QR_ECC = [0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18], QR_BLK = [0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4];
  const gfMul = (x, y) => { let z = 0; for (let i = 7; i >= 0; i--) { z = (z << 1) ^ ((z >>> 7) * 0x11d); z ^= ((y >>> i) & 1) * x; } return z; };
  function rsDivisor(deg) {
    const r = new Array(deg).fill(0); r[deg - 1] = 1;
    let root = 1;
    for (let i = 0; i < deg; i++) {
      for (let j = 0; j < deg; j++) { r[j] = gfMul(r[j], root); if (j + 1 < deg) r[j] ^= r[j + 1]; }
      root = gfMul(root, 2);
    }
    return r;
  }
  function rsRemainder(data, div) {
    const r = new Array(div.length).fill(0);
    for (const b of data) { const f = b ^ r.shift(); r.push(0); div.forEach((c, i) => { r[i] ^= gfMul(c, f); }); }
    return r;
  }
  const qrRaw = (v) => { let r = (16 * v + 128) * v + 64; if (v >= 2) { const n = Math.floor(v / 7) + 2; r -= (25 * n - 10) * n - 55; if (v >= 7) r -= 36; } return r; };
  const qrDataCw = (v) => Math.floor(qrRaw(v) / 8) - QR_ECC[v] * QR_BLK[v];

  /** Returns { size, get(x, y) } for the QR of `text`; throws Error('qr-too-long') above 271 bytes. */
  function qr(text) {
    const bytes = [...new TextEncoder().encode(text)];
    let ver = 1;
    while (ver <= 10 && 4 + (ver < 10 ? 8 : 16) + 8 * bytes.length > qrDataCw(ver) * 8) ver++;
    if (ver > 10) throw new Error('qr-too-long');
    const bits = [];
    const put = (val, n) => { for (let i = n - 1; i >= 0; i--) bits.push((val >>> i) & 1); };
    put(4, 4); put(bytes.length, ver < 10 ? 8 : 16); bytes.forEach((b) => put(b, 8));
    const capBits = qrDataCw(ver) * 8;
    put(0, Math.min(4, capBits - bits.length));
    put(0, (8 - (bits.length % 8)) % 8);
    for (let p = 0xec; bits.length < capBits; p ^= 0xec ^ 0x11) put(p, 8);
    const data = [];
    for (let i = 0; i < bits.length; i += 8) data.push(parseInt(bits.slice(i, i + 8).join(''), 2));

    // blocks + Reed-Solomon, interleaved
    const nb = QR_BLK[ver], eb = QR_ECC[ver], raw = Math.floor(qrRaw(ver) / 8), nShort = nb - (raw % nb), shortLen = Math.floor(raw / nb);
    const div = rsDivisor(eb), blocks = [];
    for (let i = 0, k = 0; i < nb; i++) {
      const dat = data.slice(k, k + shortLen - eb + (i < nShort ? 0 : 1)); k += dat.length;
      const ecc = rsRemainder(dat, div);
      if (i < nShort) dat.push(0);
      blocks.push(dat.concat(ecc));
    }
    const all = [];
    for (let i = 0; i < blocks[0].length; i++) blocks.forEach((b, j) => { if (i !== shortLen - eb || j >= nShort) all.push(b[i]); });

    // matrix
    const size = ver * 4 + 17, m = [], fn = [];
    for (let y = 0; y < size; y++) { m.push(new Array(size).fill(false)); fn.push(new Array(size).fill(false)); }
    const setF = (x, y, d) => { m[y][x] = d; fn[y][x] = true; };
    for (let i = 0; i < size; i++) { setF(6, i, i % 2 === 0); setF(i, 6, i % 2 === 0); }
    const finder = (cx, cy) => {
      for (let dy = -4; dy <= 4; dy++) for (let dx = -4; dx <= 4; dx++) {
        const d = Math.max(Math.abs(dx), Math.abs(dy)), x = cx + dx, y = cy + dy;
        if (x >= 0 && x < size && y >= 0 && y < size) setF(x, y, d !== 2 && d !== 4);
      }
    };
    finder(3, 3); finder(size - 4, 3); finder(3, size - 4);
    if (ver > 1) {
      const n = Math.floor(ver / 7) + 2, step = Math.ceil((ver * 4 + 4) / (n * 2 - 2)) * 2, pos = [6];
      for (let p = size - 7; pos.length < n; p -= step) pos.splice(1, 0, p);
      pos.forEach((cy, i) => pos.forEach((cx, j) => {
        if ((i === 0 && j === 0) || (i === 0 && j === n - 1) || (i === n - 1 && j === 0)) return;
        for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) setF(cx + dx, cy + dy, Math.max(Math.abs(dx), Math.abs(dy)) !== 1);
      }));
    }
    const bit = (v, i) => ((v >>> i) & 1) !== 0;
    const drawFormat = (mask) => {
      const d = (1 << 3) | mask; // ECC L = 01
      let rem = d;
      for (let i = 0; i < 10; i++) rem = (rem << 1) ^ ((rem >>> 9) * 0x537);
      const b = ((d << 10) | rem) ^ 0x5412;
      for (let i = 0; i <= 5; i++) setF(8, i, bit(b, i));
      setF(8, 7, bit(b, 6)); setF(8, 8, bit(b, 7)); setF(7, 8, bit(b, 8));
      for (let i = 9; i < 15; i++) setF(14 - i, 8, bit(b, i));
      for (let i = 0; i < 8; i++) setF(size - 1 - i, 8, bit(b, i));
      for (let i = 8; i < 15; i++) setF(8, size - 15 + i, bit(b, i));
      setF(8, size - 8, true);
    };
    drawFormat(0);
    if (ver >= 7) {
      let rem = ver;
      for (let i = 0; i < 12; i++) rem = (rem << 1) ^ ((rem >>> 11) * 0x1f25);
      const b = (ver << 12) | rem;
      for (let i = 0; i < 18; i++) { const a = size - 11 + (i % 3), c = Math.floor(i / 3); setF(a, c, bit(b, i)); setF(c, a, bit(b, i)); }
    }
    for (let i = 0, right = size - 1; right >= 1; right -= 2) {
      if (right === 6) right = 5;
      for (let vert = 0; vert < size; vert++) for (let j = 0; j < 2; j++) {
        const x = right - j, y = ((right + 1) & 2) === 0 ? size - 1 - vert : vert;
        if (!fn[y][x] && i < all.length * 8) { m[y][x] = bit(all[i >>> 3], 7 - (i & 7)); i++; }
      }
    }
    const MASKS = [(x, y) => (x + y) % 2 === 0, (x, y) => y % 2 === 0, (x) => x % 3 === 0, (x, y) => (x + y) % 3 === 0,
      (x, y) => (Math.floor(x / 3) + Math.floor(y / 2)) % 2 === 0, (x, y) => ((x * y) % 2) + ((x * y) % 3) === 0,
      (x, y) => ((((x * y) % 2) + ((x * y) % 3)) % 2) === 0, (x, y) => ((((x + y) % 2) + ((x * y) % 3)) % 2) === 0];
    const applyMask = (k) => { for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (!fn[y][x] && MASKS[k](x, y)) m[y][x] = !m[y][x]; };
    const penalty = () => {
      let p = 0, dark = 0;
      const lines = [];
      for (let y = 0; y < size; y++) { lines.push(m[y].map((v) => (v ? 1 : 0)).join('')); }
      for (let x = 0; x < size; x++) { let c = ''; for (let y = 0; y < size; y++) c += m[y][x] ? 1 : 0; lines.push(c); }
      for (const l of lines) {
        for (const r of l.match(/0+|1+/g)) if (r.length >= 5) p += 3 + (r.length - 5);
        for (const pat of ['10111010000', '00001011101']) for (let i = l.indexOf(pat); i >= 0; i = l.indexOf(pat, i + 1)) p += 40;
      }
      for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
        if (m[y][x]) dark++;
        if (x < size - 1 && y < size - 1 && m[y][x] === m[y][x + 1] && m[y][x] === m[y + 1][x] && m[y][x] === m[y + 1][x + 1]) p += 3;
      }
      return p + (Math.ceil(Math.abs(dark * 20 - size * size * 10) / (size * size)) - 1) * 10;
    };
    let best = 0, bestP = Infinity;
    for (let k = 0; k < 8; k++) { applyMask(k); drawFormat(k); const p = penalty(); applyMask(k); if (p < bestP) { bestP = p; best = k; } }
    applyMask(best); drawFormat(best);
    return { size, version: ver, mask: best, get: (x, y) => m[y][x] };
  }

  const api = { layoutFix, translit, titleCase, sentenceCase, joinLines, extraSpaces, trimText, jsonPretty, jsonMin, urlEnc, urlDec,
    dedupeLines, sortLines, b64enc, b64dec, stats, qr };
  if (typeof window === 'undefined') return;
  window.PillTextTools = api;
  if (!window.Pill || !window.Pill.register) return;

  // ================= UI =================
  const t = (d) => (window.Pill && window.Pill.t ? window.Pill.t(d) : d.en); // evaluated lazily, Pill.t may arrive later
  const ru = () => /^ru/i.test((window.Pill && window.Pill.lang) || navigator.language || '');
  const plural = (n, one, few, many) => { // Russian forms; English passes (one, many, many)
    const a = Math.abs(n) % 100, b = a % 10;
    return a > 10 && a < 20 ? many : b === 1 ? one : b >= 2 && b <= 4 ? few : many;
  };
  const el = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  const svg = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
  const ICON = {
    tab: svg('<path d="M5 7V4h14v3M12 4v16M9 20h6"/>'),
    paste: svg('<rect x="5" y="4" width="14" height="17" rx="2.5"/><path d="M9 4.5h6M12 17v-6M9.5 13.5 12 11l2.5 2.5"/>'),
    copy: svg('<rect x="9" y="9" width="11" height="11" rx="2.5"/><path d="M15 9V6.5A2.5 2.5 0 0 0 12.5 4h-6A2.5 2.5 0 0 0 4 6.5v6A2.5 2.5 0 0 0 6.5 15H9"/>'),
    ai: svg('<path d="M12 3l1.8 4.7L18.5 9.5l-4.7 1.8L12 16l-1.8-4.7L5.5 9.5l4.7-1.8z"/><path d="M18.5 15v4M16.5 17h4"/>'),
    x: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
  };

  const TOOLS = [
    ['layout', { ru: 'Раскладка', en: 'Layout' }, layoutFix, { ru: 'Исправить раскладку RU <-> EN', en: 'Fix RU <-> EN keyboard layout' }],
    ['upper', { ru: 'ВЕРХНИЙ', en: 'UPPER' }, (s) => s.toUpperCase()],
    ['lower', { ru: 'нижний', en: 'lower' }, (s) => s.toLowerCase()],
    ['title', { ru: 'Каждое Слово', en: 'Title Case' }, titleCase],
    ['sentence', { ru: 'Предложение', en: 'Sentence' }, sentenceCase, { ru: 'Заглавная в начале предложений', en: 'Capital at the start of sentences' }],
    ['translit', { ru: 'Транслит', en: 'Translit' }, translit, { ru: 'Кириллица <-> латиница', en: 'Cyrillic <-> Latin' }],
    ['join', { ru: 'Убрать переносы', en: 'Join lines' }, joinLines],
    ['spaces', { ru: 'Лишние пробелы', en: 'Extra spaces' }, extraSpaces],
    ['jpretty', { ru: 'JSON красиво', en: 'JSON pretty' }, jsonPretty],
    ['jmin', { ru: 'JSON в строку', en: 'JSON minify' }, jsonMin],
    ['b64e', { ru: '→ Base64', en: '→ Base64' }, b64enc],
    ['b64d', { ru: 'Base64 →', en: 'Base64 →' }, b64dec],
    ['urle', { ru: '→ URL', en: '→ URL' }, urlEnc, { ru: 'Закодировать для URL', en: 'URL-encode' }],
    ['urld', { ru: 'URL →', en: 'URL →' }, urlDec, { ru: 'Раскодировать из URL', en: 'URL-decode' }],
    ['trim', { ru: 'Обрезать', en: 'Trim' }, trimText, { ru: 'Пробелы по краям строк и текста', en: 'Trim every line and the text' }],
    ['sort', { ru: 'Сортировка', en: 'Sort lines' }, sortLines, { ru: 'А-Я; повторное нажатие: обратный порядок', en: 'A-Z; press again to reverse' }],
    ['dedupe', { ru: 'Без дублей', en: 'Unique lines' }, dedupeLines],
    ['qr', { ru: 'QR-код', en: 'QR code' }, null],
    ['count', { ru: 'Статистика', en: 'Word count' }, null],
  ];
  const ERR = {
    jpretty: { ru: 'Это не JSON', en: 'Not valid JSON' }, jmin: { ru: 'Это не JSON', en: 'Not valid JSON' },
    b64d: { ru: 'Это не Base64 (или не текст)', en: 'Not valid Base64 (or not text)' }, urld: { ru: 'Неверная URL-кодировка', en: 'Invalid URL encoding' },
  };

  // AI mode: the model rewrites the selection (or the whole text) through the same chat endpoint as the chat tab (Rust: aiactions.rs)
  const AI = [
    ['translate', { ru: 'Перевести', en: 'Translate' }, { ru: 'На английский, а английский на русский', en: 'To English; English text goes to Russian' }],
    ['fix', { ru: 'Исправить ошибки', en: 'Fix errors' }, { ru: 'Орфография и грамматика', en: 'Spelling and grammar' }],
    ['shorten', { ru: 'Сократить', en: 'Shorten' }, { ru: 'Примерно вдвое', en: 'To about half' }],
    ['summarize', { ru: 'Кратко', en: 'Summarize' }, { ru: 'Суть в нескольких предложениях', en: 'The gist in a few sentences' }],
    ['formal', { ru: 'Формально', en: 'Formal' }, { ru: 'Деловой тон', en: 'A professional tone' }],
    ['friendly', { ru: 'Дружелюбно', en: 'Friendly' }, { ru: 'Тёплый, неформальный тон', en: 'A warm, relaxed tone' }],
    ['bullets', { ru: 'Списком', en: 'As a list' }, { ru: 'Разбить на пункты', en: 'Split into bullet points' }],
    ['explain', { ru: 'Объяснить', en: 'Explain' }, { ru: 'Простыми словами', en: 'In plain words' }],
    ['continue', { ru: 'Продолжить', en: 'Continue' }, { ru: 'Дописать несколько предложений', en: 'Write a few more sentences' }],
  ];
  let ta, countEl, grid, aiGrid, toastEl, qrBox, toastT = 0, aiBusy = false;
  const labels = []; // [element, dict, attr] re-applied by applyLang()

  function say(msg, bad, sticky) {
    toastEl.textContent = msg; toastEl.classList.toggle('bad', !!bad); toastEl.classList.add('show');
    clearTimeout(toastT); if (!sticky) toastT = setTimeout(() => toastEl.classList.remove('show'), 2200);
  }
  function updateCount() {
    const s = stats(ta.value), r = ru();
    countEl.textContent = r
      ? `${s.chars} симв. · ${s.nospace} без пробелов · ${s.words} ${plural(s.words, 'слово', 'слова', 'слов')} · ${s.lines} стр.`
      : `${s.chars} ${s.chars === 1 ? 'char' : 'chars'} · ${s.nospace} without spaces · ${s.words} ${s.words === 1 ? 'word' : 'words'} · ${s.lines} ${s.lines === 1 ? 'line' : 'lines'}`;
  }
  function applyLang() {
    labels.forEach(([e, d, attr]) => { if (attr) e.setAttribute(attr, t(d)); else e.textContent = t(d); });
    updateCount();
  }
  const L = (e, d, attr) => { labels.push([e, d, attr]); if (attr) e.setAttribute(attr, t(d)); else e.textContent = t(d); return e; };

  // Replace text through the editing pipeline so the textarea's own Ctrl+Z restores it.
  function replaceRange(start, end, out) {
    ta.focus(); ta.setSelectionRange(start, end);
    const ok = out === '' ? document.execCommand('delete') : document.execCommand('insertText', false, out);
    if (!ok) { ta.setRangeText(out, start, end, 'end'); ta.dispatchEvent(new Event('input', { bubbles: true })); }
    updateCount();
  }
  function run(tool) {
    const [id, , fn] = tool, a = ta.selectionStart, b = ta.selectionEnd, sel = a !== b;
    const src = sel ? ta.value.slice(a, b) : ta.value;
    if (id === 'count') {
      const s = stats(src), r = ru();
      const min = Math.max(1, Math.round(s.words / 200));
      say(r ? `${s.words} ${plural(s.words, 'слово', 'слова', 'слов')} · ${s.sentences} предл. · ~${min} мин чтения`
        : `${s.words} ${s.words === 1 ? 'word' : 'words'} · ${s.sentences} ${s.sentences === 1 ? 'sentence' : 'sentences'} · ~${min} min read`);
      return;
    }
    if (id === 'qr') return showQr(src);
    if (!src) return;
    let out;
    try { out = fn(src); } catch (e) { say(t(ERR[id] || { ru: 'Не получилось', en: 'Failed' }), true); return; }
    if (out === src) { say(t({ ru: 'Без изменений', en: 'No change' })); return; }
    if (sel) replaceRange(a, b, out); else replaceRange(0, ta.value.length, out);
  }

  /** One AI request on the selection (or the whole text); a second click while it runs cancels. The result goes in through the editing
   *  pipeline, so Ctrl+Z brings the original back. */
  async function aiRun(action, instruction, btn) {
    if (aiBusy) { Pill.invoke('ai_transform_cancel'); return; }
    const a = ta.selectionStart, b = ta.selectionEnd, sel = a !== b;
    const src = sel ? ta.value.slice(a, b) : ta.value;
    if (!src.trim()) { say(t({ ru: 'Сначала напиши или вставь текст', en: 'Type or paste some text first' }), true); return; }
    aiBusy = true; btn.classList.add('run'); aiGrid.classList.add('busy');
    say(t({ ru: 'ИИ думает… нажми ещё раз, чтобы отменить', en: 'AI is thinking… click again to cancel' }), false, true);
    let r = null;
    try { r = await Pill.invoke('ai_transform', { text: src, action, instruction: instruction || null }); } catch (e) { r = { ok: false, error: String(e) }; }
    aiBusy = false; btn.classList.remove('run'); aiGrid.classList.remove('busy');
    if (!r || !r.ok) {
      const e = r && r.error;
      if (e === 'cancelled') { say(t({ ru: 'Отменено', en: 'Cancelled' })); return; }
      const msg = e === 'net_off' ? t({ ru: 'ИИ выключен: Настройки → Сеть → ИИ', en: 'AI is off: Settings → Network → AI' })
        : e === 'empty' ? t({ ru: 'Нет текста', en: 'No text' }) : String(e || t({ ru: 'ИИ не ответил', en: 'The AI did not answer' })).slice(0, 140);
      say(msg, true); return;
    }
    const still = sel ? ta.value.slice(a, b) === src : ta.value === src;
    if (!still) { say(t({ ru: 'Текст изменился, пока ИИ думал', en: 'The text changed while the AI was working' }), true); return; }
    const out = String(r.text);
    if (action === 'continue') replaceRange(sel ? b : ta.value.length, sel ? b : ta.value.length, (/\s$/.test(src) ? '' : ' ') + out);
    else if (sel) replaceRange(a, b, out); else replaceRange(0, ta.value.length, out);
    say(t({ ru: 'Готово · Ctrl+Z вернёт как было', en: 'Done · Ctrl+Z restores the original' }));
  }

  function showQr(text) {
    if (!text.trim()) return;
    let q;
    try { q = qr(text); } catch (e) { say(t({ ru: 'Слишком длинно для QR (макс. 271 байт)', en: 'Too long for a QR code (max 271 bytes)' }), true); return; }
    const c = qrBox.querySelector('canvas'), quiet = 4, px = Math.max(3, Math.floor(190 / (q.size + quiet * 2)));
    c.width = c.height = (q.size + quiet * 2) * px;
    const g = c.getContext('2d');
    g.fillStyle = '#fff'; g.fillRect(0, 0, c.width, c.height); g.fillStyle = '#000';
    for (let y = 0; y < q.size; y++) for (let x = 0; x < q.size; x++) if (q.get(x, y)) g.fillRect((x + quiet) * px, (y + quiet) * px, px, px);
    window.PillMotion.cancel(qrBox); // a running close is dropped: the code stays
    qrBox.classList.add('show');
  }

  async function copyAll() {
    const a = ta.selectionStart, b = ta.selectionEnd, txt = a !== b ? ta.value.slice(a, b) : ta.value;
    if (!txt) return;
    try { await navigator.clipboard.writeText(txt); } catch (e) { ta.focus(); if (a === b) ta.select(); document.execCommand('copy'); }
    say(t({ ru: 'Скопировано', en: 'Copied' }));
  }
  async function pasteIn() {
    try {
      const txt = await navigator.clipboard.readText();
      if (txt) replaceRange(ta.selectionStart, ta.selectionEnd, txt);
    } catch (e) { say(t({ ru: 'Нет доступа к буферу обмена', en: 'Clipboard is not accessible' }), true); }
  }

  function mount(view) {
    const root = el('div', 'tab-text'); view.append(root);
    const left = el('div', 'tt-left'), right = el('div', 'tt-right');
    root.append(left, right);

    const box = el('div', 'tt-box');
    ta = el('textarea', 'tt-area'); ta.spellcheck = false;
    L(ta, { ru: 'Вставь или напиши текст…', en: 'Type or paste text…' }, 'placeholder');
    ta.addEventListener('input', updateCount);
    toastEl = el('div', 'tt-toast');
    qrBox = el('div', 'tt-qr');
    const qc = el('canvas'), qx = el('button', 'tt-qr-x'); qx.innerHTML = ICON.x; qx.onclick = () => window.PillMotion.exit(qrBox, { scale: 0.97, after: (e) => e.classList.remove('show') });
    L(qx, { ru: 'Закрыть', en: 'Close' }, 'title');
    qrBox.append(qc, qx);
    box.append(ta, qrBox, toastEl);

    const foot = el('div', 'tt-foot');
    countEl = el('span', 'tt-count');
    const bp = el('button', 'tt-ico'), bc = el('button', 'tt-ico');
    bp.innerHTML = ICON.paste; bc.innerHTML = ICON.copy;
    L(bp, { ru: 'Вставить из буфера', en: 'Paste' }, 'title'); L(bc, { ru: 'Копировать', en: 'Copy' }, 'title');
    [bp, bc].forEach((b) => { b.type = 'button'; b.onmousedown = (e) => e.preventDefault(); });
    bp.onclick = pasteIn; bc.onclick = copyAll;
    foot.append(countEl, bp, bc);
    left.append(box, foot);

    const seg = el('div', 'tt-seg');
    const bt = el('button', 'on'), bai = el('button', 'tt-ai');
    bt.type = bai.type = 'button';
    L(bt, { ru: 'Инструменты', en: 'Tools' });
    bai.innerHTML = ICON.ai + '<span></span>'; L(bai.querySelector('span'), { ru: 'ИИ', en: 'AI' });
    L(bai, { ru: 'Переписать, перевести или сократить с помощью ИИ', en: 'Rewrite, translate or shorten with AI' }, 'title');
    seg.append(bt, bai);

    grid = el('div', 'tt-grid');
    TOOLS.forEach((tool) => {
      const b = el('button', 'tt-tool'); b.type = 'button';
      L(b, tool[1]); if (tool[3]) L(b, tool[3], 'title');
      b.onmousedown = (e) => e.preventDefault(); // keep the selection and focus in the text area
      b.onclick = () => run(tool);
      grid.append(b);
    });
    aiGrid = el('div', 'tt-grid tt-aigrid'); aiGrid.hidden = true;
    AI.forEach(([id, label, tip]) => {
      const b = el('button', 'tt-tool'); b.type = 'button';
      L(b, label); L(b, tip, 'title');
      b.onmousedown = (e) => e.preventDefault();
      b.onclick = () => aiRun(id, '', b);
      aiGrid.append(b);
    });
    const ask = el('div', 'tt-ask'), askIn = el('input'), askGo = el('button');
    askIn.type = 'text'; askIn.spellcheck = false; askGo.type = 'button'; askGo.innerHTML = ICON.ai;
    L(askIn, { ru: 'Свой запрос: «сделай бодрее», «на немецкий»…', en: 'Your own request: "make it punchier", "to German"…' }, 'placeholder');
    L(askGo, { ru: 'Выполнить запрос', en: 'Run the request' }, 'title');
    askGo.onmousedown = (e) => e.preventDefault();
    const go = () => { const q = askIn.value.trim(); if (q) aiRun('custom', q, askGo); };
    askIn.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); go(); } e.stopPropagation(); });
    askGo.onclick = go;
    ask.append(askIn, askGo); aiGrid.prepend(ask); // on top: the tile rows below may scroll, the request line must always be in reach
    const setMode = (on) => { bt.classList.toggle('on', !on); bai.classList.toggle('on', on); grid.hidden = on; aiGrid.hidden = !on; };
    bt.onclick = () => setMode(false); bai.onclick = () => setMode(true);
    right.append(seg, grid, aiGrid);
    applyLang();
  }

  Pill.register({ id: 'text', title: { ru: 'Текст', en: 'Text' }, icon: window.pillIcon ? window.pillIcon('text') : ICON.tab, order: 30, mount, onShow: applyLang });
})();
