// Page side of tools/dev/export-pets.mjs. Loaded by tools/dev/pets-harness.html AFTER the pet scripts, in a headless Chrome.
// window.PetsExport.dumpPet(id) builds the pet in many "contexts" (state x side x idle flag x reduced motion x look x energy),
// reads the computed style of every node and the CSS animations the browser starts, and returns ONE compact JSON-able object:
//   { id, w, h, ar, name, group, fx, nodes:[{t, p, a}], bb:[[x,y,w,h]|0], base:[style], ctx:{key:{st:{idx:patch}, an:{idx:[anim]}}}, kf:[keyframes], ... }
// The Rust engine (src-tauri/src/ui/pets) replays that: it never evaluates CSS itself. Everything here is generic (no per-pet code).
(() => {
  'use strict';
  const P = window.PillPets;
  const STATES = P.states;
  const r4 = (n) => Math.round(n * 10000) / 10000;
  const num = (s) => parseFloat(s);
  const KILL = '.pet,.pet *{animation:none!important;transition:none!important}';
  const killEl = document.createElement('style');
  killEl.textContent = KILL;
  const stage = document.getElementById('stage');

  // ---- colour: "rgb()/rgba()/color(srgb r g b / a)" -> [r,g,b,a] in 0..1
  function parseColor(s) {
    if (!s || s === 'none') return null;
    let m = /^rgba?\(([^)]+)\)$/.exec(s);
    if (m) { const v = m[1].split(/[ ,/]+/).filter(Boolean).map(num); return [v[0] / 255, v[1] / 255, v[2] / 255, v.length > 3 ? v[3] : 1]; }
    m = /^color\(srgb ([^)]+)\)$/.exec(s);
    if (m) { const v = m[1].split(/[ /]+/).filter(Boolean).map(num); return [v[0], v[1], v[2], v.length > 3 ? v[3] : 1]; }
    throw new Error('colour ' + s);
  }
  // ---- transform list (keyframes): "translateY(4%) scale(1.07, 0.9)" -> [["t",xpx,xpct,ypx,ypct],["s",sx,sy],["r",deg],["k",ax,ay],["m",a,b,c,d,e,f]]
  function lenParts(v) {
    v = v.trim();
    if (v.endsWith('px')) return [num(v), 0];
    if (v.endsWith('%')) return [0, num(v)];
    if (/^-?[\d.]+$/.test(v) && num(v) === 0) return [0, 0];
    throw new Error('length ' + v);
  }
  function angle(v) {
    v = v.trim();
    if (v.endsWith('deg')) return num(v);
    if (v.endsWith('turn')) return num(v) * 360;
    if (v.endsWith('rad')) return num(v) * 180 / Math.PI;
    if (num(v) === 0) return 0;
    throw new Error('angle ' + v);
  }
  function parseTf(s) {
    if (!s || s === 'none') return [];
    const out = [];
    const re = /([a-zA-Z]+)\(([^)]*)\)/g;
    let m;
    while ((m = re.exec(s))) {
      const f = m[1], a = m[2].split(',').map((x) => x.trim());
      if (f === 'translate') { const x = lenParts(a[0]), y = lenParts(a[1] || '0px'); out.push(['t', r4(x[0]), r4(x[1]), r4(y[0]), r4(y[1])]); }
      else if (f === 'translateX') { const x = lenParts(a[0]); out.push(['t', r4(x[0]), r4(x[1]), 0, 0]); }
      else if (f === 'translateY') { const y = lenParts(a[0]); out.push(['t', 0, 0, r4(y[0]), r4(y[1])]); }
      else if (f === 'scale') out.push(['s', r4(num(a[0])), r4(num(a[a.length > 1 ? 1 : 0]))]);
      else if (f === 'scaleX') out.push(['s', r4(num(a[0])), 1]);
      else if (f === 'scaleY') out.push(['s', 1, r4(num(a[0]))]);
      else if (f === 'rotate') out.push(['r', r4(angle(a[0]))]);
      else if (f === 'skewX') out.push(['k', r4(angle(a[0])), 0]);
      else if (f === 'skewY') out.push(['k', 0, r4(angle(a[0]))]);
      else if (f === 'skew') out.push(['k', r4(angle(a[0])), r4(angle(a[1] || '0'))]);
      else if (f === 'matrix') out.push(['m', ...a.map((x) => r4(num(x)))]);
      else throw new Error('transform fn ' + f + ' in ' + s);
    }
    return out;
  }
  // computed `transform` is always a matrix(): decompose (CSS "unmatrix", 2D): translate . rotate . skew . scale
  function decomp(str) {
    if (!str || str === 'none') return null;
    const m = /^matrix\(([^)]+)\)$/.exec(str);
    if (!m) throw new Error('computed transform ' + str);
    let [a, b, c, d, e, f] = m[1].split(',').map(num);
    let sx = Math.hypot(a, b);
    a /= sx; b /= sx;
    let sk = a * c + b * d;
    c -= a * sk; d -= b * sk;
    const sy = Math.hypot(c, d);
    c /= sy; d /= sy; sk /= sy;
    if (a * d < b * c) { a = -a; b = -b; sk = -sk; sx = -sx; }
    const rot = Math.atan2(b, a) * 180 / Math.PI;
    const o = [r4(e), r4(f), r4(rot), r4(sx), r4(sy), r4(Math.atan(sk) * 180 / Math.PI)];
    return o.every((v, i) => v === [0, 0, 0, 1, 1, 0][i]) ? null : o;
  }
  const PROP = { transform: 'transform', opacity: 'opacity', fill: 'fill', stroke: 'stroke', visibility: 'visibility', rotate: 'rotate', translate: 'translate', scale: 'scale',
    d: 'd', strokeDashoffset: 'stroke-dashoffset', strokeDasharray: 'stroke-dasharray', strokeWidth: 'stroke-width', filter: 'filter' };
  const dashArr = (s) => (!s || s === 'none' ? null : s.split(/[ ,]+/).map((x) => r4(num(x))));
  function kfValue(prop, v) {
    switch (prop) {
      case 'transform': return parseTf(v);
      case 'opacity': case 'strokeDashoffset': case 'strokeWidth': return r4(num(v));
      case 'fill': case 'stroke': { const c = parseColor(v); return c ? c.map(r4) : null; }
      case 'visibility': return v === 'visible' ? 1 : 0;
      case 'rotate': return v === 'none' ? 0 : r4(angle(v));
      case 'd': return v === 'none' ? '' : /^path\("(.*)"\)$/.exec(v)[1];
      case 'strokeDasharray': return dashArr(v);
      default: throw new Error('keyframe property ' + prop);
    }
  }
  const EASE = { linear: 'l', ease: 'e', 'ease-in': 'i', 'ease-out': 'o', 'ease-in-out': 'io', 'step-start': 'steps(1,start)', 'step-end': 'steps(1)' };
  const easing = (e) => EASE[e] || e.replace(/\s*,\s*/g, ',').replace(/\s+/g, ' ');
  const DIR = { normal: 0, reverse: 1, alternate: 2, 'alternate-reverse': 3 };
  const FILL = { none: 0, forwards: 1, backwards: 2, both: 3 };

  const kfPool = [], kfIndex = new Map();
  function poolKeyframes(a) {
    const out = a.effect.getKeyframes().map((f) => {
      const o = { o: r4(f.computedOffset), e: easing(f.easing || 'linear') };
      for (const k of Object.keys(f)) {
        if (['offset', 'computedOffset', 'easing', 'composite'].includes(k)) continue;
        o[PROP[k] ? k : (() => { throw new Error('unknown keyframe property ' + k); })()] = kfValue(k, f[k]);
      }
      return o;
    });
    const key = JSON.stringify(out);
    let i = kfIndex.get(key);
    if (i === undefined) { i = kfPool.length; kfPool.push(out); kfIndex.set(key, i); }
    return i;
  }

  // ---- style of one node (computed, animations and transitions switched off)
  const CAP = { butt: 0, round: 1, square: 2 }, JOIN = { miter: 0, round: 1, bevel: 2, 'miter-clip': 0, arcs: 0 };
  function shadows(s) { // "drop-shadow(color dx dy blur) drop-shadow(...)" -> [{c, x, y, b}]
    if (!s || s === 'none') return [];
    const out = [];
    const re = /drop-shadow\((.*?)\)(?=\s*drop-shadow|\s*$)/g;
    let m;
    while ((m = re.exec(s))) {
      const t = m[1];
      const cm = /^(rgba?\([^)]*\)|color\([^)]*\))\s+(.*)$/.exec(t);
      if (!cm) throw new Error('drop-shadow ' + t);
      const n = cm[2].split(/\s+/).map(num);
      out.push({ c: parseColor(cm[1]).map(r4), x: r4(n[0]), y: r4(n[1]), b: r4(n[2] || 0) });
    }
    return out;
  }
  // the browser re-serialises the `d` attribute ("M0 0L1 1" -> "M 0 0 L 1 1"): compare against that form so only real CSS overrides are exported
  const dProbe = document.createElementNS('http://www.w3.org/2000/svg', 'path'), dSvg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'), dCache = new Map();
  dSvg.style.cssText = 'position:absolute;visibility:hidden'; dSvg.append(dProbe); document.body.append(dSvg);
  function normD(d) {
    if (!dCache.has(d)) { dProbe.setAttribute('d', d); const m = /^path\("(.*)"\)$/.exec(getComputedStyle(dProbe).d); dCache.set(d, m ? m[1] : d); }
    return dCache.get(d);
  }
  function readStyle(el, inst) {
    const cs = getComputedStyle(el), s = {};
    if (cs.display === 'none') { s.dn = 1; return s; }
    if (cs.visibility === 'hidden' || cs.visibility === 'collapse') s.vh = 1;
    if (cs.opacity !== '1') s.o = r4(num(cs.opacity));
    const tf = decomp(cs.transform);
    if (tf) s.tf = tf;
    if (cs.translate !== 'none') { const v = cs.translate.split(' ').map((x) => r4(num(x))); s.tl = [v[0], v[1] || 0]; } // a single value means y = 0
    if (cs.rotate !== 'none') s.ro = r4(angle(cs.rotate));
    if (cs.scale !== 'none') { const v = cs.scale.split(' ').map(num); s.sc = [r4(v[0]), r4(v[v.length > 1 ? 1 : 0])]; }
    s.po = cs.transformOrigin.split(' ').map((x) => r4(num(x))); // px inside the reference box
    s.tb = cs.transformBox === 'fill-box' ? 1 : 0;
    if (el instanceof SVGElement && el.tagName !== 'svg') {
      const f = parseColor(cs.fill), st = parseColor(cs.stroke);
      s.fi = f ? f.map(r4) : 0;
      s.st = st ? st.map(r4) : 0;
      if (st) {
        s.sw = r4(num(cs.strokeWidth));
        s.lc = CAP[cs.strokeLinecap]; s.lj = JOIN[cs.strokeLinejoin];
        const da = dashArr(cs.strokeDasharray);
        if (da) s.da = da;
        if (num(cs.strokeDashoffset)) s.dof = r4(num(cs.strokeDashoffset));
      }
      if (el.tagName === 'path') { const m = /^path\("(.*)"\)$/.exec(cs.d); if (m && m[1] !== normD(el.getAttribute('d'))) s.d = m[1]; }
    }
    const fl = shadows(cs.filter);
    if (fl.length) s.fl = fl;
    return s;
  }
  // transitions declared on the node (read with transitions ON): [[prop, durMs, delayMs, easing]]
  function readTransitions(el) {
    const cs = getComputedStyle(el);
    const props = cs.transitionProperty.split(',').map((x) => x.trim()), du = cs.transitionDuration.split(',').map((x) => x.trim()),
      de = cs.transitionDelay.split(',').map((x) => x.trim()), ea = cs.transitionTimingFunction.split(/,(?![^(]*\))/).map((x) => x.trim());
    const t = (x) => (x.endsWith('ms') ? num(x) : num(x) * 1000);
    const out = [];
    props.forEach((p, i) => {
      const d = t(du[i % du.length]), dl = t(de[i % de.length]);
      if (d > 0) out.push([p, r4(d), r4(dl), easing(ea[i % ea.length])]);
    });
    return out;
  }
  // tint weight: how much of the accent a colour takes (colour-mix percentages in pets.css). Sample --pet-k=1 with a white and a black accent.
  function tintWeights(inst, nodes, styles) {
    const el = inst.el;
    const read = () => nodes.map((n) => { const cs = getComputedStyle(n); return n instanceof SVGElement && n.tagName !== 'svg' ? { f: parseColor(cs.fill), s: parseColor(cs.stroke), fl: shadows(cs.filter) } : { fl: shadows(cs.filter) }; });
    el.style.setProperty('--pet-k', '1'); el.style.setProperty('--pet-accent', '#ffffff'); const w = read();
    el.style.setProperty('--pet-accent', '#000000'); const b = read();
    const wt = (c0, cw, cb) => {
      let best = 0, bd = 0.25;
      for (let ch = 0; ch < 3; ch++) for (const [A, c1] of [[1, cw], [0, cb]]) {
        const den = A - c0[ch];
        if (Math.abs(den) > bd) { bd = Math.abs(den); best = (c1[ch] - c0[ch]) / den; }
      }
      return Math.abs(best) < 0.01 ? 0 : r4(Math.min(1, Math.max(0, best)));
    };
    nodes.forEach((n, i) => {
      const st = styles[i];
      if (st.dn) return;
      for (const [k, key] of [['fi', 'f'], ['st', 's']]) {
        if (st[k] && w[i][key]) { const p = wt(st[k], w[i][key], b[i][key]); if (p) st[k] = [...st[k], p]; }
      }
      if (st.fl) st.fl.forEach((sh, j) => {
        const cw = w[i].fl[j] && w[i].fl[j].c, cb = b[i].fl[j] && b[i].fl[j].c;
        if (!cw) return;
        if (sh.c[3] === 0 && cw[3] > 0) sh.c = [0, 0, 0, 0, 1, r4(cw[3])]; // transparent -> the accent itself at alpha da (rig glow)
        else { const p = wt(sh.c, cw, cb); if (p) sh.c = [...sh.c, p, 0]; }
      });
    });
    el.style.setProperty('--pet-k', '0');
  }

  // ---- one context -> { styles[], anims{idx:[...]}, trans[idx] }
  function snapshot(id, opt) {
    const inst = P.create(id, { state: opt.state, side: opt.side, size: 'm', idle: false });
    inst.el.style.setProperty('--pet-px', '96px');
    stage.append(inst.el);
    document.documentElement.classList.toggle('rm', !!opt.rm);
    (opt.flags || []).forEach((f) => (f === 'pet' ? inst.pet() : inst.el.classList.add('act-' + f)));
    if (opt.energy != null) inst.setEnergy(opt.energy);
    if (opt.look) inst.look(opt.look[0], opt.look[1]);
    inst.el.style.setProperty('--pet-k', '0');
    const nodes = [inst.el, ...inst.el.querySelectorAll('*')];
    const idx = new Map(nodes.map((n, i) => [n, i]));
    void inst.el.offsetWidth;
    // animations and transitions are read live
    const anims = {};
    const trans = nodes.map((n) => readTransitions(n));
    for (const a of inst.el.getAnimations({ subtree: true })) {
      if (!(a instanceof CSSAnimation)) continue; // transitions (the --pet-k fade) are replayed from their declared specs
      const t = a.effect.target, i = idx.get(t);
      if (a.effect.pseudoElement) { (anims.pseudo = anims.pseudo || []).push([a.effect.pseudoElement, a.animationName]); continue; }
      if (i === undefined) throw new Error('animation target outside the pet');
      const tm = a.effect.getTiming();
      (anims[i] = anims[i] || []).push([poolKeyframes(a), r4(tm.duration), r4(tm.delay), tm.iterations === Infinity ? -1 : tm.iterations, DIR[tm.direction], FILL[tm.fill]]);
    }
    document.head.append(killEl);
    void inst.el.offsetWidth;
    const styles = nodes.map((n) => readStyle(n, inst));
    if (!opt.noTint) tintWeights(inst, nodes, styles);
    const bbs = nodes.map((n, i) => { if (styles[i].dn || !(n instanceof SVGGraphicsElement) || n.tagName === 'svg') return 0; const b = n.getBBox(); return [r4(b.x), r4(b.y), r4(b.width), r4(b.height)]; });
    const rect = nodes.map((n, i) => (i < 2 ? { width: n.offsetWidth || n.getBoundingClientRect().width, height: n.offsetHeight || n.getBoundingClientRect().height } : null));
    // pivot: absolute in the node's own coordinate system (fill-box: bbox origin + offset); HTML boxes: fraction of the box
    styles.forEach((st, i) => {
      if (st.dn || !st.po) return;
      if (rect[i]) st.po = [r4(st.po[0] / rect[i].width), r4(st.po[1] / rect[i].height)];
      else if (st.tb && bbs[i]) st.po = [r4(bbs[i][0] + st.po[0]), r4(bbs[i][1] + st.po[1])];
      else if (st.tb) st.po = [0, 0];
      if (st.po[0] === 0 && st.po[1] === 0) delete st.po;
      delete st.tb;
    });
    // HTML boxes (the rig): px at the 96 px sample size -> viewBox units, so a `translateY(7%)` means the same at any size
    const vb = nodes[2].viewBox.baseVal, unit = vb.height / (rect[1].height || 96);
    for (const i of [0, 1]) {
      const st = styles[i];
      if (st.dn) continue;
      if (st.tf) { st.tf[0] = r4(st.tf[0] * unit); st.tf[1] = r4(st.tf[1] * unit); }
      if (st.tl) st.tl = st.tl.map((v) => r4(v * unit));
    }
    const tags = nodes.map((n) => n.tagName.toLowerCase());
    const attrs = nodes.map((n) => { const o = {}; for (const a of n.attributes) if (!['class', 'style', 'transform', 'aria-hidden', 'overflow'].includes(a.name)) o[a.name] = a.value; return o; });
    const parents = nodes.map((n) => (n === inst.el ? -1 : idx.get(n.parentElement)));
    const classes = nodes.map((n) => (n.getAttribute && n.getAttribute('class')) || '');
    const out = { styles, anims, trans, bbs, tags, attrs, parents, classes, rig: rect[1] && { w: rect[1].width, h: rect[1].height } };
    document.documentElement.classList.remove('rm');
    killEl.remove();
    inst.destroy();
    return out;
  }

  // ---- patches: only what differs from a reference snapshot
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
  function patchStyles(ref, cur) {
    const out = {};
    cur.styles.forEach((s, i) => {
      const r = ref.styles[i], p = {};
      for (const k of new Set([...Object.keys(s), ...Object.keys(r)])) if (!same(s[k], r[k])) p[k] = s[k] === undefined ? null : s[k];
      if (Object.keys(p).length) out[i] = p;
    });
    return out;
  }
  function patchAnims(ref, cur) {
    const out = {};
    for (const i of new Set([...Object.keys(cur.anims), ...Object.keys(ref.anims)])) {
      if (i === 'pseudo') continue;
      if (!same(cur.anims[i], ref.anims[i])) out[i] = cur.anims[i] || [];
    }
    return out;
  }
  function patchTrans(ref, cur) {
    const out = {};
    cur.trans.forEach((t, i) => { if (!same(t, ref.trans[i])) out[i] = t; });
    return out;
  }
  const mk = (ref, cur, staticOnly) => {
    const o = {}, st = patchStyles(ref, cur), an = staticOnly ? {} : patchAnims(ref, cur), tr = staticOnly ? {} : patchTrans(ref, cur);
    if (Object.keys(st).length) o.st = st;
    if (Object.keys(an).length) o.an = an;
    if (Object.keys(tr).length) o.tr = tr;
    return Object.keys(o).length ? o : null;
  };

  async function dumpPet(id) {
    kfPool.length = 0; kfIndex.clear();
    const def = P.registry[id];
    const out = { id, w: def.w, h: def.h, ar: def.ar, group: def.group, name: def.name, fx: def.fx !== false, ctx: {} };
    const base = snapshot(id, { state: 'calm', side: 'right' });
    out.nodes = base.tags.map((t, i) => ({ t, p: base.parents[i], a: base.attrs[i], c: base.classes[i] }));
    out.bb = base.bbs;
    out.rig = base.rig;
    out.base = base.styles;
    out.basean = base.anims;
    out.basetr = base.trans;
    const states = {};
    for (const side of ['right', 'left']) for (const st of STATES) {
      const key = st + '|' + side[0];
      const cur = side === 'right' && st === 'calm' ? base : snapshot(id, { state: st, side });
      states[key] = cur;
      // bounding boxes of nodes that are hidden in calm: take them where they are shown
      cur.bbs.forEach((b, i) => { if (b && !out.bb[i]) out.bb[i] = b; });
      const p = mk(base, cur);
      if (p) out.ctx[key] = p;
    }
    for (const side of ['right', 'left']) for (const st of STATES) {
      const ref = states[st + '|' + side[0]];
      const key = st + '|' + side[0];
      // reduced motion: statics only
      const rm = snapshot(id, { state: st, side, rm: true });
      const prm = mk(ref, rm, true); // reduced motion: the engine plays no animation and no transition at all, only these static differences
      out.ctx['rm|' + key] = prm || {};
      for (const flag of ['blink', 'twitch', 'hop', 'pet']) {
        if (flag === 'blink' && st === 'sleep') continue;
        if ((flag === 'twitch' || flag === 'hop') && !(st === 'calm' || st === 'done')) continue;
        if (side === 'left' && flag !== 'pet') continue;
        const f = snapshot(id, { state: st, side, flags: [flag] });
        const p = mk(ref, f);
        if (p) out.ctx[flag + '|' + key] = p;
      }
    }
    // look: unit steps in x and y per state (right side)
    for (const st of STATES) {
      if (st === 'sleep') continue;
      const ref = states[st + '|r'];
      for (const [nm, v] of [['lx', [1, 0]], ['ly', [0, 1]], ['lxn', [-1, 0]], ['lyn', [0, -1]]]) {
        const f = snapshot(id, { state: st, side: 'right', look: v });
        const p = mk(ref, f);
        if (p) out.ctx[nm + '|' + st] = p;
      }
    }
    // energy (music only): five samples
    {
      const ref = states['music|r'];
      out.energy = [];
      for (const e of [0, 0.25, 0.5, 0.75, 1]) {
        const f = snapshot(id, { state: 'music', side: 'right', energy: e });
        out.energy.push(mk(ref, f) || {});
      }
    }
    out.kf = kfPool.slice();
    return out;
  }
  window.PetsExport = { dumpPet, ids: () => P.ids.slice(), states: STATES };
})();
