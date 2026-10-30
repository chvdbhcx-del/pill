// Injected before any page script by tabs-b-web.mjs (Page.addScriptToEvaluateOnNewDocument), after `const __TB_CFG = {...};` is prepended.
// A stand-in for the Tauri runtime: window.__TAURI__.{core.invoke, event.listen, window.getCurrentWindow}, a frozen clock, seeded localStorage.
// Replies come from __TB_CFG.replies[cmd]: a JSON value, {"__error": "text"} (the invoke rejects) or {"__pending": true} (never answers: a "busy" state).
(() => {
  const cfg = __TB_CFG;
  const listeners = new Map(); // event name -> Set<fn>
  const calls = [];
  const NOW = cfg.now;

  // ---- frozen clock: Date.now() and new Date() are always `now` (timers, "in 18 min", ring fractions are deterministic)
  const RealDate = Date;
  class FrozenDate extends RealDate {
    constructor(...a) { if (a.length) super(...a); else super(NOW); }
    static now() { return NOW; }
  }
  window.Date = FrozenDate;

  // ---- storage seed (localStorage may throw on some origins: ignore)
  try { for (const [k, v] of Object.entries(cfg.storage || {})) localStorage.setItem(k, String(v)); } catch (e) { /* blocked */ }

  const clone = (v) => (v === undefined ? null : JSON.parse(JSON.stringify(v)));
  const tb = window.__TB = {
    cfg, calls,
    replies: cfg.replies || {},
    emit(name, payload) { for (const f of [...(listeners.get(name) || [])]) { try { f({ event: name, payload: clone(payload) }); } catch (e) { console.error('emit', name, e); } } },
    has(name) { return (listeners.get(name) || new Set()).size; },
    listeners: () => [...listeners.keys()],
  };

  async function invoke(cmd, args) {
    calls.push({ cmd, args: clone(args) });
    if (cmd === 'pill_set_state' && args && args.state === 'expanded') { setTimeout(() => tb.emit('pill:expand', { tab: args.tab || null }), 0); return null; }
    if (cmd === 'plugin:event|listen') return 0;
    const r = tb.replies[cmd];
    if (r && typeof r === 'object' && r.__pending) return new Promise(() => {});
    if (r && typeof r === 'object' && '__error' in r) throw r.__error;
    return clone(r === undefined ? null : r);
  }

  const noopWin = new Proxy({}, { get: (_, k) => (k === 'then' ? undefined : (...a) => (k === 'isMaximized' ? Promise.resolve(false) : k === 'onResized' ? Promise.resolve(() => {}) : Promise.resolve(null))) });
  window.__TAURI__ = {
    core: { invoke, convertFileSrc: (p) => p },
    event: {
      listen(name, cb) {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(cb);
        return Promise.resolve(() => listeners.get(name).delete(cb));
      },
      emit: (name, payload) => { tb.emit(name, payload); return Promise.resolve(); },
    },
    window: { getCurrentWindow: () => noopWin, getCurrent: () => noopWin },
    webview: { getCurrentWebview: () => noopWin },
  };

  // ---- no clipboard permission prompts, no real focus-driven side effects
  try { Object.defineProperty(navigator, 'clipboard', { value: { writeText: () => Promise.resolve(), readText: () => Promise.resolve('') }, configurable: true }); } catch (e) { /* ignore */ }

  // ---- determinism: no CSS animation/transition/caret blink noise in the shots (the native side renders end states)
  if (cfg.freezeCss !== false) {
    const css = '*,*::before,*::after{animation-duration:0s !important;animation-delay:0s !important;transition-duration:0s !important;transition-delay:0s !important;caret-color:transparent !important}' +
      '::-webkit-scrollbar{width:0 !important;height:0 !important}';
    const add = () => { const s = document.createElement('style'); s.id = '__tb_css'; s.textContent = css; (document.head || document.documentElement).append(s); };
    if (document.head) add(); else document.addEventListener('DOMContentLoaded', add);
  }
})();
