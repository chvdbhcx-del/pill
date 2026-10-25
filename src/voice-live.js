// Voice to text, pill side (GateGuard note: new file of the Pill app, loaded by pill.html after agents-live.js; user request: "делай все оставшееся",
// part: voice-to-text). Rust side: src-tauri/src/voice.rs. Events from Rust: voice:state {state, startedAt?, text?, code?, message?}, voice:level {rms, level}.
//  - recording: live chip 'voice' (coral dot, m:ss, 5-bar level meter, Stop) + edge 'attention' pulse
//  - transcribing: spinner chip + edge 'thinking'
//  - done: green card "Текст в буфере" (first 80 chars, Copy / Dismiss) ; error: amber/red card, never contains a key
// Also defines Pill.dictate (the pill header button calls it) and PillVoice.toggle() for other tabs (the chat microphone).
// PRIVACY: the transcript is shown only as text (textContent / esc) and is not stored here.
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const t = P.t;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const cut = (s, n) => { s = String(s || '').replace(/\s+/g, ' ').trim(); return s.length > n ? s.slice(0, n - 1) + '…' : s; };
  const inv = (c, a) => P.invoke(c, a);
  const clock = (ms) => { const s = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

  const css = document.createElement('style');
  css.textContent = `
.lp.vc-live { width: 214px; min-width: 0; box-sizing: border-box; padding: 0 4px 2px 14px; gap: 9px; justify-content: flex-start; font-variant-numeric: tabular-nums; }
.vc-dot { flex: none; width: 10px; height: 10px; border-radius: 50%; background: #ff5a4e; box-shadow: 0 0 4px 1px rgba(255, 90, 78, .35); animation: vc-pulse 2s ease-in-out infinite; }
@keyframes vc-pulse { 0%, 100% { opacity: 1; } 50% { opacity: .5; } }
.vc-t { min-width: 28px; font: 700 13px / 1 var(--font, Inter, "Segoe UI", system-ui, sans-serif); color: var(--text, #fff); }
.vc-meter { flex: 1; display: flex; align-items: center; justify-content: center; gap: 3px; height: 18px; }
.vc-meter i { display: block; width: 3px; height: 16px; border-radius: 2px; background: #ff7468; transform: scaleY(.14); transform-origin: 50% 50%; transition: transform 70ms linear; }
.vc-stop { all: unset; flex: none; box-sizing: border-box; width: 52px; height: 24px; border-radius: 12px; display: inline-flex; align-items: center; justify-content: center; gap: 6px;
  background: rgba(255, 67, 63, .20); cursor: pointer; }
.vc-stop:hover { background: rgba(255, 67, 63, .30); }
.vc-stop:focus-visible { outline: 2px solid #ff433f; outline-offset: 1px; }
.vc-stop i { flex: none; width: 8px; height: 8px; border-radius: 1.5px; background: #ff433f; }
.vc-stop span { font: 600 12px / 1 var(--font, system-ui); color: #f76f6b; }
.vc-spin { flex: none; width: 14px; height: 14px; border-radius: 50%; border: 2px solid rgba(255, 255, 255, .22); border-top-color: var(--text, #fff); animation: vc-rot .8s linear infinite; }
@keyframes vc-rot { to { transform: rotate(360deg); } }
.vc-txt { font: 600 12.5px / 1 var(--font, system-ui); color: var(--text, #fff); white-space: nowrap; }
.rm .vc-dot, .rm .vc-spin { animation: none; }`;
  document.head.append(css);

  const BARS = [0.55, 0.8, 1, 0.8, 0.55]; // centre bar follows the level most
  let startedAt = 0, timer = 0, state = 'idle', lvl = 0, shown = 0;

  function recChip() {
    return `<div class="lp vc-live" title="${esc(t({ ru: 'Диктовка: говорите', en: 'Dictating: speak now' }))}"><span class="vc-dot"></span><span class="vc-t">${clock(Date.now() - startedAt)}</span>`
      + `<span class="vc-meter">${BARS.map(() => '<i></i>').join('')}</span>`
      + `<button type="button" class="vc-stop" data-vc-stop title="${esc(t({ ru: 'Остановить и распознать (Ctrl+Alt+D)', en: 'Stop and transcribe (Ctrl+Alt+D)' }))}"><i></i><span>${esc(t({ ru: 'Стоп', en: 'Stop' }))}</span></button></div>`;
  }
  function busyChip() {
    return `<div class="lp vc-live"><span class="vc-spin"></span><span class="vc-txt">${esc(t({ ru: 'Распознаю…', en: 'Transcribing…' }))}</span></div>`;
  }
  function tick() {
    if (state !== 'recording') return;
    const s = clock(Date.now() - startedAt);
    document.querySelectorAll('.vc-t').forEach((e) => { if (e.textContent !== s) e.textContent = s; });
  }
  function paintMeter() {
    shown += (lvl - shown) * 0.6; // light smoothing on top of Rust's 25 Hz
    document.querySelectorAll('.vc-meter').forEach((m) => {
      [...m.children].forEach((b, i) => { b.style.transform = `scaleY(${Math.max(0.14, Math.min(1, shown * BARS[i] + 0.06)).toFixed(2)})`; });
    });
  }
  const clearLive = () => { clearInterval(timer); P.setLive('voice', null); P.unreact('voice'); };

  const ERR = {
    off: ['amber', { ru: 'Голос в текст выключен', en: 'Voice to text is off' }, { ru: 'Включи его в настройках', en: 'Turn it on in Settings' }, 1],
    no_key: ['amber', { ru: 'Нет ключа API', en: 'No API key' }, null, 1],
    no_endpoint: ['amber', { ru: 'Не указан адрес сервиса', en: 'No service address' }, null, 1],
    mic: ['red', { ru: 'Нет доступа к микрофону', en: 'Cannot use the microphone' }, null, 0],
    too_short: ['amber', { ru: 'Слишком короткая запись', en: 'Recording too short' }, { ru: 'Скажи что-нибудь подольше', en: 'Say something longer' }, 0],
    silent: ['amber', { ru: 'Микрофон ничего не услышал', en: 'The microphone heard nothing' }, { ru: 'Проверь микрофон по умолчанию и его громкость', en: 'Check the default microphone and its volume' }, 0],
    empty: ['amber', { ru: 'Речь не распознана', en: 'No speech recognised' }, null, 0],
    net: ['red', { ru: 'Нет связи с сервисом', en: 'Cannot reach the service' }, null, 1],
    http: ['red', { ru: 'Сервис вернул ошибку', en: 'The service returned an error' }, null, 1],
  };

  function onState(p) {
    if (!p) return;
    state = p.state;
    document.dispatchEvent(new CustomEvent('voice:state', { detail: p }));
    if (p.state === 'recording') {
      startedAt = +p.startedAt || Date.now();
      lvl = shown = 0;
      P.setLive('voice', { priority: 96, accent: '#ff5a4e', html: recChip(), ttlMs: null });
      P.react('voice', 'attention', { ttlMs: 1600 });
      clearInterval(timer);
      timer = setInterval(tick, 250);
    } else if (p.state === 'transcribing') {
      clearInterval(timer);
      P.setLive('voice', { priority: 96, html: busyChip(), ttlMs: null });
      P.react('voice', 'thinking');
    } else if (p.state === 'done') {
      clearLive();
      P.react('voice', 'done', { ttlMs: 2200 });
      const text = String(p.text || '');
      P.toast({
        id: 'voice', icon: 'mic', tone: 'green', ttlMs: 8000,
        title: t({ ru: 'Текст в буфере', en: 'Text on the clipboard' }), sub: cut(text, 80),
        actions: [
          { label: t({ ru: 'Копировать', en: 'Copy' }), kind: 'white', run: () => inv('capture_action', { action: 'copy_text', value: text }) },
          { label: t({ ru: 'Закрыть', en: 'Dismiss' }), run: () => {} },
        ],
      });
      document.dispatchEvent(new CustomEvent('voice:done', { detail: { text } }));
    } else if (p.state === 'error') {
      clearLive();
      if (p.code === 'busy') return;
      const e = ERR[p.code] || ['red', { ru: 'Диктовка не удалась', en: 'Dictation failed' }, null, 0];
      const actions = e[3] ? [{ label: t({ ru: 'Настройки', en: 'Settings' }), kind: 'white', run: () => P.invoke('settings_open', { page: 'voice' }) }, { label: t({ ru: 'Закрыть', en: 'Dismiss' }), run: () => {} }] : [{ label: t({ ru: 'Закрыть', en: 'Dismiss' }), run: () => {} }];
      P.toast({ id: 'voice', icon: 'mic', tone: e[0], ttlMs: 7000, title: t(e[1]), sub: e[2] && p.code !== 'no_key' && p.code !== 'no_endpoint' ? t(e[2]) : cut(p.message, 110), actions });
    } else { // idle (cancelled)
      clearLive();
    }
  }

  T.event.listen('voice:state', (e) => onState(e.payload));
  T.event.listen('voice:level', (e) => {
    const p = e.payload;
    if (!p || p.test || state !== 'recording') return;
    lvl = +p.level || 0;
    paintMeter();
  });

  // The Stop button lives in HTML the shell re-renders: delegate, before the pill sees the click.
  document.addEventListener('click', (e) => {
    const b = e.target.closest && e.target.closest('[data-vc-stop]');
    if (!b) return;
    e.stopPropagation();
    inv('voice_stop');
  }, true);

  // A page reload while recording: pick the state up again.
  inv('voice_state').then((s) => { if (s && s.state === 'recording') onState({ state: 'recording', startedAt: s.startedAt }); else if (s && s.state === 'transcribing') onState({ state: 'transcribing' }); });

  // pill header button: while dictation is switched off a click opens its settings page (a toast was too easy to miss)
  P.dictate = async () => {
    const s = await inv('voice_state');
    if (s && s.enabled === false && s.state === 'idle') return inv('settings_open', { page: 'voice' });
    return inv('voice_toggle');
  };
  window.PillVoice = { toggle: () => inv('voice_toggle'), state: () => inv('voice_state') };
})();
