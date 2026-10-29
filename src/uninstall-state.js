// The uninstall panel's state machine, pure (no DOM, no Tauri): used by uninstall-ui.js in the bar page and by src/uninstall-state.test.mjs in node.
//
// Two things move independently:
//   * the JOB, which lives in the backend (uninstall.rs): waiting for the official uninstaller, the leftover scan, the checklist, the deletion, the
//     elevated step, the summary. Its phase arrives as `uninstall:state` events. The panel being open or closed never changes it.
//   * the TAKEOVER: whether the full panel is on screen instead of the normal search results.
// phases:    idle | confirm | running | scanning | checklist | deleting | admin | done | still | error
// events:    open {name}      the user picked an uninstall row: a job exists -> its panel comes back; otherwise the confirm card
//            server {payload} an uninstall:state event (never opens the panel by itself)
//            typing           any keystroke / input that is not the panel's own control: the panel steps aside, the job stays
//            esc              Esc inside the panel: like typing, but a confirm card is dropped and a finished summary is dismissed
//            focus            strip click, Ctrl+U, `u <name>`, the island card: the panel comes back where it was
//            close            the explicit Close / Cancel button: everything is dropped (the UI also tells the backend)
// selectors: strip(state)     what the status strip shows: null (nothing) | {kind: 'busy' | 'ready', phase, name}
(function (root, factory) {
  if (typeof module === 'object' && module.exports) module.exports = factory();
  else root.UninstallState = factory();
})(typeof self !== 'undefined' ? self : this, function () {
  const RUNNING = ['running', 'scanning', 'deleting', 'admin'];
  const FINISHED = ['checklist', 'done', 'still', 'error'];
  const MAP = { running: 'running', scanning: 'scanning', review: 'checklist', deleting: 'deleting', elevating: 'admin', done: 'done', still: 'still', error: 'error' };
  const isBusy = (phase) => RUNNING.includes(phase);
  const isFinished = (phase) => FINISHED.includes(phase);

  const init = () => ({ phase: 'idle', takeover: false, job: null, name: '', last: null });

  function reduce(s, ev) {
    switch (ev && ev.type) {
      case 'open':
        if (s.job) return { ...s, takeover: true }; // a job exists (the same app, or another one: finish or close it first): its panel comes back
        return { ...init(), phase: 'confirm', takeover: true, name: String(ev.name || '') };
      case 'server': {
        const p = ev.payload;
        const phase = p && MAP[p.phase];
        if (!phase) return s;
        return { phase, takeover: s.takeover, job: { name: String(p.name || s.name || '') }, name: String(p.name || s.name || ''), last: p };
      }
      case 'typing':
        if (!s.takeover) return s;
        return s.job ? { ...s, takeover: false } : init(); // a confirm card is not a job: typing drops it
      case 'esc':
        if (!s.takeover) return s;
        if (!s.job || ['done', 'still', 'error'].includes(s.phase)) return init(); // dismissed: nothing is left to come back to
        return { ...s, takeover: false };
      case 'focus':
        return s.job ? { ...s, takeover: true } : s;
      case 'close':
        return init();
      default:
        return s;
    }
  }

  /** The status strip: visible only while a job exists and the full panel is not on screen. */
  function strip(s) {
    if (!s.job || s.takeover) return null;
    return { kind: isBusy(s.phase) ? 'busy' : 'ready', phase: s.phase, name: s.job.name };
  }

  /** A keystroke that is typing for the search box, not one of the panel's own controls (Space / Enter / Tab / arrows / Esc stay with the panel). */
  function isTyping(e) {
    if (!e || e.ctrlKey || e.altKey || e.metaKey) return false;
    if (e.key === 'Backspace' || e.key === 'Delete') return !e.targetIsField;
    return typeof e.key === 'string' && Array.from(e.key).length === 1 && e.key !== ' ';
  }

  return { init, reduce, strip, isBusy, isFinished, isTyping, RUNNING, FINISHED, MAP };
});
