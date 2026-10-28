// Chat tab (S12 / F12, Pro in the original): AI chat against the user's own OpenAI-compatible endpoint. Owner: net/chat agent.
// Rust: chat.rs (chat_send / chat_retry / chat_stop / chat_threads / chat_messages / chat_new / chat_delete / chat_open_link, ai_test / ai_models / ai_set_key / ai_config)
// and net.rs (the only module with sockets). Events from Rust: chat:delta {thread, run, text}, chat:done {thread, run, ok, stopped, error, id}.
// Answers are rendered by the small markdown renderer below: DOM nodes + textContent only, never innerHTML of model text; links open through Rust.
(() => {
  const t = (d) => (window.Pill && Pill.t ? Pill.t(d) : d.en);
  const inv = (c, a) => (window.Pill ? Pill.invoke(c, a) : Promise.resolve(null));
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const svg = (inner, sw = 1.7) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="${sw}" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${inner}</svg>`;
  const IC = {
    sparkles: svg('<path d="M9.5 3.5 11 8.2l4.700 1.500L11 11.200 9.500 16 8 11.200 3.300 9.700 8 8.200z"/><path d="M18 14l.8 2.200 2.200.8-2.200.8L18 20l-.8-2.200-2.200-.8 2.200-.8zM18.500 3.500l.5 1.300 1.300.5-1.300.5-.5 1.300-.5-1.300-1.300-.5 1.300-.5z"/>'),
    attach: svg('<path d="M20.500 11.500 12 20a5.500 5.500 0 0 1-7.800-7.800l8.800-8.800a3.700 3.700 0 0 1 5.200 5.200L9.700 17.300a1.800 1.800 0 0 1-2.600-2.600l8-8"/>'),
    mic: svg('<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.500 11.500a6.500 6.500 0 0 0 13 0M12 18v3"/>'),
    send: svg('<path d="M12 19V5M6 11l6-6 6 6"/>', 2),
    stop: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><rect x="7" y="7" width="10" height="10" rx="2.200"/></svg>',
    compose: svg('<path d="M12 4H6a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-6"/><path d="M18.400 2.600a1.900 1.900 0 0 1 2.700 2.700L12 14.400 8 15.500l1.100-4z"/>'),
    copy: svg('<rect x="9" y="9" width="11" height="11" rx="2.500"/><path d="M5 15V6a2 2 0 0 1 2-2h8"/>'),
    check: svg('<path d="m5 12.500 4.500 4.500L19 7.500"/>', 2),
    retry: svg('<path d="M3.500 12a8.500 8.500 0 1 0 2.800-6.300L3.500 8.500"/><path d="M3.500 3.500v5h5"/>'),
    history: svg('<path d="M3.500 12a8.500 8.500 0 1 0 2.800-6.300L3.500 8.500"/><path d="M3.500 3.500v5h5M12 7.500V12l3 2"/>'),
    gear: svg('<rect x="7" y="7" width="10" height="10" rx="2.500"/><path d="M9.500 3v2.500M14.500 3v2.500M9.500 18.500V21M14.500 18.500V21M3 9.500h2.500M3 14.500h2.500M18.500 9.500H21M18.500 14.500H21"/>'),
    trash: svg('<path d="M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3"/>'),
    x: svg('<path d="M6 6l12 12M18 6 6 18"/>', 2),
  };

  // ---------------------------------------------------------------------------------------------- markdown (safe)
  const INLINE = /`([^`\n]+)`|\*\*([^\n]+?)\*\*|(?<![\w])__([^\n]+?)__(?![\w])|~~([^\n]+?)~~|\*([^\s*][^*\n]*?)\*|(?<![\w])_([^\s_][^_\n]*?)_(?![\w])|\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)|(https?:\/\/[^\s<>]*[^\s<>.,;:!?)'"\]])/g;

  function link(label, url, depth) {
    const a = el('a', 'ch-a');
    a.dataset.url = url;
    a.title = url;
    a.tabIndex = 0;
    a.setAttribute('role', 'link');
    if (label) inline(a, label, depth + 1); else a.textContent = url;
    return a;
  }

  function inline(parent, s, depth = 0) {
    let last = 0;
    for (const m of s.matchAll(INLINE)) {
      if (m.index > last) parent.append(s.slice(last, m.index));
      last = m.index + m[0].length;
      if (m[1] != null) parent.append(el('code', 'ch-ic', m[1]));
      else if (depth > 2) parent.append(m[0]);
      else if (m[2] != null || m[3] != null) { const b = el('strong'); inline(b, m[2] ?? m[3], depth + 1); parent.append(b); }
      else if (m[4] != null) { const d = el('del'); inline(d, m[4], depth + 1); parent.append(d); }
      else if (m[5] != null || m[6] != null) { const i = el('em'); inline(i, m[5] ?? m[6], depth + 1); parent.append(i); }
      else if (m[7] != null) parent.append(link(m[7], m[8], depth));
      else parent.append(link(null, m[9], depth));
    }
    if (last < s.length) parent.append(s.slice(last));
  }

  function para(lines) {
    const p = el('p');
    lines.forEach((l, i) => { if (i) p.append(el('br')); inline(p, l.trim()); });
    return p;
  }

  async function copyText(s) {
    try { await navigator.clipboard.writeText(s); return true; } catch { /* fall through */ }
    try {
      const ta = document.createElement('textarea');
      ta.value = s; ta.style.cssText = 'position:fixed;opacity:0;left:-999px';
      document.body.append(ta); ta.select();
      const ok = document.execCommand('copy');
      ta.remove();
      return ok;
    } catch { return false; }
  }

  function codeBlock(lang, code) {
    const w = el('div', 'ch-cb'), h = el('div', 'ch-cbh');
    h.append(el('span', null, lang || ''));
    const b = el('button', 'ch-cp');
    b.type = 'button';
    b.innerHTML = IC.copy;
    b.append(el('span', null, t({ ru: 'Копировать', en: 'Copy' })));
    b.onclick = async () => {
      if (await copyText(code)) { b.innerHTML = IC.check; b.append(el('span', null, t({ ru: 'Скопировано', en: 'Copied' }))); setTimeout(() => { b.innerHTML = IC.copy; b.append(el('span', null, t({ ru: 'Копировать', en: 'Copy' }))); }, 1400); }
    };
    h.append(b);
    const pre = el('pre'), c = el('code');
    c.textContent = code;
    pre.append(c);
    w.append(h, pre);
    return w;
  }

  const isBlockStart = (l) => /^\s*```/.test(l) || /^#{1,4}\s/.test(l) || /^\s*[-*+]\s+/.test(l) || /^\s*\d+[.)]\s+/.test(l) || /^>\s?/.test(l) || /^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(l);
  const cells = (l) => l.trim().replace(/^\|/, '').replace(/\|$/, '').split('|').map((c) => c.trim());

  /** markdown text -> DocumentFragment (nodes only). Handles an unclosed code fence (streaming). */
  function md(text) {
    const lines = String(text).replace(/\r\n?/g, '\n').split('\n'), out = document.createDocumentFragment();
    let i = 0;
    while (i < lines.length) {
      const l = lines[i];
      let m;
      if ((m = /^\s*```\s*([\w+#.-]*)/.exec(l))) {
        const code = [];
        i++;
        while (i < lines.length && !/^\s*```\s*$/.test(lines[i])) code.push(lines[i++]);
        i++;
        out.append(codeBlock(m[1], code.join('\n')));
      } else if (/^\s*$/.test(l)) i++;
      else if ((m = /^(#{1,4})\s+(.*)$/.exec(l))) { const h = el('div', 'ch-h ch-h' + m[1].length); inline(h, m[2]); out.append(h); i++; }
      else if (/^\s*(-{3,}|\*{3,}|_{3,})\s*$/.test(l)) { out.append(el('hr')); i++; }
      else if (/^\s*[-*+]\s+/.test(l) || /^\s*\d+[.)]\s+/.test(l)) {
        const ordered = /^\s*\d+[.)]\s+/.test(l), ul = el(ordered ? 'ol' : 'ul');
        const rx = ordered ? /^\s*\d+[.)]\s+(.*)$/ : /^\s*[-*+]\s+(.*)$/;
        while (i < lines.length && rx.test(lines[i])) {
          const li = el('li');
          inline(li, rx.exec(lines[i])[1]);
          i++;
          while (i < lines.length && /^\s{2,}\S/.test(lines[i]) && !rx.test(lines[i])) { li.append(el('br')); inline(li, lines[i++].trim()); }
          ul.append(li);
        }
        out.append(ul);
      } else if (/^>\s?/.test(l)) {
        const q = [];
        while (i < lines.length && /^>\s?/.test(lines[i])) q.push(lines[i++].replace(/^>\s?/, ''));
        const bq = el('blockquote');
        bq.append(md(q.join('\n')));
        out.append(bq);
      } else if (l.includes('|') && i + 1 < lines.length && /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(lines[i + 1])) {
        const head = cells(l), tb = el('table'), th = el('tr');
        head.forEach((c) => { const x = el('th'); inline(x, c); th.append(x); });
        tb.append(th);
        i += 2;
        while (i < lines.length && lines[i].includes('|') && lines[i].trim()) {
          const tr = el('tr');
          cells(lines[i++]).forEach((c) => { const x = el('td'); inline(x, c); tr.append(x); });
          tb.append(tr);
        }
        const wrap = el('div', 'ch-tbl');
        wrap.append(tb);
        out.append(wrap);
      } else {
        const p = [];
        while (i < lines.length && lines[i].trim() && (p.length === 0 || !isBlockStart(lines[i]))) p.push(lines[i++]);
        out.append(para(p));
      }
    }
    return out;
  }

  // ---------------------------------------------------------------------------------------------- state
  const st = { cfg: null, thread: null, msgs: [], run: null, sending: false, acc: '', shown: 0, last: 0, early: [], aiBody: null, files: [], loaded: false, raf: 0, models: [] };
  let root, scroll, msgsEl, emptyEl, offEl, errEl, filesEl, ta, sendBtn, composeBtn, setPanel, histPanel, subEl, setUi = {};

  const model = () => (st.cfg && st.cfg.model) || '';
  const ready = () => !!(st.cfg && st.cfg.on && st.cfg.baseUrl && st.cfg.model);
  const streaming = () => st.run != null || st.sending;

  async function loadCfg() { const c = await inv('ai_config'); if (c) st.cfg = c; return st.cfg; }

  // ---------------------------------------------------------------------------------------------- rendering
  function nearBottom() { return scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight < 70; }
  function toBottom() { scroll.scrollTop = scroll.scrollHeight; }

  function msgEl(m, last) {
    if (m.role === 'user') {
      const w = el('div', 'ch-m user'), b = el('div', 'ch-bub', m.content);
      w.append(b);
      return w;
    }
    const w = el('div', 'ch-m ai'), av = el('div', 'ch-av'), body = el('div', 'ch-md');
    av.innerHTML = IC.sparkles;
    body.append(md(m.content));
    const col = el('div', 'ch-col');
    col.append(body);
    if (last) col.append(actions(m));
    w.append(av, col);
    return w;
  }

  function actions(m) {
    const a = el('div', 'ch-act');
    const cp = el('button', 'icon-btn');
    cp.type = 'button'; cp.title = t({ ru: 'Копировать', en: 'Copy' }); cp.innerHTML = IC.copy;
    cp.onclick = async () => { if (await copyText(m.content)) { cp.innerHTML = IC.check; setTimeout(() => { cp.innerHTML = IC.copy; }, 1200); } };
    const rt = el('button', 'icon-btn');
    rt.type = 'button'; rt.title = t({ ru: 'Ответить ещё раз', en: 'Answer again' }); rt.innerHTML = IC.retry;
    rt.onclick = () => retry();
    a.append(cp, rt);
    return a;
  }

  function renderMsgs() {
    const empty = !st.msgs.length && !streaming();
    emptyEl.hidden = !empty;
    msgsEl.hidden = empty;
    const lastAi = (() => { for (let i = st.msgs.length - 1; i >= 0; i--) if (st.msgs[i].role === 'assistant') return i; return -1; })();
    msgsEl.replaceChildren(...st.msgs.map((m, i) => msgEl(m, !streaming() && i === lastAi && i === st.msgs.length - 1)));
    toBottom();
  }

  function renderEmpty() {
    emptyEl.replaceChildren();
    const ic = el('div', 'ch-spark'); ic.innerHTML = IC.sparkles;
    emptyEl.append(ic, el('div', 'ch-et', t({ ru: 'Спроси что угодно', en: 'Ask me anything' })));
    subEl = el('div', 'ch-es');
    emptyEl.append(subEl);
    const ideas = [
      t({ ru: 'Объясни простыми словами, как работает VPN', en: 'Explain in simple words how a VPN works' }),
      t({ ru: 'Помоги написать вежливое письмо о переносе встречи', en: 'Help me write a polite letter about rescheduling a meeting' }),
      t({ ru: 'Придумай пять идей, чем заняться в выходные', en: 'Come up with five ideas for the weekend' }),
    ];
    const chips = el('div', 'ch-chips');
    ideas.forEach((s) => { const b = el('button', 'ch-chip', s); b.type = 'button'; b.onclick = () => send(s); chips.append(b); });
    emptyEl.append(chips);
    updateSub();
  }
  function updateSub() {
    if (!subEl) return;
    subEl.textContent = (model() ? model() + ' ' : '') + t({ ru: 'помнит весь разговор · Shift+Enter — новая строка', en: 'remembers the whole conversation · Shift+Enter new line' });
    if (!model()) subEl.textContent = t({ ru: 'Выберите модель в настройках ИИ (значок справа вверху) · Shift+Enter — новая строка', en: 'Choose a model in the AI settings (top right) · Shift+Enter new line' });
  }

  const PM = window.PillMotion;
  const popOut = (p) => { if (!p.hidden) PM.exit(p, { y: -6, scale: 0.97, hide: true }); }; // popover/panel close: fade+shrink, then [hidden]
  function showErr(msg, withRetry) {
    if (!msg) { if (!errEl.hidden) PM.collapse(errEl, { hide: true, after: (e) => e.replaceChildren() }); return; }
    const was = errEl.hidden;
    PM.cancel(errEl);
    errEl.replaceChildren();
    errEl.hidden = false;
    if (was) setTimeout(() => PM.expand(errEl), 0); // after the content below is appended
    errEl.append(el('span', null, msg));
    if (withRetry) { const b = el('button', 'btn-ghost', t({ ru: 'Повторить', en: 'Retry' })); b.type = 'button'; b.onclick = () => retry(); errEl.append(b); }
    const s = el('button', 'btn-ghost', t({ ru: 'Настройки ИИ', en: 'AI settings' })); s.type = 'button'; s.onclick = () => openSettings(true); errEl.append(s);
    const x = el('button', 'icon-btn'); x.type = 'button'; x.innerHTML = IC.x; x.title = t({ ru: 'Скрыть', en: 'Dismiss' }); x.onclick = () => showErr(''); errEl.append(x);
  }

  function setSendIcon() {
    const busy = streaming();
    sendBtn.innerHTML = busy ? IC.stop : IC.send;
    sendBtn.classList.toggle('stop', busy);
    sendBtn.title = busy ? t({ ru: 'Остановить', en: 'Stop' }) : t({ ru: 'Отправить', en: 'Send' });
  }

  function applyMode() {
    const off = !(st.cfg && st.cfg.on);
    offEl.hidden = !off;
    root.classList.toggle('is-off', off);
    updateSub();
    if (!off && !ready() && !st.opened) { st.opened = true; openSettings(true); }
  }

  // ---------------------------------------------------------------------------------------------- actions
  async function loadThread(id) {
    if (streaming()) return;
    st.thread = id;
    const m = id ? await inv('chat_messages', { thread: id }) : [];
    st.msgs = m || [];
    showErr('');
    renderMsgs();
  }

  async function newChat() {
    if (streaming()) return;
    st.thread = null; st.msgs = [];
    showErr('');
    renderMsgs();
    closeHist();
  }

  const reduced = () => document.documentElement.classList.contains('rm') || (root && !!root.closest('.rm')) || matchMedia('(prefers-reduced-motion: reduce)').matches;

  /** new bubble enter: opacity + translateY 6px -> 0, 180 ms ease-out-expo (WAAPI, no CSS file). */
  function enter(node) {
    if (reduced() || !node.animate) return;
    try { node.animate([{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], { duration: 180, easing: 'cubic-bezier(.16,1,.3,1)' }); } catch { /* ignore */ }
  }

  function startAi() {
    st.acc = ''; st.shown = 0; st.last = 0; st.aiBody = el('div', 'ch-md typing');
    const w = el('div', 'ch-m ai'), av = el('div', 'ch-av'), col = el('div', 'ch-col');
    av.innerHTML = IC.sparkles;
    st.aiBody.append(el('i'), el('i'), el('i'));
    col.append(st.aiBody);
    w.append(av, col);
    msgsEl.append(w);
    enter(w);
    toBottom();
  }

  // Drip: network chunks go to the backlog (st.acc); every frame reveals ~1/6 of what is left (time-scaled, so
  // 60 and 144 Hz feel the same and the lag stays ~100-250 ms). The markdown is parsed once per frame at most.
  function paintAi(now) {
    st.raf = 0;
    if (!st.aiBody) return;
    const dt = st.last ? Math.min(100, Math.max(1, now - st.last)) : 16.7;
    st.last = now;
    const backlog = st.acc.length - st.shown;
    if (backlog > 0) {
      let n = reduced() ? backlog : Math.max(1, Math.ceil(backlog * (1 - Math.pow(5 / 6, dt / 16.7))));
      let end = st.shown + n;
      const c = st.acc.charCodeAt(end - 1);
      if (end < st.acc.length && c >= 0xd800 && c <= 0xdbff) end++; // never split a surrogate pair
      st.shown = end;
      const stick = nearBottom();
      st.aiBody.classList.remove('typing');
      st.aiBody.replaceChildren(md(st.acc.slice(0, st.shown)));
      if (stick) toBottom();
    }
    if (st.shown < st.acc.length) st.raf = requestAnimationFrame(paintAi); else st.last = 0;
  }

  async function send(textIn) {
    if (streaming()) return;
    let text = (textIn != null ? textIn : ta.value).trim();
    if (!text && !st.files.length) return;
    if (!ready()) { openSettings(true); showErr(!(st.cfg && st.cfg.on) ? t({ ru: 'ИИ выключен.', en: 'AI is switched off.' }) : t({ ru: 'Укажите адрес API и модель.', en: 'Set the API address and the model.' })); return; }
    for (const f of st.files) text += `\n\n[${f.name}]\n\`\`\`\n${f.text}\n\`\`\``;
    showErr('');
    st.sending = true; st.early = [];
    setSendIcon();
    emptyEl.hidden = true; msgsEl.hidden = false;
    const optimistic = { id: -1, role: 'user', content: text };
    const ub = msgEl(optimistic);
    msgsEl.append(ub);
    enter(ub);
    startAi();
    const r = await inv('chat_send', { thread: st.thread, text, model: null });
    if (!r || !r.ok) {
      st.sending = false; st.aiBody = null;
      if (r && r.user) st.thread = r.thread;
      await loadThread(st.thread);
      showErr((r && r.error) || t({ ru: 'Не удалось отправить.', en: 'Could not send.' }), false);
      setSendIcon();
      return;
    }
    if (textIn == null) { ta.value = ''; autosize(); }
    st.files = []; renderFiles();
    st.thread = r.thread; st.run = r.run; st.sending = false;
    setSendIcon();
    const early = st.early; st.early = [];
    early.forEach(([k, p]) => (k === 'd' ? onDelta(p) : onDone(p)));
  }

  async function retry() {
    if (streaming() || !st.thread) return;
    if (!ready()) { openSettings(true); return; }
    showErr('');
    st.sending = true; st.early = [];
    // drop the last answer from the view (it fades out first), then stream a new one
    const lastM = msgsEl.lastElementChild;
    if (lastM && lastM.classList.contains('ai')) await PM.exit(lastM, { y: -6 });
    while (st.msgs.length && st.msgs[st.msgs.length - 1].role === 'assistant') st.msgs.pop();
    renderMsgs();
    msgsEl.hidden = false; emptyEl.hidden = true;
    startAi();
    setSendIcon();
    const r = await inv('chat_retry', { thread: st.thread, model: null });
    if (!r || !r.ok) {
      st.sending = false; st.aiBody = null;
      await loadThread(st.thread);
      showErr((r && r.error) || t({ ru: 'Не удалось повторить.', en: 'Could not retry.' }), false);
      setSendIcon();
      return;
    }
    st.run = r.run; st.sending = false;
    setSendIcon();
    const early = st.early; st.early = [];
    early.forEach(([k, p]) => (k === 'd' ? onDelta(p) : onDone(p)));
  }

  function stop() { if (st.thread != null) inv('chat_stop', { thread: st.thread }); }

  function onDelta(p) {
    if (st.sending && st.run == null) { st.early.push(['d', p]); return; }
    if (!p || p.run !== st.run || !st.aiBody) return;
    st.acc += p.text || '';
    if (!st.raf) st.raf = requestAnimationFrame(paintAi);
  }

  async function onDone(p) {
    if (st.sending && st.run == null) { st.early.push(['x', p]); return; }
    if (!p || p.run !== st.run) return;
    // flush instantly: drop the drip, loadThread below paints the full saved answer
    if (st.raf) { cancelAnimationFrame(st.raf); st.raf = 0; }
    st.acc = ''; st.shown = 0; st.last = 0;
    st.run = null; st.aiBody = null;
    const err = p.ok ? '' : (p.error || t({ ru: 'Ошибка', en: 'Error' }));
    await loadThread(st.thread);
    setSendIcon();
    if (err) showErr(err, true);
  }

  // ---------------------------------------------------------------------------------------------- composer
  function autosize() { ta.style.height = 'auto'; ta.style.height = Math.min(ta.scrollHeight, 96) + 'px'; }

  function renderFiles() {
    if (!st.files.length) { if (!filesEl.hidden) PM.collapse(filesEl, { hide: true, after: (e) => e.replaceChildren() }); return; }
    const was = filesEl.hidden;
    PM.cancel(filesEl);
    filesEl.hidden = false;
    filesEl.replaceChildren(...st.files.map((f, i) => {
      const c = el('span', 'ch-file');
      c.append(el('span', null, f.name));
      const x = el('button', 'icon-btn'); x.type = 'button'; x.innerHTML = IC.x; x.title = t({ ru: 'Убрать файл', en: 'Remove the file' }); x.onclick = () => { st.files.splice(i, 1); renderFiles(); };
      c.append(x);
      return c;
    }));
    if (was) PM.expand(filesEl);
  }

  function pickFiles() {
    const inp = document.createElement('input');
    inp.type = 'file'; inp.multiple = true;
    inp.accept = 'text/*,.md,.json,.csv,.log,.js,.ts,.py,.rs,.go,.java,.c,.cpp,.h,.cs,.html,.css,.xml,.yml,.yaml,.toml,.ini,.sql,.sh,.ps1,.bat';
    inp.onchange = async () => {
      for (const f of [...inp.files].slice(0, 5)) {
        if (f.size > 200 * 1024) { showErr(t({ ru: 'Файл больше 200 КБ: ', en: 'File is over 200 KB: ' }) + f.name); continue; }
        const text = await f.text();
        if (text.includes('\u0000')) { showErr(t({ ru: 'Только текстовые файлы: ', en: 'Text files only: ' }) + f.name); continue; }
        st.files.push({ name: f.name, text });
      }
      renderFiles();
    };
    inp.click();
  }

  // ---------------------------------------------------------------------------------------------- popovers: history + AI settings
  function closeHist() { popOut(histPanel); }
  async function openHist() {
    if (!histPanel.hidden) return closeHist();
    popOut(setPanel);
    const list = (await inv('chat_threads')) || [];
    histPanel.replaceChildren();
    const nb = el('button', 'btn ch-new'); nb.type = 'button'; nb.innerHTML = IC.compose; nb.append(el('span', null, t({ ru: 'Новый чат', en: 'New chat' }))); nb.onclick = newChat;
    histPanel.append(nb);
    if (!list.length) histPanel.append(el('div', 'ch-hn dim', t({ ru: 'Пока нет чатов', en: 'No chats yet' })));
    list.forEach((th) => {
      const r = el('div', 'ch-hr' + (th.id === st.thread ? ' sel' : ''));
      const b = el('button', 'ch-ht'); b.type = 'button';
      b.append(el('span', 'ch-htt', th.title || t({ ru: 'Без названия', en: 'Untitled' })), el('span', 'dim', new Date(th.updated_at).toLocaleDateString()));
      b.onclick = async () => { await loadThread(th.id); closeHist(); };
      const d = el('button', 'icon-btn'); d.type = 'button'; d.title = t({ ru: 'Удалить', en: 'Delete' }); d.innerHTML = IC.trash;
      d.onclick = async () => { // the row folds away, the panel stays open
        const gone = PM.collapse(r, { remove: true });
        await inv('chat_delete', { thread: th.id });
        if (th.id === st.thread) { st.thread = null; st.msgs = []; renderMsgs(); }
        await gone;
        if (!histPanel.querySelector('.ch-hr') && !histPanel.querySelector('.ch-hn')) histPanel.append(el('div', 'ch-hn dim', t({ ru: 'Пока нет чатов', en: 'No chats yet' })));
      };
      r.append(b, d);
      histPanel.append(r);
    });
    PM.cancel(histPanel);
    histPanel.hidden = false;
  }

  const PRESETS = { ollama: 'http://127.0.0.1:11434/v1', freellm: 'http://127.0.0.1:3001/v1' };

  async function saveCfg(patch) {
    const c = await inv('settings_set', { patch });
    await loadCfg();
    applyMode();
    return c;
  }

  async function openSettings(show) {
    if (show === true || setPanel.hidden) {
      closeHist();
      await loadCfg();
      buildSettings();
      PM.cancel(setPanel);
      setPanel.hidden = false;
      inv('net_features_status').then((s) => { const f = s && s.features && s.features.find((x) => x.id === 'ai'); if (f && setUi.hosts) setUi.hosts.textContent = t({ ru: 'Запросы уходят только на: ', en: 'Requests go only to: ' }) + (f.hosts.join(', ') || '—') + ' · ' + t({ ru: 'вызовов: ', en: 'calls: ' }) + f.calls; });
    } else popOut(setPanel);
  }

  function buildSettings() {
    const c = st.cfg || {};
    setPanel.replaceChildren();
    const head = el('div', 'ch-sh');
    head.append(el('b', null, t({ ru: 'Настройки ИИ', en: 'AI settings' })));
    const cl = el('button', 'icon-btn'); cl.type = 'button'; cl.innerHTML = IC.x; cl.title = t({ ru: 'Закрыть', en: 'Close' }); cl.onclick = () => popOut(setPanel); head.append(cl);

    const seg = el('div', 'seg');
    const dots = {}; // preset id -> status dot, painted by scan()
    const mk = (id, label) => { const b = el('button', c.preset === id ? 'on' : '', label); b.type = 'button'; if (id !== 'custom') { dots[id] = el('i', 'ch-dot'); b.prepend(dots[id]); } b.onclick = async () => { const p = { 'ai.preset': id }; if (PRESETS[id]) p['ai.baseUrl'] = PRESETS[id]; await saveCfg(p); buildSettings(); }; seg.append(b); };
    mk('ollama', t({ ru: 'Ollama (локально)', en: 'Ollama (local)' }));
    mk('freellm', t({ ru: 'FreeLLMAPI (локально)', en: 'FreeLLMAPI (local)' }));
    mk('custom', t({ ru: 'Другой', en: 'Custom' }));
    const scanMsg = el('div', 'ch-sm dim');

    const row = (label, ...kids) => { const r = el('div', 'ch-sr'); r.append(el('label', null, label), ...kids); return r; };
    const url = el('input', 'field'); url.value = c.baseUrl || ''; url.placeholder = 'http://127.0.0.1:11434/v1'; url.spellcheck = false;
    const urlMsg = el('div', 'ch-sm bad'); urlMsg.textContent = c.urlError || '';
    url.onchange = async () => { await saveCfg({ 'ai.baseUrl': url.value.trim(), 'ai.preset': c.preset === 'custom' || !PRESETS[c.preset] || PRESETS[c.preset] !== url.value.trim() ? 'custom' : c.preset }); buildSettings(); };
    const mdl = el('input', 'field'); mdl.value = c.model || ''; mdl.placeholder = t({ ru: 'имя модели', en: 'model name' }); mdl.spellcheck = false;
    mdl.onchange = () => saveCfg({ 'ai.model': mdl.value.trim() }).then(updateSub);
    // once the server's models are known the name is picked from a list (mdl keeps the value, the select mirrors it)
    const sel = el('select', 'field'); sel.hidden = true;
    sel.onchange = () => { mdl.value = sel.value; mdl.onchange(); };
    const showModels = (list) => {
      st.models = list;
      sel.hidden = !list.length; mdl.hidden = !!list.length;
      if (!list.length) return;
      const cur = mdl.value.trim();
      sel.replaceChildren(...(cur && !list.includes(cur) ? [cur, ...list] : list).map((m) => Object.assign(el('option', null, m), { value: m })));
      sel.value = cur || list[0];
    };
    showModels(st.models);
    const norm = (u) => String(u || '').trim().replace(/\/+$/, '');
    const fetchB = el('button', 'btn', t({ ru: 'Найти', en: 'Scan' })); fetchB.type = 'button'; fetchB.title = t({ ru: 'Найти локальные серверы и их модели', en: 'Find local servers and their models' });
    // probes the usual local ports, paints the status dots and fills the model list of the selected server
    const scan = async (manual) => {
      fetchB.disabled = true; scanMsg.className = 'ch-sm dim'; scanMsg.textContent = t({ ru: 'Ищу локальные серверы…', en: 'Looking for local servers…' });
      const res = await inv('ai_probe');
      fetchB.disabled = false;
      if (!setPanel.contains(scanMsg)) return; // the panel was rebuilt meanwhile
      const found = Array.isArray(res) ? res : [];
      found.forEach((r) => { const d = dots[r.id]; if (d) { d.className = 'ch-dot' + (r.up ? ' up' : ''); d.title = r.up ? t({ ru: 'запущен', en: 'running' }) : t({ ru: 'не найден', en: 'not found' }); } });
      const here = found.find((r) => norm(r.url) === norm(url.value));
      const others = found.filter((r) => r.up && r !== here).map((r) => r.name);
      if (here && here.up) {
        showModels(here.models);
        if (!here.models.length) { scanMsg.className = 'ch-sm bad'; scanMsg.textContent = here.name + t({ ru: ' запущен, но моделей нет. Скачайте любую, например: ollama pull qwen3', en: ' is running but has no models. Pull one, e.g.: ollama pull qwen3' }); return; }
        const cur = mdl.value.trim();
        if (!cur || !here.models.includes(cur)) { mdl.value = here.models[0]; mdl.onchange(); showModels(here.models); }
        scanMsg.className = 'ch-sm ok'; scanMsg.textContent = here.name + t({ ru: ' найден · моделей: ', en: ' found · models: ' }) + here.models.length;
      } else if (here) {
        scanMsg.className = 'ch-sm bad';
        scanMsg.textContent = here.name + t({ ru: ' не отвечает на ', en: ' is not answering on ' }) + norm(here.url).replace('http://', '').replace('/v1', '') + '.' + (others.length ? t({ ru: ' Запущено: ', en: ' Running: ' }) + others.join(', ') + t({ ru: ' — выберите его выше.', en: ' — pick it above.' }) : t({ ru: ' Запустите его и нажмите «Найти».', en: ' Start it and press Scan.' }));
      } else if (manual) { // a custom address: ask that very server
        await saveCfg({ 'ai.baseUrl': url.value.trim() });
        const r = await inv('ai_models');
        if (r && r.ok) { showModels(r.models); scanMsg.className = 'ch-sm' + (r.models.length ? ' ok' : ''); scanMsg.textContent = r.models.length ? t({ ru: 'Моделей: ', en: 'Models: ' }) + r.models.length : t({ ru: 'Сервер не вернул моделей', en: 'The server returned no models' }); }
        else { scanMsg.className = 'ch-sm bad'; scanMsg.textContent = (r && r.error) || 'error'; }
      } else { scanMsg.textContent = others.length ? t({ ru: 'Запущено локально: ', en: 'Running locally: ' }) + others.join(', ') : ''; }
    };
    fetchB.onclick = () => scan(true);
    const mrow = el('div', 'ch-sr2'); mrow.append(mdl, sel, fetchB);

    const key = el('input', 'field'); key.type = 'password'; key.autocomplete = 'off'; key.spellcheck = false;
    key.placeholder = c.hasKey ? t({ ru: '•••••••• (ключ сохранён)', en: '•••••••• (key saved)' }) : t({ ru: 'не нужен для локальных серверов', en: 'not needed for local servers' });
    const save = el('button', 'btn', t({ ru: 'Сохранить', en: 'Save' })); save.type = 'button';
    const rm = el('button', 'btn-ghost bad', t({ ru: 'Удалить', en: 'Remove' })); rm.type = 'button'; rm.hidden = !c.hasKey;
    const keyMsg = el('div', 'ch-sm');
    save.onclick = async () => { const v = key.value.trim(); if (!v) return; const r = await inv('ai_set_key', { key: v }); key.value = ''; await loadCfg(); keyMsg.textContent = r && r.ok ? t({ ru: 'Ключ сохранён: зашифрован Windows, хранится только здесь.', en: 'Key saved: encrypted by Windows, stored only on this computer.' }) : (r && r.error) || 'error'; keyMsg.className = 'ch-sm' + (r && r.ok ? '' : ' bad'); rm.hidden = !(st.cfg && st.cfg.hasKey); key.placeholder = t({ ru: '•••••••• (ключ сохранён)', en: '•••••••• (key saved)' }); };
    rm.onclick = async () => { await inv('ai_set_key', { key: '' }); await loadCfg(); buildSettings(); };
    const krow = el('div', 'ch-sr2'); krow.append(key, save, rm);

    const test = el('button', 'btn', t({ ru: 'Проверить', en: 'Test' })); test.type = 'button';
    const tmsg = el('span', 'ch-sm');
    test.onclick = async () => {
      await saveCfg({ 'ai.baseUrl': url.value.trim(), 'ai.model': mdl.value.trim() });
      test.disabled = true; tmsg.className = 'ch-sm'; tmsg.textContent = t({ ru: 'Проверяю…', en: 'Testing…' });
      const r = await inv('ai_test');
      test.disabled = false;
      if (r && r.ok) { tmsg.className = 'ch-sm ok'; tmsg.textContent = t({ ru: 'Работает', en: 'Works' }) + ' · ' + r.ms + ' ' + t({ ru: 'мс', en: 'ms' }); }
      else { tmsg.className = 'ch-sm bad'; tmsg.textContent = (r && r.error) || t({ ru: 'Нет ответа', en: 'No answer' }); }
      applyMode();
    };
    const off = el('button', 'btn-ghost', t({ ru: 'Выключить ИИ', en: 'Turn AI off' })); off.type = 'button';
    off.onclick = async () => { await saveCfg({ 'net.ai': false }); popOut(setPanel); };
    const trow = el('div', 'ch-sr2 ch-tr'); trow.append(test, tmsg, el('span', 'ch-fill'), off);
    setUi.hosts = el('div', 'ch-sm dim');
    setPanel.append(head, seg, scanMsg, row(t({ ru: 'Адрес API (OpenAI-совместимый)', en: 'API address (OpenAI-compatible)' }), url, urlMsg),
      row(t({ ru: 'Модель', en: 'Model' }), mrow), row(t({ ru: 'Ключ API', en: 'API key' }), krow, keyMsg), trow, setUi.hosts);
    setUi.hosts.title = t({ ru: 'История чатов хранится только на этом компьютере. http разрешён только для 127.0.0.1 / localhost.', en: 'Chat history stays on this computer. Plain http is allowed only for 127.0.0.1 / localhost.' });
    scan(false);
  }

  // ---------------------------------------------------------------------------------------------- mount
  function mount(r) {
    root = r;
    root.classList.add('tab-chat');
    scroll = el('div', 'ch-scroll');
    emptyEl = el('div', 'ch-empty');
    msgsEl = el('div', 'ch-msgs'); msgsEl.hidden = true;
    scroll.append(emptyEl, msgsEl);

    const bottom = el('div', 'ch-bottom');
    errEl = el('div', 'ch-err'); errEl.hidden = true;
    filesEl = el('div', 'ch-files'); filesEl.hidden = true;
    const row = el('div', 'ch-row');
    composeBtn = el('button', 'icon-btn ch-compose'); composeBtn.type = 'button'; composeBtn.innerHTML = IC.compose; composeBtn.title = t({ ru: 'Новый чат', en: 'New chat' }); composeBtn.onclick = newChat;
    const box = el('div', 'ch-box');
    const att = el('button', 'icon-btn'); att.type = 'button'; att.innerHTML = IC.attach; att.title = t({ ru: 'Прикрепить текстовый файл', en: 'Attach a text file' }); att.onclick = pickFiles;
    ta = el('textarea'); ta.rows = 1; ta.spellcheck = false; ta.placeholder = t({ ru: 'Сообщение для ИИ…', en: 'Message the AI…' });
    ta.oninput = autosize;
    ta.onkeydown = (e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); send(); } };
    const mic = el('button', 'icon-btn'); mic.type = 'button'; mic.innerHTML = IC.mic; mic.title = t({ ru: 'Голосовой ввод: нажми, скажи, нажми ещё раз', en: 'Voice input: press, speak, press again' });
    let micWait = false; // text of a dictation started here also goes into the input (voice-live.js dispatches voice:state / voice:done on document)
    mic.onclick = () => { micWait = true; if (window.PillVoice) window.PillVoice.toggle(); };
    document.addEventListener('voice:state', (e) => { const s = e.detail && e.detail.state; mic.style.color = s === 'recording' ? '#ff5a4e' : ''; if (s === 'error' || s === 'idle') micWait = false; });
    document.addEventListener('voice:done', (e) => { if (!micWait) return; micWait = false; const x = e.detail && e.detail.text; if (x) { ta.value = (ta.value && !/\s$/.test(ta.value) ? ta.value + ' ' : ta.value) + x; autosize(); ta.focus(); } });
    sendBtn = el('button', 'ch-send'); sendBtn.type = 'button';
    sendBtn.onclick = () => (streaming() ? stop() : send());
    box.append(att, ta, mic, sendBtn);
    row.append(composeBtn, box);
    bottom.append(errEl, filesEl, row);

    const tools = el('div', 'ch-tools');
    const hb = el('button', 'icon-btn'); hb.type = 'button'; hb.innerHTML = IC.history; hb.title = t({ ru: 'История чатов', en: 'Chat history' }); hb.onclick = openHist;
    const gb = el('button', 'icon-btn'); gb.type = 'button'; gb.innerHTML = IC.gear; gb.title = t({ ru: 'Настройки ИИ', en: 'AI settings' }); gb.onclick = () => openSettings();
    tools.append(hb, gb);
    histPanel = el('div', 'ch-pop ch-hist'); histPanel.hidden = true;
    setPanel = el('div', 'ch-pop ch-set'); setPanel.hidden = true;

    offEl = el('div', 'ch-off'); offEl.hidden = true;
    const badge = el('div', 'ch-spark'); badge.innerHTML = IC.sparkles;
    const en = el('button', 'btn-primary', t({ ru: 'Включить', en: 'Enable' })); en.type = 'button';
    en.onclick = async () => { await saveCfg({ 'net.ai': true }); if (!ready()) openSettings(true); };
    offEl.append(badge, el('div', 'ch-et', t({ ru: 'ИИ-чат выключен', en: 'AI chat is off' })),
      el('div', 'ch-eb', t({ ru: 'Чат работает с вашим OpenAI-совместимым сервером: локальная Ollama, FreeLLMAPI или любой другой. Пока вы не включите, ничего не отправляется. История хранится только на этом компьютере.', en: 'The chat talks to your own OpenAI-compatible server: local Ollama, FreeLLMAPI or any other. Nothing is sent until you turn it on. History stays on this computer.' })), en);

    root.append(scroll, bottom, tools, histPanel, setPanel, offEl);
    root.addEventListener('click', (e) => {
      const a = e.target.closest && e.target.closest('a.ch-a');
      if (a) { e.preventDefault(); inv('chat_open_link', { url: a.dataset.url }); return; }
      if (!histPanel.hidden && !histPanel.contains(e.target) && !tools.contains(e.target)) closeHist();
    });
    root.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target.classList && e.target.classList.contains('ch-a')) { e.preventDefault(); inv('chat_open_link', { url: e.target.dataset.url }); } });
    renderEmpty();
    setSendIcon();
    const on = (n, f) => { if (window.Pill && Pill.on) Pill.on(n, f); else if (window.__TAURI__ && window.__TAURI__.event) window.__TAURI__.event.listen(n, (e) => f(e.payload)); };
    on('chat:delta', onDelta);
    on('chat:done', onDone);
    on('settings:changed', () => { loadCfg().then(() => { applyMode(); }); });
  }

  Pill.register({
    id: 'chat', order: 40, height: 417, // content height the live app uses for this tab (Home is 338): the shell may grow the body for it
    title: t({ ru: 'Чат', en: 'Chat' }),
    icon: window.pillIcon ? window.pillIcon('chat') : svg('<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22z"/>'),
    mount,
    async onShow() {
      await loadCfg();
      applyMode();
      if (!st.loaded) {
        st.loaded = true;
        if (st.cfg && st.cfg.on) {
          const th = await inv('chat_threads');
          if (th && th.length) await loadThread(th[0].id);
        }
        renderMsgs();
      }
    },
  });
})();
