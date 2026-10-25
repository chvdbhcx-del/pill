// Tiny SAFE markdown subset for the command bar's AI answers. No library, no innerHTML: parse() returns plain data,
// render() builds DOM nodes with textContent only. Supported: paragraphs, "-"/"*"/"1." lists, **bold**, *italic*, `code`,
// fenced ``` blocks (with a copy button), "#" headings (shown as bold text). Links are shown as text, never clickable, never fetched.
// Node:  node tools/test-md-lite.js
(function (root) {
  const WINDOW = 500; // a closing mark is looked for this far ahead only (keeps unmatched "*" linear on huge answers)
  const LIST = /^\s*(?:([-*+])|(\d{1,3})[.)])\s+(.*)$/;

  // inline -> [{t:'text',s} | {t:'b'|'i',c:[...]} | {t:'code',s} | {t:'link',s,u}]
  function inline(s) {
    const out = [];
    let buf = '', i = 0;
    const flush = () => { if (buf) { out.push({ t: 'text', s: buf }); buf = ''; } };
    while (i < s.length) {
      const ch = s[i];
      if (ch === '`') {
        const j = s.indexOf('`', i + 1);
        if (j > i + 1 && j - i <= WINDOW) { flush(); out.push({ t: 'code', s: s.slice(i + 1, j) }); i = j + 1; continue; }
      } else if (ch === '*' && s[i + 1] === '*') {
        const j = s.slice(i + 2, i + 2 + WINDOW).indexOf('**');
        if (j > 0 && !/^\s|\s$/.test(s.slice(i + 2, i + 2 + j))) { flush(); out.push({ t: 'b', c: inline(s.slice(i + 2, i + 2 + j)) }); i += j + 4; continue; }
      } else if (ch === '*' && s[i + 1] && !/[\s*]/.test(s[i + 1])) {
        const end = Math.min(s.length, i + 1 + WINDOW);
        let j = -1;
        for (let k = i + 2; k < end; k++) if (s[k] === '*' && s[k - 1] !== '*' && s[k + 1] !== '*' && !/\s/.test(s[k - 1])) { j = k; break; }
        if (j > 0) { flush(); out.push({ t: 'i', c: inline(s.slice(i + 1, j)) }); i = j + 1; continue; }
      } else if (ch === '[') {
        const m = /^\[([^\]\n]{1,200})\]\(([^)\s]{0,500})\)/.exec(s.slice(i, i + 720));
        if (m) { flush(); out.push({ t: 'link', s: m[1], u: m[2] }); i += m[0].length; continue; }
      }
      buf += ch; i++;
    }
    flush();
    return out;
  }

  // text -> [{t:'p',c} | {t:'ul'|'ol',items:[c...]} | {t:'code',lang,s}]
  function parse(text) {
    const lines = String(text == null ? '' : text).replace(/\r\n?/g, '\n').split('\n'), blocks = [];
    let para = [], list = null;
    const endPara = () => { if (para.length) { blocks.push({ t: 'p', c: inline(para.join('\n')) }); para = []; } };
    for (let n = 0; n < lines.length; n++) {
      const line = lines[n], f = /^\s*```\s*([\w+#.-]*)/.exec(line);
      if (f) {
        endPara(); list = null;
        const code = [];
        for (n++; n < lines.length && !/^\s*```\s*$/.test(lines[n]); n++) code.push(lines[n]); // an unclosed fence runs to the end
        blocks.push({ t: 'code', lang: f[1] || '', s: code.join('\n') });
        continue;
      }
      if (!line.trim()) { endPara(); list = null; continue; }
      const m = LIST.exec(line);
      if (m) {
        endPara();
        const kind = m[1] ? 'ul' : 'ol';
        if (!list || list.t !== kind) { list = { t: kind, items: [] }; blocks.push(list); }
        list.items.push(inline(m[3]));
        continue;
      }
      if (list && /^\s/.test(line)) { // indented continuation of the last item
        list.items[list.items.length - 1].push(...inline('\n' + line.trim()));
        continue;
      }
      list = null;
      const h = /^#{1,6}\s+(.*)$/.exec(line);
      if (h) { endPara(); blocks.push({ t: 'p', c: [{ t: 'b', c: inline(h[1]) }] }); continue; }
      para.push(line);
    }
    endPara();
    return blocks;
  }

  const flat = (c) => c.map((x) => (x.t === 'b' || x.t === 'i' ? flat(x.c) : x.t === 'link' ? (x.u ? `${x.s} (${x.u})` : x.s) : x.s)).join('');

  // the answer as plain text, markers stripped (for "copy the answer")
  function plain(text) {
    return parse(text).map((b) => (b.t === 'code' ? b.s : b.t === 'p' ? flat(b.c) : b.items.map((it, i) => (b.t === 'ul' ? '- ' : `${i + 1}. `) + flat(it)).join('\n'))).join('\n\n').trim();
  }

  // DOM for the blocks; `doc` is the document, `onCopy(text, button)` is called by a code block's button
  function render(doc, text, onCopy, copyLabel) {
    const frag = doc.createDocumentFragment();
    const el = (tag, cls) => { const e = doc.createElement(tag); if (cls) e.className = cls; return e; };
    const fill = (parent, c) => {
      for (const x of c) {
        if (x.t === 'text') parent.append(doc.createTextNode(x.s));
        else if (x.t === 'code') { const e = el('code'); e.textContent = x.s; parent.append(e); }
        else if (x.t === 'b' || x.t === 'i') { const e = el(x.t === 'b' ? 'strong' : 'em'); fill(e, x.c); parent.append(e); }
        else { const e = el('span', 'lnk'); e.textContent = x.u ? `${x.s} (${x.u})` : x.s; parent.append(e); } // a plain span: not an <a>, nothing to click or fetch
      }
    };
    for (const b of parse(text)) {
      if (b.t === 'p') { const e = el('p'); fill(e, b.c); frag.append(e); }
      else if (b.t === 'code') {
        const w = el('div', 'cb'), pre = el('pre'), code = el('code'), btn = el('button', 'cbcopy');
        code.textContent = b.s; pre.append(code);
        btn.type = 'button'; btn.tabIndex = -1; btn.textContent = copyLabel || 'Copy';
        btn.addEventListener('mousedown', (e) => e.preventDefault()); // keep the caret in the input
        btn.addEventListener('click', () => onCopy && onCopy(b.s, btn));
        w.append(pre, btn); frag.append(w);
      } else {
        const e = el(b.t);
        for (const it of b.items) { const li = el('li'); fill(li, it); e.append(li); }
        frag.append(e);
      }
    }
    return frag;
  }

  const api = { parse, plain, render };
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  else root.MdLite = api;
})(typeof window !== 'undefined' ? window : globalThis);
