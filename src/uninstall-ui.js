// Command bar: "uninstall completely" panel (backend: uninstall.rs). bar.js hands an `uninstall` row to UninstallUI.open(); everything after
// that is this panel: confirm card -> the app's own uninstaller runs -> leftover checklist -> delete selected (Recycle Bin, .reg backups).
// Talks to Rust through uninstall_* commands and the uninstall:state event; the page only ever sends indexes, never paths.
(() => {
  const T = window.__TAURI__;
  const inv = (c, a) => T.core.invoke(c, a).catch((e) => { console.error(c, e); return null; });
  const list = document.getElementById('list'), q = document.getElementById('q');
  if (!T || !list || !q) return;
  const lang = () => { try { const c = localStorage.getItem('pill.lang'); if (c === 'ru' || c === 'en') return c; } catch (e) { /* storage blocked */ } return /^ru/i.test(navigator.language || '') ? 'ru' : 'en'; };
  const L = {
    uninstall: { ru: 'Удалить', en: 'Uninstall' }, cancel: { ru: 'Отмена', en: 'Cancel' }, close: { ru: 'Закрыть', en: 'Close' },
    folder: { ru: 'Папка: ', en: 'Folder: ' }, store: { ru: 'Приложение из Microsoft Store', en: 'Microsoft Store app' },
    risky: { ru: 'Системный компонент: от него могут зависеть другие программы.', en: 'System component: other programs may depend on it.' },
    silent: { ru: 'Тихо, без вопросов (если установщик это умеет)', en: 'Silent, no prompts (if the uninstaller supports it)' },
    how: { ru: 'Сначала запустится собственный деинсталлятор приложения. Потом Pill найдёт остатки и спросит, что удалить.', en: 'The app’s own uninstaller runs first. Then Pill looks for leftovers and asks before deleting anything.' },
    running: { ru: 'Идёт удаление. Закончи его в окне деинсталлятора, Pill его не закрывает.', en: 'Uninstalling. Finish it in the uninstaller’s window; Pill never closes it.' },
    checkAgain: { ru: 'Проверить снова', en: 'Check again' }, hide: { ru: 'Скрыть', en: 'Hide' },
    scanning: { ru: 'Ищу остатки…', en: 'Looking for leftovers…' }, deleting: { ru: 'Удаляю выбранное…', en: 'Deleting the selection…' },
    still: { ru: 'Приложение всё ещё установлено (деинсталлятор отменён или ещё работает). Ничего не удалено.', en: 'The app is still installed (the uninstaller was cancelled or is still running). Nothing was deleted.' },
    leftovers: { ru: 'Остатки: ', en: 'Leftovers: ' }, none: { ru: 'Остатков не найдено. Приложение удалено начисто.', en: 'No leftovers found. The app is gone.' },
    manual: { ru: 'Удалить вручную (нужны права администратора, Pill это не трогает):', en: 'Remove manually (needs administrator rights, Pill does not touch these):' },
    del: { ru: 'Удалить выбранное', en: 'Delete selected' },
    safe: { ru: 'Файлы уходят в Корзину, ключи реестра сначала сохраняются в .reg-копию.', en: 'Files go to the Recycle Bin; registry keys are saved to a .reg backup first.' },
    cut: { ru: 'Поиск остановлен по лимиту: список может быть неполным.', en: 'The scan hit its limit: the list may be incomplete.' },
    done: { ru: 'Готово. Удалено: ', en: 'Done. Deleted: ' }, failed: { ru: 'Не удалось:', en: 'Could not delete:' },
    backup: { ru: 'Копия реестра: ', en: 'Registry backup: ' }, log: { ru: 'Журнал: ', en: 'Log: ' },
    err: { ru: 'Не получилось: ', en: 'Failed: ' }, stale: { ru: 'Список изменился, введи запрос ещё раз.', en: 'The list changed, type the query again.' },
    kind: { folder: { ru: 'Папка', en: 'Folder' }, shortcut: { ru: 'Ярлык', en: 'Shortcut' }, key: { ru: 'Ключ HKCU', en: 'HKCU key' }, autorun: { ru: 'Автозапуск', en: 'Autorun' } },
    note: {
      'install folder': { ru: 'папка установки', en: 'install folder' },
      'shared with another installed app': { ru: 'общая с другой программой', en: 'shared with another installed app' },
      'named like the app': { ru: 'называется как приложение', en: 'named like the app' },
      'named like the app, under the publisher': { ru: 'как приложение, внутри папки издателя', en: 'named like the app, under the publisher' },
      'publisher folder, other apps may use it': { ru: 'папка издателя: её могут использовать другие программы', en: 'publisher folder, other apps may use it' },
      'Start Menu folder named like the app': { ru: 'папка в меню «Пуск»', en: 'Start Menu folder' },
      'points into the install folder': { ru: 'ведёт в папку установки', en: 'points into the install folder' },
      'key named like the app': { ru: 'ключ с именем приложения', en: 'key named like the app' },
      'key under the publisher': { ru: 'ключ издателя с именем приложения', en: 'key under the publisher' },
      'starts the app at sign-in': { ru: 'запускает приложение при входе', en: 'starts the app at sign-in' },
    },
  };
  const t = (o) => o[lang()] ?? o.en;
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const size = (b) => (b >= 1 << 30 ? (b / (1 << 30)).toFixed(1) + ' GB' : b >= 1 << 20 ? Math.round(b / (1 << 20)) + ' MB' : b >= 1 << 10 ? Math.round(b / 1024) + ' KB' : b ? b + ' B' : '');

  const box = el('div'); box.id = 'uni'; box.hidden = true;
  list.before(box);
  let phase = '', info = null;

  const btn = (label, cls, fn) => { const b = el('button', 'ubtn ' + (cls || ''), label); b.type = 'button'; b.addEventListener('click', fn); return b; };
  function show(...nodes) {
    box.replaceChildren(...nodes);
    box.hidden = false; list.style.display = 'none';
    box.querySelector('.ubtn.dflt')?.focus({ preventScroll: true });
  }
  function close() {
    box.hidden = true; box.replaceChildren(); list.style.display = ''; phase = ''; info = null;
    inv('uninstall_cancel');
    q.focus();
    q.dispatchEvent(new Event('input')); // search again: the list under the panel is stale
  }
  const hide = () => inv('cmd_toggle');

  async function open(it) {
    const id = parseInt(it.payload, 10);
    const r = await inv('uninstall_info', { id, name: it.title });
    if (!r || !r.ok) { q.dispatchEvent(new Event('input')); return; }
    info = r; phase = 'confirm';
    const silent = el('input'); silent.type = 'checkbox';
    const meta = [r.publisher, r.version, r.sizeKb ? size(r.sizeKb * 1024) : ''].filter(Boolean).join(' · ');
    const nodes = [el('div', 'u-h', r.name), el('div', 'u-s', r.store ? t(L.store) : meta)];
    if (r.location) nodes.push(el('div', 'u-s', t(L.folder) + r.location));
    if (r.risky) nodes.push(el('div', 'u-warn', t(L.risky)));
    if (r.canSilent) { const lab = el('label', 'u-chk'); lab.append(silent, document.createTextNode(' ' + t(L.silent))); nodes.push(lab); }
    nodes.push(el('div', 'u-note', t(L.how)));
    const bar = el('div', 'u-btns');
    bar.append(btn(t(L.uninstall), 'danger', () => { inv('uninstall_start', { silent: silent.checked }); }), btn(t(L.cancel), 'dflt', close));
    nodes.push(bar);
    show(...nodes);
  }

  function review(s) {
    const picks = [];
    const nodes = [el('div', 'u-h', t(L.leftovers) + s.name)];
    const foot = btn('', 'danger', () => { inv('uninstall_delete', { ids: picks.filter((p) => p.cb.checked).map((p) => p.id) }); });
    const upd = () => { const n = picks.filter((p) => p.cb.checked).length; foot.textContent = `${t(L.del)} (${n})`; foot.disabled = !n; };
    for (const it of s.items) {
      const row = el('label', 'u-item'), cb = el('input'); cb.type = 'checkbox'; cb.checked = !!it.ticked; cb.addEventListener('change', upd);
      picks.push({ id: it.id, cb });
      const txt = el('div', 'u-it');
      txt.append(el('div', 'u-p', it.path), el('div', 'u-s', [t(L.kind[it.kind] || { en: it.kind }), size(it.size), t(L.note[it.note] || { en: it.note })].filter(Boolean).join(' · ')));
      row.append(cb, txt); nodes.push(row);
    }
    if (!s.items.length) nodes.push(el('div', 'u-s', t(L.none)));
    if (s.manual && s.manual.length) { nodes.push(el('div', 'u-sub', t(L.manual))); for (const m of s.manual) nodes.push(el('div', 'u-p', m)); }
    if (s.cut) nodes.push(el('div', 'u-warn', t(L.cut)));
    nodes.push(el('div', 'u-note', t(L.safe)));
    const bar = el('div', 'u-btns');
    if (s.items.length) bar.append(foot);
    bar.append(btn(t(L.close), 'dflt', close));
    nodes.push(bar);
    upd();
    show(...nodes);
  }

  function render(s) {
    if (!s || !s.phase) return;
    phase = s.phase;
    const bar = el('div', 'u-btns');
    switch (s.phase) {
      case 'running':
        bar.append(btn(t(L.checkAgain), '', () => inv('uninstall_scan')), btn(t(L.hide), 'dflt', hide));
        return show(el('div', 'u-h', s.name), el('div', 'u-busy', t(L.running)), bar);
      case 'scanning': return show(el('div', 'u-h', s.name), el('div', 'u-busy', t(L.scanning)));
      case 'deleting': return show(el('div', 'u-h', s.name), el('div', 'u-busy', t(L.deleting)));
      case 'still':
        bar.append(btn(t(L.checkAgain), 'dflt', () => inv('uninstall_scan')), btn(t(L.close), '', close));
        return show(el('div', 'u-h', s.name), el('div', 'u-warn', t(L.still)), bar);
      case 'review': return review(s);
      case 'error':
        bar.append(btn(t(L.close), 'dflt', close));
        return show(el('div', 'u-h', s.name), el('div', 'u-warn', t(L.err) + (s.message || '')), bar);
      case 'done': {
        const nodes = [el('div', 'u-h', s.name), el('div', 'u-s', s.log ? t(L.done) + (s.done || 0) : t(L.none))];
        if (s.failed && s.failed.length) { nodes.push(el('div', 'u-sub', t(L.failed))); for (const f of s.failed) nodes.push(el('div', 'u-p', f)); }
        if (s.backup) nodes.push(el('div', 'u-p', t(L.backup) + s.backup));
        if (s.log) nodes.push(el('div', 'u-p', t(L.log) + s.log));
        bar.append(btn(t(L.close), 'dflt', close));
        nodes.push(bar);
        return show(...nodes);
      }
      default:
    }
  }

  T.event.listen('uninstall:state', (e) => render(e.payload));
  T.event.listen('bar:shown', async () => { const s = await inv('uninstall_state'); if (s && s.phase) render(s); });
  // while the panel is up it owns the keyboard: Esc closes it (or just hides the bar while the uninstaller runs), Enter/Tab act on its own buttons
  window.addEventListener('keydown', (e) => {
    if (box.hidden) return;
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (phase === 'running' || phase === 'scanning' || phase === 'deleting') hide(); else close(); }
    else if (['Enter', 'ArrowUp', 'ArrowDown', 'Tab', 'F4'].includes(e.key) || (e.ctrlKey && e.shiftKey)) e.stopPropagation();
  }, true);
  window.UninstallUI = { open };
})();
