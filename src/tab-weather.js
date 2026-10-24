// Weather tab (S11, Pro in the original): current conditions, 8 hourly slots, 7-day list. Owner: net/chat agent.
// Rust: net.rs (weather_get / weather_search_city / weather_set_city; MET Norway + Nominatim, opt-in via settings `net.weather`).
// The page never fetches anything itself: the webview has no network path, every call goes through Rust.
(() => {
  const t = (d) => (window.Pill && Pill.t ? Pill.t(d) : d.en);
  const inv = (c, a) => (window.Pill ? Pill.invoke(c, a) : Promise.resolve(null));
  const el = (tag, cls, text) => { const e = document.createElement(tag); if (cls) e.className = cls; if (text != null) e.textContent = text; return e; };
  const svg = (inner, vb = '0 0 24 24', extra = '') => `<svg viewBox="${vb}" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" ${extra}>${inner}</svg>`;
  const ruLang = () => /^ru/i.test((window.Pill && Pill.lang) || navigator.language || '');

  // ---- own weather icons (48x48 grid, filled flat colours; own drawing) ----
  const CLOUD = 'M14 38h21a8 8 0 0 0 1.2-15.9A11 11 0 0 0 15.1 20 9 9 0 0 0 14 38z';
  const SUN = (cx, cy, r) => `<g stroke="#ffc83d" stroke-width="${r / 3}" stroke-linecap="round">${[0, 45, 90, 135, 180, 225, 270, 315].map((a) => { const x = Math.cos(a * Math.PI / 180), y = Math.sin(a * Math.PI / 180); return `<path d="M${(cx + x * r * 1.5).toFixed(1)} ${(cy + y * r * 1.5).toFixed(1)}L${(cx + x * r * 1.95).toFixed(1)} ${(cy + y * r * 1.95).toFixed(1)}"/>`; }).join('')}</g><circle cx="${cx}" cy="${cy}" r="${r}" fill="#ffc83d"/>`;
  const MOON = (x, y, s) => `<g transform="translate(${x} ${y}) scale(${s})"><path d="M20 3a15 15 0 1 0 15 21A12 12 0 0 1 20 3z" fill="#dfe4ff"/><circle cx="31" cy="6" r="1.5" fill="#dfe4ff"/><circle cx="36" cy="13" r="1.1" fill="#dfe4ff"/></g>`;
  const cloud = (fill, tx = 0, ty = 0, s = 1) => `<path transform="translate(${tx} ${ty}) scale(${s})" d="${CLOUD}" fill="${fill}"/>`;
  const ICONS = {
    'clear-day': SUN(24, 24, 9),
    'clear-night': MOON(5, 6, 1.05),
    'partly-day': SUN(18, 17, 7) + cloud('#e6eaf1', 5, 5, 0.9),
    'partly-night': MOON(2, 1, 0.8) + cloud('#e6eaf1', 5, 5, 0.9),
    cloud: cloud('#8b94a5', 7, -6, 0.85) + cloud('#dfe3ea', 0, 2, 0.95),
    fog: cloud('#c9cfda', 0, -6, 0.95) + '<g stroke="#aab3c2" stroke-width="3" stroke-linecap="round"><path d="M9 35h30M14 41h20"/></g>',
    rain: cloud('#a2abbb', 0, -6, 0.95) + '<g stroke="#6fb3ff" stroke-width="3" stroke-linecap="round"><path d="M17 36l-2.5 6M25 36l-2.5 6M33 36l-2.5 6"/></g>',
    sleet: cloud('#a2abbb', 0, -6, 0.95) + '<g stroke="#6fb3ff" stroke-width="3" stroke-linecap="round"><path d="M17 36l-2.5 6M33 36l-2.5 6"/></g><circle cx="25" cy="39.5" r="2.2" fill="#fff"/>',
    snow: cloud('#a2abbb', 0, -6, 0.95) + '<g fill="#fff"><circle cx="16" cy="38" r="2.3"/><circle cx="25" cy="42" r="2.3"/><circle cx="34" cy="38" r="2.3"/></g>',
    thunder: cloud('#7b8496', 0, -6, 0.95) + '<path d="M27 28l-8 12h5.5l-2.5 8 10-13h-6z" fill="#ffd23d"/>',
  };
  const icon = (name, px) => `<svg class="wx-ico" viewBox="0 0 48 48" width="${px}" height="${px}" aria-hidden="true">${ICONS[name] || ICONS.cloud}</svg>`;
  window.PillWxIcon = icon; // the collapsed capsule (pill.js) draws the same coloured icons as the forecast
  const UI = {
    pin: svg('<path d="M20 10c0 6-8 12-8 12s-8-6-8-12a8 8 0 0 1 16 0z"/><circle cx="12" cy="10" r="3"/>'),
    refresh: svg('<path d="M20 11a8 8 0 0 0-14.5-4M4 4v4h4M4 13a8 8 0 0 0 14.5 4M20 20v-4h-4"/>'),
    wind: svg('<path d="M3 8h10a3 3 0 1 0-3-3M3 12h15a3 3 0 1 1-3 3M3 16h7a2.5 2.5 0 1 1-2.5 2.5"/>'),
    drop: svg('<path d="M12 3s6 6.2 6 10.5a6 6 0 0 1-12 0C6 9.200 12 3 12 3z"/><path d="M9.500 14a2.600 2.600 0 0 0 2.500 2.500"/>'),
    umbrella: svg('<path d="M3 12a9 9 0 0 1 18 0z"/><path d="M12 12v6.500a2 2 0 0 0 4 0"/><path d="M12 3v1"/>'),
    sunrise: svg('<path d="M12 3v7M8.500 6.500 12 3l3.500 3.500M3 20h18M6 16a6 6 0 0 1 12 0"/>'),
    sunset: svg('<path d="M12 10V3M8.500 6.500 12 10l3.500-3.500M3 20h18M6 16a6 6 0 0 1 12 0"/>'),
    cloudsun: svg('<path d="M12 2v2M4.900 4.900l1.400 1.400M20 12h2M19.100 4.900l-1.400 1.400M15.900 12.600a4 4 0 0 0-5.900-4.100"/><path d="M13 22H7a5 5 0 1 1 4.900-6H13a3 3 0 0 1 0 6z"/>'),
    search: svg('<circle cx="11" cy="11" r="7"/><path d="m20 20-3.500-3.500"/>'),
  };

  // ---- formatting ----
  const S = () => (window.Pill && Pill.settings) || {};
  const metric = () => (cfg['weather.units'] || 'metric') !== 'imperial';
  const temp = (c) => (c == null ? '–' : Math.round(metric() ? c : c * 9 / 5 + 32) + '°');
  const wind = (ms) => (metric() ? Math.round(ms) + ' ' + t({ ru: 'м/с', en: 'm/s' }) : Math.round(ms * 2.23694) + ' mph');
  const DIRS = () => (ruLang() ? ['С', 'СВ', 'В', 'ЮВ', 'Ю', 'ЮЗ', 'З', 'СЗ'] : ['N', 'NE', 'E', 'SE', 'S', 'SW', 'W', 'NW']);
  const dir = (deg) => DIRS()[Math.round(deg / 45) % 8];
  const mm = (v) => (v >= 0.1 ? (ruLang() ? String(v.toFixed(1)).replace('.', ',') : v.toFixed(1)) + ' ' + t({ ru: 'мм', en: 'mm' }) : '');
  const clock24 = () => S().clock24 !== false;
  const hm = (ms) => {
    const d = new Date(ms);
    if (clock24()) return String(d.getHours()).padStart(2, '0') + ':' + String(d.getMinutes()).padStart(2, '0');
    const h = d.getHours() % 12 || 12;
    return h + ':' + String(d.getMinutes()).padStart(2, '0') + ' ' + (d.getHours() < 12 ? 'AM' : 'PM');
  };
  const hourLabel = (ms) => {
    const d = new Date(ms);
    return clock24() ? String(d.getHours()).padStart(2, '0') + ':00' : (d.getHours() % 12 || 12) + ' ' + (d.getHours() < 12 ? 'AM' : 'PM');
  };
  const DOW = () => (ruLang() ? ['Вс', 'Пн', 'Вт', 'Ср', 'Чт', 'Пт', 'Сб'] : ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']);

  function cond(sym, ic) {
    const night = /night/.test(ic || '');
    const light = sym.startsWith('light'), heavy = sym.startsWith('heavy'), sh = sym.includes('showers');
    const L = (ru, en) => t({ ru, en });
    if (sym.includes('thunder')) return L('Гроза', 'Thunderstorm');
    if (sym.includes('sleet')) return L(light ? 'Небольшой мокрый снег' : 'Дождь со снегом', light ? 'Light sleet' : 'Sleet');
    if (sym.includes('snow')) return L(light ? 'Небольшой снег' : heavy ? 'Сильный снегопад' : sh ? 'Снегопад' : 'Снег', light ? 'Light snow' : heavy ? 'Heavy snow' : sh ? 'Snow showers' : 'Snow');
    if (sym.includes('rain')) return L(sh ? (light ? 'Небольшие ливни' : heavy ? 'Сильные ливни' : 'Ливни') : light ? 'Небольшой дождь' : heavy ? 'Сильный дождь' : 'Дождь', sh ? (light ? 'Light showers' : heavy ? 'Heavy showers' : 'Showers') : light ? 'Light rain' : heavy ? 'Heavy rain' : 'Rain');
    switch (sym) {
      case 'clearsky': return night ? L('Ясно', 'Clear') : L('Ясно', 'Clear');
      case 'fair': return L('Малооблачно', 'Mostly clear');
      case 'partlycloudy': return L('Переменная облачность', 'Partly cloudy');
      case 'fog': return L('Туман', 'Fog');
      default: return L('Пасмурно', 'Cloudy');
    }
  }

  // temperature -> colour for the range bars (own palette)
  const STOPS = [[-10, [90, 160, 255]], [0, [110, 208, 224]], [8, [127, 217, 192]], [14, [185, 220, 140]], [20, [230, 215, 122]], [26, [245, 177, 95]], [34, [255, 123, 90]]];
  function tcolor(c) {
    if (c <= STOPS[0][0]) return `rgb(${STOPS[0][1]})`;
    for (let i = 1; i < STOPS.length; i++) {
      if (c <= STOPS[i][0]) {
        const [a, ca] = STOPS[i - 1], [b, cb] = STOPS[i], f = (c - a) / (b - a);
        return `rgb(${ca.map((v, k) => Math.round(v + (cb[k] - v) * f)).join(',')})`;
      }
    }
    return `rgb(${STOPS[STOPS.length - 1][1]})`;
  }

  // ---- state ----
  let root = null, body = null, cfg = {}, data = null, visible = false, timer = 0, busy = false, searching = false;
  let editCity = false, lastErr = '', prevG = ''; // prevG: condition (icon) of the last gradient shown, for the cross-fade
  const tz = () => -new Date().getTimezoneOffset();

  async function loadCfg() {
    const c = await inv('settings_get');
    if (c) cfg = c;
    return cfg;
  }
  const patch = (p) => inv('settings_set', { patch: p }).then((c) => { if (c) cfg = c; return c; });

  async function refresh(force) {
    if (!root || busy) return;
    busy = true;
    try {
      await loadCfg();
      if (!cfg['net.weather']) { data = null; return render('off'); }
      if (!data) render('loading');
      const r = await inv('weather_get', { force: !!force, tz: tz() });
      if (!r) { lastErr = t({ ru: 'Нет ответа от приложения', en: 'No answer from the app' }); return render('error'); }
      if (r.status === 'ok') { data = r; lastErr = ''; return render(editCity ? 'city' : 'ok'); }
      if (r.status === 'nocity') { data = null; return render('city'); }
      if (r.status === 'off') { data = null; return render('off'); }
      lastErr = r.message || 'error';
      render(data ? 'ok' : 'error');
    } finally { busy = false; }
  }

  // ---- views ----
  function render(mode) {
    if (!body) return;
    body.className = 'wx-body';
    if (mode === 'ok' && data) return renderOk();
    if (mode === 'off') return renderOff();
    if (mode === 'city') return renderCity();
    if (mode === 'error') return renderError();
    body.replaceChildren(el('div', 'wx-center dim', t({ ru: 'Загрузка…', en: 'Loading…' })));
  }

  function centerCard(iconHtml, title, text) {
    const c = el('div', 'wx-center');
    const i = el('div', 'wx-badge');
    i.innerHTML = iconHtml;
    c.append(i, el('div', 'wx-title', title));
    if (text) c.append(el('div', 'wx-text', text));
    return c;
  }

  function renderOff() {
    const c = centerCard(UI.cloudsun, t({ ru: 'Погода выключена', en: 'Weather is off' }),
      t({ ru: 'Прогноз берётся у норвежского метеоинститута (api.met.no), города ищутся через OpenStreetMap Nominatim. Пока вы не включите, ничего не отправляется.', en: 'The forecast comes from the Norwegian Meteorological Institute (api.met.no) and cities are searched through OpenStreetMap Nominatim. Nothing is sent until you turn it on.' }));
    const contact = el('input', 'field wx-contact');
    contact.placeholder = t({ ru: 'Контакт для MET (e-mail или сайт), по желанию', en: 'Contact for MET (e-mail or site), optional' });
    contact.value = cfg['net.contact'] || '';
    contact.title = t({ ru: 'MET Norway просит указывать контакт в User-Agent. Отправляется только им.', en: 'MET Norway asks for a contact in the User-Agent. Sent to them only.' });
    const b = el('button', 'btn-primary wx-enable', t({ ru: 'Включить', en: 'Enable' }));
    b.onclick = async () => {
      const p = { 'net.weather': true };
      if (contact.value.trim()) p['net.contact'] = contact.value.trim();
      await patch(p);
      refresh(true);
    };
    c.append(contact, b);
    body.replaceChildren(c);
  }

  function renderCity() {
    const c = centerCard(UI.pin, t({ ru: 'Выберите город', en: 'Choose a city' }), t({ ru: 'Введите название и нажмите Enter: поиск идёт только по запросу.', en: 'Type a name and press Enter: the search runs only on request.' }));
    const row = el('div', 'wx-search');
    const inp = el('input', 'field');
    inp.placeholder = t({ ru: 'Город', en: 'City' });
    inp.value = (editCity && data && data.city) || '';
    const go = el('button', 'btn-primary', t({ ru: 'Найти', en: 'Search' }));
    const list = el('div', 'wx-results');
    const msg = el('div', 'wx-text dim');
    const run = async () => {
      const q = inp.value.trim();
      if (q.length < 2 || searching) return;
      searching = true; go.disabled = true; msg.textContent = t({ ru: 'Ищу…', en: 'Searching…' }); list.replaceChildren();
      const r = await inv('weather_search_city', { query: q, lang: ruLang() ? 'ru' : 'en' });
      searching = false; go.disabled = false;
      if (!r || r.status !== 'ok') { msg.textContent = (r && r.message) || t({ ru: 'Поиск не удался', en: 'Search failed' }); return; }
      msg.textContent = r.results.length ? '' : t({ ru: 'Ничего не найдено', en: 'Nothing found' });
      list.replaceChildren(...r.results.map((x) => {
        const b = el('button', 'wx-res');
        b.append(el('span', 'wx-res-n', x.name), el('span', 'wx-res-d dim', x.detail || ''));
        b.onclick = async () => { await inv('weather_set_city', { name: x.name, lat: x.lat, lon: x.lon }); editCity = false; data = null; render('loading'); refresh(true); };
        return b;
      }));
    };
    inp.onkeydown = (e) => { if (e.key === 'Enter') run(); };
    go.onclick = run;
    row.append(inp, go);
    c.append(row, msg, list);
    if (data) { const x = el('button', 'btn-ghost', t({ ru: 'Отмена', en: 'Cancel' })); x.onclick = () => { editCity = false; render('ok'); }; c.append(x); }
    body.replaceChildren(c);
    setTimeout(() => inp.focus(), 50);
  }

  function renderError() {
    const c = centerCard(UI.cloudsun, t({ ru: 'Не удалось получить прогноз', en: 'Could not load the forecast' }), lastErr);
    const b = el('button', 'btn', t({ ru: 'Повторить', en: 'Retry' }));
    b.onclick = () => refresh(true);
    const city = el('button', 'btn-ghost', t({ ru: 'Другой город', en: 'Change city' }));
    city.onclick = () => { editCity = true; render('city'); };
    const off = el('button', 'btn-ghost', t({ ru: 'Выключить', en: 'Turn off' }));
    off.onclick = async () => { await patch({ 'net.weather': false }); refresh(); };
    const row = el('div', 'wx-btnrow'); row.append(b, city, off);
    c.append(row);
    body.replaceChildren(c);
  }

  function renderOk() {
    const n = data.now, grid = el('div', 'wx-grid');
    // left: current conditions
    const left = el('div', 'wx-now wx-g-' + n.icon);
    // a gradient cannot transition in CSS: the previous condition's gradient stays as a layer under the content and fades out
    if (prevG && prevG !== n.icon && window.PillMotion) {
      const ghost = el('div', 'wx-g-' + prevG);
      ghost.style.cssText = 'position:absolute;inset:0;z-index:-1;pointer-events:none;border-radius:inherit';
      left.style.isolation = 'isolate';
      left.prepend(ghost);
      queueMicrotask(() => window.PillMotion.exit(ghost, { dur: 0.8, ease: 'ease-in-out', remove: true })); // after renderOk has attached the tree
    }
    prevG = n.icon;
    const head = el('div', 'wx-head');
    const city = el('button', 'wx-city');
    city.innerHTML = UI.pin;
    city.append(el('span', null, data.city));
    city.title = t({ ru: 'Сменить город', en: 'Change city' });
    city.onclick = () => { editCity = true; render('city'); };
    const rf = el('button', 'icon-btn wx-rf');
    rf.innerHTML = UI.refresh;
    rf.title = t({ ru: 'Обновить', en: 'Refresh' });
    rf.onclick = async () => { rf.classList.add('spin'); await refresh(true); };
    head.append(city, rf);
    const big = el('div', 'wx-big');
    big.innerHTML = icon(n.icon, 50);
    big.append(el('span', 'wx-deg', temp(n.temp)));
    left.append(head, big, el('div', 'wx-cond', cond(n.symbol, n.icon)),
      el('div', 'wx-feels', t({ ru: 'Ощущается как ', en: 'Feels like ' }) + temp(n.feels) + ' · ' + temp(n.min) + '…' + temp(n.max)));
    const stats = el('div', 'wx-stats');
    const stat = (ic, v) => { const s = el('div', 'wx-stat'); s.innerHTML = ic; s.append(el('b', null, v)); return s; };
    stats.append(stat(UI.wind, wind(n.wind) + ' ' + dir(n.dir)), stat(UI.drop, Math.round(n.humidity) + '%'), stat(UI.umbrella, mm(n.precip) || '0 ' + t({ ru: 'мм', en: 'mm' })));
    left.append(el('div', 'wx-sp'), stats);
    const sun = el('div', 'wx-sun');
    if (data.sun && data.sun.rise != null) {
      const a = el('span'); a.innerHTML = UI.sunrise; a.append(hm(data.sun.rise));
      const b = el('span'); b.innerHTML = UI.sunset; b.append(hm(data.sun.set));
      sun.append(a, b);
    } else sun.append(el('span', null, data.sun && data.sun.state === 'polar_day' ? t({ ru: 'Полярный день', en: 'Midnight sun' }) : t({ ru: 'Полярная ночь', en: 'Polar night' })));
    left.append(sun);

    // right: hourly + daily
    const right = el('div', 'wx-right');
    const hours = el('div', 'card wx-hours');
    data.hourly.forEach((h, i) => {
      const c = el('div', 'wx-h');
      c.append(el('span', 'wx-hl', i === 0 ? t({ ru: 'Сейчас', en: 'Now' }) : hourLabel(h.t)));
      const ic = el('span', 'wx-hi'); ic.innerHTML = icon(h.icon, 24);
      c.append(ic, el('b', null, temp(h.temp)), el('span', 'wx-hp', mm(h.precip)));
      hours.append(c);
    });
    const days = el('div', 'card wx-days');
    const gmin = Math.min(...data.daily.map((d) => d.min)), gmax = Math.max(...data.daily.map((d) => d.max)), span = Math.max(gmax - gmin, 1);
    data.daily.forEach((d, i) => {
      const r = el('div', 'wx-d');
      r.append(el('b', 'wx-dn' + (i === 0 ? ' today' : ''), i === 0 ? t({ ru: 'Сегодня', en: 'Today' }) : DOW()[d.dow]));
      const ic = el('span', 'wx-di'); ic.innerHTML = icon(d.icon, 22);
      const bar = el('span', 'wx-bar'), fill = el('i');
      fill.style.left = ((d.min - gmin) / span * 100) + '%';
      fill.style.width = Math.max((d.max - d.min) / span * 100, 4) + '%';
      fill.style.background = `linear-gradient(90deg, ${tcolor(d.min)}, ${tcolor(d.max)})`;
      bar.append(fill);
      r.append(ic, el('span', 'wx-lo', temp(d.min)), bar, el('b', 'wx-hi', temp(d.max)), el('span', 'wx-dp', mm(d.precip)));
      days.append(r);
    });
    const foot = el('div', 'wx-foot');
    const units = el('span', 'wx-units');
    ['metric', 'imperial'].forEach((u) => {
      const b = el('button', metric() === (u === 'metric') ? 'on' : '', u === 'metric' ? '°C' : '°F');
      b.onclick = async () => { await patch({ 'weather.units': u }); renderOk(); };
      units.append(b);
    });
    const warn = data.stale ? t({ ru: 'Нет сети, показан сохранённый прогноз · ', en: 'Offline, showing the saved forecast · ' }) : '';
    foot.append(units, el('span', 'wx-src dim', warn + t({ ru: 'Данные: MET Norway', en: 'Data: MET Norway' })));
    foot.title = data.warning || '';
    days.append(foot);
    right.append(hours, days);
    grid.append(left, right);
    body.replaceChildren(grid);
  }

  function schedule() {
    clearInterval(timer);
    if (visible) timer = setInterval(() => refresh(false), 5 * 60 * 1000); // Rust serves the 30-min disk cache in between
  }

  Pill.register({
    id: 'weather', order: 60, height: 417, // content height the live app uses for this tab (Home is 338): the shell may grow the body for it
    title: t({ ru: 'Погода', en: 'Weather' }),
    icon: window.pillIcon ? window.pillIcon('weather') : svg(UI.cloudsun.replace(/<\/?svg[^>]*>/g, '')),
    mount(r) {
      root = r;
      root.classList.add('tab-weather');
      body = el('div', 'wx-body');
      root.append(body);
      render('loading');
      if (window.Pill && Pill.on) Pill.on('settings:changed', (c) => { if (c) { const was = cfg['net.weather']; cfg = c; if (visible && (was !== c['net.weather'] || data == null)) refresh(); } });
    },
    onShow() { visible = true; schedule(); refresh(false); },
    onHide() { visible = false; schedule(); },
  });
})();
