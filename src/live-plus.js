// More live events as cards in the island: finished downloads, removable drives, battery, next calendar event. New file of the Pill app, loaded by
// pill.html after sysmon-live.js; user request: island events "Downloads / USB / Battery / Calendar". Rust side: src-tauri/src/liveplus.rs.
//  - live:file-done {name, path, more}            -> "Загружено: <name>" card with a file icon; a click on the card opens Explorer with the file selected
//  - live:usb {letter, added}                     -> "Подключён диск E:" / "Диск E: отключён"
//  - live:battery {kind: low|charging|full, percent} -> "Заряд 20 %" (amber, 10 % and less red) / "Заряжается" / "Заряжено"
//  - live:calendar {title, minutes}               -> "Через 10 мин: <title>" (next VEVENT of the .ics file / subscription URL from Settings)
//  - live:mail {from, subject, more}               -> "Новое письмо: <отправитель> — <тема>" (+ "и ещё N"); IMAP polling in mail.rs, opt-in (live.mail)
// Switches: Settings > Live events (live.dlDone / live.usb / live.battery / live.calendar / live.mail). PRIVACY: names and titles are only inserted as text.
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const { t } = P;
  const inv = (c, a) => P.invoke(c, a);
  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('live-plus', n, err); } });
  const svg = (d) => `<svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${d}</svg>`;
  const usbIcon = svg('<path d="M12 3v13M12 16a2 2 0 1 0 0 4 2 2 0 0 0 0-4zM12 11l4-3v-2M12 13l-4-3v-2"/><rect x="14.500" y="3.500" width="3" height="3" rx=".5"/><circle cx="8" cy="7" r="1.200"/>');
  const calIcon = svg('<rect x="3.500" y="5" width="17" height="15" rx="2.500"/><path d="M3.500 10h17M8 3v4M16 3v4"/>');
  const mailIcon = svg('<rect x="3" y="5" width="18" height="14" rx="2.500"/><path d="m3.500 7.500 8.500 6 8.500-6"/>');
  const close = () => ({ label: t({ ru: 'Закрыть', en: 'Close' }), run: () => {} });
  const clip = (s, n) => { s = String(s || ''); return s.length > n ? s.slice(0, n - 1) + '…' : s; };

  // ---- downloads. livesys.rs already announces browser downloads it watched grow (live:download "done"): remember those names and stay quiet for them
  const announced = new Map(); // lower-case name -> time
  on('live:download', (p) => { if (p && p.state === 'done' && p.name) announced.set(String(p.name).toLowerCase(), Date.now()); });
  function fileDone(p) {
    if (!p || !p.name || !P.liveOn('dlDone')) return;
    setTimeout(() => { // livesys.rs reports a browser download within ~1 s of the rename; look again after that
      const name = String(p.name), path = String(p.path || ''), more = p.more | 0;
      for (const [k, v] of announced) if (Date.now() - v > 60000) announced.delete(k);
      if (announced.has(name.toLowerCase()) || !P.liveOn('dlDone')) return;
      const reveal = () => inv('livesys_reveal', { path }); // Rust accepts only files inside the Downloads folder
      P.toast({
        id: 'filedone', icon: P.icon('file', { size: 20 }), tone: 'green', ttlMs: 8000,
        title: t({ ru: 'Загружено: ', en: 'Downloaded: ' }) + clip(name, 40),
        sub: more > 0 ? t({ ru: 'и ещё ' + more, en: 'and ' + more + ' more' }) : t({ ru: 'Нажмите, чтобы показать в папке', en: 'Click to show in folder' }),
        actions: [{ label: t({ ru: 'Показать в папке', en: 'Show in folder' }), kind: 'white', run: reveal }, close()],
        fit: (el) => { // the whole card is a button: a click outside its own buttons runs the first action (which also closes the card)
          el.style.cursor = 'pointer';
          el.addEventListener('click', (e) => { if (!e.target.closest('button')) { const b = el.querySelector('.ab'); if (b) b.click(); } });
        },
      });
    }, 1500);
  }

  function usb(p) {
    if (!p || !p.letter || !P.liveOn('usb')) return;
    const l = String(p.letter).slice(0, 1).toUpperCase() + ':';
    P.toast({
      id: 'usb:' + l, icon: usbIcon, tone: p.added ? 'accent' : undefined, ttlMs: 4000,
      title: p.added ? t({ ru: 'Подключён диск ' + l, en: 'Drive ' + l + ' connected' }) : t({ ru: 'Диск ' + l + ' отключён', en: 'Drive ' + l + ' disconnected' }),
      actions: [close()],
    });
  }

  function battery(p) {
    if (!p || !P.liveOn('battery')) return;
    const pct = Math.max(0, Math.min(100, p.percent | 0));
    let title, sub, tone, ico = 'battery';
    if (p.kind === 'low') {
      title = t({ ru: 'Заряд ' + pct + ' %', en: 'Battery ' + pct + '%' });
      sub = t({ ru: 'Подключите зарядное устройство', en: 'Plug in the charger' });
      tone = pct <= 10 ? 'red' : 'amber';
    } else if (p.kind === 'charging') {
      title = t({ ru: 'Заряжается', en: 'Charging' }); sub = pct + ' %'; tone = 'green'; ico = 'bolt';
    } else if (p.kind === 'full') {
      title = t({ ru: 'Заряжено', en: 'Fully charged' }); sub = t({ ru: 'Можно отключить зарядку', en: 'You can unplug the charger' }); tone = 'green';
    } else return;
    P.toast({ id: 'battery', icon: P.icon(ico, { size: 20 }), tone, ttlMs: p.kind === 'low' ? 8000 : 5000, title, sub, actions: [close()] });
  }

  function calendar(p) {
    // opt-in: without an explicit true the producer is off (Pill.liveOn treats a missing key as on)
    if (!p || !p.title || !(P.settings && P.settings['live.calendar'] === true)) return;
    const m = Math.max(1, p.minutes | 0);
    P.toast({
      id: 'cal:' + p.title, icon: calIcon, tone: 'accent', ttlMs: 12000,
      title: t({ ru: 'Через ' + m + ' мин: ', en: 'In ' + m + ' min: ' }) + p.title,
      actions: [close()],
    });
  }

  function mail(p) {
    // opt-in like the calendar: without an explicit true nothing is shown
    if (!p || !(P.settings && P.settings['live.mail'] === true)) return;
    const more = p.more | 0;
    P.toast({
      id: 'mail', icon: mailIcon, tone: 'accent', ttlMs: 10000,
      title: t({ ru: 'Новое письмо: ', en: 'New mail: ' }) + clip(p.from || t({ ru: 'неизвестный отправитель', en: 'unknown sender' }), 40) + ' — ' + clip(p.subject || t({ ru: '(без темы)', en: '(no subject)' }), 60),
      sub: more > 0 ? t({ ru: 'и ещё ' + more, en: 'and ' + more + ' more' }) : undefined,
      actions: [close()],
    });
  }

  on('live:file-done', fileDone);
  on('live:usb', usb);
  on('live:battery', battery);
  on('live:calendar', calendar);
  on('live:mail', mail);
})();
