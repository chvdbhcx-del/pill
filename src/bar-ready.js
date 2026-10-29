// Handshake for the lazily created command bar window (ui/lazy.rs, command.rs): tells Rust (cmd_ready) once the page has run all its scripts AND every
// event listener (bar:shown, uninstall:state, ...) is registered, because Rust shows the bar and sends bar:shown only after that.
// Loaded FIRST in bar.html: it wraps __TAURI__.event.listen to collect the registration promises of the scripts that follow.
(() => {
  const T = window.__TAURI__;
  const pending = [];
  try {
    const orig = T.event;
    T.event = Object.assign({}, orig, { listen: (...a) => { const p = orig.listen(...a); pending.push(p); return p; } });
  } catch (e) { /* event API not replaceable: the barrier below still orders the report after the registrations */ }
  const report = async () => {
    await Promise.allSettled(pending);
    try { (await T.event.listen('bar:ready-barrier', () => {}))(); } catch (e) { /* ignore */ } // one more round trip: IPC calls are handled in order
    T.core.invoke('cmd_ready').catch(() => {});
  };
  // DOMContentLoaded, not load: a hidden window may never paint, but its scripts run
  document.addEventListener('DOMContentLoaded', report, { once: true });
})();
