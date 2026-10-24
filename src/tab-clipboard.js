// Clipboard tab (history, search, pins, click-to-paste). Backend: clipboard.rs (lane L3a); UI restyled by the pill agent.
// Commands: clip_list clip_image clip_paste clip_pin clip_delete clip_wipe clip_pause clip_status, pill_activate (pill.js).
// Events: clip:added (item), clip:open (hotkey -> Pill.expand('clipboard')).
(() => {
  const P = window.Pill, t = P.t, ic = P.icon;
  const inv = (cmd, args) => P.invoke(cmd, args);
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const ago = (ts) => {
    const m = Math.floor((Date.now() - ts) / 60000);
    return m < 1 ? t({ ru: 'сейчас', en: 'now' }) : m < 60 ? m + t({ ru: ' мин', en: ' min' }) : m < 1440 ? Math.floor(m / 60) + t({ ru: ' ч', en: ' h' }) : Math.floor(m / 1440) + t({ ru: ' д', en: ' d' });
  };

  let root, listEl, searchEl, wipeBtn, privChip;
  let items = [], shown = false, paused = false, wipeArm = 0, timer = 0, sel = 0, enter = false, fresh = false, hadRows = false;
  const PM = window.PillMotion; // exits / FLIP of rebuilt rows
  const imgCache = new Map(); // id -> data URL

  function mount(el) {
    root = el;
    root.classList.add('clip');
    root.innerHTML = `
      <div class="clip-bar">
        <label class="clip-search">${ic('search', { size: 16 })}<input class="clip-q" type="search" placeholder="${esc(t({ ru: 'Поиск по истории', en: 'Search history' }))}" autocomplete="off" spellcheck="false" /></label>
        <button class="clip-wipe" type="button" title="${esc(t({ ru: 'Очистить историю', en: 'Clear history' }))}">${ic('trash', { size: 17 })}</button>
      </div>
      <div class="clip-list scroll"></div>
      <div class="clip-foot"><span class="clip-hint">${esc(t({ ru: 'Клик — вставить · Shift+клик — только копировать', en: 'Click to paste · Shift+click to copy only' }))}</span><span class="clip-priv" hidden>${esc(t({ ru: 'Приватный режим: история не пишется', en: 'Private mode: nothing is recorded' }))}</span><span class="clip-keys">${ic('updown', { size: 12, sw: 1.8 })} Enter</span></div>`;
    listEl = root.querySelector('.clip-list');
    searchEl = root.querySelector('.clip-q');
    wipeBtn = root.querySelector('.clip-wipe');
    privChip = root.querySelector('.clip-priv');
    searchEl.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(load, 150); });
    root.addEventListener('keydown', onKey);
    wipeBtn.addEventListener('click', async () => {
      if (!wipeArm) { // confirm step: second click within 3 s
        wipeBtn.classList.add('armed');
        wipeBtn.title = t({ ru: 'Нажми ещё раз: удалить всю историю', en: 'Click again to wipe everything' });
        wipeArm = setTimeout(disarm, 3000);
        return;
      }
      disarm();
      const rows = [...listEl.querySelectorAll('.clip-row')];
      const gone = Promise.all(rows.map((r, i) => PM.exit(r, { y: -4, delay: Math.min(i, 8) * 0.02 })));
      await inv('clip_wipe');
      imgCache.clear();
      searchEl.value = ''; sel = 0;
      await gone;
      await load();
      P.toast({ id: 'clip-wipe', icon: 'trash', ttlMs: 1800, title: t({ ru: 'Буфер очищен', en: 'Clipboard cleared' }) });
    });
    listEl.addEventListener('click', onClick);
    listEl.addEventListener('mousemove', (e) => { const r = e.target.closest('.clip-row'); if (r && +r.dataset.i !== sel) setSel(+r.dataset.i, false); });
  }

  function disarm() { clearTimeout(wipeArm); wipeArm = 0; wipeBtn.classList.remove('armed'); wipeBtn.title = t({ ru: 'Очистить историю', en: 'Clear history' }); }

  async function load() {
    if (!listEl) return;
    const res = await inv('clip_list', { query: searchEl.value.trim() || null, kind: null, limit: 100, offset: 0 });
    items = Array.isArray(res) ? res : [];
    sel = Math.min(sel, Math.max(0, items.length - 1));
    render();
  }

  function setSel(i, scroll = true) {
    sel = Math.max(0, Math.min(items.length - 1, i));
    listEl.querySelectorAll('.clip-row').forEach((r) => r.classList.toggle('sel', +r.dataset.i === sel));
    if (scroll) listEl.querySelector('.clip-row.sel')?.scrollIntoView({ block: 'nearest' });
  }

  function tile(it) {
    if (it.kind === 'image') return `<span class="tile"><img class="clip-thumb" data-img="${it.id}" alt="" /></span>`;
    if (it.kind === 'files') return `<span class="tile">${ic('folder', { size: 17, sw: 1.6 })}</span>`;
    return `<span class="tile clip-t">${ic('text', { size: 18, sw: 1.5 })}</span>`;
  }
  function title(it) {
    if (it.kind === 'image') return t({ ru: 'Изображение', en: 'Image' });
    if (it.kind === 'files') {
      const f = String(it.preview || '').split('\n').filter(Boolean).map((p) => p.split(/[\\/]/).pop());
      return f.slice(0, 2).join(', ') + (f.length > 2 || it.fileCount > 2 ? ' +' + ((it.fileCount || f.length) - 2) : '');
    }
    return String(it.preview || '').replace(/\s+/g, ' ').trim() + (it.truncated ? '…' : '');
  }

  function fileCount(it) {
    const n = it.fileCount || String(it.preview || '').split('\n').filter(Boolean).length || 1;
    return n + t({ ru: n % 10 === 1 && n % 100 !== 11 ? ' файл' : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14) ? ' файла' : ' файлов', en: n === 1 ? ' file' : ' files' });
  }

  function render() {
    if (!items.length) {
      const q = searchEl.value.trim();
      listEl.innerHTML = `<div class="clip-empty">${esc(q ? t({ ru: 'Ничего не найдено', en: 'Nothing found' }) : paused ? t({ ru: 'Приватный режим: ничего не записывается', en: 'Private mode is on: nothing is recorded' }) : t({ ru: 'Скопированный текст, картинки и файлы появятся здесь', en: 'Copied text, images and files will show up here' }))}</div>`;
      if (hadRows || enter) PM.enter(listEl.firstChild, { y: 6 }); // rows -> empty state (not on every keystroke inside "nothing found")
      hadRows = false; enter = false; fresh = false;
      return;
    }
    const fill = () => { listEl.innerHTML = items.map((it, i) => `
      <div class="list-row clip-row${enter ? ' stag' : ''}${i === sel ? ' sel' : ''}${it.pinned ? ' pinned' : ''}" style="--i:${i}" data-id="${it.id}" data-i="${i}">
        ${tile(it)}
        <div class="clip-body"><div class="t">${esc(title(it))}</div>
          <div class="s">${it.pinned ? `<span class="clip-pin">${ic('pin', { size: 11, fill: true, sw: 1.4 })}</span>` : ''}${it.kind === 'files' ? esc(fileCount(it)) + ' · ' : ''}${esc(String(it.source || '').replace(/\.exe$/i, ''))}${it.source ? ' · ' : ''}${esc(ago(it.usedAt))}</div></div>
        ${it.kind === 'image' ? '' : `<button class="clip-act clip-plain" data-act="plain" title="${esc(t({ ru: 'Вставить как простой текст', en: 'Paste as plain text' }))}">${ic('text', { size: 14, sw: 1.9 })}</button>`}
        ${it.kind === 'text' ? `<button class="clip-act" data-act="ai" title="${esc(t({ ru: 'ИИ-действия', en: 'AI actions' }))}">${ic('sparkles', { size: 14, sw: 1.7 })}</button>` : ''}
        <button class="clip-act" data-act="pin" title="${esc(it.pinned ? t({ ru: 'Открепить', en: 'Unpin' }) : t({ ru: 'Закрепить', en: 'Pin' }))}">${ic('pin', { size: 14, fill: !!it.pinned, sw: 1.6 })}</button>
        <button class="clip-act" data-act="del" title="${esc(t({ ru: 'Удалить', en: 'Delete' }))}">${ic('x', { size: 14, sw: 1.9 })}</button>
      </div>`).join(''); };
    // first render of a visit: the CSS stagger; later renders: rows that stay slide to their new spot (pin / delete), a copied item drops in
    if (enter) fill(); else PM.flip(listEl, fill, { sel: '.clip-row', enterNew: fresh });
    enter = false; fresh = false; hadRows = true;
    listEl.querySelectorAll('img[data-img]').forEach(async (img) => {
      const id = +img.dataset.img;
      if (!imgCache.has(id)) imgCache.set(id, await inv('clip_image', { id }));
      if (imgCache.get(id)) img.src = imgCache.get(id);
    });
  }

  // Click / Enter pastes into the window that had focus and CLOSES the island (video 15.93 s); Shift+click copies only and
  // stays open; the "T" button pastes as plain text. Setting `clipClickPaste` = false turns a plain click into copy-only.
  // clip_paste(id, copyOnly, plain) -> pasted | copied_only (elevated target) | copied (copy only) | gone
  async function activate(id, shift, plain) {
    const it = items.find((i) => i.id === id);
    if (!it) return;
    if (document.activeElement === searchEl) { searchEl.blur(); await sleep(120); } // let pill_activate(false) hand focus back first
    const copyOnly = !!shift || (P.settings && P.settings.clipClickPaste === false);
    const r = await inv('clip_paste', { id, copyOnly, plain: !!plain });
    P.toast({ id: 'clip-paste', icon: copyOnly ? 'copy' : 'clipboard', ttlMs: 1800,
      title: r === 'pasted' ? t({ ru: 'Вставлено', en: 'Pasted' }) : r === 'copied' ? t({ ru: 'Скопировано', en: 'Copied' })
        : r === 'copied_only' ? t({ ru: 'Скопировано (цель с правами админа)', en: 'Copied (target is elevated)' }) : t({ ru: 'Записи уже нет', en: 'Item is gone' }) });
    if (r === 'pasted' || r === 'copied_only') P.collapse('click'); // a click that closes the island squashes deeper (measured)
    load();
  }

  async function onClick(e) {
    const row = e.target.closest('.clip-row');
    if (!row) return;
    const id = +row.dataset.id, act = e.target.closest('[data-act]')?.dataset.act;
    if (act === 'pin') { await inv('clip_pin', { id, pinned: !items.find((i) => i.id === id)?.pinned }); return load(); }
    if (act === 'del') { imgCache.delete(id); const gone = PM.collapse(row, { remove: true }); await inv('clip_delete', { id }); await gone; return load(); }
    if (act === 'plain') return activate(id, false, true);
    if (act === 'ai') return window.PillClipAI && window.PillClipAI.open({ root, id });
    activate(id, e.shiftKey);
  }

  // Up/Down/Enter work while the search box has the keyboard; with the window unfocused nothing arrives at all. Typing a
  // character (or "/") anywhere in the tab hands the keyboard to the search box (pill.js activates the window on focusin).
  function onKey(e) {
    if (!shown || (window.PillClipAI && window.PillClipAI.isOpen())) return; // the AI panel owns the keys while it is open
    if (e.key === 'Escape') { if (searchEl.value) { e.stopImmediatePropagation(); searchEl.value = ''; load(); } return; } // first Esc clears the query, the next one closes the island
    if (!items.length) return;
    if (e.key === 'ArrowDown') { e.preventDefault(); setSel(sel + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); setSel(sel - 1); }
    else if (e.key === 'Enter') { e.preventDefault(); activate(items[sel].id, e.shiftKey); }
  }
  document.addEventListener('keydown', (e) => {
    if (!shown || !searchEl || e.defaultPrevented || (window.PillClipAI && window.PillClipAI.isOpen()) || e.ctrlKey || e.altKey || e.metaKey) return;
    if (document.activeElement === searchEl || (document.activeElement && document.activeElement.matches && document.activeElement.matches('input, textarea, [contenteditable]'))) return;
    if (e.key === '/') { e.preventDefault(); searchEl.focus(); }
    else if (e.key.length === 1) searchEl.focus(); // the character itself still lands in the freshly focused field
  });

  function refreshSoon() { if (shown) { clearTimeout(timer); fresh = true; timer = setTimeout(load, 120); } }
  const paintPriv = () => { if (privChip) { privChip.hidden = !paused; root.classList.toggle('is-priv', paused); } };

  async function onShow() {
    shown = true; sel = 0; enter = true;
    const s = await inv('clip_status');
    if (s) { paused = !!s.paused; paintPriv(); }
    load();
  }

  function onHide() { shown = false; disarm(); if (window.PillClipAI) window.PillClipAI.close(); }

  P.register({ id: 'clipboard', title: t({ ru: 'Буфер', en: 'Clipboard' }), icon: ic('clipboard'), order: 10, mount, onShow, onHide });
  // the private-mode switch lives in the Settings tab; it tells us through this hook
  P.clipPaused = (v) => { paused = !!v; paintPriv(); };
  const ev = window.__TAURI__ && window.__TAURI__.event;
  if (ev) {
    ev.listen('clip:added', refreshSoon);
    // hotkey: open the tab and put the caret in the search box (the hotkey means "find something to paste")
    ev.listen('clip:open', () => { P.expand('clipboard'); setTimeout(() => { if (shown && searchEl) searchEl.focus(); }, 320); });
  }
})();
