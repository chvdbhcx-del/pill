// Command bar page. Talks to Rust only through cmd_toggle / cmd_query / cmd_run (+ settings_get for language and theme);
// events bar:shown, bar:hidden. Owner: pill agent (restyle) over the command agent's logic.
// `? question` / `ии question` / `ai question` goes to cmd_ask (the chat model from Settings > AI); the mic button / F4 drives the same
// voice_* commands as the Text tab (voice.rs, voice:state events) and types the transcript into the input.
(() => {
  const T = window.__TAURI__;
  const invoke = (c, a) => T.core.invoke(c, a).catch((e) => { console.error(c, e); return null; });
  const root0 = document.documentElement;
  const ic = (n, o) => window.pillIcon(n, o);

  // ---- i18n (same cached language as the pill page; settings_get refines it) ----
  const nav = () => (/^ru/i.test(navigator.language || '') ? 'ru' : 'en');
  let lang = (() => { try { const c = localStorage.getItem('pill.lang'); if (c === 'ru' || c === 'en') return c; } catch (e) { /* storage blocked */ } return nav(); })();
  const t = (o) => (typeof o === 'string' ? o : o[lang] ?? o.en);
  const L = {
    ph: { ru: 'Приложения, файлы, заметки, буфер или > команда', en: 'Search apps, files, notes, clipboard, or > run' },
    tip: { ru: 'Попробуй 5*8, 5 км в милях, lock, > ipconfig. Фильтры: c буфер, n заметки, f файлы, a приложения, s настройки', en: 'Try 5*8, 5 km in mi, lock, > ipconfig. Filters: c clipboard, n notes, f files, a apps, s settings' },
    clip: { ru: 'Буфер', en: 'Clipboard' }, note: { ru: 'Заметки', en: 'Notes' }, shelf: { ru: 'Полка', en: 'Shelf' }, reminder: { ru: 'Напоминание', en: 'Reminder' },
    setting: { ru: 'Настройки', en: 'Settings' }, action: { ru: 'Действие', en: 'Action' }, tab: { ru: 'Вкладка', en: 'Tab' },
    app: { ru: 'Приложение', en: 'Application' }, file: { ru: 'Файл', en: 'File' }, folder: { ru: 'Папка', en: 'Folder' },
    shell: { ru: 'Команда', en: 'Command' }, copy: { ru: 'Скопировать', en: 'Copy' }, recent: { ru: 'Недавнее', en: 'Recent' },
    again: { ru: 'Ещё раз Enter', en: 'Press Enter again' },
    ask: { ru: 'Спросить ИИ', en: 'Ask AI' }, askTip: { ru: 'Напиши вопрос после «? », «ии » или «ai »', en: 'Type your question after "? ", "ии " or "ai "' },
    askGo: { ru: 'Enter — отправить', en: 'Enter: send' }, think: { ru: 'Думаю…', en: 'Thinking…' }, copied: { ru: 'Скопировано', en: 'Copied' },
    copyAns: { ru: 'Копировать ответ', en: 'Copy the answer' },
    askOff: { ru: 'ИИ выключен. Enter — открыть Настройки → ИИ и включить', en: 'AI is off. Enter: open Settings > AI to turn it on' },
    askNoUrl: { ru: 'Не указан адрес. Enter — открыть Настройки → ИИ', en: 'No API address. Enter: open Settings > AI' },
    askNoModel: { ru: 'Не выбрана модель. Enter — открыть Настройки → ИИ', en: 'No model chosen. Enter: open Settings > AI' },
    askFail: { ru: 'ИИ не ответил', en: 'The AI did not answer' },
    micTip: { ru: 'Диктовка (F4)', en: 'Dictate (F4)' }, micStop: { ru: 'Остановить диктовку (F4)', en: 'Stop dictation (F4)' }, micBusy: { ru: 'Распознаю…', en: 'Transcribing…' },
    voiceOff: { ru: 'Голос в текст не настроен. Нажми здесь: Настройки → «Голос в текст»', en: 'Voice to text is not set up. Click here: Settings > Voice to text' },
    voiceFail: { ru: 'Диктовка не удалась', en: 'Dictation failed' },
    openTip: { ru: 'Enter — открыть · Ctrl+Enter — показать в папке · Ctrl+Shift+Enter — от администратора', en: 'Enter: open · Ctrl+Enter: show in folder · Ctrl+Shift+Enter: as administrator' },
  };
  const SYS = {
    lock: [{ ru: 'Заблокировать компьютер', en: 'Lock the computer' }, 'lock'], sleep: [{ ru: 'Спящий режим', en: 'Sleep' }, 'moon'],
    hibernate: [{ ru: 'Гибернация', en: 'Hibernate' }, 'moon'], restart: [{ ru: 'Перезагрузка', en: 'Restart' }, 'refresh'],
    shutdown: [{ ru: 'Выключение', en: 'Shut down' }, 'power'], signout: [{ ru: 'Выйти из учётной записи', en: 'Sign out' }, 'logout'],
  };
  const SUBS = { 'Opens your default browser': { ru: 'Откроется в браузере по умолчанию', en: 'Opens your default browser' }, 'Press Enter twice to confirm': { ru: 'Нажми Enter дважды для подтверждения', en: 'Press Enter twice to confirm' },
    'Runs exactly what you type': { ru: 'Выполняется ровно то, что ты введёшь', en: 'Runs exactly what you type' }, 'cmd /c, hidden window': { ru: 'cmd /c, окно скрыто', en: 'cmd /c, hidden window' } };

  const GROUP = { calc: 'Result', convert: 'Result', shell: 'Command', system: 'System', app: 'Applications', folder: 'Folders', file: 'Files', web: 'Web' };
  const ICON = { app: 'apps', file: 'file', folder: 'folder', calc: 'calc', convert: 'convert', web: 'globe', system: 'power', shell: 'terminal', hint: 'sparkles',
    ask: 'sparkles', askcopy: 'copy', clip: 'clipboard', note: 'note', shelf: 'shelf', reminder: 'bell', tab: 'home', setting: 'settings', action: 'bolt' };
  const PLAIN = new Set(['calc', 'convert', 'shell', 'web', 'ask', 'askcopy']); // titles that are not searchable text: no match highlight

  const root = document.getElementById('bar-root');
  root.innerHTML = `<div class="body"><div class="clipbox"><div class="sf"><span class="mag">${ic('search', { size: 20, sw: 1.8 })}</span>` +
    '<input id="q" type="text" spellcheck="false" autocomplete="off"><button type="button" id="mic" tabindex="-1">' + ic('mic', { size: 18, sw: 1.7 }) + '</button>' +
    '<kbd id="hk">Alt+Shift+Space</kbd></div><div id="ans" hidden></div>' +
    '<div id="list"></div><div class="msg" id="msg"></div></div><div class="wv"></div></div>';
  const body = root.firstChild, q = document.getElementById('q'), list = document.getElementById('list');
  const msg = document.getElementById('msg'), wv = body.querySelector('.wv'), ans = document.getElementById('ans'), mic = document.getElementById('mic');

  // wavy outline along the bottom edge (decoration, same idea as the pill)
  function wave() {
    const w = Math.max(40, Math.round(body.getBoundingClientRect().width - 60)), pts = [], taper = 40;
    for (let x = 0; x <= w; x += 3) {
      const e = Math.min(1, x / taper, (w - x) / taper), k = e * e * (3 - 2 * e);
      pts.push(`${x} ${(8 + 2.4 * k * Math.sin((x / 56) * 6.2832)).toFixed(2)}`);
    }
    const d = 'M' + pts.join('L'), NS = 'http://www.w3.org/2000/svg';
    wv.replaceChildren();
    const svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', `0 0 ${w} 16`); svg.setAttribute('preserveAspectRatio', 'none');
    for (const [c, sw] of [['u', 5.2], ['l', 2.1]]) {
      const p = document.createElementNS(NS, 'path');
      p.setAttribute('class', c); p.setAttribute('d', d); p.setAttribute('fill', 'none'); p.setAttribute('stroke-width', sw); p.setAttribute('stroke-linecap', 'round');
      svg.append(p);
    }
    wv.append(svg);
  }
  new ResizeObserver(wave).observe(body);

  let items = [], sel = 0, seq = 0, timer = 0, words = [], confirmTimer = 0, fresh = false, cfg = {}, asking = 0, askId = 0, dictHere = false, copiedTimer = 0;

  const applyLang = () => { q.placeholder = t(L.ph); };
  async function loadCfg() {
    const s = await invoke('settings_get');
    if (!s) return;
    cfg = s;
    const want = s.language === 'ru' || s.language === 'en' ? s.language : nav();
    if (want !== lang) { lang = want; try { localStorage.setItem('pill.lang', lang); } catch (e) { /* ignore */ } }
    root0.classList.toggle('rm', !!s.reduceMotion || matchMedia('(prefers-reduced-motion: reduce)').matches);
    root0.dataset.theme = s.theme === 'light' || (s.theme === 'auto' && matchMedia('(prefers-color-scheme: light)').matches) ? 'light' : 'dark';
    document.getElementById('hk').textContent = String(s['hotkey.command'] || 'Alt+Shift+Space');
    applyLang();
    setMic(micState);
  }
  applyLang();

  // Highlight query words in a title using text nodes only (never innerHTML).
  function hl(el, text) {
    const low = text.toLowerCase(), r = [];
    for (const w of words) for (let i = low.indexOf(w); w && i >= 0; i = low.indexOf(w, i + w.length)) r.push([i, i + w.length]);
    r.sort((a, b) => a[0] - b[0]);
    let pos = 0;
    for (const [s, e] of r) {
      if (e <= pos) continue;
      const s2 = Math.max(s, pos);
      el.append(text.slice(pos, s2));
      const m = document.createElement('mark'); m.textContent = text.slice(s2, e); el.append(m);
      pos = e;
    }
    el.append(text.slice(pos));
  }

  // Rust sends English strings: localise the known ones here
  function view(it) {
    let title = it.title, sub = it.subtitle, icon = ICON[it.kind] || 'file', label = '';
    if (it.kind === 'system') { const m = SYS[it.payload]; if (m) { title = t(m[0]); icon = m[1]; } label = 'Pill'; sub = ''; }
    else if (it.kind === 'web') {
      const m = /^Search (.+?) for "(.*)"$/.exec(it.title);
      if (m) title = lang === 'ru' ? `Искать в браузере: «${m[2]}»` : `Search in the browser: “${m[2]}”`;
      label = m ? m[1] : 'Web'; sub = '';
    } else if (it.kind === 'shell') { title = it.title.replace(/^Run: /, lang === 'ru' ? 'Выполнить: ' : 'Run: '); label = t(L.shell); sub = SUBS[sub] ? t(SUBS[sub]) : sub; }
    else if (it.kind === 'calc' || it.kind === 'convert') { label = t(L.copy); sub = it.subtitle.replace(/\s+-\s+Enter copies$/, ''); }
    else if (it.kind === 'ask') { title = `${t(L.ask)}: ${it.payload}`; sub = askStatus(); label = 'ИИ'; }
    else if (it.kind === 'askcopy') { title = t(L.copyAns); sub = ''; label = t(L.copy); }
    else if (it.kind === 'app') label = t(L.app);
    else if (it.kind === 'file' || it.kind === 'folder') label = t(L[it.kind]);
    else if (L[it.kind]) { label = t(L[it.kind]); if (SUBS[sub]) sub = t(SUBS[sub]); if (it.kind === 'tab') icon = it.payload; }
    else if (SUBS[sub]) sub = t(SUBS[sub]);
    return { title, sub, icon, label };
  }

  // app / file / folder rows show the real shell icon (an app's own icon, a file type's icon, the folder icon); the glyph stays until it loads
  const ICONS = new Map(); // result id -> data URL ('' = none), kept for the session
  function shellIcon(it, ico) {
    const put = (u) => { if (!u) return; const im = new Image(); im.src = u; im.alt = ''; im.draggable = false; ico.classList.add('shell'); ico.replaceChildren(im); };
    if (ICONS.has(it.id)) return put(ICONS.get(it.id));
    invoke('cmd_icon', { id: it.id }).then((u) => { ICONS.set(it.id, u || ''); put(u); });
  }

  function row(it, i) {
    const v = view(it), el = document.createElement('div');
    el.className = `row ${it.kind}` + (fresh ? ' stag' : '') + (i === sel ? ' sel' : '');
    el.style.setProperty('--i', i);
    el.dataset.i = i;
    el.dataset.id = it.id; // FLIP key: rows that survive a keystroke slide instead of jumping
    if (it.kind === 'app' || it.kind === 'file' || it.kind === 'folder') el.title = t(L.openTip);
    const ico = document.createElement('span'); ico.className = 'ico'; ico.innerHTML = ic(v.icon, { size: 20, sw: 1.6 }); // static markup, never user text
    if (it.kind === 'app' || it.kind === 'file' || it.kind === 'folder') shellIcon(it, ico);
    const txt = document.createElement('div'); txt.className = 'txt';
    const tt = document.createElement('div'); tt.className = 't';
    if (!PLAIN.has(it.kind) && it.kind !== 'system') hl(tt, v.title); else tt.textContent = v.title;
    txt.append(tt);
    if (v.sub && it.kind !== 'app') { const s = document.createElement('div'); s.className = 's'; s.textContent = v.sub; txt.append(s); }
    const act = document.createElement('span'); act.className = 'act'; act.append(document.createTextNode(v.label));
    act.insertAdjacentHTML('beforeend', ic('enter', { size: 16, sw: 1.7 }));
    el._sub = v.sub;
    el.append(ico, txt, act);
    return el;
  }

  function render(res, empty) {
    items = res.filter((r) => r.kind !== 'hint');
    sel = 0;
    const fill = () => { list.replaceChildren(); items.forEach((it, i) => list.append(row(it, i))); };
    if (fresh) fill(); else window.PillMotion.flip(list, fill, { sel: '.row', enterNew: true }); // later renders: survivors slide, new rows drop in
    fresh = false; // the stagger plays once per opening, not on every keystroke
    for (const h of res.filter((r) => r.kind === 'hint')) {
      const v = view(h), d = document.createElement('div'); d.className = 'hint'; d.textContent = h.title + (v.sub ? ' - ' + v.sub : ''); list.append(d);
    }
    if (!items.length && empty) { const d = document.createElement('div'); d.className = 'tip'; d.textContent = t(L.tip); list.prepend(d); }
  }

  function select(i) {
    if (!items.length) return;
    sel = (i + items.length) % items.length;
    list.querySelectorAll('.row').forEach((r) => r.classList.toggle('sel', +r.dataset.i === sel));
    list.querySelector('.row.sel')?.scrollIntoView({ block: 'nearest' });
  }

  function nextGroup(dir) {
    if (!items.length) return;
    const g = (i) => GROUP[items[i].kind] || items[i].kind;
    let i = sel;
    for (let n = 0; n < items.length; n++) { i = (i + dir + items.length) % items.length; if (g(i) !== g(sel)) break; }
    if (dir < 0) while (i > 0 && g(i - 1) === g(i)) i--; // land on the first row of the previous group
    select(i);
  }

  // `? question`, `ии question`, `ai question`: a question for the configured chat model instead of a search
  const ASK = /^(?:\?|ии|ai)\s+/i;
  const askReady = () => cfg['net.ai'] === true && String(cfg['ai.baseUrl'] || '').trim() && String(cfg['ai.model'] || '').trim();
  const askStatus = () => (cfg['net.ai'] !== true ? t(L.askOff) : !String(cfg['ai.baseUrl'] || '').trim() ? t(L.askNoUrl) : !String(cfg['ai.model'] || '').trim() ? t(L.askNoModel) : t(L.askGo));

  function clearAns() {
    ans.hidden = true; ans.replaceChildren(); ans.className = '';
    if (asking) { asking = 0; invoke('ai_transform_cancel'); }
  }

  async function search() {
    const text = q.value.replace(/^\s+/, ''), my = ++seq;
    clearAns();
    msg.textContent = ''; delete msg.dataset.page;
    const am = ASK.exec(text);
    if (am) {
      const question = text.slice(am[0].length).trim();
      words = [];
      render(question ? [{ kind: 'ask', id: 'ask:q', title: '', subtitle: '', payload: question }] : [], false);
      if (!question) { const d = document.createElement('div'); d.className = 'tip'; d.textContent = t(L.askTip); list.prepend(d); }
      return;
    } // leading blanks go, a trailing one is kept: "c " already selects the clipboard filter
    words = text[0] === '>' ? [] : text.replace(/^(=|[cnfas]\s)/i, '').toLowerCase().split(/\s+/).filter(Boolean);
    const res = await invoke('cmd_query', { text });
    if (my !== seq || !res) return;
    render(res, !text.trim());
  }

  async function copy(text) {
    try { await navigator.clipboard.writeText(text); return; } catch (_) { /* fall through */ }
    const ta = document.createElement('textarea'); ta.value = text; document.body.append(ta); ta.select();
    try { document.execCommand('copy'); } catch (_) { /* ignore */ }
    ta.remove(); q.focus();
  }

  async function ask(it) {
    if (asking) return;
    if (!askReady()) { invoke('settings_open', { page: 'ai' }); return; }
    const id = ++askId, my = seq;
    asking = id; msg.textContent = '';
    items = []; list.replaceChildren();
    const sp = document.createElement('span'); sp.className = 'spin';
    ans.className = 'think'; ans.hidden = false; ans.replaceChildren(sp, document.createTextNode(t(L.think)));
    const r = await invoke('cmd_ask', { question: it.payload });
    if (asking !== id || my !== seq) return; // the query changed or the bar closed meanwhile
    asking = 0;
    if (r && r.ok) {
      ans.className = ''; ans.textContent = r.text; ans.scrollTop = 0;
      render([{ kind: 'askcopy', id: 'ask:copy', title: '', subtitle: '', payload: r.text }], false);
      return;
    }
    const e = r && r.error;
    ans.hidden = true; ans.replaceChildren();
    if (e === 'cancelled') return render([it], false);
    const go = e === 'net_off' || e === 'no_endpoint' || e === 'no_model';
    msg.textContent = go ? t(e === 'net_off' ? L.askOff : e === 'no_endpoint' ? L.askNoUrl : L.askNoModel) : String(e || t(L.askFail)).slice(0, 300);
    if (go) msg.dataset.page = 'ai';
    render([it], false);
  }

  async function run(it, mode) {
    if (!it) return;
    if (it.kind === 'ask') return ask(it);
    if (it.kind === 'askcopy') {
      await copy(it.payload);
      const lab = list.querySelector('.row .act');
      if (lab) { lab.firstChild.textContent = t(L.copied); clearTimeout(copiedTimer); copiedTimer = setTimeout(() => { if (lab.isConnected) lab.firstChild.textContent = t(L.copy); }, 1800); }
      return;
    }
    if (it.kind === 'calc' || it.kind === 'convert') await copy(it.payload); // copy before Rust hides the window
    if (!['app', 'file', 'folder', 'shelf', 'clip'].includes(it.kind)) mode = 'open'; // reveal / admin / copy-only only mean something for these
    const r = await invoke('cmd_run', { id: it.id, mode });
    if (!r) return;
    const el = list.querySelector(`.row[data-i="${items.indexOf(it)}"]`);
    clearTimeout(confirmTimer);
    if (r.status === 'copy') { // clipboard item, Shift+Enter: copy only
      await copy(r.message);
      invoke('cmd_toggle');
    } else if (r.status === 'updated') { // a setting or action switched: show the new state, stay open
      const s = el && el.querySelector('.s');
      if (s) s.textContent = r.message; else msg.textContent = r.message;
    } else if (r.status === 'confirm' && el) {
      const lab = el.querySelector('.act'), before = lab.firstChild.textContent;
      el.classList.add('confirm');
      lab.firstChild.textContent = t(L.again);
      confirmTimer = setTimeout(() => { el.classList.remove('confirm'); lab.firstChild.textContent = before; }, 3000);
    } else if (r.status === 'error') {
      msg.textContent = r.message;
    }
  }

  const modeOf = (e) => (e.ctrlKey && e.shiftKey ? 'admin' : e.ctrlKey ? 'folder' : e.shiftKey ? 'copy' : 'open');

  q.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(search, 80); });

  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { e.preventDefault(); invoke('cmd_toggle'); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); select(sel + 1); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); select(sel - 1); }
    else if (e.key === 'F4' && !e.altKey) { e.preventDefault(); dictate(); }
    else if (e.key === 'Tab') { e.preventDefault(); nextGroup(e.shiftKey ? -1 : 1); }
    else if (e.key === 'Enter') {
      e.preventDefault();
      run(items[sel], modeOf(e));
    }
  });

  list.addEventListener('click', (e) => {
    const r = e.target.closest('.row');
    if (r) run(items[+r.dataset.i], modeOf(e));
  });
  list.addEventListener('mousemove', (e) => {
    const r = e.target.closest('.row');
    if (r && +r.dataset.i !== sel) select(+r.dataset.i);
  });
  // A click on the transparent margin around the body counts as "outside": close.
  document.addEventListener('mousedown', (e) => { if (!e.target.closest('.body')) invoke('cmd_toggle'); });
  window.addEventListener('focus', () => q.focus());

  // ---- dictation: same pipeline as the Text tab (voice_* commands, voice:state events from voice.rs) ----
  let micState = 'idle';
  function setMic(st) {
    micState = st;
    mic.classList.toggle('rec', st === 'recording'); mic.classList.toggle('busy', st === 'transcribing');
    mic.title = t(st === 'recording' ? L.micStop : st === 'transcribing' ? L.micBusy : L.micTip);
  }
  function micMsg(text, page) { msg.textContent = text; if (page) msg.dataset.page = page; else delete msg.dataset.page; }
  async function dictate() {
    if (micState === 'recording') { invoke('voice_stop'); return; }
    if (micState !== 'idle') return;
    const s = await invoke('voice_state');
    if (!s || s.state !== 'idle') return;
    if (s.enabled === false) { micMsg(t(L.voiceOff), 'voice'); return; }
    micMsg('');
    dictHere = true; // only a take started from this bar is typed into it
    const r = await invoke('voice_start');
    if (r && r.ok === false) { dictHere = false; setMic('idle'); } // the error itself arrives as a voice:state event
  }
  mic.addEventListener('mousedown', (e) => e.preventDefault()); // keep the caret in the input
  mic.addEventListener('click', dictate);
  msg.addEventListener('click', () => { if (msg.dataset.page) invoke('settings_open', { page: msg.dataset.page }); });
  const VERR = {
    off: 'voiceOff', no_key: 'voiceOff', no_endpoint: 'voiceOff',
    mic: { ru: 'Нет доступа к микрофону', en: 'Cannot use the microphone' },
    too_short: { ru: 'Слишком короткая запись: скажи что-нибудь подольше', en: 'Recording too short: say something longer' },
    silent: { ru: 'Микрофон ничего не услышал: проверь микрофон по умолчанию', en: 'The microphone heard nothing: check the default microphone' },
    empty: { ru: 'Речь не распознана', en: 'No speech recognised' },
  };
  T.event.listen('voice:state', (e) => {
    const p = e.payload;
    if (!p || !dictHere) return;
    if (p.state === 'recording' || p.state === 'transcribing') setMic(p.state);
    else if (p.state === 'done') {
      dictHere = false; setMic('idle');
      const text = String(p.text || '').trim();
      if (!text) return;
      const a = q.selectionStart ?? q.value.length, b = q.selectionEnd ?? a, pre = q.value.slice(0, a), sp = pre && !/\s$/.test(pre) ? ' ' : '', end = a + sp.length + text.length;
      q.value = pre + sp + text + q.value.slice(b);
      q.focus(); q.setSelectionRange(end, end);
      q.dispatchEvent(new Event('input'));
    } else if (p.state === 'error') {
      if (p.code === 'busy') return;
      dictHere = false; setMic('idle');
      const v = VERR[p.code];
      if (v === 'voiceOff') micMsg(t(L.voiceOff), 'voice');
      else micMsg(v ? t(v) : `${t(L.voiceFail)}${p.message ? ': ' + String(p.message).slice(0, 160) : ''}`);
    } else if (p.state === 'idle') { dictHere = false; setMic('idle'); } // cancelled (Esc)
  });
  setMic('idle');
  ans.addEventListener('mouseup', () => { if (getSelection().isCollapsed) q.focus(); });

  function reset() {
    q.value = '';
    msg.textContent = ''; delete msg.dataset.page;
    clearAns();
    clearTimeout(timer);
    body.classList.remove('in');
    void body.offsetWidth; // restart the pop animation
    body.classList.add('in');
    fresh = true;
    q.focus();
    loadCfg();
    search();
  }
  T.event.listen('bar:shown', reset);
  T.event.listen('bar:hidden', () => { seq++; clearAns(); dictHere = false; setMic('idle'); clearTimeout(confirmTimer); q.value = ''; list.replaceChildren(); items = []; });
  loadCfg();
})();
