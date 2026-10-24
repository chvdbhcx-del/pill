// Coding-agent status in the collapsed pill (GateGuard note: new file of the Pill app, loaded by pill.html; user request:
// "agent completion reports in the pill"). Owner: agents lane (A7). Rust side: src-tauri/src/agents.rs.
//  - agents:changed {sessions, working, waiting} -> live widget 'agents': waiting (amber, prio 70) beats working (spinner, prio 60)
//  - agents:done {session, tool, title, cwd}      -> green check flash (prio 75, 1.6 s) + liquid ripple + card (Dismiss / Copy folder path)
// PRIVACY: titles come from files written by the user's own hooks; they are only ever shown as text (textContent / esc), never as HTML.
// Extra: window.PillAgentsHookText() resolves to the ready-to-paste hook text for a Settings section.
(() => {
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const { t } = P;
  const FLASH_MS = 1600, CARD_MS = 9000;
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const TOOLS = { 'claude-code': 'Claude Code', codex: 'Codex' };
  const check = '<svg class="ag-check" viewBox="0 0 24 24" width="18" height="18" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><circle class="ag-ring" cx="12" cy="12" r="9.5" stroke-width="1.6"/><path d="M7.5 12.5l3 3 6-6.5"/></svg>';

  // ---- live widget ----
  function render(snap) {
    if (!P.liveOn('agents')) { P.setLive('agents', null); P.unreact('agents'); return; } // Settings > Live events > AI agents
    const n = (k) => Math.max(0, (snap && snap[k]) | 0);
    const waiting = n('waiting'), working = n('working');
    if (waiting > 0) {
      const txt = waiting === 1 ? t({ ru: 'Нужен ваш ответ', en: 'Needs your input' }) : t({ ru: `Ждут вас: ${waiting}`, en: `${waiting} need your input` });
      P.setLive('agents', { priority: 70, accent: 'var(--amber, #ffb340)', html: `<div class="lp ag ag-wait"><span class="ag-dot"></span><span class="ag-txt">${esc(txt)}</span></div>` });
      P.react('agents', 'attention'); // edge: stays while any agent waits for input
    } else if (working > 0) {
      const txt = working === 1 ? t({ ru: 'Агент работает', en: 'Agent working' }) : t({ ru: `Агентов в работе: ${working}`, en: `${working} agents working` });
      P.setLive('agents', { priority: 60, html: `<div class="lp ag ag-work"><span class="ag-spin"></span><span class="ag-txt">${esc(txt)}</span></div>` });
      P.react('agents', 'thinking'); // edge: stays while any agent works
    } else {
      P.setLive('agents', null);
      P.unreact('agents');
    }
  }

  // ---- finished ----
  function edge(strength) {
    if (typeof P.edgePulse === 'function') { P.edgePulse(strength); return; }
    if (P.settings && P.settings.reduceMotion) return;
    const w = document.getElementById('wave');
    if (!w) return;
    w.classList.remove('lv-ripple'); void w.offsetWidth; w.classList.add('lv-ripple'); // keyframes live in agents-live.css (also defined by live-events.css)
    setTimeout(() => w.classList.remove('lv-ripple'), 900);
  }
  async function copyText(s) {
    try { await navigator.clipboard.writeText(s); return; } catch (e) { /* no focus: fall back */ }
    const ta = document.createElement('textarea');
    ta.value = s; ta.style.cssText = 'position:fixed;left:-9999px;top:0;opacity:0';
    document.body.append(ta); ta.select();
    try { document.execCommand('copy'); } catch (e) { console.error('agents copy', e); }
    ta.remove();
  }
  // Session titles are the first words of a prompt / reply: drop markdown marks (# ** ` > _ [ ]) so the card reads as plain text.
  const plainTitle = (s) => String(s || '').replace(/[#*`>_\[\]]+/g, ' ').replace(/\s+/g, ' ').trim();
  function done(p) {
    if (!p || !p.session || !P.liveOn('agents')) return;
    edge(0.9);
    P.react('agent-done', 'done', { ttlMs: 4000 });
    if (P.state === 'collapsed') {
      P.setLive('agent-done', { priority: 75, ttlMs: FLASH_MS, accent: 'var(--green, #58ddab)', html: `<div class="lp ag ag-done"><span class="ag-ok">${check}</span><span class="ag-txt">${esc(t({ ru: 'Готово', en: 'Done' }))}</span></div>` });
    }
    const tool = TOOLS[p.tool] || '';
    const actions = [{ label: t({ ru: 'Скрыть', en: 'Dismiss' }), run: () => P.invoke('agents_dismiss', { session: p.session }) }];
    if (p.cwd) actions.push({ label: t({ ru: 'Копировать путь', en: 'Copy folder path' }), kind: 'white', run: () => copyText(String(p.cwd)) });
    P.toast({
      id: 'agent-done:' + p.session, icon: check, tone: 'green', ttlMs: CARD_MS,
      title: t({ ru: 'Агент закончил', en: 'Agent finished' }),
      sub: [plainTitle(p.title), tool].filter(Boolean).join(' · ') || t({ ru: 'Сеанс завершён', en: 'Session complete' }),
      actions,
    });
  }

  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('agents-live', n, err); } });
  on('agents:changed', render);
  on('agents:done', done);
  let agentsWas = P.liveOn('agents');
  on('settings:changed', () => { const now = P.liveOn('agents'); if (now === agentsWas) return; agentsWas = now; P.invoke('agents_list').then((s) => render(s || {})); }); // the switch works at once, both ways
  P.invoke('agents_list').then((s) => { if (s) render(s); }); // events sent before this script loaded are gone: ask once

  // Settings sections are built inside pill.js and it has no extension point yet: the lead wires this text into a "Agent status" section.
  window.PillAgentsHookText = async () => {
    const s = await P.invoke('agents_hook_snippets');
    if (!s) return '';
    return [
      t({ ru: 'Статус агентов: хуки пишет ваш собственный конфиг, приложение их не меняет.', en: 'Agent status: the hooks live in your own config, the app never edits it.' }),
      '', 'Claude Code - ' + s.claude_code_where, s.claude_code,
      '', 'Codex - ' + s.codex_where, s.codex,
      '', t({ ru: 'Папка статусов: ', en: 'Status folder: ' }) + s.dir,
      '', ...(s.uncertain || []).map((x) => '- ' + x),
    ].join('\n');
  };
})();
