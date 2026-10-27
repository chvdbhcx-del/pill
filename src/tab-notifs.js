// Notification centre tab: the history of what the island showed (finished downloads, USB, battery, calendar, mail, agents, OBS, Steam, notifications...).
// Capture: ONE wrapper around Pill.toast (every producer calls P.toast at call time), so no producer needs to know about the history.
// Storage: notifhist.rs (SQLite store.db, 200 entries / 14 days). Commands: notif_history_add / _list / _clear / _mark_read / _remove.
// Settings: module.notifs (tab on/off), history.keep (record at all), history.mail (keep mail / Windows-notification bodies; off = title only, enforced in Rust too).
// PRIVACY: local only. OCR / QR texts of the capture cards are never stored (title only).
(() => {
  const P = window.Pill;
  if (!P) return;
  const t = P.t, ic = P.icon, PM = window.PillMotion;
  const inv = (c, a) => P.invoke(c, a);
  const MAX = 200, MAX_AGE = 14 * 24 * 3600 * 1000;

  // ---- what a card is: by the id prefix the producers already use ----
  const KINDS = [['notif:', 'notif'], ['mail', 'mail'], ['filedone', 'download'], ['download:', 'download'], ['steam-done', 'steam'], ['usb:', 'usb'], ['battery', 'battery'],
    ['cal:', 'calendar'], ['agent', 'agent'], ['obs', 'obs'], ['update', 'update'], ['timer-done', 'reminder'], ['remind-', 'reminder'], ['voice', 'voice'], ['cap', 'capture']];
  const KIND_ICON = { notif: 'bell', mail: 'mail', download: 'download', steam: 'download', usb: 'usb', battery: 'battery', calendar: 'calendar', agent: 'terminal', obs: 'video', update: 'refresh', reminder: 'bell', voice: 'mic', capture: 'video' };
  const EXTRA = { // glyphs the shared set does not have
    mail: '<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="m3.5 7.5 8.5 6 8.5-6"/>',
    calendar: '<rect x="3.5" y="5" width="17" height="15" rx="2.5"/><path d="M3.5 10h17M8 3v4M16 3v4"/>',
    usb: '<path d="M12 3v13M12 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM12 11l4-3v-2M12 13l-4-3v-2"/><rect x="14.5" y="3.5" width="3" height="3" rx=".5"/><circle cx="8" cy="7" r="1.2"/>',
  };
  const SKIP = /^(clip-|launcher-|dev-settings|dict-)/; // feedback toasts of the island's own buttons are not events
  const kindOf = (id) => { const s = String(id || ''); const k = KINDS.find(([p]) => s.startsWith(p)); return k ? k[1] : (s.split(':')[0].replace(/[^a-z]/gi, '').toLowerCase().slice(0, 16) || 'event'); };
  const glyph = (name, size = 17) => (window.PillIcons && window.PillIcons[name] ? ic(name, { size, sw: 1.7 })
    : `<svg viewBox="0 0 24 24" width="${size}" height="${size}" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">${EXTRA[name] || (window.PillIcons && window.PillIcons.bell) || ''}</svg>`);
  const text = (s) => String(s == null ? '' : s).replace(/\s+/g, ' ').trim();

  // ---- state ----
  let items = [];              // newest first
  let root, listEl, countEl, clearBtn, foot;
  let shown = false, clearArm = 0, tick = 0;
  const fresh = new Set();     // ids that were unread when the tab opened (their dot stays until the tab is left)
  let ready;                   // resolves when the stored history is loaded (adds wait for it, so nothing is lost or doubled)
  const recent = new Map();    // id|title|sub -> time: a producer that repeats itself within 4 s is recorded once

  const enabled = () => P.moduleOn('module.notifs') && P.settings['history.keep'] !== false;
  const unread = () => items.reduce((n, e) => n + (e.read ? 0 : 1), 0);
  const badge = () => { const n = unread(); P.setBadge('notifs', n > 99 ? '99+' : n); };
  const alive = () => { const lim = Date.now() - MAX_AGE; return items.filter((e) => e.ts >= lim); };

  // ---- capture (the single hook) ----
  function entryOf(o) {
    if (!o || SKIP.test(String(o.id || ''))) return null;
    const kind = kindOf(o.id);
    let title = text(o.title), sub = text(o.sub);
    if (!title && o.html) { // html cards (Windows notifications): .lx-nt title, .lx-nb body; anything else: its text
      const d = document.createElement('template');
      d.innerHTML = String(o.html);
      const q = (s) => { const n = d.content.querySelector(s); return n ? text(n.textContent) : ''; };
      title = q('.lx-nt'); sub = q('.lx-nb');
      if (!title) title = text(d.content.textContent).slice(0, 200);
    }
    if (!title) return null;
    const named = typeof o.icon === 'string' && o.icon[0] !== '<' ? o.icon : '';
    const icon = named || KIND_ICON[kind] || 'bell';
    if ((kind === 'mail' || kind === 'notif') && P.settings['history.mail'] !== true) sub = ''; // the same rule runs in notifhist.rs: the body does not even cross the bridge
    if (kind === 'capture' && (icon === 'scantext' || icon === 'qr')) sub = ''; // the recognised text itself stays out of the history
    return { kind, icon, tone: o.tone || '', title, sub };
  }
  function record(o) {
    if (!enabled()) return;
    const e = entryOf(o);
    if (!e) return;
    const key = (o.id || '') + '|' + e.title + '|' + e.sub, now = Date.now();
    if (now - (recent.get(key) || 0) < 4000) return;
    recent.set(key, now);
    if (recent.size > 50) for (const [k, v] of recent) if (now - v > 4000) recent.delete(k);
    ready.then(() => inv('notif_history_add', { entry: e, read: shown })).then((s) => {
      if (!s || items.some((x) => x.id === s.id)) return;
      items.unshift(s);
      if (items.length > MAX) items.length = MAX;
      if (shown) { fresh.add(s.id); render('flip'); } else badge();
    });
  }
  const origToast = P.toast;
  P.toast = function (o) { try { record(o); } catch (e) { console.error('tab-notifs', e); } return origToast.apply(this, arguments); };

  // ---- time ----
  const clock = (ts) => new Date(ts).toLocaleTimeString(P.lang === 'ru' ? 'ru-RU' : 'en-US', { hour: '2-digit', minute: '2-digit', hour12: P.settings.clock24 === false });
  function rel(ts) {
    const s = (Date.now() - ts) / 1000;
    if (s < 45) return t({ ru: 'только что', en: 'just now' });
    const m = Math.round(s / 60);
    if (m < 60) return t({ ru: m + ' мин назад', en: m + ' min ago' });
    const h = Math.floor(m / 60);
    if (h < 24) return t({ ru: h + ' ч назад', en: h + ' h ago' });
    return clock(ts);
  }
  const dayStart = (ts) => { const d = new Date(ts); d.setHours(0, 0, 0, 0); return d.getTime(); };
  function dayLabel(d0) {
    const today = dayStart(Date.now());
    if (d0 === today) return t({ ru: 'Сегодня', en: 'Today' });
    if (d0 === dayStart(today - 1)) return t({ ru: 'Вчера', en: 'Yesterday' });
    return new Date(d0).toLocaleDateString(P.lang === 'ru' ? 'ru-RU' : 'en-US', { day: 'numeric', month: 'long' });
  }

  // ---- view ----
  const el = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  function row(e, i, stag) {
    const r = el('div', 'nf-row' + (stag ? ' stag' : '') + (fresh.has(e.id) ? ' unread' : ''));
    r.dataset.id = e.id; r.dataset.kind = e.kind;
    if (e.tone) r.dataset.tone = e.tone;
    if (stag) r.style.setProperty('--i', i);
    const tile = el('span', 'nf-tile'); tile.innerHTML = glyph(e.icon || KIND_ICON[e.kind] || 'bell');
    const body = el('div', 'nf-body');
    body.append(el('div', 'nf-t', e.title));
    if (e.sub) body.append(el('div', 'nf-s', e.sub));
    const end = el('div', 'nf-end');
    const tm = el('time', 'nf-time', rel(e.ts)); tm.title = new Date(e.ts).toLocaleString(P.lang === 'ru' ? 'ru-RU' : 'en-US');
    const x = el('button', 'nf-x'); x.type = 'button'; x.dataset.act = 'rm'; x.innerHTML = ic('x', { size: 14, sw: 2 });
    x.title = x.ariaLabel = t({ ru: 'Удалить', en: 'Remove' });
    end.append(tm, x);
    r.append(el('i', 'nf-dot'), tile, body, end);
    return r;
  }
  function build(stag) {
    const kids = []; let day = null, i = 0;
    for (const e of alive()) {
      const d0 = dayStart(e.ts);
      if (d0 !== day) { day = d0; kids.push(el('div', 'nf-day label-caps', dayLabel(d0))); }
      kids.push(row(e, i++, stag));
    }
    listEl.replaceChildren(...kids);
  }
  function status() {
    const off = P.settings['history.keep'] === false, n = alive().length;
    root.classList.toggle('empty', !n);
    root.classList.toggle('off', off);
    countEl.textContent = off ? t({ ru: 'История выключена', en: 'History is off' }) : n ? t({ ru: 'Всего: ', en: 'Total: ' }) + n : t({ ru: 'Уведомления', en: 'Notifications' });
    clearBtn.hidden = !n;
    foot.textContent = off ? t({ ru: 'Запись выключена: Настройки → Модули → «Хранить историю».', en: 'Recording is off: Settings → Modules → “Keep history”.' })
      : t({ ru: 'Только на этом ПК · 14 дней, до 200 записей', en: 'Stored on this PC only · 14 days, up to 200 entries' });
  }
  function render(mode) {
    if (!root) return;
    status();
    if (mode === 'flip' && PM) PM.flip(listEl, () => build(false), { sel: '.nf-row', enterNew: true }); else build(mode === 'stag');
    badge();
  }

  function disarm() { clearTimeout(clearArm); clearArm = 0; clearBtn.classList.remove('armed'); clearBtn.lastChild.textContent = t({ ru: 'Очистить историю', en: 'Clear history' }); }
  async function clearAll() {
    const rows = [...listEl.querySelectorAll('.nf-row')];
    const gone = PM ? Promise.all(rows.map((r, i) => PM.exit(r, { y: -4, delay: Math.min(i, 8) * 0.02 }))) : Promise.resolve();
    await inv('notif_history_clear');
    items = []; fresh.clear();
    await gone;
    render('');
    if (PM) PM.enter(root.querySelector('.nf-empty'), { y: 6 });
  }
  async function removeOne(r) {
    const id = +r.dataset.id;
    items = items.filter((e) => e.id !== id); fresh.delete(id);
    badge();
    if (PM) await PM.collapse(r, { remove: true }); else r.remove();
    inv('notif_history_remove', { id });
    render(''); // an emptied day header goes with its last row; the rows below already sit where they will be
    if (!items.length && PM) PM.enter(root.querySelector('.nf-empty'), { y: 6 });
  }

  function mount(r) {
    root = r; root.classList.add('nf-root');
    const head = el('div', 'nf-head');
    countEl = el('span', 'nf-count label-caps');
    clearBtn = el('button', 'nf-clear btn-ghost'); clearBtn.type = 'button';
    clearBtn.innerHTML = ic('trash', { size: 15 }) + '<span></span>';
    clearBtn.lastChild.textContent = t({ ru: 'Очистить историю', en: 'Clear history' });
    head.append(countEl, clearBtn);
    listEl = el('div', 'nf-list scroll');
    const empty = el('div', 'nf-empty');
    empty.innerHTML = `<span class="nf-eico">${ic('bell', { size: 26, sw: 1.5 })}</span>`;
    empty.append(el('b', '', t({ ru: 'Пока ничего нет', en: 'Nothing here yet' })),
      el('span', '', t({ ru: 'Здесь останутся загрузки, события и агенты, о которых сообщал остров', en: 'Downloads, events and agents the island told you about will stay here' })));
    foot = el('div', 'nf-foot');
    root.append(head, listEl, empty, foot);
    clearBtn.addEventListener('click', () => {
      if (!clearArm) { // two clicks within 3 s, like the clipboard tab
        clearBtn.classList.add('armed'); clearBtn.lastChild.textContent = t({ ru: 'Нажми ещё раз', en: 'Click again' });
        clearArm = setTimeout(disarm, 3000); return;
      }
      disarm(); clearAll();
    });
    listEl.addEventListener('click', (e) => { const b = e.target.closest('[data-act="rm"]'); if (b) removeOne(b.closest('.nf-row')); });
    ready = inv('notif_history_list').then((l) => { items = Array.isArray(l) ? l : []; render(''); }, () => {});
    P.on('settings:changed', () => { if (root) { status(); badge(); } });
  }
  function onShow() {
    shown = true;
    fresh.clear();
    const ids = items.filter((e) => !e.read).map((e) => e.id);
    ids.forEach((id) => fresh.add(id));
    if (ids.length) { items.forEach((e) => { e.read = true; }); inv('notif_history_mark_read', { ids }); }
    render('stag');
    clearInterval(tick); tick = setInterval(() => {
      const by = new Map(items.map((e) => [e.id, e]));
      listEl.querySelectorAll('.nf-row').forEach((r) => { const e = by.get(+r.dataset.id); if (e) r.querySelector('time').textContent = rel(e.ts); });
    }, 20000);
  }
  function onHide() { shown = false; clearInterval(tick); disarm(); fresh.clear(); }

  P.register({ id: 'notifs', title: t({ ru: 'Уведомления', en: 'Notifications' }), order: 65, icon: ic('bell', { cls: 'nf-ic' }), mount, onShow, onHide });
})();
