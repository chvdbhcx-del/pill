// Pets, group "creatures": hamster in a wheel, one-line stickman, koi, snail. Original art, drawn from scratch for the Pill capsule (PillPets.add API, see pets.js).
// Shared look: --ac = the reaction tint (calm = a fresh green), used for the wheel ring, the stickman line, koi spots and the snail shell; the animals keep their own colours.
// Motion: the continuous travel (wheel spin, swimming, crawling) is integrated in the shared rAF tick so speed changes ease instead of jumping; everything else is CSS
// transforms / the SVG path property "d" (limbs bend as the same line). Own props (z, "!", sparkles, hearts, thinking dots) so they can sit where each creature has room.
(() => {
  'use strict';
  const PP = window.PillPets;
  if (!PP || !PP.add) return;
  const OL = '#17141f';
  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const still = () => document.documentElement.classList.contains('rm'); // the static preview switch; the app itself stops the tick under reduced motion
  const HM_V = { calm: 55, music: 230, thinking: 115, done: 0, attention: 520, file: 75, sleep: 0 }; // per-frame lookup tables hoisted: no object allocation in tick
  const KO_V = { calm: 5, music: 11, thinking: 3, done: 11, attention: 0, file: 6, sleep: 0 }; // per-frame lookup tables hoisted: no object allocation in tick
  const SN_V = { calm: 2.2, music: 1.6, thinking: 1, done: 5, attention: 0, file: 2.2, sleep: 0 }; // per-frame lookup tables hoisted: no object allocation in tick
  const dtOf = (h, t) => { const d = clamp((t - (h._t || t)) / 1000, 0, 0.1); h._t = t; return d; };
  const enT = (h, f) => { if (h._en) h._en.style.transform = f || ''; };
  const dA = (p) => 'M' + p.map((q) => q.join(' ')).join('L'); // polyline "d" for the attribute
  const dp = (p) => `path("${dA(p)}")`; // the same as a CSS d: value
  const kf = (n, a, b) => `@keyframes ${n}{from{d:${dp(a)}}to{d:${dp(b)}}}`;

  // ---- own props: z, "!", sparkles, hearts (all anchored at one point) + thinking dots; the host shows / animates them by data-state ----
  const ZP = 'M-2.4 -2.9h4.8l-4.8 5.8h4.8', ST = 'M0-4.6L1.1-1.1L4.6 0L1.1 1.1L0 4.6L-1.1 1.1L-4.6 0L-1.1-1.1Z', HP = 'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z';
  const fx = (x, y) => `<g class="cp" transform="translate(${x} ${y})">
<g class="cp-z">${[[0, 0, 0.85], [3.5, -5.5, 1.05], [-1, -12, 1.3]].map(([a, b, s], i) => `<g class="cp-zz cp-z${i}"><path transform="translate(${a} ${b}) scale(${s})" d="${ZP}" fill="none" stroke="#e8ebff" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></g>`).join('')}</g>
<g class="cp-ex"><g class="cp-exi"><circle class="cp-exc" r="5.8"/><rect x="-1.05" y="-3.6" width="2.1" height="4.5" rx="1.05" fill="#241606"/><circle cy="3.2" r="1.2" fill="#241606"/></g></g>
<g class="cp-sp">${[[0, -4, 1.1], [6, 5, 0.85], [-5, 8, 0.7]].map(([a, b, s], i) => `<g class="cp-sk cp-s${i}"><path transform="translate(${a} ${b}) scale(${s})" d="${ST}"/></g>`).join('')}</g>
<g class="cp-ht">${[[0, 0, 1.2, '#ff7aa2'], [5, -3, 1, '#ff9ab8'], [-4, -6, 0.8, '#ff7aa2']].map(([a, b, s, c], i) => `<g class="cp-hr cp-h${i}"><path transform="translate(${a} ${b}) scale(${s})" d="${HP}" fill="${c}"/></g>`).join('')}</g>
</g>`;
  const dots = (x, y, gap = 5) => `<g class="cp-dots" transform="translate(${x} ${y})">${[-1, 0, 1].map((k, i) => `<circle class="cp-d cp-d${i}" cx="${k * gap}" r="1.9"/>`).join('')}</g>`;
  const common = (id) => {
    const S = `.pet[data-pet="${id}"]`;
    return `
${S} { --ac: color-mix(in srgb, var(--pet-accent) calc(var(--pet-k) * 100%), #8fbf4a); }
${S}[data-state="sleep"] { --ac: #6f6f7a; }
${S} .pet-rig { animation: none; }
${S} .en { transform-box: fill-box; transform-origin: 50% 100%; }
${S} .cp > g, ${S} .cp-dots { display: none; }
${S}[data-state="sleep"] .cp-z, ${S}[data-state="attention"] .cp-ex, ${S}[data-state="done"] .cp-sp, ${S}[data-state="thinking"] .cp-dots, ${S}.act-pet .cp-ht { display: inline; }
${S} .cp-zz, ${S} .cp-exi, ${S} .cp-sk, ${S} .cp-hr, ${S} .cp-d { transform-box: fill-box; transform-origin: 50% 50%; }
${S} .cp-exc { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #ffb340); }
${S} .cp-sk path { fill: color-mix(in srgb, var(--pet-accent) var(--t-mid), #ffe27a); }
${S} .cp-d { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #e8ebff); }
${S}[data-state="sleep"] .cp-zz { animation: cr-z 3s ease-in-out infinite; }
${S}[data-state="sleep"] .cp-z1 { animation-delay: 1s; } ${S}[data-state="sleep"] .cp-z2 { animation-delay: 2s; }
${S}[data-state="attention"] .cp-exi { transform-origin: 50% 100%; animation: cr-pop 0.35s cubic-bezier(0.34, 1.56, 0.64, 1) both, cr-bob 0.7s ease-in-out 0.35s infinite alternate; }
${S}[data-state="done"] .cp-sk { animation: cr-spark 1.1s ease-out both; }
${S}[data-state="done"] .cp-s1 { animation-delay: 0.12s; } ${S}[data-state="done"] .cp-s2 { animation-delay: 0.24s; }
${S}[data-state="thinking"] .cp-d { animation: cr-dot 1.2s ease-in-out infinite; }
${S}[data-state="thinking"] .cp-d1 { animation-delay: 0.2s; } ${S}[data-state="thinking"] .cp-d2 { animation-delay: 0.4s; }
${S}.act-pet .cp-hr { animation: cr-heart 1.2s ease-out both; }
${S}.act-pet .cp-h1 { animation-delay: 0.15s; } ${S}.act-pet .cp-h2 { animation-delay: 0.3s; }
`;
  };
  const SHARED_KF = `
@keyframes cr-z { 0% { transform: translate(0, 4px) scale(0.5); opacity: 0; } 20% { opacity: 1; } 75% { opacity: 1; } 100% { transform: translate(calc(var(--d, -1) * 7px), -12px) scale(1.1); opacity: 0; } }
@keyframes cr-pop { from { transform: scale(0); } to { transform: scale(1); } }
@keyframes cr-bob { to { transform: translateY(-1.6px); } }
@keyframes cr-spark { 0% { transform: scale(0) rotate(0); opacity: 0; } 30% { transform: scale(1.2) rotate(40deg); opacity: 1; } 100% { transform: scale(0.4) rotate(120deg); opacity: 0; } }
@keyframes cr-dot { 0%, 100% { transform: scale(0.6); opacity: 0.45; } 40% { transform: scale(1.15); opacity: 1; } }
@keyframes cr-heart { 0% { transform: translate(0, 5px) scale(0.3); opacity: 0; } 25% { opacity: 1; } 100% { transform: translate(calc(var(--d, -1) * 5px), -15px) scale(1.15); opacity: 0; } }
@keyframes cr-wig { 0%, 100% { transform: rotate(0); } 30% { transform: rotate(-9deg); } 65% { transform: rotate(7deg); } }
`;
  // a tiny paper sheet (file reaction); the tint reaches the folded corner and the text lines
  const paper = (cls, tf) => `<g class="${cls}" transform="${tf}"><g class="${cls}-i"><path d="M-3.6 -4.8h5.2l2.6 2.6v7.4h-7.8z" fill="#f4f6fb" stroke="${OL}" stroke-width=".8" stroke-linejoin="round"/><path class="pp-c" d="M1.6 -4.8v2.6h2.6z" fill="#cfd6e6"/><path class="pp-l" d="M-2 -0.6h4.2M-2 1.6h4.2M-2 3.8h2.6" stroke="#7d8fb8" stroke-width=".9" stroke-linecap="round"/></g></g>`;
  const paperCss = (S) => `${S} .pp-c { fill: color-mix(in srgb, var(--pet-accent) var(--t-mid), #cfd6e6); } ${S} .pp-l { stroke: color-mix(in srgb, var(--pet-accent) var(--t-hi), #7d8fb8); }`;

  // ================= 1. hamster in a wheel (48 x 48) =================
  {
    const S = '.pet[data-pet="hamster"]';
    const spokes = Array.from({ length: 8 }, (_, i) => { const a = i * Math.PI / 4; return `M${(24 + 2.4 * Math.cos(a)).toFixed(2)} ${(22 + 2.4 * Math.sin(a)).toFixed(2)}L${(24 + 15.8 * Math.cos(a)).toFixed(2)} ${(22 + 15.8 * Math.sin(a)).toFixed(2)}`; }).join('');
    const legs = [[17.5, 'a'], [21.8, 'b'], [26.6, 'a'], [30.2, 'b']].map(([x, p]) => `<g transform="translate(${x} 34.8)"><g class="hm-lg hm-l${p}"><path d="M0 0v3" fill="none" stroke="#d99d62" stroke-width="2.7" stroke-linecap="round"/></g></g>`).join('');
    PP.add({
      id: 'hamster', group: 'creatures',
      name: { ru: 'Хомяк в колесе', en: 'Hamster' },
      desc: { ru: 'Крутит колесо тем резвее, чем занятее остров', en: 'Spins its wheel faster the busier the island is' },
      w: 48, h: 48, fx: false,
      svg: (api) => `
<path d="M24 22L14.5 46.6M24 22L33.5 46.6M11.5 46.6h6M30.5 46.6h6" fill="none" stroke="#70707b" stroke-width="2.6" stroke-linecap="round"/>
<g class="hm-wheel">
  <path d="${spokes}" fill="none" stroke="#8d8d98" stroke-width="1.05" stroke-linecap="round" opacity=".85"/>
  <circle class="hm-ring" cx="24" cy="22" r="17" fill="none" stroke-width="2.7"/>
  <circle class="hm-knob" cx="24" cy="5" r="2"/><circle class="hm-knob" cx="24" cy="39" r="2"/>
</g>
<circle cx="24" cy="22" r="2.3" fill="#9a9aa6"/>
<g class="en"><g class="hm-hop"><g class="hm-sl"><g class="hm-in">
  <g class="hm-lgs">${legs}</g>
  <ellipse cx="22" cy="31" rx="8.3" ry="5.7" fill="#e9b078"/>
  <ellipse cx="18.4" cy="28.4" rx="4.4" ry="2.6" fill="#d58f4f" transform="rotate(-12 18.4 28.4)"/>
  <ellipse cx="24.5" cy="33.4" rx="5.6" ry="2.4" fill="#fbe5ca"/>
  <g class="hm-paws"><path d="M27.4 32L31.4 24.6M30 32.8L35.2 25.8" fill="none" stroke="#efb985" stroke-width="2.6" stroke-linecap="round"/></g>
  ${paper('hm-paper', 'translate(33.4 21.6) rotate(8)')}
  <g class="hm-head">
    <circle cx="27.6" cy="24.8" r="2.3" fill="#f0a0ac"/>
    <ellipse cx="30.4" cy="28.8" rx="5.9" ry="5.1" fill="#f0bb88"/>
    <ellipse cx="32.8" cy="31.2" rx="2.8" ry="1.9" fill="#fbe5ca"/>
    <circle cx="35.8" cy="29.4" r="1.3" fill="#ee7f98"/>
    ${api.eyes([[32.3, 27.2]], 'dot', OL, 1.6)}
  </g>
</g></g></g></g>
${fx(40.5, 9)}${dots(24, 2)}`,
      css: SHARED_KF + common('hamster') + paperCss(S) + `
${S} { --rs: 0.8s; }
${S}[data-state="music"] { --rs: 0.275s; } ${S}[data-state="thinking"] { --rs: 0.55s; } ${S}[data-state="attention"] { --rs: 0.15s; } ${S}[data-state="file"] { --rs: 0.7s; }
${S} .hm-ring { stroke: var(--ac); }
${S} .hm-knob { fill: color-mix(in srgb, var(--ac) 45%, #fff); }
${S} .hm-hop, ${S} .hm-sl, ${S} .hm-in { transform-box: fill-box; transform-origin: 50% 100%; }
${S} .hm-sl { transition: transform 0.35s ease; }
${S} .hm-head { transform-box: fill-box; transform-origin: 30% 95%; transition: transform 0.3s ease; }
${S} .hm-in { animation: hm-bob var(--rs) ease-in-out infinite alternate; }
${S} .hm-lg { transform-box: fill-box; transform-origin: 50% 0; animation: hm-leg var(--rs) ease-in-out infinite alternate; }
${S} .hm-lb { animation-direction: alternate-reverse; }
${S} .hm-paws, ${S} .hm-paper { display: none; }
${S}[data-state="done"] .hm-paws, ${S}[data-state="file"] .hm-paws, ${S}[data-state="file"] .hm-paper { display: inline; }
${S}[data-state="done"] .hm-lgs, ${S}[data-state="sleep"] .hm-lgs { display: none; }
${S}[data-state="file"] .hm-paper-i { animation: cr-bob 0.7s ease-in-out infinite alternate; }
${S}[data-state="music"] .hm-head { animation: hm-nod 0.55s ease-in-out infinite alternate; }
${S}[data-state="done"] .hm-hop { animation: hm-hop 0.9s cubic-bezier(0.3, 0.7, 0.4, 1) 1; }
${S}[data-state="done"] .hm-in { animation: hm-bob 0.45s ease-in-out 0.9s infinite alternate; }
${S}[data-state="attention"] .hm-hop { animation: hm-startle 0.45s ease-out 1; }
${S}[data-state="sleep"] .hm-in { animation: none; }
${S}[data-state="sleep"] .hm-sl { transform: scale(1.06, 0.84); }
${S}[data-state="sleep"] .hm-head { transform: rotate(16deg) translateY(1px); }
${S}.act-twitch .hm-head { animation: cr-wig 0.65s ease-in-out; }
@keyframes hm-bob { from { transform: translateY(0); } to { transform: translateY(-0.9px); } }
@keyframes hm-leg { from { transform: rotate(-36deg); } to { transform: rotate(36deg); } }
@keyframes hm-nod { from { transform: rotate(-6deg); } to { transform: rotate(6deg); } }
@keyframes hm-hop { 0% { transform: scale(1, 1); } 18% { transform: translateY(1px) scale(1.08, 0.88); } 48% { transform: translateY(-13px) scale(0.94, 1.1); } 78% { transform: translateY(0) scale(1.06, 0.92); } 100% { transform: scale(1, 1); } }
@keyframes hm-startle { 0% { transform: scale(1, 1); } 30% { transform: translateY(-6px) scale(0.92, 1.12); } 100% { transform: scale(1, 1); } }`,
      init(host, api) { host._h = { a: 0, w: 0, wheel: api.$('.hm-wheel') }; host._en = api.$('.en'); },
      energy(host, e) { enT(host, e == null ? '' : `translateY(${(-e * 3).toFixed(2)}px) scale(${(1 + e * 0.03).toFixed(3)}, ${(1 - e * 0.04).toFixed(3)})`); },
      poke(host) { if (host._h) host._h.w += 320; },
      tick(host, t) {
        const s = host._h; if (!s || still()) return;
        const dt = dtOf(host, t), st = host.dataset.state;
        const T = HM_V[st] || 0;
        s.w += (T - s.w) * (1 - Math.exp(-dt / (st === 'done' ? 0.8 : st === 'sleep' ? 0.5 : 0.35))); // a state change accelerates / coasts instead of jumping
        s.a = (s.a + s.w * dt) % 360;
        s.wheel.setAttribute('transform', `rotate(${s.a.toFixed(1)} 24 22)`);
      },
    });
  }

  // ================= 2. stickman (40 x 48): ONE line weight, limbs are paths that bend =================
  {
    const S = '.pet[data-pet="stickman"]', V = '[data-state=';
    const Sh = [20, 18.5], Hp = [20, 29], Nk = [20, 14.9];
    const rest = { aL: [Sh, [14.5, 23.5], [13.2, 29.5]], aR: [Sh, [25.5, 23.5], [26.8, 29.5]], lL: [Hp, [17.5, 38], [17, 46.5], [14.8, 46.5]], lR: [Hp, [22.5, 38], [23, 46.5], [25.2, 46.5]], t: [Nk, Hp] };
    const upL = [Sh, [13, 13.5], [9.5, 7]], upR = [Sh, [27, 13.5], [30.5, 7]], dnL = [Sh, [14, 24.5], [11.5, 30]], dnR = [Sh, [26, 24.5], [28.5, 30]];
    const liftL = [Hp, [16, 36.5], [19.5, 41.5], [17.3, 43]], liftR = [Hp, [24, 36.5], [20.5, 41.5], [22.7, 43]];
    const stepA = [Hp, [18.7, 38], [15.8, 46.4], [13.6, 46.4]], stepB = [Hp, [21.3, 38], [24.2, 46.4], [26.4, 46.4]];
    const stepA2 = [Hp, [21.3, 38], [24.2, 46.4], [26.4, 46.4]], stepB2 = [Hp, [18.7, 38], [15.8, 46.4], [13.6, 46.4]];
    const sl = { N: [23.5, 31.9], H: [17, 42.5], S: [22, 34.5] };
    const P = (cls, k) => `<path class="sm-l ${cls}" d="${dA(rest[k])}"/>`;
    PP.add({
      id: 'stickman', group: 'creatures',
      name: { ru: 'Человечек', en: 'Stickman' },
      desc: { ru: 'Тонкая фигурка из штрихов: танцует, думает, прыгает и засыпает сидя', en: 'A thin line figure that dances, ponders, jumps and dozes sitting up' },
      w: 40, h: 48, fx: false,
      svg: (api) => `
<g class="en"><g class="sm-fig">
  ${P('sm-lL', 'lL')}${P('sm-lR', 'lR')}${P('sm-tor', 't')}
  <g class="sm-ag sm-agL">${P('sm-aL', 'aL')}</g><g class="sm-ag sm-agR">${P('sm-aR', 'aR')}</g>
  ${paper('sm-paper', 'translate(31.4 19.6) rotate(10)')}
  <g class="sm-head"><circle class="sm-hd" cx="20" cy="9.6" r="5.2"/>${api.eyes([[17.9, 9.9], [22.3, 9.9]], 'dot', '#f3f6ff', 1.45, '#f3f6ff')}</g>
</g></g>
${fx(34.5, 8)}${dots(33.5, 6.5)}`,
      css: SHARED_KF + common('stickman') + paperCss(S) + `
${S} .sm-l { fill: none; stroke: var(--ac); stroke-width: 2.8; stroke-linecap: round; stroke-linejoin: round; transition: d 0.35s ease; }
${S} .sm-hd { fill: color-mix(in srgb, var(--ac) 14%, #060608); stroke: var(--ac); stroke-width: 2.6; }
${S} .sm-aL { d: ${dp(rest.aL)}; } ${S} .sm-aR { d: ${dp(rest.aR)}; } ${S} .sm-lL { d: ${dp(rest.lL)}; } ${S} .sm-lR { d: ${dp(rest.lR)}; } ${S} .sm-tor { d: ${dp(rest.t)}; }
${S} .sm-fig, ${S} .sm-ag, ${S} .sm-head { transform-box: view-box; }
${S} .sm-fig { transform-origin: 20px 47px; animation: sm-sway 3.6s ease-in-out infinite alternate; }
${S} .sm-ag { transform-origin: 20px 18.5px; animation: sm-arm 2.4s ease-in-out infinite alternate; } ${S} .sm-agR { animation-direction: alternate-reverse; }
${S} .sm-head { transform-origin: 20px 14.9px; transition: transform 0.35s ease; }
${S} .sm-paper { display: none; }
/* music: dances; arms and legs swap on the beat (0.55 s), the body leans, the head nods */
${S}${V}"music"] .sm-fig { animation: sm-dance 1.1s ease-in-out infinite; }
${S}${V}"music"] .sm-ag { animation: none; }
${S}${V}"music"] .sm-aL { animation: sm-dAL 0.55s ease-in-out infinite alternate; } ${S}${V}"music"] .sm-aR { animation: sm-dAR 0.55s ease-in-out infinite alternate; }
${S}${V}"music"] .sm-lL { animation: sm-dLL 0.55s ease-in-out infinite alternate; } ${S}${V}"music"] .sm-lR { animation: sm-dLR 0.55s ease-in-out infinite alternate; }
${S}${V}"music"] .sm-head { animation: sm-nod 0.55s ease-in-out infinite alternate; }
/* thinking: right hand on the chin, left arm props the elbow, head tilts, a foot taps */
${S}${V}"thinking"] .sm-aR { d: ${dp([Sh, [27, 22.5], [22.4, 15.8]])}; } ${S}${V}"thinking"] .sm-aL { d: ${dp([Sh, [14.5, 24.5], [26.2, 24.8]])}; }
${S}${V}"thinking"] .sm-ag { animation: none; } ${S}${V}"thinking"] .sm-fig { animation-duration: 3s; }
${S}${V}"thinking"] .sm-head { transform: rotate(-8deg); }
${S}${V}"thinking"] .eyes { transform: translate(1.5px, -1.3px); }
${S}${V}"thinking"] .sm-lR { animation: sm-tap 0.5s ease-in-out infinite alternate; }
/* done: a jump, then both arms stay up and wave */
${S}${V}"done"] .sm-aL { d: ${dp(upL)}; } ${S}${V}"done"] .sm-aR { d: ${dp(upR)}; }
${S}${V}"done"] .sm-fig { animation: sm-jump 0.8s cubic-bezier(0.3, 0.7, 0.4, 1) 1, sm-cheer 0.6s ease-in-out 0.8s infinite alternate; }
${S}${V}"done"] .sm-ag { animation: sm-wave 0.4s ease-in-out infinite alternate; }
/* attention: arms fly up, hops on the spot, "!" */
${S}${V}"attention"] .sm-aL { d: ${dp([Sh, [13, 20], [9, 13.5]])}; } ${S}${V}"attention"] .sm-aR { d: ${dp([Sh, [27, 20], [30.5, 14]])}; }
${S}${V}"attention"] .sm-fig { animation: sm-hop 0.5s ease-out infinite; } ${S}${V}"attention"] .sm-ag { animation: none; }
/* file: carries a paper in the right hand and walks in place */
${S}${V}"file"] .sm-aR { d: ${dp([Sh, [26, 22], [30.6, 23.6]])}; } ${S}${V}"file"] .sm-aL { d: ${dp([Sh, [16, 23.5], [14.6, 29.5]])}; }
${S}${V}"file"] .sm-ag { animation: none; } ${S}${V}"file"] .sm-paper { display: inline; }
${S}${V}"file"] .sm-paper-i { animation: cr-bob 0.6s ease-in-out infinite alternate; }
${S}${V}"file"] .sm-fig { animation: sm-step 0.6s ease-in-out infinite alternate; }
${S}${V}"file"] .sm-lL { animation: sm-wLL 0.6s ease-in-out infinite alternate; } ${S}${V}"file"] .sm-lR { animation: sm-wLR 0.6s ease-in-out infinite alternate; }
/* sleep: sits hunched with the head down on the knees, slow breathing */
${S}${V}"sleep"] .sm-tor { d: ${dp([sl.N, sl.H])}; }
${S}${V}"sleep"] .sm-aL { d: ${dp([sl.S, [27, 37.5], [26.5, 40.8]])}; } ${S}${V}"sleep"] .sm-aR { d: ${dp([sl.S, [29.2, 35.6], [27.6, 40.2]])}; }
${S}${V}"sleep"] .sm-lL { d: ${dp([sl.H, [26, 38.5], [27.4, 46.5], [30.4, 46.5]])}; } ${S}${V}"sleep"] .sm-lR { d: ${dp([sl.H, [24, 40.6], [25.4, 46.5], [28.4, 46.5]])}; }
${S}${V}"sleep"] .sm-head { transform: translate(3.5px, 17px) rotate(26deg); }
${S}${V}"sleep"] .sm-ag { animation: none; } ${S}${V}"sleep"] .sm-fig { animation: sm-breath 4.5s ease-in-out infinite alternate; }
${S}.act-twitch .sm-ag { animation: sm-wave 0.2s ease-in-out 3 alternate; }
@keyframes sm-sway { from { transform: rotate(-1.8deg); } to { transform: rotate(1.8deg); } }
@keyframes sm-arm { from { transform: rotate(-4deg); } to { transform: rotate(4deg); } }
@keyframes sm-dance { 0%, 100% { transform: rotate(-6deg); } 25%, 75% { transform: rotate(0) translateY(-2px); } 50% { transform: rotate(6deg); } }
@keyframes sm-nod { from { transform: rotate(-8deg); } to { transform: rotate(8deg); } }
@keyframes sm-jump { 0% { transform: scale(1, 1); } 18% { transform: translateY(1px) scale(1.06, 0.92); } 48% { transform: translateY(-9px) scale(0.96, 1.05); } 78% { transform: translateY(0) scale(1.04, 0.96); } 100% { transform: scale(1, 1); } }
@keyframes sm-cheer { from { transform: rotate(-2.5deg); } to { transform: rotate(2.5deg); } }
@keyframes sm-wave { from { transform: rotate(-7deg); } to { transform: rotate(7deg); } }
@keyframes sm-hop { 0%, 100% { transform: translateY(0); } 40% { transform: translateY(-7px); } }
@keyframes sm-step { from { transform: translateY(0); } to { transform: translateY(-0.9px); } }
@keyframes sm-breath { from { transform: scale(1, 1); } to { transform: scale(1.025, 1.03); } }
${kf('sm-dAL', upL, dnL)}${kf('sm-dAR', dnR, upR)}${kf('sm-dLL', rest.lL, liftL)}${kf('sm-dLR', liftR, rest.lR)}
${kf('sm-tap', rest.lR, [Hp, [22.5, 37.2], [23, 44.3], [25.2, 44.3]])}${kf('sm-wLL', stepA, stepB)}${kf('sm-wLR', stepA2, stepB2)}`,
      init(host, api) { host._en = api.$('.en'); },
      energy(host, e) { enT(host, e == null ? '' : `translateY(${(-e * 3).toFixed(2)}px) scale(${(1 - e * 0.02).toFixed(3)}, ${(1 + e * 0.05).toFixed(3)})`); },
    });
  }

  // ================= 3. koi (96 x 48): swims end to end, turns around, tail flaps =================
  {
    const S = '.pet[data-pet="koi"]', D = 43, OX = 4.5, CX = 22, CY = 29;
    const bub = [[46.8, 27, 1.7, 0], [49, 23.5, 1.2, 1], [47.5, 20.5, 2.1, 2]].map(([x, y, r, i]) => `<circle class="k-b k-b${i}" cx="${x}" cy="${y}" r="${r}" fill="none" stroke="#cfe3ff" stroke-width="1"/>`).join('');
    PP.add({
      id: 'koi', group: 'creatures',
      name: { ru: 'Рыбка-кои', en: 'Koi' },
      desc: { ru: 'Кружит от края до края капсулы, под музыку изгибается', en: 'Cruises from end to end of the capsule and sways to the music' },
      w: 96, h: 48, fx: false,
      svg: (api) => `
<g class="k-flip" transform="translate(${OX + 10 + CX} ${CY}) translate(${-CX} ${-CY})">
  <g class="k-bubs">${bub}</g>
  <g class="en"><g class="k-loop"><g class="k-sw">
    <g class="k-tail"><path d="M9.6 29L-0.5 19.5Q3.8 29 -0.5 38.5Z"/></g>
    <path class="k-fin" d="M17 23.4Q24 13.5 31.5 23.4Z"/>
    <path d="M8.5 29C12 22.6 22 21.2 31 22.4C38 23.6 43.5 26.4 44.5 29C43.5 31.6 38 34.4 31 35.6C22 36.8 12 35.4 8.5 29Z" fill="#f4f1ea"/>
    <g class="k-spots"><ellipse cx="22" cy="25.6" rx="5.3" ry="2.7" transform="rotate(-8 22 25.6)"/><ellipse cx="32.6" cy="29.6" rx="3.4" ry="2.3"/><ellipse cx="15" cy="30.4" rx="3.1" ry="2"/></g>
    <g class="k-pec"><path class="k-pf" d="M27.5 31.6Q22.5 37.8 17.5 36.4Q21.5 33.6 24.5 31Z"/></g>
    ${api.eyes([[38.5, 27]], 'dot', OL, 1.55)}
  </g></g></g>
</g>
<g class="k-props" transform="translate(${OX + 10} 0)">
  <g class="k-fb"><g class="k-fbi" transform="translate(30 11)"><circle r="8" fill="rgba(190,215,255,.14)" stroke="#bcd3f0" stroke-width="1.3"/><path d="M-5 -3.2A6 6 0 0 1 -1.6 -6.2" fill="none" stroke="#fff" stroke-width="1.3" stroke-linecap="round" opacity=".8"/>${paper('k-paper', 'translate(0.4 0.4)')}</g></g>
  ${fx(40, 12)}${dots(34, 11, 5.5)}
</g>`,
      css: SHARED_KF + common('koi') + paperCss(S) + `
${S} { --kt: 0.45s; }
${S}[data-state="music"] { --kt: 0.25s; } ${S}[data-state="thinking"] { --kt: 0.6s; } ${S}[data-state="done"] { --kt: 0.2s; } ${S}[data-state="attention"] { --kt: 0.09s; } ${S}[data-state="file"] { --kt: 0.4s; } ${S}[data-state="sleep"] { --kt: 1.4s; }
${S} .k-tail path, ${S} .k-spots ellipse { fill: var(--ac); }
${S} .k-fin { fill: var(--ac); opacity: 0.85; }
${S} .k-pf { fill: color-mix(in srgb, var(--ac) 35%, #fff); opacity: 0.9; }
${S} .k-tail, ${S} .k-pec, ${S} .k-sw, ${S} .k-loop { transform-box: fill-box; }
${S} .k-tail { transform-origin: 100% 50%; animation: k-tail var(--kt) ease-in-out infinite alternate; }
${S} .k-pec { transform-origin: 85% 0; animation: k-pec 0.7s ease-in-out infinite alternate; }
${S} .k-sw, ${S} .k-loop, ${S} .en { transform-origin: 50% 50%; }
${S} .k-sw { animation: k-wob 1.8s ease-in-out infinite alternate; }
${S}[data-state="music"] .k-sw { animation: k-wob2 0.55s ease-in-out infinite alternate; }
${S}[data-state="attention"] .k-sw { animation: k-shiver 0.1s linear infinite alternate; }
${S}[data-state="done"] .k-loop { animation: k-loop 1.1s ease-in-out 1; }
${S}[data-state="sleep"] .k-sw { animation: none; }
${S}.act-twitch .k-tail { animation-duration: 0.1s; }
${S} .k-bubs, ${S} .k-fb { display: none; }
${S} .k-b { transform-box: fill-box; transform-origin: 50% 50%; }
${S}[data-state="thinking"] .k-bubs, ${S}[data-state="done"] .k-bubs, ${S}[data-state="file"] .k-fb { display: inline; }
${S}[data-state="thinking"] .k-b { animation: k-rise 2.8s ease-in infinite; }
${S}[data-state="done"] .k-b { animation: k-rise 0.9s ease-out both; }
${S} .k-b1 { animation-delay: 0.4s !important; } ${S} .k-b2 { animation-delay: 0.9s !important; }
${S}[data-state="done"] .k-b1 { animation-delay: 0.12s !important; } ${S}[data-state="done"] .k-b2 { animation-delay: 0.24s !important; }
${S} .k-fbi { transform-box: fill-box; transform-origin: 50% 50%; animation: k-fbob 1.4s ease-in-out infinite alternate; }
@keyframes k-tail { from { transform: rotate(-17deg) scaleY(0.92); } to { transform: rotate(17deg) scaleY(1.04); } }
@keyframes k-pec { from { transform: rotate(-12deg); } to { transform: rotate(14deg); } }
@keyframes k-wob { from { transform: rotate(-2deg); } to { transform: rotate(2deg); } }
@keyframes k-wob2 { from { transform: rotate(-7deg) translateY(1px); } to { transform: rotate(7deg) translateY(-1px); } }
@keyframes k-shiver { from { transform: translateX(-0.7px); } to { transform: translateX(0.7px); } }
@keyframes k-loop { 0% { transform: translateY(0) rotate(0); } 50% { transform: translateY(-8px) rotate(-180deg); } 100% { transform: translateY(0) rotate(-360deg); } }
@keyframes k-rise { 0% { transform: translateY(2px) scale(0.6); opacity: 0; } 25% { opacity: 1; } 100% { transform: translateY(-13px) scale(1.15); opacity: 0; } }
@keyframes k-fbob { from { transform: translateY(0); } to { transform: translateY(-1.5px); } }`,
      init(host, api) { host._k = { x: 10, dir: 1, sx: 1, sy: 1, kick: 0, flip: api.$('.k-flip'), props: api.$('.k-props') }; host._en = api.$('.en'); },
      energy(host, e) { enT(host, e == null ? '' : `scale(${(1 + e * 0.1).toFixed(3)}, ${(1 - e * 0.08).toFixed(3)})`); },
      poke(host) { if (host._k) host._k.kick = 0.8; },
      tick(host, t) {
        const s = host._k; if (!s || still()) return;
        const dt = dtOf(host, t), st = host.dataset.state;
        let v = KO_V[st] || 0;
        if (st === 'attention') v = ((t / 1000) % 1.3) < 0.3 ? 72 : 3; // quick darts with short pauses
        if (s.kick > 0) { v = Math.max(v, 5) * 3; s.kick -= dt; }
        if (st === 'sleep') { const d = D / 2 - s.x; s.x += Math.sign(d) * Math.min(Math.abs(d), 8 * dt); } // drifts to the middle, then floats
        else { s.x += s.dir * v * dt; if (s.x >= D) { s.x = D; s.dir = -1; } else if (s.x <= 0) { s.x = 0; s.dir = 1; } }
        s.sx += clamp(s.dir - s.sx, -6 * dt, 6 * dt); // the turn: the body narrows through zero and widens the other way
        s.sy += clamp((st === 'sleep' ? -1 : 1) - s.sy, -4 * dt, 4 * dt); // asleep: belly up
        const sx = Math.abs(s.sx) < 0.06 ? 0.06 * (s.dir || 1) : s.sx, sy = Math.abs(s.sy) < 0.06 ? 0.06 : s.sy;
        const bob = Math.sin(t / (st === 'music' ? 220 : 800)) * (st === 'music' ? 1.7 : st === 'sleep' ? 0.9 : 1.1);
        s.flip.setAttribute('transform', `translate(${(s.x + OX + CX).toFixed(2)} ${(CY + bob).toFixed(2)}) scale(${sx.toFixed(3)} ${sy.toFixed(3)}) translate(${-CX} ${-CY})`);
        s.props.setAttribute('transform', `translate(${(s.x + OX).toFixed(2)} ${bob.toFixed(2)})`);
      },
    });
  }

  // ================= 4. snail (96 x 48): crawls end to end, turns around, hides in its shell to sleep =================
  {
    const S = '.pet[data-pet="snail"]', D = 40, OX = 4, CX = 22;
    let spiral = ''; for (let a = 0.4; a < 12.2; a += 0.3) { const r = a * 0.78; spiral += (spiral ? 'L' : 'M') + (17 + r * Math.cos(a)).toFixed(1) + ' ' + (32 + r * Math.sin(a)).toFixed(1); }
    PP.add({
      id: 'snail', group: 'creatures',
      name: { ru: 'Улитка', en: 'Snail' },
      desc: { ru: 'Неторопливо ползёт по капсуле, а во сне прячется в ракушку', en: 'Crawls unhurried along the capsule and hides in its shell to sleep' },
      w: 96, h: 48, fx: false,
      svg: () => `
<g class="sn-flip" transform="translate(${OX + 6 + CX} 0) translate(${-CX} 0)">
 <g class="en"><g class="sn-fig">
  <g class="sn-body">
   <path class="sn-bd" d="M0.5 46.8C1 44.3 5 43 10 42.6L31 42.4C34.5 42.2 35.6 37 36.3 33.5C36.9 30.6 41.5 30.3 42.4 34.2C43.2 38 43.6 43 42.5 46.8Z"/>
   <path d="M38.2 37.2q1.8 1.8 3.6 0" fill="none" stroke="#8b7355" stroke-width="1.2" stroke-linecap="round"/>
   <g class="sn-lk"><g class="sn-st sn-s1"><path d="M38 33.5L35.4 23.6" fill="none" stroke="#dfcdb3" stroke-width="2.4" stroke-linecap="round"/><circle class="sn-tip" cx="35.2" cy="23" r="2.3"/></g>
   <g class="sn-st sn-s2"><path d="M41 33.5L43.6 24.2" fill="none" stroke="#dfcdb3" stroke-width="2.4" stroke-linecap="round"/><circle class="sn-tip" cx="43.8" cy="23.6" r="2.3"/></g></g>
  </g>
  <g class="sn-shell"><circle class="sn-sc" cx="17" cy="32" r="11.2"/><g class="sn-sp"><path d="${spiral}" fill="none" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round"/></g>
   <path d="M8.8 29A9 9 0 0 1 14.6 22.4" fill="none" stroke="#fff" stroke-width="1.5" stroke-linecap="round" opacity=".35"/></g>
  <g class="sn-leaf" transform="translate(17 19) rotate(-12)"><g class="sn-leafi"><path class="sn-lf" d="M-9.5 0Q-3 -7.5 9.5 -1Q4 6.5 -9.5 0Z"/><path d="M-8 0.2Q0 -1.2 7.5 -1.2" fill="none" stroke="#17141f" stroke-width=".9" stroke-linecap="round" opacity=".55"/></g></g>
 </g></g>
</g>
<g class="sn-props" transform="translate(${OX + 6} 0)">${fx(41, 11)}${dots(40, 11.5)}</g>`,
      css: SHARED_KF + common('snail') + `
${S} .sn-bd { fill: #dfcdb3; }
${S} .sn-tip { fill: #2a2118; }
${S} .sn-sc { fill: var(--ac); }
${S} .sn-sp path { stroke: color-mix(in srgb, var(--ac) 50%, #000); }
${S} .sn-lf { fill: color-mix(in srgb, var(--pet-accent) var(--t-mid), #8fbf4a); }
${S} .sn-fig, ${S} .sn-body, ${S} .sn-shell, ${S} .sn-leafi, ${S} .sn-st, ${S} .sn-lk { transform-box: fill-box; }
${S} .sn-fig { transform-origin: 50% 100%; } ${S} .sn-body { transform-origin: 85% 100%; transition: transform 0.45s ease, opacity 0.4s ease; animation: sn-rip 1.6s ease-in-out infinite alternate; }
${S} .sn-shell { transform-origin: 50% 100%; transition: transform 0.45s ease; }
${S} .sn-sp { transform-box: view-box; transform-origin: 17px 32px; }
${S} .sn-lk { transform-origin: 50% 100%; transform: rotate(calc(var(--lx, 0) * 10deg)); transition: transform 0.2s ease-out; }
${S} .sn-st { transform-origin: 50% 100%; transition: transform 0.4s ease; animation: sn-sw 3s ease-in-out infinite alternate; } ${S} .sn-s2 { animation-duration: 3.5s; animation-direction: alternate-reverse; }
${S} .sn-leaf { display: none; }
${S}[data-state="music"] .sn-st { animation: sn-wig 0.55s ease-in-out infinite alternate; } ${S}[data-state="music"] .sn-s2 { animation-duration: 0.55s; animation-direction: alternate-reverse; }
${S}[data-state="music"] .sn-shell { animation: sn-bob 0.55s ease-in-out infinite alternate; }
${S}[data-state="thinking"] .sn-st { animation: none; } ${S}[data-state="thinking"] .sn-s1 { transform: rotate(28deg); } ${S}[data-state="thinking"] .sn-s2 { transform: rotate(-28deg); }
${S}[data-state="thinking"] .sn-sp { animation: sn-spin 6s linear infinite; }
${S}[data-state="done"] .sn-fig { animation: sn-hop 0.9s cubic-bezier(0.3, 0.7, 0.4, 1) 1; }
${S}[data-state="done"] .sn-st { animation: sn-wig 0.3s ease-in-out infinite alternate; transform-origin: 50% 100%; } ${S}[data-state="done"] .sn-s2 { animation-direction: alternate-reverse; }
${S}[data-state="done"] .sn-shell { animation: sn-bob 0.45s ease-in-out 0.9s infinite alternate; }
${S}[data-state="attention"] .sn-st { animation: sn-shiv 0.12s linear infinite alternate; } ${S}[data-state="attention"] .sn-s1 { transform: rotate(11deg) scaleY(1.3); } ${S}[data-state="attention"] .sn-s2 { transform: rotate(-14deg) scaleY(1.3); }
${S}[data-state="file"] .sn-leaf { display: inline; } ${S}[data-state="file"] .sn-leafi { transform-origin: 50% 100%; animation: cr-bob 0.9s ease-in-out infinite alternate; }
${S}[data-state="sleep"] .sn-body { opacity: 0; transform: scaleX(0.3); animation: none; }
${S}[data-state="sleep"] .sn-shell { transform: translateY(3.8px); }
${S}.poked .sn-st, ${S}.act-twitch .sn-st { animation: sn-wig 0.2s ease-in-out 4 alternate; }
@keyframes sn-rip { from { transform: scaleX(1); } to { transform: scaleX(1.03); } }
@keyframes sn-sw { from { transform: rotate(-4deg); } to { transform: rotate(4deg); } }
@keyframes sn-wig { from { transform: rotate(-14deg); } to { transform: rotate(14deg); } }
@keyframes sn-shiv { from { transform: translateX(-0.5px) rotate(-2deg) scaleY(1.3); } to { transform: translateX(0.5px) rotate(2deg) scaleY(1.3); } }
@keyframes sn-bob { from { transform: translateY(0) rotate(-4deg); } to { transform: translateY(-1.8px) rotate(4deg); } }
@keyframes sn-spin { to { transform: rotate(360deg); } }
@keyframes sn-hop { 0% { transform: scale(1, 1); } 18% { transform: translateY(1px) scale(1.07, 0.9); } 48% { transform: translateY(-8px) scale(0.95, 1.1); } 78% { transform: translateY(0) scale(1.05, 0.94); } 100% { transform: scale(1, 1); } }`,
      init(host, api) { host._s = { x: 6, dir: 1, sx: 1, flip: api.$('.sn-flip'), props: api.$('.sn-props') }; host._en = api.$('.en'); },
      energy(host, e) { enT(host, e == null ? '' : `translateY(${(-e * 2).toFixed(2)}px) scale(${(1 + e * 0.03).toFixed(3)}, ${(1 - e * 0.03).toFixed(3)})`); },
      tick(host, t) {
        const s = host._s; if (!s || still()) return;
        const dt = dtOf(host, t), st = host.dataset.state;
        const v = SN_V[st] || 0;
        s.x += s.dir * v * dt;
        if (s.x >= D) { s.x = D; s.dir = -1; } else if (s.x <= 0) { s.x = 0; s.dir = 1; }
        s.sx += clamp(s.dir - s.sx, -4 * dt, 4 * dt);
        const sx = Math.abs(s.sx) < 0.06 ? 0.06 * (s.dir || 1) : s.sx;
        s.flip.setAttribute('transform', `translate(${(s.x + OX + CX).toFixed(2)} 0) scale(${sx.toFixed(3)} 1) translate(${-CX} 0)`);
        s.props.setAttribute('transform', `translate(${(s.x + OX).toFixed(2)} 0)`);
      },
    });
  }
})();
