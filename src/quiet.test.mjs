// Self-check for the quiet-hours window logic in quiet.js. Run: node src/quiet.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const win = {};
vm.runInNewContext(fs.readFileSync(new URL('./quiet.js', import.meta.url), 'utf8'), { window: win });
const { quietNow } = win.PillQuiet;
// 2026-10-26 is a Monday; local time constructor
const at = (day, h, m = 0) => new Date(2026, 9, 25 + day, h, m); // day 0 = Sunday 25 Oct
const S = (o) => ({ 'quiet.on': true, 'quiet.from': '22:00', 'quiet.to': '07:00', ...o });

// disabled / missing / broken settings are never quiet
assert.equal(quietNow(null, at(1, 23)), false);
assert.equal(quietNow({}, at(1, 23)), false);
assert.equal(quietNow(S({ 'quiet.on': false }), at(1, 23)), false);
assert.equal(quietNow(S({ 'quiet.from': '25:00' }), at(1, 23)), false);
assert.equal(quietNow(S({ 'quiet.to': 'x' }), at(1, 23)), false);
assert.equal(quietNow(S({ 'quiet.from': '08:00', 'quiet.to': '08:00' }), at(1, 8)), false); // empty window

// crossing midnight, every day
assert.equal(quietNow(S(), at(1, 21, 59)), false);
assert.equal(quietNow(S(), at(1, 22, 0)), true);
assert.equal(quietNow(S(), at(1, 23, 30)), true);
assert.equal(quietNow(S(), at(2, 0, 0)), true);
assert.equal(quietNow(S(), at(2, 6, 59)), true);
assert.equal(quietNow(S(), at(2, 7, 0)), false);
assert.equal(quietNow(S(), at(2, 12)), false);

// same-day window
const day = S({ 'quiet.from': '13:00', 'quiet.to': '14:30' });
assert.equal(quietNow(day, at(3, 13, 0)), true);
assert.equal(quietNow(day, at(3, 14, 29)), true);
assert.equal(quietNow(day, at(3, 14, 30)), false);
assert.equal(quietNow(day, at(3, 12, 59)), false);

// weekdays: a window belongs to the day it starts on (Mon-Fri = 1..5)
const wk = S({ 'quiet.days': ['1', '2', '3', '4', '5'] });
assert.equal(quietNow(wk, at(5, 23)), true);   // Friday 23:00
assert.equal(quietNow(wk, at(6, 3)), true);    // Saturday 03:00 is still Friday's window
assert.equal(quietNow(wk, at(6, 23)), false);  // Saturday 23:00 starts a Saturday window
assert.equal(quietNow(wk, at(0, 3)), false);   // Sunday 03:00 belongs to Saturday
assert.equal(quietNow(wk, at(1, 3)), false);   // Monday 03:00 belongs to Sunday
assert.equal(quietNow(wk, at(1, 22, 30)), true);
assert.equal(quietNow(S({ 'quiet.days': [1, 2] }), at(1, 23)), true); // numbers work too
assert.equal(quietNow(S({ 'quiet.days': [] }), at(1, 23)), false);
console.log('quiet self-check: ok');
