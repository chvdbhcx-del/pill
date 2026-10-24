// Pets: small original vector characters that live INSIDE the island's black body (clipped by it) and follow its reaction state.
// Registry: window.PillPets (also Pill.pets inside the island window). All art below is drawn from scratch (inline SVG, no external assets).
// Standalone-safe: settings.html and replica/pets-preview.html load this file without Pill; only the island hook at the end needs Pill.
// Pet API: PillPets.create(id, { state, side, size, idle, color }) -> { el, setState(s, color), tint(color), look, pet, setEnergy, setHidden, destroy }.
// States = the island reactions: calm music thinking done attention file sleep. CSS (pets.css) does the motion, JS only switches data-state.
// Tint ("painting"): the reaction colour goes into --pet-accent / --pet-k on the pet box; pets.css mixes it into accents only (cheeks, props, halo).
//
// ===== EXTENSION API: more pets from other files (loaded by <script> AFTER pets.js: pets-chars.js, pets-creatures.js, pets-things.js, pets-chars2.js) =====
//   PillPets.add({
//     id: 'kebab-id',                       // [a-z0-9-], unique; saved in the setting `pet`; an unknown saved id means "no pet"
//     group: 'characters' | 'creatures' | 'things',   // picker / preview section (the six built-ins are 'characters')
//     name: { ru, en }, desc: { ru, en },   // picker card title and subtitle (desc optional)
//     w: 48, h: 48,                         // SVG viewBox size. Box height = the usual S/M/L size, width = height * w / h, clamped to 0.5 .. 2.5 : 1
//                                           //   (the capsule grows by that width through Pill.reserve, like for any pet)
//     svg: '<g>..</g>' | (api) => string,   // INNER markup. Coordinates: origin top-left, viewBox w x h, the floor is y = h - 1, draw only inside 0..w x 0..h
//     fx: false,                            // optional: skip the shared props (z, hearts, sparkles, "!", thinking dots, file paper); default = included, anchored in a h x h square
//                                           //   on the free side of the pet and tinted like the built-ins
//     css: '.pet[data-pet="kebab-id"] .x { .. }',     // injected ONCE as <style data-pet-css="kebab-id">; every selector must start with .pet[data-pet="kebab-id"]
//     // hooks, all optional (host = the .pet wrapper element):
//     init(host, api), state(host, stateName, color), energy(host, e /* 0..1, ~30 fps, only in music; null when it stops */),
//     look(host, dx, dy /* -1..1 */), poke(host) /* click petting */, tick(host, tMs) /* shared rAF, only while visible, not under reduced motion */,
//   })
//   api for svg(api): { id, w, h, OL, ln, eyes, rc, cells, g8, STATES, TINT } (the same builders the built-ins use: ln = outlined stroke, eyes = open / happy / shut looks,
//   rc = <rect>, cells = ASCII pixel map -> rects, g8 = <g class>). api for init(): { id, w, h, host, svg, $(sel), $$(sel), reduced() }.
//   FOR FREE on the host: data-pet, data-state (calm|music|thinking|done|attention|file|sleep), data-side (right|left), data-size (s|m|l), data-where (cap|head),
//   data-look (8-bit style pets only), data-energy, the tint variables --pet-accent / --pet-k / --t-lo / --t-mid / --t-hi (use color-mix(in srgb, var(--pet-accent) var(--t-mid), base)),
//   --lx / --ly (look, -1..1), --d (prop drift direction), classes act-blink / act-twitch / act-hop (idle bursts), act-pet and poked (click petting), enter (fade-in);
//   the shared parts vocabulary of pets.css works on any markup: .fig .head .eyes .eye .earL .earR .tail .armL .armR .ant .led .bub .blush .hl .o (see pets.css), and the
//   rig breath / music dance / thinking sway / done jump / attention startle / sleep curl apply unless your css overrides them. Placement inside the island, clipping,
//   mirroring, reduced motion (animations off), pause when hidden, picker card with live preview and the replica/pets-preview.html cell are automatic.
//   Example: replica/pets-add-example.js.
(() => {
  'use strict';
  const OL = '#17141f'; // shared outline colour: reads on a black island and on a light wallpaper
  const STATES = ['calm', 'music', 'thinking', 'done', 'attention', 'file', 'sleep'];
  // reaction colours = the island edge palette (REACT in pill.js), used when the reaction carries no colour of its own; calm = no tint
  const TINT = { calm: '', music: '#d8836a', thinking: '#9e8fe8', done: '#7fae45', attention: '#e8a040', file: '#5a78a8', sleep: '#7d8cab' };

  // ---- tiny SVG builders ----
  // outlined stroke: a dark wide path under a coloured narrow one (tails, arms, antenna stems)
  const ln = (cls, d, w, col, ow = 2.4) => `<g${cls ? ` class="${cls}"` : ''}><path d="${d}" fill="none" stroke="${OL}" stroke-width="${w + ow}" stroke-linecap="round" stroke-linejoin="round"/><path d="${d}" fill="none" stroke="${col}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/></g>`;
  // three eye looks in one group: open (dots or LED bars), happy (^ ^), shut (curved lines); CSS shows one of them per state
  const eyes = (pts, kind, col, r, ink = OL) => {
    const open = pts.map(([x, y]) => (kind === 'led'
      ? `<g class="eye"><rect class="hl" style="--b:${col}" x="${x - 2}" y="${y - 2.8}" width="4" height="5.6" rx="2" fill="${col}"/></g>`
      : `<g class="eye"><ellipse cx="${x}" cy="${y}" rx="${r}" ry="${(r * (kind === 'tall' ? 1.28 : 1)).toFixed(2)}" fill="${col}"/><circle class="hl" style="--b:#fff" cx="${(x + r * 0.38).toFixed(2)}" cy="${(y - r * 0.42).toFixed(2)}" r="${(r * 0.32).toFixed(2)}" fill="#fff"/></g>`)).join('');
    const arc = (d) => `<path d="${d}" fill="none" stroke="${ink}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>`;
    const happy = pts.map(([x, y]) => arc(`M${(x - r * 1.15).toFixed(2)} ${(y + r * 0.55).toFixed(2)}Q${x} ${(y - r * 1.5).toFixed(2)} ${(x + r * 1.15).toFixed(2)} ${(y + r * 0.55).toFixed(2)}`)).join('');
    const shut = pts.map(([x, y]) => arc(`M${(x - r * 1.1).toFixed(2)} ${y}Q${x} ${(y + r * 1.1).toFixed(2)} ${(x + r * 1.1).toFixed(2)} ${y}`)).join('');
    return `<g class="eyes"><g class="e-open">${open}</g><g class="e-happy">${happy}</g><g class="e-shut">${shut}</g></g>`;
  };

  // ---- shared effects: drawn once per pet, shown by CSS depending on the state (z while asleep, ! on attention, sparkles when done, hearts when petted, ... while thinking) ----
  const zp = 'M-2.4 -2.9h4.8l-4.8 5.8h4.8';
  const heart = 'M0 3C-5.5 -1 -3.2 -5 0 -2.4C3.2 -5 5.5 -1 0 3Z';
  const star = 'M0-4.6L1.1-1.1L4.6 0L1.1 1.1L0 4.6L-1.1 1.1L-4.6 0L-1.1-1.1Z';
  const FX = `
<g class="fx">
  <g class="fx-z">
    <g class="z z1"><g transform="translate(0 0) scale(.78)">${ln('', zp, 1.6, '#e8ebff', 1.6)}</g></g>
    <g class="z z2"><g transform="translate(3 -5) scale(1)">${ln('', zp, 1.6, '#e8ebff', 1.6)}</g></g>
    <g class="z z3"><g transform="translate(-1 -11) scale(1.25)">${ln('', zp, 1.6, '#e8ebff', 1.6)}</g></g>
  </g>
  <g class="fx-heart">
    <g class="h h1"><path class="o" d="${heart}" fill="#ff7aa2" stroke-width="1.2" transform="scale(1.25)"/></g>
    <g class="h h2"><path class="o" d="${heart}" fill="#ff9ab8" stroke-width="1.2" transform="translate(4 -3)"/></g>
    <g class="h h3"><path class="o" d="${heart}" fill="#ff7aa2" stroke-width="1.2" transform="translate(-3 -6) scale(.8)"/></g>
  </g>
  <g class="fx-spark">
    <g class="s s1"><path class="o" d="${star}" fill="#ffe27a" stroke-width=".9" transform="translate(0 -4)"/></g>
    <g class="s s2"><path class="o" d="${star}" fill="#fff3b0" stroke-width=".9" transform="translate(5 5) scale(.75)"/></g>
    <g class="s s3"><path class="o" d="${star}" fill="#ffe27a" stroke-width=".9" transform="translate(-4 8) scale(.6)"/></g>
  </g>
  <g class="fx-excl"><g class="excl">
    <rect class="o" x="-1.8" y="-8" width="3.6" height="8" rx="1.8" fill="#ffb340" stroke-width="1.2"/><circle class="o" cx="0" cy="3.4" r="2" fill="#ffb340" stroke-width="1.2"/>
  </g></g>
  <g class="fx-think">
    <circle class="o d1" cx="-6" cy="0" r="1.9" fill="#e8ebff" stroke-width="1"/><circle class="o d2" cx="0" cy="0" r="1.9" fill="#e8ebff" stroke-width="1"/><circle class="o d3" cx="6" cy="0" r="1.9" fill="#e8ebff" stroke-width="1"/>
  </g>
</g>
<g class="fx-orbit">
  <circle class="o" cx="24" cy="1" r="1.9" fill="#b9aef5" stroke-width="1"/><circle class="o" cx="43" cy="34" r="1.9" fill="#b9aef5" stroke-width="1"/><circle class="o" cx="5" cy="34" r="1.9" fill="#b9aef5" stroke-width="1"/>
</g>
<g class="fx-file"><g class="paper">
  <path class="o" d="M-5 -6.5h7l4 4v10h-11z" fill="#f4f6fb" stroke-width="1.4"/><path d="M2 -6.5v4h4" fill="#cfd6e6" stroke="${OL}" stroke-width="1" stroke-linejoin="round"/>
  <path d="M-3 0.2h6M-3 2.8h6M-3 5.4h4" stroke="#7d8fb8" stroke-width="1" stroke-linecap="round"/>
</g></g>`;

  // ---- claude8: the 8-bit pet. 14x14 grid, 1 cell = 1 SVG unit; sprites are ASCII maps (letter -> colour; a value starting with "." is a CSS class so the tint can reach it).
  // Every pose is its own group; pets.css flips them with steps() only (no easing). Frames: spark dot/plus/x/ring, eyes open (3 pupil poses) / shut / happy / wide,
  // mouth smile / open / o / flat, arms down / up, feet on / off. Props (right side, mirrored for side=left): dots, "!", z, document 5x6, confetti, hearts. ----
  const rc = (x, y, w, h, f, st) => `<rect x="${x}" y="${y}" width="${w}" height="${h}" ${f[0] === '.' ? `class="${f.slice(1)}"` : `fill="${f}"`}${st ? ` style="${st}"` : ''}/>`;
  const cells = (rows, pal, x0 = 0, y0 = 0) => rows.map((r, y) => { let o = ''; for (let x = 0; x < r.length;) { const ch = r[x]; let n = 1; while (r[x + n] === ch) n++; if (ch !== '.') o += rc(x0 + x, y0 + y, n, 1, pal[ch]); x += n; } return o; }).join('');
  const g8 = (cls, inner) => `<g class="${cls}">${inner}</g>`;
  const EY = '#231611', MO = '#3a1d14', AR = '#c46a4c', HL = '.px-hl', SP = { x: '#ffd2a3' };
  const C8_BODY = ['...LLLLLLL....', '..LBBBBBBBB...', '..BBBBBBBBD...', '..BBBBBBBBD...', '..BBBBBBBBD...', '..BcBBBBBcD...', '...DDDDDDD....']; // rows y = 3..9
  const C8_DOC = ['EEEE.', 'EPPEE', 'ETTTE', 'EPPPE', 'ETTPE', 'EEEEE'];
  const C8_HEART = ['.x.x.', 'xxxxx', '.xxx.', '..x..'];
  const C8_CONF = [[4, 3, -6, -3, '#ffd34e'], [6, 3, 0, -6, '#7fe0a5'], [8, 3, 6, -3, '#ff7aa2'], [3, 6, -6, 3, '#8fb7ff'], [10, 6, 6, 3, '#ffd34e'], [6, 4, 3, -6, '.px-acc'], [5, 4, -3, -6, '#fff1e6'], [9, 4, 6, -6, '.px-acc']];
  const C8 = `<svg viewBox="0 0 14 14" overflow="visible" shape-rendering="crispEdges" aria-hidden="true"><g class="px-fig"><g transform="translate(0 2)">
${g8('px-lg', cells(['...FF...FF....'], { F: '#9c4a31' }, 0, 10))}${g8('px-feet', cells(['..FFF...FFF...'], { F: '#7d3a26' }, 0, 11))}
${g8('px-a-dn', rc(1, 7, 1, 2, AR) + rc(11, 7, 1, 2, AR))}${g8('px-a-up', rc(1, 5, 1, 2, AR) + rc(11, 5, 1, 2, AR))}
${cells(C8_BODY, { L: '#ea9371', B: '#d97757', D: '#b85d40', c: '.px-blush' }, 0, 3)}
${g8('px-m-smile', rc(5, 8, 1, 1, MO) + rc(7, 8, 1, 1, MO) + rc(6, 9, 1, 1, MO))}${g8('px-m-open', rc(5, 8, 3, 1, MO) + rc(6, 9, 1, 1, MO))}${g8('px-m-o', rc(6, 8, 1, 2, MO))}${g8('px-m-flat', rc(6, 8, 1, 1, MO))}
${g8('px-eyes', g8('px-e-open', rc(4, 6, 2, 2, EY) + rc(7, 6, 2, 2, EY) + g8('px-p-c', rc(4, 6, 1, 1, HL) + rc(7, 6, 1, 1, HL)) + g8('px-p-l', rc(4, 7, 1, 1, HL) + rc(7, 7, 1, 1, HL)) + g8('px-p-r', rc(5, 6, 1, 1, HL) + rc(8, 6, 1, 1, HL)))
  + g8('px-e-shut', rc(4, 7, 2, 1, EY) + rc(7, 7, 2, 1, EY))
  + g8('px-e-happy', rc(4, 7, 1, 1, EY) + rc(5, 6, 1, 1, EY) + rc(7, 6, 1, 1, EY) + rc(8, 7, 1, 1, EY))
  + g8('px-e-wide', rc(4, 5, 2, 3, EY) + rc(7, 5, 2, 3, EY) + rc(4, 5, 1, 1, HL) + rc(7, 5, 1, 1, HL)))}
${g8('px-spark', g8('px-s-dot', cells(['', '......x'], SP)) + g8('px-s-plus', cells(['......x', '.....xxx', '......x'], SP)) + g8('px-s-x', cells(['.....x.x', '......x', '.....x.x'], SP)) + g8('px-s-ring', cells(['.....xxx', '.....x.x', '.....xxx'], SP)))}
${g8('px-fx', g8('px-dots', g8('px-d1', rc(9, 1, 1, 1, '#e8ebff')) + g8('px-d2', rc(11, 1, 1, 1, '#e8ebff')) + g8('px-d3', rc(13, 1, 1, 1, '#e8ebff')))
  + g8('px-excl', rc(12, 1, 2, 4, '#ffb340') + rc(12, 6, 2, 2, '#ffb340'))
  + g8('px-zs', [1, 2, 3].map((i) => g8('px-z px-z' + i, `<g transform="translate(11 7)">${cells(['xxx', '.x.', 'xxx'], { x: '#e8ebff' })}</g>`)).join(''))
  + g8('px-docw', g8('px-doc', `<g transform="translate(10 6)">${cells(C8_DOC, { E: '.px-edge', P: '#f6efe4', T: '.px-tx' })}</g>`))
  + g8('px-conf', C8_CONF.map(([x, y, dx, dy, c]) => rc(x, y, 1, 1, c, `--dx:${dx}px;--dy:${dy}px`)).join(''))
  + g8('px-hearts', [[10, 5, 0], [-2, 5, 0.25], [-1, 1, 0.5]].map(([x, y, d]) => `<g class="px-h" style="animation-delay:${d}s"><g transform="translate(${x} ${y})">${cells(C8_HEART, { x: '#ff7aa2' })}</g></g>`).join('')))}
</g></g></svg>`;

  // ---- the pets (viewBox 0 0 48 48). Every pet lives INSIDE the island and stands / sits / floats on its floor (y = 47): cat sits, ghost floats over a shadow, bunny sits, robot stands, slime sits. ----
  const CAT = '#f0a35e';
  const PETS = {
    cat: {
      name: { ru: 'Кот', en: 'Cat' }, desc: { ru: 'Сидит, хвост и уши живые', en: 'Sits, lively tail and ears' },
      art: `
${ln('tail', 'M32 41C43 42 46 31 41 24', 3.8, CAT)}
<ellipse class="o" cx="24" cy="38.5" rx="9.5" ry="7.5" fill="${CAT}"/>
<ellipse cx="24" cy="40.5" rx="5.2" ry="4.4" fill="#fde7cf"/>
<ellipse class="o" cx="18.5" cy="45.5" rx="4" ry="2.3" fill="${CAT}"/><ellipse class="o" cx="29.5" cy="45.5" rx="4" ry="2.3" fill="${CAT}"/>
<g class="head">
  <g class="earL"><path class="o" d="M12.5 16L13.5 4.5L22.5 11Z" fill="${CAT}"/><path class="blush" style="--b:#f2a0a8" d="M15.2 12.6L15.6 8L19.4 10.8Z" fill="#f2a0a8"/></g>
  <g class="earR"><path class="o" d="M35.5 16L34.5 4.5L25.5 11Z" fill="${CAT}"/><path class="blush" style="--b:#f2a0a8" d="M32.8 12.6L32.4 8L28.6 10.8Z" fill="#f2a0a8"/></g>
  <ellipse class="o" cx="24" cy="21.5" rx="13" ry="11" fill="${CAT}"/>
  <path d="M24 11v3M20.4 11.6l.9 2.6M27.6 11.6l-.9 2.6" stroke="#cf7d3c" stroke-width="1.5" stroke-linecap="round" fill="none"/>
  ${eyes([[18.5, 21.5], [29.5, 21.5]], 'dot', OL, 2.1)}
  <path class="blush" style="--b:#e8808c" d="M22.8 25.3h2.4l-1.2 1.5Z" fill="#e8808c"/>
  <path d="M24 26.8q-1.8 2.2-3.8.7M24 26.8q1.8 2.2 3.8.7" fill="none" stroke="${OL}" stroke-width="1.1" stroke-linecap="round"/>
  <path d="M12.8 25.5l-4.3-1M12.8 27.4l-4 1.4M35.2 25.5l4.3-1M35.2 27.4l4 1.4" stroke="${OL}" stroke-width=".8" stroke-linecap="round" opacity=".45" fill="none"/>
</g>`,
    },
    ghost: {
      name: { ru: 'Призрак', en: 'Ghost' }, desc: { ru: 'Парит над полом, под ним тень', en: 'Floats above the floor with a tiny shadow' },
      art: `
<g class="armL"><ellipse class="o" cx="9.5" cy="29" rx="3" ry="4.6" fill="#e9ebff" transform="rotate(18 9.5 29)"/></g>
<g class="armR"><ellipse class="o" cx="38.5" cy="29" rx="3" ry="4.6" fill="#e9ebff" transform="rotate(-18 38.5 29)"/></g>
<path class="o" d="M10 45V23C10 12.5 16 7.5 24 7.5S38 12.5 38 23V45L33.5 41L29 45L24 41L19 45L14.5 41Z" fill="#e9ebff"/>
<path d="M15 17q2-5 7-6" stroke="#fff" stroke-width="2" opacity=".85" fill="none" stroke-linecap="round"/>
<g class="head">
  ${eyes([[18.5, 22.5], [29.5, 22.5]], 'tall', OL, 2)}
  <circle class="blush" style="--b:#ffb3c6" cx="14.6" cy="27.2" r="2.3" fill="#ffb3c6" opacity=".75"/><circle class="blush" style="--b:#ffb3c6" cx="33.4" cy="27.2" r="2.3" fill="#ffb3c6" opacity=".75"/>
  <ellipse class="mouth" cx="24" cy="29" rx="2" ry="2.4" fill="${OL}"/>
</g>`,
    },
    bunny: {
      name: { ru: 'Зайка', en: 'Bunny' }, desc: { ru: 'Сидит и прыгает', en: 'Sits and hops' },
      art: `
<circle class="o" cx="35.5" cy="43" r="3.4" fill="#fff"/>
<ellipse class="o" cx="24" cy="40" rx="10" ry="7.5" fill="#f6f1ea"/>
<ellipse cx="24" cy="42" rx="5" ry="4.5" fill="#e6dccf"/>
<ellipse class="o" cx="17.5" cy="46" rx="4.6" ry="2.3" fill="#f6f1ea"/><ellipse class="o" cx="30.5" cy="46" rx="4.6" ry="2.3" fill="#f6f1ea"/>
<g class="head">
  <g class="earL"><g transform="rotate(-8 17.5 10)"><ellipse class="o" cx="17.5" cy="10" rx="4.2" ry="9" fill="#f6f1ea"/><ellipse cx="17.5" cy="10.8" rx="2" ry="6" fill="#f4a8b8"/></g></g>
  <g class="earR"><g transform="rotate(8 30.5 10)"><ellipse class="o" cx="30.5" cy="10" rx="4.2" ry="9" fill="#f6f1ea"/><ellipse cx="30.5" cy="10.8" rx="2" ry="6" fill="#f4a8b8"/></g></g>
  <ellipse class="o" cx="24" cy="27.5" rx="12" ry="10.5" fill="#f6f1ea"/>
  ${eyes([[18.8, 27], [29.2, 27]], 'dot', OL, 2)}
  <circle class="blush" style="--b:#f4a8b8" cx="14.6" cy="31.4" r="2.2" fill="#f4a8b8" opacity=".6"/><circle class="blush" style="--b:#f4a8b8" cx="33.4" cy="31.4" r="2.2" fill="#f4a8b8" opacity=".6"/>
  <ellipse cx="24" cy="30.4" rx="1.7" ry="1.25" fill="#ee86a0"/>
  <path d="M24 31.6v1.3M24 32.9q-1.8 1.8-3.4.4M24 32.9q1.8 1.8 3.4.4" fill="none" stroke="${OL}" stroke-width="1.05" stroke-linecap="round"/>
</g>`,
    },
    robot: {
      name: { ru: 'Робот', en: 'Robot' }, desc: { ru: 'Стоит с антенной, глаза сканируют', en: 'Stands with its antenna, eyes scan' },
      art: `
<g class="ant">${ln('', 'M24 13V5.5', 1.6, '#8197ad', 2)}<circle class="o led" cx="24" cy="3.6" r="2.7" fill="#ff7a7a"/></g>
${ln('armL', 'M14.5 34.5L9.5 40.5', 3, '#a9bccf')}${ln('armR', 'M33.5 34.5L38.5 40.5', 3, '#a9bccf')}
<rect class="o" x="17.5" y="42.2" width="5" height="3.8" rx="1.6" fill="#a9bccf"/><rect class="o" x="25.5" y="42.2" width="5" height="3.8" rx="1.6" fill="#a9bccf"/>
<rect class="o" x="14.5" y="31" width="19" height="11.5" rx="4" fill="#8197ad"/>
<circle class="chest" cx="24" cy="36.8" r="2.1" fill="#f6c344" stroke="${OL}" stroke-width="1"/>
<g class="head">
  <circle class="o" cx="9.2" cy="22" r="2.4" fill="#8197ad"/><circle class="o" cx="38.8" cy="22" r="2.4" fill="#8197ad"/>
  <rect class="o" x="10" y="12.5" width="28" height="19" rx="6.5" fill="#a9bccf"/>
  <rect x="13.5" y="16.5" width="21" height="11.5" rx="4.5" fill="#15202b"/>
  ${eyes([[19.5, 22.2], [28.5, 22.2]], 'led', '#5eead4', 2, '#5eead4')}
</g>`,
    },
    slime: {
      name: { ru: 'Слайм', en: 'Slime' }, desc: { ru: 'Сидит и пружинит', en: 'Sits and squishes' },
      art: `
<path class="o" d="M24 8C15 8 9.5 19 9.5 30C9.5 40.5 16 45.5 24 45.5S38.5 40.5 38.5 30C38.5 19 33 8 24 8Z" fill="#7fdca6"/>
<path d="M13 34C14 41 19 43.5 24 43.5S34 41 35 34C31 38 17 38 13 34Z" fill="#5fc58c" opacity=".75"/>
<ellipse cx="16.5" cy="25" rx="2.2" ry="4" fill="#fff" opacity=".55" transform="rotate(18 16.5 25)"/>
<circle class="bub b1" cx="31.5" cy="38" r="1.6" fill="#c4f5d9"/><circle class="bub b2" cx="28.5" cy="41" r="1" fill="#c4f5d9"/>
<g class="head">
  ${eyes([[19.5, 30.5], [28.5, 30.5]], 'tall', OL, 2.1)}
  <circle class="blush" style="--b:#ffa6bd" cx="15.3" cy="34.6" r="2.1" fill="#ffa6bd" opacity=".55"/><circle class="blush" style="--b:#ffa6bd" cx="32.7" cy="34.6" r="2.1" fill="#ffa6bd" opacity=".55"/>
  <path d="M21.6 35.4q2.4 2.7 4.8 0" fill="none" stroke="${OL}" stroke-width="1.2" stroke-linecap="round"/>
</g>`,
    },
    claude8: {
      name: { ru: 'Клод 8-бит', en: 'Claude 8-bit' }, desc: { ru: 'Пиксельный Клод стоит на ножках, кадры шагами, как в 8-битной игре', en: 'Pixel Claude stands on its legs, stepped frames like an 8-bit game' },
      full: C8,
    },
  };
  const IDS = Object.keys(PETS);
  const GROUPS = [{ id: 'characters', name: { ru: 'Персонажи', en: 'Characters' } }, { id: 'creatures', name: { ru: 'Существа', en: 'Creatures' } }, { id: 'things', name: { ru: 'Вещи', en: 'Things' } }];
  IDS.forEach((k) => Object.assign(PETS[k], { group: 'characters', w: 48, h: 48, ar: 1 }));
  const AUTH = { OL, ln, eyes, rc, cells, g8, STATES, TINT }; // builders handed to external pets
  const svgOf = (id) => {
    const p = PETS[id];
    if (p.full) return p.full; // claude8: a complete <svg> of its own
    const inner = p.ext ? (typeof p.svg === 'function' ? p.svg(Object.assign({ id, w: p.w, h: p.h }, AUTH)) : p.svg) : p.art;
    if (p.w === 48 && p.h === 48) return `<svg viewBox="0 0 48 48" overflow="visible" aria-hidden="true"><g class="fig">${inner}</g>${p.fx === false ? '' : FX}</svg>`;
    // other sizes: the shared props live in a square h x h box (a nested svg, 48 units wide) on the free side; --fxx = how far a left-end pet moves it
    return `<svg viewBox="0 0 ${p.w} ${p.h}" overflow="visible" aria-hidden="true"><g class="fig">${inner}</g>${p.fx === false ? '' : `<g class="fx-host" style="--fxx:${p.w - p.h}px"><svg width="${p.h}" height="${p.h}" viewBox="0 0 48 48" overflow="visible">${FX}</svg></g>`}</svg>`;
  };
  // PillPets.add(def): see the EXTENSION API in the header. Returns true when registered.
  const cssDone = new Set(), addL = [];
  function add(def) {
    if (!def || typeof def.id !== 'string' || !/^[a-z0-9-]+$/.test(def.id) || PETS[def.id] || !def.svg) { console.warn('PillPets.add: rejected', def && def.id); return false; }
    const w = +def.w > 0 ? +def.w : 48, h = +def.h > 0 ? +def.h : 48;
    PETS[def.id] = { ext: true, group: GROUPS.some((g) => g.id === def.group) ? def.group : 'characters', w, h, ar: Math.min(2.5, Math.max(0.5, w / h)), svg: def.svg, fx: def.fx,
      name: def.name || { ru: def.id, en: def.id }, desc: def.desc || { ru: '', en: '' }, hooks: def };
    IDS.push(def.id);
    if (def.css && !cssDone.has(def.id)) { cssDone.add(def.id); const st = document.createElement('style'); st.dataset.petCss = def.id; st.textContent = def.css; (document.head || document.documentElement).append(st); }
    addL.forEach((f) => f(def.id));
    return true;
  }

  // ---- idle behaviour: occasional blink, look-around and a pet-specific twitch; stops while hidden / paused ----
  const TWITCH = { cat: ['twitch'], ghost: ['twitch'], bunny: ['twitch', 'hop'], robot: ['twitch'], slime: ['twitch'], claude8: ['twitch'] };
  const mqMotion = window.matchMedia ? matchMedia('(prefers-reduced-motion: reduce)') : { matches: false };
  const reduced = () => (window.Pill && window.Pill.reduced ? !!window.Pill.reduced() : mqMotion.matches);
  const live = new Set();
  document.addEventListener('visibilitychange', () => live.forEach((i) => i.sync()));

  const safeHook = (fn, ...a) => { try { return fn(...a); } catch (e) { console.error('pet hook', e); } }; // a broken external pet never takes the island down
  function create(id, o = {}) {
    if (!PETS[id]) id = 'cat';
    const el = document.createElement('div');
    el.className = 'pet';
    el.dataset.pet = id;
    el.dataset.side = o.side === 'left' ? 'left' : 'right';
    if (o.size) el.dataset.size = o.size;
    const def = PETS[id], hk = def.hooks || {};
    if (def.ar !== 1) el.style.setProperty('--ar', def.ar.toFixed(3));
    el.innerHTML = `<div class="pet-rig">${svgOf(id)}</div>`;
    const rig = el.firstChild;
    let state = 'calm', timer = 0, actT = 0, lookT = 0, held = false, aimed = false, lastPet = 0, petT = 0, alive = true;
    const active = () => alive && !held && !document.hidden;
    let tickOn = false;
    const tickLoop = (dt, now) => { if (!tickOn || !active() || reduced()) { tickOn = false; return false; } safeHook(hk.tick, el, now); return true; };
    function startTick() { // hooks.tick(host, tMs): the shared Pill.raf loop (own rAF when there is no Pill), only while visible and not reduced
      if (!hk.tick || tickOn || !active() || reduced()) return;
      tickOn = true;
      if (window.Pill && window.Pill.raf) window.Pill.raf(tickLoop);
      else { const f = (n) => { if (tickLoop(0, n)) requestAnimationFrame(f); }; requestAnimationFrame(f); }
    }
    const act = (name, ms) => {
      clearTimeout(actT);
      el.classList.remove('act-twitch', 'act-hop', 'act-blink');
      void el.offsetWidth; // restart the CSS animation
      el.classList.add('act-' + name);
      actT = setTimeout(() => el.classList.remove('act-' + name), ms);
    };
    const setLook = (x, y) => { // --lx / --ly drive the smooth pets; the 8-bit one only has three pupil poses (data-look l / r, none = centre)
      el.style.setProperty('--lx', x); el.style.setProperty('--ly', y);
      if (id === 'claude8') { if (+x < -0.35) el.dataset.look = 'l'; else if (+x > 0.35) el.dataset.look = 'r'; else delete el.dataset.look; }
      if (hk.look) safeHook(hk.look, el, +x, +y);
    };
    function glance(x, y, ms) { // idle look-around
      if (aimed) return;
      setLook(x, y);
      clearTimeout(lookT);
      lookT = setTimeout(() => { if (!aimed) setLook(0, 0); }, ms);
    }
    function tick() {
      timer = 0;
      if (!active()) return; // sync() restarts the chain
      if (!reduced() && o.idle !== false) {
        const r = Math.random(), calmish = state === 'calm' || state === 'done';
        if (state === 'sleep') { /* asleep: nothing */ }
        else if (r < 0.6 || !calmish) act('blink', 240);
        else if (r < 0.8) glance(Math.random() < 0.5 ? -1 : 1, Math.random() < 0.5 ? 0 : -0.5, 1300);
        else { const list = TWITCH[id] || ['twitch']; const a = list[Math.floor(Math.random() * list.length)]; act(a, a === 'hop' ? 700 : 650); }
      }
      timer = setTimeout(tick, 2400 + Math.random() * 4600);
    }
    const inst = {
      el, id,
      get state() { return state; },
      setState(s, color) { // color = the reaction's own colour; empty -> the island-edge colour of that state (calm: none)
        if (!STATES.includes(s)) s = 'calm';
        el.removeAttribute('data-state'); void el.offsetWidth; // a fresh event of the same state restarts its one-shot animation
        el.dataset.state = state = s;
        inst.tint(color || TINT[s]);
        if (hk.state) safeHook(hk.state, el, s, color || TINT[s]);
        if (s !== 'music') inst.setEnergy(null);
        if (s === 'sleep') inst.look(null);
      },
      tint(color) { // paint the accents (cheeks, props, halo) in this colour; falsy = fade the tint out, the accent colour is kept so it fades instead of flipping
        if (color) { el.style.setProperty('--pet-accent', color); el.style.setProperty('--pet-k', 1); } else el.style.setProperty('--pet-k', 0);
      },
      look(nx, ny) { // -1..1 each; null = look straight again
        if (nx == null) { aimed = false; setLook(0, 0); return; }
        if (state === 'sleep') return;
        aimed = true; clearTimeout(lookT);
        setLook((+nx).toFixed(2), (+ny || 0).toFixed(2));
      },
      pet() { // hearts + a tiny bounce, at most once a second
        const n = performance.now();
        if (n - lastPet < 1000) return false;
        lastPet = n;
        el.classList.remove('act-pet'); void el.offsetWidth; el.classList.add('act-pet');
        el.classList.add('poked');
        clearTimeout(petT); petT = setTimeout(() => el.classList.remove('act-pet', 'poked'), 1400);
        if (hk.poke) safeHook(hk.poke, el);
        return true;
      },
      setEnergy(e) { // music: 0..1 loudness drives the bob instead of the steady CSS bounce; null = back to the CSS bounce
        if (e == null) { if (el.dataset.energy) { delete el.dataset.energy; rig.style.transform = ''; if (hk.energy) safeHook(hk.energy, el, null); } return; }
        el.dataset.energy = '1';
        if (hk.energy) { safeHook(hk.energy, el, +e); return; } // the pet animates itself
        rig.style.transform = id === 'claude8' ? `translateY(calc(var(--pp) / -14 * ${Math.round(e * 2)}))` // 8-bit: whole cells only (0 / 1 / 2 up)
          : `translateY(${(e * 7).toFixed(2)}%) scale(${(1 - e * 0.04).toFixed(3)},${(1 + e * 0.09).toFixed(3)})`;
      },
      setHidden(b) { held = !!b; inst.sync(); },
      sync() { clearTimeout(timer); timer = 0; if (active()) { timer = setTimeout(tick, 1200 + Math.random() * 2500); startTick(); } },
      destroy() { alive = false; tickOn = false; clearTimeout(timer); clearTimeout(actT); clearTimeout(lookT); clearTimeout(petT); live.delete(inst); el.remove(); },
    };
    el.dataset.state = o.state && STATES.includes(o.state) ? o.state : 'calm';
    state = el.dataset.state;
    inst.tint(o.color || TINT[state]);
    if (hk.init) safeHook(hk.init, el, { id, w: def.w, h: def.h, host: el, svg: rig.querySelector('svg'), $: (q) => el.querySelector(q), $$: (q) => [...el.querySelectorAll(q)], reduced });
    if (hk.state) safeHook(hk.state, el, state, o.color || TINT[state]);
    live.add(inst);
    inst.sync();
    return inst;
  }

  const registry = { ids: IDS, groups: GROUPS, states: STATES, tint: TINT, registry: PETS, add, onAdd: (f) => addL.push(f), create, art: svgOf, name: (id, t) => (PETS[id] ? (t ? t(PETS[id].name) : PETS[id].name.en) : id) };
  window.PillPets = registry;
  if (!(window.Pill && window.Pill.onReaction)) return; // settings window / static preview: registry only
  window.Pill.pets = registry;

  // ---- island hook: one pet INSIDE the island's black body, driven by settings (pet, pet.side, pet.size) and Pill.onReaction.
  // Collapsed: a child of #clip (the body's own overflow-hidden, rounded box) standing on the capsule floor at its right / left end; Pill.reserve() widens the capsule
  // (animated like any content resize) and pets.css pads #live on that side, so the pet never overlaps the clock. Expanded: it moves into #pet-slot, the free gap of the
  // header row between the tabs and the header buttons (on the chosen side of the gap), and fades in after the panel. Nothing of it can leave the silhouette: #clip clips it. ----
  const P = window.Pill, body = document.getElementById('body'), clip = document.getElementById('clip'), head = document.getElementById('head'), hdr = document.getElementById('hdr');
  if (!body || !clip || !head || !hdr) return;
  const root = document.documentElement;
  const slot = document.createElement('div'); slot.id = 'pet-slot'; head.insertBefore(slot, hdr);
  const CAP = { s: 20, m: 27, l: 30 }, CAP8 = { s: 14, m: 28, l: 28 }; // css px of the pet box in the 30 px capsule at 100 %; keep in step with pets.css
  let inst = null, cfg = { id: '', side: 'right', size: 'm' }, cur = 'calm', curCol = '', lastBands = 0, energy = 0, peak = 0.2, lastBandT = 0, sentE = -1;

  function mount(fade) { // put the pet where the island is now: capsule floor (collapsed) or the header slot (open)
    if (!inst) return;
    const w = root.classList.contains('expanded') ? 'head' : 'cap', host = w === 'head' ? slot : clip;
    if (inst.el.parentNode === host && inst.el.dataset.where === w) return;
    inst.el.dataset.where = w;
    host.append(inst.el);
    if (fade && !P.reduced()) { // appear after the panel (open) / after the capsule has folded (collapsed)
      inst.el.style.setProperty('--enter-delay', w === 'head' ? '0.12s' : '0.15s');
      inst.el.classList.remove('enter'); void inst.el.offsetWidth; inst.el.classList.add('enter');
    }
  }
  function reserve() { // tell the capsule how much room the pet needs (the same number pads #live in pets.css)
    if (!inst) { root.style.removeProperty('--pet-res'); delete root.dataset.petSide; if (P.reserve) P.reserve(0); return; }
    const s = Math.min(2, Math.max(0.8, parseFloat(getComputedStyle(root).getPropertyValue('--s')) || 1));
    let n = (cfg.id === 'claude8' ? CAP8 : CAP)[cfg.size];
    if (cfg.id === 'claude8') n = Math.max(14, Math.floor(n * s / 14 + 1e-6) * 14) / s; // whole cells, as in the CSS
    const px = Math.ceil(n * PETS[cfg.id].ar + 12); // box (height x aspect ratio, wide pets up to 2.5:1) + 9 px inset from the end + 3 px gap
    root.style.setProperty('--pet-res', px + 'px'); root.dataset.petSide = cfg.side;
    if (P.reserve) P.reserve(px);
  }
  new MutationObserver(() => mount(true)).observe(root, { attributes: true, attributeFilter: ['class'] }); // expand / collapse

  let lastS = {}, started = false;
  function apply(s) {
    s = lastS = Object.assign({}, lastS, s || {});
    // 'none' and any id that is not registered (yet) mean no pet; a pet added later by another script is picked up by onAdd below
    const id = s.pet === 'none' || (typeof s.pet === 'string' && !PETS[s.pet]) ? '' : typeof s.pet === 'string' ? s.pet : cfg.id;
    const side = s['pet.side'] === 'left' || s['pet.side'] === 'right' ? s['pet.side'] : cfg.side;
    const size = ['s', 'm', 'l'].includes(s['pet.size']) ? s['pet.size'] : cfg.size;
    if (id !== cfg.id) {
      if (inst) { inst.destroy(); inst = null; }
      if (id) {
        inst = create(id, { state: cur, side, size, color: curCol });
        inst.el.id = 'pet';
        const hit = document.createElement('div'); // the only clickable part of the pet; the rest of it never takes pointer events
        hit.className = 'pet-hit';
        hit.addEventListener('click', () => inst && inst.pet());
        inst.el.append(hit);
        mount(false);
        if (cur === 'music' && !P.reduced()) P.raf(musicLoop);
      }
    }
    cfg = { id, side, size };
    if (inst) { inst.el.dataset.side = side; inst.el.dataset.size = size; }
    gazeSync();
    reserve();
    setTimeout(reserve, 80); // --s of a changed island size lands a moment after settings:changed
  }

  // ---- look at the cursor across the WHOLE screen. Rust `pill:gaze` {x, y} = cursor in PHYSICAL px relative to the window (any distance / monitor, only while it moves
  // and only after pill_set_gaze(true)); css px = physical / devicePixelRatio. The feed is shared with characters.js through window.PillGaze (refcounted on/off).
  // Fallback while the feed is not there: the pointer inside the window (pill:cursor / mousemove). ----
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
  const AIMK = 60; // css px: look = dir * |d| / (|d| + AIMK), so the eyes keep following towards the far screen corners and saturate smoothly
  let aim = null, ac = null, acT = 0, gx = 0, gy = 0;
  const aimLoop = (dt, now) => { // dt-filtered approach to the target; stops when settled (events wake it again)
    if (!inst) return false;
    const src = GZ.avail ? (GZ.ok ? GZ : null) : aim, live = !!src && cur !== 'sleep' && !P.reduced();
    let tx = 0, ty = 0;
    if (live) {
      if (!ac || now - acT > 200) { const r = inst.el.getBoundingClientRect(); ac = { x: r.left + r.width / 2, y: r.top + r.height / 2 }; acT = now; } // cached: not per event
      const dx = src.x - ac.x, dy = src.y - ac.y, n = 1 / (Math.hypot(dx, dy) + AIMK);
      tx = dx * n; ty = dy * n;
    }
    const k = 1 - Math.exp(-dt * 14);
    gx += (tx - gx) * k; gy += (ty - gy) * k;
    const done = Math.abs(tx - gx) < 0.01 && Math.abs(ty - gy) < 0.01;
    if (done) { gx = tx; gy = ty; }
    if (done && !live) inst.look(null); else inst.look(gx, gy);
    return !done;
  };
  const gazeSync = () => { // the Rust feed runs only while a pet is shown awake (asleep: eyes stay shut, no gaze)
    const w = !!inst && cur !== 'sleep' && !P.reduced();
    GZ.use('pet', w);
    if (inst) { ac = null; P.raf(aimLoop); }
  };
  GZ.subs.add(() => { if (inst) P.raf(aimLoop); });

  // reaction -> pose; music additionally follows the audio energy (audio:bands, 32 bands low -> high) while it is fresh
  function musicLoop(dt, now) {
    if (!inst || cur !== 'music' || P.reduced()) { if (inst) inst.setEnergy(null); return false; }
    if (performance.now() - lastBands > 900) { inst.setEnergy(null); sentE = -1; }
    else {
      energy += (targetE - energy) * (1 - Math.exp(-dt * 15.5)); // tau ~64 ms = the old 0.4 per 33 ms step, whatever the frame rate
      if (Math.abs(energy - sentE) > 0.002 || sentE < 0) { sentE = energy; inst.setEnergy(energy); } // no restyle for an unchanged value
    }
    return true;
  }
  let targetE = 0;
  P.on('audio:bands', (p) => {
    if (!inst || cur !== 'music' || !p || !Array.isArray(p.b)) return;
    let e = 0;
    for (let k = 0; k < 8; k++) e += +p.b[k] || 0;
    const n = performance.now(), dtMs = lastBandT ? Math.min(200, Math.max(1, n - lastBandT)) : 33;
    lastBandT = n;
    e /= 8; peak = Math.max(e, peak * Math.pow(0.995, dtMs / 33), 0.05); // per 33 ms (the old 30 Hz feed), independent of the event rate
    targetE = Math.min(1, e / peak);
    lastBands = performance.now();
  });
  P.onReaction((name, color) => { // color = the colour the winning source asked for ('' = none: the pet uses the edge palette of that reaction)
    cur = name; curCol = color || '';
    sentE = -1;
    if (inst) inst.setState(name, curCol);
    if (name === 'music' && inst && !P.reduced()) P.raf(musicLoop);
    gazeSync();
  });

  // fallback pointer (inside the window only): pill:cursor ({x, y} css px, near the island) and plain mousemove
  const aimAt = (x, y) => {
    aim = x == null || y == null ? null : { x, y };
    if (!GZ.avail && inst) P.raf(aimLoop);
  };
  P.on('pill:cursor', (p) => aimAt(p && p.x, p && p.y));
  document.addEventListener('mousemove', (e) => aimAt(e.clientX, e.clientY), { passive: true });
  document.documentElement.addEventListener('mouseleave', () => aimAt(null));
  P.on('pill:leave', () => aimAt(null));
  P.on('pill:hide', () => inst && inst.setHidden(true));
  P.on('pill:show', () => inst && inst.setHidden(false));
  P.on('settings:changed', apply);

  registry.onAdd(() => { if (started) apply({}); });
  // start: wait for the stored settings so a user who picked "none" never sees a flash of the default cat; and for window load, so that the
  // external pet scripts (pets-chars.js ...) that follow pets.js have registered their ids before the saved one is looked up
  const start = async () => {
    const s = await P.invoke('settings_get');
    started = true;
    apply(s || { pet: 'cat' });
  };
  if (document.readyState === 'complete') start(); else window.addEventListener('load', start);
})();
