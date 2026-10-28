// Self-check for the Settings search matching in settings-search.js. Run: node src/settings-search.test.mjs
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';

const win = {};
vm.runInNewContext(fs.readFileSync(new URL('./settings-search.js', import.meta.url), 'utf8'), { window: win });
const { match } = win.PillSettingsSearch;
const E = (page, title, desc = '', pageName = page) => ({ page, pageName, title, desc });
const rows = [
  E('appearance', 'Размер острова', 'На большом мониторе остров можно сделать крупнее', 'Внешний вид'),
  E('appearance', 'Прозрачность свёрнутого острова', 'Пилюля и её карточки', 'Внешний вид'),
  E('behavior', 'Прятать остров, пока он не нужен', 'Свёрнутый остров прячется за край экрана', 'Поведение'),
  E('behavior', 'Тихие часы', 'Пока тихо, карточки событий не показываются', 'Поведение'),
  E('general', 'Язык', 'По умолчанию — как в Windows', 'Основное'),
  E('search', 'Запускать Everything сам', 'поиск файлов', 'Поиск файлов'),
  E('modules', 'Поиск файлов в командной строке', '', 'Модули'),
];
const titles = (q, n) => Array.from(match(rows, q, n), (r) => r.title);

assert.deepEqual(titles(''), []);
assert.deepEqual(titles('   '), []);
assert.deepEqual(titles('zzz'), []);
assert.equal(match(null, 'a').length, 0);
// title match beats description match; ё / е and case are ignored
assert.deepEqual(titles('свернутого'), ['Прозрачность свёрнутого острова']);
assert.equal(titles('ОСТРОВ')[0], 'Размер острова');
assert.equal(titles('язык')[0], 'Язык');
// every word must match; words may be in the title, the page name or the description
assert.deepEqual(titles('размер остров'), ['Размер острова']);
assert.deepEqual(titles('поведение тихие'), ['Тихие часы']);
assert.deepEqual(titles('карточки пилюля'), ['Прозрачность свёрнутого острова']);
assert.deepEqual(titles('карточки тихо'), ['Тихие часы']);
// title hits rank before page-name hits before description hits
assert.deepEqual(titles('поиск файлов'), ['Поиск файлов в командной строке', 'Запускать Everything сам']);
assert.equal(titles('windows')[0], 'Язык');
// the cap and the default cap
const many = Array.from({ length: 30 }, (_, i) => E('p', 'Row ' + i));
assert.equal(match(many, 'row', 5).length, 5);
assert.equal(match(many, 'row').length, 8);
assert.equal(match(many, 'row 7')[0].title, 'Row 7');
console.log('settings-search self-check: ok');
