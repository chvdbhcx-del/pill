// Controls tab (S10 / F08): toggle tiles (Wi-Fi, Ethernet, Bluetooth + device list, dark theme) and power tiles (press twice). Owner: system agent (lane L5).
// Rust: system.rs (sys_state / sys_set / sys_power, event sys:changed). No timers while hidden: polls every 5 s only while the tab is visible.
(() => {
  const T = window.__TAURI__;
  const t = (d) => (window.Pill && Pill.t ? Pill.t(d) : d.en);
  const inv = (c, a) => (window.Pill ? Pill.invoke(c, a) : Promise.resolve(null));
  const svg = (p) => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${p}</svg>`;
  const IC = {
    tab: svg('<path d="M4 7h9M17 7h3M4 17h3M11 17h9M4 12h3M11 12h9"/><circle cx="15" cy="7" r="2"/><circle cx="9" cy="17" r="2"/><circle cx="9" cy="12" r="0.01"/>'),
    wifi: svg('<path d="M2.5 9a14 14 0 0 1 19 0M5.5 12.5a9.5 9.5 0 0 1 13 0M8.8 16a5 5 0 0 1 6.4 0"/><circle cx="12" cy="19.3" r="0.8" fill="currentColor"/>'),
    eth: svg('<rect x="3" y="4" width="18" height="12" rx="3"/><path d="M8 16v3h8v-3M8 8.5v3M12 8.5v3M16 8.5v3"/>'),
    bt: svg('<path d="M7 7.5l10 9-5 4.5V3l5 4.5-10 9"/>'),
    btoff: svg('<path d="M7 7.5l10 9-5 4.5V12M12 3l5 4.5-3 2.7M4 4l16 16"/>'),
    theme: svg('<circle cx="12" cy="12" r="9"/><path d="M12 3v18" /><path d="M12 7a5 5 0 0 1 0 10z" fill="currentColor"/>'),
    list: svg('<path d="M9 7h11M9 12h11M9 17h11"/><circle cx="5" cy="7" r="0.9" fill="currentColor"/><circle cx="5" cy="12" r="0.9" fill="currentColor"/><circle cx="5" cy="17" r="0.9" fill="currentColor"/>'),
    lock: svg('<rect x="5" y="11" width="14" height="9" rx="2.5"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>'),
    sleep: svg('<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>'),
    restart: svg('<path d="M20 12a8 8 0 1 1-2.6-5.9"/><path d="M20 4v4.5h-4.5"/>'),
    power: svg('<path d="M12 3v9"/><path d="M6.3 6.6a8 8 0 1 0 11.4 0"/>'),
  };
  const el = (tag, cls, html) => { const e = document.createElement(tag); if (cls) e.className = cls; if (html != null) e.innerHTML = html; return e; };

  let S = null;                  // last sys_state
  let root, big, pwr, pop = null, btBtn = null;
  let visible = false, rustHidden = false, poll = 0;
  const busy = new Map();        // tile id -> {text, until}: waiting for UAC / a slow call
  const flash = new Map();       // tile id -> {text, until}: short error text
  let armed = null, armT = 0;    // armed power action

  // ---- tiles ----
  const ON = () => ({ ru: 'Включён', en: 'On' }), OFF = () => ({ ru: 'Выключен', en: 'Off' });
  const toggles = () => {
    if (!S) return [];
    const list = [];
    const nic = (id, o, name, ic) => list.push({ id, name, icon: ic, on: o.on, disabled: !o.available, ssid: o.ssid });
    if (S.wifi.available) nic('wifi', S.wifi, { ru: 'Wi-Fi', en: 'Wi-Fi' }, IC.wifi);
    if (S.ethernet.available) nic('ethernet', S.ethernet, { ru: 'Ethernet', en: 'Ethernet' }, IC.eth);
    if (!S.wifi.available && !S.ethernet.available) nic('wifi', S.wifi, { ru: 'Wi-Fi', en: 'Wi-Fi' }, IC.wifi);
    list.push({ id: 'bluetooth', name: { ru: 'Bluetooth', en: 'Bluetooth' }, icon: S.bluetooth.on ? IC.bt : IC.btoff, on: S.bluetooth.on, disabled: !S.bluetooth.available, list: true });
    list.push({ id: 'dark', name: { ru: 'Тёмная тема', en: 'Dark theme' }, icon: IC.theme, on: S.dark, fem: true });
    return list;
  };
  const stateText = (d) => {
    const f = flash.get(d.id), b = busy.get(d.id);
    if (f && f.until > Date.now()) return f.text;
    if (b) return b.text;
    if (d.disabled) return t({ ru: 'Нет адаптера', en: 'No adapter' });
    if (d.on && d.ssid) return d.ssid;
    if (d.fem) return t(d.on ? { ru: 'Включена', en: 'On' } : { ru: 'Выключена', en: 'Off' });
    return t(d.on ? ON() : OFF());
  };

  let bigKey = '';
  function renderBig() {
    const list = toggles();
    const key = list.map((d) => d.id).join();
    if (key !== bigKey) { // tile set changed: rebuild; otherwise update in place so hover / press never gets interrupted
      bigKey = key;
      big.replaceChildren(...list.map((d) => {
        const b = el('button', 'tc-tile tc-big'), ic = el('span', 'tc-ico'), nm = el('span', 'tc-name'), st = el('span', 'tc-state');
        b.dataset.id = d.id; b.type = 'button';
        b.append(ic, nm, st);
        b.onclick = () => toggle(d.id);
        if (d.list) {
          btBtn = el('span', 'tc-list', IC.list);
          btBtn.setAttribute('role', 'button'); btBtn.tabIndex = 0;
          btBtn.onclick = (e) => { e.stopPropagation(); togglePop(); };
          btBtn.onkeydown = (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); e.stopPropagation(); togglePop(); } };
          b.append(btBtn);
        }
        return b;
      }));
      big.dataset.n = list.length;
    }
    list.forEach((d, i) => {
      const b = big.children[i];
      const isBusy = busy.has(d.id);
      b.classList.toggle('on', !!d.on && !d.disabled);
      b.classList.toggle('disabled', !!d.disabled);
      b.classList.toggle('busy', isBusy);
      b.disabled = !!d.disabled || isBusy;
      b.setAttribute('aria-pressed', d.on ? 'true' : 'false');
      b.children[0].innerHTML = d.icon;
      b.children[1].textContent = t(d.name);
      b.children[2].textContent = stateText(d);
      if (btBtn && d.list) btBtn.title = t({ ru: 'Подключённые устройства', en: 'Paired devices' });
    });
    if (S && !S.bluetooth.available) closePop();
  }

  const POWER = [
    { a: 'lock', ic: IC.lock, name: { ru: 'Блокировка', en: 'Lock' } },
    { a: 'sleep', ic: IC.sleep, name: { ru: 'Сон', en: 'Sleep' } },
    { a: 'restart', ic: IC.restart, name: { ru: 'Перезагрузка', en: 'Restart' } },
    { a: 'shutdown', ic: IC.power, name: { ru: 'Выключение', en: 'Shut down' } },
  ];
  function renderPwr() {
    if (!pwr.children.length) {
      pwr.replaceChildren(...POWER.map((p) => {
        const b = el('button', 'tc-tile tc-pw'), ic = el('span', 'tc-pic', p.ic), nm = el('span', 'tc-pname');
        b.type = 'button'; b.dataset.a = p.a;
        b.append(ic, nm);
        b.onclick = () => power(p.a);
        return b;
      }));
    }
    POWER.forEach((p, i) => {
      const b = pwr.children[i];
      b.classList.toggle('armed', armed === p.a);
      b.children[1].textContent = armed === p.a ? t({ ru: 'Нажмите ещё раз', en: 'Press again' }) : t(p.name);
    });
  }
  const render = () => { if (!root) return; renderBig(); renderPwr(); renderPop(); };

  // ---- actions ----
  function note(id, text, ms = 2500) { flash.set(id, { text, until: Date.now() + ms }); render(); setTimeout(render, ms + 20); }

  async function toggle(id) {
    if (!S) return;
    const want = id === 'wifi' ? !S.wifi.on : id === 'ethernet' ? !S.ethernet.on : id === 'bluetooth' ? !S.bluetooth.on : !S.dark;
    if (id === 'ethernet' || id === 'wifi') {
      // adapter admin state needs UAC (the app never elevates): no optimistic flip, wait for the result
      busy.set(id, { text: t({ ru: 'Подтвердите в UAC…', en: 'Confirm in UAC…' }) });
      render();
      const r = await inv('sys_set', { key: id, value: want });
      if (r && r.status === 'needs_uac') { setTimeout(() => { busy.delete(id); refresh(); }, 15000); return; } // sys:changed arrives when netsh ends
      busy.delete(id);
      if (!r || r.status === 'error') note(id, t({ ru: r && /UAC/.test(r.message) ? 'UAC отклонён' : 'Не удалось', en: r && /UAC/.test(r.message) ? 'UAC declined' : 'Failed' }));
      await refresh();
      return;
    }
    // instant feedback, then reconcile with the real state
    if (id === 'bluetooth') S.bluetooth.on = want; else S.dark = want;
    render();
    const r = await inv('sys_set', { key: id, value: want });
    if (!r || r.status === 'error') note(id, t({ ru: 'Не удалось', en: 'Failed' }));
    await refresh();
  }

  async function power(a) {
    clearTimeout(armT);
    const confirmed = armed === a;
    armed = confirmed ? null : a;
    render();
    const r = await inv('sys_power', { action: a, confirmed });
    if (r && r.status === 'confirm') {
      armed = a; render();
      armT = setTimeout(() => { armed = null; render(); }, (r.ms || 3000) + 50);
    } else {
      armed = null; render();
    }
  }

  // ---- Bluetooth device popover ----
  function togglePop() { pop ? closePop() : openPop(); }
  function openPop() {
    if (pop || !S || !S.bluetooth.available) return;
    pop = el('div', 'tc-pop');
    root.append(pop);
    renderPop();
    setTimeout(() => document.addEventListener('mousedown', outside, true), 0);
  }
  function closePop() {
    if (!pop) return;
    window.PillMotion.exit(pop, { base: 'translateX(-50%)', y: -6, scale: 0.97, remove: true }); pop = null; // .tc-pop is centred with translateX(-50%)
    document.removeEventListener('mousedown', outside, true);
  }
  function outside(e) { if (pop && !pop.contains(e.target) && !(btBtn && btBtn.contains(e.target))) closePop(); }
  function renderPop() {
    if (!pop || !S) return;
    const devs = S.bluetooth.devices || [];
    const rows = devs.length
      ? devs.map((d) => {
          const r = el('div', 'tc-dev' + (d.connected ? ' on' : ''));
          r.append(el('i'), el('span', 'n'), el('span', 's'));
          r.children[1].textContent = d.name;
          r.children[2].textContent = d.connected ? (d.battery != null ? d.battery + '%' : t({ ru: 'Подключено', en: 'Connected' })) : '';
          return r;
        })
      : [el('div', 'tc-empty', '')];
    if (!devs.length) rows[0].textContent = t(S.bluetooth.on ? { ru: 'Нет сопряжённых устройств', en: 'No paired devices' } : { ru: 'Bluetooth выключен', en: 'Bluetooth is off' });
    const head = el('div', 'tc-pop-h'); head.textContent = t({ ru: 'Устройства Bluetooth', en: 'Bluetooth devices' });
    const list = el('div', 'tc-pop-l'); list.append(...rows);
    const more = el('button', 'tc-pop-b'); more.type = 'button';
    more.textContent = t({ ru: 'Подключить в настройках Windows', en: 'Connect in Windows settings' });
    more.onclick = () => { closePop(); inv('sys_set', { key: 'bluetooth_settings', value: true }); };
    pop.replaceChildren(head, list, more);
  }

  // ---- state refresh: on show + every 5 s while visible, plus sys:changed ----
  function apply(s) { if (s && s.bluetooth) { S = s; render(); } }
  const refresh = () => inv('sys_state').then(apply);
  function startPoll() { stopPoll(); if (visible && !rustHidden) poll = setInterval(refresh, 5000); }
  function stopPoll() { clearInterval(poll); poll = 0; }

  Pill.register({
    id: 'controls', order: 70,
    title: t({ ru: 'Управление', en: 'Controls' }),
    icon: IC.tab,
    mount(r) {
      root = r;
      root.classList.add('tab-controls');
      big = el('div', 'tc-row tc-bigrow');
      pwr = el('div', 'tc-row tc-pwrrow');
      root.replaceChildren(big, pwr);
      render();
    },
    onShow() { visible = true; render(); refresh(); startPoll(); },
    onHide() { visible = false; stopPoll(); closePop(); armed = null; clearTimeout(armT); render(); },
  });

  const listen = (name, cb) => { if (window.Pill && Pill.on) Pill.on(name, cb); else if (T && T.event) T.event.listen(name, (e) => cb(e.payload)); };
  listen('sys:changed', (s) => { busy.clear(); apply(s); });
  listen('pill:hide', () => { rustHidden = true; stopPoll(); });
  listen('pill:show', () => { rustHidden = false; if (visible) { refresh(); startPoll(); } });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape' && pop) { closePop(); e.stopPropagation(); } }, true);
})();
