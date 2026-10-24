// PillMotion: tiny shared helper for the animations CSS cannot do (exits of [hidden]/removed elements, collapse/expand to auto height,
// FLIP of rebuilt lists). Web Animations API only, the project curves (theme.css tokens, read once, with fallbacks).
//   exit(el, {y, x, scale, dur, delay, base, ease, remove, hide, after})   fade+move out (ease-in .16s), then remove / el.hidden / after(el); resolves true, false if interrupted
//   enter(el, {y, x, scale, dur, delay, base})                       fade (ease-out) + move on the settle spring; cancels a running exit
//   swap(a, b)                                                       a exits (hidden), then b is unhidden and enters
//   collapse(el, {dur, remove, hide, after})                         height/padding/margin to 0 (+gap of the parent), then remove/hide
//   expand(el, {dur})                                                call AFTER the element is in the layout: grows from 0 to its natural height
//   flip(box, mutate, {sel, enterNew})                               mutate() rebuilds box; children matched by data-id slide from their old spot
//   cancel(el), off()                                                drop a running animation (element back to its natural state); true when motion must be skipped
// Reduced motion (html.rm, prefers-reduced-motion) or a hidden document: no animation, the end state is applied at once.
// `base` = a transform the element already has (e.g. 'translateX(-50%)'), so the keyframes do not wipe it.
(() => {
  const root = document.documentElement, tok = {};
  const T = (n, fb) => tok[n] || (tok[n] = getComputedStyle(root).getPropertyValue(n).trim() || fb);
  const OUT = () => T('--ease-out-expo', 'cubic-bezier(.16, 1, .3, 1)');
  const SPRING = () => T('--spring-settle', 'cubic-bezier(.22, 1, .36, 1)');
  const EASE_IN = 'cubic-bezier(.4, 0, 1, 1)';
  const mq = window.matchMedia ? matchMedia('(prefers-reduced-motion: reduce)') : { matches: false };
  const off = () => root.classList.contains('rm') || mq.matches || document.hidden;

  const live = new WeakMap(); // el -> { anims, res, timer, pe }
  function release(el, r) { clearTimeout(r.timer); if (r.lock) el.style.pointerEvents = r.pe; if (r.ov != null) el.style.overflow = r.ov; }
  function cancel(el) {
    const r = live.get(el);
    if (!r) return;
    live.delete(el);
    release(el, r);
    r.anims.forEach((a) => { try { a.cancel(); } catch (_) { /* gone */ } });
    r.res(false);
  }
  // specs: [[keyframes, options], ...] played together; `end(el)` runs while the end state is still held, then the animations are dropped
  function run(el, specs, end, o = {}) {
    cancel(el);
    return new Promise((res) => {
      const anims = specs.map(([kf, op]) => el.animate(kf, op));
      const r = { anims, res, timer: 0, lock: !!o.lock, pe: el.style.pointerEvents, ov: o.hideOverflow ? el.style.overflow : null };
      if (r.lock) el.style.pointerEvents = 'none';
      if (o.hideOverflow) el.style.overflow = 'hidden';
      live.set(el, r);
      Promise.all(anims.map((a) => a.finished)).then(() => {
        if (live.get(el) !== r) return;
        live.delete(el);
        release(el, r);
        if (end) end(el);
        anims.forEach((a) => a.cancel());
        res(true);
      }, () => { /* cancelled: cancel() already resolved false */ });
      // a throttled / frozen timeline must not leave the element half-way
      r.timer = setTimeout(() => anims.forEach((a) => { try { a.finish(); } catch (_) { /* gone */ } }), Math.max(...specs.map(([, op]) => op.duration + (op.delay || 0))) + 150);
    });
  }

  const tf = (base, x, y, s) => `${base ? base + ' ' : ''}translate(${x}px, ${y}px) scale(${s})`;

  function enter(el, { y = 8, x = 0, scale = 1, dur = 0.4, delay = 0, base = '' } = {}) {
    if (off()) { cancel(el); return Promise.resolve(true); }
    const specs = [[[{ opacity: 0 }, { opacity: 1 }], { duration: 220, delay: delay * 1000, easing: OUT(), fill: 'backwards' }]];
    if (y || x || scale !== 1) specs.push([[{ transform: tf(base, x, y, scale) }, { transform: tf(base, 0, 0, 1) }], { duration: dur * 1000, delay: delay * 1000, easing: SPRING(), fill: 'backwards' }]);
    return run(el, specs);
  }

  function exit(el, { y = 0, x = 0, scale = 1, dur = 0.16, delay = 0, base = '', ease = EASE_IN, remove = false, hide = false, after } = {}) {
    const fin = (e) => { if (remove) e.remove(); else if (hide) e.hidden = true; if (after) after(e); };
    if (off() || !el.isConnected) { cancel(el); fin(el); return Promise.resolve(true); }
    const op = { duration: dur * 1000, delay: delay * 1000, easing: ease, fill: 'both' };
    const specs = [[[{ opacity: 1 }, { opacity: 0 }], op]];
    if (y || x || scale !== 1) specs.push([[{ transform: tf(base, 0, 0, 1) }, { transform: tf(base, x, y, scale) }], op]);
    return run(el, specs, fin, { lock: true });
  }

  function swap(a, b) {
    if (!a || a.hidden) { b.hidden = false; return enter(b, { y: 6 }); }
    return exit(a, { y: -4, hide: true }).then((ok) => { if (ok) { b.hidden = false; return enter(b, { y: 6 }); } return false; });
  }

  const SIDES = ['paddingTop', 'paddingBottom', 'marginTop', 'marginBottom', 'borderTopWidth', 'borderBottomWidth'];
  function collapse(el, { dur = 0.3, remove = false, hide = false, after } = {}) {
    const fin = (e) => { if (remove) e.remove(); else if (hide) e.hidden = true; if (after) after(e); };
    const h = el.offsetHeight;
    if (off() || !h || !el.isConnected) { cancel(el); fin(el); return Promise.resolve(true); }
    const cs = getComputedStyle(el), gap = parseFloat(getComputedStyle(el.parentNode).rowGap) || 0;
    const from = { height: h + 'px', minHeight: cs.minHeight }, to = { height: '0px', minHeight: '0px' };
    SIDES.forEach((k) => { from[k] = cs[k]; to[k] = '0px'; });
    to.marginBottom = -gap + 'px'; // the flex/grid gap of the removed row closes with it
    return run(el, [
      [[from, to], { duration: dur * 1000, easing: OUT(), fill: 'forwards' }],
      [[{ opacity: 1 }, { opacity: 0 }], { duration: dur * 600, easing: EASE_IN, fill: 'forwards' }],
    ], fin, { lock: true, hideOverflow: true });
  }

  function expand(el, { dur = 0.34 } = {}) {
    const h = el.offsetHeight;
    if (off() || !h) { cancel(el); return Promise.resolve(true); }
    const cs = getComputedStyle(el), from = { height: '0px', minHeight: '0px' }, to = { height: h + 'px', minHeight: cs.minHeight };
    SIDES.forEach((k) => { from[k] = '0px'; to[k] = cs[k]; });
    return run(el, [
      [[from, to], { duration: dur * 1000, easing: OUT(), fill: 'backwards' }],
      [[{ opacity: 0 }, { opacity: 1 }], { duration: dur * 700, delay: dur * 150, easing: OUT(), fill: 'backwards' }],
    ], null, { hideOverflow: true });
  }

  function flip(box, mutate, { sel = ':scope > *', enterNew = false } = {}) {
    const before = new Map();
    if (!off()) box.querySelectorAll(sel).forEach((c) => { const r = c.getBoundingClientRect(); if (c.dataset.id && r.width) before.set(c.dataset.id, r); });
    mutate();
    if (!before.size) return;
    box.querySelectorAll(sel).forEach((c) => {
      const r0 = before.get(c.dataset.id);
      if (!r0) { if (enterNew) enter(c, { y: 4, dur: 0.3 }); return; }
      const r1 = c.getBoundingClientRect(), dx = r0.left - r1.left, dy = r0.top - r1.top;
      if (Math.abs(dx) < 1 && Math.abs(dy) < 1) return;
      run(c, [[[{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'translate(0px, 0px)' }], { duration: 400, easing: SPRING(), fill: 'backwards' }]]);
    });
  }

  window.PillMotion = { exit, enter, swap, collapse, expand, flip, cancel, off };
})();
