// Live reactions of the collapsed pill (GateGuard note: new file of the Pill app, loaded by pill.html;
// user request: "keyboard layout indicator and copy reaction for the pill"). Owner: live-events lane.
//  - kbd:layout {lang, name}  (watch.rs)  -> "RU" chip for ~1.4 s, priority 90
//  - clip:added {kind, fileCount, ...} (clipboard.rs) -> "Copied · text/image/N files" for ~1.3 s, priority 80
// PRIVACY: the copied text is never read here, only `kind` and `fileCount`.
(() => {
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const { t, icon } = P;
  const KBD_MS = 1400, COPY_MS = 1300;
  const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const collapsed = () => P.state === 'collapsed';
  const kbdIcon = '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><rect x="2.5" y="6" width="19" height="12" rx="3"/><path d="M6.5 10h.01M10 10h.01M14 10h.01M17.5 10h.01M7.5 14h9"/></svg>';

  function kbd(p) {
    if (!p || !p.lang || !collapsed() || !P.liveOn('keyboardLayout')) return; // Settings > Live events > Keyboard layout (watch.rs also stops polling)
    P.setLive('kbd', {
      priority: 90, ttlMs: KBD_MS,
      html: `<div class="lp lv lv-kbd" title="${esc(p.name || '')}"><span class="lv-ico">${kbdIcon}</span><span class="lv-code">${esc(String(p.lang).slice(0, 3))}</span></div>`,
    });
  }

  // 1 file / 2-4 files / 5+ files in Russian, plain plural in English.
  const files = (n) => t({
    ru: `${n} ${n % 10 === 1 && n % 100 !== 11 ? 'файл' : n % 10 >= 2 && n % 10 <= 4 && (n % 100 < 10 || n % 100 >= 20) ? 'файла' : 'файлов'}`,
    en: `${n} ${n === 1 ? 'file' : 'files'}`,
  });
  function kindLabel(p) {
    if (p.kind === 'image') return t({ ru: 'Изображение', en: 'Image' });
    if (p.kind === 'files') return files(Math.max(1, p.fileCount | 0));
    return t({ ru: 'Текст', en: 'Text' });
  }

  // Liquid edge: the shell's edgePulse(strength) when it exists, else a CSS pulse on #wave.
  function edge(strength) {
    if (P.quiet && P.quiet()) return; // quiet hours
    if (typeof P.edgePulse === 'function') { P.edgePulse(strength); return; }
    if (P.settings && P.settings.reduceMotion) return;
    const w = document.getElementById('wave');
    if (!w) return;
    w.classList.remove('lv-ripple'); void w.offsetWidth; w.classList.add('lv-ripple');
    setTimeout(() => w.classList.remove('lv-ripple'), 900);
  }

  function copied(p) {
    if (!p || !p.kind || !P.liveOn('copy')) return; // nothing stored / Settings > Live events > Reaction to copying is off
    edge(0.8);
    if (!collapsed()) return;
    P.setLive('copy', {
      priority: 80, ttlMs: COPY_MS,
      html: `<div class="lp lv lv-copy"><span class="lv-ico">${icon('clipboard', { size: 16, sw: 1.8 })}</span><span class="lv-txt">${esc(t({ ru: 'Скопировано', en: 'Copied' }))}</span><span class="lv-dim">${esc(kindLabel(p))}</span></div>`,
    });
  }

  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('live-events', n, err); } });
  on('kbd:layout', kbd);
  on('clip:added', copied);
  // The poller in watch.rs slows down while the pill is hidden.
  const inv = (c) => P.invoke(c);
  on('pill:hide', () => inv('watch_pause'));
  on('pill:show', () => inv('watch_resume'));
})();
