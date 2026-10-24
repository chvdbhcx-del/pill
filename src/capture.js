// Capture overlay (capture.html): frozen frame, area selection + markup, OCR / QR / record selection, colour-picker lens.
// Owner: capture agent. Rust side: src-tauri/src/capture.rs (capture_session / capture_frame / capture_ready / capture_commit / capture_cancel).
// The page only ever sends a selection rectangle (css px) or a pixel position; Rust crops its own copy of the frame.
// The one exception is a flattened markup PNG (area mode with annotations), sent as a raw body with the args in a header.
(() => {
  'use strict';
  const T = window.__TAURI__;
  const ru = /^ru/i.test(navigator.language || '');
  const L = (a, b) => (ru ? a : b);
  const $ = (s) => document.querySelector(s);
  const qs = new URLSearchParams(location.search);
  const meta = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.metadata;
  const label = (meta && meta.currentWindow && meta.currentWindow.label) || qs.get('m') || 'capture-0';
  const inv = async (c, a, o) => { try { return await T.core.invoke(c, a, o); } catch (e) { console.error('capture', c, e); return null; } };

  const frameEl = $('#frame'), annEl = $('#ann'), selEl = $('#sel'), sizeEl = $('#size'), hintEl = $('#hint'), barEl = $('#bar');
  const lensEl = $('#lens'), lensC = $('#lensc'), lctx = lensC.getContext('2d');
  const fctx = frameEl.getContext('2d'), actx = annEl.getContext('2d');
  const pix = document.createElement('div');
  pix.id = 'pix'; pix.hidden = true; document.body.append(pix);

  const COLORS = ['#ff3b30', '#ffcc00', '#34c759', '#0a84ff', '#ffffff'];
  const S = { lastId: 0, id: 0, mode: 'area', fw: 0, fh: 0, px: null, sel: null, tool: 'select', color: COLORS[0], ops: [], draft: null, busy: false, arming: false, cx: 0, cy: 0 };
  let drag = null, txt = null;
  const rx = () => S.fw / Math.max(1, innerWidth), ry = () => S.fh / Math.max(1, innerHeight);
  const clampX = (v) => Math.max(0, Math.min(innerWidth, v)), clampY = (v) => Math.max(0, Math.min(innerHeight, v));

  // ---------- icons (shared set from icons.js + a few own shapes) ----------
  const own = {
    select: '<path d="m5 3 14 7.500-6.200 2L10.500 19z"/>',
    rect: '<rect x="4" y="5.500" width="16" height="13" rx="2"/>',
    arrow: '<path d="M5 19 19 5M9 5h10v10"/>',
    pen: '<path d="M3 17c3-6 5-9 7-9s0 8 3 8 3-6 8-9"/>',
    blur: '<rect x="4" y="4" width="6" height="6"/><rect x="14" y="4" width="6" height="6" opacity=".45"/><rect x="9" y="9" width="6" height="6"/><rect x="4" y="14" width="6" height="6" opacity=".45"/><rect x="14" y="14" width="6" height="6"/>',
    text: '<path d="M5 7V4.500h14V7M12 4.500v15M9 19.500h6"/>',
    undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10a6 6 0 0 1 0 12h-3"/>',
    save: '<path d="M5 3h11l3 3v15H5z"/><path d="M8 3v6h8V3M8 21v-7h8v7"/>',
  };
  const ic = (n) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${own[n] || (window.PillIcons && window.PillIcons[n]) || ''}</svg>`;

  // ---------- session ----------
  async function arm() {
    if (S.arming) return;
    S.arming = true;
    try {
      const info = await inv('capture_session', { label });
      if (!info || info.id === S.lastId) return;
      clearAll();
      S.lastId = S.id = info.id; S.mode = info.mode; S.fw = info.w; S.fh = info.h;
      document.body.dataset.mode = S.mode;
      const buf = await inv('capture_frame', { label });
      const bytes = buf instanceof ArrayBuffer ? new Uint8ClampedArray(buf) : Uint8ClampedArray.from(buf || []);
      if (bytes.length !== S.fw * S.fh * 4) { console.error('capture: frame size mismatch', bytes.length, S.fw, S.fh); inv('capture_cancel'); return; }
      frameEl.width = S.fw; frameEl.height = S.fh; annEl.width = S.fw; annEl.height = S.fh;
      fctx.putImageData(new ImageData(bytes, S.fw, S.fh), 0, 0);
      if (S.mode === 'color') S.px = bytes;
      S.cx = Math.floor(S.fw / 2); S.cy = Math.floor(S.fh / 2);
      hintEl.textContent = HINT[S.mode] ? HINT[S.mode]() : '';
      hintEl.classList.toggle('off', !HINT[S.mode]);
      if (S.mode === 'color') { lensEl.hidden = false; pix.hidden = false; drawLens(); }
      // The window is still hidden here (Rust shows it on capture_ready), so rAF and timers may be paused or throttled: yield via a
      // MessageChannel, which is neither.
      await new Promise((r) => { const c = new MessageChannel(); c.port1.onmessage = () => r(); c.port2.postMessage(0); });
      inv('capture_ready', { label }).then(() => refocus());
    } finally { S.arming = false; }
  }
  // Keyboard focus into the page itself (Esc needs it): the window may be foreground while WebView2 holds no focus.
  function refocus() {
    try { window.focus(); document.body.tabIndex = -1; document.body.focus({ preventScroll: true }); } catch (_) { /* optional */ }
    if (!document.hasFocus() && T && T.window) { try { T.window.getCurrentWindow().setFocus(); } catch (_) { /* optional */ } }
  }

  const HINT = {
    area: () => L('Выдели область · Esc — отмена', 'Drag to select an area · Esc to cancel'),
    text: () => L('Выдели текст на экране · Esc — отмена', 'Select the text to read · Esc to cancel'),
    qr: () => L('Выдели QR-код · Esc — отмена', 'Select a QR code · Esc to cancel'),
    record: () => L('Выдели область записи · Enter — весь экран · Esc — отмена', 'Select the area to record · Enter for the whole screen · Esc to cancel'),
    color: () => L('Клик — скопировать цвет · стрелки — на 1 пиксель · Esc — отмена', 'Click to copy the colour · arrows move 1 pixel · Esc to cancel'),
  };

  function clearAll() {
    drag = null; closeText();
    S.id = 0; S.sel = null; S.ops = []; S.draft = null; S.px = null; S.busy = false; S.tool = 'select';
    selEl.hidden = true; barEl.hidden = true; lensEl.hidden = true; pix.hidden = true;
    document.body.classList.remove('has-sel'); document.body.removeAttribute('data-cursor');
    frameEl.width = 1; frameEl.height = 1; annEl.width = 1; annEl.height = 1; // release the pixels
  }

  // ---------- selection geometry ----------
  const norm = (a, b) => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });
  const inside = (p) => S.sel && p.x >= S.sel.x && p.x <= S.sel.x + S.sel.w && p.y >= S.sel.y && p.y <= S.sel.y + S.sel.h;
  function hit(p) {
    const s = S.sel, T8 = 8;
    if (!s) return '';
    const nl = Math.abs(p.x - s.x) <= T8, nr = Math.abs(p.x - (s.x + s.w)) <= T8, nt = Math.abs(p.y - s.y) <= T8, nb = Math.abs(p.y - (s.y + s.h)) <= T8;
    const ix = p.x > s.x - T8 && p.x < s.x + s.w + T8, iy = p.y > s.y - T8 && p.y < s.y + s.h + T8;
    if (nt && nl) return 'nw'; if (nt && nr) return 'ne'; if (nb && nl) return 'sw'; if (nb && nr) return 'se';
    if (nt && ix) return 'n'; if (nb && ix) return 's'; if (nl && iy) return 'w'; if (nr && iy) return 'e';
    return inside(p) ? 'in' : '';
  }
  const CUR = { nw: 'nwse-resize', se: 'nwse-resize', ne: 'nesw-resize', sw: 'nesw-resize', n: 'ns-resize', s: 'ns-resize', e: 'ew-resize', w: 'ew-resize' };
  function setCursor(c) { if (c) { document.body.dataset.cursor = '1'; document.body.style.setProperty('--cur', c); } else document.body.removeAttribute('data-cursor'); }
  function cursorAt(p) {
    if (S.mode !== 'area' || !S.sel) return setCursor('');
    const h = hit(p);
    setCursor(CUR[h] || (h === 'in' ? (S.tool === 'select' ? 'move' : S.tool === 'text' ? 'text' : 'crosshair') : ''));
  }

  function layoutSel() {
    const s = S.sel;
    if (!s) { selEl.hidden = true; document.body.classList.remove('has-sel'); annEl.style.clipPath = 'none'; return; }
    selEl.hidden = false; document.body.classList.add('has-sel');
    Object.assign(selEl.style, { left: s.x + 'px', top: s.y + 'px', width: s.w + 'px', height: s.h + 'px' });
    sizeEl.textContent = `${Math.round(s.w * rx())} × ${Math.round(s.h * ry())}`;
    sizeEl.classList.toggle('in', s.y < 30);
    annEl.style.clipPath = `inset(${s.y}px ${innerWidth - s.x - s.w}px ${innerHeight - s.y - s.h}px ${s.x}px)`;
  }

  // ---------- pointer ----------
  const pt = (e) => ({ x: clampX(e.clientX), y: clampY(e.clientY) });
  addEventListener('pointerdown', (e) => {
    if (e.button === 2) { e.preventDefault(); return cancel(); } // right click cancels, like Esc
    if (!document.hasFocus()) refocus(); // a click always gives the page the keyboard
    if (e.button !== 0 || e.target.closest('#bar') || e.target === txt) return;
    hintEl.classList.add('off');
    const p = pt(e);
    if (S.mode === 'color') { setPos(e.clientX * rx(), e.clientY * ry()); return pickCommit(); }
    if (S.mode === 'area' && S.sel) {
      const h = hit(p);
      if (h === 'in' && S.tool !== 'select') return startDraw(p);
      if (h && h !== 'in') { drag = { k: 'resize', h, o: { ...S.sel } }; return; }
      if (h === 'in') { drag = { k: 'move', o: { ...S.sel }, p }; return; }
    }
    S.ops = []; S.draft = null; render(); closeText(); barEl.hidden = true;
    drag = { k: 'new', p };
    S.sel = { x: p.x, y: p.y, w: 0, h: 0 };
    layoutSel();
  });
  addEventListener('pointermove', (e) => {
    if (!document.hasFocus() && T && T.window && !S.focusAsked) { S.focusAsked = true; try { T.window.getCurrentWindow().setFocus(); } catch (_) { /* optional */ } }
    if (S.mode === 'color') return setPos(e.clientX * rx(), e.clientY * ry());
    const p = pt(e);
    if (!drag) return cursorAt(p);
    if (drag.k === 'new') S.sel = norm(drag.p, p);
    else if (drag.k === 'move') {
      const o = drag.o;
      S.sel = { x: Math.max(0, Math.min(innerWidth - o.w, o.x + p.x - drag.p.x)), y: Math.max(0, Math.min(innerHeight - o.h, o.y + p.y - drag.p.y)), w: o.w, h: o.h };
    } else if (drag.k === 'resize') {
      const o = drag.o; let l = o.x, r = o.x + o.w, t = o.y, b = o.y + o.h;
      if (drag.h.includes('w')) l = p.x; if (drag.h.includes('e')) r = p.x;
      if (drag.h.includes('n')) t = p.y; if (drag.h.includes('s')) b = p.y;
      S.sel = norm({ x: l, y: t }, { x: r, y: b });
    } else if (drag.k === 'draw') extendDraw(p);
    layoutSel();
  });
  addEventListener('pointerup', () => {
    if (!drag) return;
    const d = drag; drag = null;
    if (d.k === 'draw') return finishDraw();
    if (d.k === 'new') {
      if (!S.sel || S.sel.w < 4 || S.sel.h < 4) { S.sel = null; layoutSel(); return; }
      if (S.mode === 'area') showBar(); else commit({ action: 'run' });
    } else showBar(true);
  });
  addEventListener('dblclick', (e) => { if (S.mode === 'area' && S.sel && S.tool === 'select' && inside(pt(e)) && !e.target.closest('#bar')) commit({ action: 'both' }); });
  addEventListener('contextmenu', (e) => { e.preventDefault(); cancel(); }); // also on mouseup-style right clicks (pointerdown may be swallowed)
  // ---------- markup ----------
  const fp = (p) => ({ x: p.x * rx(), y: p.y * ry() });
  const lw = () => 3 * rx();
  function drawOp(g, o) {
    g.save(); g.lineCap = 'round'; g.lineJoin = 'round'; g.strokeStyle = g.fillStyle = o.c; g.lineWidth = lw();
    if (o.t === 'rect') g.strokeRect(Math.min(o.a.x, o.b.x), Math.min(o.a.y, o.b.y), Math.abs(o.b.x - o.a.x), Math.abs(o.b.y - o.a.y));
    else if (o.t === 'arrow') {
      const a = Math.atan2(o.b.y - o.a.y, o.b.x - o.a.x), n = Math.min(Math.max(14 * rx(), lw() * 4), Math.hypot(o.b.x - o.a.x, o.b.y - o.a.y) * 0.6);
      g.beginPath(); g.moveTo(o.a.x, o.a.y); g.lineTo(o.b.x, o.b.y);
      g.moveTo(o.b.x, o.b.y); g.lineTo(o.b.x - n * Math.cos(a - 0.45), o.b.y - n * Math.sin(a - 0.45));
      g.moveTo(o.b.x, o.b.y); g.lineTo(o.b.x - n * Math.cos(a + 0.45), o.b.y - n * Math.sin(a + 0.45));
      g.stroke();
    } else if (o.t === 'pen') {
      g.beginPath(); o.pts.forEach((q, i) => (i ? g.lineTo(q.x, q.y) : g.moveTo(q.x, q.y)));
      if (o.pts.length === 1) g.lineTo(o.pts[0].x + 0.1, o.pts[0].y);
      g.stroke();
    } else if (o.t === 'blur') { // pixelate: irreversible, unlike a soft blur
      const x = Math.round(Math.min(o.a.x, o.b.x)), y = Math.round(Math.min(o.a.y, o.b.y)), w = Math.round(Math.abs(o.b.x - o.a.x)), h = Math.round(Math.abs(o.b.y - o.a.y));
      if (w > 1 && h > 1) {
        const k = Math.max(6, Math.round(10 * rx())), tmp = document.createElement('canvas');
        tmp.width = Math.max(1, Math.round(w / k)); tmp.height = Math.max(1, Math.round(h / k));
        const t = tmp.getContext('2d'); t.imageSmoothingEnabled = true; t.drawImage(frameEl, x, y, w, h, 0, 0, tmp.width, tmp.height);
        g.imageSmoothingEnabled = false; g.drawImage(tmp, 0, 0, tmp.width, tmp.height, x, y, w, h);
      }
    } else if (o.t === 'text') {
      g.font = `600 ${Math.round(16 * rx())}px "Segoe UI Variable Text","Segoe UI",system-ui,sans-serif`; g.textBaseline = 'top';
      g.lineWidth = Math.max(3, 3 * rx()); g.strokeStyle = 'rgba(0,0,0,.85)'; g.strokeText(o.s, o.a.x, o.a.y); g.fillText(o.s, o.a.x, o.a.y);
    }
    g.restore();
  }
  function render() {
    actx.clearRect(0, 0, annEl.width, annEl.height);
    S.ops.forEach((o) => drawOp(actx, o));
    if (S.draft) drawOp(actx, S.draft);
    const u = barEl.querySelector('[data-a="undo"]'); if (u) u.disabled = !S.ops.length;
  }
  function startDraw(p) {
    if (S.tool === 'text') return openText(p);
    S.draft = S.tool === 'pen' ? { t: 'pen', c: S.color, pts: [fp(p)] } : { t: S.tool, c: S.color, a: fp(p), b: fp(p) };
    drag = { k: 'draw' };
    render();
  }
  function extendDraw(p) {
    if (!S.draft) return;
    const q = fp({ x: Math.max(S.sel.x, Math.min(S.sel.x + S.sel.w, p.x)), y: Math.max(S.sel.y, Math.min(S.sel.y + S.sel.h, p.y)) });
    if (S.draft.t === 'pen') S.draft.pts.push(q); else S.draft.b = q;
    render();
  }
  function finishDraw() {
    const d = S.draft; S.draft = null;
    if (d && (d.t === 'pen' || Math.hypot(d.b.x - d.a.x, d.b.y - d.a.y) > 4)) S.ops.push(d);
    render();
  }
  function openText(p) {
    closeText();
    txt = document.createElement('input');
    txt.id = 'txt'; txt.type = 'text'; txt.style.left = p.x + 'px'; txt.style.top = p.y + 'px'; txt.style.color = S.color; txt.spellcheck = false;
    txt._p = p;
    txt.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); commitText(); } else if (e.key === 'Escape') { e.preventDefault(); closeText(); }
    });
    txt.addEventListener('blur', commitText);
    document.body.append(txt);
    setTimeout(() => txt && txt.focus(), 0);
  }
  function commitText() {
    if (!txt) return;
    const t = txt, s = t.value.trim();
    txt = null; t.remove();
    if (s) { S.ops.push({ t: 'text', c: S.color, s, a: fp(t._p) }); render(); }
  }
  function closeText() { if (txt) { const t = txt; txt = null; t.remove(); } }

  // ---------- toolbar ----------
  const TIPS = {
    select: L('Выбор', 'Select'), rect: L('Рамка', 'Rectangle'), arrow: L('Стрелка', 'Arrow'), pen: L('Карандаш', 'Pen'), blur: L('Скрыть (пиксели)', 'Hide (pixelate)'),
    text: L('Текст', 'Text'), undo: L('Отменить (Ctrl+Z)', 'Undo (Ctrl+Z)'), copy: L('Копировать (Ctrl+C)', 'Copy (Ctrl+C)'), save: L('Сохранить (Ctrl+S)', 'Save (Ctrl+S)'),
    video: L('Записать эту область', 'Record this area'), check: L('Готово: копировать и сохранить (Enter)', 'Done: copy and save (Enter)'), x: L('Отмена (Esc)', 'Cancel (Esc)'),
  };
  function buildBar() {
    const b = (a, i, cls = '') => `<button type="button" data-a="${a}" class="${cls}" title="${TIPS[i || a]}">${ic(i || a)}</button>`;
    barEl.innerHTML = ['select', 'rect', 'arrow', 'pen', 'blur', 'text'].map((t) => b(t, t, t === 'select' ? 'on' : '')).join('')
      + '<span class="sep"></span>' + COLORS.map((c, i) => `<button type="button" class="sw${i ? '' : ' on'}" data-c="${c}" style="--c:${c}"><i></i></button>`).join('')
      + '<span class="sep"></span>' + b('undo') + '<span class="sep"></span>' + b('copy') + b('save') + b('record', 'video') + b('both', 'check', 'go') + b('cancel', 'x', 'x');
    barEl.addEventListener('pointerdown', (e) => e.stopPropagation());
    barEl.addEventListener('click', (e) => {
      const el = e.target.closest('button'); if (!el) return;
      if (el.dataset.c) { S.color = el.dataset.c; barEl.querySelectorAll('.sw').forEach((s) => s.classList.toggle('on', s === el)); return; }
      const a = el.dataset.a;
      if (['select', 'rect', 'arrow', 'pen', 'blur', 'text'].includes(a)) { S.tool = a; closeText(); barEl.querySelectorAll('[data-a]').forEach((x) => x.classList.toggle('on', x.dataset.a === a)); }
      else if (a === 'undo') undo();
      else if (a === 'cancel') cancel();
      else commit({ action: a });
    });
  }
  function showBar(reposition) {
    if (S.mode !== 'area' || !S.sel) return;
    if (!barEl.firstChild) buildBar();
    barEl.hidden = false;
    const s = S.sel, w = barEl.offsetWidth, h = barEl.offsetHeight, g = 10;
    let x = Math.max(8, Math.min(innerWidth - w - 8, s.x + s.w - w)), y = s.y + s.h + g;
    if (y + h > innerHeight - 8) y = s.y - h - g;
    if (y < 8) y = Math.max(8, s.y + s.h - h - g);
    barEl.style.left = x + 'px'; barEl.style.top = y + 'px';
    render();
    return reposition;
  }

  // ---------- actions ----------
  const undo = () => { if (S.ops.length) { S.ops.pop(); render(); } };
  const cancel = () => { closeText(); inv('capture_cancel'); clearAll(); };
  function exportPng() {
    const r = { x: Math.round(S.sel.x * rx()), y: Math.round(S.sel.y * ry()), w: Math.max(1, Math.round(S.sel.w * rx())), h: Math.max(1, Math.round(S.sel.h * ry())) };
    r.w = Math.min(r.w, S.fw - r.x); r.h = Math.min(r.h, S.fh - r.y);
    const c = document.createElement('canvas'); c.width = r.w; c.height = r.h;
    const g = c.getContext('2d');
    g.drawImage(frameEl, r.x, r.y, r.w, r.h, 0, 0, r.w, r.h);
    g.drawImage(annEl, r.x, r.y, r.w, r.h, 0, 0, r.w, r.h);
    return new Promise((res) => c.toBlob((b) => b.arrayBuffer().then(res), 'image/png'));
  }
  async function commit(extra) {
    if (S.busy) return;
    commitText();
    S.busy = true;
    const args = { label, vw: innerWidth, vh: innerHeight, ...extra };
    if (S.sel) args.rect = [S.sel.x, S.sel.y, S.sel.w, S.sel.h];
    if (S.mode === 'area' && S.ops.length && extra.action !== 'record' && S.sel) {
      const png = await exportPng();
      await inv('capture_commit', new Uint8Array(png), { headers: { 'x-pill-args': JSON.stringify(args) } });
    } else await inv('capture_commit', args);
    clearAll();
  }

  // ---------- colour picker ----------
  function setPos(x, y) {
    S.cx = Math.max(0, Math.min(S.fw - 1, Math.floor(x))); S.cy = Math.max(0, Math.min(S.fh - 1, Math.floor(y)));
    drawLens();
  }
  const at = (x, y) => { if (!S.px || x < 0 || y < 0 || x >= S.fw || y >= S.fh) return null; const o = (y * S.fw + x) * 4; return [S.px[o], S.px[o + 1], S.px[o + 2]]; };
  const hex2 = (n) => n.toString(16).toUpperCase().padStart(2, '0');
  function drawLens() {
    const R = 7, N = 2 * R + 1, cell = 135 / N, c = at(S.cx, S.cy);
    if (!c) return;
    for (let j = 0; j < N; j++) for (let i = 0; i < N; i++) {
      const p = at(S.cx - R + i, S.cy - R + j);
      lctx.fillStyle = p ? `rgb(${p[0]},${p[1]},${p[2]})` : '#111';
      lctx.fillRect(i * cell, j * cell, Math.ceil(cell), Math.ceil(cell));
    }
    lctx.strokeStyle = 'rgba(0,0,0,.16)'; lctx.lineWidth = 1; lctx.beginPath();
    for (let k = 1; k < N; k++) { lctx.moveTo(k * cell, 0); lctx.lineTo(k * cell, 135); lctx.moveTo(0, k * cell); lctx.lineTo(135, k * cell); }
    lctx.stroke();
    lctx.lineWidth = 3; lctx.strokeStyle = '#000'; lctx.strokeRect(R * cell + 1, R * cell + 1, cell - 2, cell - 2);
    lctx.lineWidth = 1.5; lctx.strokeStyle = '#fff'; lctx.strokeRect(R * cell + 1, R * cell + 1, cell - 2, cell - 2);
    const hx = '#' + hex2(c[0]) + hex2(c[1]) + hex2(c[2]);
    $('#lenssw').style.background = hx; $('#lenshex').textContent = hx; $('#lensrgb').textContent = `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
    $('#lenskeys').textContent = `x ${S.cx}  y ${S.cy}`;
    const px = (S.cx + 0.5) / rx(), py = (S.cy + 0.5) / ry();
    pix.style.left = px + 'px'; pix.style.top = py + 'px';
    const w = lensEl.offsetWidth || 147, h = lensEl.offsetHeight || 215, o = 26;
    lensEl.style.left = (px + o + w > innerWidth ? px - o - w : px + o) + 'px';
    lensEl.style.top = (py + o + h > innerHeight ? py - o - h : py + o) + 'px';
  }
  function pickCommit() {
    if (S.busy) return;
    S.busy = true;
    inv('capture_commit', { label, vw: innerWidth, vh: innerHeight, x: (S.cx + 0.5) / rx(), y: (S.cy + 0.5) / ry(), action: 'pick' }).then(clearAll);
  }

  // ---------- keyboard ----------
  addEventListener('keydown', (e) => {
    if (txt) return;
    const k = e.key;
    if (k === 'Escape') { e.preventDefault(); return cancel(); }
    if (S.mode === 'color') {
      const st = e.shiftKey ? 10 : 1;
      if (k === 'ArrowLeft') setPos(S.cx - st, S.cy); else if (k === 'ArrowRight') setPos(S.cx + st, S.cy);
      else if (k === 'ArrowUp') setPos(S.cx, S.cy - st); else if (k === 'ArrowDown') setPos(S.cx, S.cy + st);
      else if (k === 'Enter') return pickCommit();
      else return;
      return e.preventDefault();
    }
    if (k === 'Enter') { e.preventDefault(); if (S.mode === 'area') commit({ action: 'both' }); else commit({ action: 'run' }); }
    else if (S.mode === 'area' && (e.ctrlKey || e.metaKey)) {
      const lk = k.toLowerCase();
      if (lk === 'z') { e.preventDefault(); undo(); } else if (lk === 'c' && S.sel) { e.preventDefault(); commit({ action: 'copy' }); } else if (lk === 's' && S.sel) { e.preventDefault(); commit({ action: 'save' }); }
    }
  });

  if (T && T.event) {
    T.event.listen('capture:begin', () => arm());
    T.event.listen('capture:end', () => clearAll());
  }
  addEventListener('resize', () => { layoutSel(); if (S.mode === 'area' && !barEl.hidden) showBar(); if (S.mode === 'color' && S.px) drawLens(); });
  window.__capture = { S, arm, commit }; // harness / debugging
  arm();
})();
