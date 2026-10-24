// Pets, group "things": small objects with expressive details (campfire, lava lamp, lens, coffee mug, Newton's cradle, pulse monitor).
// Loaded after pets.js; every object is drawn from scratch (inline SVG + CSS), registered through PillPets.add (see the EXTENSION API header in pets.js).
// Motion rules: CSS transforms / opacity (the `d` property for the lens iris), hooks only set a custom property or one inline transform (energy / look).
(() => {
  'use strict';
  if (!window.PillPets || !window.PillPets.add) return;

  // accent mixing: the reaction colour (--pet-accent) takes `k` (lo | mid | hi) of a part; calm has --pet-k = 0, so the base colour stays
  const mix = (base, k = 'hi') => `color-mix(in srgb, var(--pet-accent) var(--t-${k}), ${base})`;
  const zp = 'M-2.4 -2.9h4.8l-4.8 5.8h4.8';
  const heartD = 'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z';
  const starD = 'M0-4.6L1.1-1.1L4.6 0L1.1 1.1L0 4.6L-1.1 1.1L-4.6 0L-1.1-1.1Z';

  // ---- shared props, anchored at (x, y) inside the box (drawn here instead of the host fx, because the objects fill the whole box) ----
  // z while asleep, "!" on attention, stars when done, dots when thinking (opt), paper on file (opt), hearts when petted
  const props = (api, x, y, o = {}) => {
    const z = [[0, 0, 0.7], [3, -5, 0.9], [-1, -11, 1.1]].map(([dx, dy, s], i) => `<g class="pz pz${i + 1}"><g transform="translate(${dx} ${dy}) scale(${s})">${api.ln('', zp, 1.5, '#e8ebff', 1.6)}</g></g>`).join('');
    const st = [[-4, -3, 1, 'spk'], [3, 3, 0.7, 'spk2'], [-6, 6, 0.55, 'spk']].map(([dx, dy, s, c], i) => `<g class="ps ps${i + 1}"><path class="o ${c}" d="${starD}" transform="translate(${dx} ${dy}) scale(${s})" stroke-width="1"/></g>`).join('');
    const hh = [[0, 0, 1.2], [4, -3, 1], [-3, -6, 0.8]].map(([dx, dy, s], i) => `<g class="ph ph${i + 1}"><path class="o" d="${heartD}" fill="${i === 1 ? '#ff9ab8' : '#ff7aa2'}" stroke-width="1.2" transform="translate(${dx} ${dy}) scale(${s})"/></g>`).join('');
    const th = [-5, 0, 5].map((dx, i) => `<g class="pd pd${i + 1}"><circle class="o pdot" cx="${dx}" cy="0" r="1.7" stroke-width="1"/></g>`).join('');
    return `<g class="px" transform="translate(${x} ${y})">
<g class="px-z">${z}</g>
<g class="px-excl"><g class="pex"><g transform="scale(.85)"><rect class="o exc" x="-1.8" y="-8" width="3.6" height="8" rx="1.8" stroke-width="1.2"/><circle class="o exc" cx="0" cy="3.4" r="2" stroke-width="1.2"/></g></g></g>
<g class="px-spark">${st}</g>
<g class="px-heart">${hh}</g>
${o.think ? `<g class="px-think">${th}</g>` : ''}
${o.file === false ? '' : `<g class="px-file"><g class="pfl"><g transform="scale(.8)"><path class="o pap" d="M-5 -6.5h7l4 4v10h-11z" stroke-width="1.4"/><path d="M-3 0.2h6M-3 2.8h6M-3 5.4h4" stroke="${mix('#7d8fb8')}" stroke-width="1" stroke-linecap="round"/></g></g></g>`}
</g>`;
  };

  // shared CSS of the props + rig rules; every selector starts with the pet's own prefix
  const common = (id, rig) => {
    const p = `.pet[data-pet="${id}"]`;
    return `
${p}:not(.act-pet) .pet-rig { animation: none; }
${rig ? `${p}[data-state="done"]:not(.act-pet) .pet-rig { animation: pet-jump 0.8s cubic-bezier(0.3, 0.7, 0.4, 1) 1; }
${p}[data-state="attention"]:not(.act-pet) .pet-rig { animation: pet-startle 0.45s ease-out 1; }` : ''}
${p} .px > g { display: none; }
${p}[data-state="sleep"] .px-z, ${p}[data-state="attention"] .px-excl, ${p}[data-state="done"] .px-spark, ${p}[data-state="thinking"] .px-think, ${p}[data-state="file"] .px-file, ${p}.act-pet .px-heart { display: inline; }
${p} .pz, ${p} .ph, ${p} .ps, ${p} .pex, ${p} .pd, ${p} .pfl { transform-box: fill-box; transform-origin: 50% 50%; }
${p}[data-state="sleep"] .pz { animation: th-z 3s ease-in-out infinite; }
${p}[data-state="sleep"] .pz2 { animation-delay: 1s; } ${p}[data-state="sleep"] .pz3 { animation-delay: 2s; }
${p}[data-state="attention"] .pex { animation: th-pop 0.35s cubic-bezier(0.34, 1.56, 0.64, 1) both, th-bob 0.7s ease-in-out 0.35s infinite alternate; transform-origin: 50% 100%; }
${p}[data-state="done"] .ps { animation: th-spark 1.7s ease-out infinite both; }
${p}[data-state="done"] .ps2 { animation-delay: 0.15s; } ${p}[data-state="done"] .ps3 { animation-delay: 0.3s; }
${p}[data-state="thinking"] .pd { animation: th-dot 1.2s ease-in-out infinite; }
${p}[data-state="thinking"] .pd2 { animation-delay: 0.2s; } ${p}[data-state="thinking"] .pd3 { animation-delay: 0.4s; }
${p}[data-state="file"] .pfl { animation: th-pop 0.35s cubic-bezier(0.34, 1.56, 0.64, 1) both, th-paper 1.8s ease-in-out 0.35s infinite alternate; }
${p}.act-pet .ph { animation: th-heart 1.2s ease-out both; }
${p}.act-pet .ph2 { animation-delay: 0.15s; } ${p}.act-pet .ph3 { animation-delay: 0.3s; }
${p} .exc { fill: ${mix('#ffb340')}; }
${p} .pdot { fill: ${mix('#e8ebff')}; }
${p} .spk { fill: ${mix('#ffe27a', 'mid')}; } ${p} .spk2 { fill: ${mix('#fff3b0', 'mid')}; }
${p} .pap { fill: ${mix('#f4f6fb', 'lo')}; stroke: ${mix('#17141f', 'hi')}; }
@keyframes th-z { 0% { transform: translate(0, 4px) scale(0.5); opacity: 0; } 20%, 75% { opacity: 1; } 100% { transform: translate(4px, -11px) scale(1.1); opacity: 0; } }
@keyframes th-pop { from { transform: scale(0); } to { transform: scale(1); } }
@keyframes th-bob { to { transform: translateY(-1.6px); } }
@keyframes th-spark { 0% { transform: scale(0) rotate(0); opacity: 0; } 30% { transform: scale(1.2) rotate(40deg); opacity: 1; } 100% { transform: scale(0.4) rotate(120deg); opacity: 0; } }
@keyframes th-dot { 0%, 100% { transform: scale(0.6); opacity: 0.45; } 40% { transform: scale(1.15); opacity: 1; } }
@keyframes th-paper { from { transform: translateY(0) rotate(-3deg); } to { transform: translateY(-1.4px) rotate(3deg); } }
@keyframes th-heart { 0% { transform: translate(0, 5px) scale(0.3); opacity: 0; } 25% { opacity: 1; } 100% { transform: translate(3px, -15px) scale(1.15); opacity: 0; } }
@keyframes th-shake { from { transform: rotate(-2.2deg); } to { transform: rotate(2.2deg); } }`;
  };

  // ======================================================== 1. campfire ========================================================
  {
    const id = 'campfire', p = `.pet[data-pet="${id}"]`;
    const FLAME = 'M24 10C26 17 34 22 34 31C34 37.5 29.5 41 24 41C18.5 41 14 37.5 14 31C14 24 20 22 20.5 15C22 17 23 14 24 10Z';
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Костёр', en: 'Campfire' },
      desc: { ru: 'Огонёк с глазками между брёвен: пляшет под музыку, дымит, когда думает', en: 'A little flame with eyes between two logs: dances to music, smokes while thinking' },
      w: 48, h: 48,
      svg: (api) => `
<g class="log log1"><rect class="o" x="7.5" y="39.4" width="33" height="5.2" rx="2.6" fill="#7a4a2c" transform="rotate(-9 24 42)"/></g>
<g class="fe"><g class="flame">
  <path class="o fo" d="${FLAME}"/>
  <path class="fi" d="${FLAME}" transform="translate(24 41) scale(.62) translate(-24 -41)"/>
  ${api.eyes([[21.4, 33.2], [26.6, 33.2]], 'dot', api.OL, 1.45)}
  <path d="M22.7 36.2q1.3 1.5 2.6 0" fill="none" stroke="${api.OL}" stroke-width="1.1" stroke-linecap="round"/>
</g></g>
<g class="ember-up"><circle class="ejp" style="--dx:-5px" cx="22" cy="24" r="1.2"/><circle class="ejp" style="--dx:4px;animation-delay:.5s" cx="27" cy="22" r="1"/><circle class="ejp" style="--dx:-1px;animation-delay:1s" cx="24" cy="26" r="1.1"/></g>
<g class="log log2"><rect class="o" x="7.5" y="39.4" width="33" height="5.2" rx="2.6" fill="#94603a" transform="rotate(9 24 42)"/><circle cx="38.6" cy="44.4" r="1.7" fill="#c98c5a" transform="rotate(9 24 42)"/></g>
<g class="emb"><circle class="e1" cx="15" cy="42.2" r="1.5"/><circle class="e1 e2" cx="25" cy="41.6" r="1.7"/><circle class="e1 e3" cx="33" cy="43.4" r="1.4"/></g>
<g class="smk smk-t"><circle class="sr" cx="24" cy="20" r="2.1"/><circle class="sr sr2" cx="24" cy="20" r="2.1"/><circle class="sr sr3" cx="24" cy="20" r="2.1"/></g>
<g class="smk smk-s"><circle class="sr" cx="24" cy="37" r="1.8"/><circle class="sr sr2" cx="24" cy="37" r="1.8"/></g>
${props(api, 40, 13)}`,
      css: common(id, true) + `
${p} .fe, ${p} .flame, ${p} .ejp, ${p} .e1, ${p} .sr { transform-box: fill-box; transform-origin: 50% 100%; }
${p} .fo { fill: ${mix('#ff8a2a')}; }
${p} .fi { fill: ${mix('#ffd45a', 'mid')}; }
${p} .ejp { fill: ${mix('#ffc24a')}; display: none; }
${p} .emb, ${p} .smk { display: none; }
${p} .e1 { fill: ${mix('#ff7a2a')}; transform-origin: 50% 50%; }
${p} .sr { fill: none; stroke: #d6d9ee; stroke-width: 1.3; opacity: 0; transform-origin: 50% 50%; }
${p} .flame { animation: cf-calm 2.2s ease-in-out infinite; }
${p}[data-state="music"] .flame { animation: cf-dance 0.5s ease-in-out infinite alternate; }
${p}[data-energy] .flame { animation: cf-sway 0.45s ease-in-out infinite alternate; }
${p}[data-state="thinking"] .flame { animation: cf-think 3s ease-in-out infinite; }
${p}[data-state="done"] .flame { animation: cf-done 1.1s ease-in-out infinite alternate; }
${p}[data-state="attention"] .flame { animation: cf-alert 0.32s ease-in-out infinite alternate; }
${p}[data-state="file"] .flame { animation: cf-calm 3s ease-in-out infinite; }
${p}[data-state="sleep"] .flame { animation: cf-sleep 4s ease-in-out infinite; }
${p}[data-state="done"] .ejp, ${p}[data-state="music"] .ejp { display: inline; animation: cf-ember 1.6s ease-out infinite; opacity: 0; }
${p}[data-state="sleep"] .emb, ${p}[data-state="thinking"] .smk-t, ${p}[data-state="sleep"] .smk-s { display: inline; }
${p}[data-state="sleep"] .e1 { animation: cf-glow 2.6s ease-in-out infinite alternate; }
${p}[data-state="sleep"] .e2 { animation-delay: 0.8s; } ${p}[data-state="sleep"] .e3 { animation-delay: 1.6s; }
${p}[data-state="thinking"] .sr { animation: cf-smoke 2.4s ease-out infinite; }
${p}[data-state="sleep"] .sr { animation: cf-smoke 4.2s ease-out infinite; }
${p} .sr2 { animation-delay: 0.8s !important; } ${p} .sr3 { animation-delay: 1.6s !important; }
${p}[data-state="sleep"] .log { opacity: 0.8; }
${p}.act-pet .flame { animation: cf-poke 0.5s ease-out; }
@keyframes cf-calm { 0%, 100% { transform: scale(0.94, 0.92); } 30% { transform: scale(0.99, 1) skewX(-3deg); } 60% { transform: scale(0.91, 0.96) skewX(2.5deg); } }
@keyframes cf-dance { from { transform: rotate(-9deg) scale(0.96, 1.12); } to { transform: rotate(9deg) scale(1.04, 0.9); } }
@keyframes cf-sway { from { transform: rotate(-8deg) scale(1, 1.02); } to { transform: rotate(8deg) scale(1, 1.02); } }
@keyframes cf-think { 0%, 100% { transform: scale(0.6, 0.6) rotate(-2deg); } 50% { transform: scale(0.66, 0.7) rotate(2deg); } }
@keyframes cf-done { from { transform: scale(1, 1); } to { transform: scale(1.07, 1.1); } }
@keyframes cf-alert { from { transform: scale(1.12, 1.28) rotate(-2.5deg); } to { transform: scale(1.02, 1.2) rotate(2.5deg); } }
@keyframes cf-sleep { 0%, 100% { transform: scale(0.26, 0.22); opacity: 0.7; } 50% { transform: scale(0.3, 0.27); opacity: 0.95; } }
@keyframes cf-poke { 0% { transform: scale(0.94, 0.92); } 35% { transform: scale(1.1, 1.22) rotate(-4deg); } 100% { transform: scale(0.94, 0.92); } }
@keyframes cf-ember { 0% { transform: translate(0, 4px) scale(0.5); opacity: 0; } 20% { opacity: 1; } 100% { transform: translate(var(--dx), -15px) scale(0.3); opacity: 0; } }
@keyframes cf-smoke { 0% { transform: translateY(0) scale(0.5); opacity: 0; } 20% { opacity: 0.85; } 100% { transform: translateY(-13px) scale(1.5); opacity: 0; } }
@keyframes cf-glow { from { opacity: 0.35; } to { opacity: 1; } }`,
      init(host, api) { host._fe = api.$('.fe'); },
      energy(host, e) { if (host._fe) host._fe.style.transform = e == null ? '' : `scale(${(1 + e * 0.1).toFixed(3)}, ${(0.86 + e * 0.5).toFixed(3)})`; },
    });
  }

  // ======================================================== 2. lava lamp ========================================================
  {
    const id = 'lava-lamp', p = `.pet[data-pet="${id}"]`;
    const blob = (cls, cx, r, rise, dur, delay) => `<g class="bb ${cls}" style="--rise:${rise};--dur:${dur}s;animation-delay:-${delay}s"><circle class="blob" cx="${cx}" cy="31" r="${r}"/></g>`;
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Лава-лампа', en: 'Lava lamp' },
      desc: { ru: 'Капли воска всплывают и сливаются, цвет стекла следует за реакцией', en: 'Wax blobs rise and merge, the glass takes the reaction colour' },
      w: 48, h: 48,
      svg: (api) => `
<g class="lamp">
  <path class="o lbase" d="M13.5 46.5L15.6 37.8H32.4L34.5 46.5Z" fill="#70707c"/>
  <path d="M16.4 42H31.6" stroke="#9a9aa8" stroke-width="1.2" stroke-linecap="round"/>
  <path class="o lcap" d="M19.4 4.5H28.6L30.2 10.2H17.8Z" fill="#70707c"/>
  <path class="o glass" d="M17.8 10.2H30.2L33.2 38.2H14.8Z"/>
  <g class="bl">
    <path class="blob pool" d="M15.4 38C15.4 34 18.6 32.2 24 32.2C29.4 32.2 32.6 34 32.6 38Z"/>
    ${blob('bb1', 22.8, 3.4, 15, 8, 0)}${blob('bb2', 26, 2.7, 10, 6, 2)}${blob('bb3', 23.4, 2.1, 17, 10, 5)}
  </g>
  <path d="M20.4 13.5L17.9 34" stroke="#fff" opacity=".3" stroke-width="1.6" stroke-linecap="round" fill="none"/>
</g>
${props(api, 40, 13, { think: true })}`,
      css: common(id, true) + `
${p} .lamp { transform-box: fill-box; transform-origin: 50% 100%; }
${p} .bl { transform-box: view-box; transform-origin: 24px 26px; }
${p} .pool, ${p} .bb { transform-box: fill-box; transform-origin: 50% 100%; }
${p} .glass { fill: ${mix('#1f6b45', 'lo')}; }
${p} .blob { fill: ${mix('#9be86a')}; }
${p} .bb { animation: lv-rise var(--dur, 8s) ease-in-out infinite; }
${p} .pool { animation: lv-pool 4s ease-in-out infinite; }
${p}[data-state="music"] .bb { animation-duration: calc(var(--dur) * 0.28); }
${p}[data-state="music"] .pool { animation: lv-pool-m 0.5s ease-in-out infinite alternate; }
${p}[data-energy] .pool { animation: none; transform: scale(1, calc(0.9 + var(--e, 0.4) * 0.5)); }
${p}[data-state="thinking"] .bb { animation-duration: calc(var(--dur) * 1.3); }
${p}[data-state="thinking"] .bl { animation: lv-swirl 2.4s ease-in-out infinite alternate; }
${p}[data-state="attention"] .bb { animation-duration: calc(var(--dur) * 0.18); }
${p}[data-state="attention"] .lamp { animation: th-shake 0.22s ease-in-out infinite alternate; }
${p}[data-state="done"] .bb { animation-duration: calc(var(--dur) * 0.8); }
${p}[data-state="done"] .glass { fill: ${mix('#2d8a57', 'mid')}; }
${p}[data-state="file"] .bb { animation-duration: calc(var(--dur) * 1.1); }
${p}[data-state="sleep"] .glass, ${p}[data-state="sleep"] .pool { opacity: 0.55; }
${p}[data-state="sleep"] .bb { animation: none; transform: scale(0.1); opacity: 0; }
${p}[data-state="sleep"] .pool { animation: lv-pool-s 5s ease-in-out infinite; }
${p}.act-pet .lamp { animation: th-shake 0.12s ease-in-out 4 alternate; }
@keyframes lv-rise { 0%, 100% { transform: translate(0, 0) scale(0.5, 0.4); } 12% { transform: translate(0.4px, calc(var(--rise) * -0.15px)) scale(0.9, 1.2); } 50% { transform: translate(-0.6px, calc(var(--rise) * -1px)) scale(1, 1); } 88% { transform: translate(0.3px, calc(var(--rise) * -0.15px)) scale(0.85, 1.15); } }
@keyframes lv-pool { 0%, 100% { transform: scale(1, 1); } 50% { transform: scale(1, 1.14); } }
@keyframes lv-pool-m { from { transform: scale(1.02, 0.92); } to { transform: scale(0.98, 1.3); } }
@keyframes lv-pool-s { 0%, 100% { transform: scale(1, 0.9); } 50% { transform: scale(1, 1); } }
@keyframes lv-swirl { from { transform: rotate(-10deg); } to { transform: rotate(10deg); } }`,
      energy(host, e) { if (e == null) host.style.removeProperty('--e'); else host.style.setProperty('--e', e.toFixed(2)); },
    });
  }

  // ======================================================== 3. lens ========================================================
  {
    const id = 'lens', p = `.pet[data-pet="${id}"]`;
    const CX = 24, CY = 25, RR = 16.5;
    const f = (n) => n.toFixed(2);
    // iris = a plate with a hexagonal hole (radius r) + six seam lines (the side of the hexagon continued to the rim): animated through the CSS `d` property
    const iris = (r) => {
      r = Math.max(r, 0.35);
      const V = Array.from({ length: 6 }, (_, k) => { const a = (-90 + 60 * k) * Math.PI / 180; return [CX + r * Math.cos(a), CY + r * Math.sin(a)]; });
      const plate = `M${CX - RR} ${CY}A${RR} ${RR} 0 1 0 ${CX + RR} ${CY}A${RR} ${RR} 0 1 0 ${CX - RR} ${CY}Z` + 'M' + V.map((v) => `${f(v[0])} ${f(v[1])}`).join('L') + 'Z';
      let seams = '';
      V.forEach((v, k) => {
        const w = V[(k + 1) % 6]; let ux = v[0] - w[0], uy = v[1] - w[1]; const l = Math.hypot(ux, uy); ux /= l; uy /= l;
        const px = v[0] - CX, py = v[1] - CY, b = px * ux + py * uy, s = -b + Math.sqrt(b * b - (px * px + py * py - RR * RR));
        seams += `M${f(v[0])} ${f(v[1])}L${f(v[0] + ux * s)} ${f(v[1] + uy * s)}`;
      });
      return { plate, seams };
    };
    const pp = (r) => `path("${iris(r).plate}")`, ss = (r) => `path("${iris(r).seams}")`;
    // keyframes with the same stops for the plate and for the seams
    const kf = (name, stops) => `@keyframes ${name}-p { ${stops.map(([t, r]) => `${t}% { d: ${pp(r)}; }`).join(' ')} }
@keyframes ${name}-s { ${stops.map(([t, r]) => `${t}% { d: ${ss(r)}; }`).join(' ')} }`;
    const run = (name, dur, rest) => `${p} .plate { animation: ${name}-p ${dur} ${rest}; } ${p} .seams { animation: ${name}-s ${dur} ${rest}; }`;
    const OPEN = 9;
    const o0 = iris(OPEN);
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Объектив', en: 'Lens' },
      desc: { ru: 'Живой зелёный глаз в объективе: диафрагма моргает и трепещет под музыку', en: 'A living green eye in a lens: the iris blinks and flutters to music' },
      w: 48, h: 48,
      svg: (api) => `
<circle class="o ring" cx="${CX}" cy="${CY}" r="21.2" fill="#2a2a33"/>
<circle cx="${CX}" cy="${CY}" r="19.2" fill="none" stroke="#3d3d48" stroke-width="2.6" stroke-dasharray="1.5 1.5"/>
<circle cx="${CX}" cy="${CY}" r="17" fill="#0f0f14"/>
<g class="ball"><circle class="eyeb" cx="${CX}" cy="${CY}" r="9.6"/>
  <g class="pup"><g class="e-open"><circle cx="${CX}" cy="${CY}" r="4.3" fill="#10170f"/><circle class="hl" style="--b:#fff" cx="${CX + 1.7}" cy="${CY - 1.8}" r="1.4" fill="#fff"/></g></g>
  <g class="e-happy"><path d="M${CX - 5} ${CY + 2.5}Q${CX} ${CY - 5} ${CX + 5} ${CY + 2.5}" fill="none" stroke="#10170f" stroke-width="2.2" stroke-linecap="round"/></g>
</g>
<path class="plate" fill-rule="evenodd" fill="#4a4a57" stroke="#2a2a33" stroke-width=".6" d="${o0.plate}"/>
<path class="seams" fill="none" stroke="#2a2a33" stroke-width="1" stroke-linecap="round" d="${o0.seams}"/>
<circle class="o" cx="${CX}" cy="${CY}" r="${RR + 1.7}" fill="none" stroke-width="3.2"/>
<circle class="rimhl" cx="${CX}" cy="${CY}" r="21.2" fill="none" stroke-width="1.1" opacity=".8"/>
<circle class="flash" cx="${CX}" cy="${CY}" r="21" fill="none" stroke="#fff" stroke-width="1.4" opacity="0"/>
${props(api, 41, 10, { think: true })}`,
      css: common(id, false) + `
${p} .eyeb { fill: ${mix('#6fd36a')}; }
${p} .rimhl { stroke: ${mix('#6a6a78', 'mid')}; }
${p} .pup, ${p} .flash { transform-box: fill-box; transform-origin: 50% 50%; }
${p} .pup { transition: transform 0.14s ease-out; }
${p} .e-happy { display: none; }
${p}[data-state="done"] .e-open, ${p}.act-pet .e-open { display: none; } ${p}[data-state="done"] .e-happy, ${p}.act-pet .e-happy { display: inline; }
${p} .e-shut { display: none; }
${run('ln-calm', '5.5s', 'ease-in-out infinite')}
${p}[data-state="music"] .plate { animation: ln-music-p 0.55s ease-in-out infinite alternate; } ${p}[data-state="music"] .seams { animation: ln-music-s 0.55s ease-in-out infinite alternate; }
${p}[data-energy] .plate, ${p}[data-energy] .seams { animation: none !important; }
${p}[data-state="thinking"] .plate { animation: ln-think-p 3s ease-in-out infinite alternate; } ${p}[data-state="thinking"] .seams { animation: ln-think-s 3s ease-in-out infinite alternate; }
${p}[data-state="thinking"] .pup { animation: ln-scan 1.8s ease-in-out infinite alternate; }
${p}[data-state="done"] .plate { animation: ln-done-p 0.6s ease-out forwards; } ${p}[data-state="done"] .seams { animation: ln-done-s 0.6s ease-out forwards; }
${p}[data-state="attention"] .plate { animation: ln-att-p 0.5s ease-out forwards; } ${p}[data-state="attention"] .seams { animation: ln-att-s 0.5s ease-out forwards; }
${p}[data-state="attention"] .flash { animation: ln-flash 0.9s ease-out infinite; }
${p}[data-state="file"] .plate { animation: ln-file-p 2.6s ease-in-out infinite alternate; } ${p}[data-state="file"] .seams { animation: ln-file-s 2.6s ease-in-out infinite alternate; }
${p}[data-state="file"] .pup { transform: translateY(2.4px); }
${p}[data-state="sleep"] .plate { animation: ln-sleep-p 0.8s ease-in forwards; } ${p}[data-state="sleep"] .seams { animation: ln-sleep-s 0.8s ease-in forwards; }
${p}.act-pet .plate { animation: ln-blink-p 0.5s ease-in-out; } ${p}.act-pet .seams { animation: ln-blink-s 0.5s ease-in-out; }
${kf('ln-calm', [[0, OPEN], [86, OPEN], [90, 0.4], [95, OPEN + 0.8], [100, OPEN]])}
${kf('ln-music', [[0, 6], [100, 11.5]])}
${kf('ln-think', [[0, 6.5], [100, 10.5]])}
${kf('ln-done', [[0, 0.4], [60, 12], [100, 10.8]])}
${kf('ln-att', [[0, 0.4], [30, 13.6], [100, 12.6]])}
${kf('ln-file', [[0, 8], [100, 10.2]])}
${kf('ln-sleep', [[0, OPEN], [100, 0.4]])}
${kf('ln-blink', [[0, OPEN], [45, 0.4], [100, OPEN]])}
@keyframes ln-scan { from { transform: translateX(-2.6px); } to { transform: translateX(2.6px); } }
@keyframes ln-flash { 0% { transform: scale(0.9); opacity: 0.9; } 100% { transform: scale(1.18); opacity: 0; } }`,
      init(host, api) { host._pl = api.$('.plate'); host._sm = api.$('.seams'); host._pu = api.$('.pup'); },
      energy(host, e) {
        if (!host._pl) return;
        if (e == null) { host._pl.style.removeProperty('d'); host._sm.style.removeProperty('d'); return; }
        const d = iris(5.5 + e * 7);
        host._pl.style.setProperty('d', `path("${d.plate}")`); host._sm.style.setProperty('d', `path("${d.seams}")`);
      },
      look(host, dx, dy) { if (host._pu) host._pu.style.transform = `translate(${(dx * 2.8).toFixed(1)}px, ${((dy || 0) * 2.2).toFixed(1)}px)`; },
    });
  }

  // ======================================================== 4. coffee ========================================================
  {
    const id = 'coffee', p = `.pet[data-pet="${id}"]`;
    const wisp = (cls, x, d) => `<path class="wisp ${cls}" d="${d.replace(/(-?\d+(?:\.\d+)?)/, (m) => +m + x)}"/>`; // x baked into the path: a CSS animation would replace a transform attribute
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Кофе', en: 'Coffee' },
      desc: { ru: 'Кружка с личиком: пар танцует под музыку, а в «Готово» складывается в сердечко', en: 'A mug with a little face: steam dances to music and folds into a heart when done' },
      w: 48, h: 48,
      svg: (api) => `
<g class="mugE"><g class="mugg">
  <path class="o handle" d="M35 30h2.4a4.6 4.6 0 0 1 0 9.2H35" fill="none" stroke-width="5.4" stroke="#17141f"/>
  <path d="M35 30h2.4a4.6 4.6 0 0 1 0 9.2H35" fill="none" stroke-width="2.6" stroke="#f3ece2" stroke-linecap="round"/>
  <rect class="o" x="15" y="25" width="20" height="21" rx="5.5" fill="#f3ece2"/>
  <rect class="stripe" x="15.6" y="27.6" width="18.8" height="2.4"/>
  <ellipse class="o" cx="25" cy="25" rx="10" ry="2.7" fill="#f3ece2"/><ellipse cx="25" cy="25.1" rx="8.2" ry="1.9" fill="#5a3520"/>
  <circle class="cheek" cx="18.8" cy="38.2" r="1.6"/><circle class="cheek" cx="31.2" cy="38.2" r="1.6"/>
  ${api.eyes([[21.4, 34.2], [28.6, 34.2]], 'dot', api.OL, 1.65)}
  <path d="M23 37.6q2 2.4 4 0" fill="none" stroke="${api.OL}" stroke-width="1.2" stroke-linecap="round"/>
</g></g>
<g class="steam">${wisp('w1', 0, 'M20 21c-3-3 3-5 0-9')}${wisp('w2', 5, 'M20 22c3-3-3-6 0-11')}${wisp('w3', 10, 'M20 21c-3-3 3-5 0-9')}</g>
<g class="tdots"><circle class="td td1" cx="23" cy="17" r="1.5"/><circle class="td td2" cx="26" cy="12" r="1.9"/><circle class="td td3" cx="25" cy="6" r="2.4"/></g>
<g class="sheart"><g class="shh"><path class="o shp" d="${'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z'}" transform="translate(25 13) scale(2.3)" stroke-width="1"/></g></g>
<g class="cookie"><g class="ck"><circle class="o" cx="7.2" cy="42" r="4.7" fill="#c98a4b"/><circle cx="5.7" cy="40.8" r=".9" fill="#4a2a14"/><circle cx="8.6" cy="42.6" r=".9" fill="#4a2a14"/><circle cx="6.3" cy="44" r=".8" fill="#4a2a14"/></g></g>
${props(api, 42, 14, { file: false })}`,
      css: common(id, true) + `
${p} .mugE, ${p} .mugg, ${p} .wisp, ${p} .steam, ${p} .shh, ${p} .ck, ${p} .td { transform-box: fill-box; transform-origin: 50% 100%; }
${p} .mugE { transform-origin: 50% 100%; }
${p} .stripe { fill: ${mix('#e8a040')}; }
${p} .cheek { fill: ${mix('#f2a0a8', 'mid')}; opacity: 0.8; }
${p} .wisp { fill: none; stroke: #e8ebff; stroke-width: 1.7; stroke-linecap: round; opacity: 0; animation: cof-steam 2.6s ease-in-out infinite; transform-origin: 50% 100%; }
${p} .w2 { animation-delay: 0.9s; } ${p} .w3 { animation-delay: 1.7s; }
${p} .td { fill: ${mix('#e8ebff')}; stroke: #17141f; stroke-width: 1; transform-origin: 50% 50%; }
${p} .shp { fill: ${mix('#ff8fb0', 'lo')}; }
${p} .tdots, ${p} .sheart, ${p} .cookie { display: none; }
${p}[data-state="music"] .mugg { animation: cof-bob 0.55s ease-in-out infinite alternate; }
${p}[data-state="music"] .wisp { animation: cof-dance 0.55s ease-in-out infinite alternate; opacity: 0.9; transform: rotate(-14deg); }
${p}[data-state="music"] .w2 { animation-direction: alternate-reverse; } ${p}[data-state="music"] .w3 { animation-delay: -0.2s; }
${p}[data-state="thinking"] .steam, ${p}[data-state="done"] .steam, ${p}[data-state="sleep"] .steam { display: none; }
${p}[data-state="thinking"] .tdots, ${p}[data-state="done"] .sheart, ${p}[data-state="file"] .cookie { display: inline; }
${p}[data-state="thinking"] .td { animation: th-dot 1.3s ease-in-out infinite; }
${p}[data-state="thinking"] .td2 { animation-delay: 0.25s; } ${p}[data-state="thinking"] .td3 { animation-delay: 0.5s; }
${p}[data-state="done"] .shh { animation: cof-heart 1.4s ease-in-out infinite; }
${p}[data-state="attention"] .wisp { animation: cof-shoot 0.5s ease-out infinite; }
${p}[data-state="attention"] .w2 { animation-delay: 0.12s; } ${p}[data-state="attention"] .w3 { animation-delay: 0.25s; }
${p}[data-state="attention"] .mugg { animation: th-shake 0.1s ease-in-out infinite alternate; }
${p}[data-state="file"] .ck { animation: th-pop 0.35s cubic-bezier(0.34, 1.56, 0.64, 1) both, th-paper 1.8s ease-in-out 0.35s infinite alternate; }
${p}[data-state="sleep"] .mugg { transform: rotate(4deg); }
${p}[data-state="sleep"] .stripe { opacity: 0.7; }
${p}.act-pet .mugg { animation: cof-wiggle 0.5s ease-in-out; }
${p}[data-energy] .wisp { transform: rotate(-14deg) scaleY(calc(0.7 + var(--e, 0.5) * 0.9)); animation-name: cof-dance-e; }
@keyframes cof-steam { 0% { transform: translate(0, 3px) scaleY(0.7); opacity: 0; } 30% { opacity: 0.85; } 100% { transform: translate(1.5px, -5px) scaleY(1.1); opacity: 0; } }
@keyframes cof-dance { from { transform: rotate(-14deg) scaleY(0.9); } to { transform: rotate(14deg) scaleY(1.15); } }
@keyframes cof-dance-e { from { transform: rotate(-14deg) scaleY(calc(0.7 + var(--e, 0.5) * 0.9)); } to { transform: rotate(14deg) scaleY(calc(0.7 + var(--e, 0.5) * 0.9)); } }
@keyframes cof-bob { from { transform: translateY(0) rotate(-1.5deg); } to { transform: translateY(-1.4px) rotate(1.5deg); } }
@keyframes cof-heart { 0%, 100% { transform: translateY(1px) scale(0.85); opacity: 0.85; } 50% { transform: translateY(-2.5px) scale(1.12); opacity: 1; } }
@keyframes cof-shoot { 0% { transform: translateY(2px) scaleY(0.6); opacity: 0; } 25% { opacity: 1; } 100% { transform: translateY(-9px) scaleY(1.7); opacity: 0; } }
@keyframes cof-wiggle { 0%, 100% { transform: rotate(0); } 25% { transform: rotate(-6deg); } 60% { transform: rotate(5deg); } }`,
      init(host, api) { host._me = api.$('.mugE'); },
      energy(host, e) {
        if (e == null) { host.style.removeProperty('--e'); if (host._me) host._me.style.transform = ''; return; }
        host.style.setProperty('--e', e.toFixed(2));
        if (host._me) host._me.style.transform = `translateY(${(-e * 2.2).toFixed(2)}px)`;
      },
    });
  }

  // ======================================================== 5. Newton's cradle ========================================================
  {
    const id = 'newton', p = `.pet[data-pet="${id}"]`;
    const CXS = [17.6, 28.8, 40, 51.2, 62.4], PY = 9, BY = 36, BR = 5.6;
    const ball = (i, cx) => `<g class="nb nb${i + 1}" style="transform-origin:${cx}px ${PY}px"><path d="M${cx} ${PY}V${BY - BR + 0.6}" stroke="#a9a9b8" stroke-width="1" stroke-linecap="round" fill="none"/><circle class="o ballc" cx="${cx}" cy="${BY}" r="${BR}"/><path d="M${cx - 3.2} ${BY - 2}a3.6 3.6 0 0 1 2.4-2.6" fill="none" stroke="#fff" stroke-width="1.3" stroke-linecap="round" opacity=".7"/></g>`;
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Маятник Ньютона', en: "Newton's cradle" },
      desc: { ru: 'Офисная игрушка вместо персонажа: крайние шарики качаются в такт реакции', en: 'An office toy instead of a character: the end balls swing to the mood' },
      w: 90, h: 48,
      svg: (api) => `
<path d="M7 9V44M73 9V44" stroke="#8b8b99" stroke-width="1.8" stroke-linecap="round" fill="none"/>
<path d="M5 9.4H75" stroke="#a3a3b2" stroke-width="2.6" stroke-linecap="round"/>
<path d="M3.5 45.2H76.5" stroke="#6f6f7c" stroke-width="3" stroke-linecap="round"/>
<g class="halos">${CXS.map((cx) => `<circle class="halo" cx="${cx}" cy="${BY}" r="8.6"/>`).join('')}</g>
${CXS.map((cx, i) => ball(i, cx)).join('')}
<g class="nface">${api.eyes([[37.8, BY - 0.8], [42.2, BY - 0.8]], 'dot', api.OL, 1.05)}<path d="M38.8 ${BY + 1.8}q1.2 1.2 2.4 0" fill="none" stroke="${api.OL}" stroke-width=".9" stroke-linecap="round"/></g>
${props(api, 84, 13, { think: true })}`,
      css: common(id, false) + `
${p} .nb { transform-box: view-box; }
${p} .ballc { fill: ${mix('#8fbf4a')}; }
${p} .halo { fill: ${mix('#8fbf4a')}; opacity: 0; transform-box: fill-box; transform-origin: 50% 50%; }
${p} { --nd: 2.6s; --amp: 1; }
${p} .nb1 { animation: nw-l var(--nd) linear infinite; }
${p} .nb5 { animation: nw-r var(--nd) linear infinite; }
${p}[data-state="music"] { --nd: 0.9s; }
${p}[data-state="thinking"] { --nd: 4.4s; --amp: 0.75; }
${p}[data-state="attention"] { --nd: 0.7s; }
${p}[data-state="file"] { --nd: 3.2s; --amp: 0.8; }
${p}[data-state="done"] .nb1, ${p}[data-state="done"] .nb5, ${p}[data-state="sleep"] .nb1, ${p}[data-state="sleep"] .nb5 { animation: none; }
${p}[data-state="done"] .halo { animation: nw-halo 1.2s ease-in-out infinite alternate; }
${p}[data-state="done"] .nb1 { animation: nw-settle 1.6s ease-out 1 forwards; }
${p}[data-state="sleep"] .nb, ${p}[data-state="sleep"] .halos { opacity: 0.55; }
${p}.act-pet .nb2, ${p}.act-pet .nb3, ${p}.act-pet .nb4 { animation: nw-jiggle 0.45s ease-in-out; }
@keyframes nw-l { 0% { transform: rotate(calc(var(--amp) * 24deg)); animation-timing-function: cubic-bezier(0.5, 0, 1, 0.6); } 25%, 75% { transform: rotate(0); animation-timing-function: cubic-bezier(0, 0.4, 0.5, 1); } 100% { transform: rotate(calc(var(--amp) * 24deg)); } }
@keyframes nw-r { 0%, 25% { transform: rotate(0); animation-timing-function: cubic-bezier(0, 0.4, 0.5, 1); } 50% { transform: rotate(calc(var(--amp) * -24deg)); animation-timing-function: cubic-bezier(0.5, 0, 1, 0.6); } 75%, 100% { transform: rotate(0); } }
@keyframes nw-settle { 0% { transform: rotate(18deg); } 25% { transform: rotate(0); } 45% { transform: rotate(5deg); } 65% { transform: rotate(0); } 100% { transform: rotate(0); } }
@keyframes nw-halo { from { opacity: 0.08; transform: scale(0.85); } to { opacity: 0.4; transform: scale(1.1); } }
@keyframes nw-jiggle { 0%, 100% { transform: rotate(0); } 30% { transform: rotate(-3deg); } 65% { transform: rotate(3deg); } }`,
      energy(host, e) { if (e == null) host.style.removeProperty('--amp'); else host.style.setProperty('--amp', (0.45 + e * 0.6).toFixed(2)); },
    });
  }

  // ======================================================== 6. pulse ========================================================
  {
    const id = 'pulse', p = `.pet[data-pet="${id}"]`;
    const Y = 26;
    // ECG generator: beats start at x0 (width 34 * s): P wave, QRS spike of height amp, T wave; the line ends at x1 (the heart sits to the right)
    const ecg = (starts, amp, s = 1, x1 = 98) => {
      let d = `M4 ${Y}`;
      for (const x0 of starts) {
        const k = (v) => +(x0 + v * s).toFixed(1);
        d += `H${k(0)}H${k(5)}Q${k(7.5)} ${Y - 4} ${k(10)} ${Y}H${k(14)}L${k(15.5)} ${Y + 2}L${k(18)} ${Y - amp}L${k(20.5)} ${Y + amp * 0.5}L${k(22)} ${Y}H${k(26)}Q${k(30)} ${Y - 8} ${k(34)} ${Y}`;
      }
      return d + `H${x1}`;
    };
    const PATHS = {
      calm: ecg([10, 54], 15),
      music: ecg([6, 34, 62], 17, 0.8),
      thinking: ecg([66], 7, 0.55),
      done: ecg([36], 21, 1.3),
      attention: ecg([6, 28, 50, 72], 19, 0.6, 96),
      file: ecg([40], 6, 0.9),
      sleep: `M4 ${Y}Q10 ${Y - 3} 16 ${Y}T28 ${Y}T40 ${Y}T52 ${Y}T64 ${Y}T76 ${Y}T88 ${Y}T98 ${Y}`,
    };
    const pathPair = (k) => `<g class="ln ln-${k}"><path class="base" d="${PATHS[k]}"/><path class="run" pathLength="100" d="${PATHS[k]}"/></g>`;
    window.PillPets.add({
      id, group: 'things', fx: false,
      name: { ru: 'Пульс', en: 'Pulse' },
      desc: { ru: 'Кардиомонитор: сердце бьётся в такт реакции, линия бежит слева направо', en: 'A heart monitor: the heart beats with the mood, the trace runs left to right' },
      w: 120, h: 48,
      svg: (api) => `
<g class="ecgg">${Object.keys(PATHS).map(pathPair).join('')}</g>
<circle class="o tdot" cx="4" cy="${Y}" r="2.3" stroke-width="1"/>
<g transform="translate(108 27)"><g class="hrtE"><g class="hrt"><path class="o hp" d="${'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z'}" transform="scale(1.9)" stroke-width="1"/></g></g></g>
${props(api, 108, 10)}`,
      css: common(id, false) + `
${p} .ecgg { transform-box: view-box; transform-origin: 0 ${Y}px; }
${p} .hrt, ${p} .hrtE { transform-box: fill-box; transform-origin: 50% 55%; }
${p} .ln, ${p} .tdot { display: none; }
${p} { --pd: 2.8s; }
${p} .base, ${p} .run { fill: none; stroke: ${mix('#7fdc6a')}; stroke-width: 2.4; stroke-linecap: round; stroke-linejoin: round; }
${p} .base { opacity: 0.38; }
${p} .run { stroke-dasharray: 26 74; animation: pu-run var(--pd) linear infinite; }
${p} .hp, ${p} .tdot { fill: ${mix('#7fdc6a')}; }
${p} .hrt { animation: pu-beat var(--pd) ease-out infinite; }
${p}[data-state="calm"] .ln-calm, ${p}[data-state="music"] .ln-music, ${p}[data-state="thinking"] .ln-thinking, ${p}[data-state="done"] .ln-done, ${p}[data-state="attention"] .ln-attention, ${p}[data-state="file"] .ln-file, ${p}[data-state="sleep"] .ln-sleep { display: inline; }
${p}[data-state="music"] { --pd: 1.1s; }
${p}[data-state="thinking"] { --pd: 3.4s; }
${p}[data-state="attention"] { --pd: 0.62s; }
${p}[data-state="file"] { --pd: 2.6s; }
${p}[data-state="sleep"] { --pd: 5s; }
${p}[data-state="sleep"] .ecgg { transform: scaleY(0.9); opacity: 0.6; }
${p}[data-state="sleep"] .run { stroke-dasharray: 100 0; animation: none; opacity: 0.7; }
${p}[data-state="sleep"] .base { opacity: 0; }
${p}[data-state="sleep"] .hrt { animation-name: pu-beat-s; }
${p}[data-state="thinking"] .tdot { display: inline; animation: pu-dot 3.4s ease-in-out infinite; }
${p}[data-state="thinking"] .run { stroke-dasharray: 18 82; }
${p}[data-state="done"] .run { stroke-dasharray: 100 0; animation: pu-draw 1.2s ease-out 1; }
${p}[data-state="done"] .hrt { animation: pu-big 1.4s ease-out 1, pu-beat 2.4s ease-out 1.4s infinite; }
${p}[data-state="done"] .base { opacity: 0.2; }
${p}[data-energy] .run { animation: none; stroke-dasharray: 100 0; }
${p}[data-energy] .hrt { animation: none; }
${p}.act-pet .hrt { animation: pu-big 0.6s ease-out 1; }
@keyframes pu-run { from { stroke-dashoffset: 26; } to { stroke-dashoffset: -74; } }
@keyframes pu-draw { from { stroke-dasharray: 0 100; } to { stroke-dasharray: 100 0; } }
@keyframes pu-beat { 0%, 72% { transform: scale(1); } 80% { transform: scale(1.38); } 88% { transform: scale(1.04); } 94% { transform: scale(1.2); } 100% { transform: scale(1); } }
@keyframes pu-beat-s { 0%, 100% { transform: scale(0.9); } 50% { transform: scale(1); } }
@keyframes pu-big { 0% { transform: scale(1); } 20% { transform: scale(1.7); } 55% { transform: scale(1.1); } 100% { transform: scale(1); } }
@keyframes pu-dot { 0% { transform: translateX(0); opacity: 1; } 62% { transform: translateX(62px); opacity: 1; } 76%, 100% { transform: translateX(66px); opacity: 0; } }`,
      init(host, api) { host._eg = api.$('.ecgg'); host._hr = api.$('.hrtE'); },
      energy(host, e) {
        if (!host._eg) return;
        host._eg.style.transform = e == null ? '' : `scaleY(${(0.55 + e * 1.1).toFixed(2)})`;
        host._hr.style.transform = e == null ? '' : `scale(${(1 + e * 0.45).toFixed(2)})`;
      },
    });
  }
})();
