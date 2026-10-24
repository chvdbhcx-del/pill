// System live events of the collapsed pill: volume HUD, microphone/camera dots, downloads. New file of the Pill app, loaded by pill.html;
// user request: "делай все оставшееся" (live-events producers for volume, mic/camera, downloads). Owner: live-events lane. Rust side: src-tauri/src/livesys.rs.
//  - live:volume {level 0..100, muted}              -> speaker + bar + number chip for ~1.6 s (priority 88); the mouse wheel over the collapsed capsule changes the volume
//  - live:privacy {mic, cam, apps}                  -> small orange (mic) / green (camera) dots in the top-right corner of the capsule, soft pulse, no text, no tooltip
//  - live:download {state: progress|done|gone, id, name, bytes, total|null, path}
//                                                    -> chip with ring + file name + size while a browser download grows (priority 55), green card when it is done
// PRIVACY: nothing here reads sound, picture or file content. File names are only ever shown as text (textContent / esc), never as HTML.
// Switches: Settings > Live events > Volume / Microphone and camera / Downloads (Pill.liveOn('volume' | 'micCam' | 'downloads')).
(() => {
  'use strict';
  const T = window.__TAURI__, P = window.Pill;
  if (!P || !T || !T.event) return;
  const { t, icon } = P;
  const inv = (c, a) => P.invoke(c, a);
  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const collapsed = () => P.state === 'collapsed';
  const clamp = (n, a, b) => Math.min(b, Math.max(a, n));
  const on = (n, f) => T.event.listen(n, (e) => { try { f(e.payload); } catch (err) { console.error('live-system', n, err); } });

  // ================= volume =================
  const VOL_MS = 1600, WHEEL_STEP = 2;
  let vol = { level: -1, muted: false }, volHtml = '', volUntil = 0;
  const volOn = () => P.liveOn('volume') && (P.moduleOn('module.media') || P.moduleOn('module.controls')); // same gate as livesys.rs
  const volIcon = (lvl, muted) => icon(muted || !lvl ? 'mute' : 'volume', { size: 17, sw: 1.8 });

  function volume(p) {
    if (!p) return;
    vol = { level: clamp(p.level | 0, 0, 100), muted: !!p.muted };
    if (!volOn() || !collapsed()) return; // the Home slider moves the volume while the panel is open: no HUD then
    const w = vol.muted ? 0 : vol.level, now = Date.now();
    const el = now < volUntil && document.querySelector('.lv-vol');
    if (el) { // same HUD still on screen: update in place (a new html string would blur-swap the whole chip on every step)
      el.classList.toggle('muted', vol.muted);
      const ico = el.querySelector('.vol-ico'), name = vol.muted || !vol.level ? 'mute' : 'volume';
      if (ico.dataset.i !== name) { ico.dataset.i = name; ico.innerHTML = volIcon(vol.level, vol.muted); }
      el.querySelector('.vol-bar i').style.width = w + '%';
      el.querySelector('.vol-n').textContent = w;
    } else {
      volHtml = `<div class="lp lv lv-vol${vol.muted ? ' muted' : ''}"><span class="lv-ico vol-ico" data-i="${vol.muted || !vol.level ? 'mute' : 'volume'}">${volIcon(vol.level, vol.muted)}</span>`
        + `<span class="vol-bar"><i style="width:${w}%"></i></span><span class="lv-code vol-n">${w}</span></div>`;
    }
    volUntil = now + VOL_MS;
    P.setLive('volume', { priority: 88, ttlMs: VOL_MS, html: volHtml }); // same string while the HUD lives: only the timer is renewed
  }

  // mouse wheel over the collapsed capsule: +-2 % per notch. The Rust callback sees the change and shows the HUD.
  let wheelBusy = false;
  async function wheel(dir) {
    if (vol.level < 0) {
      if (wheelBusy) return;
      wheelBusy = true;
      const v = await inv('media_volume_get');
      wheelBusy = false;
      if (!v) return;
      vol = { level: clamp(v.volume | 0, 0, 100), muted: !!v.muted };
    }
    const next = clamp(vol.level + dir * WHEEL_STEP, 0, 100);
    if (vol.muted && dir > 0) { vol.muted = false; inv('media_mute', { muted: false }); }
    vol.level = next;
    inv('media_volume_set', { v: next });
  }
  const bodyEl = document.getElementById('body');
  if (bodyEl) {
    bodyEl.addEventListener('wheel', (e) => {
      if (!collapsed() || !volOn() || !e.deltaY) return;
      e.preventDefault();
      wheel(e.deltaY < 0 ? 1 : -1);
    }, { passive: false });
  }

  // ================= microphone / camera =================
  let priv = { mic: false, cam: false, apps: [] }, dots = null, dotsKey = '';
  function drawPriv() {
    const ok = P.liveOn('micCam'), mic = ok && priv.mic, cam = ok && priv.cam, key = (cam ? 'c' : '') + (mic ? 'm' : '');
    if (key === dotsKey) return;
    dotsKey = key;
    if (!key) { if (dots) dots.replaceChildren(); return; }
    if (!dots) {
      const live = document.getElementById('live');
      if (!live) return;
      dots = document.createElement('span');
      dots.className = 'lv-priv';
      dots.setAttribute('aria-hidden', 'true');
      live.append(dots);
    }
    dots.innerHTML = (cam ? '<i class="pv cam"></i>' : '') + (mic ? '<i class="pv mic"></i>' : ''); // static markup, nothing from the event
  }
  function privacy(p) {
    if (!p) return;
    priv = { mic: !!p.mic, cam: !!p.cam, apps: Array.isArray(p.apps) ? p.apps.slice(0, 4).map(String) : [] };
    drawPriv();
  }

  // ================= downloads =================
  const DL_TTL = 3500, DL_STALE = 15000;
  const dls = new Map(); // id -> { name, bytes, total, t }
  let dlShown = '', dlHtml = '';
  const UNITS = () => (P.lang === 'ru' ? ['Б', 'КБ', 'МБ', 'ГБ', 'ТБ'] : ['B', 'KB', 'MB', 'GB', 'TB']);
  function fmtSize(n) {
    n = Math.max(0, +n || 0);
    const u = UNITS();
    let i = 0;
    while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
    return (i === 0 || n >= 100 ? Math.round(n) : n.toFixed(1)) + ' ' + u[i];
  }
  // long names keep their extension: "very-long-name-of-a-fi….pdf"
  function cutName(s, n = 28) {
    s = String(s || '');
    if (s.length <= n) return s;
    const i = s.lastIndexOf('.'), ext = i > 0 && s.length - i <= 8 ? s.slice(i) : '';
    return s.slice(0, Math.max(4, n - ext.length - 1)) + '…' + ext;
  }
  const ringHtml = (total, bytes) => {
    const pct = total > 0 ? clamp(Math.round((bytes / total) * 100), 1, 100) : 0;
    return `<svg class="dl-ring${pct ? '' : ' ind'}" viewBox="0 0 18 18" width="18" height="18"><circle class="bg" cx="9" cy="9" r="7"/><circle class="fg" cx="9" cy="9" r="7" pathLength="100" stroke-dasharray="${pct || 28} 100"/></svg>`;
  };

  function dlRender() {
    const now = Date.now();
    for (const [k, v] of dls) if (now - v.t > DL_STALE) dls.delete(k);
    if (!dls.size || !P.liveOn('downloads')) {
      if (dlShown || dlHtml) { dlShown = ''; dlHtml = ''; P.setLive('download', null); }
      return;
    }
    let curK = '', cur = null;
    for (const [k, v] of dls) if (!cur || v.t >= cur.t) { cur = v; curK = k; }
    const more = dls.size > 1 ? '+' + (dls.size - 1) : '';
    const el = dlShown === curK && document.querySelector('.lv-dl');
    if (el) { // same download still on screen: update in place, no blur-swap once a second
      el.querySelector('.dl-size').textContent = fmtSize(cur.bytes);
      el.querySelector('.dl-more').textContent = more;
      if (cur.total > 0) el.querySelector('.dl-ring-w').innerHTML = ringHtml(cur.total, cur.bytes);
    } else {
      dlShown = curK;
      const label = cur.name ? cutName(cur.name) : t({ ru: 'Загрузка', en: 'Downloading' });
      dlHtml = `<div class="lp lv lv-dl"><span class="dl-ring-w">${ringHtml(cur.total, cur.bytes)}</span><span class="dl-name">${esc(label)}</span>`
        + `<span class="lv-dim dl-size">${esc(fmtSize(cur.bytes))}</span><span class="lv-dim dl-more">${esc(more)}</span></div>`;
    }
    P.setLive('download', { priority: 55, ttlMs: DL_TTL, html: dlHtml });
  }

  function download(p) {
    if (!p || !p.state) return;
    const id = String(p.id || '');
    if (p.state === 'progress') {
      if (!P.liveOn('downloads') || !id) return;
      dls.set(id, { name: String(p.name || ''), bytes: +p.bytes || 0, total: +p.total || 0, t: Date.now() });
      dlRender();
    } else if (p.state === 'gone') { // cancelled: the temp file disappeared and no finished file appeared
      dls.delete(id);
      dlRender();
    } else if (p.state === 'done') {
      dls.delete(id);
      dlRender();
      if (!P.liveOn('downloads')) return;
      P.react('download', 'file', { ttlMs: 3000 }); // edge reaction
      const name = String(p.name || ''), path = String(p.path || '');
      P.toast({
        id: 'download:' + id, icon: 'check', tone: 'green', ttlMs: 9000,
        title: t({ ru: 'Загрузка завершена', en: 'Download complete' }),
        sub: cutName(name, 44) + (p.bytes ? ' · ' + fmtSize(p.bytes) : ''),
        actions: [
          { label: t({ ru: 'Показать в папке', en: 'Show in folder' }), kind: 'white', run: () => inv('livesys_reveal', { path }) }, // Rust accepts only files inside Downloads
          { label: t({ ru: 'Закрыть', en: 'Close' }), run: () => {} },
        ],
      });
    }
  }

  // ================= wiring =================
  on('live:volume', volume);
  on('live:privacy', privacy);
  on('live:download', download);
  // the switches work at once, both ways
  on('settings:changed', () => setTimeout(() => {
    drawPriv();
    if (!P.liveOn('downloads')) { dls.clear(); dlRender(); }
    if (!volOn()) { volUntil = 0; P.setLive('volume', null); }
  }, 0));
  // the Rust polls (mic/camera, Downloads) pause while the pill is hidden
  on('pill:hide', () => inv('livesys_visible', { on: false }));
  on('pill:show', () => inv('livesys_visible', { on: true }));
  inv('livesys_privacy').then(privacy); // events sent before this script loaded are gone: ask once
  // updater.rs found a newer release (only when "check on start" is on): a card that opens the Updates page
  on('update:available', (p) => P.toast({
    id: 'update', icon: 'bell', tone: 'green', ttlMs: 9000,
    title: t({ ru: 'Доступна новая версия Pill', en: 'A new Pill version is available' }), sub: String((p && p.version) || ''),
    actions: [{ label: t({ ru: 'Открыть', en: 'Open' }), kind: 'white', run: () => inv('settings_open', { page: 'updates' }) }, { label: t({ ru: 'Позже', en: 'Later' }), run: () => {} }],
  }));
})();
