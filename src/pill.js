// Pill shell front-end: window.Pill API, spring-driven notch #body, tab row + header cluster, collapsed live row, event cards, Settings tab.
// Owner: pill agent (lane L1). Contract: CONTRACT.md "Front-end side". Rust side: src-tauri/src/pill.rs.
// Pill.lang ('ru'|'en') and Pill.t({ru, en}) are the shared i18n API for every tab (signature is stable).
(() => {
  'use strict';
  const T = window.__TAURI__;
  const root = document.documentElement;
  const body = document.getElementById('body');
  const liveIn = document.getElementById('live-in');
  const tabsEl = document.getElementById('tabs');
  const hdrEl = document.getElementById('hdr');
  const viewsEl = document.getElementById('views');
  const cardsEl = document.getElementById('cards');
  const waveEl = document.getElementById('wave');
  const panelEl = document.getElementById('panel');
  const icon = window.pillIcon || (() => '');

  // ---- i18n: resolved language is cached synchronously (localStorage) so the first paint is already right ----
  const nav = () => (/^ru/i.test(navigator.language || '') ? 'ru' : 'en');
  const resolveLang = (v) => (v === 'ru' || v === 'en' ? v : nav());
  let lang = (() => { try { const c = localStorage.getItem('pill.lang'); if (c === 'ru' || c === 'en') return c; } catch (e) { /* storage blocked */ } return nav(); })();
  const t = (o) => (o == null ? '' : typeof o === 'string' ? o : o[lang] ?? o.en ?? o.ru ?? '');

  // Sizes in css px at 100 %; keep in sync with pill.css and the frames in pill.rs.
  const CFG = { colH: 30, colMinW: 118, colMaxW: 500, colFrameW: 720, colFrameH: 120, /* collapsed OS frame 720x120 (pill.rs; same width as the expanded one so the island never shifts on a frame resize) */ cardFrameH: 200, /* ... and 720x200 while an event card is shown (pill.rs CARD_FRAME) */ expW: 680, expH: 331, expFrameW: 720, expFrameH: 480, expTop: 8, wavePad: 28, defH: 331, capTop: 8,
                expR: 30, expTopR: 26, K: 380, zeta: 0.78 };

  const S = { size: 100, theme: 'dark', clock24: true, autoHide: false, reduceMotion: false, hideInApps: [], language: 'auto', shape: 'capsule', edgeIdle: true, edgeCones: false, lastTab: '',
              showTime: true, showDate: false, showTemp: true, showSeconds: false, showBattery: true, opacityCol: 0, opacityExp: 0, vpnCountry: '', pinned: false,
              fps: 'auto', edgeIdleFps: 30 /* settings.json: fps = auto | 30 | 60 | 120 | 144 | 240 (cap of the shared rAF loop); edgeIdleFps = 15 | 30 | 60 (calm / sleep edge only) */ };
  const mqTheme = matchMedia('(prefers-color-scheme: light)');
  const mqMotion = matchMedia('(prefers-reduced-motion: reduce)');
  const scale = () => Math.min(2, Math.max(0.8, (+S.size || 100) / 100));
  const reduce = () => !!S.reduceMotion || mqMotion.matches;
  const capsule = () => S.shape === 'capsule';
  const safe = (f, ...a) => { try { return f && f(...a); } catch (e) { console.error(e); } };
  const h = (tag, cls, txt) => { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; };
  const esc = (s) => String(s ?? '').replace(/[&<>"]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));

  async function invoke(cmd, args) {
    if (!(T && T.core)) return null;
    try { return await T.core.invoke(cmd, args); } catch (e) { console.error('invoke', cmd, e); return null; }
  }
  const on = (name, cb) => { if (T && T.event) T.event.listen(name, (e) => safe(cb, e.payload)); };

  let hidden = false;     // window hidden by Rust (guard / auto-hide / tray): no timers, no animation
  let away = false;       // auto-hide: #body slid up
  let hovering = false;   // Rust says the cursor is on the pill
  let typing = false;     // a text field has keyboard focus (pill_activate)
  let frameExp = false;   // the OS window is in the expanded frame
  let chosen = false;     // the user (or code) picked a tab, stop auto-selecting the first one

  // ---- wavy outline along the bottom edge (pure decoration; accent coloured while a live widget asks for it) ----
  const SVGNS = 'http://www.w3.org/2000/svg';
  // ---- ONE shared rAF scheduler: Pill.raf(fn) registers fn(dt, now) and keeps it running while it returns truthy. Paused while the pill is hidden or the window is occluded. ----
  const rafSet = new Set(), rafBuf = []; // rafBuf: per-frame snapshot of rafSet, reused
  let rafId = 0, rafLast = 0;
  let fpsCap = 0; // fpsCap: setting `fps` (30 | 60 | 120 | 144 | 240), 0 = auto = no cap
  const rafWake = () => { if (!rafId && rafSet.size && !hidden && !document.hidden) { rafLast = performance.now(); rafId = requestAnimationFrame(rafStep); } };
  function rafStep(now) {
    rafId = 0;
    if (fpsCap && now - rafLast < 1000 / fpsCap - 1.5) { rafId = requestAnimationFrame(rafStep); return; } // user frame cap: skip this display frame, dt stays measured from the last executed one
    const t0 = Pill.perf ? performance.now() : 0, dt = Math.max(0, Math.min((now - rafLast) / 1000, 1 / 20));
    rafLast = now;
    rafBuf.length = 0; for (const f of rafSet) rafBuf.push(f);
    for (const f of rafBuf) { let keep = false; try { keep = !!f(dt, now); } catch (e) { console.error(e); } if (!keep) rafSet.delete(f); }
    if (Pill.perf) { Pill.perf.n++; Pill.perf.ms += performance.now() - t0; }
    rafWake();
  }
  const raf = (f) => { rafSet.add(f); rafWake(); };
  const rafPause = () => { if (rafId) { cancelAnimationFrame(rafId); rafId = 0; } };
  const syncPaused = () => root.classList.toggle('paused', hidden || document.hidden); // also freezes CSS animations
  document.addEventListener('visibilitychange', () => { syncPaused(); if (document.hidden) rafPause(); else rafWake(); });
  // device-pixel snapping: fractional positions make text soft on a transparent (non ClearType) window
  const dpr = () => window.devicePixelRatio || 1;
  const snapPx = (v) => Math.round(v * dpr()) / dpr();

  // ---- liquid bottom edge (ferrofluid) ----
  // A height field d(x) along the bottom edge (1 sample / 3 px, spring to a target shape + coupling of the deviation from it = damped wave equation),
  // drawn on a canvas strip under the body: black fill down to the curve + a thin outline. Music adds per-band spikes (audio:bands or a synthetic signal).
  const LQ = { dx: 3, span: CFG.expW + 40, strip: 46, up: 4, /* strip 46 = 24 px of wave + room for the expanded droplet (15 + 5 + drip 6 + bob) */ k: 220, c: 900, damp: 9, t: 0, cx: 0, cxs: 0, cxv: 0, want: 0, inn: 0, innV: 0, e: 0,
               drag: false, /* Pill.setDragPull: files are dragged over the island, the edge sags harder + wider under the cursor */ style: 'liquid', react: 'calm', rt: 0, rcol: '', rTimer: 0, acc: 0, bAmp: 14, playing: false, lastH: 0, edge: '#6e6e6e', fill: '#000', override: '', lastAudio: 0, playSince: 0, silent: false, spikeMove: false,
               bx: 0, by: 0, bvx: 0, btg: 0, bnext: 0, bph: 0, bon: false, sizeKey: '', bmT: 0, bhf: 0, bb: 0, bs: 0, bsv: 0, bdp: 0, bdn: 0, bde: 0 };
  LQ.N = Math.ceil(LQ.span / LQ.dx / 2) * 2 + 1; LQ.mid = (LQ.N - 1) / 2;
  LQ.h = new Float32Array(LQ.N); LQ.v = new Float32Array(LQ.N); LQ.tg = new Float32Array(LQ.N); LQ.sp = new Float32Array(LQ.N);
  LQ.calm = new Float32Array(LQ.N); LQ.drop = new Float32Array(LQ.N);
  LQ.bt = new Float32Array(32); LQ.sa = new Float32Array(32); LQ.sv = new Float32Array(32); LQ.bv = new Float32Array(32);
  for (let i = 0; i < LQ.N; i++) { // static parts: tiny wave (amplitude 1.5, slightly inside the edge) and the centre droplet sag
    const x = (i - LQ.mid) * LQ.dx, u = Math.abs(x) / 24;
    LQ.calm[i] = -1 + 1.5 * Math.sin(x / 38 + 1.3) * (0.5 + 0.5 * Math.sin(x / 140 + 0.7));
    LQ.drop[i] = 0 * (u < 1 ? Math.pow(1 - u, 1.8) : 0); // no permanent droplet: the measured real edge has none (it sags under the cursor instead)
  }
  const lq = document.createElement('canvas');
  lq.id = 'liq';
  waveEl.replaceWith(lq);
  const lctx = lq.getContext('2d');
  const smooth = (e) => { e = Math.max(0, Math.min(1, e)); return e * e * (3 - 2 * e); };
  LQ.px = new Float32Array(LQ.N + 2); LQ.py = new Float32Array(LQ.N + 2); LQ.rgb = '110,110,110';
  // Boundary offset (px below the straight bottom edge) of the field value y at x: soft max(y, 0) (the DOM body cannot be cut, so the edge only ever bulges
  // downwards) times a smoothstep taper that reaches exactly 0 with zero slope at x = +-half, i.e. where the straight part meets the corner arcs.
  const liqBound = (y, x, half) => smooth((half - Math.abs(x)) / Math.max(14, Math.min(48, half * 0.3))) * 0.5 * (y + Math.sqrt(y * y + 1.5));
  const pcv = document.createElement('canvas'); pcv.width = pcv.height = 1;
  const pcx = pcv.getContext('2d', { willReadFrequently: true });
  const rgbOf = (c) => { // any CSS colour (rgb(), color(srgb ..)) -> "r,g,b" for gradient stops
    try { pcx.clearRect(0, 0, 1, 1); pcx.fillStyle = '#6e6e6e'; pcx.fillStyle = c; pcx.fillRect(0, 0, 1, 1); const d = pcx.getImageData(0, 0, 1, 1).data; return `${d[0]},${d[1]},${d[2]}`; } catch (e) { return '110,110,110'; }
  };
  function liqSize() {
    const s = scale(), d = dpr(), hh = LQ.strip + LQ.up, key = s + '|' + d;
    if (key === LQ.sizeKey) return; // only the OS frame changed (expand / collapse): the canvas keeps its size, no reallocation / clear
    LQ.sizeKey = key;
    lq.style.width = LQ.span * s + 'px'; lq.style.height = hh * s + 'px';
    lq.style.marginLeft = -LQ.span * s / 2 + 'px'; lq.style.top = `calc(100% - ${LQ.up * s}px)`;
    lq.width = Math.round(LQ.span * s * d); lq.height = Math.round(hh * s * d);
    lctx.setTransform(d * s, 0, 0, d * s, 0, 0);
  }
  function liqColors() {
    const cs = getComputedStyle(root), eff = effReact(), rc = LQ.rcol || REACT[eff].col, accent = cs.getPropertyValue('--wave-c').trim();
    lq.style.color = LQ.override || (eff === 'music' ? accent || rc : rc || accent) || cs.getPropertyValue('--edge').trim() || '#6e6e6e';
    LQ.edge = getComputedStyle(lq).color;
    LQ.rgb = rgbOf(LQ.edge);
    LQ.fill = getComputedStyle(body).backgroundColor;
  }
  const liqRange = () => { // active half width (100 % px) and sample indices; the field is pinned to 0 outside (bottom corners stay solid)
    // the span ends exactly where the straight bottom edge ends and the corner arc starts (inset = the bottom radius as the browser really draws it:
    // CSS scales all four radii down together when they do not fit the box, which happens during the collapse squash)
    const s = scale(), r = Math.max(0, sp.r.x), tp = Math.max(0, sp.t.x), f = Math.min(1, sp.h.x / (tp + r || 1), sp.w.x / (2 * r || 1));
    const inset = Math.max(8, Math.min(40, (r * f) / s)), half = Math.max(0, sp.w.x / s / 2 - inset);
    const n = Math.floor(half / LQ.dx);
    return { half, i0: LQ.mid - n, i1: LQ.mid + n };
  };
  const noise = (t) => Math.sin(t * 1.3) * 0.5 + Math.sin(t * 2.9 + 1) * 0.3 + Math.sin(t * 5.3 + 2) * 0.2;
  // Music spikes also show while another live chip owns the row (e.g. "Агент работает"): real audio frames arrive only while a track plays, so fresh non-silent frames are enough
  const audioLive = () => !LQ.silent && performance.now() - LQ.lastAudio < 1000;
  const musicOn = () => (LQ.playing || audioLive()) && !reduce();
  // Edge reactions (named presets, see Pill.setReaction). Measured on the Character page of the real app: colour, amplitude, wavelength, motion.
  const REACT = {
    calm:      { col: '', kind: 'sine', wl: 170, spd: 25, amp: 1 },                                   // idle: 2 long waves drifting left 20-30 px/s, +-2.5 px (+-3.5 expanded)
    music:     { col: '#d8836a', kind: 'beat', wl: 22, spd: 30 },                                    // coral wave pulsing every 1.1 s, 1.5 -> 6.5 px (audio:bands spikes replace it when they arrive)
    thinking:  { col: '#9e8fe8', kind: 'sine', wl: 64, spd: 46, amp: 1.15, ball: true },             // lavender slow travelling wave + a small black ball hanging under it
    done:      { col: '#7fae45', kind: 'settle', wl: 46, spd: 34, a0: 8, tau: 0.9, ball: 'start' }, // green, irregular, relaxes to nearly flat in ~2.5 s
    attention: { col: '#e8a040', kind: 'zig', wl: 11, amp: 1.2 },                                    // amber dense zig-zag shimmer
    file:      { col: '#5a78a8', kind: 'smile', wl: 52, tau: 0.7 },                                  // steel blue, bumpy at first, then a smooth smile curve
    sleep:     { col: '', kind: 'bowl', noline: true },                                              // no line: a smooth sagging bowl
  };
  const effReact = () => (LQ.react === 'music' && reduce() ? 'calm' : LQ.react); // LQ.react is the winner of the react() resolver below
  const reactAnimating = () => { const r = REACT[effReact()]; return r.kind !== 'bowl' && !((r.kind === 'settle' || r.kind === 'smile') && LQ.rt > 3.5); };
  const pulse = (t) => { const p = (t % 1.1) / 1.1; return p < 0.21 ? Math.sin((p / 0.21) * Math.PI / 2) : p < 0.55 ? Math.cos(((p - 0.21) / 0.34) * Math.PI / 2) : 0; };
  function reactWave(x, half, expd) { // edge offset in px (x from the centre, 100 % px)
    const r = REACT[effReact()], t = LQ.t, base = expd ? 3.5 : 2.5, TAU = 6.2832, hh = half || 1;
    switch (r.kind) {
      case 'bowl': return 5 * (1 - (x / hh) ** 2);
      case 'zig': { const tri = (2 / Math.PI) * Math.asin(Math.sin(((x + 30 * t) / r.wl) * TAU)); return base * r.amp * 0.9 * tri * (0.8 + 0.2 * Math.sin(t * 9 + x / 40)); }
      case 'beat': return (1.5 + 5 * pulse(t)) * (expd ? 0.8 : 1) * Math.sin(((x - r.spd * t) / r.wl) * TAU);
      case 'settle': { const a = r.a0 * Math.exp(-LQ.rt / r.tau) + 1.2; return a * (0.62 * Math.sin(((x + r.spd * t) / r.wl) * TAU) + 0.38 * Math.sin(((x + 0.7 * r.spd * t) / (r.wl * 0.58)) * TAU + 1.7)); }
      case 'smile': { const k = Math.exp(-LQ.rt / r.tau); return 4 * (1 - (x / hh) ** 2) + 5 * k * Math.sin(((x + 30 * t) / r.wl) * TAU) * (0.6 + 0.4 * Math.sin(x / 33)); }
      default: { const A = base * (r.amp || 1), br = 0.8 + 0.2 * Math.sin(t * 0.45); return A * br * (0.62 * Math.sin(((x + r.spd * t) / r.wl) * TAU) + 0.38 * Math.sin(((x + 0.8 * r.spd * t) / (r.wl * 0.58)) * TAU + 2.1)); }
    }
  }
  function liqTargets(rg) { // target shape = idle / reaction wave (+ the cursor bulge)
    const { half, i0, i1 } = rg, dyn = Math.abs(LQ.inn) > 0.002, expd = Pill.state === 'expanded', sig2 = 2 * ((expd ? 50 : 38) * (LQ.drag ? 1.45 : 1)) ** 2;
    const bands = musicOn() && !LQ.silent && performance.now() - LQ.lastAudio < 1000 && effReact() === 'music';
    for (let i = i0; i <= i1; i++) {
      const x = (i - LQ.mid) * LQ.dx, env = smooth((half - Math.abs(x)) / 36);
      let v = (bands ? 0 : reactWave(x, half, expd)) * env - 0.6;
      if (dyn) { const dx = x - LQ.cxs; v += LQ.bAmp * LQ.inn * Math.exp(-(dx * dx) / sig2) * env; } // smooth bell under the cursor (lags it like a spring)
      LQ.tg[i] = v;
    }
  }
  // music spikes: 32 bands (bass centre, treble sides, symmetric) -> ~1 spike / 14 px, each a spring (fast attack, slow release, small overshoot)
  function liqSpikes(dt, rg) {
    const { half, i0, i1 } = rg, now = performance.now(), M = Math.max(4, Math.min(32, Math.floor((2 * half) / 14)));
    const fresh = now - LQ.lastAudio < 1000, on = musicOn() && !LQ.silent && half > 20 && fresh, peak = Pill.state === 'expanded' ? 10 : 14, step = (2 * half) / M;
    let move = false;
    for (let j = 0; j < M; j++) {
      // frequency <-> place: bass in the middle, treble towards both ends; a spike takes the loudest of the bands its slice covers,
      // so the edge shows the real spectrum (a missing frequency = a flat spot, a loud one = a tall spike)
      let b = 0;
      if (on) {
        const d = Math.abs(j + 0.5 - M / 2), d0 = Math.max(0, d - 0.5) / (M / 2), d1 = Math.min(M / 2, d + 0.5) / (M / 2);
        for (let k = Math.floor(d0 * 31), ke = Math.min(31, Math.ceil(d1 * 31)); k <= ke; k++) b = Math.max(b, LQ.bt[k]);
      }
      LQ.bv[j] = b;
    }
    for (let j = 0; j < M; j++) { // the spring
      // ferrofluid never rests: every spike also swells and sinks on its own slow rhythm (0.75..1.15 of its band)
      const pers = 0.75 + 0.4 * (0.5 + 0.5 * Math.sin(now * 0.0013 * (1 + (j % 5) * 0.37) + j * 2.399));
      const b = Math.min(1, LQ.bv[j] * pers), tgt = Math.min(peak, peak * 1.35 * Math.pow(b, 0.8)), a = LQ.sa[j], att = tgt > a;
      const w = att ? 40 : 12, z = att ? 0.55 : 0.8;
      LQ.sv[j] += (w * w * (tgt - a) - 2 * z * w * LQ.sv[j]) * dt;
      LQ.sa[j] = Math.max(0, Math.min(peak * 1.2, a + LQ.sv[j] * dt));
      if (LQ.sa[j] > 0.05 || Math.abs(LQ.sv[j]) > 0.3) move = true; else { LQ.sa[j] = 0; LQ.sv[j] = 0; }
    }
    for (let j = M; j < 32; j++) { LQ.sa[j] = 0; LQ.sv[j] = 0; }
    LQ.sp.fill(0);
    for (let j = 0; j < M; j++) {
      const a = LQ.sa[j];
      if (a <= 0) continue;
      // per-spike width (6..11 px half-width) and tip sharpness (exponent 1.8..3.0): a varied row of cones
      const hw = 6 + 5 * (0.5 + 0.5 * Math.sin(j * 12.9898)), ex = 1.8 + 1.2 * (0.5 + 0.5 * Math.sin(j * 7.31));
      const p = -half + (j + 0.5) * step + step * 0.22 * Math.sin(LQ.t * 0.9 * (1 + (j % 4) * 0.3) + j * 1.7), ia = Math.max(i0, Math.floor(LQ.mid + (p - hw) / LQ.dx)), ib = Math.min(i1, Math.ceil(LQ.mid + (p + hw) / LQ.dx));
      for (let i = ia; i <= ib; i++) { const f = Math.abs((i - LQ.mid) * LQ.dx - p) / hw; if (f < 1) { const v = a * Math.pow(1 - f, ex); if (v > LQ.sp[i]) LQ.sp[i] = v; } }
    }
    if (LQ.inn > 0.01 && S.edgeCones) { // optional ferrofluid cones around the (lagging) cursor: spacing 14 px, gaussian envelope, rise and fall individually
      const ia = Math.max(i0, Math.floor(LQ.mid + (LQ.cxs - 70) / LQ.dx)), ib = Math.min(i1, Math.ceil(LQ.mid + (LQ.cxs + 70) / LQ.dx));
      for (let i = ia; i <= ib; i++) {
        const x = (i - LQ.mid) * LQ.dx, dx = x - LQ.cxs, uu = dx / 14, idx = Math.round(uu), f = Math.abs(uu - idx);
        const v = 12 * LQ.inn * Math.exp(-(dx * dx) / 1400) * Math.pow(Math.max(0, 1 - f / 0.5), 1.5) * (0.55 + 0.45 * Math.sin(LQ.t * 3 + idx * 1.7)) * smooth((half - Math.abs(x)) / 24);
        if (v > LQ.sp[i]) LQ.sp[i] = v;
      }
    }
    LQ.spikeMove = move;
    return move;
  }
  // edge styles other than 'liquid' (setting `character`): glow = soft light under the island, none = nothing (canvas hidden); tail / legs = nothing here (characters.js)
  function liqDrawAlt(rg) {
    const { half } = rg, o = LQ.span / 2, up = LQ.up, r = effReact(), t = LQ.t, rt = LQ.rt, col = LQ.edge;
    lctx.clearRect(0, 0, LQ.span, LQ.strip + up);
    if (LQ.style === 'none' || LQ.style === 'tail' || LQ.style === 'legs' || half < 20) return; // tail / legs are drawn by characters.js on its own overlay
    if (LQ.style === 'glow') {
      const op = r === 'music' ? 0.25 + 0.45 * pulse(t) : r === 'thinking' ? 0.22 + 0.18 * Math.sin(t * 1.6) : r === 'done' ? 0.05 + 0.6 * Math.exp(-rt / 1.2)
        : r === 'attention' ? 0.25 + 0.35 * (Math.sin(t * 10) > 0 ? 1 : 0.4) : r === 'file' ? 0.35 : 0.06;
      lctx.save();
      lctx.globalAlpha = Math.max(0, Math.min(1, op)); lctx.fillStyle = col; lctx.shadowColor = col; lctx.shadowBlur = 10;
      lctx.beginPath(); lctx.ellipse(o, up + 1, half * 0.7, 3, 0, 0, 6.2832); lctx.fill();
      lctx.restore();
      lctx.clearRect(0, 0, LQ.span, up); // the light stays below the island
    }
  }
  function liqDraw(rg) {
    if (LQ.style !== 'liquid') { liqDrawAlt(rg); return; }
    const { i0, i1, half } = rg, o = LQ.span / 2, up = LQ.up;
    lctx.clearRect(0, 0, LQ.span, LQ.strip + up);
    if (i1 <= i0) return;
    // The wave IS the body's bottom edge: boundary offset below the straight bottom (canvas y = up), always >= 0 (the black body is a DOM box and cannot be cut),
    // tapered to exactly 0 (zero slope) at both ends of the straight part, where the corner arcs take over. Index k: 0 = left end, 1..n-2 = samples i0..i1, n-1 = right end.
    const n = i1 - i0 + 3, PX = LQ.px, PY = LQ.py;
    PX[0] = -half; PY[0] = 0;
    for (let i = i0; i <= i1; i++) { const x = (i - LQ.mid) * LQ.dx, k = i - i0 + 1; PX[k] = x; PY[k] = Math.min(24, liqBound(LQ.h[i] + LQ.sp[i], x, half)); }
    PX[n - 1] = half; PY[n - 1] = 0;
    lctx.beginPath();
    lctx.moveTo(o + PX[0], up - 1);
    for (let k = 0; k < n; k++) lctx.lineTo(o + PX[k], up + PY[k]);
    lctx.lineTo(o + PX[n - 1], up - 1);
    lctx.closePath();
    lctx.fillStyle = LQ.fill; lctx.fill();
    const rp = REACT[effReact()];
    lctx.beginPath();
    for (let k = 0; k < n; k++) (k ? lctx.lineTo(o + PX[k], up + PY[k]) : lctx.moveTo(o + PX[0], up + PY[0])); // the very same polyline as the fill edge
    lctx.lineJoin = 'round'; lctx.lineCap = 'round';
    const rgb = rp.noline ? '30,30,30' : LQ.rgb, fd = 0.15, gk = `${half}|${rgb}`; // the stroke dissolves over the last 15 % at both ends; the gradient is rebuilt only when its span / colour changes
    if (LQ.grKey !== gk) {
      const gr = lctx.createLinearGradient(o - half, 0, o + half, 0);
      gr.addColorStop(0, `rgba(${rgb},0)`); gr.addColorStop(fd * 0.5, `rgba(${rgb},0.3)`); gr.addColorStop(fd, `rgba(${rgb},1)`);
      gr.addColorStop(1 - fd, `rgba(${rgb},1)`); gr.addColorStop(1 - fd * 0.5, `rgba(${rgb},0.3)`); gr.addColorStop(1, `rgba(${rgb},0)`);
      LQ.gr = gr; LQ.grKey = gk;
    }
    lctx.strokeStyle = LQ.gr;
    if (rp.noline) { lctx.lineWidth = 1.2; lctx.stroke(); } // sleep: just a dark bowl edge
    else {
      // glow: two wider, translucent passes under the line (cheaper than shadowBlur every frame) so the edge also reads on light / grey wallpapers
      if (effReact() !== 'calm') { // calm has only the dark hairline: no halo (it would read as a smudge on grey)
        lctx.globalAlpha = 0.16; lctx.lineWidth = 8; lctx.stroke();
        lctx.globalAlpha = 0.32; lctx.lineWidth = 4.5; lctx.stroke();
        lctx.globalAlpha = 1;
      }
      lctx.lineWidth = 2;
      if (LQ.playing && reduce()) { lctx.shadowColor = LQ.edge; lctx.shadowBlur = 5; } // reduced motion: only a gentle static glow
      lctx.stroke();
      lctx.shadowBlur = 0;
    }
    if (rp.ball && (rp.ball !== 'start' || LQ.rt < 2.6)) { // the small black ball hanging under the line (AI thinking / done); position = ballStep() state (reduced motion: centre, static)
      const kf = Math.max(1, Math.min(n - 2, LQ.bx / LQ.dx + LQ.mid - i0 + 1)), k0 = Math.floor(kf), yl = PY[k0] + (PY[Math.min(n - 1, k0 + 1)] - PY[k0]) * (kf - k0); // continuous x, line height interpolated (no 3 px sample stepping)
      const bb = LQ.bb, r0 = 3.5 + 1.3 * bb, ry = Math.max(0.75 * r0, r0 + LQ.bs), rx = r0 * Math.sqrt(r0 / ry); // expanded: bigger, hangs lower, drip stretch bs (ellipse, volume-ish constant)
      lctx.fillStyle = LQ.fill; lctx.beginPath(); lctx.ellipse(o + LQ.bx, up + yl + 11 + 4 * bb + LQ.by + (ry - r0) * 0.5, rx, ry, 0, 0, 6.2832); lctx.fill();
    }
  }
  // Inertial droplet: x follows a slowly changing random target through a damped spring (w 1.6 rad/s, zeta 0.9: it accelerates and eases like a drop with mass),
  // y = low-frequency bob + a slight sag while it moves. All dt-based. hf = half span of the line (css px). Reset (centre, at rest) whenever the reaction has no ball.
  function ballStep(dt, hf) {
    const rp = REACT[effReact()];
    if (!rp.ball || (rp.ball === 'start' && LQ.rt >= 2.6) || reduce()) { LQ.bon = false; LQ.bx = 0; LQ.bvx = 0; LQ.by = 0; LQ.bb = 0; LQ.bs = 0; LQ.bsv = 0; LQ.bdp = 0; LQ.bhf = 0; return; }
    const ex = Pill.state === 'expanded' ? 1 : 0;
    if (!LQ.bon) { LQ.bon = true; LQ.bx = 0; LQ.bvx = 0; LQ.btg = 0; LQ.bnext = LQ.t + 1.2; LQ.bph = Math.random() * 6.2832; LQ.bb = ex; LQ.bs = 0; LQ.bsv = 0; LQ.bdp = 0; LQ.bdn = LQ.t + 2 + Math.random() * 2; LQ.bhf = hf; }
    if (hf < 8) return; // squashed to nothing (collapse): keep the state, resume when the edge is back
    if (LQ.bhf > 0 && hf !== LQ.bhf) { const k = hf / LQ.bhf; LQ.bx *= k; LQ.btg *= k; LQ.bvx *= k; } // the edge grew / shrank: keep the relative position, glide with it
    LQ.bhf = hf;
    LQ.bb += (ex - LQ.bb) * (1 - Math.exp(-dt * 3.5)); // collapsed <-> expanded regime blend (radius, y offset, speed, spring, target reach)
    const b = LQ.bb, lerp = (a, c) => a + (c - a) * b;
    if (LQ.t >= LQ.bnext) { // new target every 1.6-3.2 s (expanded 2.2-4.2 s), within +-65 % (expanded +-88 %) of the half span, never closer than 18 % to the previous one
      const reach = lerp(0.65, 0.88);
      let g = LQ.btg;
      for (let tries = 0; tries < 6 && Math.abs(g - LQ.btg) < 0.18 * hf; tries++) g = (Math.random() * 2 - 1) * reach * hf;
      if (Math.abs(g - LQ.btg) < 0.18 * hf) g = LQ.btg > 0 ? -0.5 * hf : 0.5 * hf;
      LQ.btg = g; LQ.bnext = LQ.t + lerp(1.6, 2.2) + Math.random() * lerp(1.6, 2);
    }
    // collapsed: w 1.6, v <= 45 px/s; expanded: slower spring (w 1.2) over a ~3x longer edge, v <= 90 px/s (same tanh limit)
    const W = lerp(1.6, 1.2), Z = 0.9, vm = lerp(45, 90), n = Math.max(1, Math.ceil(dt * 60)), sd = dt / n, lim = lerp(0.8, 0.94) * hf;
    for (let s = 0; s < n; s++) {
      LQ.bvx += (W * W * (LQ.btg - LQ.bx) - 2 * Z * W * LQ.bvx) * sd;
      LQ.bvx = vm * Math.tanh(LQ.bvx / vm); // soft speed limit
      LQ.bx += LQ.bvx * sd;
    }
    LQ.bx = Math.max(-lim, Math.min(lim, LQ.bx));
    // drip (expanded only): every 4-8 s stretches down ~0.8 s (slow, overdamped), then releases with a damped bounce (w 14, zeta 0.35)
    if (LQ.bdp === 0 && b > 0.6 && LQ.t >= LQ.bdn) { LQ.bdp = 1; LQ.bde = LQ.t + 0.8; }
    else if (LQ.bdp === 1 && LQ.t >= LQ.bde) { LQ.bdp = 2; LQ.bde = LQ.t + 1.6; }
    else if (LQ.bdp === 2 && LQ.t >= LQ.bde && Math.abs(LQ.bs) < 0.05 && Math.abs(LQ.bsv) < 0.2) { LQ.bdp = 0; LQ.bs = 0; LQ.bsv = 0; LQ.bdn = LQ.t + 4 + Math.random() * 4; }
    const dw = LQ.bdp === 1 ? 7 : 14, dz = LQ.bdp === 1 ? 0.9 : 0.35, dtg = LQ.bdp === 1 ? 4 : 0;
    for (let s = 0; s < n; s++) { LQ.bsv += (dw * dw * (dtg - LQ.bs) - 2 * dz * dw * LQ.bsv) * sd; LQ.bs += LQ.bsv * sd; }
    const t = LQ.t + LQ.bph;
    LQ.by = 3 * (0.6 * Math.sin(1.1 * t) + 0.4 * Math.sin(1.9 * t + 1.3)) + Math.min(2.5, Math.abs(LQ.bvx) * 0.06);
  }
  function liqStatic() { // calm frame, no loop
    const rg = liqRange();
    for (let i = 0; i < LQ.N; i++) { LQ.h[i] = 0; LQ.v[i] = 0; LQ.sp[i] = 0; }
    LQ.sa.fill(0); LQ.sv.fill(0);
    liqTargets(rg);
    for (let i = rg.i0; i <= rg.i1; i++) LQ.h[i] = LQ.tg[i];
    ballStep(0, (rg.i1 - rg.i0) * LQ.dx / 2);
    liqDraw(rg);
  }
  function liqImpulse(d) { // body height changed by d px (100 %): the edge lags behind (sags when the body moves up) and ripples
    const rg = liqRange(), g = 2.2;
    for (let i = rg.i0; i <= rg.i1; i++) {
      const x = (i - LQ.mid) * LQ.dx, prof = Math.max(0, 1 - (x / (rg.half || 1)) ** 2);
      LQ.v[i] += (-d + 0.3 * Math.abs(d)) * g * prof;
    }
  }
  let idleFps = 30; // setting `edgeIdleFps` (15 | 30 | 60): the only legitimate throttle of the edge loop
  const idleGap = () => { const r = effReact(); return r === 'calm' || r === 'sleep' ? 0.92 / idleFps : 0; }; // min seconds between edge frames; 0 = every display frame
  // returns true while something still moves
  function liqStep(dt) {
    if (reduce()) { liqStatic(); return false; } // reduced motion: one calm static frame, no loop
    if (LQ.style !== 'liquid') { // tail / glow / none: no wave physics; throttled to `edgeIdleFps` ONLY in calm / sleep, every other reaction runs at the display rate; stops once static
      LQ.acc += dt;
      if (LQ.acc < idleGap() && !springBusy) return true;
      dt = Math.min(LQ.acc, 0.05); LQ.acc = 0; LQ.t += dt; LQ.rt += dt;
      liqDraw(liqRange());
      const r = effReact();
      return LQ.style === 'glow' && r !== 'calm' && r !== 'sleep' && !((r === 'done' || r === 'file') && LQ.rt > 4);
    }
    LQ.acc += dt;
    const busy = LQ.want > 0 || LQ.inn !== 0 || springBusy || LQ.spikeMove || LQ.e > 0.08;
    if (!busy && LQ.acc < idleGap()) return true; // only the calm / sleep drift is throttled (edgeIdleFps); reactions and the cursor pull run at full rate
    dt = Math.min(LQ.acc, 1 / 20); LQ.acc = 0;
    LQ.t += dt; LQ.rt += dt;
    LQ.innV += (220 * (LQ.want - LQ.inn) - 2 * (LQ.want > LQ.inn ? 0.55 : 0.4) * 14.8 * LQ.innV) * dt; // rises fast, releases with a damped oscillation
    LQ.inn += LQ.innV * dt;
    if (LQ.want === 0 && Math.abs(LQ.inn) < 0.004 && Math.abs(LQ.innV) < 0.05) { LQ.inn = 0; LQ.innV = 0; }
    LQ.cxv += (196 * (LQ.cx - LQ.cxs) - 2 * 0.8 * 14 * LQ.cxv) * dt; LQ.cxs += LQ.cxv * dt; // cursor follows with spring lag
    const rg = liqRange(), { i0, i1 } = rg;
    liqTargets(rg);
    const n = Math.max(1, Math.ceil(dt * 120)), sd = dt / n, df = Math.exp(-LQ.damp * sd), { h, v, tg } = LQ;
    for (let s = 0; s < n; s++) {
      for (let i = i0 + 1; i < i1; i++) v[i] = (v[i] + (LQ.k * (tg[i] - h[i]) + LQ.c * ((h[i - 1] - tg[i - 1]) + (h[i + 1] - tg[i + 1]) - 2 * (h[i] - tg[i]))) * sd) * df;
      for (let i = i0 + 1; i < i1; i++) { h[i] += v[i] * sd; if (h[i] > 18) { h[i] = 18; v[i] = 0; } }
    }
    for (let i = 0; i < LQ.N; i++) if (i <= i0 || i >= i1) { h[i] = 0; v[i] = 0; }
    let e = 0;
    for (let i = i0 + 1; i < i1; i++) e = Math.max(e, Math.abs(v[i]) * 0.05, Math.abs(h[i] - tg[i]));
    LQ.e = e;
    const sm = liqSpikes(dt, rg);
    ballStep(dt, (i1 - i0) * LQ.dx / 2);
    const moving = e > 0.08 || LQ.want > 0 || LQ.inn !== 0 || sm || (musicOn() && !LQ.silent) || (!reduce() && ((S.edgeIdle !== false && effReact() === 'calm') || (effReact() !== 'calm' && reactAnimating())));
    if (!moving) for (let i = i0 + 1; i < i1; i++) { h[i] = tg[i]; v[i] = 0; }
    liqDraw(rg);
    return moving;
  }
  function liqPlaying(on) {
    on = !!on;
    if (on && !LQ.playing) { LQ.playSince = performance.now(); LQ.silent = false; }
    LQ.playing = on;
    liqColors();
    if (on || LQ.sa.some((x) => x > 0) || (!reduce() && S.edgeIdle !== false)) kick(); else if (!rafSet.has(tick)) liqStatic();
  }
  function setBands(p) { // event audio:bands { b: [32 floats 0..1, low -> high], rms }
    if (!p || !Array.isArray(p.b)) return;
    let sum = 0;
    LQ.bm || (LQ.bm = new Float32Array(32).fill(0.2));
    // time-based decay: 0.993 per 33 ms (the old ~30 Hz event), whatever the event rate; dt from the payload `t` (ms) when present, else from the local clock
    const nowMs = performance.now(), srcT = +p.t, hasT = isFinite(srcT) && p.t != null, prevT = hasT ? LQ.bmT : LQ.lastAudio;
    let dtMs = hasT ? srcT - prevT : nowMs - prevT;
    dtMs = !(prevT > 0) || !(dtMs > 0) ? 33 : Math.min(dtMs, 250);
    LQ.bmT = hasT ? srcT : nowMs;
    const kd = Math.pow(0.993, dtMs / 33);
    // Band values are dB (0..1 = -60..0 dB), so most bands sit high all the time. Level = distance below ONE running peak shared by all bands, seen through a
    // ~23 dB window: the frequencies that are really loud reach the top, quiet ones stay flat, and a few dB of beat movement is a visible swing.
    // The tilt (+6 dB at the top) makes up for music's natural treble roll-off.
    const W = 0.38, adj = LQ.adj || (LQ.adj = new Float32Array(32));
    let mx = 0;
    for (let k = 0; k < 32; k++) { const raw = Math.max(0, Math.min(1, +p.b[k] || 0)); sum += raw; adj[k] = raw + 0.1 * k / 31; mx = Math.max(mx, adj[k]); }
    LQ.gpk = Math.max(mx, (LQ.gpk || 0.5) * kd);
    const lo = Math.max(0, LQ.gpk - W);
    LQ.sl || (LQ.sl = new Float32Array(32).fill(-1)); LQ.ad || (LQ.ad = new Float32Array(32).fill(0.05));
    const aSlow = 1 - Math.exp(-dtMs / 700), aDev = 1 - Math.exp(-dtMs / 2500);
    for (let k = 0; k < 32; k++) {
      const raw = Math.max(0, Math.min(1, +p.b[k] || 0));
      if (LQ.sl[k] < 0) LQ.sl[k] = raw;
      // presence = is this frequency really in the music (shared dB window above); z = how far the band is above / below its own recent level, in units of its own usual swing.
      // The spike sits at mid height when the band is steady, shoots up on every rise of THAT band and sinks on every fall, so each frequency moves with its own part of the music.
      const presence = Math.max(0, Math.min(1, (adj[k] - lo) / W)), dev = raw - LQ.sl[k];
      LQ.sl[k] += dev * aSlow;
      LQ.ad[k] += (Math.abs(dev) - LQ.ad[k]) * aDev;
      const z = dev / Math.max(0.02, LQ.ad[k] * 2);
      LQ.bt[k] = Math.min(1, presence * (0.25 + 0.75 * Math.max(0, Math.min(1.2, 0.5 + 0.6 * z))));
    }
    LQ.silent = sum < 0.05 && !(p.rms > 0.01);
    LQ.lastAudio = performance.now();
    if (!LQ.silent) { const e = rx.get('audio'); if (e) e.until = Date.now() + 2500; else react('audio', 'music', { ttlMs: 2500 }); } // real sound = music baseline; the resolver timer re-checks `until`
    if (musicOn()) kick();
  }
  // current translateY of #body (px): its CSS transition (var(--dur-slow) ease-out, expanded / capsule offsets, auto-hide slide) is mirrored with an ease-out quartic
  const ty = { from: 0, to: 0, t0: 0 };
  function bodyTy() {
    if (away) return -1.3 * sp.h.x; // .away: translateY(-130%)
    const to = Pill.state === 'expanded' ? snapPx(CFG.expTop * scale()) : capsule() ? snapPx(CFG.capTop * scale()) : 0, now = performance.now();
    const p = reduce() ? 1 : Math.min(1, (now - ty.t0) / 380), cur = ty.from + (ty.to - ty.from) * (1 - Math.pow(1 - p, 4));
    if (!ty.t0) { ty.from = ty.to = to; ty.t0 = -1e9; return to; } // first call: the page loaded at rest
    if (to !== ty.to) { ty.from = cur; ty.to = to; ty.t0 = now; }
    return cur;
  }
  let lastMv = { x: 0, y: 0 };
  function setCursor(x, y) { // window-relative css px (clientX / clientY); null = cursor gone. Fed by mousemove and by the Rust event pill:cursor (~60 Hz near the island).
    if (x == null || y == null || reduce() || hidden) { LQ.want = 0; if (LQ.inn !== 0) kick(); return; }
    poke();
    if (Math.hypot(x - lastMv.x, y - lastMv.y) > 2) { lastMv = { x, y }; if (openT && hovering && Pill.state !== 'expanded') armOpen(); } // hover intent: the pointer has to rest
    // body box from the spring state (no layout read): left/width/height are what paintBody writes, the vertical offset is the CSS translateY transition of #body, tracked below
    const s = scale(), bw = sp.w.x, bcx = snapPx((window.innerWidth - bw) / 2) + bw / 2, bbot = bodyTy() + sp.h.x, half = bw / s / 2, lx = (x - bcx) / s, dyb = (y - bbot) / s, expd = Pill.state === 'expanded';
    // measured: collapsed sag up to 14 px (19-20 with the pointer inside), onset ~25 px before contact; expanded: 15-25 px just under the edge, a +5..11 px bulge while the pointer is above it (up to ~200 px)
    let f, A;
    if (dyb > 0) { f = 1 - dyb / 25; A = expd ? 20 : 14; }
    else { const up = -dyb; f = 1; A = expd ? 11 * Math.max(0, 1 - up / 220) : 14 + 5 * smooth(up / 12); }
    f *= Math.max(0, 1 - Math.max(0, Math.abs(lx) - half) / 60);
    if (LQ.drag) { A = Math.max(A, 21); f = Math.max(f, 0.9); } // a file drag hovers: the edge always reaches for it
    LQ.want = A > 0.5 ? smooth(f) : 0; LQ.bAmp = A;
    LQ.cx = Math.max(-half, Math.min(half, lx));
    if (LQ.want > 0 || LQ.inn !== 0) kick();
  }
  // ---- edge style: the Character page saves `character` = liquid | tail | legs | glow | none; apply it live (settings:changed + start) ----
  function applyCharacter() {
    const v = ['liquid', 'tail', 'legs', 'glow', 'none'].includes(S.character) ? S.character : 'liquid';
    if (v === LQ.style) return;
    LQ.style = v; lq.style.display = v === 'none' ? 'none' : '';
    liqColors();
    kick();
    if (!rafSet.has(tick)) liqStatic();
  }
  // ---- reaction resolver: sources (agents, media, shelf, reminders...) ask for a preset with Pill.react(source, name, { ttlMs, color }); the highest priority wins,
  // a finished transient falls back to whatever is still active. Pill.setReaction below is the low-level setter / preview and temporarily overrides it.
  const PRIO = { attention: 6, thinking: 5, file: 4, done: 3, music: 2, sleep: 1, calm: 0 };
  const rx = new Map(); // source -> { name, color, until (ms, 0 = stays) }
  let rxT = 0, preview = 0, rtag = null, lastPoke = 0, sleepT = 0;
  const rxL = new Set(); // Pill.onReaction observers (pets.js): fn(name, color) on every effective change, also a fresh event of the same preset
  function paintReact(name, color, tag) { // tag identifies the winning entry: a fresh event of the same preset restarts the animation
    if (name === LQ.react && (color || '') === LQ.rcol && tag === rtag) return;
    rtag = tag; LQ.react = name; LQ.rt = 0; LQ.rcol = color || '';
    rxL.forEach((f) => safe(f, name, LQ.rcol));
    liqColors();
    if (!reduce()) kick(); else if (!rafSet.has(tick)) liqStatic();
  }
  function resolve() {
    clearTimeout(rxT);
    const now = Date.now();
    let best = null, next = Infinity;
    for (const [k, e] of rx) {
      if (e.until && e.until <= now) { rx.delete(k); continue; }
      if (e.until) next = Math.min(next, e.until);
      if (!best || PRIO[e.name] > PRIO[best.name]) best = e;
    }
    if (next < Infinity) rxT = setTimeout(resolve, Math.max(next - now, 30) + 5);
    if (!preview) paintReact(best ? best.name : 'calm', best ? best.color : '', best);
  }
  // activity or the pointer: wake from sleep and restart the idle timer (5 min with nothing happening and the pointer away -> 'sleep')
  function poke() {
    const n = performance.now();
    if (n - lastPoke < 1000 && !rx.has('sleep')) return;
    lastPoke = n; clearTimeout(sleepT);
    sleepT = setTimeout(() => { if (hovering || Pill.state !== 'collapsed') poke(); else react('sleep', 'sleep'); }, 300000);
    if (rx.delete('sleep')) resolve();
  }
  function react(source, name, o) {
    if (!source || !Object.prototype.hasOwnProperty.call(PRIO, name)) return;
    const ttl = (o && o.ttlMs) || 0, color = (o && o.color) || '', old = rx.get(source);
    if (old && !ttl && !old.until && old.name === name && old.color === color) return; // same standing state again (agents:changed, media:changed)
    if (name !== 'sleep') poke();
    rx.set(source, { name, color, until: ttl ? Date.now() + ttl : 0 });
    resolve();
  }
  function unreact(source) { if (rx.delete(source)) resolve(); }
  function setReaction(name, o) { // low-level setter / preview: named presets calm music thinking done attention file sleep; { ttlMs, color }. With ttlMs the resolved state returns afterwards.
    if (!REACT[name]) name = 'calm';
    clearTimeout(LQ.rTimer);
    const ttl = (o && o.ttlMs) || 0;
    preview = ttl && name !== 'calm' ? 1 : 0;
    paintReact(name, (o && o.color) || '', {});
    if (preview) LQ.rTimer = setTimeout(() => { preview = 0; rtag = {}; resolve(); }, ttl);
    return name;
  }


  // ---- spring: width / height / border radii of #body + the liquid edge share ONE loop (the scheduler above) that stops itself when everything is settled ----
  const sp = { w: { x: 0, v: 0, t: 0, k: 380, z: 0.62 }, h: { x: 0, v: 0, t: 0, k: 380, z: 0.7 }, r: { x: 0, v: 0, t: 0, k: 380, z: 0.62 }, t: { x: 0, v: 0, t: 0, k: 380, z: 0.62 } };
  // notes A/B/C: expand ~230 ms with +8 % width overshoot; collapse ~270 ms and a squash BELOW the final size (width ~70 %, height ~40 %, deeper after a click), back in ~0.45 s;
  // tab switch: panel height 413 -> 329 reaches the target in ~170 ms, undershoots ~10 %, settles in ~470 ms; capsule content change: width overshoot ~10 %
  const MODES = {
    expand:   { w: [420, 0.6], h: [420, 0.74], r: [420, 0.6], t: [420, 0.6] },
    // damping is set so the undershoot stays inside the content: the travel is ~300 px tall / ~370 px wide, a 6 % overshoot of that used to push
    // the capsule below its own 28 px (a clipped sliver for ~0.3 s that read as the island vanishing), so h is near critical and w keeps a soft squash
    collapse: { w: [166, 0.74], h: [166, 0.88], r: [166, 0.7], t: [166, 0.7] },
    squash:   { w: [150, 0.7], h: [150, 0.86], r: [150, 0.7], t: [150, 0.7] },   // after a click that closes the island (paste)
    tab:      { h: [250, 0.55] },
    resize:   { w: [380, 0.57], h: [380, 0.7], r: [380, 0.6], t: [380, 0.6] },
  };
  let springBusy = false;
  const pending = []; // completion callbacks of every go(): none is dropped when a later go() (e.g. the ResizeObserver) retargets mid-animation
  function paintBody(imp) {
    const r = Math.max(0, sp.r.x), tp = Math.max(0, sp.t.x), s = scale();
    body.style.width = sp.w.x + 'px';
    body.style.height = sp.h.x + 'px';
    body.style.left = snapPx((window.innerWidth - sp.w.x) / 2) + 'px';   // no translate(-50%): keeps text on whole device pixels
    panelEl.style.left = snapPx((sp.w.x - CFG.expW * s) / 2) + 'px';
    body.style.borderRadius = `${tp}px ${tp}px ${r}px ${r}px`;
    const d = (sp.h.x - LQ.lastH) / s;
    LQ.lastH = sp.h.x;
    if (imp && Math.abs(d) > 0.02 && !reduce() && !hidden) liqImpulse(d);
  }
  const onResize = () => { applyOffsets(); paintBody(false); liqSize(); if (!rafSet.has(tick)) liqStatic(); };
  window.addEventListener('resize', onResize);
  (function watchDpr() { const mq = matchMedia(`(resolution: ${dpr()}dppx)`); mq.addEventListener('change', () => { onResize(); watchDpr(); }, { once: true }); })(); // monitor / DPI change: layout again
  function finish() {
    const fs = pending.splice(0);
    fs.forEach((f) => safe(f));
    if (Pill.state === 'collapsed') next(); // queued event cards run whichever retarget settled last
  }
  const kick = () => { if (!reduce()) raf(tick); };
  function tick(dt) {
    let sMove = false, settled = false;
    if (springBusy) {
      const n = Math.max(1, Math.ceil(dt * 240)), step = dt / n;
      for (const q of Object.values(sp)) {
        const c = 2 * Math.sqrt(q.k) * q.z;
        for (let i = 0; i < n; i++) { q.v += (-q.k * (q.x - q.t) - c * q.v) * step; q.x += q.v * step; }
        if (Math.abs(q.x - q.t) > 0.5 || Math.abs(q.v) > 3) sMove = true; else { q.x = q.t; q.v = 0; } // px / px per s: below this nothing is visible
      }
      paintBody(true);
      if (!sMove) { springBusy = false; settled = true; }
    }
    const lMove = liqStep(dt);
    if (settled) queueMicrotask(finish); // the Rust frame shrink / queued cards must not wait for the liquid to stop wobbling
    return sMove || lMove;
  }
  function snap(tg) {
    for (const k in tg) { sp[k].x = sp[k].t = tg[k]; sp[k].v = 0; }
    springBusy = false;
    paintBody(false);
    if (!rafSet.has(tick)) liqStatic();
    finish();
  }
  function go(tg, done, mode) {
    const mp = MODES[mode || 'resize'] || MODES.resize;
    for (const k in tg) { sp[k].t = tg[k]; if (mp[k] && !(mode === 'resize' && springBusy)) { sp[k].k = mp[k][0]; sp[k].z = mp[k][1]; } } // a capsule content resize must not retune a running expand / collapse
    if (done) pending.push(done);
    if (reduce() || hidden || document.hidden) { snap(tg); return; }
    springBusy = true;
    kick();
  }

  let petRes = 0; // css px of the capsule reserved for the pet (Pill.reserve, pets.js); #live gets the same padding in pets.css
  const colTarget = () => {
    const s = scale(), hgt = CFG.colH * s;
    const ev = (x) => { x = Math.ceil(x); return x + (x % 2); };
    if ((cardLive && liveKey === 'card') || liveTall) { // the island grown to an event card (or a tall live widget): its own content height, round ends
      const r = liveIn.getBoundingClientRect(), hh = ev(Math.max(hgt, r.height)), rr = Math.min(24 * s, hh / 2);
      return { w: ev(Math.min(CFG.colMaxW * s, Math.max(CFG.colMinW * s, r.width + petRes * s))), h: hh, r: rr, t: capsule() ? rr : 0 };
    }
    return { w: ev(Math.min(CFG.colMaxW * s, Math.max(CFG.colMinW * s, liveIn.getBoundingClientRect().width + petRes * s))), h: hgt, r: hgt / 2, t: capsule() ? hgt / 2 : 0 };
  };
  // body height of the current tab (home 338, note / weather / chat 417; register({height}) overrides), clamped to the OS frame (pill.rs EXPANDED)
  const DEF_H = { home: 331, clipboard: 331, shelf: 331, text: 331, controls: 331, settings: 331, note: 417, weather: 413, chat: 417 };
  const tabH = () => {
    const want = (cur && (cur.height || DEF_H[cur.id])) || CFG.defH, max = CFG.expFrameH - CFG.expTop - CFG.wavePad;
    if (want > max && cur && !cur.warned) { cur.warned = true; console.warn(`pill: tab '${cur.id}' wants ${want}px, clamped to ${max}px by the ${CFG.expFrameH}px frame`); }
    return Math.min(want, max);
  };
  const expTarget = () => { const s = scale(), hh = tabH(); panelEl.style.height = hh + 'px'; return { w: CFG.expW * s, h: hh * s, r: CFG.expR * s, t: CFG.expTopR * s }; };
  let lastRect = '';
  function reportRect(tg) {
    const exp = Pill.state === 'expanded', s = scale();
    const fw = (frameExp ? CFG.expFrameW : CFG.colFrameW) * s;
    const r = { x: (fw - tg.w) / 2, y: (exp ? CFG.expTop : capsule() ? CFG.capTop : 0) * s, w: tg.w, h: tg.h }, key = JSON.stringify(r);
    if (key !== lastRect) { lastRect = key; invoke('pill_rect', r); }
  }
  function retarget(snapNow, done, mode) {
    bodyTy(); // note the moment the CSS translateY transition of #body starts (setCursor mirrors it without a layout read)
    liqColors();
    const tg = Pill.state === 'expanded' ? expTarget() : colTarget();
    if (snapNow) { if (done) pending.push(done); snap(tg); } else go(tg, done, mode);
    reportRect(tg);
  }

  // ---- collapsed live row: highest priority widget, else the clock: [weather icon] time date [battery] temp ----
  const lives = new Map();
  let liveTall = false; // the winning live widget asks to be sized by its own height (setLive { tall }): the shelf drop target
  let liveKey = '', liveT = 0, battery = null, slots = { left: '', right: '' }, wx = null, clockSig = '', clockTime = null;
  const invalidateLive = () => { liveKey = ''; clockSig = ''; };
  const loc = () => (lang === 'ru' ? 'ru-RU' : 'en-GB');
  // 12-hour clock without the AM/PM suffix (the island keeps the capsule narrow: "1:02", not "1:02AM")
  const timeText = (d) => d.toLocaleTimeString(loc(), { hour: S.clock24 ? '2-digit' : 'numeric', minute: '2-digit', second: S.showSeconds ? '2-digit' : undefined, hour12: !S.clock24 }).replace(/\s*[AaPp]\.?\s?[Mm]\.?$/, '').replace(/\s*(дп|пп)$/i, '').trim();
  const dateText = (d) => new Intl.DateTimeFormat(loc(), { weekday: 'short', day: 'numeric', month: 'short' }).format(d).replace(/[,.]/g, '');
  // weather icon ids come from net.rs (icon_for): clear-day clear-night partly-day partly-night cloud fog rain sleet snow thunder
  const WXI = { 'clear-day': ['sun', 'lp-sun'], 'clear-night': ['moon', ''], 'partly-day': ['weather', ''], 'partly-night': ['cloudmoon', ''], cloud: ['cloud', ''], fog: ['cloud', ''], rain: ['rain', ''], sleet: ['snow', ''], snow: ['snow', ''], thunder: ['bolt', ''] };
  function wxIconHtml(w) {
    let id = w.icon;
    if (!WXI[id]) { const c = String(w.code || w.text || '').toLowerCase(); id = /thunder/.test(c) ? 'thunder' : /snow|sleet/.test(c) ? 'snow' : /rain|drizzle|shower/.test(c) ? 'rain' : /fog/.test(c) ? 'fog' : /clear|sun/.test(c) ? 'clear-day' : /part|fair/.test(c) ? 'partly-day' : 'cloud'; }
    if (typeof window.PillWxIcon === 'function') { // coloured forecast icon from tab-weather.js (loaded after this file, so resolved at render time)
      try { const svg = window.PillWxIcon(id, 20); if (svg) return `<span class="lp-ico lp-wx">${svg}</span>`; } catch (e) { console.error(e); }
    }
    const [ic, cls] = WXI[id];
    return `<span class="lp-ico ${cls}">${icon(ic, { size: 18, sw: 1.6 })}</span>`;
  }
  const digits = (s) => [...s].map((c) => `<span class="d" data-c="${esc(c)}">${esc(c)}</span>`).join('');
  function setTime(el, str, anim) { // the digit that changed rolls up (minute changes only)
    const sp2 = [...el.children];
    if (sp2.length !== str.length) { el.innerHTML = digits(str); return; }
    [...str].forEach((c, i) => {
      const d = sp2[i];
      if (d.dataset.c === c) return;
      const old = d.dataset.c;
      d.dataset.c = c;
      if (!anim || reduce() || hidden || document.hidden) { d.textContent = c; return; }
      d.classList.add('roll');
      d.innerHTML = `<b class="o">${esc(old)}</b><b class="n">${esc(c)}</b>`;
      setTimeout(() => { d.classList.remove('roll'); d.textContent = c; }, 280);
    });
  }
  function renderClock() {
    const d = new Date(), tt = timeText(d), showWx = wx && S.showTemp;
    const b = battery && battery.has_battery && S.showBattery
      ? `<span class="lp-batt${battery.percent <= 15 && !battery.charging ? ' low' : ''}${battery.charging ? ' chg' : ''}"><i style="--rest:${100 - battery.percent}%"></i><span class="lp-bp">${battery.percent}%</span></span>` : '';
    // three columns (1fr auto 1fr): the time stays on the capsule axis whatever the side slots hold; icon + date left, battery + temperature right
    const left = (slots.left || (showWx ? wxIconHtml(wx) : '')) + (S.showDate ? `<span class="lp-date">${esc(dateText(d))}</span>` : '');
    const right = b + (slots.right || (showWx ? `<span class="lp-aux">${Math.round(wx.temp)}°</span>` : ''));
    const sig = `<span class="lp-l">${left}</span>${S.showTime ? '<span class="lp-time">%T%</span>' : '<span></span>'}<span class="lp-r">${right}</span>`;
    if (sig !== clockSig || !clockTime) {
      const swap = liveKey !== 'clock';
      clockSig = sig; liveKey = 'clock';
      if (swap) liveGhost();
      liveIn.innerHTML = `<div class="lp lp-clock" style="--lpw:172px">${sig.replace('%T%', digits(tt))}</div>`;
      if (swap) swapFx();
      clockTime = liveIn.querySelector('.lp-time');
      // equal side columns = the time sits exactly on the capsule axis (fr tracks collapse under shrink-to-fit sizing, so measure the cells)
      const lpc = liveIn.firstChild, s = scale(), cw = (e) => [...e.children].reduce((a, k) => a + k.getBoundingClientRect().width / s, 0) + 12 * Math.max(0, e.children.length - 1);
      lpc.style.setProperty('--cw', Math.ceil(Math.max(cw(lpc.querySelector('.lp-l')), cw(lpc.querySelector('.lp-r')))) + 'px');
      sizeLive();
    } else if (clockTime) setTime(clockTime, tt, !S.showSeconds);
  }
  // Exit phase of a live-row swap: the outgoing content stays ~90 ms as an absolutely positioned copy (.lp-ghost, fades + blurs out in pill.css) while the new content blurs in.
  // It is a sibling of #live-in inside #live and out of flow, so the width measured from liveIn (sizeLive / colTarget / ResizeObserver) never includes it.
  function liveGhost() {
    if (reduce() || hidden || document.hidden || !liveIn.firstChild || !liveIn.parentNode) return;
    const g = h('span', 'lp-ghost'), c = liveIn.cloneNode(true);
    c.removeAttribute('id'); c.className = '';
    g.append(c); liveIn.parentNode.append(g);
    setTimeout(() => g.remove(), 120);
  }
  function swapFx() { liveIn.classList.remove('swap'); void liveIn.offsetWidth; if (!reduce()) liveIn.classList.add('swap'); }
  function sizeLive() { // whole, even css px width so the centred content never lands on a half pixel
    liveIn.style.width = '';
    const w = Math.ceil(liveIn.getBoundingClientRect().width / scale());
    liveIn.style.width = w + (w % 2) + 'px';
  }
  let cardLive = null; // the event card currently shown inside the island (see next())
  function renderLive() {
    clearTimeout(liveT);
    if (cardLive) { // an event card owns the live row until it is dismissed
      if (liveKey !== 'card' || cardLive.parentNode !== liveIn) { liveKey = 'card'; clockSig = ''; clockTime = null; liveGhost(); liveIn.replaceChildren(cardLive); root.classList.add('tall'); sizeLive(); swapFx(); }
      const tone = getComputedStyle(cardLive).getPropertyValue('--tone').trim();
      if (tone) root.style.setProperty('--wave-c', tone); else root.style.removeProperty('--wave-c');
      liqColors();
      return;
    }
    if (liveKey === 'card') { liveKey = ''; root.classList.remove('tall'); liveIn.style.width = ''; }
    const now = Date.now();
    let best = null, bid = '';
    for (const [id, v] of lives) {
      if (v.until && v.until <= now) { lives.delete(id); continue; }
      if (!best || v.p > best.p) { best = v; bid = id; }
    }
    const wantTall = !!(best && best.tall);
    root.classList.toggle('tall', wantTall); // 'tall' = pill.css `.tall #live-in`: natural size, measured by colTarget (the card path above returns earlier and manages it itself)
    if (wantTall !== liveTall) { liveTall = wantTall; liveIn.style.width = ''; }
    if (best) {
      const html = /class="lp[\s"]/.test(best.html) ? best.html : `<div class="lp">${best.html}</div>`, key = bid + '|' + (best.key || html); // best.key: the caller edits the row in place, only a new key re-renders it
      if (key !== liveKey) { liveKey = key; clockSig = ''; clockTime = null; liveGhost(); liveIn.innerHTML = html; sizeLive(); swapFx(); }
    } else renderClock();
    const accent = best && best.accent ? best.accent : '';
    if (accent) root.style.setProperty('--wave-c', accent); else root.style.removeProperty('--wave-c');
    liqColors();
    liqPlaying(!!accent);
    if (hidden) return;
    let next = best ? Infinity : S.showSeconds ? 1000 - (now % 1000) + 15 : 60000 - (now % 60000) + 30; // clock: once a minute (once a second with seconds)
    for (const v of lives.values()) if (v.until) next = Math.min(next, v.until - now + 5);
    if (next < Infinity) liveT = setTimeout(renderLive, Math.max(next, 50));
  }
  new ResizeObserver(() => { if (Pill.state === 'collapsed' && !hidden) retarget(false, null, 'resize'); }).observe(liveIn);
  let batT = 0;
  async function pollBattery() {
    clearTimeout(batT);
    if (hidden) return;
    if (S.showBattery) {
      const b = await invoke('battery_state');
      if (b && JSON.stringify(b) !== JSON.stringify(battery)) { battery = b; invalidateLive(); renderLive(); }
    }
    batT = setTimeout(pollBattery, 60000);
  }
  // temperature: event weather:current { temp, unit, code, icon, text } or weather_get -> { status:'ok', now:{ temp (deg C), icon, symbol } } (net.rs); nothing is shown when weather is off / unknown
  let wxT = 0, wxNet = null, wxAt = 0; // wxAt: when the last forecast arrived
  const WX_MS = 15 * 60000;
  function setWx(n) {
    wx = n && n.temp != null && isFinite(+n.temp) ? { temp: +n.temp, icon: n.icon, code: n.code || n.symbol, text: n.text } : null;
    invalidateLive(); renderLive();
  }
  async function refreshWeather() {
    clearTimeout(wxT);
    wxNet = wxWanted();
    if (!wxNet) { if (wx) setWx(null); return; }
    if (hidden) return; // nobody sees the capsule: no fetch while the island is hidden, pill:show comes back through wxOnShow()
    const r = await invoke('weather_get', { force: false, tz: -new Date().getTimezoneOffset() });
    if (r && r.status === 'ok' && r.now) { setWx(r.now); wxAt = Date.now(); } else if (r && (r.status === 'off' || r.status === 'nocity')) setWx(null);
    if (!hidden) wxT = setTimeout(refreshWeather, WX_MS);
  }
  const wxWanted = () => !!S['net.weather'] && S['module.weather'] !== false; // Settings > Weather switch AND Settings > Modules > Weather
  function wxOnShow() { // pill:show: one refresh if the forecast is older than 15 min, else just re-arm the timer for the rest of it
    clearTimeout(wxT);
    if (wxWanted() && wxAt && Date.now() - wxAt < WX_MS) wxT = setTimeout(refreshWeather, Math.max(1000, WX_MS - (Date.now() - wxAt)));
    else refreshWeather();
  }

  // ---- tabs + header cluster ----
  const tabs = new Map();
  let cur = null, prevTab = null, expWait = false; // expWait: doExpand waits for the OS frame to reach the expanded size before it starts the spring
  const visibleTabs = () => [...tabs.values()].filter((x) => !x.hidden && !x.off).sort((a, b) => a.order - b.order);
  // ---- Settings > Modules: a module that is off hides its tab (live, from settings:changed). Home stays while the player OR the launcher is on. ----
  const MODTAB = { clipboard: 'module.clipboard', shelf: 'module.shelf', text: 'module.text', chat: 'module.chat', note: 'module.note', weather: 'module.weather', controls: 'module.controls', notifs: 'module.notifs', agents: 'live.agents' };
  const modOn = (k) => S[k] !== false;
  const tabOff = (id) => (id === 'home' ? !modOn('module.media') && !modOn('module.launcher') : MODTAB[id] ? !modOn(MODTAB[id]) : false);
  function applyModules() {
    let changed = false;
    tabs.forEach((rec, id) => {
      const off = tabOff(id);
      if (off === !!rec.off) return;
      rec.off = off; changed = true;
      if (rec.btn) rec.btn.style.display = off ? 'none' : '';
    });
    if (!changed) return;
    if (cur && cur.off) { const first = visibleTabs()[0]; if (first) activate(first.id); } // the open tab was switched off: go to the first one that is left
    if (Pill.state === 'expanded') retarget(false, null, 'tab');
  }
  function activate(id, persist) {
    const tb = tabs.get(id);
    if (!tb || tb === cur) return;
    if (persist && !tb.hidden && S.lastTab !== id) { S.lastTab = id; invoke('settings_set', { patch: { lastTab: id } }); } // the island reopens on the last used tab
    if (cur) {
      const old = cur, order = [...visibleTabs(), tabs.get('settings')];
      viewsEl.dataset.dir = order.indexOf(tb) > order.indexOf(old) ? 'r' : 'l';
      old.btn && old.btn.classList.remove('on'); old.view.classList.remove('on');
      if (Pill.state === 'expanded' && !reduce()) { old.view.classList.add('out'); setTimeout(() => old.view.classList.remove('out'), 160); }
      if (!old.hidden) prevTab = old.id;
      if (Pill.state === 'expanded') safe(old.onHide);
    }
    cur = tb;
    tb.btn && tb.btn.classList.add('on'); tb.view.classList.add('on');
    setBtn.classList.toggle('cur', tb.id === 'settings');
    if (Pill.state === 'expanded') { safe(tb.onShow); if (!expWait) retarget(false, null, 'tab'); } // tabs differ in height: the panel height follows its own spring
  }
  function register(o) {
    if (!o || !o.id || tabs.has(o.id)) return;
    const view = h('div', 'view');
    viewsEl.append(view);
    const rec = { ...o, order: o.order ?? 100, view, btn: null, title: t(o.title) || o.id };
    if (!o.hidden) {
      const btn = h('button', 'tab');
      btn.innerHTML = `${o.icon || ''}<span class="tl"></span><span class="tb"></span>`;
      btn.querySelector('.tl').textContent = rec.title;
      // whole-pixel label width (--tw, used by .tab .tl in pill.css): a fractional width put every tab after the active one on x.39 and blurred their icons
      const fitTl = () => { const c = fitTl.c || (fitTl.c = document.createElement('canvas').getContext('2d')); c.font = '600 12px PillUI, "Segoe UI Variable Text", "Segoe UI", NotoUI, Inter, system-ui, sans-serif'; btn.style.setProperty('--tw', Math.ceil(c.measureText(rec.title).width + 0.5) + 'px'); };
      fitTl(); if (document.fonts && document.fonts.ready) document.fonts.ready.then(fitTl);
      btn.title = rec.title;
      btn.onclick = () => { chosen = true; activate(o.id, true); };
      rec.btn = btn;
    }
    tabs.set(o.id, rec);
    visibleTabs().forEach((x) => tabsEl.append(x.btn));
    safe(() => o.mount && o.mount(view));
    if (!chosen && !rec.hidden) activate(visibleTabs()[0].id);
  }
  function setBadge(id, n) {
    const tb = tabs.get(id);
    if (!tb || !tb.btn) return;
    tb.btn.querySelector('.tb').textContent = n ? String(n) : '';
  }

  // header: VPN country badge (only when known), dictation, pin, settings
  const vpnEl = h('span', 'vpn');
  vpnEl.innerHTML = icon('shield', { size: 14, sw: 1.9 }) + '<b></b>';
  const mkBtn = (name, ic, ttl) => { const b = h('button', 'hbtn'); b.innerHTML = icon(ic, { size: 17 }); b.title = ttl; b.dataset.name = name; return b; };
  const dictBtn = mkBtn('dict', 'wave', ''), pinBtn = mkBtn('pin', 'pin', ''), setBtn = mkBtn('settings', 'settings', '');
  hdrEl.append(vpnEl, dictBtn, pinBtn, setBtn);
  function setVpn(cc) {
    S.vpnCountry = cc ? String(cc).toUpperCase().slice(0, 3) : '';
    vpnEl.classList.toggle('on', !!S.vpnCountry);
    vpnEl.querySelector('b').textContent = S.vpnCountry;
  }
  function setPinned(v) { S.pinned = !!v; pinBtn.classList.toggle('on', S.pinned); }
  dictBtn.onclick = () => {
    if (typeof Pill.dictate === 'function') return safe(Pill.dictate);
    toast({ id: 'dict-soon', icon: 'wave', title: t({ ru: 'Диктовка скоро', en: 'Dictation is coming soon' }), ttlMs: 1800 });
  };
  pinBtn.onclick = () => setPinned(!S.pinned);
  // The settings live in their own window (settings_open { page }); the in-pill Settings tab stays only as a hidden fallback when the command is unavailable.
  const PAGES = ['general', 'character', 'hotkeys', 'ai', 'voice', 'weather', 'sound', 'live', 'clipboard', 'modules', 'updates'];
  async function openSettings(page) {
    page = PAGES.includes(page) ? page : 'general';
    try { if (!(T && T.core)) throw new Error('no tauri'); await T.core.invoke('settings_open', { page }); return true; } catch (e) { console.warn('settings_open unavailable, using the in-pill settings', e); return false; }
  }
  setBtn.onclick = async () => {
    if (cur && cur.id === 'settings') return activate(prevTab && tabs.has(prevTab) ? prevTab : visibleTabs()[0].id);
    if (await openSettings('general')) doCollapse(true);
    else { chosen = true; activate('settings'); }
  };
  function retitle() {
    dictBtn.title = t({ ru: 'Диктовка', en: 'Dictation' });
    pinBtn.title = t({ ru: 'Закрепить', en: 'Pin open' });
    setBtn.title = t({ ru: 'Настройки', en: 'Settings' });
  }

  // ---- expand / collapse ----
  const keepOpen = () => S.pinned;
  function doExpand(tab, section) {
    if (tab === 'settings') { // the command bar asks for the settings tab: open the window instead and give the frame back
      openSettings(section).then((okw) => { if (okw) { if (frameExp || Pill.state === 'expanded') { if (Pill.state === 'expanded') doCollapse(true); else invoke('pill_set_state', { state: 'collapsed' }); } } else doExpand('settings_fallback'); });
      return;
    }
    if (tab === 'settings_fallback') tab = 'settings';
    const was = Pill.state === 'expanded', frameWas = frameExp, fw = CFG.expFrameW * scale() - 2;
    // the OS window grows just before this event: start the spring after the first rAF in which the page already has the expanded frame size (capped at 50 ms),
    // so the surface resize does not eat the first spring frames
    expWait = !was && !frameWas && !reduce() && !hidden && !document.hidden && window.innerWidth < fw;
    Pill.state = 'expanded'; frameExp = true; cancelAway();
    root.classList.add('expanded');
    if (tab && tabs.has(tab) && !tabs.get(tab).off) { chosen = true; if (tabs.get(tab) !== cur) activate(tab, true); else if (!was) safe(cur.onShow); }
    else if (!was && cur) safe(cur.onShow);
    if (!was) {
      if (!expWait) { retarget(false, null, 'expand'); return; }
      const t0 = performance.now(); let fired = false;
      const go1 = () => { if (fired) return; fired = true; expWait = false; if (Pill.state === 'expanded') retarget(false, null, 'expand'); };
      const poll = () => { if (fired) return; if (window.innerWidth >= fw || performance.now() - t0 >= 50) go1(); else requestAnimationFrame(poll); };
      requestAnimationFrame(poll); setTimeout(go1, 55); // the timer covers a paused rAF
    }
  }
  function doCollapse(force, deep) {
    if (Pill.state !== 'expanded') return;
    if (S.pinned && force !== true) return;
    Pill.state = 'collapsed';
    root.classList.remove('expanded');
    if (document.activeElement && document.activeElement !== document.body) document.activeElement.blur();
    if (cur) safe(cur.onHide);
    retarget(false, () => {
      if (Pill.state !== 'collapsed') return; // re-expanded during the animation
      frameExp = false;
      invoke('pill_set_state', { state: 'collapsed' }).then(() => reportRect(colTarget()));
      scheduleAway(); next();
    }, deep ? 'squash' : 'collapse');
  }
  function forceCollapse() { // hidden by Rust: no animation at all
    const was = Pill.state === 'expanded';
    Pill.state = 'collapsed'; root.classList.remove('expanded');
    if (was && cur) safe(cur.onHide);
    if (frameExp) { frameExp = false; invoke('pill_set_state', { state: 'collapsed' }); }
    retarget(true);
  }

  // ---- auto-hide: slide #body up, then Rust does SW_HIDE; cursor at the top edge or an event brings it back ----
  let awayT = 0;
  const cancelAway = () => clearTimeout(awayT);
  function scheduleAway() {
    cancelAway();
    if (S.autoHide && !away && !hidden && Pill.state === 'collapsed' && !hovering && !curCard && !queue.length) awayT = setTimeout(async () => { if (S.autoHideKeepDesktop && await invoke('pill_desktop_fg')) scheduleAway(); else slideAway(); }, 1500); // 'Do not hide on the desktop': stay while the desktop is in front
  }
  function slideAway() {
    if (!S.autoHide || away || Pill.state !== 'collapsed' || hovering || curCard || queue.length) return;
    away = true; body.classList.add('away');
    setTimeout(() => { if (away) invoke('pill_auto_hide', { on: true }); }, reduce() ? 20 : 340);
  }
  const reveal = () => { if (away) invoke('pill_auto_hide', { on: false }); };

  // ---- event cards: notch-shaped, hang from the edge. { id, html | (icon, title, sub, tone), actions:[{label, icon, kind, run}], ttlMs } ----
  const queue = [];
  let curCard = null;
  const iconHtml = (v) => (typeof v === 'string' && v[0] === '<' ? v : icon(v, { size: 20 }));
  function buildCard(o) {
    const el = h('div', 'tc in' + (o.cls ? ' ' + o.cls : ''));
    if (o.tone) el.dataset.tone = o.tone;
    if (o.icon) { const i = h('div', 'ti' + (o.square ? ' sq' : '')); i.innerHTML = iconHtml(o.icon); el.append(i); }
    if (o.title) {
      const tx = h('div', 'tx'); tx.append(h('div', 'tt', o.title));
      if (o.sub) tx.append(h('div', 'ts', o.sub));
      el.append(tx);
    } else { const m = h('div', 'msg'); m.innerHTML = o.html || ''; el.append(m); }
    if (o.actions && o.actions.length) {
      const ta = h('div', 'ta');
      for (const a of o.actions) {
        const b = h('button', 'ab' + (a.kind ? ' ' + a.kind : '') + (a.icon && !a.label ? ' icon' : ''));
        if (a.icon) b.innerHTML = icon(a.icon, { size: 17, sw: 1.9 });
        if (a.label) b.append(document.createTextNode(a.label));
        b.onclick = () => { safe(a.run); dismiss(); };
        ta.append(b);
      }
      el.append(ta);
    }
    return { el };
  }
  // The card is shown INSIDE the pill: the island itself grows to the card (renderLive puts it in the live row, colTarget measures it,
  // the usual spring resizes the body and the liquid edge stays the pill's own border). The OS frame is taller while a card is up.
  function next() {
    if (curCard || away || hidden || Pill.state === 'expanded' || !queue.length) return;
    const o = queue.shift(), { el } = buildCard(o);
    el.classList.remove('in'); el.classList.add('inpill');
    invoke('pill_cards', { on: true }); // pill.rs CARD_FRAME
    curCard = { id: o.id, el, t: setTimeout(dismiss, o.ttlMs || 4000) };
    cardLive = el;
    root.classList.add('carding');
    renderLive();
    // o.fit(el, room): a card that grows with its text trims its line-clamp to the css px the OS frame leaves under the island (4 px spare)
    if (o.fit) { safe(o.fit, el, Math.floor(CFG.cardFrameH - CFG.capTop - CFG.wavePad) - 4); sizeLive(); }
  }
  function endCard(c) { c.el.remove(); if (curCard === c) curCard = null; if (cardLive === c.el) cardLive = null; }
  function dismiss() {
    const c = curCard;
    if (!c || c.out) return;
    c.out = true; clearTimeout(c.t);
    c.el.classList.add('out');
    setTimeout(() => {
      endCard(c); next();
      if (!curCard) { renderLive(); setTimeout(() => { if (!curCard) { root.classList.remove('carding'); invoke('pill_cards', { on: false }); } }, 600); } // the OS frame shrinks after the island has folded back
      scheduleAway();
    }, reduce() ? 0 : 180);
  }
  function toast(o) {
    if (!o || hidden) return;
    cancelAway();
    if (o.id != null) {
      for (let i = queue.length - 1; i >= 0; i--) if (queue[i].id === o.id) queue.splice(i, 1);
      if (curCard && curCard.id === o.id) { clearTimeout(curCard.t); endCard(curCard); }
    }
    queue.push(o);
    reveal(); next();
  }
  function dropCards() { queue.length = 0; root.classList.remove('carding'); invoke('pill_cards', { on: false }); if (curCard) { clearTimeout(curCard.t); endCard(curCard); renderLive(); } }

  // ---- settings state ----
  function applyOffsets() { // vertical offsets of the floating island on whole device pixels (a monitor / DPI change re-runs this from the resize handler)
    root.style.setProperty('--off-exp', snapPx(CFG.expTop * scale()) + 'px');
    root.style.setProperty('--off-cap', snapPx(CFG.capTop * scale()) + 'px');
  }
  function applySettings() {
    root.style.setProperty('--s', scale());
    const layoutSig = [scale(), S.shape, devicePixelRatio].join('|'), clockSig2 = [S.clock24, S.showTime, S.showDate, S.showTemp, S.showSeconds, S.showBattery, lang].join('|');
    if (layoutSig !== applySettings.layout) liqSize();
    applyOffsets();
    root.dataset.theme = S.theme === 'auto' ? (mqTheme.matches ? 'light' : 'dark') : (S.theme || 'dark');
    root.dataset.shape = capsule() ? 'capsule' : 'notch';
    // folded opacity: the pass-through mode has its own transparency (passThroughOpacity, 0..80)
    root.style.setProperty('--op-col', String(S.passThrough ? 1 - Math.min(80, Math.max(0, +S.passThroughOpacity || 0)) / 100 : 1 - Math.min(60, Math.max(0, +S.opacityCol || 0)) / 100));
    root.style.setProperty('--op-exp', String(1 - Math.min(60, Math.max(0, +S.opacityExp || 0)) / 100));
    root.classList.toggle('rm', reduce());
    { const f = Math.round(+S.fps), e = Math.round(+S.edgeIdleFps); fpsCap = [30, 60, 120, 144, 240].includes(f) ? f : 0; idleFps = e === 15 || e === 60 ? e : 30; }
    setVpn(S.vpnCountry);
    applyModules();
    // every settings_set (also the lastTab write on a tab switch) comes back as settings:changed: only touch what actually changed, never snap a running animation
    if (clockSig2 !== applySettings.clock) { applySettings.clock = clockSig2; invalidateLive(); renderLive(); }
    if (layoutSig !== applySettings.layout) { applySettings.layout = layoutSig; retarget(true); }
    liqColors();
    if (S.autoHide) scheduleAway(); else cancelAway();
  }
  mqTheme.addEventListener('change', () => { if (S.theme === 'auto') applySettings(); });
  mqMotion.addEventListener('change', applySettings);
  function put(key, val) { S[key] = val; applySettings(); invoke('settings_set', { patch: { [key]: val } }); }

  // language switch: translated strings are built at mount time, so the cheapest correct way is a reload of the (collapsed) page
  async function checkLang() {
    const want = resolveLang(S.language);
    try { localStorage.setItem('pill.lang', want); } catch (e) { /* storage blocked */ }
    if (want === lang) return;
    try { sessionStorage.setItem('pill.reopen', 'settings'); } catch (e) { /* ignore */ }
    if (frameExp) await invoke('pill_set_state', { state: 'collapsed' });
    location.reload();
  }

  function buildSettings(view) {
    const redraws = [];
    const scroll = h('div', 'st view-scroll');
    view.append(scroll);
    const card = () => { const c = h('div', 'st-card'); scroll.append(c); return c; };
    const head = (txt) => scroll.append(h('h3', '', txt));
    const row = (c, label, desc, ctl) => {
      const r = h('div', 'st-row'), l = h('div', 'l');
      l.append(h('b', '', label)); if (desc) l.append(h('span', '', desc));
      r.append(l, ctl); c.append(r); return r;
    };
    const seg = (opts, get, set, track = true) => {
      const w = h('div', 'seg');
      const draw = () => [...w.children].forEach((b, i) => b.classList.toggle('on', String(opts[i][0]) === String(get())));
      opts.forEach(([v, l]) => { const b = h('button', '', l); b.onclick = () => { set(v); draw(); }; w.append(b); });
      if (track) redraws.push(draw);
      draw();
      return w;
    };
    const sw = (key) => {
      const w = h('label', 'switch'), i = h('input'), k = h('i');
      i.type = 'checkbox'; i.checked = !!S[key]; i.onchange = () => put(key, i.checked);
      w.append(i, k); redraws.push(() => { i.checked = !!S[key]; }); return w;
    };
    const range = (key, min, max, step, fmt) => {
      const w = h('div', 'st-range'), r = h('input', 'slider thin'), out = h('output');
      Object.assign(r, { type: 'range', min, max, step, value: S[key] });
      const paint = () => { r.style.setProperty('--p', ((r.value - min) / (max - min)) * 100 + '%'); out.textContent = fmt(+r.value); };
      r.oninput = paint;
      r.onchange = () => put(key, +r.value);
      w.append(r, out); redraws.push(() => { r.value = S[key]; paint(); }); paint(); return w;
    };

    head(t({ ru: 'Общие', en: 'General' }));
    let c = card();
    row(c, t({ ru: 'Язык', en: 'Language' }), t({ ru: 'По умолчанию как в Windows', en: 'By default, the same as Windows' }),
      seg([['auto', 'Windows'], ['ru', 'Русский'], ['en', 'English']], () => S.language || 'auto', (v) => put('language', v)));
    row(c, t({ ru: 'Тема', en: 'Theme' }), '', seg([['dark', t({ ru: 'Тёмная', en: 'Dark' })], ['light', t({ ru: 'Светлая', en: 'Light' })], ['auto', t({ ru: 'Авто', en: 'Auto' })]], () => S.theme, (v) => put('theme', v)));
    row(c, t({ ru: 'Размер острова', en: 'Pill size' }), t({ ru: 'Масштаб всего интерфейса', en: 'Scales the whole interface' }), range('size', 80, 200, 5, (v) => v + ' %'));
    row(c, t({ ru: 'Форма острова', en: 'Pill shape' }), '', seg([['notch', t({ ru: 'Вырез', en: 'Notch' })], ['capsule', t({ ru: 'Капсула', en: 'Capsule' })]], () => S.shape, (v) => put('shape', v)));
    row(c, t({ ru: 'Прозрачность свёрнутого', en: 'Transparency of the folded pill' }), t({ ru: 'Пилюля и карточки', en: 'The pill and its cards' }), range('opacityCol', 0, 60, 5, (v) => v + ' %'));
    row(c, t({ ru: 'Прозрачность открытого', en: 'Transparency of the open pill' }), '', range('opacityExp', 0, 60, 5, (v) => v + ' %'));

    head(t({ ru: 'Свёрнутый остров', en: 'Folded pill' }));
    c = card();
    row(c, t({ ru: 'Время', en: 'Time' }), t({ ru: 'Пока на острове ничего не происходит', en: 'While nothing else is happening' }), sw('showTime'));
    row(c, t({ ru: 'Дата', en: 'Date' }), t({ ru: 'Рядом со временем', en: 'Next to the time' }), sw('showDate'));
    row(c, t({ ru: 'Температура', en: 'Temperature' }), t({ ru: 'Нужна включённая погода', en: 'Needs the weather feature' }), sw('showTemp'));
    row(c, t({ ru: 'Секунды', en: 'Seconds' }), '', sw('showSeconds'));
    row(c, t({ ru: 'Формат часов', en: 'Clock format' }), '', seg([[false, '12 h'], [true, '24 h']], () => !!S.clock24, (v) => put('clock24', v)));
    row(c, t({ ru: 'Заряд батареи', en: 'Battery charge' }), t({ ru: 'На компьютере с батареей', en: 'On a computer that has a battery' }), sw('showBattery'));
    row(c, t({ ru: 'Живой край', en: 'Liquid edge always moving' }), t({ ru: 'Лёгкая волна на нижнем крае', en: 'A gentle wave on the bottom edge' }), sw('edgeIdle'));
    const prev = seg(Pill.reactions.map((n, i) => [n, t([{ ru: 'Покой', en: 'Calm' }, { ru: 'Музыка', en: 'Music' }, { ru: 'ИИ думает', en: 'AI thinking' }, { ru: 'Готово', en: 'Done' }, { ru: 'Внимание', en: 'Attention' }, { ru: 'Файл', en: 'File' }, { ru: 'Сон', en: 'Sleep' }][i])]), () => LQ.react, (v) => Pill.setReaction(v, { ttlMs: v === 'calm' ? 0 : 6000 }), false);
    prev.classList.add('wrap');
    row(c, t({ ru: 'Посмотреть реакцию края', en: 'Preview an edge reaction' }), t({ ru: 'Показывается 6 секунд', en: 'Shown for 6 seconds' }), prev);
    row(c, t({ ru: 'Скрывать, пока не нужен', en: 'Hide until it is needed' }), t({ ru: 'Подведи курсор к верхнему краю экрана', en: 'Rest the cursor at the top edge of the screen' }), sw('autoHide'));
    row(c, t({ ru: 'Уменьшить анимацию', en: 'Reduce motion' }), '', sw('reduceMotion'));

    head(t({ ru: 'Экран', en: 'Screen' }));
    c = card();
    const mon = h('div'); row(c, t({ ru: 'Монитор', en: 'Monitor' }), '', mon);
    row(c, t({ ru: 'Показывать на скриншотах', en: 'Show in screenshots' }), t({ ru: 'Остров попадёт в записи экрана', en: 'The pill appears in recordings' }), sw('showInCapture'));
    c = card();
    const ta = h('textarea', 'field');
    ta.rows = 2; ta.placeholder = t({ ru: 'по одному exe в строке, например game.exe', en: 'one exe per line, e.g. game.exe' });
    ta.value = (S.hideInApps || []).join('\n');
    ta.onchange = () => put('hideInApps', ta.value.split(/[\n,;]+/).map((x) => x.trim()).filter(Boolean));
    redraws.push(() => { if (document.activeElement !== ta) ta.value = (S.hideInApps || []).join('\n'); });
    const wr = h('div', 'st-row'); wr.style.display = 'block';
    const lb = h('div', 'l'); lb.append(h('b', '', t({ ru: 'Прятать, пока впереди эти приложения', en: 'Hide while these apps are in front' })));
    lb.style.marginBottom = '8px'; wr.append(lb, ta); c.append(wr);

    head(t({ ru: 'Буфер обмена', en: 'Clipboard' }));
    c = card();
    const pw = h('label', 'switch'), pi = h('input'), pk = h('i');
    pi.type = 'checkbox'; pw.append(pi, pk);
    pi.onchange = () => { invoke('clip_pause', { paused: pi.checked }); if (Pill.clipPaused) Pill.clipPaused(pi.checked); };
    row(c, t({ ru: 'Приватный режим', en: 'Private mode' }), t({ ru: 'История не записывается, пока включено', en: 'Nothing is recorded while it is on' }), pw);
    redraws.push(async () => { const s = await invoke('clip_status'); if (s) pi.checked = !!s.paused; });

    head(t({ ru: 'Клавиши', en: 'Hotkeys' }));
    const keys = h('div', 'st-card st-keys'); scroll.append(keys);
    const drawKeys = () => {
      keys.replaceChildren();
      [[t({ ru: 'Показать / свернуть', en: 'Toggle pill' }), S['hotkey.toggle']], [t({ ru: 'Командная строка', en: 'Command bar' }), S['hotkey.command']],
       [t({ ru: 'Буфер обмена', en: 'Clipboard' }), S['hotkey.clipboard']], [t({ ru: 'Выход', en: 'Quit' }), 'Ctrl+Alt+Q']]
        .forEach(([n, k]) => { const kb = h('kbd', '', k || '-'); keys.append(h('span', '', n), kb); });
    };
    redraws.push(drawKeys);
    scroll.append(h('div', 'st-note', t({ ru: 'Изменения сохраняются сразу. Клавиши меняются в окне настроек, на странице «Клавиши».', en: 'Changes are saved immediately. Hotkeys are changed in the Settings window, on the Hotkeys page.' })));

    return { refresh: async () => {
      redraws.forEach((f) => f());
      const list = (await invoke('pill_monitors')) || [];
      mon.replaceChildren(seg([['primary', t({ ru: 'Основной', en: 'Primary' })], ...list.map((_, i) => [String(i + 1), String(i + 1)])], () => S.monitor || 'primary', (v) => put('monitor', v), false));
      mon.title = list.join('\n');
    } };
  }

  // ---- public API ----
  const Pill = window.Pill = {
    state: 'collapsed',
    settings: S,
    invoke,
    on,
    t,
    icon,
    register,
    liveOn(k) { const v = S['live.' + k]; return v === undefined ? k !== 'obs' : !!v; }, // Settings > Live events: Pill.liveOn('music') etc.; OBS is the only one that is off by default
    moduleOn: modOn,           // Settings > Modules: Pill.moduleOn('module.media')
    setBadge,
    setVpn,
    setPinned,
    setCursor,
    setDragPull(on) { on = !!on; if (on === LQ.drag) return; LQ.drag = on; if (!on && LQ.inn !== 0) kick(); }, // tab-shelf.js: files are dragged over the island (the edge sags toward the cursor)
    react, unreact,           // Pill.react(source, name, { ttlMs, color }) / Pill.unreact(source): priority resolver (see PRIO), several sources never fight
    setReaction,               // 'calm' | 'music' | 'thinking' | 'done' | 'attention' | 'file' | 'sleep', { ttlMs, color }
    reactions: ['calm', 'music', 'thinking', 'done', 'attention', 'file', 'sleep'],
    onReaction(fn) { rxL.add(fn); safe(fn, LQ.react, LQ.rcol); return () => rxL.delete(fn); }, // observer of the effective reaction; called immediately with the current one
    raf,                       // the one shared animation loop: Pill.raf((dt, now) => keepRunning)
    reduced: reduce,
    _lq: LQ, // debug / harness
    // edge colour override for the liquid outline (null = default / accent of the live widget)
    setEdgeColor(c) { LQ.override = c || ''; liqColors(); if (!rafSet.has(tick)) liqStatic(); },
    get fps() { return fpsCap; }, // user frame cap of the shared loop (setting `fps`), 0 = auto = display rate
    perf: null, // set to { n: 0, ms: 0 } to measure the cost of the shared rAF loop
    // weather lane: Pill.setClockSlots({ left: '<span class="lp-ico lp-sun">…</span>', right: '<span class="lp-aux">16°</span>' }); null clears
    setClockSlots(o) { slots = { left: (o && o.left) || '', right: (o && o.right) || '' }; invalidateLive(); renderLive(); },
    setLive(id, w) { // w: { priority, html, key?, ttlMs, accent } ; key: the row is re-rendered only when it changes (the caller edits it in place) ; accent colours (and animates) the wavy outline
      if (!w) lives.delete(id);
      else { lives.set(id, { p: w.priority || 0, html: w.html || '', key: w.key || '', until: w.ttlMs ? Date.now() + w.ttlMs : 0, accent: w.accent || '', tall: !!w.tall }); reveal(); }
      if (!hidden) renderLive();
    },
    toast,
    expand(tab, via) {
      if (T && T.core) invoke('pill_set_state', { state: 'expanded', tab: tab || null, via: via || 'page' });
      else doExpand(tab); // plain browser preview
    },
    collapse: (how) => doCollapse(true, how === 'click'),
    openCommandBar: () => invoke('cmd_toggle'),
    relayoutLive() { if (hidden || cardLive) return; sizeLive(); if (Pill.state === 'collapsed') retarget(false, null, 'resize'); }, // the live row's CSS changed (characters.css keeps a slot for the legs' eyes): re-measure its fixed width and resize the capsule to it
    reserve(px) { px = Math.max(0, Math.round(+px || 0)); if (px === petRes) return; petRes = px; if (Pill.state === 'collapsed' && !hidden) retarget(false, null, 'resize'); }, // widen the collapsed capsule by px css px (a pet lives there); animated like any content resize
  };
  Object.defineProperty(Pill, 'lang', { get: () => lang, enumerable: true });
  retitle();

  let settingsUi = null;
  register({
    id: 'settings', title: t({ ru: 'Настройки', en: 'Settings' }), order: 900, hidden: true,
    icon: icon('settings'),
    mount(v) { settingsUi = buildSettings(v); },
    onShow() { settingsUi && settingsUi.refresh(); },
  });

  // ---- Rust events ----
  on('pill:expand', (p) => { poke(); doExpand(p && p.tab, p && p.section); });
  on('pill:open-tab', (p) => { if (p && p.tab === 'settings') doExpand('settings', p.section); }); // command bar payload { tab, noteId?, section? }
  on('pill:collapse', () => doCollapse());
  on('chat:focus', () => setTimeout(() => { const f = cur && cur.view && cur.view.querySelector('textarea, input[type=text], input:not([type])'); if (f) f.focus(); }, 380)); // chat hotkey: cursor straight into the input
  on('pill:cursor', (p) => setCursor(p && p.x, p && p.y)); // Rust: {x, y} css px relative to the window, ~60 Hz near the island, {x:null, y:null} on leave
  on('audio:bands', setBands);   // media lane: { b: [32 floats], rms } ~30 Hz while a track plays
  on('weather:current', setWx);  // weather lane: { temp, unit, code, icon, text }
  let openT = 0, leaveT = 0;
  const armOpen = () => { clearTimeout(openT); openT = setTimeout(() => { if (hovering && Pill.state !== 'expanded') Pill.expand(undefined, 'hover'); }, 250); };
  on('pill:hover', () => { hovering = true; poke(); cancelAway(); clearTimeout(leaveT); if (Pill.state !== 'expanded') armOpen(); });
  on('pill:leave', () => {
    hovering = false; clearTimeout(openT);
    if (Pill.state === 'expanded' && !typing) { clearTimeout(leaveT); leaveT = setTimeout(() => { if (!hovering && !typing) doCollapse(); }, 380); } else scheduleAway();
  });
  on('pill:hide', () => { // stop every timer and animation: zero page work while hidden
    hidden = true; clearTimeout(liveT); clearTimeout(batT); clearTimeout(wxT); LQ.want = 0; LQ.inn = 0; LQ.innV = 0; rafPause(); syncPaused(); cancelAway(); dropCards(); hovering = false;
    forceCollapse();
  });
  window.addEventListener('load', () => { if (wx) { invalidateLive(); renderLive(); } }); // tab-weather.js (PillWxIcon) is loaded by now: redraw the clock row with the coloured icon
  on('pill:show', () => { hidden = false; syncPaused(); rafWake(); wxOnShow(); away = false; body.classList.remove('away'); retarget(true); renderLive(); pollBattery(); scheduleAway(); next(); });
  on('settings:changed', (p) => {
    Object.assign(S, p); applySettings(); checkLang();
    if (wxWanted() !== wxNet) refreshWeather();
    applyCharacter();
    if (settingsUi && Pill.state === 'expanded' && cur && cur.id === 'settings') settingsUi.refresh();
  });

  // ---- keyboard focus for text fields (the window is NOACTIVATE until a field needs keys) ----
  const TEXT = 'textarea, input:not([type]), input[type=text], input[type=search], input[type=password], input[type=number], input[type=url], input[type=email], [contenteditable]:not([contenteditable=false])';
  document.addEventListener('focusin', (e) => { if (e.target.matches && e.target.matches(TEXT)) { typing = true; invoke('pill_activate', { on: true }); } });
  document.addEventListener('focusout', (e) => {
    if (!(e.target.matches && e.target.matches(TEXT))) return;
    typing = false; invoke('pill_activate', { on: false });
    if (Pill.state === 'expanded' && !hovering) setTimeout(() => { if (!typing && !hovering) doCollapse(); }, 300);
  });
  document.addEventListener('keydown', (e) => { if (e.key === 'Escape') doCollapse(); });
  document.getElementById('stage').addEventListener('mousedown', (e) => { if (e.target.id === 'stage') doCollapse(); }); // click outside the body
  document.addEventListener('mousemove', (e) => setCursor(e.clientX, e.clientY), { passive: true });
  document.documentElement.addEventListener('mouseleave', () => setCursor(null));
  // painted fill of every .slider (theme.css reads --p)
  document.addEventListener('input', (e) => {
    const r = e.target;
    if (r && r.classList && r.classList.contains('slider')) r.style.setProperty('--p', ((r.value - (r.min || 0)) / ((r.max || 100) - (r.min || 0))) * 100 + '%');
  });

  // ---- start ----
  applySettings();
  (async () => {
    const s = await invoke('settings_get');
    if (s) { Object.assign(S, s); applySettings(); checkLang(); }
    applyCharacter(); poke();
    if (!chosen && S.lastTab && tabs.has(S.lastTab) && !tabs.get(S.lastTab).hidden && !tabs.get(S.lastTab).off) activate(S.lastTab);
    scheduleAway(); pollBattery(); refreshWeather();
    let re = null;
    try { re = sessionStorage.getItem('pill.reopen'); sessionStorage.removeItem('pill.reopen'); } catch (e) { /* ignore */ }
    if (re) setTimeout(() => Pill.expand(re), 250);
  })();
})();
