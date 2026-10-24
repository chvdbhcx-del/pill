// Characters (group 'characters'): our own takes on the "character gallery" idea of Island-style apps, drawn from scratch.
// pixel (8-bit shuffler) | spark (ember face) | eyes (only the eyes) | bot (monitor face) | kaomoji (text face from paths) | swarm (16 dots forming shapes)
// Loaded by <script> AFTER pets.js; uses only PillPets.add (see EXTENSION API in pets.js). Motion = CSS transforms / opacity only; the host pauses and reduces it.
(() => {
  'use strict';
  if (!window.PillPets || !window.PillPets.add) return;
  const OL = '#17141f';
  // body / part colour: the reaction colour (host sets --pet-accent, and --t-* = how much of it a part takes; 0 when calm) mixed into a base colour
  const mix = (base, k = 'var(--t-hi)') => `color-mix(in srgb, var(--pet-accent) ${k}, ${base})`;
  const scope = (id) => `.pet[data-pet="${id}"]`;

  // ---- shared smooth props (z, "!" badge, thinking dots, paper, hearts when petted, done sparkle): anchor = the [x, y] of each group in the pet's viewBox ----
  const ZP = 'M-2.4 -2.9h4.8l-4.8 5.8h4.8';
  const HEART = 'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z';
  const STAR = 'M0-4.6L1.1-1.1L4.6 0L1.1 1.1L0 4.6L-1.1 1.1L-4.6 0L-1.1-1.1Z';
  const props = (o) => {
    const at = (k, inner) => (o[k] ? `<g class="c-${k}" transform="translate(${o[k][0]} ${o[k][1]})">${inner}</g>` : '');
    const z = (tr, c) => `<g class="z ${c}"><g transform="${tr}"><path class="zp" d="${ZP}"/></g></g>`;
    const h = (tr, c) => `<g class="h ${c}"><path d="${HEART}" fill="#ff7aa2" transform="${tr}"/></g>`;
    return at('z', z('scale(.8)', 'z1') + z('translate(3 -5)', 'z2') + z('translate(-1 -11) scale(1.25)', 'z3'))
      + at('ex', '<g class="exb"><circle class="exc" r="5.6"/><rect x="-.9" y="-3.3" width="1.8" height="4.5" rx=".9" fill="#2a1d08"/><circle cy="2.7" r="1.05" fill="#2a1d08"/></g>')
      + at('dots', '<circle class="d1" cx="-5.5" r="1.7"/><circle class="d2" r="1.7"/><circle class="d3" cx="5.5" r="1.7"/>')
      + at('paper', '<g class="pap"><path class="pp1" d="M-4.5 -6h6l3.5 3.5v8.5h-9.5z"/><path class="pp2" d="M1.5 -6v3.5h3.5"/><path class="pp3" d="M-2.5 0h6M-2.5 2.5h6"/></g>')
      + at('hearts', h('translate(0 0) scale(1.1)', 'h1') + h('translate(5 -3)', 'h2') + h('translate(-5 -5) scale(.8)', 'h3'))
      + at('spark', `<g class="s s1"><path d="${STAR}" fill="#ffe27a"/></g><g class="s s2"><path d="${STAR}" fill="#fff3b0" transform="translate(-6 7) scale(.65)"/></g>`);
  };
  const propsCss = (P) => `
${P} .c-z, ${P} .c-ex, ${P} .c-dots, ${P} .c-paper, ${P} .c-hearts, ${P} .c-spark { display: none; }
${P}[data-state="sleep"] .c-z, ${P}[data-state="attention"] .c-ex, ${P}[data-state="thinking"] .c-dots, ${P}[data-state="file"] .c-paper, ${P}[data-state="done"] .c-spark, ${P}.poked .c-hearts { display: inline; }
${P} :is(.z, .exb, .pap, .h, .s, .c-dots circle) { transform-box: fill-box; transform-origin: 50% 50%; }
${P} .zp { fill: none; stroke: ${mix('#e8ebff')}; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
${P} .exc { fill: ${mix('#ffb340')}; }
${P} .c-dots circle { fill: ${mix('#e8ebff')}; }
${P} .pp1 { fill: #f4f6fb; stroke: ${mix('#17141f')}; stroke-width: 1; stroke-linejoin: round; } ${P} .pp2 { fill: ${mix('#cfd6e6', 'var(--t-mid)')}; } ${P} .pp3 { fill: none; stroke: ${mix('#7d8fb8')}; stroke-width: 1; stroke-linecap: round; }
${P}[data-state="sleep"] .c-z .z { animation: ch-z 3s ease-in-out infinite; } ${P}[data-state="sleep"] .c-z .z2 { animation-delay: 1s; } ${P}[data-state="sleep"] .c-z .z3 { animation-delay: 2s; }
${P}[data-state="attention"] .exb { animation: ch-pop .35s cubic-bezier(.34,1.56,.64,1) both, ch-bob .7s ease-in-out .35s infinite alternate; }
${P}[data-state="thinking"] .c-dots circle { animation: ch-dot 1.2s ease-in-out infinite; } ${P}[data-state="thinking"] .c-dots .d2 { animation-delay: .2s; } ${P}[data-state="thinking"] .c-dots .d3 { animation-delay: .4s; }
${P}[data-state="file"] .pap { animation: ch-pop .35s cubic-bezier(.34,1.56,.64,1) both, ch-pap 1.8s ease-in-out .35s infinite alternate; }
${P}[data-state="done"] .c-spark .s { animation: ch-spark 1.1s ease-out both; } ${P}[data-state="done"] .c-spark .s2 { animation-delay: .15s; }
${P}.poked .c-hearts .h { animation: ch-heart 1.1s ease-out both; } ${P}.poked .c-hearts .h1 { --hx: 0px; } ${P}.poked .c-hearts .h2 { --hx: 5px; animation-delay: .15s; } ${P}.poked .c-hearts .h3 { --hx: -5px; animation-delay: .3s; }
@keyframes ch-z { 0% { transform: translate(0, 4px) scale(.5); opacity: 0; } 20% { opacity: 1; } 75% { opacity: 1; } 100% { transform: translate(6px, -11px) scale(1.1); opacity: 0; } }
@keyframes ch-pop { from { transform: scale(0); } to { transform: scale(1); } }
@keyframes ch-bob { to { transform: translateY(-1.6px); } }
@keyframes ch-dot { 0%, 100% { transform: scale(.6); opacity: .45; } 40% { transform: scale(1.15); opacity: 1; } }
@keyframes ch-pap { from { transform: translateY(0) rotate(-3deg); } to { transform: translateY(-1.4px) rotate(3deg); } }
@keyframes ch-spark { 0% { transform: scale(0) rotate(0); opacity: 0; } 30% { transform: scale(1.2) rotate(40deg); opacity: 1; } 100% { transform: scale(.4) rotate(120deg); opacity: 0; } }
@keyframes ch-heart { 0% { transform: translate(0, 4px) scale(.4); opacity: 0; } 25% { opacity: 1; } 100% { transform: translate(var(--hx, 0px), -14px) scale(1); opacity: 0; } }`;
  const glanceKf = (P, sel, amt = 4) => { const n = 'ch-gl-' + P.replace(/\W/g, ''); return `${P}.act-twitch ${sel} { animation: ${n} .65s ease-in-out; } @keyframes ${n} { 0%, 100% { transform: translateX(0); } 30% { transform: translateX(-${amt}px); } 65% { transform: translateX(${amt}px); } }`; };
  const blinkKf = '@keyframes ch-blink { 45% { transform: scaleY(.08); } }';

  // =====================================================================================================================================
  // 1. pixel: a little 8-bit antenna blob with square eyes and feet that shuffle. 12 x 9 cells of 3.5 units, every motion is steps().
  // =====================================================================================================================================
  {
    const P = scope('pixel'), U = 3.5, EY = '#150f14';
    window.PillPets.add({
      id: 'pixel', group: 'characters', fx: false, w: 48, h: 48,
      name: { ru: 'Пиксель', en: 'Pixel' },
      desc: { ru: '8-битный герой: квадратные глазки, шаркающие ножки, цвет по настроению', en: '8-bit hero: square eyes, shuffling feet, colour follows the mood' },
      svg: (api) => {
        const R = api.rc, cl = (rows, f, x = 0, y = 0) => api.cells(rows, { x: f }, x, y);
        const body = cl(['..x......x..', '...x....x...', '..xxxxxxxx..', '.xxxxxxxxxx.', 'xxxxxxxxxxxx', 'xxxxxxxxxxxx', 'xxxxxxxxxxxx', 'xxxxxxxxxxxx'], '.px-b')
          + R(0, 6, 12, 2, '#000', 'opacity:.16') + R(3, 2, 3, 1, '#fff', 'opacity:.3') + R(11, 3, 1, 3, '#000', 'opacity:.12');
        const eyes = api.g8('px-eyes',
          api.g8('px-e px-e-open', R(2, 3, 2, 2, EY) + R(8, 3, 2, 2, EY))
          + api.g8('px-e px-e-wide', R(2, 3, 2, 3, EY) + R(8, 3, 2, 3, EY) + R(2, 3, 1, 1, '#fff', 'opacity:.8') + R(8, 3, 1, 1, '#fff', 'opacity:.8'))
          + api.g8('px-e px-e-happy', R(1, 4, 1, 1, EY) + R(2, 3, 1, 1, EY) + R(3, 4, 1, 1, EY) + R(8, 4, 1, 1, EY) + R(9, 3, 1, 1, EY) + R(10, 4, 1, 1, EY))
          + api.g8('px-e px-e-shut', R(2, 4, 2, 1, EY) + R(8, 4, 2, 1, EY)));
        const mouth = api.g8('px-m px-m-flat', R(5, 6, 2, 1, EY)) + api.g8('px-m px-m-smile', R(4, 5, 1, 1, EY) + R(7, 5, 1, 1, EY) + R(5, 6, 2, 1, EY))
          + api.g8('px-m px-m-grin', R(4, 5, 4, 1, EY) + R(5, 6, 2, 1, EY)) + api.g8('px-m px-m-o', R(5, 5, 2, 2, EY));
        const feet = api.g8('px-fA', cl(['xx.xx..xx.xx'], '.px-b', 0, 8)) + api.g8('px-fB', cl(['x.xx.xx.xx.x'], '.px-b', 0, 8));
        const plus = cl(['.x.', 'xxx', '.x.'], '.px-sp');
        const heart = ['.x.x.', 'xxxxx', '.xxx.', '..x..'];
        return `<g class="px-fig"><g transform="translate(3 15.5) scale(${U})">${feet}${body}${eyes}${mouth}</g>
<g class="px-p px-dots">${[14.5, 21.5, 28.5].map((x, i) => `<g class="px-d px-d${i + 1}"><rect x="${x}" y="4" width="3.5" height="3.5"/></g>`).join('')}</g>
<g class="px-p px-excl"><rect class="px-ex" x="40" y="1" width="3.5" height="7"/><rect class="px-ex" x="40" y="9.5" width="3.5" height="3.5"/></g>
<g class="px-p px-zs">${[0, 1, 2].map((i) => `<g class="px-z px-z${i + 1}"><g transform="translate(${[31, 35.5, 40][i]} ${[10, 5.5, 1][i]}) scale(${1.2 + i * 0.4})">${cl(['xxx', '.x.', 'xxx'], '.px-zc')}</g></g>`).join('')}</g>
<g class="px-p px-doc"><g transform="translate(35 1)"><rect width="10" height="12" fill="#f4f1ea"/><rect x="7" width="3" height="3" fill="#b9c3d8"/><rect x="2" y="5" width="6" height="1" fill="#6f84b0"/><rect x="2" y="7.5" width="6" height="1" fill="#6f84b0"/><rect x="2" y="10" width="4" height="1" fill="#6f84b0"/></g></g>
<g class="px-p px-spk"><g class="px-sk1"><g transform="translate(3 5) scale(1.6)">${plus}</g></g><g class="px-sk2"><g transform="translate(38 3) scale(1.6)">${plus}</g></g></g>
<g class="px-p px-hearts"><g class="px-h px-h1"><g transform="translate(5 6) scale(1.5)">${cl(heart, '.px-hc')}</g></g><g class="px-h px-h2"><g transform="translate(36 3) scale(1.5)">${cl(heart, '.px-hc')}</g></g></g></g>`;
      },
      css: `
${P} .pet-rig { animation: none !important; }
${P} { --pxb: ${mix('#e2785a')}; }
${P}[data-state="sleep"] { --pxb: #6f6a78; }
${P} .px-b { fill: var(--pxb); } ${P} .px-sp { fill: ${mix('#ffe27a', 'var(--t-mid)')}; } ${P} .px-hc { fill: #ff7aa2; } ${P} .px-zc { fill: ${mix('#e8ebff')}; }
${P} .px-d, ${P} .px-ex { fill: ${mix('#e8ebff')}; } ${P} .px-ex { fill: ${mix('#ffb340')}; }
${P} .px-e, ${P} .px-m, ${P} .px-p, ${P} .px-fB { display: none; }
${P} .px-e-open, ${P} .px-m-flat { display: inline; }
${P} .px-eyes { translate: calc(var(--pxlk, 0) * 1px) calc(var(--pxlky, 0) * 1px); }
${P} .px-fA { animation: px-fa .9s steps(1) infinite; } ${P} .px-fB { display: inline; animation: px-fb .9s steps(1) infinite; }
${P} .px-fig { animation: none; }
/* music: hop on the beat, quick feet, happy squint */
${P}[data-state="music"] .px-fig { animation: px-bob .5s steps(1) infinite; }
${P}[data-state="music"] .px-fA, ${P}[data-state="music"] .px-fB { animation-duration: .25s; }
${P}[data-state="music"] .px-e-open, ${P}[data-state="done"] .px-e-open, ${P}[data-state="sleep"] .px-e-open, ${P}[data-state="attention"] .px-e-open { display: none; }
${P}[data-state="music"] .px-e-happy, ${P}[data-state="done"] .px-e-happy, ${P}[data-state="attention"] .px-e-wide, ${P}[data-state="sleep"] .px-e-shut { display: inline; }
${P}[data-state="music"] .px-m-flat, ${P}[data-state="done"] .px-m-flat, ${P}[data-state="attention"] .px-m-flat, ${P}[data-state="sleep"] .px-m-flat { display: none; }
${P}[data-state="music"] .px-m-grin, ${P}[data-state="done"] .px-m-grin, ${P}[data-state="attention"] .px-m-o { display: inline; }
/* thinking: looks up and to the side, feet still, three dots blink one after another */
${P}[data-state="thinking"] .px-eyes { translate: 1px -1px; }
${P}[data-state="thinking"] .px-fA, ${P}[data-state="thinking"] .px-fB { animation: none; } ${P}[data-state="thinking"] .px-fB { display: none; }
${P}[data-state="thinking"] .px-dots { display: inline; }
${P}[data-state="thinking"] .px-d { animation: px-dt 1.2s steps(1) infinite; } ${P}[data-state="thinking"] .px-d2 { animation-delay: .2s; } ${P}[data-state="thinking"] .px-d3 { animation-delay: .4s; }
/* done: stepped jump, happy face, sparkles blink */
${P}[data-state="done"] .px-fig { animation: px-jump .6s steps(1) 1, px-bob 1.8s steps(1) .6s infinite; }
${P}[data-state="done"] .px-spk { display: inline; } ${P}[data-state="done"] .px-sk1 { animation: px-dt .5s steps(1) infinite; } ${P}[data-state="done"] .px-sk2 { animation: px-dt .5s steps(1) .25s infinite; }
/* attention: shakes, wide eyes, round mouth, a blinking "!" */
${P}[data-state="attention"] .px-fig { animation: px-shake .16s steps(1) 6, px-bob .7s steps(1) 1s infinite; }
${P}[data-state="attention"] .px-excl { display: inline; animation: px-dt .5s steps(1) infinite; }
/* file: looks at the sheet it holds */
${P}[data-state="file"] .px-eyes { translate: 1px 0; } ${P}[data-state="file"] .px-doc { display: inline; animation: px-pap 1.2s steps(1) infinite; }
/* sleep: sinks, feet gone, eyes shut, z drifts up in steps */
${P}[data-state="sleep"] .px-fig { translate: 0 ${U}px; animation: px-sl 2.4s steps(1) infinite; }
${P}[data-state="sleep"] .px-fA, ${P}[data-state="sleep"] .px-fB { display: none; }
${P}[data-state="sleep"] .px-zs { display: inline; } ${P}[data-state="sleep"] .px-z { animation: px-zf 2.4s steps(4) infinite; } ${P}[data-state="sleep"] .px-z2 { animation-delay: .8s; } ${P}[data-state="sleep"] .px-z3 { animation-delay: 1.6s; }
/* energy (music, audio levels): whole cells up, set by JS as translate; no CSS loop */
${P}[data-energy] .px-fig { animation: none; }
/* petted: happy squint, little hop, hearts */
${P}.poked .px-e-open, ${P}.poked .px-e-wide, ${P}.poked .px-e-shut { display: none !important; } ${P}.poked .px-e-happy { display: inline !important; }
${P}.act-pet .px-fig { animation: px-jump .6s steps(1) 1, px-bob 1.8s steps(1) .6s infinite; }
${P}.poked .px-hearts { display: inline; } ${P}.poked .px-h { animation: px-ht 1s steps(3) 1 both; } ${P}.poked .px-h2 { animation-delay: .25s; }
${P}.act-blink .px-e-open { animation: px-dt .2s steps(1) 1; } ${P}.act-twitch .px-fig { animation: px-jump .5s steps(1) 2; }
@keyframes px-fa { 0% { opacity: 1; } 50% { opacity: 0; } }
@keyframes px-fb { 0% { opacity: 0; } 50% { opacity: 1; } }
@keyframes px-bob { 0%, 100% { transform: none; } 50% { transform: translateY(-${U}px); } }
@keyframes px-jump { 0%, 100% { transform: none; } 20% { transform: translateY(-7px); } 40% { transform: translateY(-10.5px); } 60% { transform: translateY(-7px); } 80% { transform: translateY(-3.5px); } }
@keyframes px-shake { 0%, 100% { transform: translateX(-${U}px); } 50% { transform: translateX(${U}px); } }
@keyframes px-dt { 0% { opacity: 1; } 50% { opacity: .15; } }
@keyframes px-pap { 0%, 100% { transform: none; } 50% { transform: translateY(-2px); } }
@keyframes px-sl { 0%, 100% { transform: none; } 50% { transform: translateY(1.75px); } }
@keyframes px-zf { 0% { transform: translate(0, 3px); opacity: 0; } 25% { opacity: 1; } 75% { opacity: 1; } 100% { transform: translate(5px, -8px); opacity: 0; } }
@keyframes px-ht { 0% { transform: translateY(3px); opacity: 1; } 100% { transform: translateY(-9px); opacity: 0; } }`,
      init(host, api) { host._px = api.$('.px-fig'); },
      look(host, dx, dy) { // whole cells only: -1 / 0 / +1 across, and one cell up / down (the gaze reaches the far screen corners)
        const q = (v, th) => (v == null || Math.abs(v) < th ? 0 : v < 0 ? -1 : 1);
        host.style.setProperty('--pxlk', q(dx, 0.3)); host.style.setProperty('--pxlky', q(dy, 0.5));
      },
      energy(host, e) { if (host._px) host._px.style.translate = e == null ? '' : `0 ${-Math.round(e * 2) * U}px`; },
    });
  }

  // =====================================================================================================================================
  // 2. spark: a small round glowing face with a flame tip, an ember that shows the mood.
  // =====================================================================================================================================
  {
    const P = scope('spark');
    const eye = (x) => `<g class="sk-eo"><ellipse cx="${x}" cy="29" rx="2.3" ry="3.4" fill="${OL}"/><circle cx="${x + 0.8}" cy="27.6" r=".85" fill="#fff"/></g>`;
    window.PillPets.add({
      id: 'spark', group: 'characters', fx: false, w: 48, h: 48,
      name: { ru: 'Искра', en: 'Spark' },
      desc: { ru: 'Огонёк с лицом: цвет и мина показывают настроение', en: 'A glowing ember face: colour and expression show the mood' },
      svg: () => `
<g class="sk-flame"><path d="M24 15.5C19.5 12 21.5 6.5 24 2C26.5 6.5 28.5 12 24 15.5Z"/></g>
<circle class="sk-body" cx="24" cy="31" r="15.5"/>
<ellipse cx="24" cy="41.5" rx="11" ry="4.5" fill="#000" opacity=".07"/>
<ellipse cx="17.5" cy="22.5" rx="4.6" ry="2.5" fill="#fff" opacity=".6" transform="rotate(-28 17.5 22.5)"/>
<g class="sk-face">
  <circle class="sk-ch" cx="14.3" cy="34.6" r="2.4"/><circle class="sk-ch" cx="33.7" cy="34.6" r="2.4"/>
  <g class="sk-open">${eye(18.5)}${eye(29.5)}</g>
  <g class="sk-happy"><path d="M15.3 30.5Q18.5 24.6 21.7 30.5M26.3 30.5Q29.5 24.6 32.7 30.5" class="sk-st"/></g>
  <g class="sk-shut"><path d="M15.5 29Q18.5 32.4 21.5 29M26.5 29Q29.5 32.4 32.5 29" class="sk-st"/></g>
  <path class="sk-m sk-m-s sk-st" d="M20.8 35Q24 38.6 27.2 35"/>
  <path class="sk-m sk-m-d" d="M20.6 34.2Q24 41.5 27.4 34.2Z" fill="${OL}"/><path class="sk-m sk-m-d" d="M22.4 38.2Q24 36.6 25.6 38.2Q24 40.2 22.4 38.2Z" fill="#ff8a8a"/>
  <path class="sk-m sk-m-f sk-st" d="M21.8 36.6H26.2"/>
  <ellipse class="sk-m sk-m-o" cx="24" cy="36.6" rx="1.8" ry="2.3" fill="${OL}"/>
  <path class="sk-m sk-m-z sk-st" d="M22.8 36.2q1.2 1 2.4 0"/>
</g>
<g class="sk-glint" transform="translate(38 9)"><g class="sk-gi"><path d="${STAR}" fill="#fff3b0"/></g></g>
${props({ z: [37, 17], ex: [39, 9], dots: [35, 8], paper: [39, 12], hearts: [38, 10], spark: [38, 9] })}`,
      css: `${propsCss(P)}
${glanceKf(P, '.sk-face', 3)} ${blinkKf}
${P} .sk-body { fill: ${mix('#f7f2e8')}; }
${P} .sk-flame { fill: ${mix('#ffb347', 'var(--t-mid)')}; transform-box: fill-box; transform-origin: 50% 100%; animation: sk-fl 1.3s ease-in-out infinite alternate; }
${P} .sk-ch { fill: ${mix('#f5a0aa', 'var(--t-mid)')}; opacity: .65; }
${P} .sk-st { fill: none; stroke: ${OL}; stroke-width: 1.9; stroke-linecap: round; stroke-linejoin: round; }
${P} .sk-face { transition: translate .15s ease-out; translate: calc(var(--lx, 0) * 2.4px) calc(var(--ly, 0) * 1.6px); }
${P} .sk-eo { transform-box: fill-box; transform-origin: 50% 50%; }
${P}.act-blink .sk-eo { animation: ch-blink .2s ease-in-out; }
${P} .sk-happy, ${P} .sk-shut, ${P} .sk-m, ${P} .sk-glint { display: none; }
${P} .sk-m-s { display: inline; }
${P} .sk-gi { transform-box: fill-box; transform-origin: 50% 50%; animation: sk-gl 3.6s ease-in-out infinite; }
${P}[data-state="calm"] .sk-glint, ${P}[data-state="music"] .sk-glint { display: inline; }
/* music: squint + open mouth, flame dances, face bobs */
${P}[data-state="music"] .sk-open, ${P}[data-state="done"] .sk-open, ${P}[data-state="sleep"] .sk-open { display: none; }
${P}[data-state="music"] .sk-happy, ${P}[data-state="done"] .sk-happy, ${P}[data-state="sleep"] .sk-shut { display: inline; }
${P}[data-state="music"] .sk-m-s { display: none; } ${P}[data-state="music"] .sk-m-d { display: inline; }
${P}[data-state="music"] .sk-flame { animation: sk-fl .45s ease-in-out infinite alternate; }
${P}[data-state="music"] .sk-gi { animation-duration: 1s; }
/* thinking: small calm flame, flat mouth, glances up */
${P}[data-state="thinking"] .sk-m-s, ${P}[data-state="attention"] .sk-m-s, ${P}[data-state="sleep"] .sk-m-s { display: none; }
${P}[data-state="thinking"] .sk-m-f { display: inline; } ${P}[data-state="thinking"] .sk-face { translate: 2px -2.2px; }
${P}[data-state="thinking"] .sk-eo { transform: scaleY(.8); }
${P}[data-state="thinking"] .sk-flame { transform: scale(.8); animation: none; }
/* done / attention / file / sleep */
${P}[data-state="done"] .sk-ch { opacity: 1; }
${P}[data-state="attention"] .sk-m-o { display: inline; } ${P}[data-state="attention"] .sk-eo { transform: scale(1.28); }
${P}[data-state="attention"] .sk-flame { animation: sk-fl .22s ease-in-out infinite alternate; transform: scaleY(1.25); }
${P}[data-state="file"] .sk-face { translate: 2.8px .4px; }
${P}[data-state="sleep"] .sk-body { fill: #8f93a6; } ${P}[data-state="sleep"] .sk-m-z { display: inline; } ${P}[data-state="sleep"] .sk-ch { opacity: .3; }
${P}[data-state="sleep"] .sk-flame { transform: scale(.45); animation: none; opacity: .6; }
/* petted: happy squint */
${P}.poked .sk-open, ${P}.poked .sk-shut { display: none !important; } ${P}.poked .sk-happy { display: inline !important; }
${P}.act-twitch .sk-flame { animation: sk-fl .12s ease-in-out 5 alternate; }
@keyframes sk-fl { from { transform: scale(.88, .8) rotate(-3deg); } to { transform: scale(1.04, 1.1) rotate(3deg); } }
@keyframes sk-gl { 0%, 55%, 100% { transform: scale(.2); opacity: 0; } 70% { transform: scale(1) rotate(45deg); opacity: 1; } 85% { transform: scale(.5) rotate(90deg); opacity: .6; } }`,
      init(host, api) { host._fl = api.$('.sk-flame'); },
      energy(host, e) { if (host._fl) host._fl.style.scale = e == null ? '' : `1 ${(1 + e * 0.8).toFixed(2)}`; },
    });
  }

  // =====================================================================================================================================
  // 3. eyes: only a pair of eyes. Pills that blink, follow the cursor, squint into arcs, widen, close.
  // =====================================================================================================================================
  {
    const P = scope('eyes');
    const one = (x, c) => `<g transform="translate(${x} 22)" class="ey ${c}">
  <g class="ey-o"><rect class="ey-f" x="-6" y="-10" width="12" height="20" rx="6"/></g>
  <path class="ey-s ey-h" d="M-7 4Q0 -9 7 4"/><path class="ey-s ey-c" d="M-7 -1Q0 7 7 -1"/></g>`;
    window.PillPets.add({
      id: 'eyes', group: 'characters', fx: false, w: 60, h: 40,
      name: { ru: 'Глазки', en: 'Eyes' },
      desc: { ru: 'Только глаза: моргают, смотрят за курсором, щурятся', en: 'Just the eyes: blink, follow the cursor, squint' },
      svg: () => `<g class="ey-all">${one(19, 'ey-l')}${one(41, 'ey-r')}<path class="ey-s ey-sm" d="M26 31.5Q30 36 34 31.5"/></g>
${props({ z: [51, 14], ex: [53, 8], dots: [30, 5], paper: [53, 11], hearts: [30, 6], spark: [52, 8] })}`,
      css: `${propsCss(P)}
${glanceKf(P, '.ey-all', 5)} ${blinkKf}
${P} { --ey: ${mix('#f1eee8')}; }
${P}[data-state="sleep"] { --ey: #7f8296; }
${P} .ey-f { fill: var(--ey); }
${P} .ey-s { fill: none; stroke: var(--ey); stroke-width: 3.4; stroke-linecap: round; display: none; }
${P} .ey-sm { stroke-width: 2.2; }
${P} .ey-all { translate: calc(var(--lx, 0) * 4px) calc(var(--ly, 0) * 3px); transition: translate .12s ease-out; }
${P} .ey-o, ${P} .ey-s { transform-box: fill-box; transform-origin: 50% 50%; }
${P}.act-blink .ey-o { animation: ch-blink .2s ease-in-out; }
/* music: arcs + smile, the two eyes bounce in turn */
${P}[data-state="music"] .ey-o, ${P}[data-state="done"] .ey-o, ${P}[data-state="sleep"] .ey-o { display: none; }
${P}[data-state="music"] .ey-h, ${P}[data-state="done"] .ey-h, ${P}[data-state="sleep"] .ey-c { display: inline; }
${P}[data-state="music"] .ey-sm, ${P}[data-state="done"] .ey-sm { display: inline; }
${P}[data-state="music"] .ey-h { animation: ey-bn .55s ease-in-out infinite alternate; } ${P}[data-state="music"] .ey-r .ey-h { animation-delay: .27s; }
${P}[data-state="music"] .ey-sm { animation: ey-sm .55s ease-in-out infinite alternate; }
/* thinking: lazy lidded pills glancing around */
${P}[data-state="thinking"] .ey-o { transform: scale(.92, .72); } ${P}[data-state="thinking"] .ey-all { translate: 0 -2.5px; animation: ey-scan 2.4s ease-in-out infinite alternate; }
/* attention: tall alert pills, file: look right at the sheet, sleep: closed, sunk */
${P}[data-state="attention"] .ey-o { transform: scale(1.12, 1.2); }
${P}[data-state="file"] .ey-all { translate: 6px 1px; }
${P}[data-state="sleep"] .ey-all { translate: 0 5px; }
${P}[data-state="done"] .ey-h { animation: ey-bn .8s ease-out 1; }
/* petted: happy arcs + smile */
${P}.poked .ey-o, ${P}.poked .ey-c { display: none !important; } ${P}.poked .ey-h, ${P}.poked .ey-sm { display: inline !important; }
@keyframes ey-bn { from { transform: translateY(0); } to { transform: translateY(-3px); } }
@keyframes ey-sm { from { transform: scaleX(1); } to { transform: scaleX(1.25); } }
@keyframes ey-scan { from { transform: translateX(-4px); } to { transform: translateX(4px); } }`,
    });
  }

  // =====================================================================================================================================
  // 4. bot: a robot with a screen for a face; the face is drawn with bright pixels (12 x 7 grid).
  // =====================================================================================================================================
  {
    const P = scope('bot');
    window.PillPets.add({
      id: 'bot', group: 'characters', fx: false, w: 48, h: 48,
      name: { ru: 'Бот', en: 'Bot' },
      desc: { ru: 'Робот с экраном вместо лица: настроение рисуют светящиеся пиксели', en: 'A robot with a screen for a face: the mood is drawn in glowing pixels' },
      svg: (api) => {
        const F = (cls, rows) => api.g8('bt-f ' + cls, api.cells(rows, { x: '.bf' }));
        return `
<g class="bt-feet"><rect x="16.5" y="42.5" width="6" height="4.5" rx="1.8" fill="#8b99a8"/><rect x="25.5" y="42.5" width="6" height="4.5" rx="1.8" fill="#8b99a8"/></g>
<rect x="15" y="32" width="18" height="12" rx="4.5" fill="#9aa8b6"/><circle class="bt-chest" cx="24" cy="38.5" r="2" />
<rect x="9.5" y="35" width="4" height="7" rx="2" fill="#8b99a8"/><rect x="34.5" y="35" width="4" height="7" rx="2" fill="#8b99a8"/>
<g class="head">
  <g class="bt-ant"><rect x="23.2" y="3.5" width="1.6" height="6" fill="#8b99a8"/><circle class="bt-led" cx="24" cy="3.4" r="2.4"/></g>
  <rect x="4.5" y="18" width="3" height="8" rx="1.5" fill="#8b99a8"/><rect x="40.5" y="18" width="3" height="8" rx="1.5" fill="#8b99a8"/>
  <rect class="bt-head" x="7" y="9" width="34" height="26" rx="8"/>
  <rect x="10.5" y="12.5" width="27" height="19" rx="5" fill="#0a1218"/>
  <g class="bt-face"><g transform="translate(10.8 14.3) scale(2.2)">
    ${F('bt-f-calm', ['', '', '..xx....xx..', '..xx....xx..', '..xx....xx..'])}
    ${F('bt-f-hap', ['', '..x......x..', '.x.x....x.x.', '', '....x..x....', '.....xx.....'])}
    ${F('bt-f-done', ['', '..x......x..', '.x.x....x.x.', '', '...x....x...', '....xxxx....'])}
    ${F('bt-f-d1', ['', '', '', '..xx', '..xx'])}${F('bt-f-d2', ['', '', '', '.....xx', '.....xx'])}${F('bt-f-d3', ['', '', '', '........xx', '........xx'])}
    ${F('bt-f-att', ['.....xx.....', '.....xx.....', '.....xx.....', '.....xx.....', '', '.....xx.....', '.....xx.....'])}
    ${F('bt-f-slp', ['', '', '', '.xxx....xxx.'])}
  </g></g>
  <path d="M12.5 15.5Q18 13.4 26 13.4" stroke="#fff" stroke-width="1" opacity=".12" fill="none" stroke-linecap="round"/>
</g>
${props({ z: [40, 12], ex: [42, 9], paper: [41, 14], hearts: [38, 8], spark: [40, 8] })}`;
      },
      css: `${propsCss(P)}
${glanceKf(P, '.bt-face', 3)}
${P} { --bf: ${mix('#5eead4')}; }
${P}[data-state="sleep"] { --bf: #4f6672; }
${P} .bf { fill: var(--bf); filter: drop-shadow(0 0 .6px var(--bf)); }
${P} .bt-head { fill: ${mix('#cfd8e1', 'var(--t-lo)')}; }
${P} .bt-chest { fill: ${mix('#f6c344', 'var(--t-mid)')}; stroke: ${OL}; stroke-width: .8; }
${P} .bt-led { fill: ${mix('#ff6f6f', 'var(--t-hi)')}; }
${P} .bt-ant { transform-box: fill-box; transform-origin: 50% 100%; transition: transform .25s ease-out; }
${P} .bt-face { translate: calc(var(--lx, 0) * 2.6px) calc(var(--ly, 0) * 1.4px); transition: translate .15s ease-out; }
${P} .bt-f { display: none; transform-box: fill-box; transform-origin: 50% 50%; }
${P} .bt-f-calm { display: inline; }
${P}.act-blink .bt-f-calm { animation: bt-bl .2s ease-in-out; }
@keyframes bt-bl { 45% { transform: scaleY(.1); } }
${P}[data-state="music"] .bt-f-calm, ${P}[data-state="done"] .bt-f-calm, ${P}[data-state="thinking"] .bt-f-calm, ${P}[data-state="attention"] .bt-f-calm, ${P}[data-state="sleep"] .bt-f-calm { display: none; }
${P}[data-state="music"] .bt-f-hap, ${P}[data-state="done"] .bt-f-done, ${P}[data-state="thinking"] .bt-f-d1, ${P}[data-state="thinking"] .bt-f-d2, ${P}[data-state="thinking"] .bt-f-d3, ${P}[data-state="attention"] .bt-f-att, ${P}[data-state="sleep"] .bt-f-slp { display: inline; }
${P}[data-state="music"] .bt-led { animation: bt-led .55s steps(2, jump-none) infinite; } ${P}[data-state="music"] .bt-chest { animation: bt-chest .55s steps(2, jump-none) infinite; }
${P}[data-state="thinking"] .bt-f-d1, ${P}[data-state="thinking"] .bt-f-d2, ${P}[data-state="thinking"] .bt-f-d3 { animation: bt-dot 1.2s steps(1) infinite; }
${P}[data-state="thinking"] .bt-f-d2 { animation-delay: .3s; } ${P}[data-state="thinking"] .bt-f-d3 { animation-delay: .6s; }
${P}[data-state="thinking"] .bt-led { animation: bt-led .5s steps(2, jump-none) infinite; } ${P}[data-state="thinking"] .bt-face { translate: 0 0; }
${P}[data-state="attention"] .bt-led { animation: bt-led .28s steps(2, jump-none) infinite; } ${P}[data-state="attention"] .bt-ant { transform: scaleY(1.25); }
${P}[data-state="attention"] .bt-f-att { animation: bt-led .5s steps(2, jump-none) infinite; }
${P}[data-state="file"] .bt-face { translate: 2.6px .4px; }
${P}[data-state="sleep"] .bt-led { opacity: .3; } ${P}[data-state="sleep"] .bt-face { translate: 0 1.5px; }
${P}.act-twitch .bt-ant { animation: bt-fk .5s ease-in-out; }
${P}.poked .bt-f { display: none !important; } ${P}.poked .bt-f-hap { display: inline !important; }
@keyframes bt-led { 50% { opacity: .25; } } @keyframes bt-chest { 50% { fill: #ff8fa3; } } @keyframes bt-dot { 0%, 100% { opacity: .2; } 20%, 70% { opacity: 1; } }
@keyframes bt-fk { 0%, 100% { transform: rotate(0); } 30% { transform: rotate(-16deg); } 60% { transform: rotate(9deg); } }`,
      init(host, api) { host._bf = api.$('.bt-face'); },
      energy(host, e) { if (host._bf) host._bf.style.scale = e == null ? '' : (1 + e * 0.14).toFixed(3); },
    });
  }

  // =====================================================================================================================================
  // 5. kaomoji: a living text face ( · ω · ) built from real paths (no fonts), the expression follows the state.
  // =====================================================================================================================================
  {
    const P = scope('kaomoji');
    const E = (cls, fn) => `<g class="${cls}">${fn(36)}${fn(60)}</g>`;
    window.PillPets.add({
      id: 'kaomoji', group: 'characters', fx: false, w: 96, h: 48,
      name: { ru: 'Каомодзи', en: 'Kaomoji' },
      desc: { ru: 'Живая текстовая рожица ( · ω · ), мимика под настроение', en: 'A living text face ( · ω · ) that changes expression with the mood' },
      svg: () => `<g class="kx-face">
  <g class="kx-arms"><path class="kx-s" d="M6 6L15 17M90 6L81 17"/></g>
  <g class="kx-tilde"><path class="kx-s" d="M2 27q3-4 6 0t6 0"/><path class="kx-s" d="M82 27q3-4 6 0t6 0"/></g>
  <path class="kx-s" d="M23 8Q10.5 27 23 46"/><path class="kx-s" d="M73 8Q85.5 27 73 46"/>
  <ellipse class="kx-ch" cx="27" cy="31.5" rx="3.2" ry="1.9"/><ellipse class="kx-ch" cx="69" cy="31.5" rx="3.2" ry="1.9"/>
  <g class="kx-eyes">
    ${E('kx-e kx-ed', (x) => `<circle class="kx-eb kx-f" cx="${x}" cy="23.5" r="2.8"/>`)}
    ${E('kx-e kx-eh', (x) => `<path class="kx-s" d="M${x - 4.5} 26Q${x} 18 ${x + 4.5} 26"/>`)}
    ${E('kx-e kx-ew', (x) => `<circle class="kx-s" cx="${x}" cy="23.5" r="5.4"/><circle class="kx-f" cx="${x}" cy="23.5" r="2.1"/>`)}
    ${E('kx-e kx-es', (x) => `<path class="kx-s" d="M${x - 4.5} 24H${x + 4.5}"/>`)}
  </g>
  <g transform="translate(48 31.5)">
    <path class="kx-m kx-mw kx-s" d="M-5.5 -1.5Q-5.5 3.2 -2.7 3.2Q0 3.2 0 -.5Q0 3.2 2.7 3.2Q5.5 3.2 5.5 -1.5"/>
    <path class="kx-m kx-mf kx-s" d="M-4 1H4"/>
    <path class="kx-m kx-mt kx-f" d="M-4.8 -1.5H4.8L0 4.6Z"/>
    <circle class="kx-m kx-mo kx-s" r="2.8" cy="1"/>
  </g>
  <g class="kx-arrow"><path class="kx-s" d="M84 27H93M89.5 23.2L93.5 27L89.5 30.8"/></g>
</g>
<g class="kx-notes"><g class="kx-n1"><g transform="translate(87 18)"><ellipse rx="2.4" ry="1.8" transform="rotate(-20)" class="kx-f"/><path class="kx-s kx-sn" d="M2 -.5V-9q3 .5 3.5 4"/></g></g><g class="kx-n2"><g transform="translate(9 16) scale(.85)"><ellipse rx="2.4" ry="1.8" transform="rotate(-20)" class="kx-f"/><path class="kx-s kx-sn" d="M2 -.5V-9q3 .5 3.5 4"/></g></g></g>
${props({ z: [86, 17], ex: [88, 9], dots: [48, 4], paper: [88, 12], hearts: [48, 6], spark: [88, 9] })}`,
      css: `${propsCss(P)}
${glanceKf(P, '.kx-eyes', 4)} ${blinkKf}
${P} { --kc: ${mix('#eceaf2')}; }
${P}[data-state="sleep"] { --kc: #8e8c9c; }
${P} .kx-s { fill: none; stroke: var(--kc); stroke-width: 2.4; stroke-linecap: round; stroke-linejoin: round; }
${P} .kx-f { fill: var(--kc); stroke: none; } ${P} .kx-sn { stroke-width: 1.5; }
${P} .kx-ch { fill: ${mix('#ff9ab8', 'var(--t-mid)')}; opacity: .35; }
${P} .kx-e, ${P} .kx-m, ${P} .kx-arms, ${P} .kx-tilde, ${P} .kx-arrow, ${P} .kx-notes { display: none; }
${P} .kx-ed, ${P} .kx-mw { display: inline; }
${P} .kx-eyes { translate: calc(var(--lx, 0) * 2.4px) calc(var(--ly, 0) * 1.6px); transition: translate .14s ease-out; }
${P} .kx-face { transition: translate .18s ease-out; translate: calc(var(--lx, 0) * 1.2px) 0; }
${P} .kx-eb { transform-box: fill-box; transform-origin: 50% 50%; } ${P}.act-blink .kx-eb { animation: ch-blink .2s ease-in-out; }
/* music: ^ ω ^ bouncing, ~ waves on both sides, notes float */
${P}[data-state="music"] .kx-ed, ${P}[data-state="done"] .kx-ed { display: none; }
${P}[data-state="music"] .kx-eh, ${P}[data-state="done"] .kx-eh, ${P}[data-state="attention"] .kx-ew, ${P}[data-state="sleep"] .kx-es { display: inline; }
${P}[data-state="attention"] .kx-ed, ${P}[data-state="sleep"] .kx-ed { display: none; }
${P}[data-state="music"] .kx-face { animation: kx-bn .275s ease-in-out infinite alternate; }
${P}[data-state="music"] .kx-tilde, ${P}[data-state="music"] .kx-notes { display: inline; } ${P}[data-state="music"] .kx-tilde { animation: kx-wv .55s ease-in-out infinite alternate; }
${P}[data-state="music"] .kx-n1 { animation: kx-nt 1.6s ease-out infinite; } ${P}[data-state="music"] .kx-n2 { animation: kx-nt 1.6s ease-out .8s infinite; } ${P}[data-state="music"] .kx-notes { opacity: .9; }
/* thinking: · _ · , eyes up, dots above; done: \\( ^ ▽ ^ )/; attention: ( ◎ o ◎ ) !; file: looks at the paper + arrow; sleep: ( - ω - ) zzZ */
${P}[data-state="thinking"] .kx-mw, ${P}[data-state="done"] .kx-mw, ${P}[data-state="attention"] .kx-mw { display: none; }
${P}[data-state="thinking"] .kx-mf { display: inline; } ${P}[data-state="done"] .kx-mt { display: inline; } ${P}[data-state="attention"] .kx-mo { display: inline; }
${P}[data-state="thinking"] .kx-eyes { translate: 2px -2px; } ${P}[data-state="thinking"] .kx-face { rotate: -3deg; transform-origin: 48px 40px; }
${P}[data-state="done"] .kx-arms { display: inline; animation: kx-aw .5s ease-in-out infinite alternate; transform-box: view-box; transform-origin: 48px 20px; } ${P}[data-state="done"] .kx-ch { opacity: .7; }
${P}[data-state="attention"] .kx-eyes { transform: scale(1.04); }
${P}[data-state="file"] .kx-eyes { translate: 3.2px 0; } ${P}[data-state="file"] .kx-arrow { display: inline; animation: kx-ar .8s ease-in-out infinite alternate; }
${P}[data-state="sleep"] .kx-face { translate: 0 2px; }
${P}.poked .kx-e, ${P}.poked .kx-mt, ${P}.poked .kx-mf, ${P}.poked .kx-mo { display: none !important; } ${P}.poked .kx-eh, ${P}.poked .kx-mw { display: inline !important; }
@keyframes kx-bn { from { transform: translateY(0); } to { transform: translateY(-3px); } }
@keyframes kx-wv { from { transform: translateY(-2px); } to { transform: translateY(2px); } }
@keyframes kx-nt { 0% { transform: translate(0, 6px); opacity: 0; } 20% { opacity: 1; } 100% { transform: translate(3px, -10px); opacity: 0; } }
@keyframes kx-aw { from { transform: rotate(-6deg); } to { transform: rotate(6deg); } }
@keyframes kx-ar { from { transform: translateX(0); } to { transform: translateX(2px); } }`,
    });
  }

  // =====================================================================================================================================
  // 6. swarm: 16 dots. Every state is a target shape; the dots glide there with CSS transitions (translate / scale / opacity).
  // =====================================================================================================================================
  {
    const P = scope('swarm'), N = 16, CX = 24, CY = 26;
    const ring = (r, wob = 0) => Array.from({ length: N }, (_, i) => { const a = (i / N) * Math.PI * 2 - Math.PI / 2, rr = r + wob * Math.sin(i * 2.4); return [CX + Math.cos(a) * rr, CY + Math.sin(a) * rr]; });
    const wave = (ph, amp) => Array.from({ length: N }, (_, i) => [5 + (i * 38) / (N - 1), CY + Math.sin(ph + i * 0.75) * amp]);
    const along = (pts, closed) => { // N points evenly spaced along a polyline
      const Q = closed ? [...pts, pts[0]] : pts, L = [0];
      for (let i = 1; i < Q.length; i++) L.push(L[i - 1] + Math.hypot(Q[i][0] - Q[i - 1][0], Q[i][1] - Q[i - 1][1]));
      return Array.from({ length: N }, (_, k) => {
        const d = (closed ? k / N : k / (N - 1)) * L[L.length - 1]; let i = 1;
        while (i < L.length - 1 && L[i] < d) i++;
        const t = (d - L[i - 1]) / (L[i] - L[i - 1] || 1);
        return [Q[i - 1][0] + (Q[i][0] - Q[i - 1][0]) * t, Q[i - 1][1] + (Q[i][1] - Q[i - 1][1]) * t];
      });
    };
    let seed = 7; const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    const SHAPE = {
      calm: ring(14, 1.8), thinking: ring(14), music: wave(0, 8),
      done: along([[8, 27], [19, 38], [41, 12]]),
      attention: [...[5, 10, 15, 20, 25].flatMap((y) => [[21.4, y], [26.6, y]]), [24, 29.5], [24, 40.5], [22, 38.5], [26, 38.5], [22, 42.5], [26, 42.5]],
      file: along([[13, 10], [28, 10], [35, 17], [35, 41], [13, 41]], true),
      sleep: Array.from({ length: N }, () => [6 + rnd() * 36, 31 + rnd() * 13]),
    };
    const look = (s, i) => (s === 'thinking' ? { sc: 0.5 + 0.55 * (i / (N - 1)), op: 0.25 + 0.75 * (i / (N - 1)) } : s === 'sleep' ? { sc: 0.85, op: 0.85 } : { sc: 1, op: 1 });
    const put = (d, [x, y], s, i, jit = 0) => { const l = look(s, i); d.style.translate = `${(x + (jit ? (Math.random() - 0.5) * jit : 0)).toFixed(1)}px ${(y + (jit ? (Math.random() - 0.5) * jit : 0)).toFixed(1)}px`; d.style.scale = l.sc; d.style.opacity = l.op; };
    window.PillPets.add({
      id: 'swarm', group: 'characters', fx: false, w: 48, h: 48,
      name: { ru: 'Рой', en: 'Swarm' },
      desc: { ru: 'Стая точек: кольцо, волна, орбита, галочка, «!»', en: 'A flock of dots: ring, wave, orbit, check mark, "!"' },
      svg: () => `<g class="sw-look"><g class="sw-g">${Array.from({ length: N }, (_, i) => `<circle class="sw" style="--i:${i}" cx="0" cy="0" r="2.2"/>`).join('')}</g></g>`,
      css: `
${P} { --sc: ${mix('#b6c6ea')}; }
${P}[data-state="sleep"] { --sc: #a3a9bd; }
${P} .sw { fill: var(--sc); transition: translate .8s cubic-bezier(.34, 1.25, .5, 1), scale .5s, opacity .5s; transition-delay: calc(var(--i) * 14ms); }
${P}[data-state="music"] .sw { transition: scale .3s, opacity .3s; transition-delay: 0s; }
${P} .sw-look { translate: calc(var(--lx, 0) * 2px) calc(var(--ly, 0) * 1.5px); transition: translate .2s ease-out; }
${P} .sw-g { transform-origin: ${CX}px ${CY}px; transform-box: view-box; }
${P}[data-state="calm"] .sw-g { animation: sw-spin 22s linear infinite, sw-br 3.4s ease-in-out infinite alternate; }
${P}[data-state="thinking"] .sw-g { animation: sw-spin 1.7s linear infinite; }
${P}[data-state="sleep"] .sw-g { animation: sw-sl 5s ease-in-out infinite alternate; }
${P}.act-twitch .sw-look { animation: sw-pl .6s ease-in-out; }
@keyframes sw-spin { to { rotate: 360deg; } }
@keyframes sw-br { from { transform: scale(.93); } to { transform: scale(1.07); } }
@keyframes sw-sl { from { transform: translateY(0); } to { transform: translateY(1.5px); } }
@keyframes sw-pl { 50% { transform: scale(1.15); } }`,
      init(host, api) { host._sw = { dots: api.$$('.sw'), s: 'calm', boost: 0, e: null }; },
      state(host, s) {
        const w = host._sw; if (!w) return;
        w.s = s; clearTimeout(w.pk);
        w.dots.forEach((d, i) => put(d, SHAPE[s][i], s, i));
      },
      energy(host, e) { if (host._sw) host._sw.e = e; },
      poke(host) { // burst outwards, then regroup
        const w = host._sw; if (!w || w.s === 'music') { if (w) w.boost = performance.now(); return; }
        w.dots.forEach((d, i) => put(d, SHAPE[w.s][i], w.s, i, 18));
        clearTimeout(w.pk); w.pk = setTimeout(() => w.dots.forEach((d, i) => put(d, SHAPE[w.s][i], w.s, i)), 380);
      },
      tick(host, t) { // music: the wave runs on JS (a transition per frame would lag); amplitude follows the audio energy when there is any
        const w = host._sw; if (!w || w.s !== 'music') return;
        const amp = 3.5 + (w.e == null ? 0.35 : w.e) * 11 + Math.max(0, 1 - (performance.now() - w.boost) / 500) * 5;
        for (let i = 0; i < N; i++) w.dots[i].style.translate = `${(5 + (i * 38) / (N - 1)).toFixed(1)}px ${(CY + Math.sin(t / 170 - i * 0.7) * amp).toFixed(1)}px`;
      },
    });
  }
})();
