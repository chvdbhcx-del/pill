// Settings > Live events: status line and controls under the OBS, Steam and Notifications rows (GateGuard note: new file of the Pill app, loaded by
// settings.html after settings.js; user request: "делай все оставшееся", part: Live-events producers OBS/Steam/Notifications).
// settings.js calls window.PillLiveExtRow(key, rowEl, helpers) for every live row. Rust side: obs.rs, steam.rs, notif.rs.
// Commands used: obs_status, obs_set_password, steam_status, notif_status, notif_request, liveplus_ics_status, mail_status, mail_set_password, mail_test,
// settings_set (through put). The OBS and mailbox passwords never come back.
(() => {
  'use strict';
  window.PillLiveExtRow = function (key, r, X) {
    const { h, btn, msg, inv, get, put, t, onLeave } = X;
    if (!['live.obs', 'live.steam', 'live.notifications', 'live.calendar', 'live.mail'].includes(key)) return;
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
    } else if (key === 'live.calendar') {
      const path = h('input', 'field'); path.type = 'text'; path.spellcheck = false; path.style.cssText = 'flex:1;min-width:0';
      path.placeholder = t({ ru: 'Путь к файлу .ics (C:\\Users\\Имя\\calendar.ics) или ссылка https:// / webcal://', en: 'Path to an .ics file (C:\\Users\\Name\\calendar.ics) or an https:// / webcal:// link' });
      path.value = get('live.calendar.path') || '';
      path.onchange = () => { put({ 'live.calendar.path': path.value.trim().replace(/^"(.*)"$/, '$1') }); refresh(); };
      const inl = h('div', 'inline'); inl.style.cssText = 'align-items:center;gap:8px;margin-top:10px'; inl.append(path);
      blk.append(inl, h('div', 'footnote', t({ ru: 'Файл с диска только читается. Ссылка-подписка (https:// или webcal://, например «секретный адрес в формате iCal» Google Календаря) скачивается раз в 30 минут, не больше 2 МБ; последняя копия лежит рядом с настройками, так что работает и без интернета. Адрес нигде не записывается, в журнал попадает только имя сервера. Понимаются время UTC и местное, события на весь день (без напоминания) и повторы DAILY / WEEKLY / MONTHLY / YEARLY; часовой пояс TZID не учитывается, время считается местным. Чтобы вставить путь, в Проводнике: Shift + правая кнопка → «Копировать как путь».', en: 'A file on disk is only read. A subscription link (https:// or webcal://, e.g. the “secret address in iCal format” of Google Calendar) is downloaded every 30 minutes, 2 MB at most; the last copy is kept next to the settings, so it also works offline. The address is never logged, only the server name. UTC and local times, all-day events (no reminder) and DAILY / WEEKLY / MONTHLY / YEARLY repeats are understood; a TZID zone is ignored and the time is taken as local. To paste a path, in Explorer: Shift + right click → “Copy as path”.' })));
      var refresh = async () => {
        const s = await inv('liveplus_ics_status');
        if (!s) return;
        if (off()) return show(t({ ru: 'Выключено', en: 'Off' }));
        const src = s.host ? ' ' + t({ ru: '(с ' + s.host + ')', en: '(from ' + s.host + ')' }) : '';
        if (s.state === 'nopath') show(t({ ru: 'Укажите путь к файлу .ics или ссылку https:// / webcal://', en: 'Enter the path to an .ics file or an https:// / webcal:// link' }), 'warn');
        else if (s.state === 'bad') show(s.why === 'http' ? t({ ru: 'Ссылка http:// без шифрования не принимается: используйте https:// или webcal://', en: 'A plain http:// link is not accepted: use https:// or webcal://' }) : s.why === 'scheme' ? t({ ru: 'Поддерживаются путь к файлу и ссылки https:// и webcal://, другие схемы (file://, ftp:// и т. п.) не принимаются', en: 'A file path and https:// or webcal:// links are supported; other schemes (file://, ftp://, ...) are rejected' }) : t({ ru: 'Ссылка не распознана', en: 'The link is not valid' }), 'bad');
        else if (s.state === 'missing' && s.host) show(s.busy || !s.fetchErr ? t({ ru: 'Загружаю календарь с ' + s.host + '…', en: 'Downloading the calendar from ' + s.host + '…' }) : t({ ru: 'Не удалось загрузить календарь с ' + s.host + ': ' + s.fetchErr, en: 'Could not download the calendar from ' + s.host + ': ' + s.fetchErr }), s.busy || !s.fetchErr ? 'warn' : 'bad');
        else if (s.state === 'missing') show(t({ ru: 'Файл не найден или не читается', en: 'The file was not found or cannot be read' }), 'bad');
        else if (s.state === 'big') show(t({ ru: 'Файл слишком большой (больше 8 МБ)', en: 'The file is too large (over 8 MB)' }), 'bad');
        else if (s.fetchErr) show(t({ ru: 'Показана сохранённая копия' + src + '; последняя загрузка не удалась: ' + s.fetchErr, en: 'Showing the saved copy' + src + '; the last download failed: ' + s.fetchErr }), 'warn');
        else if (!s.next) show(t({ ru: 'Событий в файле: ' + s.events + '. Предстоящих со временем начала нет', en: 'Events in the file: ' + s.events + '. None upcoming with a start time' }), 'warn');
        else {
          const m = s.next.inMin, when = m >= 1440 ? Math.floor(m / 1440) + t({ ru: ' дн.', en: ' d' }) : m >= 60 ? Math.floor(m / 60) + t({ ru: ' ч ', en: ' h ' }) + (m % 60) + t({ ru: ' мин', en: ' min' }) : m + t({ ru: ' мин', en: ' min' });
          show(t({ ru: 'Событий в календаре' + src + ': ' + s.events + '. Ближайшее через ' + when + ': ' + s.next.title, en: 'Events in the calendar' + src + ': ' + s.events + '. Next in ' + when + ': ' + s.next.title }), 'ok');
        }
      };
      poll(refresh, 3000);
    } else if (key === 'live.mail') {
      const fld = (type, ph, w) => { const i = h('input', 'field'); i.type = type; i.autocomplete = 'off'; i.spellcheck = false; i.placeholder = ph; if (w) i.style.width = w; else i.style.cssText = 'flex:1;min-width:0'; return i; };
      const row = (...els) => { const d = h('div', 'inline'); d.style.cssText = 'align-items:center;gap:8px;margin-top:10px'; d.append(...els); return d; };
      const host = fld('text', t({ ru: 'Сервер IMAP, например imap.gmail.com', en: 'IMAP server, e.g. imap.gmail.com' }));
      const port = fld('number', '993', '96px'); port.min = 1; port.max = 65535;
      port.title = t({ ru: 'Порт IMAP с шифрованием TLS (обычно 993). Незашифрованный порт 143 не используется', en: 'IMAP port with TLS encryption (usually 993). The unencrypted port 143 is not used' });
      const user = fld('text', t({ ru: 'Логин (обычно адрес почты)', en: 'Login (usually the e-mail address)' }));
      const box = fld('text', 'INBOX', '140px');
      box.title = t({ ru: 'Папка, за которой следить (по умолчанию INBOX)', en: 'Folder to watch (INBOX by default)' });
      host.value = get('mail.host') || ''; port.value = get('mail.port') || 993; user.value = get('mail.user') || ''; box.value = get('mail.mailbox') || 'INBOX';
      host.onchange = () => {
        let v = host.value.trim().toLowerCase().replace(/^[a-z]+:\/\//, '').replace(/\/.*$/, '');
        const m = v.match(/:(\d{1,5})$/);
        if (m) { v = v.slice(0, -m[0].length); port.value = m[1]; put({ 'mail.port': +m[1] }); }
        host.value = v; put({ 'mail.host': v });
      };
      port.onchange = () => { const n = Math.round(+port.value); if (n >= 1 && n <= 65535) put({ 'mail.port': n }); else port.value = get('mail.port') || 993; };
      user.onchange = () => put({ 'mail.user': user.value.trim() });
      box.onchange = () => { box.value = box.value.trim() || 'INBOX'; put({ 'mail.mailbox': box.value }); };
      const pw = fld('password', t({ ru: 'Пароль приложения', en: 'App password' }));
      const hold = { until: 0 }; // a test result stays on the status line for a while
      const save = btn(t({ ru: 'Сохранить пароль', en: 'Save password' }), async () => {
        const ok = await inv('mail_set_password', { password: pw.value });
        pw.value = '';
        hold.until = Date.now() + 6000;
        show(ok ? t({ ru: 'Пароль сохранён (зашифрован Windows DPAPI)', en: 'Password saved (encrypted with Windows DPAPI)' }) : t({ ru: 'Не удалось сохранить пароль', en: 'Could not save the password' }), ok ? 'ok' : 'bad');
        refresh(true);
      }, 'sm');
      const test = btn(t({ ru: 'Проверить подключение', en: 'Test connection' }), async () => {
        test.disabled = true; hold.until = Date.now() + 60000;
        show(t({ ru: 'Проверяю подключение…', en: 'Testing the connection…' }));
        const r = await inv('mail_test');
        test.disabled = false; hold.until = Date.now() + 8000;
        show(r ? r.message : t({ ru: 'Проверка не удалась', en: 'The test failed' }), r && r.ok ? 'ok' : 'bad');
      }, 'sm');
      blk.append(row(host, port), row(user, box), row(pw, save), row(test), h('div', 'footnote', t({
        ru: 'Gmail, Яндекс и Mail.ru не принимают обычный пароль: создайте «пароль приложения» в настройках безопасности аккаунта и включите доступ по IMAP. Pill подключается только по TLS (порт 993), раз в 2 минуты смотрит заголовки новых писем (отправитель и тема), не читает тексты и вложения и не помечает письма прочитанными. Пароль хранится зашифрованным (Windows DPAPI) и после сохранения больше нигде не показывается. Пустое поле и «Сохранить пароль» стирают пароль.',
        en: 'Gmail, Yandex and Mail.ru do not accept the normal password: create an “app password” in the account security settings and enable IMAP access. Pill connects over TLS only (port 993), looks at the headers of new messages every 2 minutes (sender and subject), never reads bodies or attachments and never marks mail as read. The password is stored encrypted (Windows DPAPI) and is never shown again after saving. An empty field and “Save password” erase it.',
      })));
      const ago = (at) => { const s = Math.max(0, Date.now() / 1000 - at); return s < 60 ? t({ ru: 'только что', en: 'just now' }) : s < 3600 ? Math.floor(s / 60) + t({ ru: ' мин назад', en: ' min ago' }) : Math.floor(s / 3600) + t({ ru: ' ч назад', en: ' h ago' }); };
      var refresh = async (force) => {
        const s = await inv('mail_status');
        if (!s) return;
        pw.placeholder = s.hasPassword ? t({ ru: '••••••• сохранён', en: '••••••• saved' }) : t({ ru: 'Пароль приложения', en: 'App password' });
        if (force !== true && Date.now() < hold.until) return;
        if (off()) return show(t({ ru: 'Выключено', en: 'Off' }));
        if (s.state === 'nosetup') show(s.message, 'warn');
        else if (s.state === 'error') show(s.message, 'bad');
        else if (s.state === 'ok') show(t({ ru: 'Ящик проверен ' + ago(s.at) + ', непрочитанных писем: ' + s.unseen, en: 'Mailbox checked ' + ago(s.at) + ', unread messages: ' + s.unseen }), 'ok');
        else show(t({ ru: 'Подключаюсь…', en: 'Connecting…' }));
      };
      poll(refresh, 3000);
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
        else if (s === 'unsupported') show(t({ ru: 'Windows не даёт этому приложению читать уведомления (обычно нужен пакет приложения). Карточки уведомлений недоступны, переключатель ничего не изменит.', en: 'Windows does not let this app read notifications (it usually needs an app package). Notification cards are unavailable; the switch changes nothing.' }), 'warn');
        else show(t({ ru: 'Windows ещё не спрашивал разрешение. Разрешите в Параметры → Конфиденциальность → Уведомления или нажмите кнопку', en: 'Windows has not been asked yet. Allow it in Settings → Privacy → Notifications or press the button' }), 'warn');
      };
      poll(async () => { const s = await inv('notif_status'); if (s) paint(s.status); }, 3000);
    }
  };
})();
