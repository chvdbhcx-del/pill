// Shelf tab (S07): drop zone, file tiles, native drag-out, live "N files" widget. Backend: shelf.rs (lane L3b); UI restyled by the pill agent.
(() => {
  const P = window.Pill, T = window.__TAURI__, t = P.t, ic = P.icon;
  const inv = (c, a) => P.invoke(c, a);
  let items = [];
  let root = null, zone = null, list = null, headBar = null, dragAll = null, clearBtn = null;
  let dragOver = false, outDrag = false, sel = new Set(), enter = false;

  const el = (tag, cls, text) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text != null) e.textContent = text;
    return e;
  };

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

  let seen = -1;
  function setItems(v) {
    const was = seen;
    items = v || [];
    sel = new Set([...sel].filter((id) => items.some((i) => i.id === id)));
    const n = seen = items.length;
    if (was >= 0 && n > was) P.react('shelf', 'file', { ttlMs: 3000 }); // item added (not on first load, not on removal)
    P.setBadge('shelf', n);
    // Transient: a permanent widget would hide the clock for good. Shown for 5 s when the count changes (not on first load).
    P.setLive('shelf', n && was >= 0 && n !== was
      ? { priority: 30, ttlMs: 5000, html: `<div class="lp" style="--lpw:140px"><span class="lp-ico">${ic('shelf', { size: 18, sw: 1.6 })}</span><span class="lp-count">${n}</span></div>` } : null);
    render();
  }

  function render() {
    if (!root) return;
    zone.classList.toggle('over', dragOver);
    zone.classList.toggle('empty', !items.length);
    headBar.hidden = !items.length;
    dragAll.querySelector('b').textContent = t({ ru: 'Перетащить все', en: 'Drag all' }) + ' (' + items.length + ')';
    const fill = () => list.replaceChildren(...items.map(chip));
    if (enter) fill(); else PM.flip(list, fill); // chips that stay slide into the freed spot, no stagger replay
    enter = false;
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
      // The cursor leaves the island during a native drag: keep it open (pin) until the drop; a completed drop closes it, like a paste.
      const wasPinned = !!(P.settings && P.settings.pinned);
      if (!wasPinned) P.setPinned(true);
      inv('shelf_drag_out', { ids }).then((res) => { if (res === 'copied') setTimeout(() => P.collapse(), 150); }).finally(() => {
        outDrag = false; document.body.classList.remove('sh-dragging');
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
      empty.innerHTML = `${ic('shelf', { size: 22, sw: 1.6 })}<b>${t({ ru: 'Полка пуста', en: 'The shelf is empty' })}</b><span>${t({ ru: 'Перетащи файлы на остров — они полежат здесь, пока ты не вытащишь их в нужное окно', en: 'Drop files on the pill and they wait here until you drag them into the window you need' })}</span>`;
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

  if (T && T.event) {
    T.event.listen('shelf:changed', (e) => setItems(e.payload && e.payload.items));
    // Files dragged in: the island opens on the shelf, except when Home was the last tab: Home's launcher row takes drops too,
    // so Home opens instead; a drop on the row goes to the launcher (tab-home.js), a drop anywhere else on Home goes to the shelf.
    const onHome = () => !!(P.lastTabId && P.lastTabId() === 'home');
    const shelfOn = () => !P.moduleOn || P.moduleOn('module.shelf'); // Settings > Modules > File shelf
    T.event.listen('tauri://drag-enter', () => {
      if (outDrag) return;
      if (onHome()) { P.expand('home'); return; }
      if (!shelfOn()) return;
      dragOver = true;
      P.expand('shelf');
      render();
    });
    T.event.listen('tauri://drag-leave', () => { dragOver = false; render(); });
    T.event.listen('tauri://drag-drop', (e) => {
      dragOver = false;
      const paths = e.payload && e.payload.paths;
      const onRow = P.launcherHit && P.launcherHit(e.payload && e.payload.position);
      if (!outDrag && shelfOn() && !onRow && paths && paths.length) {
        inv('shelf_add', { paths });
        if (onHome()) P.expand('shelf'); // dropped beside the launcher row: show where the files went
      }
      render();
    });
  }
})();
