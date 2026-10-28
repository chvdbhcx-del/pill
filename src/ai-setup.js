// Settings > AI setup (first-run wizard + reopenable page). New file of the Pill app; user request: "при установке предлагай скачать все для ии,
// whisper и локальную модельку, или подключать облачную". settings.js calls window.PillAiSetupPage(page, helpers) from pAiSetup(); Rust side: aisetup.rs.
// Commands: aisetup_status, aisetup_whisper_install, aisetup_whisper_set, aisetup_whisper_remove, aisetup_ollama_download,
// aisetup_ollama_run_installer, aisetup_ollama_pull, aisetup_ai_use, aisetup_cancel, voice_has_key, settings_close, settings_set (through put).
// Event: aisetup:progress {id, item, done, total, phase, message}. Nothing here talks to the network: every download starts from a button
// that shows its source host and size right next to it.
(() => {
  'use strict';
  const T = window.__TAURI__ || {};

  window.PillAiSetupPage = async function (p, X) {
    const { h, card, cap, row, btn, msg, inv, get, put, link, t, go, onLeave } = X;
    const MB = t({ ru: ' МБ', en: ' MB' }), GB = t({ ru: ' ГБ', en: ' GB' });
    const size = (b) => (b >= 1e9 ? (b / 1e9).toFixed(1) + GB : Math.round(b / 1e6) + MB);

    // ---- state shared by the renders
    let S = null;                // aisetup_status
    let selModel = 'small';      // whisper model chip
    let busy = null;             // id of the running job (whisper | ollama-setup | ollama-pull)
    const prog = {};             // last progress event per job id
    const result = {};           // last final event per job id: {ok, text}
    let widgets = {};            // job id -> repaint function of its progress box (rebuilt on every render)
    let alive = true;
    const root = h('div', 'as-scope');
    p.append(root);

    // ---- progress box of one job: bar + text + Cancel, and the result line when it is over
    const itemName = (q) => ({
      engine: t({ ru: 'движок whisper.cpp', en: 'whisper.cpp engine' }), base: t({ ru: 'модель Base', en: 'Base model' }), small: t({ ru: 'модель Small', en: 'Small model' }),
      turbo: t({ ru: 'модель Large-v3-turbo', en: 'Large-v3-turbo model' }), installer: t({ ru: 'установщик Ollama', en: 'Ollama installer' }),
    }[q.item] || q.item || '');
    function progBox(id) {
      const box = h('div', 'pad as-prog');
      const line = h('div', 'as-ptxt'), fill = h('i'), bar = h('div', 'as-bar', [fill]);
      const cancel = btn(t({ ru: 'Отмена', en: 'Cancel' }), () => { cancel.disabled = true; inv('aisetup_cancel'); }, 'sm danger');
      const top = h('div', 'inline'); top.append(line, cancel);
      const res = h('div', 'note');
      box.append(top, bar, res);
      const paint = () => {
        const q = prog[id], running = busy === id;
        top.hidden = bar.hidden = !running;
        cancel.disabled = false;
        const r = result[id];
        res.hidden = running || !r;
        if (!running && r) msg(res, r.text, 'note ' + (r.ok ? 'ok' : 'bad'));
        box.hidden = !running && !r;
        if (!running || !q) return;
        const total = q.total || 0, pc = total > 0 ? Math.min(100, Math.floor((q.done / total) * 100)) : 0;
        const known = total > 0 && q.done <= total * 1.02;
        let txt;
        if (q.phase === 'verify') txt = t({ ru: 'Проверяю SHA-256…', en: 'Verifying SHA-256…' });
        else if (q.phase === 'unpack') txt = t({ ru: 'Распаковываю…', en: 'Unpacking…' });
        else if (q.phase === 'start') txt = t({ ru: 'Запускаю сервер…', en: 'Starting the server…' });
        else if (q.phase === 'pull') txt = (q.item || '') + ' · ' + (q.message || '') + (known && q.done ? ' · ' + pc + ' %' : '');
        else txt = itemName(q) + ' · ' + size(q.done) + (known ? ' / ' + size(total) + ' · ' + pc + ' %' : '');
        line.textContent = txt;
        bar.classList.toggle('ind', !known && q.phase !== 'pull' || ['verify', 'unpack', 'start'].includes(q.phase));
        fill.style.width = known && ['download', 'pull'].includes(q.phase) ? pc + '%' : '0%'; // the .ind class animates the unknown cases
      };
      widgets[id] = paint;
      paint();
      return box;
    }

    // ---- data
    async function load() {
      S = (await inv('aisetup_status')) || S;
      if (S) busy = S.busy || null;
    }
    async function render() {
      if (!alive) return;
      if (!S) await load();
      const sc = document.querySelector('#content'), keep = sc ? sc.scrollTop : 0; // a re-render must not jump to the top
      widgets = {};
      root.replaceChildren();
      if (!S) { root.append(h('div', 'footnote', t({ ru: 'Не удалось прочитать состояние.', en: 'Could not read the state.' }))); return; }
      build();
      if (keep && sc) sc.scrollTop = keep;
    }
    const lock = () => !!busy;

    // ---- the page
    function build() {
      const intro = h('div', 'as-intro');
      intro.append(t({ ru: 'Голос в текст и ИИ-чат можно скачать на этот компьютер или подключить облачный сервис. Ничего не скачивается и не запускается без вашего нажатия; у кнопки указано, откуда и сколько.', en: 'Voice to text and the AI chat can run on this computer or through a cloud service. Nothing is downloaded or started without your click; the button shows the source and size.' }));
      root.append(intro);
      buildVoice();
      buildChat();
      buildFooter();
    }

    // ============================================================ voice to text
    const WQ = {
      base: { name: 'Base', note: { ru: 'Самая быстрая и лёгкая. Русский и редкие слова распознаёт заметно хуже.', en: 'The fastest and lightest. Clearly weaker on Russian and rare words.' } },
      small: { name: 'Small', note: { ru: 'Хороший баланс: русский и английский, работает быстро даже на процессоре. Рекомендуется.', en: 'A good balance: Russian and English, fast even on a CPU. Recommended.' } },
      turbo: { name: 'Large-v3-turbo (q5)', note: { ru: 'Лучшее качество из трёх. Версия для процессора: заметно медленнее Small, фраза в 10 секунд может обрабатываться несколько секунд.', en: 'The best quality of the three. A CPU build: clearly slower than Small, a 10-second phrase can take several seconds.' } },
    };

    function buildVoice() {
      const w = S.whisper;
      cap(root, t({ ru: 'Голос в текст (Whisper)', en: 'Voice to text (Whisper)' }));
      if (!w.models.some((m) => m.id === selModel)) selModel = 'small';

      // ---- local
      const c = card(root);
      const state = w.running ? t({ ru: `работает на 127.0.0.1:${w.running} · модель ${(WQ[w.model] || {}).name || w.model}`, en: `running on 127.0.0.1:${w.running} · model ${(WQ[w.model] || {}).name || w.model}` })
        : w.enabled ? t({ ru: 'включён, но сервер не запущен (см. server.log в папке ai/whisper)', en: 'enabled, but the server is not running (see server.log in the ai/whisper folder)' })
        : t({ ru: 'не установлен или выключен', en: 'not installed or off' });
      const r = row(c, t({ ru: 'Локально, на этом компьютере', en: 'Locally, on this computer' }),
        t({ ru: 'Pill запускает whisper.cpp скрыто, только на 127.0.0.1, и закрывает его вместе с собой. Запись не покидает компьютер, ключ не нужен. ', en: 'Pill runs whisper.cpp hidden, on 127.0.0.1 only, and closes it with itself. The recording stays on this computer and no key is needed. ' }) + t({ ru: 'Сейчас: ', en: 'Now: ' }) + state, null);
      r.sub.className = w.running ? 'ok' : '';
      if (w.engine.installed && w.models.some((m) => m.installed)) {
        const sw = h('label', 'sw'), inp = h('input'); inp.type = 'checkbox'; inp.checked = !!w.running; inp.disabled = lock();
        sw.append(inp, h('i'));
        inp.onchange = async () => {
          inp.disabled = true;
          const res = await inv('aisetup_whisper_set', { on: inp.checked, model: inp.checked && w.models.find((m) => m.id === selModel && m.installed) ? selModel : null });
          if (res && res.ok === false) result.whisper = { ok: false, text: res.error || 'error' };
          await load(); render();
        };
        r.append(h('div', 'c', sw));
      }

      const chips = h('div', 'pad');
      const cl = h('div', 'chips');
      w.models.forEach((m) => {
        const b = h('button', 'chip' + (m.id === selModel ? ' on' : ''), [(WQ[m.id] || {}).name || m.id, h('span', 'as-sz', size(m.size))]); b.type = 'button';
        if (m.installed) b.append(h('span', 'badge free', t({ ru: 'на диске', en: 'on disk' })));
        if (m.id === w.model && w.running) b.append(h('span', 'badge pro', t({ ru: 'используется', en: 'in use' })));
        b.onclick = () => { selModel = m.id; render(); };
        cl.append(b);
      });
      chips.append(cl, h('div', 'note as-mt', t((WQ[selModel] || {}).note || { ru: '', en: '' })));
      c.append(chips);

      // what a click will download
      const m = w.models.find((x) => x.id === selModel);
      const need = [];
      if (!w.engine.installed) need.push({ what: t({ ru: `Движок whisper.cpp ${w.engine.tag} (Windows, процессор)`, en: `whisper.cpp ${w.engine.tag} engine (Windows, CPU)` }), host: w.engine.host, size: w.engine.size });
      if (!m.installed) need.push({ what: t({ ru: `Модель ${(WQ[m.id] || {}).name}`, en: `${(WQ[m.id] || {}).name} model` }), host: m.host, size: m.size });
      const plan = h('div', 'pad');
      if (need.length) {
        const ul = h('ul', 'as-list');
        need.forEach((n) => ul.append(h('li', '', [n.what + ' · ', h('b', '', size(n.size)), ' · ' + n.host])));
        const total = need.reduce((a, n) => a + n.size, 0);
        plan.append(h('div', 'note', t({ ru: 'Будет скачано:', en: 'Will be downloaded:' })), ul);
        const go1 = btn(t({ ru: `Скачать ${size(total)} и включить`, en: `Download ${size(total)} and turn on` }), async () => {
          go1.disabled = true; delete result.whisper;
          const res = await inv('aisetup_whisper_install', { model: selModel });
          if (!res || res.ok === false) { result.whisper = { ok: false, text: (res && res.error) || 'error' }; await load(); render(); return; }
          busy = 'whisper'; prog.whisper = { id: 'whisper', phase: 'download', done: 0, total: 0, item: !w.engine.installed ? 'engine' : m.id }; render();
        }, 'pri');
        go1.disabled = lock();
        const hint = h('span', 'note', t({ ru: 'Файлы проверяются по SHA-256, который зашит в Pill.', en: 'Files are verified against a SHA-256 built into Pill.' }));
        const ln = h('div', 'inline as-mt'); ln.append(go1, hint);
        plan.append(ln);
      } else if (!(w.running && w.model === selModel)) {
        const on = btn(w.running ? t({ ru: 'Переключить на эту модель', en: 'Switch to this model' }) : t({ ru: 'Включить', en: 'Turn on' }), async () => {
          on.disabled = true;
          const res = await inv('aisetup_whisper_set', { on: true, model: selModel });
          result.whisper = res && res.ok ? { ok: true, text: t({ ru: 'Готово: голос в текст использует локальный сервер.', en: 'Done: voice-to-text uses the local server.' }) } : { ok: false, text: (res && res.error) || 'error' };
          await load(); render();
        }, 'pri');
        on.disabled = lock();
        plan.append(on);
      } else {
        plan.append(h('div', 'note ok', t({ ru: 'Всё установлено и работает. Диктовка: Ctrl+Alt+D (клавишу можно сменить во вкладке «Клавиши»).', en: 'Everything is installed and running. Dictate with Ctrl+Alt+D (the key can be changed under Hotkeys).' })));
      }
      c.append(plan, progBox('whisper'));

      const foot = h('div', 'pad');
      foot.append(h('div', 'note', t({ ru: 'Папка: ', en: 'Folder: ' }) + S.dir + '\\whisper · ' + t({ ru: 'порт 8179 (если занят, берётся следующий свободный; ваш собственный сервер на 8178 не затрагивается)', en: 'port 8179 (the next free one if busy; your own server on 8178 is left alone)' })));
      if (w.engine.installed || w.models.some((x) => x.installed)) {
        let armed = 0;
        const rm = btn(t({ ru: 'Удалить с диска', en: 'Delete from disk' }), async () => {
          if (!armed) { armed = 1; rm.textContent = t({ ru: 'Точно удалить?', en: 'Really delete?' }); setTimeout(() => { if (alive && armed === 1) { armed = 0; rm.textContent = t({ ru: 'Удалить с диска', en: 'Delete from disk' }); } }, 3000); return; }
          rm.disabled = true;
          const res = await inv('aisetup_whisper_remove');
          result.whisper = res && res.ok ? { ok: true, text: t({ ru: 'Удалено. Голос в текст выключен.', en: 'Deleted. Voice-to-text is off.' }) } : { ok: false, text: (res && res.error) || 'error' };
          await load(); render();
        }, 'sm danger');
        rm.disabled = lock();
        const l = h('div', 'as-mt'); l.append(rm);
        foot.append(l);
      }
      c.append(foot);

      // ---- cloud
      const cc = card(root);
      const cr = row(cc, t({ ru: 'Через облако', en: 'Through the cloud' }),
        t({ ru: 'Groq и Gemini бесплатны, OpenAI платный. Нужен ключ API; запись уходит на сервер выбранного сервиса только когда вы закончили диктовку.', en: 'Groq and Gemini are free, OpenAI is paid. An API key is needed; the recording goes to the chosen service only when you finish dictating.' }), null);
      cr.append(h('div', 'c', btn(t({ ru: 'Выбрать сервис и ключ', en: 'Choose service and key' }), () => go('voice'))));
      const keyNote = h('span', '');
      cr.sub.append(' ', keyNote);
      inv('voice_has_key').then((k) => { if (k && k.has && alive) { keyNote.textContent = t({ ru: 'Ключ уже сохранён.', en: 'A key is already saved.' }); keyNote.className = 'ok'; } });
    }

    // ============================================================ AI chat
    function buildChat() {
      cap(root, t({ ru: 'ИИ-чат', en: 'AI chat' }));
      const c = card(root);
      const o = S.ollama;
      const re = btn(t({ ru: 'Проверить снова', en: 'Check again' }), async () => { re.disabled = true; await load(); render(); }, 'sm');

      // servers that already answer on this computer
      const up = S.servers.filter((s) => s.up);
      const hd = row(c, t({ ru: 'Локальная модель', en: 'Local model' }),
        up.length ? t({ ru: 'Найдено на этом компьютере: ', en: 'Found on this computer: ' }) + up.map((s) => s.name).join(', ') + '.'
          : t({ ru: 'Запущенных локальных серверов (Ollama, LM Studio, llama.cpp, FreeLLMAPI) не найдено. Запросы к ним идут только на 127.0.0.1.', en: 'No local servers (Ollama, LM Studio, llama.cpp, FreeLLMAPI) found. Requests to them go to 127.0.0.1 only.' }), re);
      hd.sub.className = up.length ? 'ok' : '';

      up.forEach((s) => {
        const usedHere = S.ai.on && S.ai.baseUrl.replace(/\/+$/, '') === s.url;
        const sel = h('select', 'field');
        s.models.forEach((m) => { const op = h('option', '', m); op.value = m; sel.append(op); });
        if (usedHere && s.models.includes(S.ai.model)) sel.value = S.ai.model;
        sel.disabled = !s.models.length;
        const use = btn(usedHere && sel.value === S.ai.model ? t({ ru: 'Используется', en: 'In use' }) : t({ ru: 'Использовать', en: 'Use' }), async () => {
          use.disabled = true;
          const res = await inv('aisetup_ai_use', { server: s.id, model: sel.value });
          result['use:' + s.id] = res && res.ok ? { ok: true, text: t({ ru: 'Чат переключён на ', en: 'The chat now uses ' }) + s.name + ' · ' + sel.value } : { ok: false, text: (res && res.error) || 'error' };
          await load(); render();
        }, 'sm');
        use.disabled = !s.models.length || (usedHere && sel.value === S.ai.model);
        sel.onchange = () => { use.disabled = false; use.textContent = t({ ru: 'Использовать', en: 'Use' }); };
        const ctl = h('div', 'inline'); ctl.append(sel, use);
        const r = row(c, s.name, s.models.length ? s.models.length + t({ ru: ' моделей · ', en: ' models · ' }) + s.url : s.url, ctl);
        const rr = result['use:' + s.id];
        if (rr) r.sub.append(' ', h('span', rr.ok ? 'ok' : 'bad', rr.text));
      });

      // Ollama: install, then models
      if (!o.up) {
        const inst = o.installer;
        const ctl = h('div', 'c');
        if (o.installed) {
          ctl.append(h('span', 'note', t({ ru: 'Ollama установлена, но не запущена: откройте её из меню Пуск.', en: 'Ollama is installed but not running: open it from the Start menu.' })));
        } else if (inst.ready) {
          const run = btn(t({ ru: 'Запустить установщик', en: 'Run the installer' }), async () => {
            const res = await inv('aisetup_ollama_run_installer');
            result['ollama-setup'] = res && res.ok ? { ok: true, text: t({ ru: 'Установщик запущен. Когда он закончит, нажмите «Проверить снова».', en: 'The installer is running. When it finishes, press “Check again”.' }) } : { ok: false, text: (res && res.error) || 'error' };
            render();
          }, 'pri');
          run.disabled = lock();
          ctl.append(run);
        } else {
          const dl = btn(t({ ru: `Скачать установщик (≈ ${size(inst.size)})`, en: `Download the installer (≈ ${size(inst.size)})` }), async () => {
            dl.disabled = true; delete result['ollama-setup'];
            const res = await inv('aisetup_ollama_download');
            if (!res || res.ok === false) { result['ollama-setup'] = { ok: false, text: (res && res.error) || 'error' }; await load(); render(); return; }
            busy = 'ollama-setup'; prog['ollama-setup'] = { id: 'ollama-setup', item: 'installer', phase: 'download', done: 0, total: inst.size }; render();
          }, 'pri');
          dl.disabled = lock();
          ctl.append(dl);
        }
        row(c, t({ ru: 'Ollama: программа для локальных моделей', en: 'Ollama: the app for local models' }),
          o.installed ? t({ ru: 'Официальный установщик уже не нужен.', en: 'The official installer is not needed any more.' })
            : t({ ru: `Официальный установщик с ollama.com (перенаправляет на GitHub), около ${size(inst.size)}. Скачивание его не запускает: запуск отдельной кнопкой, дальше запрос Windows. Хеша для проверки нет: сверьте издателя в окне Windows.`, en: `The official installer from ollama.com (redirects to GitHub), about ${size(inst.size)}. Downloading does not start it: running is a separate button, then the Windows prompt. There is no published hash: check the publisher in the prompt.` }), ctl);
        c.append(progBox('ollama-setup'));
      } else {
        const cat = h('div', 'pad');
        cat.append(h('div', 'note', t({ ru: 'Рекомендуемые небольшие модели. Скачивает сама Ollama (источник ollama.com), Pill только передаёт ей команду и показывает ход.', en: 'Recommended small models. Ollama itself downloads them (source: ollama.com); Pill only sends the command and shows the progress.' })));
        const have = (s) => (S.servers.find((x) => x.id === 'ollama') || { models: [] }).models.some((m) => m === s || m === s + ':latest');
        const ul = h('div', 'as-models');
        o.catalog.forEach((m) => {
          const line = h('div', 'as-mrow');
          line.append(h('b', '', m.name), h('span', 'as-sz', '≈ ' + size(m.size)));
          const ctl = h('span', 'as-fillx');
          if (have(m.name)) {
            ctl.append(h('span', 'badge free', t({ ru: 'уже скачана', en: 'already pulled' })));
          } else {
            const b = btn(t({ ru: 'Скачать', en: 'Download' }), async () => {
              b.disabled = true; delete result['ollama-pull'];
              const res = await inv('aisetup_ollama_pull', { model: m.name });
              if (!res || res.ok === false) { result['ollama-pull'] = { ok: false, text: (res && res.error) || 'error' }; await load(); render(); return; }
              busy = 'ollama-pull'; prog['ollama-pull'] = { id: 'ollama-pull', item: m.name, phase: 'pull', done: 0, total: m.size, message: '' }; render();
            }, 'sm');
            b.disabled = lock();
            ctl.append(b);
          }
          line.append(ctl);
          ul.append(line);
        });
        cat.append(ul);
        c.append(cat, progBox('ollama-pull'));
        const pr = result['ollama-pull'];
        if (pr && pr.ok) {
          const use = btn(t({ ru: 'Использовать ', en: 'Use ' }) + pr.model, async () => {
            use.disabled = true;
            const res = await inv('aisetup_ai_use', { server: 'ollama', model: pr.model });
            result['ollama-pull'] = res && res.ok ? { ok: true, text: t({ ru: 'Чат переключён на ', en: 'The chat now uses ' }) + 'Ollama · ' + pr.model } : { ok: false, text: (res && res.error) || 'error' };
            await load(); render();
          }, 'pri');
          const u = h('div', 'pad'); u.append(use); c.append(u);
        }
      }

      const cc = card(root);
      const cr = row(cc, t({ ru: 'Через облако', en: 'Through the cloud' }),
        t({ ru: 'Любой OpenAI-совместимый сервис (OpenAI, OpenRouter, Groq, свой сервер). Нужны адрес API и ключ; ключ шифруется Windows и хранится только здесь.', en: 'Any OpenAI-compatible service (OpenAI, OpenRouter, Groq, your own server). An API address and a key are needed; the key is encrypted by Windows and stays here.' }), null);
      cr.append(h('div', 'c', btn(t({ ru: 'Настроить облачный ИИ', en: 'Set up a cloud AI' }), () => go('ai'))));
    }

    // ============================================================ footer
    function buildFooter() {
      const c = card(root);
      const done = !!get('aiSetupDone');
      const r = row(c, done ? t({ ru: 'Это окно больше не открывается само при запуске', en: 'This window no longer opens by itself at start' }) : t({ ru: 'Настрою позже', en: 'I will set this up later' }),
        t({ ru: 'Страница остаётся в настройках, в разделе «Установка ИИ»: вернуться можно в любой момент.', en: 'The page stays in Settings under “AI setup”: you can come back at any time.' }), null);
      const b = btn(done ? t({ ru: 'Показывать при запуске', en: 'Show at start again' }) : t({ ru: 'Позже', en: 'Later' }), async () => {
        if (done) { await put({ aiSetupDone: false }); render(); return; }
        await put({ aiSetupDone: true });
        inv('settings_close');
      });
      b.disabled = !done && lock();
      r.append(h('div', 'c', b));
      root.append(h('div', 'footnote', t({ ru: 'Скачанное лежит в ', en: 'Downloads live in ' }) + S.dir + '. ' + t({ ru: 'Источники: github.com (движок), huggingface.co (модели Whisper), ollama.com (установщик Ollama и её модели).', en: 'Sources: github.com (engine), huggingface.co (Whisper models), ollama.com (Ollama installer and its models).' })));
    }

    // ---- events
    let off = null;
    if (T.event && T.event.listen) {
      T.event.listen('aisetup:progress', (e) => {
        const q = e.payload;
        if (!q || !alive) return;
        if (q.phase === 'done' || q.phase === 'error' || q.phase === 'cancelled') {
          busy = null;
          if (q.phase === 'done') {
            if (q.id === 'whisper') result.whisper = { ok: true, text: t({ ru: 'Готово: голос в текст теперь использует локальный сервер ', en: 'Done: voice-to-text now uses the local server ' }) + q.message };
            else if (q.id === 'ollama-setup') result['ollama-setup'] = { ok: true, text: t({ ru: 'Установщик скачан. Нажмите «Запустить установщик».', en: 'The installer is downloaded. Press “Run the installer”.' }) };
            else if (q.id === 'ollama-pull') result['ollama-pull'] = { ok: true, model: q.message, text: t({ ru: 'Модель скачана: ', en: 'Model downloaded: ' }) + q.message };
          } else if (q.phase === 'cancelled') result[q.id] = { ok: false, text: t({ ru: 'Отменено. Недокачанный файл удалён.', en: 'Cancelled. The partial file was deleted.' }) };
          else result[q.id] = { ok: false, text: q.message || 'error' };
          load().then(render);
          return;
        }
        busy = q.id; prog[q.id] = q;
        if (widgets[q.id]) widgets[q.id](); else render();
      }).then((u) => { off = u; });
    }
    onLeave(() => { alive = false; if (off) off(); off = null; });

    await render();
  };
})();
