// Capture lane, pill side: the recording state of the collapsed capsule, the Home "Record" tile turning into "Stop 0:NN",
// and the result cards (screenshot, screen text, QR, colour, recording, errors). Owner: capture agent.
// User request: "screen capture, OCR, QR, colour picker and recorder for the Pill replica". Rust side: src-tauri/src/capture.rs.
// Events from Rust: rec:state {recording, startedAt, mode}, capture:done {kind, path?, text?, ...}.
// Exact look of the capsule from the video notes: 258 px wide, red dot 10 px pulsing 2.0 s (opacity 1 <-> .5), white m:ss timer
// that changes once a second, grey MP4, area icon, red Stop button 56x24 on rgba(255,67,63,.20) (.30 on hover).
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const t = P.t, ic = P.icon;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const base = (p) => String(p || '').split(/[\\/]/).pop();
  const cut = (s, n) => { s = String(s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n - 1) + '…' : s; };
  const inv = (c, a) => P.invoke(c, a);
  const act = (action, value) => inv('capture_action', { action, value });

  let rec = null, timer = 0, tileHtml = null, hid = false;

  // m:ss (no leading zero on minutes), h:mm:ss after an hour
  const clock = (ms) => {
    const s = Math.max(0, Math.floor(ms / 1000)), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = String(s % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${r}` : `${m}:${r}`;
  };
  const dur = (sec) => clock(sec * 1000);

  function capsule() {
    const stop = t({ ru: 'Стоп', en: 'Stop' });
    return `<div class="lp lp-rec" title="${esc(t({ ru: 'Идёт запись экрана', en: 'Recording the screen' }))}"><span class="rec-dot"></span><span class="rec-t">${clock(Date.now() - rec.startedAt)}</span>`
      + `<span class="rec-sp"></span><span class="rec-fmt">${P.settings && P.settings['capture.format'] === 'gif' ? 'GIF' : 'MP4'}</span><span class="rec-area">${ic('area', { size: 12, sw: 1.8 })}</span>`
      + `<button type="button" class="rec-stop" data-rec-stop title="${esc(t({ ru: 'Остановить запись (Ctrl+Alt+R)', en: 'Stop recording (Ctrl+Alt+R)' }))}"><i></i><span>${esc(stop)}</span></button></div>`;
  }

  function tick() {
    if (!rec) return;
    const s = clock(Date.now() - rec.startedAt);
    document.querySelectorAll('.rec-t').forEach((e) => { if (e.textContent !== s) e.textContent = s; });
  }
  const arm = () => { clearInterval(timer); timer = rec && !hid ? setInterval(tick, 250) : 0; }; // no ticks while the island is hidden (nobody sees the text)

  // The Home "Record screen" tile becomes a red "Stop 0:NN" tile while recording.
  function syncTile() {
    const b = document.querySelector('.th-tile[data-tile="record"]');
    if (!b) return false;
    if (rec && !b.classList.contains('cl-rec')) {
      tileHtml = b.innerHTML;
      b.classList.add('cl-rec');
      b.innerHTML = `<i class="cl-sq"></i><span class="cl-tt"><b>${esc(t({ ru: 'Стоп', en: 'Stop' }))}</b><em class="rec-t">${clock(Date.now() - rec.startedAt)}</em></span>`;
    } else if (!rec && b.classList.contains('cl-rec')) {
      b.classList.remove('cl-rec');
      if (tileHtml != null) b.innerHTML = tileHtml;
    }
    return true;
  }
  new MutationObserver(() => { if (rec) syncTile(); }).observe(document.body, { childList: true, subtree: true });

  function recStart(startedAt) {
    rec = { startedAt: +startedAt || Date.now() };
    if (P.liveOn('recording')) P.setLive('rec', { priority: 100, html: capsule(), ttlMs: null }); // Settings > Live events > Screen recording
    arm();
    syncTile();
  }
  function recStop() {
    rec = null;
    clearInterval(timer);
    P.setLive('rec', null);
    syncTile();
  }
  function onRec(p) {
    if (p && p.recording) recStart(p.startedAt); else recStop();
  }

  // The Stop button lives in HTML that the shell re-renders: handle clicks by delegation, before the pill sees them.
  document.addEventListener('click', (e) => {
    const b = e.target.closest && e.target.closest('[data-rec-stop]');
    if (!b) return;
    e.stopPropagation();
    inv('capture_stop');
  }, true);

  // ---------- result cards ----------
  const SQ = 'monitor';
  function card(p) {
    if (!p || !p.kind) return;
    const id = 'cap';
    switch (p.kind) {
      case 'screenshot': {
        const saved = !!p.path;
        if (saved) P.react('capture', 'file', { ttlMs: 3000 });
        const actions = [];
        if (saved) {
          actions.push({ label: t({ ru: 'Открыть', en: 'Open' }), kind: 'white', run: () => act('open', p.path) });
          actions.push({ label: t({ ru: 'В папке', en: 'Show in folder' }), run: () => act('reveal', p.path) });
          actions.push({ label: t({ ru: 'Копировать', en: 'Copy' }), run: () => act('copy_image', p.path) });
        }
        P.toast({
          id, icon: SQ, tone: 'green', ttlMs: 7000, actions,
          title: saved ? t({ ru: 'Скриншот сохранён', en: 'Screenshot saved' }) : t({ ru: 'Скриншот скопирован', en: 'Screenshot copied' }),
          sub: saved ? base(p.path) : `${p.w}×${p.h}`,
        });
        break;
      }
      case 'text':
        P.toast(p.found
          ? { id, icon: 'scantext', tone: 'green', ttlMs: 6000, title: t({ ru: 'Текст скопирован', en: 'Text copied' }), sub: cut(p.text, 72) + (p.chars > 72 ? ` · ${p.chars}` : '') }
          : { id, icon: 'scantext', tone: 'amber', ttlMs: 4500, title: t({ ru: 'Текст не найден', en: 'No text found' }), sub: t({ ru: 'Попробуй выделить область покрупнее', en: 'Try a larger area' }) });
        break;
      case 'qr':
        if (!p.found) { P.toast({ id, icon: 'qr', tone: 'amber', ttlMs: 4500, title: t({ ru: 'QR-код не найден', en: 'No QR code found' }), sub: t({ ru: 'Выдели код целиком, с белой рамкой', en: 'Select the whole code, with its white border' }) }); break; }
        P.toast({
          id, icon: 'qr', tone: 'accent', ttlMs: 12000, title: t({ ru: 'QR-код', en: 'QR code' }), sub: cut(p.text, 90),
          actions: [
            ...(p.isUrl ? [{ label: t({ ru: 'Открыть ссылку', en: 'Open link' }), kind: 'white', run: () => act('open_url', p.text) }] : []),
            { label: t({ ru: 'Копировать', en: 'Copy' }), run: () => act('copy_text', p.text) },
          ],
        });
        break;
      case 'color': {
        const rgb = (p.rgb || []).join(', '), hist = (p.history || []).filter((h) => h !== p.hex).slice(0, 9);
        P.toast({
          id, ttlMs: 9000,
          html: `<span class="cl-card"><span class="cl-big" style="background:${esc(p.hex)}"></span><span class="cl-t"><b>${esc(p.hex)}</b><i>rgb(${esc(rgb)}) · ${esc(t({ ru: 'скопировано', en: 'copied' }))}</i></span>`
            + (hist.length ? `<span class="cl-hist" title="${esc(t({ ru: 'Недавние цвета: клик копирует', en: 'Recent colours: click to copy' }))}">${hist.map((h) => `<button type="button" class="cl-sw" data-hex="${esc(h)}" title="${esc(h)}" style="background:${esc(h)}"></button>`).join('')}</span>` : '') + '</span>',
        });
        break;
      }
      case 'record':
        if (p.path) P.react('capture', 'file', { ttlMs: 3000 });
        P.toast({
          id, icon: 'video', tone: 'green', ttlMs: 9000, title: t({ ru: 'Запись сохранена', en: 'Recording saved' }), sub: `${base(p.path)} · ${dur(p.seconds || 0)}`,
          actions: [
            { label: t({ ru: 'Открыть', en: 'Open' }), kind: 'white', run: () => act('open', p.path) },
            { label: t({ ru: 'В папке', en: 'Show in folder' }), run: () => act('reveal', p.path) },
          ],
        });
        break;
      case 'error':
        P.toast({ id, icon: 'x', tone: 'red', ttlMs: 10000, title: t({ ru: 'Не получилось', en: 'Capture failed' }), sub: cut(p.message, 180) });
        break;
      default:
    }
  }

  // colour history swatches inside the card: click copies that colour
  document.addEventListener('click', (e) => {
    const s = e.target.closest && e.target.closest('.cl-sw[data-hex]');
    if (!s) return;
    e.stopPropagation();
    act('copy_text', s.dataset.hex);
    s.classList.add('did'); setTimeout(() => s.classList.remove('did'), 400);
  }, true);

  // ---------- public bits used by the Home tiles ----------
  // Tiles collapse the pill first so it is not part of the frozen frame, then ask Rust to start. record_screen toggles.
  window.PillCapture = {
    recording: () => !!rec,
    start(mode) {
      if (mode === 'record_screen' && rec) return inv('capture_start', { mode });
      P.collapse();
      setTimeout(() => inv('capture_start', { mode }), 450);
    },
  };

  T.event.listen('pill:hide', () => { hid = true; arm(); });
  T.event.listen('pill:show', () => { hid = false; tick(); arm(); });
  T.event.listen('rec:state', (e) => onRec(e.payload));
  T.event.listen('settings:changed', () => { if (rec) P.setLive('rec', P.liveOn('recording') ? { priority: 100, html: capsule(), ttlMs: null } : null); }); // the switch works while a recording runs
  T.event.listen('capture:done', (e) => card(e.payload));
  // a page reload while recording: restore the capsule
  inv('capture_settings').then((s) => { if (s && s.recording && s.recStartedAt) recStart(s.recStartedAt); });
})();
