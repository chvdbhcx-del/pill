// Pets, characters set 2 (group 'characters'): kitty, drop, jelly, sloth, astronaut, sprout. Our own interpretations of the character gallery of an Island-style app.
// Loaded after src/pets.js. All art is original inline SVG (viewBox 48 x 48, floor y = 47); every state has its own look; CSS animates transform / opacity only.
// Own animations live on inner groups (not on .pet-rig), so music `energy` and the shared poke squeeze keep working. Reduced motion / pause are handled by pets.css.
(() => {
  'use strict';
  const P = window.PillPets;
  if (!P || !P.add) return;
  const q = (id) => `.pet[data-pet="${id}"]`;
  // switch the shared rig animation (breath / dance / jump / sleep shrink ...) off for pets that animate inner groups; the poke squeeze (.act-pet) stays
  const rigOff = (id) => `${q(id)}:not(.act-pet) .pet-rig, ${q(id)}[data-state]:not(.act-pet) .pet-rig { animation: none; }`;
  const st = (el, k, v) => { if (el) el.style[k] = v; };
  const WH = '#f7f4ef';

  // ============================== 1. kitty: front-facing white cat face with a collar bell ==============================
  P.add({
    id: 'kitty', group: 'characters', w: 48, h: 48,
    name: { ru: 'Котик', en: 'Kitty' }, desc: { ru: 'Мордочка с ушами и усами, колокольчик меняет цвет', en: 'A muzzle with ears and whiskers; the bell changes colour' },
    svg: (a) => `
<g class="head">
  <g class="earL"><path class="o" d="M9.5 23L10.5 5.5L23 14.5Z" fill="${WH}"/><path d="M12.8 18L13.4 10.5L19.4 14.6Z" fill="#f4a9b8"/></g>
  <g class="earR"><path class="o" d="M38.5 23L37.5 5.5L25 14.5Z" fill="${WH}"/><path d="M35.2 18L34.6 10.5L28.6 14.6Z" fill="#f4a9b8"/></g>
  <path d="M14.5 39.2Q24 47.2 33.5 39.2" fill="none" stroke="${a.OL}" stroke-width="4.6" stroke-linecap="round"/>
  <path class="kcol" d="M14.5 39.2Q24 47.2 33.5 39.2" fill="none" stroke-width="2.7" stroke-linecap="round"/>
  <g class="whL"><path d="M10 31L1.5 29M10 33.5L1.2 34M10.5 36L2.5 39" fill="none" stroke="#ece7dc" stroke-width="1.1" stroke-linecap="round"/></g>
  <g class="whR"><path d="M38 31L46.5 29M38 33.5L46.8 34M37.5 36L45.5 39" fill="none" stroke="#ece7dc" stroke-width="1.1" stroke-linecap="round"/></g>
  <ellipse class="o" cx="24" cy="27" rx="17" ry="14" fill="${WH}"/>
  <path d="M24 14.6v3.4M20.4 15.3l.8 3M27.6 15.3l-.8 3" stroke="#d6cfc2" stroke-width="1.3" stroke-linecap="round" fill="none"/>
  ${a.eyes([[17.5, 26.5], [30.5, 26.5]], 'dot', a.OL, 2.3)}
  <circle class="blush" style="--b:#f4aab5" cx="11.6" cy="31.6" r="2.5" fill="#f4aab5" opacity=".85"/><circle class="blush" style="--b:#f4aab5" cx="36.4" cy="31.6" r="2.5" fill="#f4aab5" opacity=".85"/>
  <path d="M22 30.8h4l-2 2.5Z" fill="#ee8ba0"/>
  <path d="M24 33.3v1.3M24 34.6q-2.2 2.2-4.1.4M24 34.6q2.2 2.2 4.1.4" fill="none" stroke="${a.OL}" stroke-width="1.1" stroke-linecap="round"/>
  <g class="kbell"><circle class="o kb" cx="24" cy="44" r="3.1" style="stroke-width:1.3"/><path d="M21.2 44.7h5.6" stroke="${a.OL}" stroke-width=".9"/><circle cx="24" cy="46" r=".7" fill="${a.OL}"/><circle cx="22.9" cy="42.7" r=".8" fill="#fff" opacity=".8"/></g>
</g>`,
    css: `
${q('kitty')} .kb { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #f2b84b); }
${q('kitty')} .kcol { stroke: color-mix(in srgb, var(--pet-accent) var(--t-hi), #e0505e); }
${q('kitty')} .earL { --k: 1; } ${q('kitty')} .earR { --k: -1; }
${q('kitty')} .whL { transform-box: fill-box; transform-origin: 100% 50%; } ${q('kitty')} .whR { transform-box: fill-box; transform-origin: 0 50%; }
${q('kitty')} .kbell { transform-box: fill-box; transform-origin: 50% 0; }
@keyframes c2-wh { from { transform: rotate(-9deg); } to { transform: rotate(9deg); } }
@keyframes c2-ear { 0%, 100% { transform: rotate(0); } 40% { transform: rotate(calc(var(--k) * -13deg)); } 70% { transform: rotate(calc(var(--k) * 5deg)); } }
@keyframes c2-bell { from { transform: rotate(-16deg); } to { transform: rotate(16deg); } }
${q('kitty')}[data-state="music"] .earL, ${q('kitty')}[data-state="music"] .earR { animation: c2-ear 0.6s ease-in-out infinite; }
${q('kitty')}[data-state="music"] .earR { animation-delay: 0.3s; }
${q('kitty')}[data-state="music"] .whL { animation: c2-wh 0.2s ease-in-out infinite alternate; }
${q('kitty')}[data-state="music"] .whR { animation: c2-wh 0.2s ease-in-out infinite alternate-reverse; }
${q('kitty')}[data-state="music"] .kbell { animation: c2-bell 0.5s ease-in-out infinite alternate; }
${q('kitty')}.act-twitch .earL { animation: c2-ear 0.6s ease-in-out; }
${q('kitty')}.act-twitch .whL, ${q('kitty')}.act-twitch .whR { animation: c2-wh 0.12s ease-in-out 4 alternate; }
${q('kitty')}[data-state="thinking"] .earR { transform: rotate(-10deg); }
${q('kitty')}[data-state="thinking"] .eyes { transform: translate(2px, -1.6px); }
${q('kitty')}[data-state="done"] .kbell { animation: c2-bell 0.4s ease-in-out 3 alternate; }
${q('kitty')}[data-state="attention"] .earL, ${q('kitty')}[data-state="attention"] .earR { transform: translateY(-1.2px) scaleY(1.16); }
${q('kitty')}[data-state="attention"] .eye { transform: scale(1.28); }
${q('kitty')}[data-state="attention"] .whL, ${q('kitty')}[data-state="attention"] .whR { animation: c2-wh 0.09s linear infinite alternate; }
${q('kitty')}[data-state="file"] .eyes { transform: translate(0, 1.5px); }
${q('kitty')}[data-state="sleep"] .earL, ${q('kitty')}[data-state="sleep"] .earR { transform: rotate(calc(var(--k) * -16deg)) scaleY(0.92); }
${q('kitty')}[data-state="sleep"] .whL { transform: rotate(14deg); } ${q('kitty')}[data-state="sleep"] .whR { transform: rotate(-14deg); }
${q('kitty')}[data-state="sleep"] .kbell { transform: rotate(8deg); }
${q('kitty')}[data-state="sleep"] .kb { opacity: .7; }`,
    init(host) { host._kb = host.querySelector('.kbell'); host._kf = host.querySelector('.head'); },
    energy(host, e) {
      st(host._kb, 'scale', e == null ? '' : (1 + e * 0.45).toFixed(2));
      st(host._kf, 'translate', e == null ? '' : `0 ${(-e * 1.8).toFixed(1)}px`);
    },
  });

  // ============================== 2. drop: soft jelly blob, body colour follows the reaction ==============================
  P.add({
    id: 'drop', group: 'characters', w: 48, h: 48,
    name: { ru: 'Капля', en: 'Drop' }, desc: { ru: 'Мягкое желе с румянцем, колышется под музыку', en: 'Soft jelly with blush cheeks; wobbles to the music' },
    svg: (a) => `
<ellipse class="dpud" cx="24" cy="45.8" rx="18" ry="2.6"/>
<g class="dbody">
  <path class="o dfill" d="M24 13C13 13 6 21.5 6 31C6 40.5 12 46 24 46S42 40.5 42 31C42 21.5 35 13 24 13Z"/>
  <path class="dsh" d="M9 36C11 43 16 44.4 24 44.4S37 43 39 36C33 40.5 15 40.5 9 36Z"/>
  <ellipse cx="15.2" cy="23.5" rx="2.7" ry="4.6" fill="#fff" opacity=".55" transform="rotate(26 15.2 23.5)"/><circle cx="20.5" cy="17.8" r="1.2" fill="#fff" opacity=".55"/>
  ${a.eyes([[18, 30], [30, 30]], 'tall', a.OL, 2.2)}
  <ellipse cx="12.4" cy="34.8" rx="3" ry="2.1" fill="#ff7fa6" opacity=".8"/><ellipse cx="35.6" cy="34.8" rx="3" ry="2.1" fill="#ff7fa6" opacity=".8"/>
  <path d="M21.4 35.2q2.6 3 5.2 0" fill="none" stroke="${a.OL}" stroke-width="1.4" stroke-linecap="round"/>
</g>`,
    css: `${rigOff('drop')}
${q('drop')} { --dc: color-mix(in srgb, var(--pet-accent) var(--t-hi), #f48db4); }
${q('drop')} .dfill { fill: var(--dc); } ${q('drop')} .dsh { fill: color-mix(in srgb, var(--dc) 78%, #3a0d20); opacity: .5; }
${q('drop')} .dpud { fill: var(--dc); opacity: 0; transition: opacity .5s; }
${q('drop')} .dbody { transform-box: fill-box; transform-origin: 50% 100%; animation: c2-dr-idle 3.2s ease-in-out infinite; }
@keyframes c2-dr-idle { 0%, 100% { transform: scale(1.03, 0.96); } 50% { transform: scale(0.97, 1.05); } }
@keyframes c2-dr-dance { 0%, 100% { transform: scale(1.13, 0.84) rotate(-3deg); } 50% { transform: scale(0.88, 1.16) rotate(3deg); } }
@keyframes c2-dr-jump { 0% { transform: scale(1.15, 0.8); } 35% { transform: translateY(-9px) scale(0.9, 1.18); } 65% { transform: translateY(0) scale(1.1, 0.88); } 100% { transform: scale(1, 1); } }
@keyframes c2-dr-trem { 0%, 100% { transform: scale(0.93, 1.12) translateX(-.6px); } 50% { transform: scale(0.93, 1.12) translateX(.6px); } }
@keyframes c2-dr-poke { 0% { transform: scale(1.2, 0.7); } 30% { transform: scale(0.85, 1.22); } 55% { transform: scale(1.1, 0.9); } 80% { transform: scale(0.97, 1.04); } 100% { transform: scale(1, 1); } }
${q('drop')}[data-state="music"] .dbody { animation: c2-dr-dance 0.55s ease-in-out infinite; }
${q('drop')}[data-state="thinking"] .dbody { animation: c2-dr-idle 4.8s ease-in-out infinite; transform-origin: 40% 100%; }
${q('drop')}[data-state="thinking"] .eyes { transform: translate(2px, -1.8px); }
${q('drop')}[data-state="done"] .dbody { animation: c2-dr-jump 0.8s cubic-bezier(.3,.7,.4,1) 1, c2-dr-idle 3.2s ease-in-out 0.8s infinite; }
${q('drop')}[data-state="attention"] .dbody { animation: c2-dr-trem 0.16s linear infinite; }
${q('drop')}[data-state="file"] .dbody { animation: c2-dr-idle 2.2s ease-in-out infinite; }
${q('drop')}[data-state="file"] .eyes { transform: translate(0, 1.6px); }
${q('drop')}[data-state="sleep"] .dbody { animation: none; transform: scale(1.22, 0.55); transition: transform 1.2s ease-out; }
${q('drop')}[data-state="sleep"] .dpud { opacity: .5; }
${q('drop')}.poked .dbody { animation: c2-dr-poke 0.6s ease-out 1; }
${q('drop')}[data-energy] .dbody { animation: none !important; }`,
    init(host) { host._db = host.querySelector('.dbody'); },
    energy(host, e) { // squash on the beat: loud = tall and thin, quiet = wide and flat
      st(host._db, 'scale', e == null ? '' : `${(1.1 - e * 0.22).toFixed(3)} ${(0.88 + e * 0.3).toFixed(3)}`);
    },
  });

  // ============================== 3. jelly: glowing jellyfish ==============================
  const tent = (x, dx, end, cls) => `<path class="jt ${cls}" d="M${x} 25C${x - 3 + dx} 29 ${x + 3 + dx} 33 ${x} 37S${x - 2} ${end - 4} ${x + 1} ${end}" fill="none" stroke-width="2.1" stroke-linecap="round"/>`;
  P.add({
    id: 'jelly', group: 'characters', w: 48, h: 48,
    name: { ru: 'Медуза', en: 'Jellyfish' }, desc: { ru: 'Светится в тёмной воде, пульсирует, щупальца качаются', en: 'Glows in dark water, pulses, tentacles sway' },
    svg: (a) => `
<g class="jmv">
  <g class="jbub"><circle class="jb1" cx="10" cy="30" r="1.5"/><circle class="jb2" cx="38.5" cy="26" r="1.2"/><circle class="jb3" cx="33" cy="41" r="1.1"/></g>
  <g class="jtents">${tent(14, 0, 42, 't1')}${tent(19, 1, 46, 't2')}${tent(24, 0, 47, 't3')}${tent(29, -1, 46, 't4')}${tent(34, 0, 42, 't5')}</g>
  <g class="jbell">
    <path class="jfill" d="M8.5 25.5C8.5 13 15 6 24 6S39.5 13 39.5 25.5Q35.5 28 31.8 25.6Q28 28.2 24 25.6Q20 28.2 16.2 25.6Q12.5 28 8.5 25.5Z"/>
    <ellipse cx="24" cy="12.8" rx="9.5" ry="5" fill="#fff" opacity=".28"/><circle cx="14.5" cy="14.6" r="1.4" fill="#fff" opacity=".4"/><circle cx="33.5" cy="14.6" r="1.4" fill="#fff" opacity=".4"/>
    ${a.eyes([[19.5, 19.2], [28.5, 19.2]], 'dot', a.OL, 1.7)}
    <circle class="blush" style="--b:#ff9ec0" cx="15.6" cy="22.4" r="1.9" fill="#ff9ec0" opacity=".7"/><circle class="blush" style="--b:#ff9ec0" cx="32.4" cy="22.4" r="1.9" fill="#ff9ec0" opacity=".7"/>
    <path d="M22.6 22.4q1.4 1.5 2.8 0" fill="none" stroke="${a.OL}" stroke-width="1.1" stroke-linecap="round"/>
  </g>
</g>`,
    css: `${rigOff('jelly')}
${q('jelly')} { --jc: color-mix(in srgb, var(--pet-accent) var(--t-hi), #3fd0c0); }
${q('jelly')} .jfill { fill: var(--jc); opacity: .95; } ${q('jelly')} .jt { stroke: var(--jc); opacity: .85; } ${q('jelly')} .jbub circle { fill: #e8fffb; opacity: 0; }
${q('jelly')} .jmv { transform-box: view-box; transform-origin: 24px 0; animation: c2-jf 3.8s ease-in-out infinite; }
${q('jelly')} .jbell { transform-box: fill-box; transform-origin: 50% 85%; animation: c2-jb 3.8s ease-in-out infinite; }
${q('jelly')} .jtents { transform-box: fill-box; transform-origin: 50% 0; }
${q('jelly')} .jt { transform-box: fill-box; transform-origin: 50% 0; animation: c2-jt 2.6s ease-in-out infinite alternate; }
${q('jelly')} .t2 { animation-delay: -.5s; animation-duration: 3s; } ${q('jelly')} .t3 { animation-delay: -1.1s; } ${q('jelly')} .t4 { animation-delay: -1.6s; animation-duration: 3.2s; } ${q('jelly')} .t5 { animation-delay: -2.1s; }
@keyframes c2-jf { 0%, 100% { transform: translateY(0); } 50% { transform: translateY(-2.5px); } }
@keyframes c2-jb { 0%, 100% { transform: scale(1, 1); } 50% { transform: scale(1.04, 0.96); } }
@keyframes c2-jt { from { transform: rotate(-7deg); } to { transform: rotate(7deg); } }
@keyframes c2-jpulse { from { transform: scale(1.1, 0.88); } to { transform: scale(0.93, 1.1); } }
@keyframes c2-jtp { from { transform: scaleY(0.88); } to { transform: scaleY(1.1); } }
@keyframes c2-jjump { 0% { transform: translateY(0); } 40% { transform: translateY(-7px); } 100% { transform: translateY(0); } }
@keyframes c2-jsink { from { transform: translateY(0); } to { transform: translateY(8px); } }
@keyframes c2-jrise { 0% { transform: translateY(3px); opacity: 0; } 25% { opacity: .9; } 100% { transform: translateY(-14px); opacity: 0; } }
${q('jelly')}[data-state="music"] .jbell { animation: c2-jpulse 0.5s ease-in-out infinite alternate; }
${q('jelly')}[data-state="music"] .jtents { animation: c2-jtp 0.5s ease-in-out infinite alternate; }
${q('jelly')}[data-state="music"] .jt { animation-duration: .5s; }
${q('jelly')}[data-state="thinking"] .jbell { animation: c2-jb 5s ease-in-out infinite; }
${q('jelly')}[data-state="thinking"] .jmv { animation-duration: 6s; }
${q('jelly')}[data-state="thinking"] .jbub circle { animation: c2-jrise 3.4s ease-in infinite; }
${q('jelly')}[data-state="thinking"] .jb2 { animation-delay: 1.1s; } ${q('jelly')}[data-state="thinking"] .jb3 { animation-delay: 2.2s; }
${q('jelly')}[data-state="thinking"] .eyes { transform: translate(1.5px, -1.2px); }
${q('jelly')}[data-state="done"] .jmv { animation: c2-jjump 0.7s cubic-bezier(.3,.7,.4,1) 1, c2-jf 3.8s ease-in-out 0.7s infinite; }
${q('jelly')}[data-state="done"] .jbell { animation: c2-jpulse 0.35s ease-in-out 3 alternate, c2-jb 3.8s ease-in-out 1.05s infinite; }
${q('jelly')}[data-state="attention"] .jbell { animation: c2-jpulse 0.18s ease-in-out infinite alternate; transform: scale(0.9, 1.1); }
${q('jelly')}[data-state="attention"] .jtents { transform: scaleY(1.08); } ${q('jelly')}[data-state="attention"] .jt { animation-duration: .25s; }
${q('jelly')}[data-state="file"] .jt { animation-duration: 1.1s; } ${q('jelly')}[data-state="file"] .eyes { transform: translate(0, 1.4px); }
${q('jelly')}[data-state="sleep"] .jmv { animation: c2-jsink 7s ease-in-out infinite alternate; opacity: .55; }
${q('jelly')}[data-state="sleep"] .jbell { animation: c2-jb 7s ease-in-out infinite; } ${q('jelly')}[data-state="sleep"] .jt { animation-duration: 6s; }
${q('jelly')}.poked .jbell { animation: c2-jpulse 0.22s ease-in-out 3 alternate; }
${q('jelly')}[data-energy] .jbell, ${q('jelly')}[data-energy] .jtents { animation: none !important; }`,
    init(host) { host._jb = host.querySelector('.jbell'); host._jt = host.querySelector('.jtents'); },
    energy(host, e) {
      st(host._jb, 'scale', e == null ? '' : `${(0.94 + e * 0.2).toFixed(3)} ${(1.08 - e * 0.2).toFixed(3)}`);
      st(host._jt, 'scale', e == null ? '' : `1 ${(0.92 + e * 0.2).toFixed(3)}`);
    },
  });

  // ============================== 4. sloth: hangs from the capsule's top edge by its arms ==============================
  const FUR = '#8b5e3c';
  P.add({
    id: 'sloth', group: 'characters', w: 48, h: 48,
    name: { ru: 'Ленивец', en: 'Sloth' }, desc: { ru: 'Висит, уцепившись за край, и раскачивается', en: 'Hangs from the edge by its arms and sways' },
    svg: (a) => `
<g class="swing">
  ${a.ln('', 'M16.5 -12C15 1 14 12 15.5 25', 4.2, FUR, 2)}${a.ln('', 'M31.5 -12C33 1 34 12 32.5 25', 4.2, FUR, 2)}
  <g class="curl">
    <ellipse class="o" cx="18" cy="45.2" rx="3.4" ry="2.2" fill="#7a5233"/><ellipse class="o" cx="30" cy="45.2" rx="3.4" ry="2.2" fill="#7a5233"/>
    <ellipse class="o" cx="24" cy="36" rx="9.6" ry="10.4" fill="#9a6a45"/><ellipse cx="24" cy="38.2" rx="5.4" ry="6.4" fill="#c99d70"/>
    <g class="head">
      <ellipse class="o" cx="24" cy="22" rx="11.2" ry="9.6" fill="#a97650"/>
      <ellipse cx="24" cy="23.3" rx="8.6" ry="7.2" fill="#ecd9b6"/>
      <ellipse cx="19" cy="22.5" rx="2.9" ry="3.6" fill="#4a2e1f" transform="rotate(-22 19 22.5)"/><ellipse cx="29" cy="22.5" rx="2.9" ry="3.6" fill="#4a2e1f" transform="rotate(22 29 22.5)"/>
      ${a.eyes([[19, 22.8], [29, 22.8]], 'dot', '#fffaf0', 1.3, '#f5e9d3')}
      <circle class="blush" style="--b:#f09aa9" cx="14.6" cy="27" r="1.9" fill="#f09aa9" opacity=".7"/><circle class="blush" style="--b:#f09aa9" cx="33.4" cy="27" r="1.9" fill="#f09aa9" opacity=".7"/>
      <ellipse cx="24" cy="26" rx="1.8" ry="1.25" fill="#3b2418"/><path d="M21.7 27.8Q24 30.2 26.3 27.8" fill="none" stroke="#3b2418" stroke-width="1.1" stroke-linecap="round"/>
      <g class="sfl"><g class="sfp">${[0, 72, 144, 216, 288].map((r) => `<circle cx="31.5" cy="${(14.6 - 2.7).toFixed(1)}" r="2" transform="rotate(${r} 31.5 14.6)"/>`).join('')}</g><circle cx="31.5" cy="14.6" r="1.5" fill="#ffd75e"/></g>
    </g>
  </g>
</g>`,
    css: `${rigOff('sloth')}
${q('sloth')} .sfp circle { fill: color-mix(in srgb, var(--pet-accent) var(--t-lo), #f783ac); stroke: #17141f; stroke-width: .5; }
${q('sloth')} .swing { transform-box: view-box; transform-origin: 24px 0; animation: c2-sw 4.6s ease-in-out infinite; }
${q('sloth')} .curl { transform-box: view-box; transform-origin: 24px 10px; transition: transform 1s ease-out; }
@keyframes c2-sw { 0%, 100% { transform: rotate(-3.2deg); } 50% { transform: rotate(3.2deg); } }
@keyframes c2-swm { 0%, 100% { transform: rotate(-9deg); } 50% { transform: rotate(9deg); } }
@keyframes c2-swd { 0% { transform: rotate(0); } 20% { transform: rotate(-12deg); } 45% { transform: rotate(10deg); } 70% { transform: rotate(-5deg); } 100% { transform: rotate(0); } }
${q('sloth')}[data-state="music"] .swing { animation: c2-swm 1s ease-in-out infinite; }
${q('sloth')}[data-state="thinking"] .swing { animation-duration: 7s; }
${q('sloth')}[data-state="done"] .swing { animation: c2-swd 0.9s ease-in-out 1, c2-sw 4.6s ease-in-out 0.9s infinite; }
${q('sloth')}[data-state="attention"] .swing { animation: none; transform: rotate(0); }
${q('sloth')}[data-state="attention"] .head { transform: translateY(-1.4px); }
${q('sloth')}[data-state="attention"] .eyes { transform: translate(0, -2px); }
${q('sloth')}[data-state="file"] .swing { animation-duration: 3.2s; } ${q('sloth')}[data-state="file"] .eyes { transform: translate(0, 1.4px); }
${q('sloth')}[data-state="sleep"] .swing { animation-duration: 9s; }
${q('sloth')}[data-state="sleep"] .curl { transform: scale(0.9, 0.84) translateY(2px); }
${q('sloth')}[data-state="sleep"] .head { transform: rotate(14deg) translateY(1.5px); }
${q('sloth')}.poked .swing { animation: c2-swd 0.8s ease-in-out 1; }
${q('sloth')}[data-energy] .swing { animation: c2-swm 0.7s ease-in-out infinite; }`,
    init(host) { host._sc = host.querySelector('.curl'); },
    energy(host, e) { st(host._sc, 'translate', e == null ? '' : `0 ${(e * 1.8).toFixed(1)}px`); },
  });

  // ============================== 5. astronaut: weightless, the visor glass follows the reaction ==============================
  const SUIT = '#eef1f7';
  const astar = (x, y, s, d) => `<path class="astar" style="animation-delay:${d}s" transform="translate(${x} ${y}) scale(${s})" d="M0-3L.8-.8L3 0L.8.8L0 3L-.8.8L-3 0L-.8-.8Z"/>`;
  P.add({
    id: 'astronaut', group: 'characters', w: 48, h: 48,
    name: { ru: 'Космонавт', en: 'Astronaut' }, desc: { ru: 'Парит в невесомости, цвет стекла шлема по реакции', en: 'Floats weightlessly; the helmet glass takes the reaction colour' },
    svg: (a) => `
<g class="astars">${astar(6.5, 8, 1, 0)}${astar(42, 6, .7, .7)}${astar(4.5, 31, .6, 1.4)}${astar(43.5, 30, .9, 2.1)}</g>
<g class="amv">
  <rect class="o" x="15" y="28" width="18" height="11" rx="3.5" fill="#8d99b3"/>
  <g class="legL"><ellipse class="o" cx="20.6" cy="43.8" rx="3.3" ry="3.1" fill="${SUIT}"/><path d="M17.5 45.2h6.2" stroke="#9aa5bd" stroke-width="1.2" stroke-linecap="round"/></g>
  <g class="legR"><ellipse class="o" cx="27.4" cy="43.8" rx="3.3" ry="3.1" fill="${SUIT}"/><path d="M24.3 45.2h6.2" stroke="#9aa5bd" stroke-width="1.2" stroke-linecap="round"/></g>
  <rect class="o" x="16.5" y="28" width="15" height="14.5" rx="6" fill="${SUIT}"/><circle class="apn" cx="24" cy="35.6" r="2.1"/>
  <g class="armL"><ellipse class="o" cx="13.2" cy="34.2" rx="3" ry="5.6" fill="${SUIT}" transform="rotate(18 13.2 34.2)"/></g>
  <g class="armR"><ellipse class="o" cx="34.8" cy="34.2" rx="3" ry="5.6" fill="${SUIT}" transform="rotate(-18 34.8 34.2)"/></g>
  <g class="athumb"><ellipse class="o" cx="37.6" cy="29.4" rx="3" ry="5.2" fill="${SUIT}" transform="rotate(-28 37.6 29.4)"/><rect class="o" x="38.6" y="20.8" width="3.2" height="6.4" rx="1.6" fill="${SUIT}" style="stroke-width:1.4"/></g>
  ${a.ln('', 'M33.5 10L36.6 5', 1.5, '#cfd5e6', 2)}<circle class="o aled" cx="37" cy="4.4" r="2.1" style="stroke-width:1.2"/>
  <ellipse class="o" cx="24" cy="30" rx="8.2" ry="2.1" fill="#c9d1e3" style="stroke-width:1.2"/>
  <circle class="o" cx="24" cy="19" r="12.6" fill="#f4f6fb"/>
  <ellipse class="o avis" cx="24" cy="19.6" rx="9.7" ry="8.3" style="stroke-width:1.3"/>
  <path d="M17.2 15.2Q19.4 11.6 24 11.6" fill="none" stroke="#fff" stroke-width="1.5" stroke-linecap="round" opacity=".55"/>
  ${a.eyes([[20, 19.6], [28, 19.6]], 'dot', '#f3f8ff', 1.9, '#f3f8ff')}
  <path class="amo" d="M22.4 23.3q1.6 1.6 3.2 0" fill="none" stroke="#f3f8ff" stroke-width="1.1" stroke-linecap="round" opacity=".9"/>
  <g class="az"><g transform="translate(29.4 15.6) scale(.7)"><path d="M-2.4 -2.9h4.8l-4.8 5.8h4.8" fill="none" stroke="#e8ebff" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></g><g transform="translate(32.6 11.6) scale(.5)"><path d="M-2.4 -2.9h4.8l-4.8 5.8h4.8" fill="none" stroke="#e8ebff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></g></g>
</g>`,
    css: `${rigOff('astronaut')}
${q('astronaut')} .avis { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #1a2340); }
${q('astronaut')} .apn { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #7fa8ff); stroke: #17141f; stroke-width: 1; }
${q('astronaut')} .aled { fill: color-mix(in srgb, var(--pet-accent) var(--t-hi), #ff7a7a); }
${q('astronaut')} .astar { fill: #fff3b0; opacity: .8; transform-box: fill-box; transform-origin: 50% 50%; animation: c2-tw 2.6s ease-in-out infinite; }
${q('astronaut')} .amv { transform-box: view-box; transform-origin: 24px 26px; animation: c2-af 6s ease-in-out infinite; }
${q('astronaut')} .legL, ${q('astronaut')} .legR { transform-box: fill-box; transform-origin: 50% 0; }
${q('astronaut')} .athumb, ${q('astronaut')} .az { display: none; }
@keyframes c2-af { 0%, 100% { transform: translateY(-2px) rotate(-3.5deg); } 50% { transform: translateY(2px) rotate(3.5deg); } }
@keyframes c2-tw { 0%, 100% { opacity: .25; transform: scale(.6); } 50% { opacity: 1; transform: scale(1.1); } }
@keyframes c2-ad { 0%, 100% { transform: translateY(0) rotate(-13deg); } 50% { transform: translateY(-3px) rotate(13deg); } }
@keyframes c2-arm { from { transform: rotate(-45deg); } to { transform: rotate(45deg); } }
@keyframes c2-leg { from { transform: rotate(-22deg); } to { transform: rotate(22deg); } }
@keyframes c2-aspin { from { transform: rotate(0) scale(1); } 50% { transform: rotate(180deg) scale(1.08); } to { transform: rotate(360deg) scale(1); } }
@keyframes c2-ashake { 0%, 100% { transform: translateX(-1px) rotate(-2deg); } 50% { transform: translateX(1px) rotate(2deg); } }
@keyframes c2-led { 50% { opacity: .2; } }
@keyframes c2-zp { 0%, 100% { opacity: .35; } 50% { opacity: 1; } }
${q('astronaut')}[data-state="music"] .amv { animation: c2-ad 0.8s ease-in-out infinite; }
${q('astronaut')}[data-state="music"] .armL { animation: c2-arm 0.4s ease-in-out infinite alternate; }
${q('astronaut')}[data-state="music"] .armR { animation: c2-arm 0.4s ease-in-out infinite alternate-reverse; }
${q('astronaut')}[data-state="music"] .legL { animation: c2-leg 0.4s ease-in-out infinite alternate; }
${q('astronaut')}[data-state="music"] .legR { animation: c2-leg 0.4s ease-in-out infinite alternate-reverse; }
${q('astronaut')}[data-state="thinking"] .amv { animation-duration: 9s; }
${q('astronaut')}[data-state="thinking"] .eyes { transform: translate(1.6px, -1.5px); }
${q('astronaut')}[data-state="done"] .amv { animation: c2-aspin 0.9s ease-in-out 1, c2-af 6s ease-in-out 0.9s infinite; }
${q('astronaut')}[data-state="done"] .armR { display: none; } ${q('astronaut')}[data-state="done"] .athumb { display: inline; }
${q('astronaut')}[data-state="attention"] .amv { animation: c2-ashake 0.14s linear infinite; }
${q('astronaut')}[data-state="attention"] .aled { animation: c2-led 0.3s linear infinite; }
${q('astronaut')}[data-state="attention"] .eye { transform: scale(1.3); }
${q('astronaut')}[data-state="file"] .eyes { transform: translate(0, 1.4px); } ${q('astronaut')}[data-state="file"] .armL { transform: rotate(-30deg); }
${q('astronaut')}[data-state="sleep"] .amv { animation: c2-af 14s ease-in-out infinite; }
${q('astronaut')}[data-state="sleep"] .az { display: inline; } ${q('astronaut')}[data-state="sleep"] .az g { animation: c2-zp 2.4s ease-in-out infinite; }
${q('astronaut')}[data-state="sleep"] .astar { animation-duration: 6s; } ${q('astronaut')}[data-state="sleep"] .amo { opacity: 0; }
${q('astronaut')} .fx-z { display: none !important; }
${q('astronaut')}[data-energy] .amv { animation-duration: 0.8s; }`,
    init(host) { host._am = host.querySelector('.amv'); },
    energy(host, e) { st(host._am, 'scale', e == null ? '' : (1 + e * 0.1).toFixed(3)); },
  });

  // ============================== 6. sprout: a sprout in a terracotta pot with a face ==============================
  const FW = [[-15, -2, -70], [-9, -10, -35], [0, -13, 0], [10, -10, 40], [15, -2, 75], [-12, 6, -120], [12, 6, 120]];
  P.add({
    id: 'sprout', group: 'characters', w: 48, h: 48,
    name: { ru: 'Росток', en: 'Sprout' }, desc: { ru: 'Листья качаются, от радости распускается цветок', en: 'Leaves sway; a flower blooms when it is happy' },
    svg: (a) => `
<g class="smv">
  <g class="fwk">${FW.map(([dx, dy, r], i) => `<g transform="translate(24 12)"><path class="fl fl${i % 3}" style="--dx:${dx}px;--dy:${dy}px;--r:${r}deg" d="M0-3.4Q2.2 0 0 3.4Q-2.2 0 0-3.4Z"/></g>`).join('')}</g>
  <g class="sprt">
    ${a.ln('', 'M24 32C24 26 25 20 24 13.5', 2.4, '#4f9d45', 2)}
    <g class="lfL"><path class="o lf" d="M24 22.5C15.5 23.5 10 19 9 12.5C17 11.5 23 15.5 24 22.5Z" style="stroke-width:1.6"/><path d="M23 21C19 18.5 14.5 16 11.5 14" fill="none" stroke="#17141f" stroke-width=".7" opacity=".35"/></g>
    <g class="lfR"><path class="o lf" d="M24 18C32.5 19 38 14.5 39 8C31 7 25 11 24 18Z" style="stroke-width:1.6"/><path d="M25 16.5C29 14 33.5 11.5 36.5 9.5" fill="none" stroke="#17141f" stroke-width=".7" opacity=".35"/></g>
    <ellipse class="o lf lbud" cx="24" cy="12.4" rx="2.1" ry="3.2" style="stroke-width:1.3"/>
    <g class="sflw"><g class="sfp">${[0, 72, 144, 216, 288].map((r) => `<circle cx="24" cy="${(13 - 3.3).toFixed(1)}" r="2.5" transform="rotate(${r} 24 13)"/>`).join('')}</g><circle cx="24" cy="13" r="2.1" fill="#ffd75e" stroke="#17141f" stroke-width=".6"/></g>
  </g>
  <path class="o" d="M14.5 33.5H33.5L31.7 46.2Q31.5 47 30.6 47H17.4Q16.5 47 16.3 46.2Z" fill="#c9603d"/>
  <path d="M17.5 36L18.7 45" stroke="#e58b63" stroke-width="1.3" stroke-linecap="round" opacity=".7"/>
  <rect class="o" x="12.3" y="30" width="23.4" height="5.6" rx="2.2" fill="#d97246"/>
  ${a.eyes([[20.4, 39.6], [27.6, 39.6]], 'dot', a.OL, 1.7)}
  <circle class="blush" style="--b:#ff9a86" cx="17.2" cy="42.6" r="1.7" fill="#ff9a86" opacity=".85"/><circle class="blush" style="--b:#ff9a86" cx="30.8" cy="42.6" r="1.7" fill="#ff9a86" opacity=".85"/>
  <path d="M22.5 42.2q1.5 1.8 3 0" fill="none" stroke="${a.OL}" stroke-width="1.1" stroke-linecap="round"/>
</g>`,
    css: `${rigOff('sprout')}
${q('sprout')} .lf { fill: color-mix(in srgb, var(--pet-accent) var(--t-lo), #6fcf5a); }
${q('sprout')} .sfp circle { fill: color-mix(in srgb, var(--pet-accent) var(--t-lo), #ff7aa8); stroke: #17141f; stroke-width: .6; }
${q('sprout')} .smv { transform-box: view-box; transform-origin: 24px 47px; }
${q('sprout')} .sprt { transform-box: view-box; transform-origin: 24px 31px; animation: c2-sp 3.8s ease-in-out infinite; transition: transform .6s ease-out; }
${q('sprout')} .lfL, ${q('sprout')} .lfR { transition: transform .5s ease-out; }
${q('sprout')} .lfL { transform-box: fill-box; transform-origin: 100% 100%; animation: c2-lfl 2.8s ease-in-out infinite alternate; }
${q('sprout')} .lfR { transform-box: fill-box; transform-origin: 0 100%; animation: c2-lfr 3.2s ease-in-out infinite alternate; }
${q('sprout')} .sflw { transform-box: fill-box; transform-origin: 50% 50%; transform: scale(0); }
${q('sprout')} .fl { fill: #7fe08a; opacity: 0; transform-box: fill-box; transform-origin: 50% 50%; } ${q('sprout')} .fl1 { fill: #ffd75e; } ${q('sprout')} .fl2 { fill: #ff8fb4; }
@keyframes c2-sp { 0%, 100% { transform: rotate(-3.5deg); } 50% { transform: rotate(3.5deg); } }
@keyframes c2-lfl { from { transform: rotate(-4deg); } to { transform: rotate(6deg); } }
@keyframes c2-lfr { from { transform: rotate(4deg); } to { transform: rotate(-6deg); } }
@keyframes c2-spm { 0%, 100% { transform: rotate(-9deg); } 50% { transform: rotate(9deg); } }
@keyframes c2-bob { 0%, 100% { transform: translateY(0) scale(1.04, .94); } 50% { transform: translateY(-3px) scale(.96, 1.05); } }
@keyframes c2-jmp { 0% { transform: scale(1.06, .9); } 40% { transform: translateY(-6px) scale(.95, 1.08); } 70% { transform: translateY(0) scale(1.04, .94); } 100% { transform: none; } }
@keyframes c2-bloom { 0% { transform: scale(0) rotate(-90deg); } 70% { transform: scale(1.25) rotate(8deg); } 100% { transform: scale(1) rotate(0); } }
@keyframes c2-fw { 0% { transform: translate(0, 0) scale(.2) rotate(0); opacity: 0; } 18% { opacity: 1; } 100% { transform: translate(var(--dx), var(--dy)) scale(1.1) rotate(var(--r)); opacity: 0; } }
@keyframes c2-perk { 0%, 100% { transform: scaleY(1.14) translateX(-.5px); } 50% { transform: scaleY(1.14) translateX(.5px); } }
${q('sprout')}[data-state="music"] .smv { animation: c2-bob 0.6s ease-in-out infinite; }
${q('sprout')}[data-state="music"] .sprt { animation: c2-spm 0.6s ease-in-out infinite; }
${q('sprout')}[data-state="thinking"] .sprt { animation-duration: 6s; transform-origin: 24px 31px; }
${q('sprout')}[data-state="thinking"] .lfR { animation-duration: 1.6s; }
${q('sprout')}[data-state="thinking"] .eyes { transform: translate(1.4px, -1.4px); }
${q('sprout')}[data-state="done"] .smv { animation: c2-jmp 0.8s cubic-bezier(.3,.7,.4,1) 1; }
${q('sprout')}[data-state="done"] .sprt { transform: scaleY(1.18); animation: none; }
${q('sprout')}[data-state="done"] .sflw { animation: c2-bloom 0.7s cubic-bezier(.3,1.3,.5,1) 0.25s both; }
${q('sprout')}[data-state="done"] .lbud { opacity: 0; }
${q('sprout')}[data-state="done"] .fl { animation: c2-fw 1.1s ease-out 0.2s both; }
${q('sprout')}[data-state="attention"] .sprt { animation: c2-perk 0.14s linear infinite; }
${q('sprout')}[data-state="attention"] .lfL { animation: none; transform: rotate(16deg); } ${q('sprout')}[data-state="attention"] .lfR { animation: none; transform: rotate(-16deg); }
${q('sprout')}[data-state="attention"] .eye { transform: scale(1.25); }
${q('sprout')}[data-state="file"] .eyes { transform: translate(0, 1.4px); } ${q('sprout')}[data-state="file"] .sprt { animation-duration: 2.4s; }
${q('sprout')}[data-state="sleep"] .sprt { animation: none; transform: rotate(27deg) scaleY(.86); }
${q('sprout')}[data-state="sleep"] .lfL { animation: none; transform: rotate(-42deg); } ${q('sprout')}[data-state="sleep"] .lfR { animation: none; transform: rotate(44deg); }
${q('sprout')}[data-state="sleep"] .lbud { transform-box: fill-box; transform-origin: 50% 100%; transform: rotate(40deg); }
${q('sprout')}.poked .smv { animation: c2-jmp 0.5s ease-out 1; }
${q('sprout')}[data-energy] .smv { animation: none !important; }`,
    init(host) { host._sm = host.querySelector('.smv'); },
    energy(host, e) { st(host._sm, 'translate', e == null ? '' : `0 ${(-e * 3.2).toFixed(1)}px`); st(host._sm, 'scale', e == null ? '' : `${(1.04 - e * 0.08).toFixed(3)} ${(0.96 + e * 0.1).toFixed(3)}`); },
  });
})();
