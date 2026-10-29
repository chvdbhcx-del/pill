// Live events from OBS, Steam and Windows notifications, pill side (GateGuard note: new file of the Pill app, loaded by pill.html after agents-live.js;
// user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications). Rust side: src-tauri/src/obs.rs, steam.rs, notif.rs.
//  - live:obs {recording, streaming, recMs, streamMs, file?}     -> red REC / LIVE chip (prio 90) with a timer; card "Recording saved" when file arrives
//  - live:steam {state, game, appid, percent, speed, etaSec}    -> ring chip with the game and percent (prio 45, below the music pill); card on state "done"
//  - live:notification {app, title, body, id}                   -> card: app letter (bell when no app name), title <= 2 lines, body <= 3 lines, "Close"
// PRIVACY: every string from outside (file names, game names, notification texts) is only ever inserted escaped as text, never as HTML.
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const t = P.t;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const base = (p) => String(p || '').split(/[\\/]/).pop();
  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('live-ext', n, err); } });
  const none = () => {};
  const clock = (ms) => {
    const s = Math.max(0, Math.floor(ms / 1000)), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = String(s % 60).padStart(2, '0');
    return h ? `${h}:${String(m).padStart(2, '0')}:${r}` : `${m}:${r}`;
  };
  const ok = '<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 12.5l4 4 9-9.5"/></svg>';

  // ---------------------------------------------------------------- OBS
  const obs = { rec: 0, live: 0, tick: 0 }; // start times (epoch ms), 0 = not active
  let obsHid = false;
  const obsTick = () => document.querySelectorAll('.lx-rt').forEach((e) => { const s = clock(Date.now() - (+e.dataset.since || Date.now())); if (e.textContent !== s) e.textContent = s; });
  const obsArm = () => { clearInterval(obs.tick); obs.tick = (obs.rec || obs.live) && !obsHid ? setInterval(obsTick, 500) : 0; }; // no ticks while the island is hidden (nobody sees the text)
  function obsRender() {
    if (!P.liveOn('obs') || (!obs.rec && !obs.live)) {
      P.setLive('obs', null);
      clearInterval(obs.tick); obs.tick = 0;
      return;
    }
    const part = (cls, label, since) => `<span class="lx-dot ${cls}"></span><b class="lx-lab ${cls}">${label}</b><span class="lx-rt" data-since="${since}">${clock(Date.now() - since)}</span>`;
    const html = [obs.rec && part('rec', 'REC', obs.rec), obs.live && part('live', 'LIVE', obs.live)].filter(Boolean).join('<span class="lx-sep"></span>');
    P.setLive('obs', { priority: 90, ttlMs: null, accent: '#ff423a', html: `<div class="lp lx lx-obs" title="${esc(t({ ru: 'OBS записывает или ведёт трансляцию', en: 'OBS is recording or streaming' }))}">${html}</div>` });
    if (!obs.tick && !obsHid) obsArm();
  }
  T.event.listen('pill:hide', () => { obsHid = true; obsArm(); });
  T.event.listen('pill:show', () => { obsHid = false; obsTick(); obsArm(); });
  on('live:obs', (p) => {
    if (!p) return;
    obs.rec = p.recording ? (obs.rec || Date.now() - (+p.recMs || 0)) : 0; // a stream event must not restart a running REC timer
    obs.live = p.streaming ? (obs.live || Date.now() - (+p.streamMs || 0)) : 0;
    obsRender();
    if (p.file && P.liveOn('obs')) {
      const f = String(p.file);
      P.toast({
        id: 'obs-saved', icon: ok, tone: 'green', ttlMs: 12000,
        title: t({ ru: 'Запись сохранена', en: 'Recording saved' }), sub: base(f),
        actions: [{ label: t({ ru: 'Показать в папке', en: 'Show in folder' }), kind: 'white', run: () => P.invoke('obs_reveal', { path: f }) }, { label: t({ ru: 'Закрыть', en: 'Close' }), run: none }],
      });
    }
  });

  // ---------------------------------------------------------------- Steam
  const mbs = (b) => { const m = b / 1048576; return (m >= 10 ? Math.round(m) : m.toFixed(1)) + ' ' + t({ ru: 'МБ/с', en: 'MB/s' }); };
  const eta = (s) => (s >= 3600 ? Math.floor(s / 3600) + t({ ru: ' ч ', en: ' h ' }) + Math.round((s % 3600) / 60) + t({ ru: ' мин', en: ' min' }) : Math.max(1, Math.round(s / 60)) + t({ ru: ' мин', en: ' min' }));
  function steamChip(p) {
    const pct = Math.max(0, Math.min(100, +p.percent || 0)), C = 2 * Math.PI * 6.5;
    const ring = `<svg class="lx-ring" viewBox="0 0 16 16" width="16" height="16"><circle cx="8" cy="8" r="6.5" class="bg"/><circle cx="8" cy="8" r="6.5" class="fg" style="stroke-dasharray:${C.toFixed(2)};stroke-dashoffset:${(C * (1 - pct / 100)).toFixed(2)}"/></svg>`;
    const right = p.state === 'paused' ? t({ ru: 'пауза', en: 'paused' }) : p.state === 'installing' ? t({ ru: 'установка', en: 'installing' }) : pct + '%';
    const extra = p.state === 'downloading' && p.speed > 0 ? `<span class="lx-sp">${esc(mbs(p.speed))}</span>` : '';
    const tip = [p.game, p.state === 'downloading' && p.speed > 0 ? mbs(p.speed) : '', p.etaSec > 0 ? t({ ru: 'осталось ', en: 'left ' }) + eta(p.etaSec) : ''].filter(Boolean).join(' · ');
    return `<div class="lp lx lx-steam" title="${esc(tip)}">${ring}<span class="lx-g">${esc(p.game || 'Steam')}</span><span class="lx-pg"><b class="lx-p">${esc(right)}</b>${extra}</span></div>`;
  }
  on('live:steam', (p) => {
    if (!p || !P.liveOn('steam')) { P.setLive('steam', null); return; }
    if (p.state === 'done') {
      P.setLive('steam', null);
      if (typeof P.edgePulse === 'function') P.edgePulse(0.7);
      P.toast({ id: 'steam-done:' + p.appid, icon: P.icon('download', { size: 20 }), tone: 'green', ttlMs: 8000, title: p.game || 'Steam', sub: t({ ru: 'Загрузка Steam завершена', en: 'Steam download finished' }), actions: [{ label: t({ ru: 'Закрыть', en: 'Close' }), run: none }] });
    } else if (p.state === 'idle') P.setLive('steam', null);
    else P.setLive('steam', { priority: 45, ttlMs: null, html: steamChip(p) });
  });

  // ---------------------------------------------------------------- Windows notifications
  // the card grows with its text (title 2 + body 3 lines at most); when the OS frame leaves less room than that, drop body lines first, then title lines
  function fitNotif(el, room) {
    let tl = 2, bl = 3;
    const set = () => { el.style.setProperty('--lx-tl', tl); el.style.setProperty('--lx-bl', bl); };
    set();
    while (el.offsetHeight > room && (bl > 1 || tl > 1)) { if (bl > 1) bl--; else tl--; set(); }
  }
  on('live:notification', (p) => {
    if (!p || !P.liveOn('notifications')) return;
    const app = String(p.app || '').trim(), title = String(p.title || '').trim() || app, body = String(p.body || '').trim();
    // known app: its first letter; system notifications come without one: a generic bell instead of an arbitrary letter from the title
    const icon = app ? `<span class="lx-ico">${esc(Array.from(app)[0].toUpperCase())}</span>` : P.icon('bell', { size: 18 });
    P.toast({
      id: 'notif:' + p.id, icon, ttlMs: 7000, cls: 'tn', fit: fitNotif,
      html: `<div class="lx-n" title="${esc(app)}"><div class="lx-nt">${esc(title)}</div>${body ? `<div class="lx-nb">${esc(body)}</div>` : ''}</div>`,
      actions: [{ label: t({ ru: 'Закрыть', en: 'Close' }), run: none }],
    });
  });

  // ---------------------------------------------------------------- uninstall: a finished step while the user is elsewhere
  // Rust (uninstall.rs) sends this only when the command bar is closed; the bar's own status strip covers the open bar. Quiet hours apply (not urgent).
  on('uninstall:ready', (p) => {
    const name = String(p.name || ''), n = +p.count || 0, kind = String(p.kind || '');
    const sub = kind === 'review' ? t({ ru: 'Остатков к проверке: ', en: 'Leftovers to review: ' }) + n
      : kind === 'done' ? (n ? t({ ru: 'Готово, удалено: ', en: 'Done, removed: ' }) + n : t({ ru: 'Остатков не найдено', en: 'No leftovers found' }))
      : kind === 'still' ? t({ ru: 'Приложение всё ещё установлено', en: 'The app is still installed' }) : t({ ru: 'Не получилось', en: 'It did not work' });
    P.toast({
      id: 'uninstall:' + name, icon: kind === 'error' || kind === 'still' ? 'trash' : 'check', tone: kind === 'error' || kind === 'still' ? undefined : 'green', ttlMs: 12000,
      title: t({ ru: 'Удаление: ', en: 'Uninstall of ' }) + name, sub,
      actions: [
        { label: t({ ru: 'Открыть', en: 'Open' }), kind: 'white', run: () => T.core.invoke('uninstall_open_panel').catch(() => {}) },
        { label: t({ ru: 'Закрыть', en: 'Close' }), run: none },
      ],
    });
  });

  // a switch flipped off removes its chip at once (Rust also stops its worker)
  on('settings:changed', () => {
    if (!P.liveOn('obs')) { obs.rec = obs.live = 0; obsRender(); }
    if (!P.liveOn('steam')) P.setLive('steam', null);
  });
})();
