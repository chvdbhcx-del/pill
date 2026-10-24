// Settings > Live events: status line and controls under the OBS, Steam and Notifications rows (GateGuard note: new file of the Pill app, loaded by
// settings.html after settings.js; user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications).
// settings.js calls window.PillLiveExtRow(key, rowEl, helpers) for every live row. Rust side: obs.rs, steam.rs, notif.rs.
// Commands used: obs_status, obs_set_password, steam_status, notif_status, notif_request, settings_set (through put). The OBS password never comes back.
(() => {
  'use strict';
  window.PillLiveExtRow = function (key, r, X) {
    const { h, btn, msg, inv, get, put, t, onLeave } = X;
    if (!['live.obs', 'live.steam', 'live.notifications'].includes(key)) return;
    const blk = h('div', 'pad');
    const st = h('div', 'note');
    blk.append(st);
    r.after(blk);
    const off = () => !get(key);
    const poll = (fn, ms) => { fn(); const id = setInterval(fn, ms); onLeave(() => clearInterval(id)); };
    const show = (text, kind) => msg(st, text, 'note' + (kind ? ' ' + kind : ''));

    if (key === 'live.obs') {
      const inl = h('div', 'inline');
      inl.style.cssText = 'align-items:center;gap:8px;margin-top:10px';
      const port = h('input', 'field'); port.type = 'number'; port.min = 1; port.max = 65535; port.style.width = '96px';
      port.title = t({ ru: 'Порт сервера WebSocket в OBS (по умолчанию 4455)', en: 'WebSocket server port in OBS (default 4455)' });
      port.value = get('live.obs.port') || 4455;
      port.onchange = () => { const n = Math.round(+port.value); if (n >= 1 && n <= 65535) put({ 'live.obs.port': n }); else port.value = get('live.obs.port') || 4455; };
      const pw = h('input', 'field'); pw.type = 'password'; pw.autocomplete = 'off'; pw.spellcheck = false;
      pw.placeholder = t({ ru: 'Пароль сервера WebSocket (если задан в OBS)', en: 'WebSocket server password (if set in OBS)' });
      const save = btn(t({ ru: 'Сохранить пароль', en: 'Save password' }), async () => {
        const ok = await inv('obs_set_password', { password: pw.value });
        pw.value = '';
        msg(st, ok ? '' : t({ ru: 'Не удалось сохранить пароль', en: 'Could not save the password' }), 'note bad');
        refresh();
      }, 'sm');
      inl.append(port, pw, save);
      const hint = h('div', 'footnote', t({ ru: 'Только этот компьютер (127.0.0.1). Пароль хранится зашифрованным (DPAPI) и никуда не отправляется, кроме самого OBS. Пустое поле и «Сохранить» стирают пароль.', en: 'This computer only (127.0.0.1). The password is stored encrypted (DPAPI) and goes nowhere but to OBS itself. An empty field and "Save" erase it.' }));
      blk.append(inl, hint);
      var refresh = async () => {
        const s = await inv('obs_status');
        if (!s) return;
        if (off()) return show(t({ ru: 'Выключено', en: 'Off' }));
        if (document.activeElement !== port) port.value = s.port || 4455;
        if (s.state === 'connected') show(t({ ru: 'OBS подключён', en: 'OBS connected' }), 'ok');
        else if (s.state === 'auth') show(t({ ru: 'OBS просит пароль или он неверный: введите его ниже', en: 'OBS asks for a password, or it is wrong: enter it below' }), 'warn');
        else show(t({ ru: 'OBS не найден (порт ' + s.port + '). Запустите OBS и включите Сервис → Настройки сервера WebSocket → «Включить сервер WebSocket»', en: 'OBS not found (port ' + s.port + '). Start OBS and enable Tools → WebSocket Server Settings → "Enable WebSocket server"' }), 'warn');
      };
      poll(refresh, 2000);
    } else if (key === 'live.steam') {
      poll(async () => {
        const s = await inv('steam_status');
        if (!s) return;
        if (off()) show(t({ ru: 'Выключено', en: 'Off' }));
        else if (!s.installed) show(t({ ru: 'Steam не найден на этом компьютере', en: 'Steam not found on this computer' }), 'warn');
        else if (s.running) show(t({ ru: 'Steam запущен: слежу за загрузками (файлы steamapps читаются только на чтение)', en: 'Steam is running: watching downloads (the steamapps files are only read)' }), 'ok');
        else show(t({ ru: 'Steam не запущен: начну следить, когда он откроется', en: 'Steam is not running: I will start watching when it opens' }));
      }, 3000);
    } else {
      const ask = btn(t({ ru: 'Разрешить доступ', en: 'Allow access' }), async () => { const s = await inv('notif_request'); paint(s && s.status); }, 'sm');
      ask.hidden = true;
      const hint = h('div', 'footnote', t({ ru: 'Pill не убирает исходное уведомление: Windows может показать и своё всплывающее окно, если не включён режим фокусировки. Доступ разрешается один раз: Параметры → Конфиденциальность → Уведомления.', en: 'Pill does not remove the original notification: Windows may still show its own popup unless Focus Assist is on. Access is granted once: Settings → Privacy → Notifications.' }));
      const ir = h('div', 'inline', ask); ir.style.marginTop = '10px';
      blk.append(ir, hint);
      var paint = (s) => {
        ask.hidden = !(s === 'pending' || s === 'denied' || s === 'unsupported');
        if (off()) { ask.hidden = true; return show(t({ ru: 'Выключено', en: 'Off' })); }
        if (s === 'granted') show(t({ ru: 'Доступ к уведомлениям разрешён: карточки приходят', en: 'Notification access granted: cards arrive' }), 'ok');
        else if (s === 'denied') show(t({ ru: 'Доступ запрещён. Разрешите в Параметры → Конфиденциальность → Уведомления', en: 'Access denied. Allow it in Settings → Privacy → Notifications' }), 'bad');
        else if (s === 'unsupported') show(t({ ru: 'Windows не отдаёт чтение уведомлений этому приложению (обычно нужен пакет приложения). Карточки уведомлений недоступны, переключатель ничего не сделает.', en: 'Windows does not give notification access to this app (it usually needs an app package). Notification cards are unavailable; the switch will do nothing.' }), 'warn');
        else show(t({ ru: 'Windows ещё не спрашивал разрешение. Разрешите в Параметры → Конфиденциальность → Уведомления или нажмите кнопку', en: 'Windows has not been asked yet. Allow it in Settings → Privacy → Notifications or press the button' }), 'warn');
      };
      poll(async () => { const s = await inv('notif_status'); if (s) paint(s.status); }, 3000);
    }
  };
})();
