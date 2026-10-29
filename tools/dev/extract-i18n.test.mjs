// Tests for tools/dev/extract-i18n.mjs. Run: node --test tools/dev/extract-i18n.test.mjs
//  - idempotence: two CLI runs write byte-identical tables, and the committed table.rs equals a fresh run;
//  - invariants: both sides non-empty, ids unique and sorted, id == FNV-1a(en NUL ru);
//  - call-site count: the extractor's object-literal sites (+ the dynamic ones) match a raw `ru:` key grep within 1 %
//    (the grep also hits a few comments, one doc example in i18n-lite.js and the positional / ternary helpers are extra on our side);
//  - spot checks of known strings, placeholders, ternaries, positional helpers and strings inside `${}` of html templates;
//  - lexer robustness on a synthetic file (regex literals, nested templates, comments, escapes).
import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { extract, render, idOf } from './extract-i18n.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(HERE, '..', '..');
const SRC = path.join(ROOT, 'src');
const TABLE = path.join(ROOT, 'src-tauri', 'ui-core', 'src', 'i18n', 'table.rs');
const tool = path.join(HERE, 'extract-i18n.mjs');
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'i18n-'));
const run = (out) => { const r = spawnSync(process.execPath, [tool, '--out', out], { encoding: 'utf8' }); assert.equal(r.status, 0, r.stderr); return fs.readFileSync(out, 'utf8'); };

test('extraction is idempotent and the committed table is current', () => {
  const a = run(path.join(tmp, 'a.rs')), b = run(path.join(tmp, 'b.rs'));
  assert.equal(a, b);
  assert.equal(fs.readFileSync(TABLE, 'utf8').replace(/\r\n/g, '\n'), a, 'table.rs is stale: run node tools/dev/extract-i18n.mjs');
  const chk = spawnSync(process.execPath, [tool, '--check'], { encoding: 'utf8' });
  assert.equal(chk.status, 0, chk.stderr);
});

const R = extract(SRC);
const byEn = (en) => R.list.filter((e) => e.en === en);

test('every pair is non-empty, ids are unique, sorted and content-addressed', () => {
  assert.ok(R.list.length > 1000, 'expected >1000 unique pairs, got ' + R.list.length);
  const seen = new Set();
  let prev = -1n;
  for (const e of R.list) {
    assert.ok(e.ru.trim() && e.en.trim(), 'empty side: ' + e.ru + ' / ' + e.en);
    assert.ok(!seen.has(e.id), 'duplicate id ' + e.id);
    seen.add(e.id);
    assert.ok(e.id > prev, 'not sorted');
    prev = e.id;
    assert.equal(e.id, idOf(e.ru, e.en));
  }
  const pairs = new Set(R.list.map((e) => e.ru + '\0' + e.en));
  assert.equal(pairs.size, R.list.length, 'duplicate (ru, en) pair');
});

test('call-site count agrees with a raw grep of the sources (documented 1 % tolerance)', () => {
  let raw = 0;
  for (const f of fs.readdirSync(SRC).filter((n) => /\.(js|html)$/.test(n))) {
    raw += [...fs.readFileSync(path.join(SRC, f), 'utf8').matchAll(/(^|[^A-Za-z0-9_$])('ru'|"ru"|ru)\s*:/g)].length;
  }
  const objDyn = R.dynamic.filter((d) => d.kind === 'obj' && d.why.startsWith('non-literal')).length; // 'unpaired' ones have no ru key (en-only fallbacks)
  const ours = R.byKind.obj + objDyn;
  const tol = Math.ceil(raw * 0.01);
  assert.ok(Math.abs(ours - raw) <= tol, `object sites ${ours} vs raw grep ${raw} (tolerance ${tol})`);
  assert.ok(R.sites >= raw - tol, 'total sites must cover the grep count');
  assert.ok(R.dynamic.length <= 20, 'too many dynamic sites: ' + R.dynamic.length);
});

test('spot checks: plain, placeholder, ternary, positional helper, nested template', () => {
  const has = (ru, en) => assert.ok(R.list.some((e) => e.ru === ru && e.en === en), `missing ${ru} / ${en}`);
  has('Скриншот сохранён', 'Screenshot saved');                        // capture-live.js, inside a P.toast({ title: t({..}) })
  has('Выдели область · Esc — отмена', 'Drag to select an area · Esc to cancel'); // capture.js positional L(a, b)
  has('Шаг {0} из {1}', 'Step {0} of {1}');                          // template literal with ${wiz} / ${STEPS}
  has('Искать в браузере: «{0}»', 'Search in the browser: “{0}”');   // ternary lang === 'ru' ? `..` : `..`
  has('Напомню: ', 'Reminder set: ');                                // isRu() ? .. : .. (trailing space kept)
  has('Небольшой снег', 'Light snow');                               // weather: zipped condition chains
  has('Стоп', 'Stop');                                               // inside an html template ${esc(t({..}))}
  has('Ящик проверен {0}, непрочитанных писем: {1}', 'Mailbox checked {0}, unread messages: {1}'); // concatenation
  assert.ok(byEn('Close').length >= 1);
});

test('lexer handles regex literals, nested templates, comments and escapes', () => {
  const d = fs.mkdtempSync(path.join(tmp, 'syn-'));
  fs.writeFileSync(path.join(d, 'x.js'), [
    "// t({ ru: 'в комментарии', en: 'in a comment' })",
    "const re = /['\"`]\\/ru:/g; const div = a / b / c;",
    "const css = `.a { ru: 1; } ${t({ ru: 'Внутри', en: 'Inside' })} ${`${t({ ru: 'Глубже', en: 'Deeper' })}`}`;",
    "x(t({ 'ru': \"Кавычки\", \"en\": 'Quotes \\'q\\' \\u00e9 \\n' }), t({ en: 'En first', ru: 'Сначала en' }));",
    "const L = (a, b) => (ru ? a : b); L('Позиция', 'Positional'); L(n ? 'А' : 'Б', n ? 'A' : 'B');",
    "const s = isRu() ? `через ${m} мин` : `in ${m} min`; const loc = lang === 'ru' ? 'ru-RU' : 'en-US';",
    "t({ ru: 'Один ' + a + ' два', en: 'One ' + a + ' two' }); t({ ru: v, en: w });",
  ].join('\n'));
  const r = extract(d);
  const got = new Set(r.list.map((e) => e.ru + ' | ' + e.en));
  for (const want of ['Внутри | Inside', 'Глубже | Deeper', "Кавычки | Quotes 'q' é \n", 'Сначала en | En first', 'Позиция | Positional', 'А | A', 'Б | B', 'через {0} мин | in {0} min', 'Один {0} два | One {0} two']) {
    assert.ok(got.has(want), 'missing ' + JSON.stringify(want) + ' in ' + JSON.stringify([...got]));
  }
  assert.ok(![...got].some((g) => g.includes('комментарии') || g.includes('ru-RU')), 'comment / locale leaked');
  assert.equal(r.dynamic.length, 1); // t({ ru: v, en: w })
  assert.equal(render(r), render(extract(d)));
});
