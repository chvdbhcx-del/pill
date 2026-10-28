// Quiet hours: ONE pure function, used by pill.js at its choke points (toast / setLive / react) and tested in node (quiet.test.mjs).
// Settings: quiet.on (bool), quiet.from / quiet.to ("HH:MM", the window may cross midnight), quiet.days (the days a window STARTS on, "0" = Sunday ... "6" = Saturday;
// a missing list means every day). from == to is an empty window (never quiet). Windows Focus Assist cannot be read (no public API) and is not used.
(function (g) {
  const HM = /^(\d{1,2}):(\d{2})$/;
  const mins = (s) => { const m = HM.exec(String(s == null ? '' : s)); if (!m) return -1; const h = +m[1], n = +m[2]; return h < 24 && n < 60 ? h * 60 + n : -1; };
  function quietNow(s, d) {
    if (!s || s['quiet.on'] !== true) return false;
    const f = mins(s['quiet.from'] == null ? '22:00' : s['quiet.from']), t = mins(s['quiet.to'] == null ? '07:00' : s['quiet.to']);
    if (f < 0 || t < 0 || f === t) return false;
    const days = Array.isArray(s['quiet.days']) ? s['quiet.days'].map(String) : ['0', '1', '2', '3', '4', '5', '6'];
    const has = (x) => days.includes(String(x));
    const day = d.getDay(), m = d.getHours() * 60 + d.getMinutes();
    return f < t ? has(day) && m >= f && m < t : (m >= f && has(day)) || (m < t && has((day + 6) % 7)); // after midnight the window still belongs to the day it started on
  }
  g.PillQuiet = { quietNow, mins };
})(typeof window !== 'undefined' ? window : globalThis);
