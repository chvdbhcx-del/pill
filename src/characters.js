// Characters: the `tail` and `legs` values of the setting `character`. Our own interpretations of two island characters seen in the user's video
// (behaviour/colour/shape measurements only: replica/video/notes-B.md sections 6 and 8). All art is simple SVG drawn from scratch.
//   tail: a tail curls out of the island's right side near the bottom corner, state-dependent shape and tip colour.
//   legs: white oval eyes inside the capsule (collapsed only, like the live row) + two thin legs with coloured feet under the bottom edge.
// Registry: window.PillChars.preview(svg, kind) -> { setState(s), destroy() } (Settings > Character cards; works without Pill).
// Island hook (only inside the pill window): one overlay <svg id="chars"> in #body, driven by settings (character), Pill.onReaction, audio:bands, pill:cursor.
// States = the island reactions: calm music thinking done attention file sleep ('ai' is accepted as thinking).
(() => {
  'use strict';
  const NS = 'http://www.w3.org/2000/svg', DEG = Math.PI / 180, TAU = Math.PI * 2;
  const STATES = ['calm', 'music', 'thinking', 'done', 'attention', 'file', 'sleep'];
  const INK = '#17141f';
  // tip / feet colours per state (notes-B section 8); a reaction that carries its own colour overrides them
  const TIP = { calm: '#f5e9dc', music: '#f08c78', thinking: '#a99be8', done: '#8fbf4a', attention: '#e8a040', file: '#8fbf4a', sleep: '#666666' };
  const FOOT = { calm: '#e0522a', music: '#f08c78', thinking: '#a99be8', done: '#8fbf4a', attention: '#e8a040', file: '#8fbf4a', sleep: '#7a7a7a' };
  const norm = (s) => (s === 'ai' ? 'thinking' : STATES.includes(s) ? s : 'calm');
  const clamp = (x, a, b) => Math.min(b, Math.max(a, x));
  const ss = (a, b, x) => { x = clamp((x - a) / (b - a), 0, 1); return x * x * (3 - 2 * x); }; // smoothstep
  const el = (tag, attrs, parent) => { const e = document.createElementNS(NS, tag); if (attrs) for (const k in attrs) e.setAttribute(k, attrs[k]); if (parent) parent.append(e); return e; };
  const set = (e, attrs) => { for (const k in attrs) e.setAttribute(k, attrs[k]); };
  const n2 = (n) => (Math.round(n * 100) / 100).toString();
  const ZP = 'M-2 -2.4h4l-4 4.8h4'; // the little "z"

  // ---------------------------------------------------------------- tail (local frame: origin = root on the island edge, +x outward, +y down; css px at 100 %)
  // The spine is integrated from a heading function f(u) in degrees (0 = right, + = clockwise/down), u = 0 root .. 1 tip: one model for every state.
  function tailPose(s, t, rt, e) {
    const sin = Math.sin;
    // every pose is a smooth arc that leaves the island's corner heading outward and bends DOWN, like the original (a hanging tail), plus the state's own motion
    const arc = (h0, h1) => (u) => h0 + (h1 - h0) * (u * (2 - u)); // eases out: most of the bend happens early, the tip hangs nearly straight
    switch (s) {
      case 'music': { const a = 0.55 + 0.45 * e, ph = TAU * t / 0.7, b = arc(8, 78); return { L: 36, w: 4.4, r: 3.6, f: (u) => b(u) + 26 * a * sin(ph - 1.8 * u) * (0.2 + u) }; } // swings with the beat, curvature pulses
      case 'thinking': return { L: 34, w: 4.2, r: 3.5, f: (u) => 10 + 150 * ss(0, 0.5, u) - 250 * ss(0.38, 1, u) - 10 * sin(t * 0.9 - 3 * u) * u }; // S / question-mark loop, slow
      case 'done': { const k = Math.exp(-rt / 0.9), b = arc(16, 84); return { L: 36, w: 4.4, r: 3.6, f: (u) => b(u) + 20 * k * sin(rt * 10 - u) * u }; } // a settling wag
      case 'attention': { const b = arc(14, 80); return { L: 30, w: 7, r: 5.6, f: (u) => b(u) + 2.5 * sin(t * 24) * u }; } // ~1.6x thick, big round tip, a quiver
      case 'file': { const b = arc(18, 82); return { L: 38, w: 2.8, r: 3, f: (u) => b(u) + 4 * sin(t * 2.2 - 2 * u) * u }; } // thin
      case 'sleep': { const b = arc(38, 100); return { L: 34, w: 4.2, r: 3.4, f: (u) => b(u) + 3 * sin(t * 0.8) * u }; } // drooping, slow breathing
      default: { const b = arc(14, 88); return { L: 38, w: 4.4, r: 3.6, f: (u) => b(u) + 5 * sin(t * 1.3 - 2.4 * u) * (0.3 + u) }; } // calm: a hanging arc, gentle sway
    }
  }
  function buildTail(parent) {
    const g = el('g', null, parent);
    const line = { fill: 'none', 'stroke-linejoin': 'round', 'stroke-linecap': 'round' }; // round ends: the root hides under the island, the tip is covered by its ball
    const o = el('path', Object.assign({ stroke: INK, 'stroke-opacity': '.5' }, line), g), p = el('path', line, g);
    const co = el('circle', { fill: INK, 'fill-opacity': '.5' }, g), c = el('circle', null, g);
    const z = el('path', { d: ZP, fill: 'none', stroke: '#9a9aa6', 'stroke-width': '1.1', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', opacity: '0' }, g);
    return {
      g,
      update(st) {
        const P = tailPose(st.s, st.t, st.rt, st.e == null ? 0.75 : st.e), n = 36, seg = P.L / n; // enough points that the spine reads as a smooth curve at 3 px thickness
        let x = 0, y = 0, d = 'M0 0';
        for (let i = 0; i < n; i++) { const th = P.f((i + 0.5) / n) * DEG; x += Math.cos(th) * seg; y += Math.sin(th) * seg; d += 'L' + n2(x) + ' ' + n2(y); }
        set(o, { d, 'stroke-width': n2(P.w + 1.8) }); set(p, { d, 'stroke-width': n2(P.w), stroke: st.s === 'attention' ? '#f2b13a' : '#f0a050' });
        set(co, { cx: n2(x), cy: n2(y), r: n2(P.r + 0.9) }); set(c, { cx: n2(x), cy: n2(y), r: n2(P.r), fill: st.col || TIP[st.s] });
        if (st.s === 'sleep') { const q = (st.t % 2.5) / 2.5; set(z, { transform: `translate(${n2(x + 6)} ${n2(y - 5 - 7 * q)}) scale(.9)`, opacity: n2(Math.sin(Math.PI * q) * 0.9) }); } else z.setAttribute('opacity', '0');
      },
    };
  }

  // ---------------------------------------------------------------- legs (origin = the middle of the island's bottom edge)
  function buildLegs(parent) {
    const g = el('g', null, parent);
    const legs = [-1, 1].map((i) => ({
      i,
      o: el('path', { fill: 'none', stroke: '#6a6a74', 'stroke-opacity': '.38', 'stroke-width': '3.4', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, g), // a faint rim so the legs read over a dark desktop
      l: el('path', { fill: 'none', stroke: '#050505', 'stroke-width': '2.2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, g),
      f: el('ellipse', { rx: '3.8', ry: '2.1', stroke: '#0a0a0c', 'stroke-opacity': '.55', 'stroke-width': '.8' }, g),
    }));
    let stepLeg = 0, stepT0 = -9, lastN = 0;
    return {
      g,
      update(st) {
        const s = st.s, t = st.t, rt = st.rt, lift = [0, 0];
        let len = 15, sway = 0, bend = 0, tilt = 0;
        if (s === 'music') { // one foot lifts ~6 px and the leg bends; the legs alternate on every detected beat, else every 0.5 s
          const b = st.beat;
          if (t < stepT0) stepT0 = t;
          if (b && b.fresh) { if (b.n !== lastN) { lastN = b.n; stepLeg ^= 1; stepT0 = t; } }
          else if (t - stepT0 > 0.5) { stepLeg ^= 1; stepT0 = t; }
          lift[stepLeg] = 6 * Math.sin(Math.PI * Math.min(1, (t - stepT0) / 0.42));
        } else if (s === 'thinking') sway = 1.2 * Math.sin(t * 1.5);
        else if (s === 'done') { const k = Math.exp(-rt / 1.2); bend = 3.2 * k * (1 + 0.3 * Math.cos(rt * 11)); tilt = 16 * k; } // bent, feet tilted, then settle
        else if (s === 'attention') len = 16.5; // straight and long
        const col = st.col || FOOT[s];
        legs.forEach((L, k) => {
          const i = L.i, hx = i * 6.5 + sway * 0.3, ax = i * 6.5 + sway + i * bend * 0.5, ay = len - lift[k] - bend * 0.4;
          const kx = i * 6.5 + sway * 0.6 + i * (bend + lift[k] * 0.55), ky = len * 0.5 - lift[k] * 0.4;
          const d = `M${n2(hx)} -0.4Q${n2(kx)} ${n2(ky)} ${n2(ax)} ${n2(ay)}`; // one smooth bow instead of two straight sticks with a kink at the knee
          L.o.setAttribute('d', d); L.l.setAttribute('d', d);
          set(L.f, { cx: n2(ax + i * 0.9), cy: n2(ay + 1.4), fill: col, transform: `rotate(${n2(tilt * i)} ${n2(ax + i * 0.9)} ${n2(ay + 1.4)})` });
        });
      },
    };
  }

  // ---------------------------------------------------------------- eyes (origin = the middle between the two eyes)
  function buildEyes(parent) {
    const g = el('g', { class: 'ch-eyes' }, parent), inner = el('g', null, g);
    const eyes = [-1, 1].map((i) => ({ i, o: el('ellipse', null, inner), a: el('path', { fill: 'none', 'stroke-width': '2', 'stroke-linecap': 'round', 'stroke-linejoin': 'round' }, inner) }));
    const z = el('path', { d: ZP, fill: 'none', 'stroke-width': '1.1', 'stroke-linecap': 'round', 'stroke-linejoin': 'round', opacity: '0' }, inner);
    return {
      g,
      update(st) {
        const s = st.s, col = st.eyeCol, kind = s === 'sleep' ? 'shut' : s === 'music' || s === 'done' || s === 'file' ? 'happy' : 'open';
        const p = st.t % 4.6, blink = s === 'sleep' || p > 0.14 ? 1 : 1 - 0.88 * Math.sin(Math.PI * p / 0.14); // a quick blink every ~4.6 s
        const rx = s === 'attention' ? 3.6 : 3, ry = (s === 'attention' ? 7.2 : s === 'thinking' ? 5.4 : 6) * blink;
        const ox = s === 'thinking' ? 0.8 : 0, oy = s === 'thinking' ? -1 : 0;
        inner.setAttribute('transform', s === 'sleep' ? '' : `translate(${n2(st.look.x * 2.4 + ox)} ${n2(st.look.y * 1.8 + oy)})`); // pupil-less ovals shift up to ~2.4 px toward the cursor
        eyes.forEach((E) => {
          const cx = E.i * 6.8;
          if (kind === 'open') { set(E.o, { cx, cy: 0, rx, ry: n2(ry), fill: col, display: '' }); E.a.setAttribute('display', 'none'); }
          else {
            E.o.setAttribute('display', 'none');
            set(E.a, { display: '', stroke: col, d: kind === 'happy' ? `M${cx - 3.6} 2.4Q${cx} -4.8 ${cx + 3.6} 2.4` : `M${cx - 3.6} -1.8Q${cx} 4.2 ${cx + 3.6} -1.8` });
          }
        });
        if (s === 'sleep') { const q = (st.t % 2.5) / 2.5; set(z, { stroke: '#9a9aa6', transform: `translate(${n2(6.8 + 7)} ${n2(-4 - 6 * q)}) scale(.9)`, opacity: n2(Math.sin(Math.PI * q) * 0.9) }); } else z.setAttribute('opacity', '0');
      },
    };
  }

  // ---------------------------------------------------------------- Settings > Character preview card (the svg has viewBox 0 0 149 92, see settings.js)
  function preview(svg, kind) {
    const blk = (d) => el('path', { d, fill: '#000' }, svg);
    let tail = null, legs = null, eyes = null;
    if (kind === 'tail') {
      blk('M-6 0H96V22a12 12 0 0 1-12 12H-6Z');
      tail = buildTail(svg); tail.g.setAttribute('transform', 'translate(93.3 28.5) scale(1.8)');
    } else {
      blk('M16 0H133V23a13 13 0 0 1-13 13H29a13 13 0 0 1-13-13Z');
      legs = buildLegs(svg); legs.g.setAttribute('transform', 'translate(74.5 36) scale(1.4)');
      eyes = buildEyes(svg); eyes.g.setAttribute('transform', 'translate(74.5 18) scale(1.15)');
    }
    let state = 'calm', t0 = performance.now(), alive = true, raf = 0;
    const mq = window.matchMedia ? matchMedia('(prefers-reduced-motion: reduce)') : { matches: false };
    function frame(now) {
      raf = 0;
      if (!alive) return;
      const still = mq.matches, T = still ? 0 : now / 1000;
      const st = { s: state, t: T, rt: still ? 99 : (now - t0) / 1000, col: '', e: null, beat: null, eyeCol: '#f4f1ea', look: { x: still ? 0 : 0.8 * Math.sin(T * 0.7), y: still ? 0 : 0.4 * Math.sin(T * 0.45 + 1) } };
      if (tail) tail.update(st);
      if (legs) { legs.update(st); eyes.update(st); }
      if (!still && !document.hidden) raf = requestAnimationFrame(frame);
    }
    const kick = () => { if (!raf && alive) raf = requestAnimationFrame(frame); };
    const vis = () => { if (!document.hidden) kick(); };
    document.addEventListener('visibilitychange', vis);
    kick();
    return {
      setState(s) { state = norm(s); t0 = performance.now(); frame(performance.now()); },
      destroy() { alive = false; cancelAnimationFrame(raf); document.removeEventListener('visibilitychange', vis); },
    };
  }

  window.PillChars = { states: STATES, preview };
  const P = window.Pill;
  if (!(P && P.onReaction)) return; // settings window / static preview: registry only

  // ---------------------------------------------------------------- island hook
  const root = document.documentElement, body = document.getElementById('body'), live = document.getElementById('live-in');
  if (!body || !live) return;
  const svg = el('svg', { id: 'chars', 'aria-hidden': 'true' });
  body.append(svg);
  const scaleOf = () => clamp((+P.settings.size || 100) / 100, 0.8, 2);
  const beat = { n: 0, t0: 0, last: 0, fresh: false };
  const st = { s: 'calm', t: 0, rt: 99, col: '', e: null, beat, eyeCol: '#f4f1ea', look: { x: 0, y: 0 } };
  let style = '', tail = null, legs = null, eyes = null, rtStart = performance.now();
  let petOn = false, petSide = 'right', eyeCol = '#f4f1ea', colAt = -1e9, aim = null, lookT = { x: 0, y: 0 }, eyeWorld = null, eS = 0, en = 0, prevEn = 0, pk = 0.2, lastBandT = 0;
  let bW = 0, bH = 0, eyeX = 0, eyeY = 0, eyeCx = null; // body / eye-slot geometry, cached: measured only when a ResizeObserver fires (never per frame)

  function themeEyes(now) { // white on the black body, dark when the user picked the light island
    if (now - colAt < 800) return;
    colAt = now;
    const bg = getComputedStyle(body).backgroundColor, m = bg.match(/[\d.]+/g);
    if (!m || m.length < 3) return;
    const k = /^color\(/.test(bg) ? 255 : 1;
    eyeCol = (0.299 * m[0] + 0.587 * m[1] + 0.114 * m[2]) * k > 140 ? '#1c1c1e' : '#f4f1ea';
  }
  function measure(entry) { // W/H from the observer entry; the rects (eye slot only) are read once per observer fire, right after layout
    const bs = entry && entry.borderBoxSize && entry.borderBoxSize[0];
    let br = null;
    if (bs) { bW = bs.inlineSize; bH = bs.blockSize; } else { br = body.getBoundingClientRect(); bW = br.width; bH = br.height; }
    if (eyes) {
      br = br || body.getBoundingClientRect();
      const lr = live.getBoundingClientRect();
      eyeX = br.left; eyeY = br.top; eyeCx = { w: lr.width, l: lr.left - br.left, r: lr.right - br.left, c: null };
      const ec = live.querySelector('.lp-clock.eic'), col = ec && ec.querySelector(eyesSide() === 'left' ? '.lp-l' : '.lp-r'); // the clock row's own empty side column (pill.js marks it .eic): the eyes stand in its middle
      if (col) { const cr = col.getBoundingClientRect(); eyeCx.c = cr.left + cr.width / 2 - br.left; }
      // the eyes stand on the same line as the text of the live row: the middle of its first text (the typeface centres capitals in the line box)
      eyeCx.ty = null;
      const tw = document.createTreeWalker(live, NodeFilter.SHOW_TEXT, { acceptNode: (n) => (n.nodeValue.trim() && !(n.parentElement && n.parentElement.closest('.lp-ghost')) ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT) }), tn = tw.nextNode();
      if (tn) { const rg = document.createRange(); rg.selectNodeContents(tn); const rr = rg.getBoundingClientRect(); if (rr.height) eyeCx.ty = rr.top + rr.height / 2 - br.top; }
    }
  }
  function place(now) { // cheap: only moves the groups to where the island is right now (cached geometry; re-measured by the observers)
    if (!style) return;
    if (!bW) measure();
    const s = scaleOf(), W = bW, H = bH;
    if (tail) { // on the bottom-right corner arc, heading along its outward normal
      const parts = body.style.borderRadius.split(' '), r = clamp(parseFloat(parts[2]) || H / 2, 1, Math.min(W, H) / 2), a = 35 * DEG;
      tail.g.setAttribute('transform', `translate(${n2(W - r + (r - 0.6) * Math.cos(a))} ${n2(H - r + (r - 0.6) * Math.sin(a))}) scale(${n2(s)})`);
    }
    if (legs) { // the legs stand right under the eyes (one character); before the eyes are measured: the middle of the island
      const lr0 = eyeCx || { w: 0, l: 0, r: 0 }, left0 = eyesSide() === 'left';
      let lx = lr0.c != null ? lr0.c : lr0.w > 1 ? (left0 ? lr0.l + 18 * s : lr0.r - 18 * s) : W / 2;
      lx = clamp(lx, 14 * s, Math.max(14 * s, W - 14 * s));
      legs.g.setAttribute('transform', `translate(${n2(lx)} ${n2(H - 0.2)}) scale(${n2(s)})`);
    }
    if (eyes) { // in the slot that characters.css keeps free on the pet-free side of the live row; stays in the capsule band while the island opens
      const lr = eyeCx || { w: 0, l: 0, r: 0 }, left = eyesSide() === 'left';
      let cx = lr.c != null ? lr.c : lr.w > 1 ? (left ? lr.l + 18 * s : lr.r - 18 * s) : (left ? 18 * s : W - 18 * s);
      cx = clamp(cx, 14 * s, Math.max(14 * s, W - 14 * s));
      const cy = lr.ty != null ? clamp(lr.ty, 8 * s, Math.max(8 * s, H - 8 * s)) : Math.min(H, 30 * s) / 2;
      eyes.g.setAttribute('transform', `translate(${n2(cx)} ${n2(cy)}) scale(${n2(s)})`);
      eyeWorld = { x: eyeX + cx, y: eyeY + cy };
      themeEyes(now == null ? performance.now() : now);
    }
  }
  const eyesSide = () => (petOn && petSide === 'left' ? 'right' : 'left');

  function draw(now, dt, still) {
    st.t = still ? 0 : now / 1000; st.rt = still ? 99 : (now - rtStart) / 1000;
    beat.fresh = !still && now - beat.last < 900;
    if (beat.fresh) eS += (en - eS) * Math.min(1, dt * 12);
    st.e = beat.fresh ? eS : null;
    st.eyeCol = eyeCol;
    const src = GZ.avail ? (GZ.ok ? GZ : null) : aim; // global gaze feed, else the pointer inside the window
    if (eyes && eyeWorld && src && !still && st.s !== 'sleep') { // look toward the cursor wherever it is on the screen: direction * |d| / (|d| + AIMK)
      const dx = src.x - eyeWorld.x, dy = src.y - eyeWorld.y, n = 1 / (Math.hypot(dx, dy) + AIMK);
      lookT.x = dx * n; lookT.y = dy * n;
    } else { lookT.x = 0; lookT.y = 0; }
    const kk = still ? 1 : 1 - Math.exp(-dt * 14);
    st.look.x += (lookT.x - st.look.x) * kk; st.look.y += (lookT.y - st.look.y) * kk;
    place(now);
    if (tail) tail.update(st);
    if (legs) legs.update(st);
    if (eyes) eyes.update(st);
  }
  function loop(dt, now) { // every frame of the shared Pill.raf loop (vsync-paced; Pill.fps caps it there); returns true to keep running
    if (!style) return false;
    if (P.reduced()) { draw(now, 0, true); return false; }
    draw(now, dt, false);
    return true;
  }
  const kick = () => { if (!style) return; if (P.reduced()) draw(performance.now(), 0, true); else P.raf(loop); };

  // ---- global gaze: Rust `pill:gaze` {x, y} = cursor in PHYSICAL px relative to the window (any distance / monitor, only while it moves, only after pill_set_gaze(true));
  // css px = physical / devicePixelRatio. Shared with pets.js through window.PillGaze (refcounted on/off). Fallback: the pointer inside the window. ----
  const GZ = window.PillGaze || (window.PillGaze = (() => {
    const want = new Set(), subs = new Set(), g = { x: 0, y: 0, ok: false, avail: false, subs };
    let on = false, vis = true;
    const sync = () => { const w = want.size > 0 && vis; if (w === on) return; on = w; if (!w) g.ok = g.avail = false; P.invoke('pill_set_gaze', { on: w }); };
    g.use = (id, b) => { if (b) want.add(id); else want.delete(id); sync(); };
    P.on('pill:gaze', (p) => {
      if (!on || !p || !isFinite(p.x) || !isFinite(p.y)) return;
      const d = window.devicePixelRatio || 1;
      g.x = p.x / d; g.y = p.y / d; g.ok = g.avail = true;
      subs.forEach((f) => f(g));
    });
    P.on('pill:hide', () => { vis = false; g.ok = false; sync(); });
    P.on('pill:show', () => { vis = true; sync(); });
    return g;
  })());
  const AIMK = 60; // css px: saturation of the distance (|d| / (|d| + AIMK) -> 1 far away)
  const gazeSync = () => { GZ.use('eyes', !!eyes && st.s !== 'sleep' && !P.reduced()); }; // asleep: eyes shut, no gaze feed
  GZ.subs.add(() => { if (eyes) kick(); });

  function apply(c) {
    const v = c === 'tail' || c === 'legs' ? c : '';
    const side0 = root.dataset.charSide;
    root.dataset.charSide = eyesSide();
    if (v === style) { if (side0 !== root.dataset.charSide && P.relayoutLive) P.relayoutLive(); kick(); return; }
    style = v;
    svg.replaceChildren(); tail = legs = eyes = null;
    if (v) root.dataset.char = v; else delete root.dataset.char;
    if (P.relayoutLive) P.relayoutLive(); // the eye slot of `legs` appears / disappears: the live row was sized without it
    svg.style.display = v ? '' : 'none';
    if (v === 'tail') tail = buildTail(svg);
    if (v === 'legs') { legs = buildLegs(svg); eyes = buildEyes(svg); }
    rtStart = performance.now();
    gazeSync();
    kick();
  }

  P.onReaction((name, color) => { st.s = norm(name); st.col = color || ''; rtStart = performance.now(); gazeSync(); kick(); }); // immediate: new colours / shape on the very next frame
  P.on('audio:bands', (p) => {
    if (!style || !p || !Array.isArray(p.b)) return;
    let e = 0;
    for (let k = 0; k < 8; k++) e += +p.b[k] || 0;
    const now = performance.now(), dtMs = lastBandT ? Math.min(200, Math.max(1, now - lastBandT)) : 33;
    lastBandT = now;
    e /= 8; pk = Math.max(e, pk * Math.pow(0.995, dtMs / 33), 0.05); en = Math.min(1, e / pk); // peak decay per 33 ms (the old 30 Hz feed), independent of the event rate
    beat.last = now;
    if (en > 0.72 && prevEn <= 0.72 && now - beat.t0 > 320) { beat.n++; beat.t0 = now; } // rising edge of the low-band energy = a step
    prevEn = en;
  });
  const aimAt = (x, y) => { aim = x == null || y == null ? null : { x, y }; if (eyes) kick(); };
  P.on('pill:cursor', (p) => aimAt(p && p.x, p && p.y));
  document.addEventListener('mousemove', (e) => aimAt(e.clientX, e.clientY), { passive: true });
  root.addEventListener('mouseleave', () => aimAt(null));
  P.on('pill:leave', () => aimAt(null));
  new ResizeObserver((es) => { measure(es[es.length - 1]); place(); }).observe(body); // the spring changes the body size every frame; keep the characters glued to it (also in reduced motion)
  new ResizeObserver(() => { if (eyes) measure(); place(); }).observe(live);
  document.addEventListener('pill:live-changed', () => { if (eyes) { measure(); place(); } }); // the clock row was rebuilt (a switch flipped, the weather arrived): where the eyes stand may have changed without any size change
  const fromSettings = (s) => {
    if (!s) return;
    if (s.pet !== undefined) petOn = !!s.pet && s.pet !== 'none';
    if (s['pet.side'] === 'left' || s['pet.side'] === 'right') petSide = s['pet.side'];
    apply(s.character !== undefined ? s.character : P.settings.character);
    measure(); place();
  };
  P.on('settings:changed', (s) => { fromSettings(Object.assign({}, P.settings, s)); setTimeout(() => { measure(); place(); }, 80); }); // --s of a changed island size lands a moment after the event
  (async () => { fromSettings(await P.invoke('settings_get') || P.settings); })();
})();
