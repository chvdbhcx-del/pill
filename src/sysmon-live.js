// System monitor in the pill: CPU / RAM / GPU meters on top of the Controls tab + a warning chip in the collapsed island. New file of the Pill app,
// loaded by pill.html after live-system.js; user request: "давай делать все", item: system monitor. Rust side: src-tauri/src/sysmon.rs.
//  - sysmon:tick {cpu, ram:{usedMb,totalMb}, gpus:[{index,name,util,tempC,memUsedMb,memTotalMb,powerW}]}  every 2 s while the switch is on and the pill is visible
//  - meters: a row injected above the tiles of the Controls tab (tab-controls.js is not touched; tab-controls.css has 260 px stacked, so sysmon-live.css
//    shortens the big tiles by the height of the row while .tab-controls.has-sm is set)
//  - chip: GPU >= 85 C or CPU >= 95 % for 10 s -> Pill.setLive('sysmon') for ~8 s, then a 5 minute cool-down per source; never fires while the island is expanded
// Switch: Settings > Live events > System monitor (live.sysmon, off by default). PRIVACY: only load counters, nothing is stored or sent.
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const { t } = P;
  const inv = (c, a) => P.invoke(c, a);
  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('sysmon-live', n, err); } });
  // Pill.liveOn() treats an unknown key as ON; this switch is opt-in, so read the setting itself
  const enabled = () => !!(P.settings && P.settings['live.sysmon'] !== false);
  const GPU_C = 85, CPU_PCT = 95, RAM_PCT = 90, HOLD_MS = 10000, COOL_MS = 5 * 60000, CHIP_MS = 8000, MAX_GPU = 2;
  const num = (v) => (typeof v === 'number' && isFinite(v) ? v : null);
  const gb = (mb) => (mb / 1024).toFixed(1);

  // ================= meters =================
  const root = document.querySelector('.tab-controls');
  let row = null, D = null, built = '', cells = {};
  if (root) {
    row = document.createElement('div');
    row.className = 'sm-row';
    row.hidden = true;
    root.prepend(row);
  }

  function build(keys) {
    built = keys.join();
    cells = {};
    row.replaceChildren(...keys.map((k) => {
      const m = document.createElement('div');
      m.className = 'sm-m';
      m.innerHTML = '<div class="sm-h"><span class="sm-l"></span><span class="sm-v"></span></div><i class="sm-b"><b></b></i>'; // static markup
      cells[k] = { m, l: m.querySelector('.sm-l'), v: m.querySelector('.sm-v'), b: m.querySelector('.sm-b b') };
      return m;
    }));
  }
  function put(k, label, val, pct, hot, tip) {
    const c = cells[k];
    c.l.textContent = label;
    c.v.textContent = val;
    c.b.style.width = Math.max(0, Math.min(100, pct || 0)) + '%';
    c.m.classList.toggle('hot', !!hot);
    c.m.title = tip || '';
  }

  function draw() {
    if (!row) return;
    const ok = enabled();
    row.hidden = !ok;
    root.classList.toggle('has-sm', ok);
    if (!ok) return;
    const gpus = D && Array.isArray(D.gpus) ? D.gpus.slice(0, MAX_GPU) : []; // ponytail: 2 GPUs fit the row, a third would need a second line
    const keys = ['cpu', 'ram', ...gpus.map((_, i) => 'g' + i)];
    if (keys.join() !== built) build(keys);
    const cpu = D ? num(D.cpu) : null, r = D && D.ram, rp = r && r.totalMb > 0 ? (r.usedMb / r.totalMb) * 100 : null;
    put('cpu', 'CPU', cpu == null ? '–' : Math.round(cpu) + '%', cpu, cpu >= CPU_PCT, t({ ru: 'Процессор', en: 'Processor' }));
    put('ram', 'RAM', rp == null ? '–' : gb(r.usedMb) + ' GB', rp, rp >= RAM_PCT,
      rp == null ? '' : `${gb(r.usedMb)} / ${gb(r.totalMb)} GB (${Math.round(rp)}%)`);
    gpus.forEach((g, i) => {
      const u = num(g.util), c = num(g.tempC);
      const val = (u == null ? '–' : Math.round(u) + '%') + (c == null ? '' : ' · ' + Math.round(c) + '°');
      const tip = [g.name, g.memTotalMb ? `${gb(g.memUsedMb || 0)} / ${gb(g.memTotalMb)} GB` : '', num(g.powerW) != null ? Math.round(g.powerW) + ' W' : ''].filter(Boolean).join(' · ');
      put('g' + i, gpus.length > 1 ? 'GPU ' + g.index : 'GPU', val, u, c >= GPU_C, tip);
    });
  }

  // ================= warning chip =================
  const hotSince = new Map(), shownAt = new Map();
  let lastTick = 0;
  const warnSvg = '<svg viewBox="0 0 24 24" width="17" height="17" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3.5l9.5 16.5h-19z"/><path d="M12 10v4.5M12 17.4v.1"/></svg>';

  function clearChip() { hotSince.clear(); P.setLive('sysmon', null); }

  function watch(p) {
    const now = Date.now();
    if (now - lastTick > 3 * 2000 + 1000) hotSince.clear(); // a gap in the ticks (pill hidden): the hot streak must be continuous
    lastTick = now;
    const cand = [];
    const consider = (key, hot, text) => {
      if (!hot) { hotSince.delete(key); return; }
      if (!hotSince.has(key)) hotSince.set(key, now);
      if (now - hotSince.get(key) >= HOLD_MS && now - (shownAt.get(key) || 0) >= COOL_MS) cand.push({ key, text });
    };
    (Array.isArray(p.gpus) ? p.gpus : []).forEach((g) => {
      const c = num(g.tempC);
      consider('gpu' + g.index, c != null && c >= GPU_C, `GPU ${g.index | 0} · ${Math.round(c)}°C`);
    });
    const cpu = num(p.cpu);
    consider('cpu', cpu != null && cpu >= CPU_PCT, `CPU ${Math.round(cpu)}%`);
    if (!cand.length || P.state === 'expanded') return; // the meters are in front of the user; the chip comes after the island collapses
    const w = cand[0]; // GPUs come first; the others keep waiting (their cool-down has not started)
    shownAt.set(w.key, now);
    P.setLive('sysmon', { priority: 90, ttlMs: CHIP_MS, html: `<div class="lp lv lv-sm"><span class="lv-ico sm-ico">${warnSvg}</span><span class="lv-code">${w.text}</span></div>` });
  }

  // ================= wiring =================
  on('sysmon:tick', (p) => {
    if (!p || !enabled()) return;
    D = p;
    draw();
    watch(p);
  });
  function sync() {
    draw();
    if (!enabled()) { D = null; clearChip(); return; }
    if (!D) inv('sysmon_get').then((p) => { if (p && enabled()) { D = p; draw(); } }); // first picture at once, not after 2 s
  }
  on('settings:changed', () => setTimeout(sync, 0));
  sync();
})();
