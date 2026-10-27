// Shelf: drop target on the folded island (a file drag near the island opens a compact "drop to keep" card, the drop lands on the shelf),
// shelf tab (file tiles, native drag-out), count badge on the folded island, live "N files" widget. Backend: shelf.rs (lane L3b); UI restyled by the pill agent.
// How a drag from another app reaches this window: pill.rs (cursor loop) drops the click-through style while a drag that started elsewhere is inside
// the drop zone near the island, so OLE delivers it and Tauri raises tauri://drag-enter | drag-leave | drag-drop (files only: CF_HDROP).
(() => {
  const P = window.Pill, T = window.__TAURI__, t = P.t, ic = P.icon;
  const inv = (c, a) => P.invoke(c, a);
  let items = [];
  let root = null, zone = null, list = null, headBar = null, dragAll = null, clearBtn = null;
  let dragOver = false, outDrag = false, sel = new Set(), enter = false;
  let peek = null;      // the compact drop card is up: { dwell, safety, landed }
  let lastDrop = null;  // { x, y, t } where the last drop happened (css px), the tiles fly in from there
  let known = new Set(); // ids already drawn in the grid (a chip that is not in it is new: it flies in)

  const el = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  };
  const shelfOn = () => !P.moduleOn || P.moduleOn('module.shelf'); // Settings > Modules > File shelf
  const liveOn = () => !P.liveOn || P.liveOn('shelf');             // Settings > Live events > File shelf (badge + the "N files" widget)
  const cssVar = (n, fb) => getComputedStyle(document.documentElement).getPropertyValue(n).trim() || fb;
  const zoom = () => parseFloat(cssVar('--s', '1')) || 1;          // #live / #panel are CSS-zoomed by the user size
  const still = () => !window.PillMotion || window.PillMotion.off(); // reduced motion / hidden document: no animation
  const files = (n) => (P.lang === 'ru' ? (n % 10 === 1 && n % 100 !== 11 ? 'файл' : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 12 || n % 100 > 14) ? 'файла' : 'файлов') : n === 1 ? 'file' : 'files');

  // file-type tile colours (own palette)
  const EXT = {
    pdf: '#e8494f', png: '#3b9aff', jpg: '#3b9aff', jpeg: '#3b9aff', gif: '#3b9aff', webp: '#3b9aff', bmp: '#3b9aff', svg: '#3b9aff',
    txt: '#8b8d98', md: '#8b8d98', log: '#8b8d98', doc: '#3b6fe0', docx: '#3b6fe0', xls: '#2ea664', xlsx: '#2ea664', csv: '#2ea664',
    ppt: '#e8793a', pptx: '#e8793a', zip: '#d9a21b', rar: '#d9a21b', '7z': '#d9a21b', mp3: '#a65be8', wav: '#a65be8', flac: '#a65be8',
    mp4: '#e85b9a', mkv: '#e85b9a', mov: '#e85b9a', avi: '#e85b9a', exe: '#6b7280', msi: '#6b7280',
  };
  const IMG = /^(png|jpe?g|gif|webp|bmp)$/;
  const extOf = (n) => { const m = /\.([^.\\/]{1,5})$/.exec(n || ''); return m ? m[1].toLowerCase() : ''; };
  const size = (b) => (b < 1024 ? b + ' B' : b < 1048576 ? (b / 1024).toFixed(b < 10240 ? 1 : 0) + ' KB' : (b / 1048576).toFixed(1) + ' MB');

  // ---------------------------------------------------------------- count badge on the folded island
  // A small chip hanging on the right end of the capsule while the shelf is not empty (pill.css hides it while the island is open).
  const badge = el('div', 'sh-badge');
  badge.innerHTML = ic('shelf', { size: 12, sw: 2 }) + '<b></b>';
  const bodyEl = document.getElementById('body');
  if (bodyEl) bodyEl.append(badge);
  function paintBadge(n, pop) {
    const on = n > 0 && shelfOn() && liveOn();
    badge.classList.toggle('on', on);
    badge.querySelector('b').textContent = n > 99 ? '99+' : String(n);
    badge.title = t({ ru: 'На полке: ', en: 'On the shelf: ' }) + n;
    if (pop && on) { badge.classList.remove('pop'); void badge.offsetWidth; badge.classList.add('pop'); }
  }

  let seen = -1;
  function setItems(v) {
    const was = seen;
    items = v || [];
    sel = new Set([...sel].filter((id) => items.some((i) => i.id === id)));
    const n = seen = items.length;
    if (was >= 0 && n > was) P.react('shelf', 'file', { ttlMs: 3000 }); // item added (not on first load, not on removal)
    P.setBadge('shelf', n);
    paintBadge(n, was >= 0 && n !== was);
    // Transient: a permanent widget would hide the clock for good (the badge above is the permanent part). Shown for 5 s when the count changes (not on first load).
    P.setLive('shelf', n && was >= 0 && n !== was && shelfOn() && liveOn() && !(peek && peek.landed) // (the drop card has just said it)
      ? { priority: 30, ttlMs: 5000, html: `<div class="lp" style="--lpw:140px"><span class="lp-ico">${ic('shelf', { size: 18, sw: 1.6 })}</span><span class="lp-count">${n}</span></div>` } : null);
    render(was >= 0 && n > was);
  }

  function render(added) {
    if (!root) return;
    zone.classList.toggle('over', dragOver);
    zone.classList.toggle('empty', !items.length);
    headBar.hidden = !items.length;
    const lab = dragAll.querySelector('b'), txt = t({ ru: 'Перетащить все', en: 'Drag all' }) + ' (' + items.length + ')';
    if (lab.textContent !== txt) { lab.textContent = txt; if (added) { lab.classList.remove('bump'); void lab.offsetWidth; lab.classList.add('bump'); } }
    const fill = () => list.replaceChildren(...items.map(chip));
    if (enter) fill(); else PM.flip(list, fill); // chips that stay slide into the freed spot, no stagger replay
    // a chip that was not on the grid before flies in from the drop point and the zone gives a "landed" pulse
    const fresh = enter || !known.size || !root.classList.contains('on') || P.state !== 'expanded' ? [] : [...list.children].filter((c) => !known.has(+c.dataset.id)); // only while the tab is on screen
    known = new Set(items.map((i) => i.id));
    enter = false;
    fresh.forEach((c, k) => flyIn(c, k));
    if (fresh.length && !still()) { zone.classList.remove('landed'); void zone.offsetWidth; zone.classList.add('landed'); setTimeout(() => zone.classList.remove('landed'), 800); }
  }

  function flyIn(c, k) {
    c.classList.add('fresh');
    setTimeout(() => c.classList.remove('fresh'), 1600);
    if (still() || !c.animate) return;
    const r = c.getBoundingClientRect(), s = zoom(), d = lastDrop && Date.now() - lastDrop.t < 6000 ? lastDrop : { x: r.left + r.width / 2, y: -30 };
    const dx = (d.x - (r.left + r.width / 2)) / s, dy = (d.y - (r.top + r.height / 2)) / s;
    c.animate([
      { transform: `translate(${dx}px, ${dy}px) scale(.3) rotate(${k % 2 ? 8 : -8}deg)`, opacity: 0 },
      { opacity: 1, offset: 0.22 },
      { transform: 'none', opacity: 1 },
    ], { duration: 700, delay: Math.min(k, 6) * 60, easing: cssVar('--spring-pop', 'cubic-bezier(.34, 1.56, .64, 1)'), fill: 'backwards' });
  }

  // Remove with an exit: the chip shrinks away first, the backend call follows, shelf:changed then rebuilds the grid (FLIP closes the gap).
  const PM = window.PillMotion;
  function removeChips(ids, cmd, args) {
    const chips = ids.map((id) => list.querySelector(`.sh-chip[data-id="${id}"]`)).filter(Boolean);
    const gone = Promise.all(chips.map((c, i) => PM.exit(c, { scale: 0.8, delay: Math.min(i, 8) * 0.02, after: (e) => { e.style.visibility = 'hidden'; } })));
    return gone.then(() => { setTimeout(() => chips.forEach((c) => { if (c.isConnected) c.style.visibility = ''; }), 900); return inv(cmd, args); }); // never leave an invisible chip if the call fails
  }

  function tile(it) {
    const ext = extOf(it.name);
    if (it.is_dir) { const d = el('div', 'sh-ico sh-folder'); d.innerHTML = ic('folder', { size: 36, sw: 1.5, fill: true }); return d; }
    if (it.icon && (IMG.test(ext) || !EXT[ext])) { const img = el('img', 'sh-ico sh-img'); img.src = it.icon; img.draggable = false; return img; }
    const d = el('div', 'sh-ico sh-doc');
    d.style.setProperty('--c', EXT[ext] || '#7c8090');
    d.append(el('span', null, (ext || 'file').toUpperCase().slice(0, 4)));
    return d;
  }

  function chip(it, idx) {
    const c = el('div', 'sh-chip' + (enter ? ' stag' : '') + (it.missing ? ' missing' : '') + (sel.has(it.id) ? ' sel' : ''));
    c.title = it.missing ? it.path + '\n' + t({ ru: '(файл не найден)', en: '(file not found)' }) : it.path;
    c.dataset.id = it.id;
    c.style.setProperty('--i', idx);
    c.append(tile(it));
    c.append(el('div', 'sh-name', it.name));
    c.append(el('div', 'sh-sub', it.missing ? t({ ru: 'не найден', en: 'missing' }) : it.size != null ? (extOf(it.name) || 'file').toUpperCase().slice(0, 4) + ' · ' + size(it.size) : it.is_dir ? t({ ru: 'Папка', en: 'Folder' }) : (extOf(it.name) || 'file').toUpperCase()));
    const x = el('button', 'sh-x');
    x.innerHTML = ic('x', { size: 11, sw: 2.2 });
    x.title = t({ ru: 'Убрать с полки', en: 'Remove from shelf' });
    x.onmousedown = (e) => e.stopPropagation();
    x.onclick = (e) => { e.stopPropagation(); removeChips([it.id], 'shelf_remove', { id: it.id }); };
    c.append(x);
    c.ondblclick = () => inv('shelf_open', { id: it.id });
    c.oncontextmenu = (e) => { e.preventDefault(); menu(it, e.clientX, e.clientY); };
    c.onmousedown = (e) => startDrag(e, it, c);
    return c;
  }

  // Native drag-out: press on a tile and move past a threshold while the button is still down.
  function startDrag(e, it, c, all) {
    if (e.button !== 0 || (it && it.missing)) return;
    if (!all && e.ctrlKey) { sel.has(it.id) ? sel.delete(it.id) : sel.add(it.id); c.classList.toggle('sel'); return; }
    const x0 = e.clientX, y0 = e.clientY;
    const move = (m) => {
      if (Math.hypot(m.clientX - x0, m.clientY - y0) < 6) return;
      off();
      const ids = all ? items.filter((i) => !i.missing).map((i) => i.id) : sel.has(it.id) ? [...sel] : [it.id];
      if (!ids.length) return;
      outDrag = true;
      document.body.classList.add('sh-dragging');
      // the tiles being dragged stay behind as ghosts (dimmed, dashed) until the drag ends
      const ghosts = ids.map((id) => list.querySelector(`.sh-chip[data-id="${id}"]`)).filter(Boolean);
      ghosts.forEach((g) => g.classList.add('sh-src'));
      // The cursor leaves the island during a native drag: keep it open (pin) until the drop; a completed drop closes it, like a paste.
      const wasPinned = !!(P.settings && P.settings.pinned);
      if (!wasPinned) P.setPinned(true);
      inv('shelf_drag_out', { ids }).then((res) => { if (res === 'copied') setTimeout(() => P.collapse(), 150); }).finally(() => {
        outDrag = false; document.body.classList.remove('sh-dragging');
        ghosts.forEach((g) => g.classList.remove('sh-src'));
        if (!wasPinned) P.setPinned(false);
      });
    };
    const off = () => { document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', off); };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', off);
  }

  // Fallback actions (also the only route if native drag-out misbehaves).
  function menu(it, x, y) {
    document.querySelector('.sh-menu')?.remove();
    const m = el('div', 'sh-menu');
    const add = (label, fn) => { const b = el('button', null, label); b.onclick = () => { PM.exit(m, { scale: 0.96, y: -4, remove: true }); fn(); }; m.append(b); };
    if (!it.missing) {
      add(t({ ru: 'Открыть', en: 'Open' }), () => inv('shelf_open', { id: it.id }));
      add(t({ ru: 'Показать в папке', en: 'Show in folder' }), () => inv('shelf_reveal', { id: it.id }));
    }
    add(t({ ru: 'Копировать путь', en: 'Copy path' }), () => inv('shelf_copy_path', { id: it.id }));
    add(t({ ru: 'Убрать', en: 'Remove' }), () => removeChips([it.id], 'shelf_remove', { id: it.id }));
    m.style.left = Math.min(x, innerWidth - 150) + 'px';
    m.style.top = Math.min(y, innerHeight - 130) + 'px';
    document.body.append(m);
    const close = (e) => { if (!m.contains(e.target)) { PM.exit(m, { scale: 0.96, y: -4, remove: true }); document.removeEventListener('mousedown', close, true); } };
    document.addEventListener('mousedown', close, true);
  }

  // Empty state: a card slides into a tray (own drawing; CSS animates .sh-card, .sh-tray). Still frame under reduced motion.
  const ART = '<svg class="sh-art" viewBox="0 0 120 84" width="120" height="84" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">'
    + '<g class="sh-card"><path d="M44 6h22l10 10v26a4 4 0 0 1-4 4H44a4 4 0 0 1-4-4V10a4 4 0 0 1 4-4z"/><path d="M66 6v10h10"/><path d="M47 28h22M47 36h14"/></g>'
    + '<path class="sh-tray sh-tray-f" d="M14 50h92l-9 24a4 4 0 0 1-3.7 2.6H26.7A4 4 0 0 1 23 74z"/><path class="sh-tray" d="M30 60h60"/></svg>';

  P.register({
    id: 'shelf',
    title: t({ ru: 'Полка', en: 'Shelf' }),
    order: 20,
    icon: ic('shelf'),
    mount(r) {
      root = r;
      root.classList.add('sh-root');
      zone = el('div', 'sh-zone');
      headBar = el('div', 'sh-head');
      dragAll = el('button', 'sh-dragall');
      dragAll.innerHTML = ic('grip', { size: 14, sw: 1.5 }) + '<b></b>';
      dragAll.title = t({ ru: 'Зажми и перетащи все файлы в нужное окно', en: 'Press and drag every file into another window' });
      dragAll.onmousedown = (e) => startDrag(e, null, dragAll, true);
      clearBtn = el('button', 'sh-clear');
      clearBtn.innerHTML = ic('trash', { size: 16, sw: 1.6 }) + '<span>' + t({ ru: 'Очистить', en: 'Clear' }) + '</span>';
      clearBtn.onclick = () => removeChips(items.map((i) => i.id), 'shelf_clear');
      headBar.append(dragAll, clearBtn);
      list = el('div', 'sh-list scroll');
      const empty = el('div', 'sh-empty');
      empty.innerHTML = `${ART}<b>${t({ ru: 'Полка пуста', en: 'The shelf is empty' })}</b><span>${t({ ru: 'Тащи файлы к острову: он раскроется, отпусти — они полежат здесь, пока не понадобятся. Потом вытащи их в нужное окно', en: 'Drag files toward the pill: it opens, let go and they wait here until you need them. Then drag them into any window' })}</span>`;
      const drop = el('div', 'sh-drop');
      drop.innerHTML = `${ic('download', { size: 22, sw: 1.7 })}<b>${t({ ru: 'Отпусти файлы здесь', en: 'Drop files here' })}</b>`;
      zone.append(headBar, list, empty, drop);
      root.replaceChildren(zone);
      render();
    },
    onShow() { enter = true; refresh(); },
    onHide() { sel.clear(); },
  });

  const refresh = () => inv('shelf_list').then(setItems);
  refresh();

  // ---------------------------------------------------------------- the compact drop card (collapsed island)
  // Reuses the live row: Pill.setLive('shelf-drop', { tall }) grows the island with its usual spring to the card's own height; the clock comes back
  // when it is removed. Left in place only while a file drag hovers; a drag that stays ~0.7 s opens the full island as before (Home, so a drop can still
  // hit the launcher row, if that was the last tab; the shelf otherwise).
  const DWELL_MS = 700;
  const dropHtml = () => `<div class="lp lp-drop"><span class="dz-ico">${ic('download', { size: 22, sw: 1.8 })}<span class="dz-ok">${ic('check', { size: 22, sw: 2.2 })}</span></span>`
    + `<span class="dz-tx"><b>${t({ ru: 'Отпустите, чтобы положить на полку', en: 'Drop to keep on the shelf' })}</b>`
    + `<i>${items.length ? t({ ru: 'Уже на полке: ', en: 'On the shelf now: ' }) + items.length : t({ ru: 'Файлы останутся на своих местах', en: 'Files stay where they are' })}</i></span></div>`;

  function startPeek() {
    if (peek) return;
    peek = { dwell: setTimeout(openFull, DWELL_MS), safety: setTimeout(endPeek, 30000), landed: false };
    P.setLive('shelf-drop', { priority: 95, tall: true, key: 'drop', html: dropHtml(), ttlMs: 30000 });
    P.setDragPull(true);
    P.setEdgeColor('var(--accent)');
  }
  function endPeek() {
    if (!peek) return;
    clearTimeout(peek.dwell); clearTimeout(peek.safety);
    peek = null;
    P.setLive('shelf-drop', null);
    P.setDragPull(false);
    P.setEdgeColor(null);
  }
  function openFull() { // the drag rests on the card: open the whole island under it
    if (!peek || peek.landed) return;
    endPeek();
    if (onHome()) { P.expand('home'); return; }
    dragOver = true; P.expand('shelf'); render();
  }
  // Drop: the file tiles fly from the drop point into the icon, the icon pops, the card says what happened, then folds back to the clock (+ badge).
  function land(paths, pos) {
    clearTimeout(peek.dwell); clearTimeout(peek.safety);
    peek.landed = true;
    peek.safety = setTimeout(endPeek, 3000); // the answer from shelf_add normally ends it sooner
    const w = document.querySelector('.lp-drop'), ico = w && w.querySelector('.dz-ico');
    if (!w) return;
    w.style.width = w.getBoundingClientRect().width / zoom() + 'px'; // the island keeps the card's width while the text changes to the answer
    w.classList.add('go');
    if (still() || !w.animate) return;
    const wr = w.getBoundingClientRect(), ir = ico.getBoundingClientRect(), s = zoom();
    const x0 = pos ? (pos.x - wr.left) / s : wr.width / s / 2, y0 = pos ? (pos.y - wr.top) / s : 8;
    const tx = (ir.left + ir.width / 2 - wr.left) / s, ty = (ir.top + ir.height / 2 - wr.top) / s;
    paths.slice(0, 3).forEach((p, k) => {
      const e = extOf(p.split(/[\\/]/).pop()), f = el('div', 'dz-fly sh-doc');
      f.style.setProperty('--c', EXT[e] || '#7c8090');
      f.append(el('span', null, (e || 'file').toUpperCase().slice(0, 4)));
      w.append(f);
      const a = f.animate([
        { transform: `translate(${x0 - 14 + k * 9}px, ${y0 - 18}px) scale(1.15) rotate(${(k - 1) * 12}deg)`, opacity: 1 },
        { transform: `translate(${tx - 14}px, ${ty - 18}px) scale(.2) rotate(0deg)`, opacity: 0.2 },
      ], { duration: 560, delay: k * 70, easing: cssVar('--spring-settle', 'cubic-bezier(.22, 1, .36, 1)'), fill: 'forwards' });
      a.onfinish = () => f.remove();
    });
  }
  function landed(added, total) { // shelf_add answered: `added` of `total` paths were new
    if (!peek || !peek.landed) return;
    const w = document.querySelector('.lp-drop');
    if (w) {
      const dup = !added;
      w.classList.add(dup ? 'dup' : 'ok');
      w.querySelector('.dz-tx b').textContent = dup ? t({ ru: 'Уже лежит на полке', en: 'Already on the shelf' }) : t({ ru: 'Лежит на полке', en: 'On the shelf' });
      w.querySelector('.dz-tx i').textContent = dup ? t({ ru: 'Ничего нового не добавлено', en: 'Nothing new was added' })
        : '+' + added + ' ' + files(added) + (added < total ? t({ ru: ', остальные уже были', en: ', the rest were already there' }) : '');
    }
    clearTimeout(peek.safety);
    peek.safety = setTimeout(endPeek, added ? 1400 : 1700);
  }

  // ---------------------------------------------------------------- Tauri drag events (files only: wry hands over CF_HDROP paths)
  const onHome = () => !!(P.lastTabId && P.lastTabId() === 'home');
  const cssPos = (p) => { const d = window.devicePixelRatio || 1; return p ? { x: p.x / d, y: p.y / d, t: Date.now() } : null; };
  if (T && T.event) {
    T.event.listen('shelf:changed', (e) => setItems(e.payload && e.payload.items));
    T.event.listen('settings:changed', () => { paintBadge(Math.max(seen, 0)); if (!shelfOn()) endPeek(); });
    T.event.listen('pill:hide', endPeek);
    // Files dragged in. Folded island: the compact drop card (startPeek). Open island: as before it switches to the shelf, except when Home was the
    // last tab: Home's launcher row takes drops too, so Home opens instead; a drop on the row goes to the launcher (tab-home.js), anywhere else on Home to the shelf.
    T.event.listen('tauri://drag-enter', () => {
      if (outDrag) return;
      if (!shelfOn()) { if (onHome()) P.expand('home'); return; }
      if (P.state !== 'expanded') { startPeek(); return; }
      if (onHome()) { P.expand('home'); return; }
      dragOver = true;
      P.expand('shelf');
      render();
    });
    T.event.listen('tauri://drag-leave', () => { dragOver = false; if (peek && !peek.landed) endPeek(); render(); });
    T.event.listen('tauri://drag-drop', (e) => {
      dragOver = false;
      const pl = e.payload || {}, paths = pl.paths;
      const onRow = P.launcherHit && P.launcherHit(pl.position);
      if (!outDrag && shelfOn() && !onRow && paths && paths.length) {
        lastDrop = cssPos(pl.position);
        const card = !!peek;
        if (card) land(paths, lastDrop);
        inv('shelf_add', { paths }).then((n) => { if (card) landed(n || 0, paths.length); });
        if (!card && onHome()) P.expand('shelf'); // dropped beside the launcher row: show where the files went
      } else endPeek();
      render();
    });
  }
})();
