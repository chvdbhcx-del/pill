// Settings > Voice to text (GateGuard note: new file of the Pill app, loaded by settings.html after settings.js; user request: "делай все оставшееся",
// part: voice-to-text). settings.js calls window.PillVoiceSettingsPage(page, helpers) from its pVoice(); Rust side: src-tauri/src/voice.rs.
// Commands used: settings_set (through put), voice_set_key, voice_has_key, voice_test_mic, voice_test_service, net_features_status.
// Keys never come back to the page: voice_has_key only says whether one is stored (and whether the AI key of the same host is used).
(() => {
  'use strict';
  const T = window.__TAURI__ || {};
  const PROV = [
    { id: 'groq', name: 'Groq · Whisper', free: 1, region: 1, url: 'https://api.groq.com/openai/v1', model: 'whisper-large-v3-turbo', keyUrl: 'https://console.groq.com/keys',
      help: { ru: 'Бесплатно, без карты: до 8 часов речи в день, ответ за секунду. Хорошо понимает русский.', en: 'Free, no card: up to 8 hours of speech a day, answers within a second. Understands Russian well.' } },
    { id: 'gemini', name: 'Google Gemini', free: 1, region: 1, url: 'https://generativelanguage.googleapis.com/v1beta/openai', model: 'gemini-2.5-flash', keyUrl: 'https://aistudio.google.com/apikey',
      help: { ru: 'Бесплатный тариф Google AI Studio. Запись уходит как аудио в модель Gemini, она возвращает текст.', en: 'The free Google AI Studio tier. The recording goes to a Gemini model as audio and it returns the text.' } },
    { id: 'openai', name: 'OpenAI', url: 'https://api.openai.com/v1', model: 'whisper-1', keyUrl: 'https://platform.openai.com/api-keys',
      help: { ru: 'Платный сервис. Модель whisper-1 или gpt-4o-mini-transcribe.', en: 'A paid service. The whisper-1 or gpt-4o-mini-transcribe model.' } },
    { id: 'custom', name: { ru: 'Другой / локальный', en: 'Other / local' }, url: 'http://127.0.0.1:8000/v1', model: '',
      help: { ru: 'Любой сервер с маршрутом …/audio/transcriptions (faster-whisper-server, LocalAI, whisper.cpp). Для 127.0.0.1 ключ не нужен, запись остаётся на компьютере.', en: 'Any server with a …/audio/transcriptions route (faster-whisper-server, LocalAI, whisper.cpp). For 127.0.0.1 no key is needed and the recording stays here.' } },
  ];

  window.PillVoiceSettingsPage = function (p, X) {
    const { h, card, row, sw, seg, btn, msg, inv, get, put, link, t, onLeave } = X;
    const provId = () => { const s = get('voice.provider'); return PROV.some((x) => x.id === s) ? s : 'groq'; };
    let cur = provId();
    const url = () => get('voice.endpoint') || PROV[0].url;

    // ---- switch
    const c = card(p);
    const hk = get('hotkey.voice') !== undefined ? get('hotkey.voice') : 'Ctrl+Alt+D';
    row(c, t({ ru: 'Голос в текст', en: 'Voice to text' }),
      t({ ru: `Выключен по умолчанию. Включённый, он слушает микрофон только пока идёт диктовка: ${hk || 'горячая клавиша не назначена'}, скажи, затем нажми ещё раз или отпусти клавишу (режим ниже; Esc тоже останавливает): текст окажется в буфере. Запись уходит только на адрес ниже и только когда ты её закончил.`,
        en: `Off by default. When on, it listens to the microphone only while you dictate: ${hk || 'no hotkey assigned'}, speak, then press again or release the key (mode below; Esc stops too): the text lands on the clipboard. The recording goes only to the address below, and only when you finish it.` }),
      sw('voice.enabled'));
    row(c, t({ ru: 'Режим горячей клавиши', en: 'Hotkey mode' }),
      t({ ru: 'Удерживание короче 0,35 с считается случайным нажатием: запись отменяется без ошибки и ничего не отправляется', en: 'A hold shorter than 0.35 s counts as an accidental tap: the recording is dropped silently and nothing is sent' }),
      seg([['toggle', t({ ru: 'Нажать и говорить, нажать ещё раз', en: 'Press, speak, press again' })], ['hold', t({ ru: 'Удерживать, пока говорю', en: 'Hold while speaking' })]],
        () => (get('voice.mode') === 'hold' ? 'hold' : 'toggle'), (v) => put({ 'voice.mode': v })));

    // ---- provider chips
    const top = h('div', 'pad');
    const chips = h('div', 'chips');
    const help = h('div', 'note'); help.style.marginTop = '12px';
    const warn = h('div', 'note warn'); warn.style.marginTop = '8px';
    top.append(chips, help, warn); c.append(top);

    // ---- key
    const kr = h('div', 'row block');
    kr.append(h('div', 'l', [h('b', '', t({ ru: 'Ключ API', en: 'API key' })), h('span', '', t({ ru: 'Хранится только на этом компьютере, зашифрован Windows и никому не передаётся. Если для этого адреса уже есть ключ ИИ-помощника, он подойдёт и здесь.', en: 'Stored only on this computer, encrypted by Windows and not shared with anyone. If the AI assistant already has a key for this address, it is used here too.' }))]));
    const key = h('input', 'field'); key.type = 'password'; key.autocomplete = 'off'; key.spellcheck = false; key.setAttribute('aria-label', t({ ru: 'Ключ API', en: 'API key' }));
    const keyMsg = h('div', 'note'); keyMsg.style.marginTop = '8px';
    let ks = { has: false, source: 'none' };
    const paintKey = () => {
      key.placeholder = ks.source === 'voice' ? t({ ru: '•••••••• (ключ сохранён)', en: '•••••••• (key saved)' })
        : ks.source === 'ai' ? t({ ru: '•••••••• (используется ключ ИИ-помощника)', en: '•••••••• (the AI assistant key is used)' })
        : t({ ru: 'не нужен для локальных серверов', en: 'not needed for local servers' });
      rm.hidden = ks.source !== 'voice';
    };
    const refreshKey = async () => { const r = await inv('voice_has_key', { provider: cur }); if (r) ks = r; paintKey(); };
    const rm = btn(t({ ru: 'Удалить', en: 'Remove' }), async () => { await inv('voice_set_key', { provider: cur, key: '' }); await refreshKey(); msg(keyMsg, ''); }, 'danger');
    const save = btn(t({ ru: 'Сохранить', en: 'Save' }), async () => {
      const v = key.value.trim(); if (!v) return;
      const r = await inv('voice_set_key', { provider: cur, key: v }); key.value = '';
      await refreshKey();
      msg(keyMsg, r && r.ok ? t({ ru: 'Ключ сохранён: зашифрован Windows, хранится только здесь.', en: 'Key saved: encrypted by Windows, stored only on this computer.' }) : (r && r.error) || 'error', r && r.ok ? 'note ok' : 'note bad');
    });
    const ki = h('div', 'inline'); ki.append(key, save, rm);
    kr.append(ki, keyMsg); c.append(kr);

    // ---- address + model
    const ar = h('div', 'row block');
    ar.append(h('div', 'l', [h('b', '', t({ ru: 'Адрес API и модель', en: 'API address and model' }))]));
    const urlIn = h('input', 'field'); urlIn.value = url(); urlIn.placeholder = 'https://api.groq.com/openai/v1'; urlIn.spellcheck = false; urlIn.setAttribute('aria-label', t({ ru: 'Адрес API', en: 'API address' }));
    const mdl = h('input', 'field mono'); mdl.value = get('voice.model') || ''; mdl.placeholder = t({ ru: 'имя модели', en: 'model name' }); mdl.spellcheck = false; mdl.setAttribute('aria-label', t({ ru: 'Модель', en: 'Model' }));
    mdl.style.flex = '0 0 38%';
    const il = h('div', 'inline'); il.append(urlIn, mdl); ar.append(il); c.append(ar);

    const hosts = h('div', 'footnote'); p.append(hosts);
    const paintHosts = async () => {
      const s = await inv('net_features_status');
      const f = s && s.features && s.features.find((x) => x.id === 'voice');
      if (!f) return;
      hosts.textContent = t({ ru: 'Запросы уходят только на: ', en: 'Requests go only to: ' }) + (f.hosts.join(', ') || '—') + ' · ' + t({ ru: 'вызовов: ', en: 'calls: ' }) + f.calls + (f.on ? '' : ' · ' + t({ ru: 'сейчас выключено', en: 'currently off' }));
    };

    const drawChips = () => {
      chips.replaceChildren();
      PROV.forEach((x) => {
        const b = h('button', 'chip' + (x.id === cur ? ' on' : ''), [typeof x.name === 'string' ? x.name : t(x.name)]); b.type = 'button';
        if (x.free) b.append(h('span', 'badge free', t({ ru: 'бесплатно', en: 'free' })));
        b.onclick = async () => {
          cur = x.id;
          const patch = { 'voice.provider': x.id };
          if (x.id !== 'custom' || !/^https?:\/\/(127\.0\.0\.1|localhost)/i.test(urlIn.value.trim())) patch['voice.endpoint'] = x.url;
          patch['voice.model'] = x.id === 'custom' ? (get('voice.model') || '') : x.model;
          await put(patch);
          urlIn.value = get('voice.endpoint') || ''; mdl.value = get('voice.model') || '';
          drawChips(); refreshKey(); paintHosts();
        };
        chips.append(b);
      });
      const x = PROV.find((q) => q.id === cur);
      help.replaceChildren(t(x.help));
      if (x.keyUrl) { help.append(' '); help.append(link(t({ ru: 'Получить ключ', en: 'Get a key' }), x.keyUrl)); }
      warn.textContent = x.region ? t({ ru: 'Groq и Gemini доступны не во всех странах: если сервис не отвечает, включи VPN или выбери другой из списка.', en: 'Groq and Gemini are not available in every country: if the service does not answer, turn on a VPN or pick another one from the list.' }) : '';
      warn.hidden = !x.region;
    };
    urlIn.onchange = async () => {
      const v = urlIn.value.trim();
      const m = PROV.find((x) => x.url === v);
      cur = m ? m.id : 'custom';
      await put({ 'voice.endpoint': v, 'voice.provider': cur });
      drawChips(); refreshKey(); paintHosts();
    };
    mdl.onchange = () => put({ 'voice.model': mdl.value.trim() });

    // ---- behaviour
    row(c, t({ ru: 'Язык речи', en: 'Spoken language' }), t({ ru: 'Подсказка сервису: так он реже путает языки', en: 'A hint for the service: it mixes languages up less often' }),
      seg([['auto', t({ ru: 'Авто', en: 'Detect' })], ['ru', 'Русский'], ['en', 'English']], () => get('voice.language') || 'auto', (v) => put({ 'voice.language': v })));
    row(c, t({ ru: 'Вставлять текст сразу', en: 'Paste the text at once' }), t({ ru: 'В окно, где стоял курсор. Текст остаётся в буфере в любом случае. Не работает в окнах, запущенных от администратора', en: 'Into the window where the caret was. The text stays on the clipboard either way. It does not work in windows run as administrator' }), sw('voice.paste'));
    row(c, t({ ru: 'Останавливать при тишине', en: 'Stop on silence' }), t({ ru: 'Запись сама заканчивается через 4 секунды тишины после речи. В любом случае она ограничена 120 секундами', en: 'The recording ends by itself after 4 seconds of silence following speech. It is capped at 120 seconds either way' }), sw('voice.autostop'));

    // ---- tests
    const tc = card(p);
    const mic = h('div', 'pad');
    const meter = h('div'); meter.style.cssText = 'height:6px;border-radius:3px;background:var(--track,#2a292d);overflow:hidden;margin-top:10px';
    const fill = h('div'); fill.style.cssText = 'height:100%;width:0;border-radius:3px;background:var(--green,#58ddab);transition:width .06s linear';
    meter.append(fill);
    const micMsg = h('div', 'note'); micMsg.style.marginTop = '8px';
    const micBtn = btn(t({ ru: 'Проверить микрофон', en: 'Test the microphone' }), async () => {
      micBtn.disabled = true; fill.style.width = '0'; msg(micMsg, t({ ru: 'Говорите 2 секунды…', en: 'Speak for 2 seconds…' }), 'note');
      const r = await inv('voice_test_mic'); micBtn.disabled = false; fill.style.width = '0';
      if (r && r.ok) msg(micMsg, t({ ru: 'Микрофон работает. Запись не сохранена и никуда не отправлена.', en: 'The microphone works. Nothing was saved or sent.' }), 'note ok');
      else msg(micMsg, (r && r.error) || t({ ru: 'Нет ответа', en: 'No answer' }), 'note bad');
    });
    const mh = h('div', 'inline'); mh.append(micBtn, h('span', 'note', t({ ru: 'Записывает 2 секунды и показывает уровень. Ничего не отправляется.', en: 'Records 2 seconds and shows the level. Nothing is sent.' })));
    mic.append(mh, meter, micMsg); tc.append(mic);

    const sv = h('div', 'pad');
    const svMsg = h('div', 'note'); svMsg.style.marginTop = '8px';
    const svBtn = btn(t({ ru: 'Проверить сервис', en: 'Test the service' }), async () => {
      svBtn.disabled = true; msg(svMsg, t({ ru: 'Проверяю…', en: 'Testing…' }), 'note');
      const r = await inv('voice_test_service'); svBtn.disabled = false; paintHosts();
      if (r && r.ok) msg(svMsg, t({ ru: 'Работает', en: 'Works' }) + ' · ' + r.ms + ' ' + t({ ru: 'мс', en: 'ms' }) + ' · ' + (r.host || ''), 'note ok');
      else if (r && r.code === 'off') msg(svMsg, t({ ru: 'Сначала включи «Голос в текст» выше', en: 'Switch “Voice to text” on above first' }), 'note warn');
      else msg(svMsg, (r && r.error) || t({ ru: 'Нет ответа', en: 'No answer' }), 'note bad');
    });
    const sh = h('div', 'inline'); sh.append(svBtn, h('span', 'note', t({ ru: 'Отправляет 1 секунду тишины, чтобы проверить адрес, ключ и модель.', en: 'Sends 1 second of silence to check the address, key and model.' })));
    sv.append(sh, svMsg); tc.append(sv);

    // level meter of the microphone test (events with test:true come from the 2 s test only)
    let off = null;
    if (T.event && T.event.listen) {
      T.event.listen('voice:level', (e) => { const q = e.payload; if (q && q.test) fill.style.width = Math.round((+q.level || 0) * 100) + '%'; }).then((u) => { off = u; });
    }
    onLeave(() => { if (off) off(); off = null; });

    drawChips(); refreshKey(); paintHosts();
  };
})();
