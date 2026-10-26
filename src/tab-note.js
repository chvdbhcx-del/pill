// Note tab: notes with a rich-text editor (autosave) on the left, timer ring + reminders on the right.
// Rust: notes.rs (note_*), timers.rs (remind_*, timer_*; events remind:due, remind:changed, timer:tick, timer:done).
// Owner: text/note agent. Request: text, note, timer and reminder tabs for the Pill replica.
(() => {
  'use strict';
  const t = (d) => (window.Pill && window.Pill.t ? window.Pill.t(d) : d.en); // evaluated lazily
  const isRu = () => ((window.Pill && window.Pill.lang) || 'en') === 'ru';
  const el = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const svg = (d) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
  const ICON = {
    tab: svg('<rect x="5" y="4" width="13" height="16" rx="2.5"/><path d="M9 8.5h5M9 12.5h5M9 16.5h3M20.5 6.5l-5.5 5.5"/>'),
    bell: svg('<path d="M6 16v-5a6 6 0 0 1 12 0v5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>'),
    play: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 4.5v15l12-7.5z"/></svg>',
    stop: '<svg viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>',
    pencil: svg('<path d="M4 20l1-4L16 5l3 3L8 19z"/>'),
    check: svg('<path d="M5 12.5l4.5 4.5L19 7.5"/>'),
    trash: svg('<path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13"/>'),
    copy: svg('<rect x="9" y="9" width="11" height="11" rx="2.5"/><path d="M15 9V6.5A2.5 2.5 0 0 0 12.5 4h-6A2.5 2.5 0 0 0 4 6.5v6A2.5 2.5 0 0 0 6.5 15H9"/>'),
    x: svg('<path d="M6 6l12 12M18 6 6 18"/>'),
    clock: svg('<circle cx="12" cy="12" r="8"/><path d="M12 8v4l3 2"/>'),
    plus: svg('<path d="M12 6v12M6 12h12"/>'),
    mark: svg('<path d="M9 14l6-6 3 3-6 6H9z"/><path d="M4 20h9"/>'),
    ul: svg('<path d="M9.5 7H20M9.5 12H20M9.5 17H20"/><circle cx="5" cy="7" r="1" fill="currentColor"/><circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="5" cy="17" r="1" fill="currentColor"/>'),
    ol: svg('<path d="M10 7h10M10 12h10M10 17h10M4 5.5l1.5-.7V10M4 15.2c0-1.2 2.5-1.2 2.5 0S4 17 4 18.2h2.6"/>'),
    todo: svg('<path d="M3.5 7l1.5 1.5L8 5.5M3.5 16.5L5 18l3-3M12 7h8M12 16.5h8"/>'),
    live: svg('<circle cx="12" cy="13" r="7"/><path d="M12 9.500V13l2.500 1.500M10 3h4"/>'),
  };

  // ================= helpers (pure) =================
  const ALLOWED = new Set(['B', 'I', 'S', 'MARK', 'UL', 'OL', 'LI', 'DIV', 'BR', 'A', 'INPUT']);
  const RENAME = { STRONG: 'B', EM: 'I', STRIKE: 'S', DEL: 'S', P: 'DIV', H1: 'DIV', H2: 'DIV', H3: 'DIV', H4: 'DIV', H5: 'DIV', H6: 'DIV', BLOCKQUOTE: 'DIV', PRE: 'DIV', TR: 'DIV' };
  const DROP = new Set(['SCRIPT', 'STYLE', 'IFRAME', 'OBJECT', 'EMBED', 'LINK', 'META', 'TEMPLATE', 'NOSCRIPT', 'SVG', 'MATH', 'FORM', 'TEXTAREA', 'SELECT', 'BUTTON', 'IMG', 'VIDEO', 'AUDIO', 'CANVAS', 'HEAD', 'TITLE']);

  /** Whitelist sanitizer: b i s mark ul ol li div br a input[type=checkbox]; no styles, classes, handlers or scripts. */
  function sanitize(html) {
    const src = new DOMParser().parseFromString('<!doctype html><body>' + String(html || ''), 'text/html').body; // inert document
    const out = document.createElement('div');
    (function walk(node, dst) {
      for (const c of node.childNodes) {
        if (c.nodeType === 3) { dst.append(document.createTextNode(c.nodeValue)); continue; }
        if (c.nodeType !== 1 || DROP.has(c.tagName)) continue;
        const tag = RENAME[c.tagName] || c.tagName;
        if (!ALLOWED.has(tag)) { walk(c, dst); continue; } // unknown tag: keep its text, drop the tag
        if (tag === 'INPUT') {
          if ((c.getAttribute('type') || '').toLowerCase() !== 'checkbox') continue;
          const i = document.createElement('input'); i.type = 'checkbox';
          if (c.hasAttribute('checked')) i.setAttribute('checked', '');
          dst.append(i); continue;
        }
        const e = document.createElement(tag.toLowerCase());
        if (tag === 'A') { const h = (c.getAttribute('href') || '').trim(); if (/^(https?:\/\/|mailto:)/i.test(h)) e.setAttribute('href', h); }
        walk(c, e); dst.append(e);
      }
    })(src, out);
    return out.innerHTML;
  }

  /** "12" = minutes, "1:30" = 1 min 30 s, "1:02:03" = h:mm:ss, "90s", "1h30m", "2 мин". Returns seconds or 0. */
  function parseDur(s) {
    s = String(s || '').trim().toLowerCase();
    let m;
    if ((m = /^(\d+)$/.exec(s))) return +m[1] * 60;
    if ((m = /^(\d+):(\d{1,2})$/.exec(s))) return +m[1] * 60 + +m[2];
    if ((m = /^(\d+):(\d{1,2}):(\d{1,2})$/.exec(s))) return +m[1] * 3600 + +m[2] * 60 + +m[3];
    m = /^(?:(\d+)\s*(?:h|ч)[a-zа-я]*)?\s*(?:(\d+)\s*(?:m|м)[a-zа-я]*)?\s*(?:(\d+)\s*(?:s|с)[a-zа-я]*)?$/.exec(s);
    return m && (m[1] || m[2] || m[3]) ? (+m[1] || 0) * 3600 + (+m[2] || 0) * 60 + (+m[3] || 0) : 0;
  }
  const fmtClock = (sec) => {
    sec = Math.max(0, Math.ceil(sec));
    const h = Math.floor(sec / 3600), m = Math.floor((sec % 3600) / 60), s = sec % 60;
    return h ? `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}` : `${m}:${String(s).padStart(2, '0')}`;
  };
  if (typeof window === 'undefined') return;
  window.PillNoteTools = { sanitize, parseDur, fmtClock };
  if (!window.Pill || !window.Pill.register) return;

  // ================= backend =================
  async function call(cmd, args) { // like Pill.invoke but keeps the error text (remind_add returns "no_time" / "past")
    const T = window.__TAURI__;
    if (!(T && T.core)) return { ok: false, err: 'no tauri' };
    try { return { ok: true, v: await T.core.invoke(cmd, args) }; } catch (e) { return { ok: false, err: String(e) }; }
  }
  const store = {
    get(k, d) { try { const v = localStorage.getItem(k); return v == null ? d : v; } catch (e) { return d; } },
    set(k, v) { try { localStorage.setItem(k, v); } catch (e) { /* storage blocked */ } },
  };

  let ui = {};      // elements
  let notes = [];   // [{id, title}]
  let curId = null, dirty = false, saveT = 0, saving = false;
  let timer = { active: false, id: 0, dueAt: 0, totalS: 0 };
  let selSec = Math.max(1, +store.get('pill.timer.sel', 300) || 300);
  const PM = window.PillMotion;
  let remKnown = false; // false until the first reminders draw of a visit (no enter animation then)
  let tickI = 0, liveSec = -1, zeroSynced = false, visible = false, hid = false, remI = 0, delArm = 0, toastT = 0;

  function say(msg, bad) {
    ui.toast.textContent = msg; ui.toast.classList.toggle('bad', !!bad); ui.toast.classList.add('show');
    clearTimeout(toastT); toastT = setTimeout(() => ui.toast.classList.remove('show'), 2000);
  }

  // ================= notes =================
  const emptyTitle = () => t({ ru: 'Заметка', en: 'Note' });
  const firstLine = () => (ui.ed.innerText || '').split('\n').map((x) => x.trim()).find(Boolean) || '';
  function setEmptyClass() { ui.wrap.classList.toggle('empty', !ui.ed.textContent.trim() && !ui.ed.querySelector('li,input')); }

  function drawChips() {
    ui.chips.replaceChildren();
    notes.forEach((n) => {
      const b = el('button', 'tn-chip' + (n.id === curId ? ' on' : ''));
      b.append(el('span', 'tn-tx', n.title || emptyTitle()));
      b.type = 'button'; b.title = n.title || emptyTitle();
      b.onclick = () => openNote(n.id);
      ui.chips.append(b);
    });
    const add = el('button', 'tn-add'); add.type = 'button'; add.innerHTML = ICON.plus; add.title = t({ ru: 'Новая заметка', en: 'New note' });
    add.onclick = newNote;
    ui.chips.append(add);
    const on = ui.chips.querySelector('.on'); if (on) on.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  }
  function setSaved(state) { // 'saved' | 'saving' | 'error'
    ui.saved.dataset.s = state;
    ui.saved.title = state === 'saved' ? t({ ru: 'Сохранено', en: 'Saved' }) : state === 'saving' ? t({ ru: 'Сохраняется…', en: 'Saving…' }) : t({ ru: 'Не удалось сохранить', en: 'Could not save' });
  }

  async function save() {
    clearTimeout(saveT);
    if (curId == null || !dirty) return;
    const id = curId, html = sanitize(ui.ed.innerHTML), title = firstLine().slice(0, 60);
    dirty = false; saving = true; setSaved('saving');
    const r = await call('note_save', { id, body_html: html, title });
    saving = false;
    if (!r.ok || !r.v) { dirty = true; setSaved('error'); return; }
    const n = notes.find((x) => x.id === id);
    if (n && n.title !== r.v.title) { n.title = r.v.title; drawChips(); }
    if (!dirty) setSaved('saved');
  }
  function touch() {
    dirty = true; setEmptyClass(); setSaved('saving');
    clearTimeout(saveT); saveT = setTimeout(save, 400);
  }
  async function openNote(id) {
    if (id === curId) return;
    await save();
    const r = await call('note_get', { id });
    if (!r.ok || !r.v) return;
    curId = id; store.set('pill.note.cur', id);
    ui.ed.innerHTML = sanitize(r.v.body_html);
    dirty = false; setEmptyClass(); setSaved('saved'); drawChips();
  }
  async function newNote() {
    await save();
    const r = await call('note_save', { id: null, body_html: '', title: '' });
    if (!r.ok || !r.v) return say(t({ ru: 'Не удалось создать заметку', en: 'Could not create a note' }), true);
    notes.push({ id: r.v.id, title: '' });
    curId = null; await openNote(r.v.id);
    ui.ed.focus();
  }
  async function loadNotes() {
    const r = await call('note_list', { folder_id: null });
    if (!r.ok) return;
    notes = (r.v || []).map((n) => ({ id: n.id, title: n.title }));
    if (!notes.length) return newNote();
    const want = +store.get('pill.note.cur', 0);
    curId = null; await openNote(notes.some((n) => n.id === want) ? want : notes[0].id);
  }
  async function deleteNote() {
    if (!delArm) { // two clicks: the first arms the button for 2.5 s
      ui.del.classList.add('arm'); ui.del.title = t({ ru: 'Нажми ещё раз, чтобы удалить', en: 'Click again to delete' });
      delArm = setTimeout(disarm, 2500); return;
    }
    disarm();
    const id = curId, i = notes.findIndex((n) => n.id === id);
    clearTimeout(saveT); dirty = false;
    await call('note_delete', { id });
    notes = notes.filter((n) => n.id !== id); curId = null;
    if (!notes.length) return newNote();
    await openNote(notes[Math.min(i, notes.length - 1)].id);
  }
  function disarm() { clearTimeout(delArm); delArm = 0; ui.del.classList.remove('arm'); ui.del.title = t({ ru: 'Удалить заметку', en: 'Delete note' }); }

  // ---- editor commands ----
  const exec = (cmd, arg) => { ui.ed.focus(); document.execCommand(cmd, false, arg); touch(); syncTools(); };
  function highlight() {
    ui.ed.focus();
    const sel = getSelection();
    if (!sel.rangeCount) return;
    const r = sel.getRangeAt(0);
    const node = r.commonAncestorContainer.nodeType === 1 ? r.commonAncestorContainer : r.commonAncestorContainer.parentElement;
    const mark = node && node.closest && node.closest('mark');
    if (mark && ui.ed.contains(mark)) { // toggle off
      const range = document.createRange(); range.selectNode(mark); sel.removeAllRanges(); sel.addRange(range);
      document.execCommand('insertHTML', false, mark.innerHTML);
    } else if (!r.collapsed) {
      const box = document.createElement('div'); box.append(r.cloneContents());
      document.execCommand('insertHTML', false, '<mark>' + sanitize(box.innerHTML) + '</mark>');
    }
    touch(); syncTools();
  }
  function checklist() {
    ui.ed.focus();
    document.execCommand('insertHTML', false, '<input type="checkbox">&nbsp;');
    touch();
  }
  function syncTools() {
    if (!ui.tools) return;
    const q = (c) => { try { return document.queryCommandState(c); } catch (e) { return false; } };
    const sel = getSelection(), n = sel && sel.anchorNode && (sel.anchorNode.nodeType === 1 ? sel.anchorNode : sel.anchorNode.parentElement);
    const inEd = !!(n && ui.ed.contains(n));
    ui.tools.bold.classList.toggle('on', inEd && q('bold'));
    ui.tools.italic.classList.toggle('on', inEd && q('italic'));
    ui.tools.strike.classList.toggle('on', inEd && q('strikeThrough'));
    ui.tools.mark.classList.toggle('on', inEd && !!(n.closest && n.closest('mark')));
    ui.tools.ul.classList.toggle('on', inEd && q('insertUnorderedList'));
    ui.tools.ol.classList.toggle('on', inEd && q('insertOrderedList'));
  }
  function lineStartsWithCheckbox() {
    const sel = getSelection();
    if (!sel.rangeCount || !sel.isCollapsed) return null;
    let n = sel.anchorNode;
    while (n && n.parentNode !== ui.ed && n !== ui.ed) n = n.parentNode;
    if (!n || n === ui.ed) return null;
    const first = n.firstChild;
    return first && first.nodeType === 1 && first.tagName === 'INPUT' ? n : null;
  }
  async function copyNote() {
    const txt = (ui.ed.innerText || '').trim();
    if (!txt) return;
    try { await navigator.clipboard.writeText(txt); } catch (e) { const r = document.createRange(); r.selectNodeContents(ui.ed); const s = getSelection(); s.removeAllRanges(); s.addRange(r); document.execCommand('copy'); s.removeAllRanges(); }
    say(t({ ru: 'Скопировано', en: 'Copied' }));
  }
  function remindFromNote() {
    const sel = getSelection(), picked = sel && !sel.isCollapsed && ui.ed.contains(sel.anchorNode) ? sel.toString().trim() : '';
    const txt = (picked || firstLine()).replace(/\s+/g, ' ').slice(0, 80);
    ui.rin.value = txt ? txt + ' ' : '';
    ui.rin.focus(); ui.rin.setSelectionRange(ui.rin.value.length, ui.rin.value.length);
    showMsg(t({ ru: 'Допиши, когда напомнить: «завтра в 9»', en: 'Add when to remind: "tomorrow at 9"' }));
  }

  // ================= timer =================
  const CIRC = 2 * Math.PI * 34;
  function remainingS() { return timer.active ? Math.max(0, (timer.dueAt - Date.now()) / 1000) : 0; }
  function paintTimer() {
    const run = timer.active, rem = remainingS();
    const clock = fmtClock(run ? rem : selSec);
    ui.timeTx.textContent = clock; ui.time.dataset.len = String(clock.length); // font size steps with the length, the box never reflows
    ui.arc.style.strokeDashoffset = run ? String(CIRC * (1 - rem / Math.max(1, timer.totalS))) : '0';
    ui.ring.classList.toggle('run', run);
    ui.start.innerHTML = (run ? ICON.stop : ICON.play) + '<span>' + (run ? t({ ru: 'Стоп', en: 'Stop' }) : t({ ru: 'Старт', en: 'Start' })) + '</span>';
    ui.start.classList.toggle('stop', run);
    ui.presets.querySelectorAll('.tn-p').forEach((b) => b.classList.toggle('on', !run && +b.dataset.m * 60 === selSec));
    const sec = run ? Math.ceil(rem) : -1;
    if (sec !== liveSec) { // collapsed pill shows the running timer
      liveSec = sec;
      // key = state + clock length: the chip is built once per run (and when the clock gains / loses a digit), the seconds are then written into it in place
      window.Pill.setLive('timer', run ? { priority: 60, key: 'run' + clock.length, html: `<span class="tn-live">${ICON.live}<b>${clock}</b></span>` } : null);
      const b = run && document.querySelector('#live-in .tn-live b'); // a new html string would replay the blur swap every second
      if (b && b.textContent !== clock) b.textContent = clock;
    }
  }
  function armTick() { // 4 Hz while the Note tab shows the ring (its stroke eases over .26 s), else once per displayed second; nothing while the pill is hidden
    clearTimeout(tickI); tickI = 0;
    if (!timer.active || hid) return;
    const left = timer.dueAt - Date.now();
    tickI = setTimeout(tick, visible || left <= 0 ? 250 : (left % 1000 || 1000) + 20);
  }
  function tick() {
    tickI = 0;
    if (!timer.active) return;
    paintTimer();
    if (remainingS() <= 0 && !zeroSynced) { zeroSynced = true; setTimeout(syncTimer, 1500); } // Rust fires timer:done; resync if the event was missed
    armTick();
  }
  function applyTimer(s) {
    timer = s && s.active ? { active: true, id: s.id, dueAt: s.due_at, totalS: s.total_s } : { active: false, id: 0, dueAt: 0, totalS: 0 };
    zeroSynced = false;
    paintTimer();
    armTick();
  }
  async function syncTimer() { const r = await call('timer_state'); if (r.ok) applyTimer(r.v); }
  async function startStop() {
    if (timer.active) { const r = await call('timer_cancel'); applyTimer(r.ok ? r.v : null); return; }
    const r = await call('timer_start', { seconds: selSec });
    if (r.ok) applyTimer(r.v); else say(t({ ru: 'Таймер недоступен', en: 'Timer unavailable' }), true);
  }
  function setSel(sec) { selSec = Math.min(99 * 3600, Math.max(1, sec)); store.set('pill.timer.sel', selSec); paintTimer(); }
  function customTime() {
    const inp = el('input', 'tn-custom'); inp.type = 'text'; inp.spellcheck = false;
    inp.placeholder = t({ ru: '12, 1:30, 90с', en: '12, 1:30, 90s' });
    const btn = ui.custom;
    btn.replaceWith(inp); inp.focus();
    let done = false;
    const end = (apply) => {
      if (done) return; done = true;
      if (apply) { const s = parseDur(inp.value); if (s) setSel(s); else if (inp.value.trim()) say(t({ ru: 'Не понял время', en: 'Could not read that time' }), true); }
      inp.replaceWith(btn);
    };
    inp.onkeydown = (e) => { if (e.key === 'Enter') end(true); else if (e.key === 'Escape') { e.stopPropagation(); end(false); } };
    inp.onblur = () => end(true);
  }

  // ================= reminders =================
  const loc = () => (isRu() ? 'ru-RU' : 'en-US');
  const hour12 = () => !(window.Pill && window.Pill.settings && window.Pill.settings.clock24 !== false);
  const hm = (d) => d.toLocaleTimeString(loc(), { hour: 'numeric', minute: '2-digit', hour12: hour12() });
  function fmtDue(ms) {
    const now = new Date(), d = new Date(ms), diff = ms - now.getTime();
    if (diff <= 0) return t({ ru: 'просрочено', en: 'overdue' }) + ' · ' + hm(d);
    if (diff < 3600e3) { const m = Math.max(1, Math.round(diff / 60e3)); return isRu() ? `через ${m} мин` : `in ${m} min`; }
    const day = (x) => Math.floor(new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime() / 864e5);
    const k = day(d) - day(now);
    if (k === 0) return hm(d);
    if (k === 1) return t({ ru: 'завтра', en: 'tomorrow' }) + ' ' + hm(d);
    if (k < 7) return d.toLocaleDateString(loc(), { weekday: 'short' }) + ' ' + hm(d);
    return d.toLocaleDateString(loc(), { day: 'numeric', month: 'short' }) + ' ' + hm(d);
  }
  function fmtRepeat(rule) {
    if (!rule || rule === 'none') return '';
    if (rule === 'daily') return t({ ru: 'каждый день', en: 'daily' });
    if (rule === 'weekly') return t({ ru: 'каждую неделю', en: 'weekly' });
    if (rule === 'monthly') return t({ ru: 'каждый месяц', en: 'monthly' });
    const m = +rule.split(':')[1] || 0;
    if (m && m % 1440 === 0) return isRu() ? `каждые ${m / 1440} дн.` : `every ${m / 1440} d`;
    if (m && m % 60 === 0) return isRu() ? `каждые ${m / 60} ч` : `every ${m / 60} h`;
    return isRu() ? `каждые ${m} мин` : `every ${m} min`;
  }
  let msgT = 0;
  function showMsg(txt, bad) {
    PM.cancel(ui.rmsg); // a running exit is dropped: the line stays
    ui.rmsg.textContent = txt; ui.rmsg.classList.toggle('bad', !!bad); ui.rmsg.classList.add('show');
    clearTimeout(msgT); msgT = setTimeout(() => PM.exit(ui.rmsg, { y: -4, after: (e) => e.classList.remove('show') }), 3500);
  }

  async function drawReminders() {
    const r = await call('remind_list');
    const list = (r.ok && r.v) || [];
    const rows = [], wasEmpty = ui.rlist.style.display === 'none';
    ui.rempty.style.display = list.length ? 'none' : '';
    ui.rlist.style.display = list.length ? '' : 'none';
    if (remKnown) { if (list.length && wasEmpty) PM.enter(ui.rlist, { y: 6 }); else if (!list.length && !wasEmpty) PM.enter(ui.rempty, { y: 6 }); }
    ui.rhead.dataset.n = list.length ? String(list.length) : '';
    list.forEach((x) => {
      const row = el('div', 'tn-r' + (x.fired ? ' od' : ''));
      row.dataset.id = x.id;
      const ic = el('i', 'tn-r-ic'); ic.innerHTML = ICON.bell;
      const main = el('div', 'tn-r-main'), txt = el('div', 'tn-r-text', x.text), meta = el('div', 'tn-r-meta');
      txt.title = x.text;
      meta.append(el('span', '', fmtDue(x.due_at)));
      const rep = fmtRepeat(x.repeat_rule); if (rep) meta.append(el('b', 'tn-badge', '↻ ' + rep));
      main.append(txt, meta);
      const act = el('div', 'tn-r-act');
      const mk = (icon, title, fn) => { const b = el('button'); b.type = 'button'; b.innerHTML = icon; b.title = title; b.onclick = fn; act.append(b); };
      mk(ICON.check, t({ ru: 'Готово', en: 'Done' }), () => { const rep = x.repeat_rule && x.repeat_rule !== 'none'; const gone = rep ? null : PM.collapse(row, { remove: true }); return call('remind_done', { id: x.id }).then(() => gone).then(drawReminders); });
      mk(ICON.clock, t({ ru: 'Отложить на 10 мин', en: 'Snooze 10 min' }), () => call('remind_snooze', { id: x.id, minutes: 10 }).then(drawReminders));
      mk(ICON.x, t({ ru: 'Удалить', en: 'Delete' }), () => { const gone = PM.collapse(row, { remove: true }); return call('remind_delete', { id: x.id }).then(() => gone).then(drawReminders); });
      row.append(ic, main, act);
      rows.push(row);
    });
    // rows that stay slide to their new place; a reminder that was not there before drops in (not on the first draw of a visit)
    PM.flip(ui.rlist, () => ui.rlist.replaceChildren(...rows), { sel: '.tn-r', enterNew: remKnown });
    remKnown = true;
  }
  async function addReminder() {
    const text = ui.rin.value.trim();
    if (!text) return;
    const r = await call('remind_add', { text });
    if (r.ok) {
      ui.rin.value = '';
      showMsg((isRu() ? 'Напомню: ' : 'Reminder set: ') + fmtDue(r.v.due_at));
      drawReminders();
    } else if (/past/.test(r.err)) showMsg(t({ ru: 'Это время уже прошло', en: 'That moment has already passed' }), true);
    else if (/no_time/.test(r.err)) showMsg(t({ ru: 'Не нашёл время. Например: «завтра в 9» или «через 20 минут»', en: 'No time found. Try "tomorrow at 9" or "in 20 minutes"' }), true);
    else showMsg(t({ ru: 'Не удалось сохранить напоминание', en: 'Could not save the reminder' }), true);
  }

  // card in the collapsed pill when something fires
  function onDue(p) {
    if (!p) return;
    const cards = !window.Pill.liveOn || window.Pill.liveOn('reminders'); // Settings > Live events > Reminders and timers: off = no card, no edge reaction
    if (cards) window.Pill.react('remind', 'attention', { ttlMs: 5000 }); // timer or reminder fired
    const id = p.id, late = p.late ? (isRu() ? 'пропущено · ' : 'missed · ') : '';
    if (p.kind === 'timer') {
      applyTimer(null);
      if (cards) window.Pill.toast({ id: 'timer-done', ttlMs: 20000, icon: 'bell', title: t({ ru: 'Таймер закончился', en: 'Timer finished' }), actions: [
        { label: t({ ru: 'Ещё раз', en: 'Again' }), run: startStop },
        { label: t({ ru: 'Ок', en: 'OK' }), run: () => {} },
      ] });
    } else {
      const repeating = p.repeat && p.repeat !== 'none';
      if (cards) window.Pill.toast({ id: 'remind-' + id, ttlMs: 30000, icon: 'bell', title: p.text, sub: late ? late.replace(/ · $/, '') : repeating ? fmtRepeat(p.repeat) : '', actions: [
        { label: t({ ru: 'Отложить 10 мин', en: 'Snooze 10 min' }), run: () => call('remind_snooze', { id, minutes: 10 }).then(drawReminders) },
        { label: t({ ru: 'Готово', en: 'Done' }), run: () => (repeating ? null : call('remind_done', { id }).then(drawReminders)) },
      ] });
    }
    drawReminders();
  }

  // ================= mount =================
  function mount(view) {
    const root = el('div', 'tab-note'); view.append(root);
    const left = el('div', 'tn-left'), right = el('div', 'tn-right');
    root.append(left, right);

    // ---- left: chips, editor, toolbar ----
    ui.chips = el('div', 'tn-chips');
    ui.wrap = el('div', 'tn-edwrap empty');
    ui.ed = el('div', 'tn-ed'); ui.ed.contentEditable = 'true'; ui.ed.spellcheck = false;
    ui.ed.setAttribute('role', 'textbox'); ui.ed.setAttribute('aria-multiline', 'true');
    const ph = el('div', 'tn-ph', t({ ru: 'Пиши сюда что угодно: номер заказа, список дел, кусок кода. Сохраняется само.', en: 'Write anything: order number, to-do list, a piece of code. Saves itself.' }));
    ui.toast = el('div', 'tn-toast');
    ui.wrap.append(ui.ed, ph, ui.toast);

    const bar = el('div', 'tn-bar');
    ui.tools = {};
    const tool = (key, html, title, fn) => {
      const b = el('button', 'tn-t'); b.type = 'button'; b.innerHTML = html; b.title = title;
      b.onmousedown = (e) => e.preventDefault(); // keep the selection in the editor
      b.onclick = fn; ui.tools[key] = b; bar.append(b); return b;
    };
    tool('bold', '<b>B</b>', t({ ru: 'Жирный (Ctrl+B)', en: 'Bold (Ctrl+B)' }), () => exec('bold'));
    tool('italic', '<i>I</i>', t({ ru: 'Курсив (Ctrl+I)', en: 'Italic (Ctrl+I)' }), () => exec('italic'));
    tool('strike', '<s>S</s>', t({ ru: 'Зачёркнутый', en: 'Strikethrough' }), () => exec('strikeThrough'));
    tool('mark', ICON.mark, t({ ru: 'Выделить маркером', en: 'Highlight' }), highlight);
    bar.append(el('i', 'tn-sep'));
    tool('ul', ICON.ul, t({ ru: 'Список', en: 'Bullet list' }), () => exec('insertUnorderedList'));
    tool('ol', ICON.ol, t({ ru: 'Нумерованный список', en: 'Numbered list' }), () => exec('insertOrderedList'));
    tool('todo', ICON.todo, t({ ru: 'Чек-лист', en: 'Checklist' }), checklist);
    bar.append(el('span', 'tn-grow'));
    ui.saved = el('span', 'tn-saved'); ui.saved.innerHTML = ICON.check; ui.saved.dataset.s = 'saved';
    bar.append(ui.saved);
    const rb = el('button', 'tn-remind'); rb.type = 'button'; rb.onmousedown = (e) => e.preventDefault();
    rb.innerHTML = ICON.bell + '<span></span>'; rb.querySelector('span').textContent = t({ ru: 'Напомнить', en: 'Remind' });
    rb.title = t({ ru: 'Создать напоминание из заметки', en: 'Create a reminder from this note' });
    rb.onclick = remindFromNote; bar.append(rb);
    tool('copy', ICON.copy, t({ ru: 'Копировать текст', en: 'Copy text' }), copyNote);
    ui.del = tool('del', ICON.trash, t({ ru: 'Удалить заметку', en: 'Delete note' }), deleteNote);
    left.append(ui.chips, ui.wrap, bar);

    // editor events
    ui.ed.addEventListener('input', touch);
    ui.ed.addEventListener('change', (e) => { const c = e.target; if (c && c.matches && c.matches('input[type=checkbox]')) { c.toggleAttribute('checked', c.checked); touch(); } });
    ui.ed.addEventListener('click', (e) => {
      const a = e.target.closest && e.target.closest('a');
      if (a) { e.preventDefault(); if ((e.ctrlKey || e.metaKey) && a.href) window.Pill.invoke('chat_open_link', { url: a.href }); }
    });
    ui.ed.addEventListener('paste', (e) => {
      e.preventDefault();
      const cd = e.clipboardData, html = cd && cd.getData('text/html'), txt = cd ? cd.getData('text/plain') : '';
      if (html) document.execCommand('insertHTML', false, sanitize(html));
      else if (txt) document.execCommand('insertText', false, txt);
      touch();
    });
    ui.ed.addEventListener('drop', (e) => e.preventDefault());
    ui.ed.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
      const line = lineStartsWithCheckbox();
      if (!line) return;
      e.preventDefault();
      if (!line.textContent.replace(/ /g, '').trim()) { line.firstChild.remove(); touch(); return; } // empty checklist line: Enter ends the list
      document.execCommand('insertParagraph');
      document.execCommand('insertHTML', false, '<input type="checkbox">&nbsp;');
      touch();
    });
    ui.ed.addEventListener('blur', () => save());
    document.addEventListener('selectionchange', () => { if (visible) syncTools(); });
    window.addEventListener('pagehide', () => { save(); });

    // ---- right: timer + reminders ----
    const card = el('div', 'tn-card'); right.append(card);
    const top = el('div', 'tn-top');
    ui.ring = el('div', 'tn-ring');
    ui.ring.innerHTML = '<svg viewBox="0 0 80 80"><circle class="tr" cx="40" cy="40" r="34"/><circle class="ar" cx="40" cy="40" r="34" transform="rotate(-90 40 40)"/></svg>';
    ui.arc = ui.ring.querySelector('.ar'); ui.arc.style.strokeDasharray = String(CIRC);
    ui.time = el('div', 'tn-time'); ui.timeTx = el('span', 'tn-tx'); ui.time.append(ui.timeTx); ui.ring.append(ui.time);
    const side = el('div', 'tn-side');
    ui.start = el('button', 'tn-start'); ui.start.type = 'button'; ui.start.onclick = startStop;
    ui.custom = el('button', 'tn-ctime'); ui.custom.type = 'button';
    ui.custom.innerHTML = ICON.pencil + '<span></span>'; ui.custom.querySelector('span').textContent = t({ ru: 'Своё время', en: 'Custom time' });
    ui.custom.onclick = customTime;
    side.append(ui.start, ui.custom);
    top.append(ui.ring, side);
    ui.presets = el('div', 'tn-presets');
    [5, 10, 15, 25, 45].forEach((m) => {
      const b = el('button', 'tn-p'); b.append(el('span', 'tn-tx', String(m))); b.type = 'button'; b.dataset.m = m;
      b.onclick = () => setSel(m * 60);
      ui.presets.append(b);
    });
    ui.presets.append(el('span', 'tn-unit', t({ ru: 'мин', en: 'min' })));

    ui.rhead = el('div', 'tn-rhead');
    ui.rhead.innerHTML = ICON.bell + '<span></span>'; ui.rhead.querySelector('span').textContent = t({ ru: 'Напоминания', en: 'Reminders' });
    ui.rlist = el('div', 'tn-rlist');
    ui.rempty = el('div', 'tn-rempty');
    ui.rempty.innerHTML = [t({ ru: 'Напиши ниже, о чём и когда:', en: 'Write below what and when:' }),
      t({ ru: '«позвонить маме завтра в 9»,', en: '"call mum tomorrow at 9",' }),
      t({ ru: '«таблетки каждый день в 23:00»,', en: '"pills every day at 23:00",' }),
      t({ ru: '«через 20 минут выключить плиту».', en: '"turn off the stove in 20 minutes".' })].map(esc).join('<br>');
    ui.rmsg = el('div', 'tn-rmsg');
    ui.rin = el('input', 'tn-rin'); ui.rin.type = 'text'; ui.rin.spellcheck = false;
    ui.rin.placeholder = t({ ru: 'Позвонить маме завтра в 9', en: 'Call mum tomorrow at 9' });
    ui.rin.onkeydown = (e) => { if (e.key === 'Enter') { e.preventDefault(); addReminder(); } };
    card.append(top, ui.presets, el('hr'), ui.rhead, ui.rlist, ui.rempty, ui.rmsg, ui.rin);
    ui.rlist.before(ui.rempty); // the hint sits in the list area; list and hint never show together

    paintTimer(); setSaved('saved');
    loadNotes(); syncTimer(); drawReminders();
    window.Pill.on('remind:due', onDue);
    window.Pill.on('remind:changed', () => { drawReminders(); syncTimer(); });
    window.Pill.on('timer:tick', (p) => { if (p && p.id) applyTimer({ active: true, id: p.id, due_at: Date.now() + p.remaining_ms, total_s: p.total_s }); });
  }

  // Command bar "reminder" row: pill:open-tab {tab:'note', section:'reminders'} scrolls to the reminders block and focuses its input
  let wantRem = false;
  function revealReminders() {
    wantRem = false;
    if (!ui.rlist) return;
    (ui.rlist.closest('.tn-right') || ui.rlist).scrollIntoView({ block: 'nearest', behavior: 'smooth' });
    if (ui.rin) ui.rin.focus({ preventScroll: true });
  }
  window.Pill.register({
    id: 'note', title: { ru: 'Заметка', en: 'Note' }, icon: window.pillIcon ? window.pillIcon('note') : ICON.tab, order: 50, mount,
    onShow() { visible = true; remKnown = false; if (wantRem) setTimeout(revealReminders, 380); syncTimer(); drawReminders(); clearInterval(remI); remI = setInterval(drawReminders, 30000); syncTools(); },
    onHide() { visible = false; clearInterval(remI); remI = 0; save(); },
  });
  window.Pill.on('pill:hide', () => { hid = true; armTick(); }); // nobody sees the chip or the ring: no ticks while the island is hidden
  window.Pill.on('pill:show', () => { hid = false; tick(); }); // tick() repaints at once and re-arms
  // Command bar: `pill:open-tab` {tab:'note', noteId} selects that note (remembered for the first mount too).
  window.Pill.on('pill:open-tab', (p) => {
    if (!p || p.tab !== 'note') return;
    if (p.section === 'reminders') { wantRem = true; if (visible) setTimeout(revealReminders, 100); }
    if (!p.noteId) return;
    store.set('pill.note.cur', p.noteId);
    if (ui.ed) openNote(+p.noteId);
  });
})();
