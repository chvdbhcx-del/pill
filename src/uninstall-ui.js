// Command bar: "uninstall completely" panel (backend: uninstall.rs). bar.js hands an `uninstall` row to UninstallUI.open(); everything after that is
// this panel: confirm card -> the app's own uninstaller runs -> deep leftover scan -> checklist in two groups (Certain pre-ticked, Probable not)
// -> delete selected (Recycle Bin, .reg backups) -> optional elevated step for what needs administrator rights -> summary.
// Talks to Rust through uninstall_* commands and the uninstall:state / uninstall:item events; the page only ever sends indexes, never paths.
// Motion: every step enters on the settle spring and the previous one leaves first (PillMotion), rows drop in staggered, groups expand and collapse,
// ticks draw their check, numbers glide, deleted rows slide out one by one, failures shake once. transform / opacity only (groups animate height,
// the shared PillMotion way); html.rm / prefers-reduced-motion = every state is applied at once (PillMotion.off()).
(() => {
  const T = window.__TAURI__;
  const inv = (c, a) => T.core.invoke(c, a).catch((e) => { console.error(c, e); return null; });
  const list = document.getElementById('list'), q = document.getElementById('q');
  if (!T || !list || !q) return;
  const M = window.PillMotion;
  const off = () => !M || M.off();
  const lang = () => { try { const c = localStorage.getItem('pill.lang'); if (c === 'ru' || c === 'en') return c; } catch (e) { /* storage blocked */ } return /^ru/i.test(navigator.language || '') ? 'ru' : 'en'; };
  const L = {
    uninstall: { ru: 'Удалить', en: 'Uninstall' }, cancel: { ru: 'Отмена', en: 'Cancel' }, close: { ru: 'Закрыть', en: 'Close' },
    folder: { ru: 'Папка: ', en: 'Folder: ' }, store: { ru: 'Приложение из Microsoft Store', en: 'Microsoft Store app' },
    risky: { ru: 'Системный компонент: от него могут зависеть другие программы.', en: 'System component: other programs may depend on it.' },
    silent: { ru: 'Тихо, без вопросов (если установщик это умеет)', en: 'Silent, no prompts (if the uninstaller supports it)' },
    how: { ru: 'Сначала запустится собственный деинсталлятор приложения. Потом Pill глубоко поищет остатки по всему ПК и покажет список: ничего не удаляется, пока ты не отметишь и не нажмёшь кнопку.', en: 'The app’s own uninstaller runs first. Then Pill searches the whole PC for leftovers and shows a list: nothing is deleted until you tick it and press the button.' },
    running: { ru: 'Идёт удаление. Закончи его в окне деинсталлятора, Pill его не закрывает.', en: 'Uninstalling. Finish it in the uninstaller’s window; Pill never closes it.' },
    waiting: { ru: 'Жду, пока приложение исчезнет из списка установленных…', en: 'Waiting for the app to leave the installed list…' },
    checkAgain: { ru: 'Проверить снова', en: 'Check again' }, hide: { ru: 'Скрыть', en: 'Hide' },
    scanning: { ru: 'Ищу остатки по всему ПК…', en: 'Searching the whole PC for leftovers…' }, found: { ru: 'найдено: ', en: 'found: ' },
    still: { ru: 'Приложение всё ещё установлено (деинсталлятор отменён или ещё работает). Ничего не удалено.', en: 'The app is still installed (the uninstaller was cancelled or is still running). Nothing was deleted.' },
    leftovers: { ru: 'Остатки: ', en: 'Leftovers: ' }, none: { ru: 'Остатков не найдено. Приложение удалено начисто.', en: 'No leftovers found. The app is gone.' },
    items: { ru: ' объектов', en: ' items' },
    certain: { ru: 'Точно его', en: 'Certain' }, probable: { ru: 'Вероятно его', en: 'Probable' }, admin: { ru: 'Нужны права администратора', en: 'Needs administrator' },
    certainTip: { ru: 'Указывают в папку установки, несут GUID продукта или называются как приложение. Отмечены.', en: 'Point into the install folder, carry the product GUID or are named like the app. Pre-ticked.' },
    probableTip: { ru: 'Содержат полное название приложения как слова или это общая папка издателя. Не отмечены.', en: 'Contain the app’s full name as words, or a shared publisher folder. Not ticked.' },
    selAll: { ru: 'Все «точно»', en: 'All certain' }, selAllTip: { ru: 'Отметить всё, что точно принадлежит приложению', en: 'Tick everything that certainly belongs to the app' },
    selected: { ru: 'Выбрано: ', en: 'Selected: ' },
    del: { ru: 'Удалить выбранное', en: 'Delete selected' }, adminDel: { ru: 'Удалить системные остатки (администратор)', en: 'Remove system leftovers (administrator)' },
    adminScan: { ru: 'Поискать ещё как администратор', en: 'Search more as administrator' },
    adminErr: { ru: 'Права администратора не получены: ', en: 'Administrator step did not run: ' }, cancelled: { ru: 'запрос Windows отклонён', en: 'the Windows prompt was declined' },
    manual: { ru: 'Только список, Pill это не трогает (драйверы и т. п.):', en: 'Listed only, Pill does not touch these (drivers and the like):' },
    safe: { ru: 'Файлы уходят в Корзину, ключи реестра сначала сохраняются в .reg-копию (её можно импортировать обратно).', en: 'Files go to the Recycle Bin; registry keys are saved to a .reg backup first (you can import it back).' },
    hiddenOthers: { ru: ' объектов в профилях других пользователей не показаны: удали их из сеанса этого пользователя.', en: ' items in other users’ profiles are not shown: remove them from that user’s own session.' },
    cut: { ru: 'Поиск остановлен по лимиту времени или числа объектов: список может быть неполным.', en: 'The scan hit its time or entry limit: the list may be incomplete.' },
    deleting: { ru: 'Удаляю выбранное…', en: 'Deleting the selection…' },
    elevScan: { ru: 'Жду разрешения Windows: поиск от администратора…', en: 'Waiting for the Windows permission prompt: searching as administrator…' },
    elevDel: { ru: 'Жду разрешения Windows: удаляю системные остатки…', en: 'Waiting for the Windows permission prompt: removing system leftovers…' },
    doneTitle: { ru: 'Готово', en: 'Done' },
    removed: { ru: 'Удалено: ', en: 'Removed ' }, filesW: { ru: 'файлов и папок', en: 'files and folders' }, keysW: { ru: 'ключей реестра', en: 'registry keys' }, otherW: { ru: 'служб и задач', en: 'services and tasks' },
    needAdmin: { ru: 'Нужны права администратора: ', en: 'Need administrator: ' },
    failed: { ru: 'Не удалось:', en: 'Could not delete:' },
    backup: { ru: 'Копия реестра: ', en: 'Registry backup: ' }, log: { ru: 'Журнал: ', en: 'Log: ' },
    openBk: { ru: 'Открыть папку с копиями', en: 'Open backups folder' },
    err: { ru: 'Не получилось: ', en: 'Failed: ' },
    kind: { folder: { ru: 'Папка', en: 'Folder' }, file: { ru: 'Файл', en: 'File' }, shortcut: { ru: 'Ярлык', en: 'Shortcut' }, key: { ru: 'Ключ реестра', en: 'Registry key' },
      value: { ru: 'Значение реестра', en: 'Registry value' }, autorun: { ru: 'Автозапуск', en: 'Autostart' }, service: { ru: 'Служба', en: 'Service' }, task: { ru: 'Задача', en: 'Task' }, item: { ru: 'Объект', en: 'Item' } },
    stage: {
      starting: { ru: 'запуск', en: 'starting' }, 'files and folders': { ru: 'файлы и папки', en: 'files and folders' }, 'registry (user)': { ru: 'реестр (пользователь)', en: 'registry (user)' },
      'registry (system)': { ru: 'реестр (система)', en: 'registry (system)' }, 'Windows Installer records': { ru: 'записи Windows Installer', en: 'Windows Installer records' },
      'firewall rules': { ru: 'правила брандмауэра', en: 'firewall rules' }, services: { ru: 'службы', en: 'services' }, 'scheduled tasks': { ru: 'задачи планировщика', en: 'scheduled tasks' },
    },
    note: {
      'install folder': { ru: 'папка установки', en: 'install folder' },
      'inside the install folder': { ru: 'внутри папки установки', en: 'inside the install folder' },
      'shared with another installed app': { ru: 'общая с другой программой', en: 'shared with another installed app' },
      'named like the app': { ru: 'называется как приложение', en: 'named like the app' },
      'named like the app, under the publisher': { ru: 'как приложение, внутри папки издателя', en: 'named like the app, under the publisher' },
      'named like the app, in a place for your own files': { ru: 'как приложение, но в папке для твоих файлов: проверь перед удалением', en: 'named like the app, but in a place for your own files: check before deleting' },
      'named like the app, at the top of a drive': { ru: 'как приложение, в корне диска: там могут лежать твои файлы', en: 'named like the app, at the top of a drive: it may hold your own files' },
      'install folder, but it is a folder of its own at the top of a drive and not named like the app': { ru: 'папка установки, но это отдельная папка в корне диска с чужим именем', en: 'install folder, but a folder of its own at the top of a drive, not named like the app' },
      'install folder, but it holds several programs': { ru: 'папка установки, но в ней несколько программ', en: 'install folder, but it holds several programs' },
      'key shared with other versions of the product': { ru: 'ключ общий с другими версиями продукта', en: 'key shared with other versions of the product' },
      'named like the publisher': { ru: 'называется как издатель', en: 'named like the publisher' },
      "contains the app's full name": { ru: 'в названии есть полное имя приложения', en: 'contains the app’s full name' },
      'publisher folder holding only this app': { ru: 'папка издателя, в ней только это приложение', en: 'publisher folder holding only this app' },
      'publisher folder, other apps may use it': { ru: 'папка издателя: её могут использовать другие программы', en: 'publisher folder, other apps may use it' },
      'emptied folder that held the app': { ru: 'пустая папка, где лежало приложение', en: 'emptied folder that held the app' },
      'installer file named like the app': { ru: 'установщик с именем приложения', en: 'installer file named like the app' },
      'Start Menu folder named like the app': { ru: 'папка в меню «Пуск»', en: 'Start Menu folder' },
      'shortcut named like the app': { ru: 'ярлык с именем приложения', en: 'shortcut named like the app' },
      'points into the install folder': { ru: 'ведёт в папку установки', en: 'points into the install folder' },
      'key under the publisher': { ru: 'ключ издателя с именем приложения', en: 'key under the publisher' },
      'publisher key holding only this app': { ru: 'ключ издателя, в нём только это приложение', en: 'publisher key holding only this app' },
      'uninstall entry of this product, its uninstaller is gone': { ru: 'запись об удалении, деинсталлятора уже нет', en: 'uninstall entry, its uninstaller is gone' },
      'App Paths entry in the install folder': { ru: 'запись App Paths ведёт в папку установки', en: 'App Paths entry in the install folder' },
      'COM class or handler in the install folder': { ru: 'COM-класс или обработчик из папки установки', en: 'COM class or handler in the install folder' },
      'carries the product GUID': { ru: 'несёт GUID продукта', en: 'carries the product GUID' },
      'starts the app at sign-in': { ru: 'запускает приложение при входе', en: 'starts the app at sign-in' },
      'install folder path in a history list': { ru: 'путь к папке установки в списке истории', en: 'install folder path in a history list' },
      'association with a removed handler': { ru: 'ассоциация с удалённым обработчиком', en: 'association with a removed handler' },
      'Windows Installer record of this product': { ru: 'запись Windows Installer об этом продукте', en: 'Windows Installer record of this product' },
      'service runs a file in the install folder': { ru: 'служба запускает файл из папки установки', en: 'service runs a file in the install folder' },
      'scheduled task starts a file in the install folder': { ru: 'задача запускает файл из папки установки', en: 'scheduled task starts a file in the install folder' },
      'task named like the app': { ru: 'задача с именем приложения', en: 'task named like the app' },
      'firewall rule for a program in the install folder': { ru: 'правило брандмауэра для программы из папки установки', en: 'firewall rule for a program in the install folder' },
    },
  };
  const t = (o) => o[lang()] ?? o.en;
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const size = (b) => (b >= 1 << 30 ? (b / (1 << 30)).toFixed(1) + ' GB' : b >= 1 << 20 ? Math.round(b / (1 << 20)) + ' MB' : b >= 1 << 10 ? Math.round(b / 1024) + ' KB' : b ? b + ' B' : '0 B');
  const SVG = (inner, cls) => { const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); s.setAttribute('viewBox', '0 0 16 16'); s.setAttribute('class', cls || ''); s.innerHTML = inner; return s; }; // static markup only
  const stag = (node, i) => { node.classList.add('stag'); node.style.setProperty('--i', i); return node; };

  // a number that glides to its new value (text only, one rAF loop per node, instant under reduced motion)
  function glide(node, to, fmt = (v) => String(Math.round(v))) {
    const from = Number(node.dataset.v || 0);
    node.dataset.v = to;
    cancelAnimationFrame(node._raf);
    if (off() || from === to) { node.textContent = fmt(to); return; }
    const t0 = performance.now();
    const step = (now) => {
      const k = Math.min(1, (now - t0) / 380), e = 1 - Math.pow(1 - k, 3);
      node.textContent = fmt(from + (to - from) * e);
      if (k < 1) node._raf = requestAnimationFrame(step);
    };
    node._raf = requestAnimationFrame(step);
  }

  const box = el('div'); box.id = 'uni'; box.hidden = true;
  list.before(box);
  let phase = '', inner = null, gen = 0, S = null;

  const btn = (label, cls, fn) => { const b = el('button', 'ubtn ' + (cls || ''), label); b.type = 'button'; b.addEventListener('click', fn); return b; };
  // a tick box whose check draws itself
  function tick(checked, onChange) {
    const w = el('span', 'ucb'), i = el('input'); i.type = 'checkbox'; i.checked = !!checked;
    w.append(i, SVG('<path d="M3.5 8.5l3 3 6-7"/>'));
    i.addEventListener('change', () => onChange && onChange(i.checked));
    w.input = i;
    return w;
  }

  // ---- steps: the old one leaves, the new one settles in ----
  function go(build) {
    const my = ++gen, next = build();
    next.classList.add('u-step');
    const apply = () => {
      if (my !== gen) return;
      inner = next;
      box.replaceChildren(next);
      if (box.hidden) {
        box.hidden = false;
        M && M.enter(box, { y: -8, dur: 0.4 });
      }
      M && M.enter(next, { y: 10, dur: 0.4 });
      box.querySelector('.ubtn.dflt')?.focus({ preventScroll: true });
    };
    if (box.hidden) {
      // the result list steps aside first
      if (list.style.display !== 'none' && !off() && list.childElementCount) M.exit(list, { y: -4, dur: 0.12, after: (e) => { e.style.display = 'none'; apply(); } });
      else { list.style.display = 'none'; apply(); }
    } else if (inner && inner.isConnected && !off()) M.exit(inner, { y: -6, dur: 0.14 }).then((ok) => ok && apply());
    else apply();
  }
  function close() {
    phase = ''; S = null; gen++;
    inv('uninstall_cancel');
    const finish = () => { box.hidden = true; box.replaceChildren(); inner = null; list.style.display = ''; M && M.enter(list, { y: 4, dur: 0.3 }); q.focus(); q.dispatchEvent(new Event('input')); };
    if (box.hidden || off()) finish(); else M.collapse(box, { dur: 0.26, after: finish });
  }
  const hide = () => inv('cmd_toggle');

  // ---- confirm card ----
  async function open(it) {
    const id = parseInt(it.payload, 10);
    const r = await inv('uninstall_info', { id, name: it.title });
    if (!r || !r.ok) { q.dispatchEvent(new Event('input')); return; }
    phase = 'confirm'; S = null;
    const card = el('div');
    const silent = tick(false);
    const meta = [r.publisher, r.version, r.sizeKb ? size(r.sizeKb * 1024) : ''].filter(Boolean).join(' · ');
    const parts = [el('div', 'u-h', r.name), el('div', 'u-s', r.store ? t(L.store) : meta)];
    if (r.location) parts.push(el('div', 'u-s', t(L.folder) + r.location));
    if (r.risky) parts.push(el('div', 'u-warn', t(L.risky)));
    if (r.canSilent) { const lab = el('label', 'u-chk'); lab.append(silent, document.createTextNode(' ' + t(L.silent))); parts.push(lab); }
    parts.push(el('div', 'u-note', t(L.how)));
    const bar = el('div', 'u-btns');
    bar.append(btn(t(L.uninstall), 'danger', () => { inv('uninstall_start', { silent: silent.input.checked }); }), btn(t(L.cancel), 'dflt', close));
    parts.push(bar);
    parts.forEach((p, i) => card.append(stag(p, i)));
    go(() => card);
  }

  // ---- progress steps ----
  const bar = (cls) => { const b = el('div', 'uprog ' + (cls || '')); b.append(el('i')); return b; };

  function busyPage(title, text, extra, cls) {
    const page = el('div');
    const parts = [el('div', 'u-h', title), bar(cls), el('div', 'u-busy', text)];
    if (extra) parts.push(extra);
    parts.forEach((p, i) => page.append(stag(p, i)));
    return page;
  }

  function scanningPage(s) {
    const stage = el('span', 'u-stage', t(L.stage[s.step] || { en: s.step || '' }));
    const num = el('b', 'tnum'); num.textContent = '0';
    const line = el('div', 'u-note'); line.append(document.createTextNode(t(L.found)), num, document.createTextNode(' · '), stage);
    const page = busyPage(s.name, t(L.scanning), line);
    page.dataset.scan = '1'; page._num = num; page._stage = stage;
    glide(num, s.found || 0);
    return page;
  }
  function updateScan(s) {
    glide(inner._num, s.found || 0);
    const next = t(L.stage[s.step] || { en: s.step || '' });
    if (inner._stage.textContent !== next) {
      const st = inner._stage;
      if (off()) st.textContent = next; else M.exit(st, { y: -3, dur: 0.1, after: () => { st.textContent = next; M.enter(st, { y: 3, dur: 0.25 }); } });
    }
  }

  // ---- the checklist ----
  const kindLabel = (k) => t(L.kind[k] || { en: k });
  function rowFor(it, i) {
    const r = el('label', 'u-item'); r.dataset.shown = it.path; r.dataset.id = it.id;
    const cb = tick(it.ticked, () => S.upd());
    S.picks.push({ it, cb, row: r });
    const txt = el('div', 'u-it');
    const meta = [kindLabel(it.kind), it.size ? size(it.size) : '', t(L.note[it.note] || { en: it.note })].filter(Boolean).join(' · ');
    const m = el('div', 'u-s', meta);
    txt.append(el('div', 'u-p', it.path), m);
    r.append(cb, txt);
    r._meta = m;
    return stag(r, i);
  }

  // a group: header (chevron, title, count, size) and a body that expands / collapses with a height animation
  function group(titleKey, tipKey, items, open) {
    const g = el('div', 'u-grp');
    const head = el('button', 'u-gh'); head.type = 'button'; head.title = tipKey ? t(L[tipKey]) : '';
    const chev = SVG('<path d="M5 6l3 3 3-3"/>', 'u-chev'); if (open) chev.classList.add('open');
    const cnt = el('b', 'tnum'), sz = el('span', 'tnum u-gsz');
    head.append(chev, el('span', 'u-gt', t(L[titleKey])), cnt, sz);
    const body = el('div', 'u-gb'); body.hidden = !open;
    items.forEach((it, i) => body.append(rowFor(it, i)));
    g.append(head, body);
    g.cnt = cnt; g.sz = sz; g.body = body; g.items = items;
    head.addEventListener('click', () => {
      const willOpen = body.hidden;
      chev.classList.toggle('open', willOpen);
      if (willOpen) { body.hidden = false; M && M.expand(body); } else if (M) M.collapse(body, { dur: 0.26, hide: true }); else body.hidden = true;
    });
    return g;
  }

  function recount(g) {
    const live = S.picks.filter((p) => g.items.includes(p.it) && !p.gone);
    glide(g.cnt, live.length);
    glide(g.sz, live.reduce((a, p) => a + (p.it.size || 0), 0), (v) => (v ? ' · ' + size(v) : ''));
    if (!live.length && !g.hidden && g.items.length) { if (M) M.collapse(g, { dur: 0.26, hide: true }); else g.hidden = true; }
  }

  function listPage(s, summary) {
    S = { name: s.name, picks: [], groups: [], adminScanned: S ? S.adminScanned : false, removed: 0 };
    if (s.adminScanned) S.adminScanned = true;
    const page = el('div');
    const parts = [];
    if (summary) parts.push(summary);
    else parts.push(el('div', 'u-h', t(L.leftovers) + s.name));
    if (s.adminError) parts.push(el('div', 'u-warn', t(L.adminErr) + (s.adminError === 'cancelled' ? t(L.cancelled) : s.adminError)));
    const user = s.items.filter((i) => !i.admin), adm = s.items.filter((i) => i.admin);
    const cert = user.filter((i) => i.tier === 'certain'), prob = user.filter((i) => i.tier !== 'certain');
    const mk = (key, tip, arr, open) => { const g = group(key, tip, arr, open); S.groups.push(g); parts.push(g); return g; };
    if (cert.length) mk('certain', 'certainTip', cert, true);
    if (prob.length) mk('probable', 'probableTip', prob, !cert.length);
    if (adm.length) mk('admin', null, adm, true);
    if (!s.items.length) parts.push(el('div', 'u-s', t(L.none)));
    if (s.manual && s.manual.length) { parts.push(el('div', 'u-sub', t(L.manual))); for (const m of s.manual) parts.push(el('div', 'u-p', m)); }
    if (s.cut) parts.push(el('div', 'u-warn', t(L.cut)));
    if (s.hiddenOthers) parts.push(el('div', 'u-note', s.hiddenOthers + t(L.hiddenOthers)));
    if (s.items.length) parts.push(el('div', 'u-note', t(L.safe)));
    const admRow = el('div', 'u-btns u-admrow');
    parts.push(admRow);
    parts.forEach((p, i) => { if (!p.classList.contains('stag') && !p.classList.contains('u-grp')) stag(p, Math.min(i, 6)); page.append(p); });
    // footer
    const foot = el('div', 'u-foot');
    const selTxt = el('span', 'u-sel'), selNum = el('b', 'tnum'), selSz = el('span', 'tnum');
    selTxt.append(document.createTextNode(t(L.selected)), selNum, selSz);
    const delBtn = btn('', 'danger', () => {
      const ids = S.picks.filter((p) => !p.it.admin && p.cb.input.checked && !p.gone).map((p) => p.it.id);
      if (ids.length) { busy(true); inv('uninstall_delete', { ids }); }
    });
    const admBtn = btn('', 'danger u-adm', () => {
      const ids = S.picks.filter((p) => p.it.admin && p.cb.input.checked && !p.gone).map((p) => p.it.id);
      if (ids.length) inv('uninstall_admin', { mode: 'delete', ids });
    });
    const admTxt = document.createTextNode('');
    admBtn.append(SVG('<path d="M8 1.5l5.5 2v4.2c0 3.3-2.3 5.6-5.5 6.8-3.2-1.2-5.5-3.5-5.5-6.8V3.5z"/>', 'u-ico'), admTxt);
    const row1 = el('div', 'u-btns');
    if (s.items.some((i) => i.tier === 'certain')) row1.append(Object.assign(btn(t(L.selAll), 'ghost', () => S.picks.forEach((p) => { if (p.it.tier === 'certain' && !p.gone) p.cb.input.checked = true; S.upd(); })), { title: t(L.selAllTip) }));
    if (user.length) row1.append(delBtn);
    row1.append(btn(t(L.close), 'dflt', close));
    if (!S.adminScanned) admRow.append(btn(t(L.adminScan), 'ghost', () => inv('uninstall_admin', { mode: 'scan', ids: [] })));
    if (adm.length) admRow.append(admBtn);
    if (!admRow.childElementCount) admRow.remove();
    if (user.length) foot.append(selTxt);
    foot.append(row1);
    page.append(foot);
    S.upd = () => {
      const u = S.picks.filter((p) => !p.it.admin && p.cb.input.checked && !p.gone), a = S.picks.filter((p) => p.it.admin && p.cb.input.checked && !p.gone);
      glide(selNum, u.length);
      glide(selSz, u.reduce((x, p) => x + (p.it.size || 0), 0), (v) => (v ? ' · ' + size(v) : ''));
      delBtn.textContent = `${t(L.del)} (${u.length})`; delBtn.disabled = !u.length || !!(page.classList.contains('u-busy-page'));
      admTxt.textContent = ` ${t(L.adminDel)} (${a.length})`;
      admBtn.disabled = !a.length;
    };
    S.groups.forEach(recount);
    S.upd();
    page._foot = foot;
    return page;
  }

  // deleting: the checklist stays, its buttons rest, a progress line runs and rows leave as the backend reports them
  function busy(on) {
    if (!inner) return;
    inner.classList.toggle('u-busy-page', on);
    inner.querySelectorAll('.ubtn').forEach((b) => { if (on) b.disabled = true; });
    if (on && !inner.querySelector('.uprog.top')) {
      const b = bar('top'); const c = el('div', 'u-note'), n = el('b', 'tnum'); n.textContent = '0';
      c.append(document.createTextNode(t(L.deleting) + ' '), n);
      const wrap = el('div', 'u-delprog'); wrap.append(b, c); wrap._n = n;
      inner.prepend(wrap);
      inner._prog = wrap;
      M && M.enter(wrap, { y: -6, dur: 0.3 });
    }
  }
  function itemDone(p) {
    if (!S || !p) return;
    if (inner && inner._prog) glide(inner._prog._n, p.n || 0, (v) => `${Math.round(v)} / ${p.total || '?'}`);
    const pick = S.picks.find((x) => x.it.path === p.shown && !x.gone);
    if (!pick) return;
    if (p.ok) {
      pick.gone = true;
      const r = pick.row;
      const finish = () => S.groups.forEach(recount);
      if (off()) { r.remove(); finish(); } else M.exit(r, { x: 18, dur: 0.18, after: (e) => { e.style.opacity = '0'; M.collapse(e, { dur: 0.24, remove: true, after: finish }); } });
      finish();
    } else {
      pick.row.classList.add('u-fail');
      pick.row._meta.textContent = p.error || '';
      pick.cb.input.checked = false;
      S.upd();
    }
  }

  function donePage(s) {
    const any = s.log || s.files || s.keys || s.other || (s.failed && s.failed.length);
    const head = el('div', 'u-done');
    const ok = SVG('<circle cx="8" cy="8" r="6.5"/><path d="M5 8.4l2.1 2.1L11 6"/>', 'u-ok');
    const cnt = [[s.files, L.filesW], [s.keys, L.keysW], [s.other, L.otherW]].filter((x) => x[0]).map((x) => `${x[0]} ${t(x[1])}`).join(', ');
    const col = el('div');
    col.append(el('div', 'u-h', s.name), el('div', 'u-s', any ? (t(L.removed) + (cnt || '0')) : t(L.none)));
    if (s.needAdmin) col.append(el('div', 'u-s', t(L.needAdmin) + s.needAdmin));
    head.append(ok, col);
    if (s.failed && s.failed.length) { const f = el('div'); f.append(el('div', 'u-sub', t(L.failed))); s.failed.forEach((x) => f.append(el('div', 'u-p u-failtxt', x))); head.append(f); }
    const extra = el('div');
    if (s.backup) extra.append(el('div', 'u-p', t(L.backup) + s.backup));
    if (s.log) extra.append(el('div', 'u-p', t(L.log) + s.log));
    if (s.backup || s.log) { const lnk = el('button', 'u-link', t(L.openBk)); lnk.type = 'button'; lnk.addEventListener('click', () => inv('uninstall_open_backups')); extra.append(lnk); }
    const wrap = el('div'); wrap.append(head, extra);
    if (s.items && s.items.length) { const page = listPage(s, wrap); return page; }
    S = null;
    const bar2 = el('div', 'u-btns'); bar2.append(btn(t(L.close), 'dflt', close));
    const page = el('div');
    [head, extra, bar2].forEach((p, i) => page.append(stag(p, i)));
    return page;
  }

  function elevPage(s) {
    const page = busyPage(s.name, t(s.mode === 'delete' ? L.elevDel : L.elevScan), null, 'elev');
    page.prepend(SVG('<path d="M8 1.5l5.5 2v4.2c0 3.3-2.3 5.6-5.5 6.8-3.2-1.2-5.5-3.5-5.5-6.8V3.5z"/>', 'u-shield'));
    return page;
  }

  function render(s) {
    if (!s || !s.phase) return;
    const prev = phase;
    phase = s.phase;
    switch (s.phase) {
      case 'running': {
        const b2 = el('div', 'u-btns'); b2.append(btn(t(L.checkAgain), '', () => inv('uninstall_scan')), btn(t(L.hide), 'dflt', hide));
        const page = busyPage(s.name, t(L.running), el('div', 'u-note', t(L.waiting)));
        page.append(stag(b2, 4));
        return go(() => page);
      }
      case 'scanning':
        if (prev === 'scanning' && inner && inner._num) return updateScan(s);
        return go(() => scanningPage(s));
      case 'deleting':
        if (S && inner && inner.classList.contains('u-step') && inner._foot) return busy(true);
        return go(() => busyPage(s.name, t(L.deleting)));
      case 'elevating': return go(() => elevPage(s));
      case 'still': {
        const b2 = el('div', 'u-btns'); b2.append(btn(t(L.checkAgain), 'dflt', () => inv('uninstall_scan')), btn(t(L.close), '', close));
        const page = el('div');
        [el('div', 'u-h', s.name), el('div', 'u-warn', t(L.still)), b2].forEach((p, i) => page.append(stag(p, i)));
        return go(() => page);
      }
      case 'review': return go(() => listPage(s));
      case 'error': {
        const b2 = el('div', 'u-btns'); b2.append(btn(t(L.close), 'dflt', close));
        const page = el('div');
        [el('div', 'u-h', s.name), el('div', 'u-warn u-shake', t(L.err) + (s.message || '')), b2].forEach((p, i) => page.append(stag(p, i)));
        return go(() => page);
      }
      case 'done': return go(() => donePage(s));
      default:
    }
  }

  T.event.listen('uninstall:state', (e) => render(e.payload));
  T.event.listen('uninstall:item', (e) => itemDone(e.payload));
  T.event.listen('bar:shown', async () => { const s = await inv('uninstall_state'); if (s && s.phase && s.phase !== phase) render(s); });
  // while the panel is up it owns the keyboard: Esc closes it (or just hides the bar while something runs), Enter/Tab act on its own buttons
  window.addEventListener('keydown', (e) => {
    if (box.hidden) return;
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (['running', 'scanning', 'deleting', 'elevating'].includes(phase)) hide(); else close(); }
    else if (['Enter', 'ArrowUp', 'ArrowDown', 'Tab', 'F4'].includes(e.key) || (e.ctrlKey && e.shiftKey)) e.stopPropagation();
  }, true);
  window.UninstallUI = { open };
})();
