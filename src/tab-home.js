// Home tab: player card, volume row, 2x4 tiles, launcher strip + collapsed-pill music widget. Owner: pill agent (media logic from lane L2).
// Rust (media.rs) pushes `media:changed`; the seek bar is interpolated here between events.
(() => {
  const P = window.Pill, t = P.t, ic = P.icon;
  const inv = (c, a) => P.invoke(c, a);
  const tauri = window.__TAURI__;
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
  const fmt = (ms) => { const s = Math.max(0, Math.floor(ms / 1000)); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };

  let st = null;                       // last media state
  let art = { id: 0, url: null };      // cover cache keyed by thumb_id
  let base = { pos: 0, at: 0 };        // position at time `at` (performance.now)
  let vol = { volume: 0, muted: false };
  let shown = false, dragging = false, raf = 0, hidden = false;
  let mode = 'standard', liveKey = '', sessTimer = 0, sessions = [];
  // volume slider: the knob / fill / number glide to the goal (eased, ~150 ms), the system volume is written at once in 1 % steps
  let volGoal = 0, volShown = 0, volRaf = 0, volT = 0, volHold = false, volUserAt = -1e9;
  let volSent = -1, volWant = -1, volBusy = false;
  let el = {};

  const accent = () => (st && st.accent) || '';
  const appName = () => String((st && st.app) || '').replace(/\.exe$/i, '').trim();

  // ---------- settings ----------
  // settings.json: musicPill (title | mini | lyrics; lyrics has no widget yet and shows the title), module.media / module.launcher (Settings > Modules)
  let cfg = {}, wasMedia = true;
  async function loadMode(e) {
    const r = e && e.payload && typeof e.payload === 'object' ? e.payload : await inv('settings_get');
    if (r && typeof r === 'object') cfg = r;
    mode = cfg.musicPill === 'mini' ? 'mini' : 'title';
    applyMods();
    updateLive();
  }
  // a module that is off hides its part of the Home tab; the player module also stops the media events (Rust) and the pill widget
  function applyMods() {
    const media = cfg['module.media'] !== false, launch = cfg['module.launcher'] !== false;
    if (el.root) el.root.style.display = media ? '' : 'none';
    if (el.launch) el.launch.style.display = launch ? '' : 'none';
    if (media === wasMedia) return;
    wasMedia = media;
    if (media) inv('media_state', { known: null }).then(apply);
    else { st = null; render(); }
  }

  // ---------- state intake ----------
  function apply(s) {
    if (!s || cfg['module.media'] === false) return;
    if (s.thumbnail) art = { id: s.thumb_id, url: s.thumbnail };
    else if (s.has_session && s.thumb_id !== art.id) {
      // Cover changed but the event omitted it (we had missed it): fetch once.
      art = { id: s.thumb_id, url: null };
      inv('media_state', { known: null }).then((r) => { if (r && r.thumbnail) { art = { id: r.thumb_id, url: r.thumbnail }; render(); updateLive(); } });
    }
    st = s;
    base = { pos: s.position_ms, at: performance.now() };
    render();
    updateLive();
    if (shown) { clearTimeout(sessTimer); sessTimer = setTimeout(loadSessions, 400); }
  }

  const pos = () => {
    if (!st) return 0;
    const p = st.playing ? base.pos + (performance.now() - base.at) : base.pos;
    return st.duration_ms ? Math.min(p, st.duration_ms) : p;
  };

  // ---------- Home tab ----------
  const TILES = [
    ['area', 'area', { ru: 'Область', en: 'Area' }], ['screen', 'monitor', { ru: 'Весь экран', en: 'Full screen' }],
    ['ocr', 'scantext', { ru: 'Текст с экрана', en: 'Screen text' }], ['qr', 'qr', { ru: 'QR-код', en: 'QR code' }],
    ['picker', 'pipette', { ru: 'Пипетка', en: 'Color picker' }], ['plain', 'eraser', { ru: 'Чистый текст', en: 'Plain text' }],
    ['record', 'video', { ru: 'Запись экрана', en: 'Record screen' }], ['command', 'search', { ru: 'Команда', en: 'Command' }],
  ];
  const cap = (mode) => () => { if (window.PillCapture) window.PillCapture.start(mode); }; // capture-live.js: collapses the pill, then capture_start
  const WORKS = { command: () => P.openCommandBar(), area: cap('area'), screen: cap('screen'), ocr: cap('text'), qr: cap('qr'), picker: cap('color'), record: cap('record_screen') };

  function mount(root) {
    root.classList.add('th-view');
    root.innerHTML = `
      <div class="th-grid">
        <div class="th-player card">
          <div class="th-empty">${ic('note2', { size: 26 })}<span>${esc(t({ ru: 'Сейчас ничего не играет', en: 'Nothing is playing' }))}</span></div>
          <div class="th-art"></div>
          <div class="th-col">
            <button class="th-src" type="button"></button>
            <div class="th-title"></div>
            <div class="th-artist"></div>
            <input class="slider thin th-pos" type="range" min="0" max="0" value="0" aria-label="${esc(t({ ru: 'Перемотка', en: 'Seek' }))}">
            <span class="th-t0">0:00</span><span class="th-t1">0:00</span>
            <div class="th-ctl">
              <button data-th="prev" type="button" title="${esc(t({ ru: 'Назад', en: 'Previous' }))}">${ic('prev', { size: 18 })}</button>
              <button data-th="play_pause" class="th-main" type="button" title="${esc(t({ ru: 'Пауза / пуск', en: 'Play / pause' }))}"></button>
              <button data-th="next" type="button" title="${esc(t({ ru: 'Вперёд', en: 'Next' }))}">${ic('next', { size: 18 })}</button>
            </div>
          </div>
        </div>
        <div class="th-tiles">${TILES.map(([id, i, name]) => {
          const soon = !WORKS[id];
          return `<button class="th-tile card hov${soon ? ' soon' : ''}" data-tile="${id}" type="button"${soon ? ` title="${esc(t({ ru: 'Скоро', en: 'Soon' }))}" aria-disabled="true"` : ''}>${ic(i, { size: 18, sw: 1.6 })}<span>${esc(t(name))}</span></button>`;
        }).join('')}</div>
        <div class="th-vol card">
          <button class="th-mute" type="button" title="${esc(t({ ru: 'Без звука', en: 'Mute' }))}"></button>
          <input class="slider white thin th-v" type="range" min="0" max="100" step="any" value="0" aria-label="${esc(t({ ru: 'Громкость', en: 'Volume' }))}">
          <output class="th-vn">0</output>
          <button class="th-dev" type="button" title="${esc(t({ ru: 'Устройство вывода', en: 'Output device' }))}" aria-haspopup="listbox" aria-expanded="false">${ic('device', { size: 15, sw: 1.6 })}<span>${esc(t({ ru: 'По умолчанию', en: 'Default' }))}</span>${ic('chevron', { size: 14, sw: 1.8 })}</button>
          <div class="th-devmenu" role="listbox"></div>
        </div>
        <div class="th-launch card">
          <div class="th-apps">
            <div class="th-add" role="button" tabindex="0" title="${esc(t({ ru: 'Перетащи сюда программу, папку или файл', en: 'Drop a program, folder or file here' }))}">${ic('plus', { size: 20, sw: 1.5 })}</div>
            <span class="th-hint">${esc(t({ ru: 'Перетащи сюда ярлыки программ, папки или файлы', en: 'Drop program shortcuts, folders or files here' }))}</span>
          </div>
        </div>
      </div>`;
    const q = (s) => root.querySelector(s);
    el = { root: q('.th-player'), art: q('.th-art'), src: q('.th-src'), title: q('.th-title'), artist: q('.th-artist'),
      pos: q('.th-pos'), t0: q('.th-t0'), t1: q('.th-t1'), main: q('.th-main'), mute: q('.th-mute'), v: q('.th-v'), vn: q('.th-vn'),
      dev: q('.th-dev'), devMenu: q('.th-devmenu'), launch: q('.th-launch'), apps: q('.th-apps'), add: q('.th-add') };
    wireDevices();
    wireLauncher();

    root.querySelectorAll('[data-th]').forEach((b) => b.addEventListener('click', () => inv('media_control', { action: b.dataset.th })));
    root.querySelectorAll('[data-tile]').forEach((b) => b.addEventListener('click', () => { const f = WORKS[b.dataset.tile]; if (f) f(); }));
    el.src.addEventListener('click', () => { // several players: cycle through them
      if (sessions.length < 2) return;
      const i = sessions.findIndex((s) => s.selected);
      inv('media_select', { id: sessions[(i + 1) % sessions.length].id });
    });
    el.pos.addEventListener('input', () => { dragging = true; el.t0.textContent = fmt(+el.pos.value); fill(el.pos); });
    el.pos.addEventListener('change', () => {
      dragging = false; sec0 = sec1 = -1;
      base = { pos: +el.pos.value, at: performance.now() };
      inv('media_seek', { ms: Math.round(+el.pos.value) });
    });
    // The thumb is driven here, not by the browser: a click or drag sets the goal under the pointer (short lag, no jitter), the wheel,
    // the keys, mute and outside changes glide. Native mousedown (thumb drag, focus) is turned off; keys and focus are handled below.
    const volAt = (e) => { // pointer x -> 0..100 over the thumb's travel (the 12 px thumb: 6 px inset at each end, in scaled px)
      const r = el.v.getBoundingClientRect(), h = 6 * r.width / (el.v.offsetWidth || r.width || 1);
      return Math.max(0, Math.min(100, ((e.clientX - r.left - h) / Math.max(1, r.width - 2 * h)) * 100));
    };
    el.v.addEventListener('mousedown', (e) => e.preventDefault());
    el.v.addEventListener('pointerdown', (e) => {
      if (e.button) return;
      volHold = true;
      try { el.v.setPointerCapture(e.pointerId); } catch (_) {}
      el.v.focus({ preventScroll: true });
      volUser(volAt(e));
    });
    el.v.addEventListener('pointermove', (e) => { if (volHold) volUser(volAt(e)); });
    for (const ev of ['pointerup', 'pointercancel', 'lostpointercapture']) el.v.addEventListener(ev, () => { volHold = false; });
    el.v.addEventListener('keydown', (e) => {
      const d = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1, PageDown: -10, PageUp: 10 }[e.key];
      const g = d ? volRound(volGoal) + d : e.key === 'Home' ? 0 : e.key === 'End' ? 100 : null;
      if (g === null) return;
      e.preventDefault();
      volUser(g);
    });
    el.mute.addEventListener('click', () => { vol.muted = !vol.muted; volUserAt = performance.now(); drawVol(true); inv('media_mute', { muted: vol.muted }); });
    // mouse wheel over the volume row: 2 % per notch (a touchpad's small deltas add up, 1 % steps), eased like every other change
    let wheelAcc = 0;
    q('.th-vol').addEventListener('wheel', (e) => {
      if (!e.deltaY) return;
      e.preventDefault();
      wheelAcc -= (e.deltaMode === 0 ? e.deltaY / 100 : e.deltaY / 3) * 2; // pixels: 100 per notch; lines: 3 per notch; wheel up = louder
      const steps = Math.trunc(wheelAcc);
      if (!steps) return;
      wheelAcc -= steps;
      volUser(volRound(volGoal) + steps);
    }, { passive: false });
    render();
    drawVol();
    applyMods();
  }

  function fill(r) { r.style.setProperty('--p', (r.max > 0 ? (r.value / r.max) * 100 : 0) + '%'); }

  const volRound = (x) => Math.round(Math.max(0, Math.min(100, x)));
  const volBusyNow = () => volHold || volBusy || performance.now() - volUserAt < 600; // a drag / write / own change in flight: outside readings wait

  /// Show the state of `vol` (icon at once, knob / fill / number glide). `force` = a change of our own (mute click), not an outside reading.
  function drawVol(force) {
    if (!el.v) return;
    el.mute.innerHTML = ic(vol.muted || !vol.volume ? 'mute' : 'volume', { size: 18, sw: 1.6 });
    if (force || !volBusyNow()) volGo(vol.muted ? 0 : vol.volume);
  }

  function volPaint() {
    el.v.value = volShown;
    fill(el.v);
    el.vn.textContent = volRound(volShown);
  }

  /// Move the shown value to `g`: eased (25 ms time constant while dragged, 55 ms otherwise = ~150 ms to settle); at once for the very first
  /// reading, while the panel is hidden and with reduced motion.
  let volInit = false;
  function volGo(g) {
    volGoal = g;
    if (!volInit || !shown || hidden || matchMedia('(prefers-reduced-motion: reduce)').matches) { volInit = true; volShown = g; volPaint(); return; }
    if (!volRaf) { volT = performance.now(); volRaf = requestAnimationFrame(volStep); }
  }
  function volStep(now) {
    volRaf = 0;
    const dt = Math.min(0.05, (now - volT) / 1000);
    volT = now;
    volShown += (volGoal - volShown) * (1 - Math.exp(-dt / (volHold ? 0.025 : 0.055)));
    if (Math.abs(volGoal - volShown) < 0.05) volShown = volGoal;
    volPaint();
    if (volShown !== volGoal) volRaf = requestAnimationFrame(volStep);
  }

  /// The user moved the volume (drag, click, wheel, keys): goal + system write. Muted + louder = unmute.
  function volUser(v) {
    v = Math.max(0, Math.min(100, v));
    const wasMuted = vol.muted;
    vol.muted = false;
    vol.volume = volRound(v);
    volUserAt = performance.now();
    el.mute.innerHTML = ic(vol.volume ? 'volume' : 'mute', { size: 18, sw: 1.6 });
    volGo(v);
    if (wasMuted) inv('media_mute', { muted: false });
    volWant = vol.volume;
    if (!volBusy) volFlush();
  }
  /// One write in flight, the newest value follows it: every 1 % the pointer passes reaches the system, with no pile-up of calls.
  async function volFlush() {
    volBusy = true;
    while (volWant !== volSent) {
      const v = volWant;
      volSent = v;
      try { await inv('media_volume_set', { v }); } catch (_) {}
    }
    volBusy = false;
  }

  // ---------- cover gradient: two colours sampled from the cover (data URL -> canvas is not tainted), cached per cover ----------
  const covCache = new Map();          // key -> ['hsl(..)', 'hsl(..)'] | null (grey / failed: keep the neutral card)
  let covPend = '';
  const covKey = () => art.id + ':' + (art.url ? art.url.length : 0);
  function setCov(c) {
    if (!el.root) return;
    el.root.classList.toggle('th-cov', !!c);
    if (c) { el.root.style.setProperty('--cov-a', c[0]); el.root.style.setProperty('--cov-b', c[1]); }
  }
  // 24x24 sample -> 12 hue buckets weighted by saturation and mid-lightness; a = heaviest bucket, b = heaviest bucket >= 60deg away
  // (else a darkened, hue-shifted a). Near-black / near-white / grey pixels are ignored; lightness is clamped so the gradient stays dark.
  function pickColors(im) {
    const N = 24, cv = document.createElement('canvas'); cv.width = cv.height = N;
    const cx = cv.getContext('2d', { willReadFrequently: true });
    cx.drawImage(im, 0, 0, N, N);
    const d = cx.getImageData(0, 0, N, N).data, B = Array.from({ length: 12 }, () => ({ w: 0, r: 0, g: 0, b: 0 }));
    for (let i = 0; i < d.length; i += 4) {
      if (d[i + 3] < 200) continue;
      const [h, s, l] = rgb2hsl(d[i], d[i + 1], d[i + 2]);
      if (l < .12 || l > .9 || s < .18) continue;
      const w = s * (1 - Math.abs(2 * l - 1)), k = B[Math.min(11, Math.floor(h / 30))];
      k.w += w; k.r += d[i] * w; k.g += d[i + 1] * w; k.b += d[i + 2] * w;
    }
    let ia = 0; B.forEach((k, i) => { if (k.w > B[ia].w) ia = i; });
    if (!B[ia].w) return null;
    let ib = -1;
    B.forEach((k, i) => { const dist = Math.min((i - ia + 12) % 12, (ia - i + 12) % 12); if (dist >= 2 && k.w > B[ia].w * .15 && (ib < 0 || k.w > B[ib].w)) ib = i; });
    const avg = (k) => rgb2hsl(k.r / k.w, k.g / k.w, k.b / k.w);
    const cl = (v, lo, hi) => Math.max(lo, Math.min(hi, v));
    const [ha, sa, la] = avg(B[ia]);
    const a = [ha, cl(sa, .35, .85), cl(la, .32, .6)];
    let b;
    if (ib >= 0) { const [h, s, l] = avg(B[ib]); b = [h, cl(s, .3, .8), cl(l, .22, .5)]; }
    else b = [(ha + 25) % 360, a[1], a[2] * .65];
    return [a, b].map(([h, s, l]) => `hsl(${Math.round(h)}, ${Math.round(s * 100)}%, ${Math.round(l * 100)}%)`);
  }
  function rgb2hsl(r, g, b) {
    r /= 255; g /= 255; b /= 255;
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b), l = (mx + mn) / 2, c = mx - mn;
    if (!c) return [0, 0, l];
    const s = c / (1 - Math.abs(2 * l - 1));
    const h = mx === r ? ((g - b) / c + 6) % 6 : mx === g ? (b - r) / c + 2 : (r - g) / c + 4;
    return [h * 60, s, l];
  }
  function paintCover() {
    if (!el.root || hidden) return;       // hidden window: no work; pill:show repaints
    if (!art.url || !(st && st.has_session)) { setCov(null); return; }
    const key = covKey();
    if (covCache.has(key)) { setCov(covCache.get(key)); return; }
    if (covPend === key) return;
    covPend = key;
    const im = new Image();
    const done = (c) => { if (covPend === key) covPend = ''; if (covCache.size > 40) covCache.clear(); covCache.set(key, c); if (covKey() === key) setCov(c); };
    im.onload = () => { let c = null; try { c = pickColors(im); } catch (e) { /* tainted / decode failure: neutral card */ } done(c); };
    im.onerror = () => done(null);
    im.src = art.url;
  }

  function render() {
    if (!el.root) return;
    const has = !!(st && st.has_session);
    el.root.classList.toggle('th-none', !has);
    if (accent()) el.root.style.setProperty('--th-accent', accent()); else el.root.style.removeProperty('--th-accent');
    paintCover();
    if (!has) return;
    setArt();
    el.src.textContent = appName();
    setText(el.title, st.title || appName() || t({ ru: 'Неизвестно', en: 'Unknown' }));
    setText(el.artist, [st.artist, st.album].filter(Boolean).join(' - '));
    const pl = !!st.playing;
    if (pl !== lastPl || !el.main.firstElementChild) {
      el.main.innerHTML = ic(pl ? 'pause' : 'play', { size: 20 });
      if (lastPl !== null && !rm() && el.main.firstElementChild) el.main.firstElementChild.animate([{ transform: 'scale(.55)', opacity: 0 }, { transform: 'none', opacity: 1 }], { duration: 220, easing: tok('--ease-out-expo', 'ease-out') });
      lastPl = pl;
    }
    el.pos.disabled = !st.duration_ms;
    el.pos.max = st.duration_ms;
    el.t1.textContent = st.duration_ms ? '-' + fmt(st.duration_ms - pos()) : '--:--';
    sec0 = sec1 = -1; // labels were rewritten: let tick() refresh both
    tick();
  }

  // ---------- motion helpers (Web Animations; skipped under reduced motion = root class .rm) ----------
  const rm = () => document.documentElement.classList.contains('rm');
  const tok = (n, fb) => getComputedStyle(document.documentElement).getPropertyValue(n).trim() || fb;
  let lastPl = null, artKey = null;

  // title / artist: quick fade + slide only when the text really changes (render() runs on every media event)
  function setText(node, txt) {
    if (node.textContent === txt) return;
    const had = node.textContent !== '';
    node.textContent = txt;
    if (had && !rm()) node.animate([{ opacity: 0, transform: 'translateY(5px)' }, { opacity: 1, transform: 'none' }], { duration: 220, easing: tok('--ease-out-expo', 'ease-out') });
  }

  // cover: the new image sits on top of the old one (grid stack, see CSS) and fades in over 250 ms; render() runs on every
  // media event, so the DOM is touched only when the cover actually changed
  function setArt() {
    const key = art.url || '';
    if (key === artKey) return;
    const prev = artKey; artKey = key;
    const tpl = document.createElement('template');
    tpl.innerHTML = art.url ? `<img alt="" src="${esc(art.url)}">` : ic('note2', { size: 34, sw: 1.5 });
    const n = tpl.content.firstElementChild;
    const old = [...el.art.children];
    if (prev === null || !old.length || rm()) { el.art.replaceChildren(n); return; }
    el.art.append(n);
    const gone = () => old.forEach((o) => o.remove());
    const a = n.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 250, easing: 'ease-out' });
    a.onfinish = gone; a.oncancel = gone;
  }

  // value + fill every frame (position comes from performance.now, so it is smooth at any refresh rate);
  // the two time labels are written only when their displayed whole second changes
  let sec0 = -1, sec1 = -1;
  function tick() {
    if (!el.pos || !st || dragging) return;
    const p = pos();
    el.pos.value = p; fill(el.pos);
    const a = Math.floor(p / 1000);
    if (a !== sec0) { sec0 = a; el.t0.textContent = fmt(p); }
    if (st.duration_ms) {
      const b = Math.floor((st.duration_ms - p) / 1000);
      if (b !== sec1) { sec1 = b; el.t1.textContent = '-' + fmt(st.duration_ms - p); }
    }
  }

  // seek bar interpolation runs on the shared scheduler (Pill.raf): it stops by itself when paused / hidden
  const seekTick = () => { if (!shown || hidden) return false; tick(); return !!(st && st.has_session && st.playing); };
  function loop() { P.raf(seekTick); }

  async function loadSessions() {
    if (!el.src) return;
    sessions = (await inv('media_sessions')) || [];
    el.src.classList.toggle('multi', sessions.length > 1);
    el.src.title = sessions.length > 1 ? t({ ru: 'Сменить плеер', en: 'Switch player' }) : '';
  }

  async function loadVol() {
    const v = await inv('media_volume_get');
    if (v) { vol = v; volSent = -1; drawVol(); }
  }

  // ---------- collapsed pill widget: cover left, equalizer right (vendor "music" pill); wavy outline takes the cover accent ----------
  function updateLive() {
    // Settings > Live events > Music (and Modules > Media player) off: no widget, no edge reaction
    if (!P.liveOn('music') || cfg['module.media'] === false || !st || !st.has_session) { P.unreact('media'); if (liveKey) { liveKey = ''; P.setLive('media', null); } return; }
    if (st.playing) P.react('media', 'music'); else P.unreact('media'); // edge baseline while a track plays
    if (!st.playing) { if (liveKey) { liveKey = ''; P.setLive('media', null); } return; } // paused: the capsule goes back to the clock + weather
    const key = [mode, st.playing, st.title, st.artist, art.id, accent()].join('|');
    if (key === liveKey) return;
    liveKey = key;
    const eq = `<span class="lp-eq${st.playing ? '' : ' off'}"><i></i><i></i><i></i><i></i></span>`;
    const cover = art.url ? `<img class="lp-cover" alt="" src="${esc(art.url)}">` : '<span class="lp-cover"></span>';
    let inner = cover + eq, w = 250;
    if (mode === 'title') { inner = eq + `<span class="lp-name">${esc(st.title || appName())}</span>`; w = 0; }
    else if (mode === 'mini') {
      inner = `${cover}<span class="lp-name">${esc(st.title || appName())}</span><span class="lp-btns"><button data-th-act="prev">${ic('prev', { size: 14 })}</button><button data-th-act="play_pause">${ic(st.playing ? 'pause' : 'play', { size: 14 })}</button><button data-th-act="next">${ic('next', { size: 14 })}</button></span>`;
      w = 0;
    }
    const style = `--eq:${esc(accent() || 'var(--accent)')};--lpw:${w}px`;
    P.setLive('media', { priority: 50, html: `<div class="lp lp-music" style="${style}">${inner}</div>`, accent: st.playing ? accent() || 'var(--accent)' : '' });
  }

  // Buttons inside the live widget are plain HTML: handle them by delegation, before the pill sees the click.
  document.addEventListener('click', (e) => {
    const b = e.target.closest && e.target.closest('[data-th-act]');
    if (!b) return;
    e.stopPropagation();
    inv('media_control', { action: b.dataset.thAct });
  }, true);

  // ---------- last used tab ----------
  // pill.js persists `lastTab` itself (settings key, written on user tab clicks); the shelf uses this to decide where a file drag opens the island.
  P.lastTabId = () => String((P.settings && P.settings.lastTab) || '');

  // ---------- output device dropdown (under the volume slider) ----------
  let devs = [], devOpen = false;

  const defName = () => { const d = devs.find((x) => x.default); return d ? d.name : ''; };
  function drawDev() {
    if (!el.dev) return;
    el.dev.querySelector('span').textContent = defName() || t({ ru: 'По умолчанию', en: 'Default' });
    el.dev.title = defName() || t({ ru: 'Устройство вывода', en: 'Output device' });
    el.devMenu.replaceChildren(...devs.map((d) => {
      const b = document.createElement('button');
      b.type = 'button'; b.setAttribute('role', 'option'); b.dataset.id = d.id; b.title = d.name;
      b.className = d.default ? 'cur' : '';
      b.innerHTML = `<i class="ck">${d.default ? ic('check', { size: 14, sw: 2.2 }) : ''}</i><span></span>`;
      b.querySelector('span').textContent = d.name;
      return b;
    }));
  }
  async function loadDevs() { devs = (await inv('media_devices')) || []; drawDev(); }
  function setDevMenu(on) {
    devOpen = on && devs.length > 0;
    el.devMenu.classList.toggle('open', devOpen);
    el.dev.setAttribute('aria-expanded', String(devOpen));
  }
  async function pickDev(id) {
    setDevMenu(false);
    const r = await inv('media_set_default_device', { id });
    if (r === 'settings') {
      P.toast({ id: 'dev-settings', icon: 'volume', ttlMs: 4500,
        title: t({ ru: 'Windows не даёт менять устройство из приложения: открыл настройки звука', en: 'Windows does not let apps switch the output device: opened the sound settings' }) });
    }
    await loadDevs();
    loadVol(); // the slider now belongs to the new default device
  }
  function wireDevices() {
    el.dev.addEventListener('click', async () => { if (devOpen) return setDevMenu(false); await loadDevs(); setDevMenu(true); });
    el.devMenu.addEventListener('click', (e) => { const b = e.target.closest('button[data-id]'); if (b) pickDev(b.dataset.id); });
    // outside click / Esc close the menu first (Esc must not collapse the island while the menu is open)
    document.addEventListener('mousedown', (e) => { if (devOpen && !e.target.closest('.th-devmenu, .th-dev')) setDevMenu(false); }, true);
    document.addEventListener('keydown', (e) => { if (devOpen && e.key === 'Escape') { e.stopImmediatePropagation(); setDevMenu(false); } }, true);
  }

  // ---------- launcher row (programs / folders / files): drop to add, click opens, x or right-click removes, drag reorders ----------
  let links = [], suppressClick = false;

  function appNode(l) {
    const n = document.createElement('div');
    n.className = 'th-app' + (l.missing ? ' missing' : '');
    n.dataset.id = l.id; n.tabIndex = 0; n.setAttribute('role', 'button');
    n.title = l.missing ? `${l.title}\n${l.target}\n${t({ ru: '(не найдено)', en: '(not found)' })}` : l.title;
    n.innerHTML = l.icon ? `<img alt="" draggable="false" src="${esc(l.icon)}">` : ic(l.is_dir ? 'folder' : 'file', { size: 26, sw: 1.5 });
    const x = document.createElement('button');
    x.type = 'button'; x.className = 'th-app-x'; x.tabIndex = -1;
    x.title = t({ ru: 'Убрать', en: 'Remove' });
    x.innerHTML = ic('x', { size: 10, sw: 2.4 });
    x.addEventListener('mousedown', (e) => e.stopPropagation());
    x.addEventListener('click', (e) => { e.stopPropagation(); dropLink(l.id, n); });
    n.append(x);
    n.addEventListener('click', () => { if (suppressClick) return; openLink(l); });
    n.addEventListener('keydown', (e) => { if (e.key === 'Enter') openLink(l); else if (e.key === 'Delete') dropLink(l.id, n); });
    n.addEventListener('contextmenu', (e) => { e.preventDefault(); dropLink(l.id, n); });
    n.addEventListener('mousedown', (e) => startReorder(e, n, l));
    return n;
  }

  let seenIds = null; // ids rendered last time: only genuinely new icons pop in, a removal slides the rest closed (FLIP)
  function renderLinks() {
    if (!el.apps) return;
    el.launch.classList.toggle('has', links.length > 0);
    const key = (n) => (n === el.add ? 'add' : n.dataset.id);
    const old = new Map([...el.apps.querySelectorAll('.th-app'), el.add].map((n) => [key(n), n.offsetLeft]));
    el.apps.querySelectorAll('.th-app').forEach((n) => n.remove());
    const nodes = links.map((l) => { const n = appNode(l); if (seenIds && !seenIds.has(l.id)) n.classList.add('in'); return n; });
    el.add.before(...nodes);
    const removed = seenIds && [...seenIds].some((id) => !links.some((l) => l.id === id));
    seenIds = new Set(links.map((l) => l.id));
    if (removed && !rm()) {
      const easing = tok('--ease-out-expo', 'ease-out');
      [...nodes, el.add].forEach((n) => {
        const o = old.get(key(n)), d = o == null ? 0 : o - n.offsetLeft;
        if (d) n.animate([{ transform: `translateX(${d}px)` }, { transform: 'none' }], { duration: 280, easing });
      });
    }
  }
  function setLinks(v) { links = Array.isArray(v) ? v : []; renderLinks(); }
  const loadLinks = () => inv('launcher_list').then(setLinks);

  async function openLink(l) {
    const ok = await inv('launcher_open', { id: l.id });
    if (ok) return P.collapse(); // like the clipboard: the launched program takes over, the island steps back
    P.toast({ id: 'launcher-open', icon: 'folder', ttlMs: 2400, title: t({ ru: 'Не удалось открыть: ', en: 'Could not open: ' }) + l.title });
  }
  function dropLink(id, node) {
    node.classList.add('gone'); // tiny fade, then the row closes up
    setTimeout(() => inv('launcher_remove', { id }).then(loadLinks), 160);
  }

  // Pointer-driven reorder (HTML5 drag and drop is not available while Tauri owns file drops). Threshold 6 px.
  function startReorder(e, node, l) {
    if (e.button !== 0 || e.target.closest('.th-app-x')) return;
    const items = [...el.apps.querySelectorAll('.th-app')];
    const from = items.indexOf(node);
    const z = node.getBoundingClientRect().width / node.offsetWidth || 1; // CSS zoom of the panel
    const pitch = items.length > 1 ? items[1].offsetLeft - items[0].offsetLeft : node.offsetWidth + 12;
    const x0 = e.clientX;
    let dragging = false, to = from;
    const move = (m) => {
      const dx = (m.clientX - x0) / z;
      if (!dragging) { if (Math.abs(dx) < 6) return; dragging = true; node.classList.add('drag'); el.apps.classList.add('reordering'); }
      node.style.transform = `translateX(${dx}px)`;
      to = Math.max(0, Math.min(items.length - 1, Math.round(from + dx / pitch)));
      items.forEach((n, k) => {
        if (n === node) return;
        n.style.transform = from < to && k > from && k <= to ? `translateX(${-pitch}px)` : from > to && k >= to && k < from ? `translateX(${pitch}px)` : '';
      });
    };
    const up = () => {
      document.removeEventListener('mousemove', move); document.removeEventListener('mouseup', up);
      if (!dragging) return;
      suppressClick = true; setTimeout(() => { suppressClick = false; }, 0);
      el.apps.classList.remove('reordering');
      items.forEach((n) => { n.style.transform = ''; n.classList.remove('drag'); });
      if (to !== from) {
        const [m] = links.splice(from, 1); links.splice(to, 0, m); renderLinks(); // optimistic; launcher:changed confirms
        inv('launcher_move', { id: l.id, newIndex: to });
      }
    };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', up);
  }

  // Files dropped on the row (Tauri window drag events carry physical px positions). The shelf listener asks launcherHit()
  // first, so a drop on the row never lands on the shelf as well.
  const hitRow = (pos) => {
    if (!shown || !el.launch || !pos || cfg['module.launcher'] === false) return false;
    const r = el.launch.getBoundingClientRect(), d = window.devicePixelRatio || 1;
    const x = pos.x / d, y = pos.y / d;
    return x >= r.left && x <= r.right && y >= r.top - 10 && y <= r.bottom + 10;
  };
  P.launcherHit = hitRow;

  function wireLauncher() {
    el.add.addEventListener('click', () => P.toast({ id: 'launcher-hint', icon: 'plus', ttlMs: 2600,
      title: t({ ru: 'Перетащи программу, папку или файл на эту строку', en: 'Drag a program, folder or file onto this row' }) }));
    loadLinks();
  }
  if (tauri && tauri.event) {
    const over = (p) => { if (el.launch) el.launch.classList.toggle('over', hitRow(p && p.payload && p.payload.position)); };
    tauri.event.listen('tauri://drag-enter', over);
    tauri.event.listen('tauri://drag-over', over);
    tauri.event.listen('tauri://drag-leave', () => el.launch && el.launch.classList.remove('over'));
    tauri.event.listen('tauri://drag-drop', async (e) => {
      const pl = e.payload || {};
      if (el.launch) el.launch.classList.remove('over');
      if (!pl.paths || !pl.paths.length || !hitRow(pl.position)) return;
      const n = await inv('launcher_add', { paths: pl.paths });
      if (!n) P.toast({ id: 'launcher-add', icon: 'folder', ttlMs: 2200, title: t({ ru: 'Уже в списке или не найдено', en: 'Already there or not found' }) });
      loadLinks();
    });
    tauri.event.listen('launcher:changed', (e) => setLinks(e.payload && e.payload.items));
  }

  // ---------- wiring ----------
  P.register({
    id: 'home', title: t({ ru: 'Главная', en: 'Home' }), order: 0, icon: ic('home'),
    mount,
    onShow() { shown = true; loadSessions(); loadVol(); loadDevs(); loadLinks(); loop(); },
    onHide() { shown = false; cancelAnimationFrame(raf); if (el.devMenu) setDevMenu(false); },
  });

  if (tauri && tauri.event) {
    tauri.event.listen('media:changed', (e) => apply(e.payload));
    // the volume changed somewhere else (keys, the mixer, the wheel over the capsule) while the panel is open: the slider glides there
    tauri.event.listen('live:volume', (e) => {
      const p = e.payload;
      if (!p || !shown || volBusyNow()) return;
      vol = { volume: volRound(p.level), muted: !!p.muted };
      volSent = -1;
      drawVol();
    });
    tauri.event.listen('pill:hide', () => { hidden = true; cancelAnimationFrame(raf); inv('media_pause_updates'); });
    tauri.event.listen('pill:show', () => { hidden = false; inv('media_resume_updates'); loadMode(); paintCover(); loop(); });
    tauri.event.listen('settings:changed', loadMode);
  }
  inv('media_state', { known: null }).then(apply);
  loadMode();
})();
