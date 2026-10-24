// Self-check for the pure text tools in tab-text.js. Run: node src/tab-text.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const src = fs.readFileSync(new URL('./tab-text.js', import.meta.url), 'utf8');
const win = {}; // no Pill here, so the script only exposes window.PillTextTools
vm.runInNewContext(src, { window: win, btoa, atob, TextEncoder, TextDecoder, Intl, Uint8Array, console });
const T = win.PillTextTools;

// keyboard layout, both directions, auto-detected
assert.equal(T.layoutFix('ghbdtn'), 'привет');
assert.equal(T.layoutFix('руддщ цщкдв'), 'hello world');
assert.equal(T.layoutFix('Ghbdtn vbh'), 'Привет мир');
assert.equal(T.layoutFix(T.layoutFix('qwerty ;\'[]')), 'qwerty ;\'[]');
// transliteration both ways
assert.equal(T.translit('Привет, мир'), 'Privet, mir');
assert.equal(T.translit('Privet, mir'), 'Привет, мир');
assert.equal(T.translit(T.translit('Москва')), 'Москва');
assert.equal(T.translit('Щука ЖУК'), 'Shchuka ZHUK');
assert.equal(T.translit('Shchuka'), 'Щука');
// json
assert.equal(T.jsonPretty('{"a":[1,2],"b":"x"}'), '{\n  "a": [\n    1,\n    2\n  ],\n  "b": "x"\n}');
assert.equal(T.jsonMin(T.jsonPretty('{"a":[1,2],"b":"x"}')), '{"a":[1,2],"b":"x"}');
assert.throws(() => T.jsonPretty('{nope'));
// base64 round trip (UTF-8) and errors
for (const s of ['hello', 'Привет, мир! 😀', '', 'a\nb\tc']) assert.equal(T.b64dec(T.b64enc(s)), s);
assert.equal(T.b64enc('hi'), 'aGk=');
assert.equal(T.b64dec('aGk'), 'hi');
assert.throws(() => T.b64dec('%%%'));
// url
assert.equal(T.urlDec(T.urlEnc('a b&c=д')), 'a b&c=д');
assert.throws(() => T.urlDec('%E0%A4%A'));
// case / lines / spaces
assert.equal(T.titleCase('hELLO wORLD-foo don\'t'), 'Hello World-Foo Don\'t');
assert.equal(T.sentenceCase('hello there. HOW are you? fine\nok'), 'Hello there. How are you? Fine\nOk');
assert.equal(T.joinLines('a\r\n b \n\nc'), 'a b c');
assert.equal(T.extraSpaces('a   b \t c  \n  d'), 'a b c\nd');
assert.equal(T.sortLines('b\nA\nc10\nc9'), 'A\nb\nc9\nc10');
assert.equal(T.sortLines('A\nb\nc9\nc10'), 'c10\nc9\nb\nA');
assert.equal(T.dedupeLines('a\nb\na\nc\nb'), 'a\nb\nc');
assert.deepEqual({ ...T.stats('56 chars\nx y') }, { chars: 12, nospace: 9, words: 4, lines: 2, sentences: 1 });
// qr: structure checks (the decode check lives in the scratchpad: jsQR)
const q = T.qr('https://example.com');
assert.equal(q.size, q.version * 4 + 17);
assert.ok(q.get(0, 0) && q.get(6, 6) && !q.get(7, 7) && q.get(q.size - 1, 0) && q.get(0, q.size - 1));
assert.equal(T.qr('x'.repeat(271)).version, 10);
assert.throws(() => T.qr('x'.repeat(272)), /qr-too-long/);
console.log('tab-text self-check: ok');
