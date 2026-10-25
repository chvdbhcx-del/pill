// Settings window page (settings.html). Reads/writes settings.json through settings_get / settings_set; every change is saved at once.
// Rust side: settingswin.rs (window, autostart, hotkey_check / hotkey_set), settings.rs, chat.rs (ai_*), net.rs (weather_*), clipboard.rs (clip_*),
// media.rs (media_devices), launcher.rs (launcher_list), agents.rs (agents_hook_snippets). Language: i18n-lite.js (window.t, window.I18n).
(() => {
  'use strict';
  const T = window.__TAURI__ || {};
  const win = T.window && T.window.getCurrentWindow ? T.window.getCurrentWindow() : null;
  async function inv(cmd, args) {
    if (!(T.core && T.core.invoke)) return null;
    try { return await T.core.invoke(cmd, args); } catch (e) { console.error('invoke', cmd, e); return null; }
  }
  const safe = (f, ...a) => { try { return f && f(...a); } catch (e) { console.error(e); } };

  // ---------------------------------------------------------------------------------------------------------- state
  let S = {};              // merged settings (settings_get)
  let syncs = [];          // per-control "read the value again" closures of the page on screen
  let cleanups = [];       // per-page teardown (animation loops, key recorder)
  let cur = null;          // id of the page on screen
  let ver = '';
  const DEF = {
    language: 'auto', theme: 'dark', size: 100, clock24: true, autoHide: false, reduceMotion: false, showInCapture: true, hideInApps: [], shape: 'notch',
    character: 'liquid', pet: 'cat', 'pet.side': 'right', 'pet.size': 'm', hideInFullscreen: true, autoHideKeepDesktop: false, underTaskbar: true, passThrough: false, passThroughOpacity: 50, offsetX: 0,
    musicPill: 'title', opacityCol: 0, opacityExp: 0, showTime: true, showDate: false, showTemp: true, showSeconds: false, showBattery: true, monitor: 'primary',
    'audio.reactive': true, 'sound.reminders': true, 'sound.reminderTone': 'chime', 'sound.volumeDevice': '', 'sound.micDevice': '',
    updateAuto: true, 'net.weather': false, 'net.ai': false, 'weather.units': 'metric', 'weather.city': '',
    'clipboard.clickPaste': true, 'clipboard.maxItems': 1000, 'clipboard.retentionDays': 30, 'clipboard.maxImageMb': 512, 'clipboard.skipSecrets': true,
    'capture.format': 'mp4', 'capture.fps': 30, 'capture.encoder': 'auto', 'capture.shotAction': 'both', 'capture.revealAfterSave': false, fps: 'auto', edgeIdleFps: 30, 'everything.autostart': true,
  };
  const get = (k) => (S[k] !== undefined ? S[k] : DEF[k]);
  const sync = () => syncs.forEach((f) => safe(f));
  async function put(patch) {
    Object.assign(S, patch);
    sync();
    const r = await inv('settings_set', { patch });
    if (r && typeof r === 'object') S = Object.assign({}, r);
    return r;
  }

  // ---------------------------------------------------------------------------------------------------------- dom helpers
  const h = (tag, cls, c) => {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (c != null) (Array.isArray(c) ? c : [c]).forEach((x) => e.append(x instanceof Node ? x : document.createTextNode(String(x))));
    return e;
  };
  const $ = (s) => document.querySelector(s);
  const svg = (inner) => `<svg viewBox="0 0 24 24" aria-hidden="true">${inner}</svg>`;
  const openLink = (url) => inv('chat_open_link', { url });
  const link = (text, url) => { const a = h('a', 'link', text); a.href = '#'; a.onclick = (e) => { e.preventDefault(); openLink(url); }; return a; };

  function card(page, cls) { const c = h('div', 'card' + (cls ? ' ' + cls : '')); page.append(c); return c; }
  function cap(page, txt) { page.append(h('div', 'cap', txt)); }
  function row(c, title, sub, ctl, o = {}) {
    const r = h('div', 'row' + (o.block ? ' block' : '') + (o.cls ? ' ' + o.cls : ''));
    const l = h('div', 'l', [h('b', '', title)]);
    r.sub = null;
    if (sub) { r.sub = h('span', '', sub); l.append(r.sub); }
    r.append(l);
    if (ctl) {
      if (o.block) r.append(ctl);
      else r.append(h('div', 'c', ctl));
    }
    c.append(r);
    return r;
  }
  function msg(el, text, kind) { if (!el) return; el.textContent = text || ''; el.className = kind || ''; }

  function sw(key, opt = {}) {
    const w = h('label', 'sw'), i = h('input'), k = h('i');
    i.type = 'checkbox';
    i.onchange = () => (opt.set ? opt.set(i.checked) : put({ [key]: opt.invert ? !i.checked : i.checked }));
    w.append(i, k);
    syncs.push(() => { i.checked = opt.read ? opt.read() : opt.invert ? !get(key) : !!get(key); });
    i.checked = opt.read ? opt.read() : opt.invert ? !get(key) : !!get(key);
    w.input = i;
    return w;
  }
  function seg(opts, getter, setter) {
    const w = h('div', 'seg');
    const draw = () => [...w.children].forEach((b, i) => b.classList.toggle('on', String(opts[i][0]) === String(getter())));
    opts.forEach(([v, l]) => { const b = h('button', '', l); b.type = 'button'; b.onclick = () => { setter(v); draw(); }; w.append(b); });
    syncs.push(draw); draw();
    return w;
  }
  function select(opts, getter, setter) {
    const s = h('select', 'field');
    opts.forEach(([v, l]) => { const o = h('option', '', l); o.value = v; s.append(o); });
    s.onchange = () => setter(s.value);
    const draw = () => { s.value = String(getter()); };
    syncs.push(draw); draw();
    return s;
  }
  function range(key, min, max, step, fmt) {
    const w = h('div', 'rng'), r = h('input'), out = h('output');
    Object.assign(r, { type: 'range', min, max, step });
    const paint = () => { r.style.setProperty('--p', ((r.value - min) / (max - min)) * 100 + '%'); out.textContent = fmt(+r.value); };
    r.oninput = paint;
    r.onchange = () => put({ [key]: +r.value });
    syncs.push(() => { r.value = get(key); paint(); });
    r.value = get(key); paint();
    w.append(r, out);
    return w;
  }
  function btn(label, fn, cls) { const b = h('button', 'btn' + (cls ? ' ' + cls : ''), label); b.type = 'button'; b.onclick = fn; return b; }
  function area(key, o = {}) {
    const a = h('textarea', 'field' + (o.mono ? ' mono' : ''));
    a.spellcheck = false; a.rows = o.rows || 3;
    if (o.placeholder) a.placeholder = o.placeholder;
    const read = o.read || (() => (get(key) || []).join('\n'));
    const write = o.write || ((txt) => put({ [key]: txt.split(/[\n,;]+/).map((x) => x.trim()).filter(Boolean) }));
    a.onchange = () => write(a.value);
    syncs.push(() => { if (document.activeElement !== a) a.value = read(); });
    a.value = read();
    return a;
  }
  const pct = (v) => v + ' %';

  // ---------------------------------------------------------------------------------------------------------- icons
  const IC = {
    character: '<circle cx="12" cy="12" r="9"/><path d="M8.5 14a4.2 4.2 0 0 0 7 0"/><path d="M9 9.5h.01M15 9.5h.01"/>',
    general: '<path d="M21 6h-9M8 6H3M21 12h-3M14 12H3M21 18h-9M8 18H3"/><circle cx="10" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>',
    hotkeys: '<rect x="2.5" y="5.5" width="19" height="13" rx="2.5"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M7 14.5h10"/>',
    ai: '<rect x="4" y="8" width="16" height="12" rx="3"/><path d="M12 8V4.5M9 13.5h.01M15 13.5h.01M2.5 14v2M21.5 14v2"/><circle cx="12" cy="4" r="1"/>',
    voice: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>',
    weather: '<path d="M12 2v2M4.9 4.9l1.4 1.4M20 12h2M19.1 4.9l-1.4 1.4M15.9 12.6a4 4 0 0 0-5.9-4.1"/><path d="M13 22H7a5 5 0 1 1 4.9-6H13a3 3 0 0 1 0 6z"/>',
    sound: '<path d="M4 9.5v5h3.5L12 18.5v-13L7.5 9.5z"/><path d="M15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11"/>',
    live: '<path d="M13 2.5 5 13.5h6l-1 8 8-11h-6z"/>',
    clipboard: '<rect x="5" y="4.5" width="14" height="17" rx="3"/><path d="M9 4.5V4a1.5 1.5 0 0 1 1.5-1.5h3A1.5 1.5 0 0 1 15 4v.5"/><path d="M9 11h.01M9 15.5h.01M12.5 11H15M12.5 15.5H15"/>',
    modules: '<rect x="3.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="3.5" width="7" height="7" rx="1.8"/><rect x="3.5" y="13.5" width="7" height="7" rx="1.8"/><rect x="13.5" y="13.5" width="7" height="7" rx="1.8"/>',
    updates: '<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>',
    aisetup: '<path d="M12 3.5v11M7.5 10.5l4.5 4.5 4.5-4.5M4.5 15.5v3a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-3"/>',
    power: '<path d="M12 3v8"/><path d="M6.8 6.6a8 8 0 1 0 10.4 0"/>',
  };

  // ---------------------------------------------------------------------------------------------------------- hotkey recorder
  const HK = [
    ['toggle', { ru: 'Открыть / закрыть остров', en: 'Open / close the pill' }, null, 'Ctrl+Alt+I'],
    ['command', { ru: 'Командная строка', en: 'Command bar' }, { ru: 'Программы, калькулятор, конвертер, вопрос ИИ', en: 'Apps, calculator, converter, ask the AI' }, 'Alt+Shift+Space'],
    ['chat', { ru: 'Чат с ИИ', en: 'AI chat' }, { ru: 'Остров открывается на вкладке «Чат», курсор сразу в поле ввода', en: 'The pill opens on the Chat tab with the cursor in the input' }, 'Ctrl+Alt+A'],
    ['voice', { ru: 'Голос в текст', en: 'Voice to text' }, { ru: 'Нажми, скажи, нажми ещё раз: текст окажется в буфере', en: 'Press, speak, press again: the text lands on the clipboard' }, 'Ctrl+Alt+D'],
    ['record', { ru: 'Запись экрана: начать / остановить', en: 'Screen recording: start / stop' }, null, 'Ctrl+Alt+R'],
    ['shotArea', { ru: 'Скриншот области', en: 'Screenshot of an area' }, null, 'Ctrl+Alt+S'],
    ['shotFull', { ru: 'Скриншот всего экрана', en: 'Screenshot of the whole screen' }, null, 'Ctrl+Alt+Shift+S'],
    ['ocr', { ru: 'Текст с экрана (OCR)', en: 'Text from the screen (OCR)' }, null, 'Ctrl+Alt+O'],
    ['clipboard', { ru: 'История буфера', en: 'Clipboard history' }, null, 'Ctrl+Alt+V'],
    ['pastePlain', { ru: 'Вставить без форматирования', en: 'Paste without formatting' }, { ru: 'Текст из Word или с сайта без шрифтов и цветов', en: 'Text from Word or a website without fonts and colours' }, 'Ctrl+Alt+Shift+V'],
    ['mic', { ru: 'Микрофон: выключить / включить', en: 'Microphone: mute / unmute' }, null, 'Ctrl+Alt+M'],
    ['pipette', { ru: 'Пипетка', en: 'Colour picker' }, null, 'Ctrl+Alt+C'],
    ['qr', { ru: 'QR-код с экрана', en: 'QR code from the screen' }, null, 'Ctrl+Alt+Shift+Q'],
    ['passThrough', { ru: 'Сквозной режим: включить / выключить', en: 'Pass-through mode: on / off' }, null, ''],
    ['vpn', { ru: 'VPN: включить / выключить', en: 'VPN: on / off' }, { ru: 'Пока ничего не делает: переключателя VPN в Pill ещё нет, клавиша только запоминается', en: 'Does nothing yet: Pill has no VPN switch, the key is only remembered' }, ''],
  ];
  function codeKey(e) {
    const c = e.code || '';
    if (/^Key[A-Z]$/.test(c)) return c.slice(3);
    if (/^Digit\d$/.test(c)) return c.slice(5);
    if (/^F([1-9]|1\d|2[0-4])$/.test(c)) return c;
    return ({ Space: 'Space', Enter: 'Enter', Tab: 'Tab', ArrowUp: 'Up', ArrowDown: 'Down', ArrowLeft: 'Left', ArrowRight: 'Right', Home: 'Home', End: 'End', PageUp: 'PageUp', PageDown: 'PageDown', Insert: 'Insert', Delete: 'Delete' })[c] || null;
  }
  const capText = (p) => (p === 'Shift' ? '⇧' : p);
  function drawKeys(box, combo) {
    box.replaceChildren();
    box.className = 'keys' + (combo ? '' : ' empty');
    if (!combo) { box.textContent = t({ ru: 'не назначено', en: 'not assigned' }); return; }
    combo.split('+').forEach((p) => box.append(h('span', 'k', capText(p))));
  }
  let rec = null;
  function stopRec() { if (rec) { rec.stop(); rec = null; } }
  function recorder(action, title, getCombo, onMsg) {
    const box = h('div', 'keys');
    box.tabIndex = 0; box.setAttribute('role', 'button');
    box.title = t({ ru: 'Нажмите, затем сочетание клавиш. Esc — отмена, Backspace — сбросить', en: 'Click, then press the shortcut. Esc cancels, Backspace clears' });
    const draw = () => drawKeys(box, getCombo());
    syncs.push(() => { if (rec && rec.box === box) return; draw(); });
    draw();
    const start = () => {
      if (rec && rec.box === box) return;
      stopRec(); onMsg('');
      box.className = 'keys rec'; box.textContent = t({ ru: 'Нажмите сочетание…', en: 'Press the shortcut…' });
      const live = (e) => {
        const mods = [e.ctrlKey && 'Ctrl', e.altKey && 'Alt', e.shiftKey && 'Shift', e.metaKey && 'Win'].filter(Boolean);
        return mods;
      };
      const kd = async (e) => {
        e.preventDefault(); e.stopPropagation();
        if (e.key === 'Escape') { stopRec(); draw(); return; }
        const mods = live(e);
        if ((e.key === 'Backspace' || e.key === 'Delete') && !mods.length) { stopRec(); await commit(''); return; }
        const key = codeKey(e);
        if (!key) { box.replaceChildren(...mods.map((p) => h('span', 'k', capText(p))), document.createTextNode(' …')); box.className = 'keys rec'; return; }
        stopRec();
        await commit(mods.concat(key).join('+'));
      };
      const cancelOut = (e) => { if (!box.contains(e.target)) { stopRec(); draw(); } };
      const bl = () => { stopRec(); draw(); };
      window.addEventListener('keydown', kd, true);
      window.addEventListener('mousedown', cancelOut, true);
      window.addEventListener('blur', bl);
      rec = { box, stop: () => { window.removeEventListener('keydown', kd, true); window.removeEventListener('mousedown', cancelOut, true); window.removeEventListener('blur', bl); } };
    };
    async function commit(combo) {
      const r = await inv('hotkey_set', { action, combo });
      if (r && r.ok) { S['hotkey.' + action] = r.combo; onMsg(''); }
      else if (r) {
        const why = { invalid: t({ ru: 'Нужна клавиша-модификатор: Ctrl, Alt, Shift или Win', en: 'A modifier key is needed: Ctrl, Alt, Shift or Win' }),
          reserved: t({ ru: 'Это сочетание зарезервировано: Ctrl+Alt+Q закрывает Pill', en: 'This shortcut is reserved: Ctrl+Alt+Q quits Pill' }),
          used: t({ ru: 'Уже занято другим действием', en: 'Already used by another action' }),
          taken: t({ ru: 'Занято другой программой', en: 'Taken by another program' }) }[r.reason] || t({ ru: 'Не удалось сохранить', en: 'Could not save' });
        onMsg(why, true);
      }
      draw();
    }
    box.onclick = start;
    box.onkeydown = (e) => { if ((e.key === 'Enter' || e.key === ' ') && !rec) { e.preventDefault(); start(); } };
    return box;
  }
  function hotkeyPage(p) {
    const c = card(p);
    HK.forEach(([id, title, sub, def]) => {
      const key = 'hotkey.' + id;
      const r = row(c, t(title), sub ? t(sub) : '', null);
      const subEl = r.sub || (() => { const s = h('span', ''); r.querySelector('.l').append(s); return s; })();
      const base = sub ? t(sub) : '';
      const note = (m, bad) => { subEl.textContent = m || base; subEl.className = bad ? 'bad' : ''; };
      const getc = () => (S[key] !== undefined ? S[key] : def);
      r.append(h('div', 'c', recorder(id, title, getc, note)));
    });
    // launcher shortcuts
    p.append(h('div', 'footnote', t({ ru: 'Ярлыки лаунчера: каждому можно дать свою клавишу, и он будет открываться ею из любой программы.', en: 'Launcher shortcuts: give each item its own key and it opens from any program.' })));
    const lc = card(p); lc.style.marginTop = '8px';
    inv('launcher_list').then((list) => {
      if (!Array.isArray(list) || !list.length) { row(lc, t({ ru: 'Лаунчер пуст', en: 'The launcher is empty' }), t({ ru: 'Добавь программы на вкладке «Главная»: перетащи их на остров', en: 'Add programs on the Home tab: drop them on the pill' }), null); return; }
      list.forEach((l) => {
        const key = 'hotkey.launcher.' + l.id;
        const r = row(lc, l.title, '', null, { cls: 'app' });
        const ico = l.icon ? Object.assign(h('img', 'app-ico'), { src: l.icon, alt: '' }) : h('div', 'app-ico');
        r.prepend(ico);
        r.append(h('div', 'c', recorder('launcher.' + l.id, l.title, () => (S[key] !== undefined ? S[key] : ''), (m, bad) => { const sp = r.querySelector('.l b'); sp.title = m || ''; if (m) sp.style.color = bad ? 'var(--red)' : ''; else sp.style.color = ''; })));
      });
      syncs.forEach((f) => safe(f));
    });
  }

  // ---------------------------------------------------------------------------------------------------------- Character previews (own art)
  const REACT = [
    ['calm', { ru: 'Покой', en: 'Calm' }], ['music', { ru: 'Музыка', en: 'Music' }], ['ai', { ru: 'ИИ думает', en: 'AI thinking' }], ['done', { ru: 'Готово', en: 'Done' }],
    ['attention', { ru: 'Внимание', en: 'Attention' }], ['file', { ru: 'Файл', en: 'File' }], ['sleep', { ru: 'Сон', en: 'Sleep' }],
  ];
  const STYLES = [
    ['liquid', { ru: 'Жидкий край', en: 'Liquid edge' }, { ru: 'Нижний край острова тянется к курсору и красится в цвет события.', en: 'The island edge leans to the cursor and takes the colour of the event.' }],
    ['tail', { ru: 'Хвост', en: 'Tail' }, { ru: 'В острове кто-то живёт, но видно только хвост.', en: 'Someone lives in the island, but only the tail shows.' }],
    ['legs', { ru: 'Ножки', en: 'Legs' }, { ru: 'У острова есть глаза и ножки: он переминается под музыку и закрывает глаза во сне.', en: 'The island has eyes and little legs: it steps to the music and shuts its eyes to sleep.' }],
    ['glow', { ru: 'Свечение', en: 'Glow' }, { ru: 'Под островом горит мягкий свет цвета события.', en: 'A soft light in the colour of the event glows under the island.' }],
    ['none', { ru: 'Без украшений', en: 'Plain' }, { ru: 'Просто чёрный остров, как раньше.', en: 'Just a black island, like before.' }],
  ];
  const LINE = { calm: '#3a393e', music: '#d8836a', ai: '#9e8fe8', done: '#8fbf4a', attention: '#e8a040', file: '#5a78a8', sleep: '#2c2b2f' };
  const GLOW = { calm: '#ffffff', music: '#d8836a', ai: '#9e8fe8', done: '#8fbf4a', attention: '#e8a040', file: '#5a78a8', sleep: '#888' };
  const tp = (u) => Math.pow(Math.sin(Math.PI * u), 0.6);
  const beat = (T) => { const b = 0.5 + 0.5 * Math.cos((2 * Math.PI * T) / 1.1); return b * b; };
  // wave offset (px) of the liquid edge at u in 0..1; T = seconds, t = seconds since the state started
  const WAVE = {
    calm: (u, T) => 1.6 * Math.sin(u * 8.2 - T * 0.8) + 0.9 * Math.sin(u * 3.8 + T * 0.5),
    music: (u, T) => (1.5 + 5 * beat(T)) * tp(u) * Math.sin(u * 75.4 - T * 9),
    ai: (u, T) => 3 * tp(u) * Math.sin(u * 25.1 - T * 2.2),
    done: (u, T, t) => (8 * Math.exp(-t / 1.1) + 1.2) * tp(u) * (Math.sin(u * 18.8 - T * 3) + 0.5 * Math.sin(u * 31.4 + T * 2)),
    attention: (u, T) => 3 * tp(u) * Math.sin(u * 138 - T * 14) * (0.8 + 0.2 * Math.sin(T * 9)),
    file: (u, T, t) => Math.exp(-t / 0.7) * 5 * tp(u) * Math.sin(u * 22 - T * 4) + 3.2 * Math.sin(Math.PI * u),
    sleep: (u) => 5 * Math.sin(Math.PI * u),
  };
  function previews(host) {
    const NS = 'http://www.w3.org/2000/svg';
    const mk = (tag, attrs, parent) => { const e = document.createElementNS(NS, tag); Object.entries(attrs || {}).forEach(([k, v]) => e.setAttribute(k, v)); if (parent) parent.append(e); return e; };
    let state = 'calm', t0 = performance.now(), alive = true, raf = 0;
    const X0 = 16, X1 = 133, R = 13, BASE = 36;
    const cards = {}, chars = []; // chars = the tail / legs cards, drawn by characters.js
    host.querySelectorAll('svg[data-style]').forEach((s) => {
      const k = s.dataset.style;
      s.setAttribute('viewBox', '0 0 149 92'); s.setAttribute('preserveAspectRatio', 'xMidYMid slice');
      if (k === 'liquid') {
        cards.liquid = { blk: mk('path', { fill: '#000' }, s), ln: mk('path', { fill: 'none', 'stroke-width': '1.5', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, s), ball: mk('circle', { r: '3.4', fill: '#000', opacity: '0' }, s) };
      } else if (k === 'tail' || k === 'legs') {
        if (window.PillChars) chars.push(window.PillChars.preview(s, k)); // own art + own animation loop (characters.js)
      } else if (k === 'glow') {
        cards.glow = { gl: mk('ellipse', { cx: '74.5', cy: String(BASE + 1), rx: '48', ry: '9', style: 'filter:blur(7px)' }, s) };
        mk('path', { d: `M${X0} 0H${X1}V${BASE - R}a${R} ${R} 0 0 1-${R} ${R}H${X0 + R}a${R} ${R} 0 0 1-${R}-${R}Z`, fill: '#000' }, s);
      } else {
        mk('path', { d: `M${X0} 0H${X1}V${BASE - R}a${R} ${R} 0 0 1-${R} ${R}H${X0 + R}a${R} ${R} 0 0 1-${R}-${R}Z`, fill: '#000' }, s);
      }
    });
    function frame(now) {
      if (!alive) return;
      const T = now / 1000, tt = (now - t0) / 1000;
      const c = cards;
      if (c.liquid) {
        const w = WAVE[state];
        let d = '', top = `M${X0} 0H${X1}`, pts = [];
        for (let x = X1; x >= X0; x -= 1.5) {
          const u = (x - X0) / (X1 - X0), dd = Math.min(x - X0, X1 - x);
          const corner = dd < R ? R - Math.sqrt(R * R - (R - dd) * (R - dd)) : 0;
          pts.push([x, BASE - corner + w(u, T, tt)]);
        }
        const line = pts.map((p, i) => (i ? 'L' : 'M') + p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join('');
        c.liquid.blk.setAttribute('d', top + 'L' + pts.map((p) => p[0].toFixed(1) + ' ' + p[1].toFixed(1)).join('L') + 'Z');
        c.liquid.ln.setAttribute('d', line);
        c.liquid.ln.setAttribute('stroke', LINE[state]);
        c.liquid.ln.setAttribute('stroke-opacity', state === 'sleep' ? '0.7' : '1');
        let ballOp = 0, bx = 0, by = 0;
        if (state === 'ai') { ballOp = 1; bx = 30 + ((T * 24) % 90); by = BASE + 15 + 5 * Math.sin(T * 3); }
        if (state === 'done') { ballOp = Math.exp(-tt / 0.8); bx = 100; by = BASE + 14 + 3 * Math.sin(T * 3); }
        c.liquid.ball.setAttribute('opacity', ballOp.toFixed(2)); c.liquid.ball.setAttribute('cx', bx.toFixed(1)); c.liquid.ball.setAttribute('cy', by.toFixed(1));
      }
      if (c.glow) {
        let op = 0;
        if (state === 'calm') op = 0.06;
        else if (state === 'music') op = 0.25 + 0.45 * beat(T);
        else if (state === 'ai') op = 0.22 + 0.18 * Math.sin(T * 1.6);
        else if (state === 'done') op = 0.6 * Math.exp(-tt / 1.2);
        else if (state === 'attention') op = 0.25 + 0.35 * (Math.sin(T * 10) > 0 ? 1 : 0.4);
        else if (state === 'file') op = 0.35;
        else op = 0.07 + 0.04 * Math.sin(T * 1.2);
        c.glow.gl.setAttribute('fill', GLOW[state]); c.glow.gl.setAttribute('opacity', Math.max(0, op).toFixed(2));
      }
      if (!document.hidden && !(window.matchMedia && matchMedia('(prefers-reduced-motion: reduce)').matches)) raf = requestAnimationFrame(frame);
      else raf = 0;
    }
    const kick = () => { if (!raf) raf = requestAnimationFrame(frame); };
    const vis = () => { if (!document.hidden) kick(); };
    document.addEventListener('visibilitychange', vis);
    kick();
    return {
      setState(s) { state = s; t0 = performance.now(); cancelAnimationFrame(raf); raf = 0; frame(performance.now()); kick(); chars.forEach((c) => c.setState(s)); },
      destroy() { alive = false; cancelAnimationFrame(raf); document.removeEventListener('visibilitychange', vis); chars.forEach((c) => c.destroy()); },
    };
  }

  // ---------------------------------------------------------------------------------------------------------- pages
  const PAGES = [
    { id: 'character', icon: 'character', name: { ru: 'Персонаж', en: 'Character' }, build: pCharacter },
    { sep: true },
    { id: 'general', icon: 'general', name: { ru: 'Общее', en: 'General' }, build: pGeneral },
    { id: 'hotkeys', icon: 'hotkeys', name: { ru: 'Клавиши', en: 'Hotkeys' }, head: { ru: 'Горячие клавиши', en: 'Hotkeys' }, build: pHotkeys },
    { id: 'ai', icon: 'ai', name: { ru: 'ИИ', en: 'AI' }, build: pAi },
    { id: 'voice', icon: 'voice', name: { ru: 'Голос в текст', en: 'Voice to text' }, build: pVoice },
    { id: 'aisetup', icon: 'aisetup', name: { ru: 'Установка ИИ', en: 'AI setup' }, build: pAiSetup },
    { id: 'weather', icon: 'weather', name: { ru: 'Погода', en: 'Weather' }, build: pWeather },
    { id: 'sound', icon: 'sound', name: { ru: 'Звук', en: 'Sound' }, build: pSound },
    { id: 'live', icon: 'live', name: { ru: 'Живые события', en: 'Live events' }, build: pLive },
    { id: 'clipboard', icon: 'clipboard', name: { ru: 'Буфер обмена', en: 'Clipboard' }, build: pClipboard },
    { id: 'modules', icon: 'modules', name: { ru: 'Модули', en: 'Modules' }, build: pModules },
    { sep: true },
    { id: 'updates', icon: 'updates', name: { ru: 'Обновления', en: 'Updates' }, build: pUpdates },
  ];

  // ---- Character
  function pCharacter(p) {
    let state = 'calm';
    const petInst = []; // live pet previews of the Pet section below (pets.js); they follow the reaction chips too
    const c = card(p);
    const box = h('div', 'react');
    box.append(h('span', 'lab', t({ ru: 'Посмотреть реакцию:', en: 'Preview a reaction:' })));
    const chips = REACT.map(([id, name]) => { const b = h('button', 'rchip' + (id === state ? ' on' : ''), t(name)); b.type = 'button'; b.onclick = () => { state = id; chips.forEach((x, i) => x.classList.toggle('on', REACT[i][0] === id)); pv.setState(id); petInst.forEach((i) => i.setState(id === 'ai' ? 'thinking' : id)); }; box.append(b); return b; });
    c.append(box);
    cap(p, t({ ru: 'Остров', en: 'Island' }));
    const grid = h('div', 'pgrid');
    const cards = STYLES.map(([id, name, desc]) => {
      const b = h('button', 'pcard'); b.type = 'button'; b.title = t(desc);
      b.innerHTML = `<svg data-style="${id}"></svg>`;
      b.append(h('b', '', t(name)), h('span', '', t(desc)));
      b.onclick = () => put({ character: id });
      grid.append(b); return b;
    });
    p.append(grid);
    const pv = previews(grid);
    cleanups.push(() => pv.destroy());
    syncs.push(() => cards.forEach((b, i) => b.classList.toggle('on', STYLES[i][0] === get('character'))));

    // ---- Pet: a small character living inside the island (pets.js draws it; keys pet, pet.side, pet.size). Cards come from the registry, one grid per group. ----
    const PP = window.PillPets;
    if (PP) {
      const gr = PP.groups.map((g) => ({ g, ids: PP.ids.filter((id) => PP.registry[id].group === g.id) })).filter((x) => x.ids.length);
      const pids = ['none'], pcards = [];
      gr.forEach(({ g, ids }, gi) => {
        cap(p, gi === 0 ? t({ ru: 'Питомец', en: 'Pet' }) + (gr.length > 1 ? ' · ' + t(g.name) : '') : t(g.name));
        const pg = h('div', 'pgrid');
        (gi === 0 ? ['none', ...ids] : ids).forEach((id) => { if (id !== 'none') pids.push(id); pcards.push(petCard(pg, id)); });
        p.append(pg);
      });
      function petCard(pg, id) {
        const b = h('button', 'pcard'); b.type = 'button';
        const stage = h('div', 'pet-stage' + (id === 'none' ? ' none' : ''));
        if (id !== 'none') { const inst = PP.create(id, { state: state === 'ai' ? 'thinking' : state, size: get('pet.size') }); stage.append(inst.el); petInst.push(inst); }
        const nm = id === 'none' ? t({ ru: 'Без питомца', en: 'None' }) : t(PP.registry[id].name);
        const ds = id === 'none' ? t({ ru: 'Остров без жителей.', en: 'An island with nobody on it.' }) : t(PP.registry[id].desc);
        b.title = ds;
        b.append(stage, h('b', '', nm), h('span', '', ds));
        b.onclick = () => put({ pet: id });
        pg.append(b); return b;
      }
      cleanups.push(() => petInst.forEach((i) => i.destroy()));
      syncs.push(() => { pcards.forEach((b, i) => b.classList.toggle('on', pids[i] === get('pet'))); petInst.forEach((i) => { i.el.dataset.size = get('pet.size'); }); });
      const pc = card(p);
      row(pc, t({ ru: 'Сторона', en: 'Side' }), t({ ru: 'С какого края острова стоит питомец', en: 'Which end of the island the pet stands at' }),
        segKey('pet.side', [['left', t({ ru: 'Слева', en: 'Left' })], ['right', t({ ru: 'Справа', en: 'Right' })]]));
      row(pc, t({ ru: 'Размер', en: 'Size' }), t({ ru: 'Питомец следует за реакциями острова; щелчок по нему его гладит', en: 'The pet follows the island reactions; click it to pet it' }),
        segKey('pet.size', [['s', 'S'], ['m', 'M'], ['l', 'L']]));
    }
    sync();
    p.append(h('div', 'footnote', t({ ru: 'Стиль края применяется к острову сразу. Реакции включаются сами: агенты, музыка, файлы, напоминания; через 5 минут тишины остров засыпает.', en: 'The edge style applies to the island at once. Reactions fire on their own: agents, music, files, reminders; after 5 quiet minutes the island falls asleep.' })));
  }

  // ---- General
  function pGeneral(p) {
    let c = card(p);
    row(c, t({ ru: 'Язык', en: 'Language' }), t({ ru: 'По умолчанию — как в Windows', en: 'By default, the same as Windows' }),
      segKey('language', [['auto', t({ ru: 'Как в Windows', en: 'Windows' })], ['ru', 'Русский'], ['en', 'English']]));
    // autostart: the registry is the truth, never touched until the user flips the switch
    let ar = null, auto = false;
    const asw = sw('', { read: () => auto, set: async (on) => {
      try { auto = await T.core.invoke('autostart_set', { on }); msg(ar.sub, ''); } catch (e) { auto = !on; msg(ar.sub, String(e), 'bad'); }
      asw.input.checked = auto;
    } });
    ar = row(c, t({ ru: 'Запускать вместе с Windows', en: 'Start with Windows' }), '', asw);
    if (!ar.sub) { ar.sub = h('span', ''); ar.querySelector('.l').append(ar.sub); }
    inv('autostart_get').then((v) => { auto = !!v; asw.input.checked = auto; });
    row(c, t({ ru: 'Прятать в полноэкранных приложениях', en: 'Hide in full-screen apps' }), t({ ru: 'Игры, видео на весь экран, презентации', en: 'Games, full-screen video, presentations' }), sw('hideInFullscreen'));
    row(c, t({ ru: 'Показывать остров на скриншотах и в записи экрана', en: 'Show the pill in screenshots and screen recordings' }),
      t({ ru: 'Обычно его там нет: ни скриншоты, ни запись, ни трансляция остров не видят. Включи, если фоновая запись NVIDIA или другой программы пишет, что остров мешает записи рабочего стола', en: 'Normally it is not there: screenshots, recordings and streams do not see the pill. Turn it on if the background recording of NVIDIA or another program says the pill gets in the way of recording the desktop' }), sw('showInCapture'));
    row(c, t({ ru: 'Прятать остров, пока он не нужен', en: 'Hide the pill until it is needed' }),
      t({ ru: 'Свёрнутый остров уходит за край экрана и не мешает нажимать то, что под ним: вкладки браузера, кнопки окна. Задержи курсор у самого верхнего края экрана над ним, и он вернётся; с карточкой, записью или диктовкой он выходит сам', en: 'The folded pill slips behind the screen edge and does not block what is under it: browser tabs, window buttons. Rest the cursor at the very top edge of the screen above it and it comes back; with a card, a recording or dictation it comes out by itself' }), sw('autoHide'));
    row(c, t({ ru: 'Не прятать на рабочем столе', en: 'Do not hide on the desktop' }),
      t({ ru: 'Для автоскрытия: пока впереди сам рабочий стол, без окон программ, остров остаётся на виду', en: 'For auto-hide: while the desktop itself is in front, with no program windows, the pill stays in view' }), sw('autoHideKeepDesktop'));
    row(c, t({ ru: 'Прятать остров в этих программах', en: 'Hide the pill in these programs' }),
      t({ ru: 'По одной на строку, имя процесса: photoshop.exe, premiere.exe. Пока окно такой программы впереди, остров полностью скрыт, и возвращается, когда впереди другое окно.', en: 'One per line, the process name: photoshop.exe, premiere.exe. While a window of such a program is in front, the pill is hidden completely and comes back when another window is in front.' }),
      area('hideInApps', { rows: 3 }), { block: true });

    cap(p, t({ ru: 'Остров', en: 'Island' }));
    c = card(p);
    row(c, t({ ru: 'Форма острова', en: 'Island shape' }), '', segKey('shape', [['notch', t({ ru: 'Вырез', en: 'Notch' })], ['capsule', t({ ru: 'Капсула', en: 'Capsule' })]]));
    row(c, t({ ru: 'Тема острова', en: 'Island theme' }), t({ ru: 'Цвета самого острова. Тему Windows переключает центр управления', en: 'The colours of the pill itself. The Windows theme is switched by the control centre' }),
      segKey('theme', [['dark', t({ ru: 'Тёмная', en: 'Dark' })], ['light', t({ ru: 'Светлая', en: 'Light' })], ['auto', t({ ru: 'Как в Windows', en: 'Windows' })]]));
    row(c, t({ ru: 'Размер острова', en: 'Island size' }), t({ ru: 'На большом мониторе остров можно сделать крупнее', en: 'On a large monitor the pill can be made bigger' }), range('size', 80, 200, 5, pct));
    row(c, t({ ru: 'Прозрачность свёрнутого острова', en: 'Transparency of the folded island' }), t({ ru: 'Пилюля и её карточки', en: 'The pill and its cards' }), range('opacityCol', 0, 60, 5, pct));
    row(c, t({ ru: 'Прозрачность раскрытого острова', en: 'Transparency of the open island' }), '', range('opacityExp', 0, 60, 5, pct));
    row(c, t({ ru: 'Положение вдоль верхнего края', en: 'Position along the top edge' }),
      t({ ru: '0 — посередине; минус — левее, плюс — правее. До края монитора доходит окно раскрытого острова, свёрнутый стоит над его серединой', en: '0 is the middle; minus is further left, plus further right. The window of the open pill reaches the monitor edge, the folded one stays above its middle' }), range('offsetX', -50, 50, 1, pct));
    row(c, t({ ru: 'Под панелью задач, если она сверху', en: 'Below the taskbar if it is at the top' }), t({ ru: 'Остров встаёт под панель, а не поверх её кнопок', en: 'The pill sits below the bar, not over its buttons' }), sw('underTaskbar'));
    const mon = h('div');
    row(c, t({ ru: 'Монитор', en: 'Monitor' }), '', mon);
    inv('pill_monitors').then((list) => { list = list || []; mon.replaceChildren(segKey('monitor', [['primary', t({ ru: 'Основной', en: 'Primary' })], ...list.map((_, i) => [String(i + 1), String(i + 1)])])); mon.title = list.join('\n'); });
    row(c, t({ ru: 'Сквозной режим', en: 'Pass-through mode' }),
      t({ ru: 'Свёрнутый остров пропускает клики и не раскрывается под курсором: он на виду и ничему не мешает. Открывается клавишами. Включить и выключить можно здесь, своей клавишей или командой «сквозной» в командной строке', en: 'The folded pill lets clicks through and does not open under the cursor: it stays in view and gets in nobody’s way. It opens with keys. Switch it here, with its own key or with the “pass-through” command in the command bar' }), sw('passThrough'));
    row(c, t({ ru: 'Прозрачность в сквозном режиме', en: 'Transparency in pass-through mode' }), '', range('passThroughOpacity', 0, 80, 5, pct));
    row(c, t({ ru: 'Жидкий край по частотам', en: 'Liquid edge by frequency' }),
      t({ ru: 'При музыке край показывает звук как эквалайзер: слева низкие частоты, справа высокие. Работает только с персонажем «Жидкий край»', en: 'With music the edge shows the sound like an equaliser: low frequencies on the left, high on the right. Works only with the “Liquid edge” character' }), sw('audio.reactive'));
    row(c, t({ ru: 'Пилюля во время музыки', en: 'Pill during music' }),
      t({ ru: '«С названием» — трек и полоса времени. «Мини-плеер» — обложка, полоса и кнопки; открывается щелчком, а не наведением. «С текстом» — строка песни, которая звучит', en: '“Title” shows the track and a time bar. “Mini player” shows the cover, the bar and buttons; it opens with a click, not by hovering. “Lyrics” shows the line being sung' }),
      segKey('musicPill', [['title', t({ ru: 'С названием', en: 'Title' })], ['mini', t({ ru: 'Мини-плеер', en: 'Mini player' })], ['lyrics', t({ ru: 'С текстом', en: 'Lyrics' })]]), { block: true });

    cap(p, t({ ru: 'Свёрнутый остров', en: 'Folded island' }));
    c = card(p);
    row(c, t({ ru: 'Время', en: 'Time' }), t({ ru: 'Пока на острове ничего не происходит', en: 'While nothing else is happening on the pill' }), sw('showTime'));
    row(c, t({ ru: 'Дата', en: 'Date' }), t({ ru: 'Рядом со временем', en: 'Next to the time' }), sw('showDate'));
    row(c, t({ ru: 'Температура', en: 'Temperature' }), t({ ru: 'Нужна включённая погода', en: 'Needs the weather switched on' }), sw('showTemp'));
    row(c, t({ ru: 'Секунды', en: 'Seconds' }), '', sw('showSeconds'));
    row(c, t({ ru: 'Формат часов', en: 'Clock format' }), '', seg([[false, '12 h'], [true, '24 h']], () => !!get('clock24'), (v) => put({ clock24: v })));
    row(c, t({ ru: 'Заряд батареи', en: 'Battery charge' }), t({ ru: 'На компьютере с батареей', en: 'On a computer that has a battery' }), sw('showBattery'));
    row(c, t({ ru: 'Уменьшить анимацию', en: 'Reduce motion' }), '', sw('reduceMotion'));

    // fps / edgeIdleFps are read by pill.js; an unset key means auto / 30
    cap(p, t({ ru: 'Плавность', en: 'Smoothness' }));
    c = card(p);
    const fpsSeg = (key, vals) => seg(vals.map((v) => [v, v === 'auto' ? t({ ru: 'Авто', en: 'Auto' }) : String(v)]), () => get(key), (v) => put({ [key]: v === 'auto' ? v : +v }));
    row(c, t({ ru: 'Частота кадров', en: 'Frame rate' }), t({ ru: 'Авто = частота монитора. Больше кадров — плавнее, но выше нагрузка на видеокарту', en: 'Auto = the monitor refresh rate. More frames are smoother but cost more GPU' }), fpsSeg('fps', ['auto', 30, 60, 120, 144, 240]));
    row(c, t({ ru: 'Кадры в покое', en: 'Idle edge frame rate' }), t({ ru: 'Для спокойного края острова, когда ничего не происходит', en: 'For the calm island edge while nothing is happening' }), fpsSeg('edgeIdleFps', [15, 30, 60]));

    // file search + settings backup (settings_backup.rs: no keys/passwords in the export; import keeps only known settings)
    cap(p, t({ ru: 'Поиск файлов и копия настроек', en: 'File search and settings backup' }));
    c = card(p);
    row(c, t({ ru: 'Запускать Everything сам', en: 'Start Everything by itself' }),
      t({ ru: 'Если поиск файлов не отвечает, при первом поиске за сеанс запускается Everything (скрыто, в трее)', en: 'If file search does not answer, the first search of a session starts Everything (hidden in the tray)' }), sw('everything.autostart'));
    const bk = row(c, t({ ru: 'Резервная копия настроек', en: 'Settings backup' }),
      t({ ru: 'Экспорт кладёт файл в «Загрузки». Ключи и пароли в него не попадают', en: 'Export puts a file in Downloads. Keys and passwords are not included' }), null);
    const note = h('span', '');
    const pick = h('input');
    pick.type = 'file'; pick.accept = '.json,application/json'; pick.hidden = true;
    pick.onchange = () => {
      const f = pick.files && pick.files[0];
      pick.value = '';
      if (!f) return;
      if (f.size > 1048576) { msg(note, t({ ru: 'Файл больше 1 МБ', en: 'The file is larger than 1 MB' }), 'bad'); return; }
      const rd = new FileReader();
      rd.onload = async () => {
        const r = await inv('settings_import', { jsonText: String(rd.result || '') });
        if (!r) { msg(note, t({ ru: 'Не удалось импортировать', en: 'Import failed' }), 'bad'); return; }
        if (r.ok) {
          const s = settingsCounts(r.applied, r.skipped);
          msg(note, r.error ? s + ' · ' + r.error : s, r.error ? 'bad' : 'ok');
          const all = await inv('settings_get');
          if (all) { S = Object.assign({}, all); sync(); }
        } else msg(note, (r.error || '') + (r.skipped ? ' · ' + settingsCounts(0, r.skipped) : ''), 'bad');
      };
      rd.onerror = () => msg(note, t({ ru: 'Не удалось прочитать файл', en: 'Could not read the file' }), 'bad');
      rd.readAsText(f);
    };
    const settingsCounts = (a, s) => t({ ru: `Применено ${a}, пропущено ${s}`, en: `Applied ${a}, skipped ${s}` });
    const exp = btn(t({ ru: 'Экспорт', en: 'Export' }), async () => {
      const r = await inv('settings_export');
      msg(note, r && r.ok ? t({ ru: 'Сохранено: ', en: 'Saved: ' }) + r.path : (r && r.error) || t({ ru: 'Не удалось сохранить', en: 'Could not save' }), r && r.ok ? 'ok' : 'bad');
    });
    const imp = btn(t({ ru: 'Импорт', en: 'Import' }), () => pick.click());
    bk.querySelector('.l').append(note);
    bk.append(h('div', 'c', [exp, imp, pick]));
  }

  // ---- Hotkeys
  function pHotkeys(p) { hotkeyPage(p); }

  // ---- AI
  const PROV = [
    { id: 'ollama', name: 'Ollama', tag: { ru: 'локально', en: 'local' }, url: 'http://127.0.0.1:11434/v1', preset: 'ollama', help: { ru: 'Модели на этом компьютере. Ключ не нужен.', en: 'Models on this computer. No key needed.' } },
    { id: 'freellm', name: 'FreeLLMAPI', tag: { ru: 'локально', en: 'local' }, url: 'http://127.0.0.1:3001/v1', preset: 'freellm', help: { ru: 'Свой шлюз на этом компьютере. Ключ не нужен.', en: 'Your own gateway on this computer. No key needed.' } },
    { id: 'groq', name: 'Groq', free: 1, region: 1, url: 'https://api.groq.com/openai/v1', keyUrl: 'https://console.groq.com/keys', help: { ru: 'Бесплатно, без карты: около 1000 запросов в день. Тот же ключ подходит для диктовки.', en: 'Free, no card: about 1000 requests a day. The same key works for dictation.' } },
    { id: 'gemini', name: 'Google Gemini', free: 1, region: 1, url: 'https://generativelanguage.googleapis.com/v1beta/openai', keyUrl: 'https://aistudio.google.com/apikey', help: { ru: 'Бесплатный тариф Google AI Studio: картинки, длинные документы. Тот же ключ подходит для диктовки.', en: 'The free Google AI Studio tier: pictures, long documents. The same key works for dictation.' } },
    { id: 'openrouter', name: 'OpenRouter', free: 1, region: 1, url: 'https://openrouter.ai/api/v1', keyUrl: 'https://openrouter.ai/keys', help: { ru: 'Много моделей за одним ключом, у части есть бесплатный лимит.', en: 'Many models behind one key, some with a free allowance.' } },
    { id: 'deepseek', name: 'DeepSeek', url: 'https://api.deepseek.com/v1', keyUrl: 'https://platform.deepseek.com/api_keys', help: { ru: 'Платный сервис с низкими ценами.', en: 'A paid service with low prices.' } },
    { id: 'custom', name: { ru: 'Другой', en: 'Custom' }, url: '', help: { ru: 'Любой OpenAI-совместимый адрес: LM Studio, vLLM, Polza.ai, OmniRoute, APINET.', en: 'Any OpenAI-compatible address: LM Studio, vLLM, Polza.ai, OmniRoute, APINET.' } },
  ];
  const aiState = { cfg: null, models: [] };
  async function pAi(p) {
    const cfg = (await inv('ai_config')) || {};
    aiState.cfg = cfg;
    const provId = () => { const s = get('ai.provider'); if (s && PROV.some((x) => x.id === s)) return s; const m = PROV.find((x) => x.url && x.url === (cfg.baseUrl || '')); return m ? m.id : 'custom'; };
    let curProv = provId();
    const c = card(p);
    row(c, t({ ru: 'ИИ-помощник', en: 'AI assistant' }), t({ ru: 'Выключен по умолчанию. Запросы уходят только на адрес ниже и только когда ты пишешь в чате или в командной строке.', en: 'Off by default. Requests go only to the address below, and only when you write in the chat or the command bar.' }), sw('net.ai'));
    const top = h('div', 'pad');
    const chips = h('div', 'chips');
    const help = h('div', 'note'); help.style.marginTop = '12px';
    const warn = h('div', 'note warn'); warn.style.marginTop = '8px';
    const draw = () => {
      chips.replaceChildren();
      PROV.forEach((x) => {
        const b = h('button', 'chip' + (x.id === curProv ? ' on' : ''), [typeof x.name === 'string' ? x.name : t(x.name)]); b.type = 'button';
        if (x.free) b.append(h('span', 'badge free', t({ ru: 'бесплатно', en: 'free' })));
        if (x.tag) b.append(h('span', 'badge free', t(x.tag)));
        b.onclick = async () => {
          curProv = x.id;
          const patch = { 'ai.provider': x.id, 'ai.preset': x.preset || 'custom' };
          if (x.url) patch['ai.baseUrl'] = x.url;
          await put(patch);
          aiState.cfg = (await inv('ai_config')) || cfg;
          urlIn.value = get('ai.baseUrl') || ''; draw();
        };
        chips.append(b);
      });
      const x = PROV.find((q) => q.id === curProv);
      help.replaceChildren(t(x.help));
      if (x.keyUrl) { help.append(' '); help.append(link(t({ ru: 'Получить ключ', en: 'Get a key' }), x.keyUrl)); }
      warn.textContent = x.region ? t({ ru: 'Groq, Gemini и OpenRouter доступны не во всех странах: если сервис не отвечает, включи VPN или выбери другой из списка.', en: 'Groq, Gemini and OpenRouter are not available in every country: if the service does not answer, turn on a VPN or pick another one from the list.' }) : '';
      warn.hidden = !x.region;
    };
    top.append(chips, help, warn);
    c.append(top);

    // key
    const kr = h('div', 'row block');
    kr.append(h('div', 'l', [h('b', '', t({ ru: 'Ключ API', en: 'API key' })), h('span', '', t({ ru: 'Ключ выбранного сервиса. Хранится только на этом компьютере, зашифрован Windows, не вшит в приложение и никому не передаётся.', en: 'The key of the chosen service. Stored only on this computer, encrypted by Windows; it is not built into the app and not shared with anyone.' }))]));
    const key = h('input', 'field'); key.type = 'password'; key.autocomplete = 'off'; key.spellcheck = false;
    const keyMsg = h('div', 'note'); keyMsg.style.marginTop = '8px';
    const setPh = () => { key.placeholder = aiState.cfg.hasKey ? t({ ru: '•••••••• (ключ сохранён)', en: '•••••••• (key saved)' }) : t({ ru: 'не нужен для локальных серверов', en: 'not needed for local servers' }); };
    const rm = btn(t({ ru: 'Удалить', en: 'Remove' }), async () => { await inv('ai_set_key', { key: '' }); aiState.cfg = (await inv('ai_config')) || cfg; setPh(); rm.hidden = true; msg(keyMsg, ''); }, 'danger');
    const save = btn(t({ ru: 'Сохранить', en: 'Save' }), async () => {
      const v = key.value.trim(); if (!v) return;
      const r = await inv('ai_set_key', { key: v }); key.value = '';
      aiState.cfg = (await inv('ai_config')) || cfg; setPh(); rm.hidden = !aiState.cfg.hasKey;
      msg(keyMsg, r && r.ok ? t({ ru: 'Ключ сохранён: зашифрован Windows, хранится только здесь.', en: 'Key saved: encrypted by Windows, stored only on this computer.' }) : (r && r.error) || 'error', r && r.ok ? 'note ok' : 'note bad');
    });
    setPh(); rm.hidden = !cfg.hasKey;
    const ki = h('div', 'inline'); ki.append(key, save, rm);
    kr.append(ki, keyMsg); c.append(kr);

    // address + model
    const ar = h('div', 'row block');
    ar.append(h('div', 'l', [h('b', '', t({ ru: 'Адрес API и модель', en: 'API address and model' }))]));
    const urlIn = h('input', 'field'); urlIn.value = cfg.baseUrl || ''; urlIn.placeholder = 'http://127.0.0.1:11434/v1'; urlIn.spellcheck = false;
    const mdl = h('input', 'field'); mdl.value = cfg.model || ''; mdl.placeholder = t({ ru: 'имя модели', en: 'model name' }); mdl.spellcheck = false; mdl.setAttribute('list', 'st-models'); mdl.style.fontFamily = 'var(--mono)'; mdl.style.fontSize = '12px';
    const dl = h('datalist'); dl.id = 'st-models';
    const listB = btn(t({ ru: 'Список', en: 'List' }), async () => {
      await put({ 'ai.baseUrl': urlIn.value.trim() });
      listB.disabled = true; const r = await inv('ai_models'); listB.disabled = false;
      if (r && r.ok) { dl.replaceChildren(...r.models.map((m) => Object.assign(h('option'), { value: m }))); msg(tmsg, r.models.length ? t({ ru: 'Моделей: ', en: 'Models: ' }) + r.models.length : t({ ru: 'Сервер не вернул моделей', en: 'The server returned no models' }), 'note'); if (!mdl.value && r.models[0]) { mdl.value = r.models[0]; mdl.onchange(); } }
      else msg(tmsg, (r && r.error) || 'error', 'note bad');
    });
    urlIn.onchange = async () => {
      const v = urlIn.value.trim(); const m = PROV.find((x) => x.url === v);
      await put({ 'ai.baseUrl': v, 'ai.provider': m ? m.id : 'custom', 'ai.preset': m ? m.preset || 'custom' : 'custom' });
      curProv = m ? m.id : 'custom'; aiState.cfg = (await inv('ai_config')) || cfg; draw();
      msg(tmsg, aiState.cfg.urlError || '', 'note bad');
    };
    mdl.onchange = () => put({ 'ai.model': mdl.value.trim() });
    const ai = h('div', 'inline'); ai.append(urlIn, mdl, listB); mdl.style.flex = '0 0 38%'; ai.append(dl);
    const tmsg = h('div', 'note'); tmsg.style.marginTop = '8px'; tmsg.textContent = cfg.urlError || ''; if (cfg.urlError) tmsg.className = 'note bad';
    ar.append(ai); c.append(ar);
    // test
    const tr = h('div', 'pad inline');
    const test = btn(t({ ru: 'Проверить', en: 'Test' }), async () => {
      await put({ 'ai.baseUrl': urlIn.value.trim(), 'ai.model': mdl.value.trim() });
      test.disabled = true; msg(tmsg, t({ ru: 'Проверяю…', en: 'Testing…' }), 'note');
      const r = await inv('ai_test'); test.disabled = false;
      if (r && r.ok) msg(tmsg, t({ ru: 'Работает', en: 'Works' }) + ' · ' + r.ms + ' ' + t({ ru: 'мс', en: 'ms' }), 'note ok');
      else msg(tmsg, (r && r.error) || t({ ru: 'Нет ответа', en: 'No answer' }), 'note bad');
    });
    tr.append(test, tmsg); tmsg.style.margin = '0 0 0 8px'; c.append(tr);
    const hosts = h('div', 'footnote'); p.append(hosts);
    inv('net_features_status').then((s) => { const f = s && s.features && s.features.find((x) => x.id === 'ai'); if (f) hosts.textContent = t({ ru: 'Запросы уходят только на: ', en: 'Requests go only to: ' }) + (f.hosts.join(', ') || '—') + ' · ' + t({ ru: 'вызовов: ', en: 'calls: ' }) + f.calls; });
    draw();
  }

  // ---- AI setup wizard (the page is ai-setup.js; Rust side: aisetup.rs)
  function pAiSetup(p) {
    if (window.PillAiSetupPage) return window.PillAiSetupPage(p, { h, card, cap, row, sw, seg, btn, msg, inv, get, put, link, t, go: show, onLeave: (f) => cleanups.push(f) });
    p.append(h('div', 'footnote', t({ ru: 'Страница установки ИИ не загрузилась (ai-setup.js).', en: 'The AI setup page did not load (ai-setup.js).' })));
  }

  // ---- Voice to text (the real page is voice-ui.js; the stub below is only the fallback when that file is missing)
  function pVoice(p) {
    if (window.PillVoiceSettingsPage) return window.PillVoiceSettingsPage(p, { h, card, row, sw, seg, btn, msg, inv, get, put, link, t, onLeave: (f) => cleanups.push(f) });
    p.append(h('div', 'dis-banner', t({ ru: 'Голос в текст ещё не встроен в Pill: страница показывает, как он будет выглядеть. Ничего отсюда не сохраняется и никуда не отправляется.', en: 'Voice to text is not built into Pill yet: this page shows how it will look. Nothing here is saved or sent anywhere.' })));
    const c = card(p, 'dis');
    c.style.opacity = '0.55'; c.style.pointerEvents = 'none';
    c.append(h('div', 'pad txt', t({ ru: 'Нажми Ctrl+Alt+D, скажи и нажми ещё раз: текст окажется в буфере. В чате для этого есть кнопка с микрофоном. Запись отправляется в выбранный сервис распознавания.', en: 'Press Ctrl+Alt+D, speak and press again: the text lands on the clipboard. The chat has a microphone button for the same. The recording is sent to the recognition service you choose.' })));
    const top = h('div', 'pad'); const chips = h('div', 'chips dis');
    [['Groq · Whisper', 1], ['Google Gemini', 1], ['OpenAI', 0]].forEach(([n, f], i) => { const b = h('button', 'chip' + (i === 0 ? ' on' : ''), n); b.type = 'button'; b.disabled = true; if (f) b.append(h('span', 'badge free', t({ ru: 'бесплатно', en: 'free' }))); chips.append(b); });
    top.append(chips, h('div', 'note', t({ ru: 'Бесплатно, без карты: до 8 часов речи в день, ответ за секунду. Хорошо понимает русский.', en: 'Free, no card: up to 8 hours of speech a day, answers within a second. Understands Russian well.' })));
    top.lastChild.style.marginTop = '12px'; c.append(top);
    const ar = h('div', 'row block'); ar.append(h('div', 'l', [h('b', '', t({ ru: 'Адрес API и модель', en: 'API address and model' }))]));
    const i1 = Object.assign(h('input', 'field'), { value: 'https://api.groq.com/openai/v1', disabled: true }), i2 = Object.assign(h('input', 'field'), { value: 'whisper-large-v3-turbo', disabled: true });
    i2.style.fontFamily = 'var(--mono)'; i2.style.fontSize = '12px'; i2.style.flex = '0 0 38%';
    const il = h('div', 'inline'); il.append(i1, i2); ar.append(il); c.append(ar);
    row(c, t({ ru: 'Язык речи', en: 'Spoken language' }), t({ ru: 'Подсказка сервису: так он реже путает языки', en: 'A hint for the service: it mixes languages up less often' }), seg([['auto', t({ ru: 'Авто', en: 'Detect' })], ['ru', 'Русский'], ['en', 'English']], () => 'auto', () => {}));
    row(c, t({ ru: 'Вставлять текст сразу', en: 'Paste the text at once' }), t({ ru: 'В окно, где стоял курсор. Текст остаётся в буфере в любом случае', en: 'Into the window where the caret was. The text stays on the clipboard either way' }), (() => { const s = sw('', { read: () => false, set: () => {} }); s.input.disabled = true; return s; })());
  }

  // ---- Weather
  async function pWeather(p) {
    let c = card(p);
    row(c, t({ ru: 'Погода', en: 'Weather' }), t({ ru: 'Выключена по умолчанию. Включённая, она обращается только к api.met.no и nominatim.openstreetmap.org', en: 'Off by default. When on, it talks only to api.met.no and nominatim.openstreetmap.org' }), sw('net.weather'));
    const body = h('div', 'pad');
    const title = h('b', ''); title.style.cssText = 'display:block;font-weight:500;font-size:13px;line-height:18px;margin-bottom:8px';
    const setTitle = () => { title.textContent = t({ ru: 'Город: ', en: 'City: ' }) + (get('weather.city') || t({ ru: 'не выбран', en: 'not chosen' })); };
    syncs.push(setTitle); setTitle();
    body.append(title, h('div', 'txt', t({ ru: 'Прогноз — Норвежский метеорологический институт (MET Norway), поиск городов — OpenStreetMap. Ключ и регистрация не нужны. Город хранится только на этом компьютере; по адресу в сети он не определяется.', en: 'The forecast comes from the Norwegian Meteorological Institute (MET Norway), the city search from OpenStreetMap. No key or registration needed. The city is stored only on this computer; it is not detected from your network address.' })));
    const q = Object.assign(h('input', 'field'), { placeholder: t({ ru: 'Город: Казань, Berlin, Алматы…', en: 'City: Kazan, Berlin, Almaty…' }) }); q.spellcheck = false;
    const found = h('div', 'found'); const st = h('div', 'note'); st.style.marginTop = '8px';
    const go = async () => {
      const v = q.value.trim(); if (v.length < 2) return;
      msg(st, t({ ru: 'Ищу…', en: 'Searching…' }), 'note'); found.replaceChildren();
      const r = await inv('weather_search_city', { query: v, lang: I18n.lang });
      if (!r) { msg(st, t({ ru: 'Нет ответа', en: 'No answer' }), 'note bad'); return; }
      if (r.status === 'off') { msg(st, t({ ru: 'Сначала включи погоду выше: поиск обращается к серверу карт.', en: 'Switch the weather on above first: the search talks to a map server.' }), 'note warn'); return; }
      if (r.status !== 'ok') { msg(st, r.message || 'error', 'note bad'); return; }
      msg(st, r.results.length ? '' : t({ ru: 'Ничего не найдено', en: 'Nothing found' }), 'note');
      r.results.forEach((x) => { const b = h('button', '', [x.name, h('span', '', x.detail || '')]); b.type = 'button'; b.onclick = async () => { const ok = await inv('weather_set_city', { name: x.name, lat: x.lat, lon: x.lon }); if (ok) { S['weather.city'] = x.name; setTitle(); found.replaceChildren(); q.value = ''; msg(st, t({ ru: 'Город сохранён', en: 'City saved' }), 'note ok'); } }; found.append(b); });
    };
    q.onkeydown = (e) => { if (e.key === 'Enter') go(); };
    const ir = h('div', 'inline'); ir.style.marginTop = '12px'; ir.append(q, btn(t({ ru: 'Найти', en: 'Find' }), go));
    body.append(ir, st, found); c.append(body);
    row(c, t({ ru: 'Единицы', en: 'Units' }), '', segKey('weather.units', [['metric', '°C · ' + t({ ru: 'м/с', en: 'm/s' })], ['imperial', '°F · mph']]));
    const rs = h('div', 'note');
    row(c, t({ ru: 'Обновить прогноз', en: 'Refresh the forecast' }), t({ ru: 'Обычно прогноз обновляется сам, не чаще раза в полчаса', en: 'The forecast refreshes by itself, at most once in half an hour' }),
      btn(t({ ru: 'Обновить сейчас', en: 'Refresh now' }), async (e) => {
        const b = e.currentTarget; b.disabled = true; msg(rs, t({ ru: 'Запрашиваю…', en: 'Requesting…' }), 'note');
        const r = await inv('weather_get', { force: true, tz: -new Date().getTimezoneOffset() }); b.disabled = false;
        if (!r) msg(rs, t({ ru: 'Нет ответа', en: 'No answer' }), 'note bad');
        else if (r.status === 'off') msg(rs, t({ ru: 'Погода выключена', en: 'Weather is off' }), 'note warn');
        else if (r.status === 'nocity') msg(rs, t({ ru: 'Сначала выбери город', en: 'Choose a city first' }), 'note warn');
        else if (r.status === 'ok') msg(rs, t({ ru: 'Обновлено: ', en: 'Updated: ' }) + (r.city || '') + (r.now && r.now.temp != null ? ', ' + Math.round(r.now.temp) + '°' : ''), 'note ok');
        else msg(rs, r.message || 'error', 'note bad');
      }));
    c.lastChild.querySelector('.l').append(rs); rs.style.marginTop = '4px';
  }

  // ---- Sound
  const playTone = (kind) => inv('sound_play', { tone: kind }); // Rust (timers.rs): the same synthesised signal the reminders play
  async function pSound(p) {
    const devs = (await inv('media_devices')) || [];
    const mics = (await inv('media_capture_devices')) || [];
    let c = card(p);
    row(c, t({ ru: 'Чью громкость регулирует остров', en: 'Whose volume the pill controls' }),
      t({ ru: 'Звук при этом никуда не переключается: меняется только то, чью громкость показывают и меняют ползунок, колесо и карточка. Куда идёт звук, выбирается в списке рядом с громкостью на вкладке «Главная»', en: 'The sound is not switched anywhere: only the device whose volume the slider, the wheel and the card show and change. Where the sound goes is chosen in the list next to the volume on the Home tab' }),
      select([['', t({ ru: 'Как в Windows', en: 'Same as Windows' })], ...devs.map((d) => [d.id, d.name])], () => get('sound.volumeDevice') || '', (v) => put({ 'sound.volumeDevice': v })));
    row(c, t({ ru: 'Какой микрофон выключает и запоминает остров', en: 'Which microphone the pill mutes and remembers' }), t({ ru: 'Его выключает клавиша «Микрофон» и переключатель в центре управления. Системный микрофон по умолчанию Pill не переключает', en: 'The “Microphone” key and the control-centre switch mute it. Pill does not change the Windows default microphone' }),
      select([['', t({ ru: 'Как в Windows', en: 'Same as Windows' })], ...mics.map((d) => [d.id, d.name])], () => get('sound.micDevice') || '', (v) => put({ 'sound.micDevice': v })));
    row(c, t({ ru: 'Жидкий край по частотам', en: 'Reactive edge' }), t({ ru: 'Край острова двигается под музыку: слева низкие частоты, справа высокие', en: 'The pill edge moves with the music: low frequencies on the left, high on the right' }), sw('audio.reactive'));
    cap(p, t({ ru: 'Напоминания', en: 'Reminders' }));
    c = card(p);
    row(c, t({ ru: 'Звук напоминаний', en: 'Sound for reminders' }), t({ ru: 'Короткий сигнал, когда срабатывает напоминание или таймер', en: 'A short signal when a reminder or a timer goes off' }), sw('sound.reminders'));
    const tone = segKey('sound.reminderTone', [['chime', t({ ru: 'Мягкий', en: 'Soft' })], ['bell', t({ ru: 'Звонок', en: 'Bell' })], ['beep', t({ ru: 'Сигнал', en: 'Beep' })]]);
    row(c, t({ ru: 'Сигнал', en: 'Signal' }), '', [tone, btn(t({ ru: 'Проверить', en: 'Play' }), () => playTone(get('sound.reminderTone')), 'sm')]);
  }

  // ---- Live events
  const LIVE = [
    ['live.volume', { ru: 'Громкость', en: 'Volume' }, { ru: 'Полоска при изменении громкости, колесо мыши над островом', en: 'A bar when the volume changes, the mouse wheel over the pill' }, true],
    ['live.music', { ru: 'Музыка', en: 'Music' }, { ru: 'Название трека и эквалайзер в пилюле, пока играет музыка', en: 'The track title and an equaliser in the pill while music plays' }, true],
    ['live.micCam', { ru: 'Микрофон и камера', en: 'Microphone and camera' }, { ru: 'Точки в острове, пока приложение слушает или снимает', en: 'Dots in the pill while an app listens or films' }, true],
    ['live.downloads', { ru: 'Загрузки', en: 'Downloads' }, { ru: 'Прогресс и карточка с готовым файлом', en: 'Progress and a card with the finished file' }, true],
    ['live.recording', { ru: 'Запись экрана', en: 'Screen recording' }, { ru: 'Красная точка и таймер в пилюле, пока идёт запись', en: 'A red dot and a timer in the pill while a recording runs' }, true],
    ['live.notifications', { ru: 'Уведомления', en: 'Notifications' }, { ru: 'Карточки с событиями вместо всплывающих окон', en: 'Event cards instead of pop-up windows' }, true],
    ['live.reminders', { ru: 'Напоминания и таймеры', en: 'Reminders and timers' }, { ru: 'Карточка, когда напоминание или таймер сработали', en: 'A card when a reminder or a timer goes off' }, true],
    ['live.keyboardLayout', { ru: 'Раскладка клавиатуры', en: 'Keyboard layout' }, { ru: 'Короткая подсказка при смене языка ввода и Caps Lock', en: 'A short hint when the input language or Caps Lock changes' }, true],
    ['live.copy', { ru: 'Реакция на копирование', en: 'Reaction to copying' }, { ru: 'Остров слегка вздрагивает, когда в буфер попал новый текст', en: 'The pill twitches slightly when new text reaches the clipboard' }, true],
    ['live.obs', { ru: 'OBS: запись и трансляция', en: 'OBS: recording and streaming' }, { ru: 'Пока OBS записывает или ведёт трансляцию, в пилюле горит REC или LIVE, а сохранённая запись приходит карточкой. Нужен сервер WebSocket самого OBS: Сервис → Настройки сервера WebSocket → «Включить сервер WebSocket»', en: 'While OBS records or streams, REC or LIVE shows in the pill and the saved recording arrives as a card. Needs the WebSocket server of OBS itself: Tools → WebSocket Server Settings → “Enable WebSocket server”' }, false],
    ['live.agents', { ru: 'ИИ-агенты в терминале', en: 'AI agents in the terminal' }, { ru: 'Claude Code и Codex: в пилюле видно, что агент работает, ждёт ответа или закончил, а когда он ждёт или закончил — приходит карточка', en: 'Claude Code and Codex: the pill shows that an agent works, waits for an answer or has finished, and a card arrives when it waits or finishes' }, true],
    ['live.steam', { ru: 'Загрузки Steam', en: 'Steam downloads' }, { ru: 'Игры и обновления, которые качает Steam: сколько осталось и скорость, а в конце карточка', en: 'Games and updates Steam downloads: time left and speed, and a card at the end' }, true],
    ['live.sysmon', { ru: 'Монитор системы', en: 'System monitor' }, { ru: 'Загрузка процессора, памяти и видеокарт NVIDIA (с температурой) на вкладке «Управление»; в пилюле предупреждение, если видеокарта горячая (от 85 °C) или процессор забит (от 95 % дольше 10 с)', en: 'CPU, memory and NVIDIA GPU load (with temperature) on the Controls tab; a warning in the pill when a GPU is hot (85 °C and up) or the CPU is pinned (95 % for over 10 s)' }, true],
    ['live.dlDone', { ru: 'Готовый файл в «Загрузках»', en: 'Finished file in Downloads' }, { ru: 'Карточка «Загружено: имя файла», когда файл докачался в папку «Загрузки»; клик по карточке открывает папку с выделенным файлом', en: 'A “Downloaded: file name” card when a file finishes in your Downloads folder; a click on the card opens the folder with the file selected' }, true],
    ['live.usb', { ru: 'Флешки и съёмные диски', en: 'USB and removable drives' }, { ru: 'Карточка, когда съёмный диск подключили или отключили', en: 'A card when a removable drive is connected or removed' }, true],
    ['live.battery', { ru: 'Батарея', en: 'Battery' }, { ru: 'Предупреждение при заряде 20 % и 10 % на ноутбуке, а также «Заряжается» и «Заряжено»; на компьютере без батареи ничего не делает', en: 'A warning at 20 % and 10 % on a laptop, plus “Charging” and “Fully charged”; does nothing on a PC without a battery' }, true],
    ['live.calendar', { ru: 'Ближайшее событие календаря', en: 'Next calendar event' }, { ru: 'За 10 минут до начала события из локального файла .ics: «Через 10 мин: название». Файл только читается, без сети и аккаунтов', en: 'Ten minutes before an event from a local .ics file: “In 10 min: title”. The file is only read, no network, no accounts' }, false],
  ];
  // events Pill has no source for yet: the switch is shown but cannot be flipped (nothing would happen)
  const LIVE_NOSRC = new Set([]); // OBS, Steam and notifications are real now (obs.rs, steam.rs, notif.rs)
  function pLive(p) {
    const c = card(p);
    LIVE.forEach(([key, title, sub, def]) => {
      DEF[key] = def;
      const w = sw(key);
      const nosrc = LIVE_NOSRC.has(key);
      if (nosrc) w.input.disabled = true;
      const lr = row(c, t(title), t(sub) + (nosrc ? ' ' + t({ ru: '(Пока не встроено: у Pill нет источника этого события.)', en: '(Not built yet: Pill has no source for this event.)' }) : ''), w);
      if (window.PillLiveExtRow) window.PillLiveExtRow(key, lr, { h, btn, msg, inv, get, put, t, onLeave: (f) => cleanups.push(f) }); // OBS / Steam / notifications: status line and controls (settings-live-ext.js)
      if (key === 'live.agents') {
        const bl = h('div', 'pad');
        const pre = h('pre', 'snip'); pre.hidden = true;
        const note = h('div', 'note', t({ ru: 'Pill ничего не дописывает в настройки агентов сам. Нажми «Показать», скопируй фрагменты и вставь в настройки Claude Code (хуки в ~/.claude/settings.json) и Codex (notify в ~/.codex/config.toml). Действует для новых сессий.', en: 'Pill never edits agent settings by itself. Press “Show”, copy the snippets and paste them into the settings of Claude Code (hooks in ~/.claude/settings.json) and Codex (notify in ~/.codex/config.toml). Applies to new sessions.' }));
        const show = btn(t({ ru: 'Показать', en: 'Show' }), async () => {
          const r = await inv('agents_hook_snippets'); if (!r) return;
          pre.textContent = 'Claude Code - ' + (r.claude_code_where || '') + '\n' + (r.claude_code || '') + '\n\nCodex - ' + (r.codex_where || '') + '\n' + (r.codex || '');
          pre.hidden = false;
        }, 'sm');
        const top = h('div', 'inline'); top.style.cssText = 'align-items:center;gap:16px'; note.style.flex = '1'; top.append(note, show);
        bl.append(top, pre); c.append(bl);
      }
    });
    p.append(h('div', 'footnote', t({ ru: 'Что читается. Громкость: только уровень и «без звука» устройства вывода по умолчанию. Микрофон и камера: в реестре Windows (только чтение, ветка вашего пользователя) отметки о том, какое приложение сейчас пользуется устройством; звук и изображение не открываются и не читаются. Загрузки: только имена и размеры файлов во временных браузерных файлах (.crdownload, .part, .tmp) в папке «Загрузки»; содержимое файлов не читается. Готовый файл: только имена и размеры файлов в «Загрузках». Съёмные диски: список букв дисков. Батарея: уровень заряда и подключено ли питание. Календарь: только файл .ics, который вы указали. Пока переключатель выключен, ничего из этого не опрашивается.', en: 'What is read. Volume: only the level and the mute flag of the default output device. Microphone and camera: marks in the Windows registry (read-only, your user branch) saying which app is using the device right now; sound and picture are never opened or read. Downloads: only the names and sizes of browser temp files (.crdownload, .part, .tmp) in your Downloads folder; file contents are not read. Finished file: only the names and sizes of files in Downloads. Removable drives: the list of drive letters. Battery: the charge level and whether the power is plugged in. Calendar: only the .ics file you point to. While a switch is off, none of this is polled.' })));
  }

  // ---- Clipboard
  async function pClipboard(p) {
    let c = card(p);
    row(c, t({ ru: 'Клик по записи вставляет её', en: 'A click on an entry pastes it' }), t({ ru: 'Иначе запись только копируется. Shift+клик всегда только копирует', en: 'Otherwise the entry is only copied. Shift+click always only copies' }), sw('clipboard.clickPaste'));
    row(c, t({ ru: 'Размер истории', en: 'History size' }), t({ ru: 'Закреплённые записи не считаются', en: 'Pinned entries do not count' }), segKey('clipboard.maxItems', [[50, '50'], [200, '200'], [500, '500'], [1000, '1000']], true));
    row(c, t({ ru: 'Хранить записи', en: 'Keep entries' }), t({ ru: 'Старше этого срока записи стираются, закреплённые остаются', en: 'Older entries are erased, pinned ones stay' }),
      segKey('clipboard.retentionDays', [[7, t({ ru: '7 дн.', en: '7 d' })], [30, t({ ru: '30 дн.', en: '30 d' })], [90, t({ ru: '90 дн.', en: '90 d' })], [365, t({ ru: 'год', en: '1 y' })]], true));
    row(c, t({ ru: 'Самая большая картинка', en: 'Largest picture' }), t({ ru: 'Картинки крупнее этого размера в историю не попадают', en: 'Pictures larger than this do not enter the history' }),
      select([[64, '64 MB'], [256, '256 MB'], [512, '512 MB'], [1024, '1024 MB']], () => get('clipboard.maxImageMb'), (v) => put({ 'clipboard.maxImageMb': +v })));
    const pr = row(c, t({ ru: 'Приватный режим', en: 'Private mode' }), t({ ru: 'История не записывается, пока включено', en: 'Nothing is recorded while it is on' }), null);
    let paused = false;
    const psw = sw('', { read: () => paused, set: async (on) => { paused = on; await inv('clip_pause', { paused: on }); } });
    pr.append(h('div', 'c', psw));
    const cnt = h('div', 'footnote');
    const refresh = async () => { const s = await inv('clip_status'); if (s) { paused = !!s.paused; psw.input.checked = paused; cnt.textContent = t({ ru: 'В истории записей: ', en: 'Entries in the history: ' }) + s.count; } };
    row(c, t({ ru: 'Не записывать пароли и ключи', en: 'Skip passwords and keys' }), t({ ru: 'Эвристика: длинные случайные строки, номера карт, токены не попадают в историю', en: 'A heuristic: long random strings, card numbers and tokens do not enter the history' }), sw('clipboard.skipSecrets'));
    row(c, t({ ru: 'Не запоминать копирование из программ', en: 'Do not remember copying from programs' }),
      t({ ru: 'По одной на строку, имя процесса. Менеджеры паролей, которые помечают копирование, и так пропускаются', en: 'One per line, the process name. Password managers that mark their copies are skipped anyway' }),
      area('clipboard.denyList', { mono: true, rows: 5,
        read: () => (Array.isArray(S['clipboard.denyList']) ? S['clipboard.denyList'] : ['keepass', 'keepassxc', '1password', 'bitwarden', 'lastpass', 'dashlane', 'enpass', 'mstsc']).map((x) => x + '.exe').join('\n'),
        write: (txt) => put({ 'clipboard.denyList': txt.split(/[\n,;]+/).map((x) => x.trim().replace(/\.exe$/i, '').toLowerCase()).filter(Boolean) }) }), { block: true });
    // wipe with confirmation
    const wr = row(c, t({ ru: 'Стереть историю', en: 'Wipe history' }), t({ ru: 'Удаляет все записи и картинки, закреплённые тоже', en: 'Deletes every entry and picture, pinned ones too' }), null);
    const wc = h('div', 'c'); wr.append(wc);
    const idle = () => { wc.replaceChildren(btn(t({ ru: 'Стереть…', en: 'Wipe…' }), ask, 'danger')); };
    const ask = () => { wc.replaceChildren(btn(t({ ru: 'Да, стереть всё', en: 'Yes, wipe everything' }), async () => { await inv('clip_wipe'); idle(); await refresh(); msg(wr.sub, t({ ru: 'Готово: история пуста', en: 'Done: the history is empty' }), 'ok'); }, 'danger'), btn(t({ ru: 'Отмена', en: 'Cancel' }), idle)); };
    idle();
    p.append(cnt);
    cap(p, t({ ru: 'Скриншоты и запись экрана', en: 'Screenshots and screen recording' }));
    c = card(p);
    row(c, t({ ru: 'Формат записи экрана', en: 'Screen recording format' }), t({ ru: 'MP4 — для длинных записей, GIF — короткие ролики в чат (до минуты, 15 кадров в секунду; после остановки GIF собирается несколько секунд)', en: 'MP4 for long recordings, GIF for short clips for a chat (up to a minute, 15 frames per second; the GIF takes a few seconds to build after you stop)' }), segKey('capture.format', [['mp4', 'MP4'], ['gif', 'GIF']]));
    row(c, t({ ru: 'Частота кадров записи', en: 'Recording frame rate' }), t({ ru: 'Для MP4. 60 кадров плавнее, но файл больше и компьютеру тяжелее', en: 'For MP4. 60 frames is smoother, but the file is bigger and the computer works harder' }), segKey('capture.fps', [[30, '30'], [60, '60']], true));
    row(c, t({ ru: 'Кодировщик видео', en: 'Video encoder' }), t({ ru: 'Авто берёт видеокарту NVIDIA (NVENC), если она работает, иначе процессор (x264)', en: 'Auto uses the NVIDIA card (NVENC) when it works, otherwise the processor (x264)' }),
      segKey('capture.encoder', [['auto', t({ ru: 'Авто', en: 'Auto' })], ['nvenc', 'NVENC'], ['x264', 'x264']]));
    row(c, t({ ru: 'Скриншот по Enter и целого экрана', en: 'Screenshot on Enter and of the whole screen' }), t({ ru: 'Что делают Enter, двойной щелчок в выделении и скриншот всего экрана. Кнопки «Копировать» и «Сохранить» в панели делают ровно то, что написано', en: 'What Enter, a double click in the selection and the whole-screen shot do. The Copy and Save buttons of the panel do exactly what they say' }),
      segKey('capture.shotAction', [['both', t({ ru: 'Копия и файл', en: 'Copy and file' })], ['copy', t({ ru: 'Только копия', en: 'Copy only' })], ['save', t({ ru: 'Только файл', en: 'File only' })]]));
    row(c, t({ ru: 'Показать файл в папке после сохранения', en: 'Show the file in its folder after saving' }), t({ ru: 'Открывает Проводник с выделенным скриншотом', en: 'Opens Explorer with the screenshot selected' }), sw('capture.revealAfterSave'));
    const pathField = (key, ph) => { const i = h('input', 'field'); i.placeholder = ph; i.spellcheck = false; i.onchange = () => put({ [key]: i.value.trim() }); syncs.push(() => { if (document.activeElement !== i) i.value = get(key) || ''; }); i.value = get(key) || ''; return i; };
    row(c, t({ ru: 'Папка скриншотов', en: 'Screenshot folder' }), t({ ru: 'Пусто = Изображения\\Screenshots\\Pill', en: 'Empty = Pictures\\Screenshots\\Pill' }), pathField('capture.folder', 'C:\\…'), { block: true });
    row(c, t({ ru: 'Папка видео', en: 'Video folder' }), t({ ru: 'Пусто = Видео\\Pill', en: 'Empty = Videos\\Pill' }), pathField('capture.videoFolder', 'C:\\…'), { block: true });
    p.append(h('div', 'footnote', t({ ru: 'Запись идёт через ffmpeg: его нужно поставить самому (Pill ничего не скачивает). Если он лежит в нестандартном месте, путь к ffmpeg.exe можно указать в settings.json, ключ capture.ffmpeg.', en: 'Recording goes through ffmpeg: install it yourself (Pill downloads nothing). If it lives in an unusual place, set the path to ffmpeg.exe in settings.json, key capture.ffmpeg.' })));
    refresh();
  }

  // ---- Modules
  const MODS = [
    ['media', { ru: 'Медиаплеер', en: 'Media player' }, { ru: 'Обложка, управление и эквалайзер', en: 'Cover, controls and equaliser' }],
    ['shelf', { ru: 'Полка файлов', en: 'File shelf' }, { ru: 'Перетащи файлы на остров, они полежат, пока не понадобятся', en: 'Drop files on the pill, they wait until you need them' }],
    ['launcher', { ru: 'Лаунчер', en: 'Launcher' }, { ru: 'Ярлыки программ на вкладке «Главная»', en: 'Program shortcuts on the Home tab' }],
    ['fileSearch', { ru: 'Поиск файлов в командной строке', en: 'File search in the command bar' }, { ru: 'Файлы и папки через Everything или поиск Windows', en: 'Files and folders through Everything or Windows search' }],
    ['clipboard', { ru: 'История буфера', en: 'Clipboard history' }, { ru: 'Вкладка «Буфер»', en: 'The Clipboard tab' }],
    ['text', { ru: 'Текстовые инструменты', en: 'Text tools' }, { ru: 'Вкладка «Текст»: регистр, раскладка, JSON, Base64', en: 'The Text tab: case, layout, JSON, Base64' }],
    ['chat', { ru: 'Чат с ИИ', en: 'AI chat' }, { ru: 'Вкладка «Чат»', en: 'The Chat tab' }],
    ['note', { ru: 'Заметки и таймер', en: 'Notes and timer' }, { ru: 'Вкладка «Заметка», напоминания', en: 'The Note tab, reminders' }],
    ['weather', { ru: 'Погода', en: 'Weather' }, { ru: 'Вкладка «Погода»', en: 'The Weather tab' }],
    ['controls', { ru: 'Управление системой', en: 'System controls' }, { ru: 'Вкладка «Управление»: сеть, Bluetooth, тема, питание', en: 'The Controls tab: network, Bluetooth, theme, power' }],
  ];
  function pModules(p) {
    const c = card(p);
    MODS.forEach(([id, title, sub]) => { DEF['module.' + id] = true; row(c, t(title), t(sub), sw('module.' + id)); });
    p.append(h('div', 'footnote', t({ ru: 'Выключенный модуль сразу прячет свою вкладку и перестаёт работать в фоне: история буфера не пишется, плеер не опрашивается, погода не запрашивается. Сами данные (история, заметки, полка, ярлыки) остаются. Напоминания и таймеры срабатывают, даже если вкладка «Заметка» выключена.', en: 'A module that is off hides its tab at once and stops its background work: the clipboard history is not written, the player is not polled, the weather is not requested. The data itself (history, notes, shelf, shortcuts) stays. Reminders and timers still fire when the Note tab is off.' })));
  }

  // ---- Updates
  async function pUpdates(p) {
    const info = (await inv('settings_info')) || {};
    const v = info.version || ver || '';
    const c = card(p);
    const r = row(c, t({ ru: 'Версия ', en: 'Version ' }) + v, t({ ru: 'Pill ищет обновления по кнопке и один раз при запуске (это можно выключить ниже). Устанавливается новая версия только по вашему нажатию: она скачивается с GitHub и проверяется подписью.', en: 'Pill looks for updates when you press the button and once at launch (you can turn that off below). A new version is installed only when you click it: it is downloaded from GitHub and its signature is verified.' }), null);
    const out = h('div', 'footnote');
    let found = false;
    const b = btn(t({ ru: 'Проверить сейчас', en: 'Check now' }), async () => {
      if (found) { // second step: download, verify, install, restart
        b.disabled = true; msg(out, t({ ru: 'Скачиваю и устанавливаю…', en: 'Downloading and installing…' }), 'note');
        const res = (await inv('update_install')) || {};
        b.disabled = false;
        if (res.status === 'error') msg(out, t({ ru: 'Не удалось установить: ', en: 'Install failed: ' }) + (res.error || ''), 'note err');
        return;
      }
      b.disabled = true; msg(out, t({ ru: 'Проверяю…', en: 'Checking…' }), 'note');
      const res = (await inv('update_check')) || {};
      b.disabled = false;
      if (res.status === 'available') {
        found = true; b.textContent = t({ ru: 'Установить и перезапустить', en: 'Install and restart' });
        msg(out, t({ ru: 'Доступна версия ', en: 'Version available: ' }) + res.version + (res.notes ? '. ' + String(res.notes).slice(0, 300) : ''), 'note ok');
      } else if (res.status === 'none') msg(out, t({ ru: 'У вас последняя версия.', en: 'You are up to date.' }), 'note ok');
      else msg(out, t({ ru: 'Не удалось проверить: ', en: 'Could not check: ' }) + (res.error || ''), 'note err');
    });
    r.append(h('div', 'c', b));
    row(c, t({ ru: 'Проверять при запуске', en: 'Check on start' }), t({ ru: 'Через ~20 секунд после старта один запрос к GitHub. Установить обновление всё равно нужно вручную.', en: 'One request to GitHub ~20 s after launch. Installing is still up to you.' }), sw('updateAuto'));
    c.append(out);
    const d = h('details', 'disc');
    d.append(h('summary', '', t({ ru: 'Что нового', en: 'What is new' })));
    d.append(h('ul', '', [t({ ru: 'Отдельное окно настроек, как у оригинала: персонаж, общее, клавиши, ИИ, погода, звук, живые события, буфер, модули.', en: 'A separate settings window: character, general, hotkeys, AI, weather, sound, live events, clipboard, modules.' })].map((x) => h('li', '', x))));
    c.append(d);
    if (info.dir) p.append(h('div', 'footnote', t({ ru: 'Данные и настройки лежат здесь: ', en: 'Data and settings live here: ' }) + info.dir));
  }

  // keyed seg helper that also applies the language switch
  function segKey(key, opts, num) {
    return seg(opts, () => get(key), (v) => {
      const val = num ? +v : v;
      if (key === 'language') { put({ language: val }).then(() => { I18n.set(val); renderAll(); }); return; }
      put({ [key]: val });
    });
  }

  // ---------------------------------------------------------------------------------------------------------- shell: sidebar, pages, window
  function buildSide() {
    const side = $('#side'); side.replaceChildren();
    const head = h('div', 'sb-head');
    const logo = h('div', 'sb-logo', [h('i')]);
    head.append(logo, h('div', '', [h('div', 'sb-name', 'Pill'), h('div', 'sb-ver', ver ? t({ ru: 'Версия ', en: 'Version ' }) + ver : '')]));
    const list = h('div'); list.id = 'navlist';
    PAGES.forEach((pg) => {
      if (pg.sep) { list.append(h('div', 'sb-sep')); return; }
      const b = h('button', 'sb-item'); b.type = 'button'; b.dataset.id = pg.id;
      b.innerHTML = svg(IC[pg.icon]); b.append(t(pg.name));
      b.onclick = () => show(pg.id);
      list.append(b);
    });
    const foot = h('div', 'sb-foot', [h('div', 'sb-note', t({ ru: 'Изменения сохраняются сразу', en: 'Changes are saved instantly' }))]);
    const q = h('button', 'sb-item quit'); q.type = 'button'; q.innerHTML = svg(IC.power); q.append(t({ ru: 'Выйти из Pill', en: 'Quit Pill' }));
    q.onclick = () => inv('settings_quit');
    foot.append(q);
    side.append(head, list, foot);
  }
  async function show(id) {
    const pg = PAGES.find((x) => x.id === id) || PAGES[0];
    stopRec();
    cleanups.forEach((f) => safe(f)); cleanups = []; syncs = [];
    cur = pg.id;
    document.querySelectorAll('.sb-item[data-id]').forEach((b) => b.classList.toggle('on', b.dataset.id === pg.id));
    const page = $('#page');
    const next = h('div');
    next.append(h('h1', '', t(pg.head || pg.name)));
    page.replaceChildren(next);     // hard cut: the old page is gone in the same frame
    $('#content').scrollTop = 0;
    await safe(pg.build, next);
  }
  function renderAll() { buildSide(); return show(cur || 'character'); }

  async function init() {
    const [s, info] = await Promise.all([inv('settings_get'), inv('settings_info')]);
    if (s) S = s;
    if (info) ver = info.version || '';
    I18n.set(get('language'));
    const want = new URLSearchParams(location.search).get('page');
    cur = PAGES.some((x) => x.id === want) ? want : 'character';
    buildSide();
    await show(cur);
    // caption buttons
    $('#b-min').onclick = () => win && win.minimize();
    $('#b-max').onclick = () => win && win.toggleMaximize();
    $('#b-close').onclick = () => inv('settings_close');
    const syncMax = async () => { try { document.body.classList.toggle('max', !!(win && (await win.isMaximized()))); } catch (e) { /* no window API */ } };
    if (win && win.onResized) win.onResized(syncMax);
    syncMax();
    if (T.event && T.event.listen) {
      T.event.listen('settings:changed', (e) => {
        if (e.payload && typeof e.payload === 'object') S = Object.assign({}, e.payload);
        if (I18n.lang !== (I18n.set(get('language')))) renderAll(); else sync();
      });
      T.event.listen('settings:page', (e) => { if (PAGES.some((x) => x.id === e.payload)) show(e.payload); });
    }
    // show the window after the first paint (it is created hidden: no white flash), then play the short entrance
    requestAnimationFrame(() => requestAnimationFrame(async () => {
      try { if (win) { await win.show(); await win.setFocus(); } } catch (e) { /* window API unavailable */ }
      $('#win').classList.add('ready');
    }));
  }
  window.__settings = { show, get: () => S, renderAll };
  init();
})();
