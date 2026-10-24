// AI quick actions for a clipboard item: a panel over the Clipboard tab with Translate / Shorten / Fix / Explain.
// Opened by the sparkle button of a row (tab-clipboard.js -> PillClipAI.open). Backend: aiactions.rs (ai_clip_text, ai_transform,
// ai_transform_cancel) and clipboard.rs (clip_paste_text). The original item is never changed; nothing is stored here.
(() => {
  const P = window.Pill, t = P.t, ic = P.icon, PM = window.PillMotion;
  const inv = (cmd, args) => P.invoke(cmd, args);
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  const ACTIONS = [
    { id: 'translate', icon: 'globe', label: { ru: 'Перевести', en: 'Translate' }, sub: { ru: 'на английский или русский', en: 'to English or Russian' } },
    { id: 'shorten', icon: 'convert', label: { ru: 'Сократить', en: 'Shorten' }, sub: { ru: 'примерно вдвое', en: 'to about half' } },
    { id: 'fix', icon: 'edit', label: { ru: 'Исправить ошибки', en: 'Fix spelling and grammar' }, sub: { ru: 'орфография и грамматика', en: 'spelling and grammar' } },
    { id: 'explain', icon: 'sparkles', label: { ru: 'Объяснить просто', en: 'Explain simply' }, sub: { ru: 'простыми словами', en: 'in plain words' } },
  ];

  let panel = null, body = null, titleEl = null, job = 0, running = false, ctx = null;

  const isOpen = () => !!panel;

  function close() {
    if (!panel) return;
    if (running) inv('ai_transform_cancel');
    job++; running = false;
    const p = panel; panel = null; body = null;
    PM.exit(p, { y: 4, remove: true });
    if (ctx && ctx.onClose) ctx.onClose();
    ctx = null;
  }

  function open(o) {
    if (panel) { panel.remove(); panel = null; }
    ctx = o; job++; running = false;
    const a = document.activeElement;
    if (a && a.matches && a.matches('input, textarea')) a.blur(); // hands keyboard focus back to the app below, like a paste does
    panel = document.createElement('div');
    panel.className = 'clip-ai';
    panel.setAttribute('role', 'dialog');
    panel.innerHTML = `<div class="clip-ai-head"><span class="clip-ai-ico">${ic('sparkles', { size: 15, sw: 1.7 })}</span><span class="clip-ai-title"></span>
      <button class="clip-act clip-ai-x" type="button" data-ai="close" title="${esc(t({ ru: 'Закрыть', en: 'Close' }))}">${ic('x', { size: 14, sw: 1.9 })}</button></div><div class="clip-ai-body"></div>`;
    titleEl = panel.querySelector('.clip-ai-title');
    body = panel.querySelector('.clip-ai-body');
    panel.addEventListener('click', onClick);
    o.root.appendChild(panel);
    PM.enter(panel, { y: 6 });
    menu();
  }

  function setBody(title, html) {
    titleEl.textContent = title;
    body.innerHTML = html;
    PM.enter(body, { y: 4, dur: 0.25 });
  }

  function menu() {
    running = false;
    setBody(t({ ru: 'ИИ-действия', en: 'AI actions' }), `<div class="clip-ai-menu">${ACTIONS.map((a) => `
      <button class="clip-ai-opt" type="button" data-ai="run" data-a="${a.id}"><span class="clip-ai-oi">${ic(a.icon, { size: 16, sw: 1.7 })}</span>
        <span class="clip-ai-ot"><span class="t">${esc(t(a.label))}</span><span class="s">${esc(t(a.sub))}</span></span></button>`).join('')}</div>`);
    const first = body.querySelector('.clip-ai-opt');
    if (first) first.focus({ preventScroll: true });
  }

  async function run(id) {
    const a = ACTIONS.find((x) => x.id === id);
    if (!a) return;
    const my = ++job;
    running = true;
    setBody(t(a.label), `<div class="clip-ai-wait"><span class="clip-ai-spin"></span><span>${esc(t({ ru: 'Думаю…', en: 'Working…' }))}</span>
      <button class="btn-ghost clip-ai-btn" type="button" data-ai="cancel">${esc(t({ ru: 'Отмена', en: 'Cancel' }))}</button></div>`);
    const text = await inv('ai_clip_text', { id: ctx.id });
    if (my !== job) return;
    if (!text || !String(text).trim()) return fail(a, 'empty');
    const r = await inv('ai_transform', { text, action: a.id });
    if (my !== job) return;
    running = false;
    if (r && r.ok) return result(a, r);
    fail(a, (r && r.error) || 'unknown');
  }

  function result(a, r) {
    const text = String(r.text || '');
    setBody(t(a.label), `<div class="clip-ai-out scroll"></div>
      ${r.clipped ? `<div class="clip-ai-note">${esc(t({ ru: 'Обработаны первые 8000 символов.', en: 'Only the first 8000 characters were processed.' }))}</div>` : ''}
      <div class="clip-ai-btns">
        <button class="btn clip-ai-btn" type="button" data-ai="copy">${ic('copy', { size: 14 })}${esc(t({ ru: 'Копировать', en: 'Copy' }))}</button>
        <button class="btn-primary clip-ai-btn" type="button" data-ai="paste">${ic('enter', { size: 14 })}${esc(t({ ru: 'Вставить', en: 'Paste' }))}</button>
        <span class="clip-ai-sp"></span>
        <button class="btn-ghost clip-ai-btn" type="button" data-ai="menu">${esc(t({ ru: 'Назад', en: 'Back' }))}</button>
        <button class="btn-ghost clip-ai-btn" type="button" data-ai="close">${esc(t({ ru: 'Закрыть', en: 'Close' }))}</button>
      </div>`);
    body.querySelector('.clip-ai-out').textContent = text; // textContent: model output is never parsed as HTML
    body.dataset.text = text;
  }

  function fail(a, code) {
    running = false;
    if (code === 'cancelled') return menu();
    const msg = code === 'empty' ? t({ ru: 'У этой записи нет текста.', en: 'This item has no text.' })
      : code === 'net_off' ? t({ ru: 'ИИ выключен. Включите его в настройках: текст уйдёт только на адрес, который вы там укажете.', en: 'AI is switched off. Turn it on in Settings: the text goes only to the address you set there.' })
      : String(code);
    const main = code === 'net_off'
      ? `<button class="btn-primary clip-ai-btn" type="button" data-ai="enable">${esc(t({ ru: 'Включить ИИ', en: 'Turn AI on' }))}</button>`
      : code === 'empty' ? '' : `<button class="btn clip-ai-btn" type="button" data-ai="run" data-a="${a.id}">${esc(t({ ru: 'Повторить', en: 'Retry' }))}</button>`;
    setBody(t(a.label), `<div class="clip-ai-err" role="alert"></div><div class="clip-ai-btns">${main}<span class="clip-ai-sp"></span>
      <button class="btn-ghost clip-ai-btn" type="button" data-ai="menu">${esc(t({ ru: 'Назад', en: 'Back' }))}</button>
      <button class="btn-ghost clip-ai-btn" type="button" data-ai="close">${esc(t({ ru: 'Закрыть', en: 'Close' }))}</button></div>`);
    body.querySelector('.clip-ai-err').textContent = msg;
  }

  async function put(text, copyOnly) {
    const a = document.activeElement;
    if (a && a.matches && a.matches('input, textarea')) { a.blur(); await sleep(120); }
    const r = await inv('clip_paste_text', { text, copyOnly });
    P.toast({ id: 'clip-ai', icon: copyOnly ? 'copy' : 'clipboard', ttlMs: 1800,
      title: r === 'pasted' ? t({ ru: 'Вставлено', en: 'Pasted' }) : r === 'copied' ? t({ ru: 'Скопировано', en: 'Copied' })
        : r === 'copied_only' ? t({ ru: 'Скопировано (цель с правами админа)', en: 'Copied (target is elevated)' }) : t({ ru: 'Не получилось', en: 'Failed' }) });
    if (r === 'pasted' || r === 'copied_only') { close(); P.collapse('click'); }
  }

  function onClick(e) {
    const b = e.target.closest('[data-ai]');
    if (!b) return;
    e.stopPropagation();
    switch (b.dataset.ai) {
      case 'close': return close();
      case 'menu': job++; return menu();
      case 'run': return run(b.dataset.a);
      case 'cancel': job++; running = false; inv('ai_transform_cancel'); return menu();
      case 'copy': return put(body.dataset.text || '', true);
      case 'paste': return put(body.dataset.text || '', false);
      case 'enable': return inv('settings_open', { page: 'ai' });
    }
  }

  // Escape closes the panel (not the island) whenever the window has the keyboard
  document.addEventListener('keydown', (e) => { if (panel && e.key === 'Escape') { e.stopImmediatePropagation(); e.preventDefault(); close(); } }, true);

  window.PillClipAI = { open, close, isOpen };
})();
