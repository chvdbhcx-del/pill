// State machine of the uninstall panel (src/uninstall-state.js): typing always leaves the takeover and keeps the job, the strip shows only while a job
// runs (or waits for review) and the panel is not on screen. Run: node src/uninstall-state.test.mjs
import { createRequire } from 'node:module';
import assert from 'node:assert/strict';

const U = createRequire(import.meta.url)('./uninstall-state.js');
const run = (s, ...evs) => evs.reduce((a, e) => U.reduce(a, e), s);
const srv = (phase, name = 'qBittorrent', extra = {}) => ({ type: 'server', payload: { phase, name, ...extra } });

// idle -> confirm
let s = U.init();
assert.equal(s.phase, 'idle');
assert.equal(U.strip(s), null);
s = run(s, { type: 'open', name: 'qBittorrent' });
assert.deepEqual([s.phase, s.takeover, s.job], ['confirm', true, null]);
assert.equal(U.strip(s), null, 'a confirm card is not a job: no strip');

// typing in the confirm card drops it (nothing was started)
assert.deepEqual(run(s, { type: 'typing' }), U.init());
assert.deepEqual(run(s, { type: 'esc' }), U.init());

// confirm -> running -> scanning -> checklist -> deleting -> done, the panel stays up all the way
const path = [['running', 'running'], ['scanning', 'scanning'], ['review', 'checklist'], ['deleting', 'deleting'], ['elevating', 'admin'], ['done', 'done']];
let t = s;
for (const [server, phase] of path) {
  t = run(t, srv(server));
  assert.equal(t.phase, phase, server);
  assert.equal(t.takeover, true, 'server events never move the panel by themselves');
  assert.equal(U.strip(t), null, 'panel on screen: no strip');
}
assert.deepEqual(run(U.init(), srv('still')).phase, 'still');
assert.deepEqual(run(U.init(), srv('error')).phase, 'error');
assert.equal(run(U.init(), { type: 'server', payload: { phase: 'bogus' } }).phase, 'idle', 'unknown phases are ignored');
assert.equal(run(U.init(), { type: 'server', payload: null }).phase, 'idle');

// typing leaves the takeover but keeps the job, in every running phase; the strip appears as "busy"
for (const phase of ['running', 'scanning', 'deleting', 'elevating']) {
  const a = run(s, srv(phase), { type: 'typing' });
  assert.equal(a.takeover, false, phase);
  assert.ok(a.job && a.job.name === 'qBittorrent', phase + ': the job is kept');
  assert.deepEqual(U.strip(a), { kind: 'busy', phase: U.MAP[phase], name: 'qBittorrent' }, phase);
  // the job keeps moving while the user searches elsewhere, and the panel does not come back by itself
  const b = run(a, srv('scanning'), srv('scanning'), srv('review'));
  assert.equal(b.takeover, false);
  assert.equal(b.phase, 'checklist');
  assert.deepEqual(U.strip(b), { kind: 'ready', phase: 'checklist', name: 'qBittorrent' }, 'finished while away: the strip turns into "review"');
  // coming back restores the panel (the UI redraws it from `last`)
  const c = run(b, { type: 'focus' });
  assert.deepEqual([c.takeover, c.phase, U.strip(c)], [true, 'checklist', null]);
  assert.equal(c.last.phase, 'review');
}

// the checklist is not lost when the user types: it stays a job until Close
let c = run(s, srv('scanning'), srv('review', 'qBittorrent', { items: [{ id: 0 }] }), { type: 'typing' });
assert.ok(c.job && c.phase === 'checklist' && c.last.items.length === 1);
assert.deepEqual(U.strip(c), { kind: 'ready', phase: 'checklist', name: 'qBittorrent' });
c = run(c, { type: 'typing' }, { type: 'typing' });
assert.equal(c.phase, 'checklist', 'more typing changes nothing');

// Esc: a running or checklist job only leaves the panel; a confirm card, a summary, "still installed" and an error are dismissed
for (const phase of ['running', 'scanning', 'deleting', 'elevating', 'review']) {
  const a = run(s, srv(phase), { type: 'esc' });
  assert.ok(a.job && !a.takeover, 'Esc keeps the job: ' + phase);
}
for (const phase of ['done', 'still', 'error']) assert.deepEqual(run(s, srv(phase), { type: 'esc' }), U.init(), 'Esc dismisses ' + phase);

// Esc / typing with the panel already away does nothing
const away = run(s, srv('scanning'), { type: 'esc' });
assert.deepEqual(run(away, { type: 'esc' }), away);
assert.deepEqual(run(away, { type: 'typing' }), away);

// focus (strip click, Ctrl+U, `u qBittorrent`, the island card) without a job does nothing
assert.deepEqual(run(U.init(), { type: 'focus' }), U.init());
// `u <name>` with a job brings the panel back instead of a second confirm card
const again = run(away, { type: 'open', name: 'qBittorrent' });
assert.deepEqual([again.takeover, again.phase, again.job.name], [true, 'scanning', 'qBittorrent']);
// another app's row while a job exists: the same, the job is not replaced
assert.equal(run(away, { type: 'open', name: 'Other' }).job.name, 'qBittorrent');

// explicit Close / Cancel drops everything, also when the strip is showing
assert.deepEqual(run(away, { type: 'close' }), U.init());
assert.equal(U.strip(run(away, { type: 'close' })), null);

// the strip is visible only when a job exists AND the panel is not shown
for (const phase of ['running', 'scanning', 'deleting', 'elevating', 'review', 'done', 'still', 'error']) {
  const open = run(s, srv(phase));
  assert.equal(U.strip(open), null, phase + ' with the panel up');
  assert.notEqual(U.strip(run(open, { type: 'typing' })) && U.strip(run(open, { type: 'typing' })).kind, null, phase + ' with the panel away');
}
assert.equal(U.strip(U.init()), null);
assert.equal(U.strip(run(U.init(), { type: 'open', name: 'x' }, { type: 'typing' })), null, 'a dropped confirm card leaves no strip');
// a job that was started while the panel was already away (the bar page reloaded, the state comes from the backend)
const recovered = run(U.init(), srv('scanning'));
assert.deepEqual([recovered.takeover, U.strip(recovered).kind], [false, 'busy']);
assert.equal(run(recovered, { type: 'focus' }).takeover, true);

// bar hide / show do not exist for the machine: nothing in it depends on the window being visible
assert.deepEqual(run(away, { type: 'hidden' }), away);

// what counts as typing for the search box
const key = (k, extra = {}) => ({ key: k, ...extra });
for (const k of ['a', 'Z', 'щ', '7', '.', '-', '😀']) assert.ok(U.isTyping(key(k)), k);
assert.ok(U.isTyping(key('Backspace')) && U.isTyping(key('Delete')));
for (const k of [' ', 'Enter', 'Tab', 'Escape', 'ArrowDown', 'ArrowUp', 'F4', 'Shift', 'Control', 'Home']) assert.ok(!U.isTyping(key(k)), k);
assert.ok(!U.isTyping(key('u', { ctrlKey: true })) && !U.isTyping(key('c', { metaKey: true })) && !U.isTyping(key('x', { altKey: true })), 'shortcuts are not typing');
assert.ok(!U.isTyping(key('Backspace', { targetIsField: true })), 'a field edits itself');
assert.ok(!U.isTyping(null));

// the machine never mutates its input
const frozen = Object.freeze({ ...U.init() });
assert.doesNotThrow(() => U.reduce(frozen, { type: 'open', name: 'x' }));

console.log('uninstall-state self-check: ok');
