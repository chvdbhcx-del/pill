// Self-check for src/md-lite.js. Run: node tools/test-md-lite.js
const assert = require('node:assert/strict');
const M = require('../src/md-lite.js');

// paragraphs, lists, bold / italic / code
let b = M.parse('Hello **big** *world* and `x*y`\nsecond line\n\n- one\n* two **b**\n\n1. a\n2) b\n3. c');
assert.deepEqual(b.map((x) => x.t), ['p', 'ul', 'ol']);
assert.deepEqual(b[0].c.map((x) => x.t), ['text', 'b', 'text', 'i', 'text', 'code', 'text']);
assert.equal(b[0].c[5].s, 'x*y'); // markers inside code stay literal
assert.ok(b[0].c[4].s.includes('\nsecond line') === false && b[0].c[4].s.startsWith(' and '));
assert.equal(b[1].items.length, 2);
assert.equal(b[2].items.length, 3);
// math-like stars and unmatched marks stay text
assert.equal(M.plain('2 * 3 = 6 and 5*4 *x'), '2 * 3 = 6 and 5*4 *x');
assert.equal(M.plain('**unclosed bold'), '**unclosed bold');
assert.equal(M.plain('snake_case_name'), 'snake_case_name');
// fenced code: kept verbatim, language noted, unclosed fence runs to the end
b = M.parse('before\n```js\nlet a = 1; // **not bold**\n\n- not a list\n```\nafter');
assert.deepEqual(b.map((x) => x.t), ['p', 'code', 'p']);
assert.equal(b[1].lang, 'js');
assert.equal(b[1].s, 'let a = 1; // **not bold**\n\n- not a list');
assert.equal(M.parse('```\nopen').at(-1).s, 'open');
// headings become bold text, "**x** y" at line start is a paragraph, not a bullet
assert.equal(M.parse('## Title')[0].c[0].t, 'b');
assert.equal(M.parse('**Note:** hi')[0].t, 'p');
// links are text only
assert.equal(M.plain('see [docs](https://e.com/x) now'), 'see docs (https://e.com/x) now');
// plain(): markers stripped
assert.equal(M.plain('# T\n\n**a** *b* `c`\n\n- x\n- y\n\n1. p\n2. q\n\n```\ncode\n```'), 'T\n\na b c\n\n- x\n- y\n\n1. p\n2. q\n\ncode');
assert.equal(M.plain(''), '');
assert.equal(M.plain(null), '');
// pathological input stays fast
let t0 = Date.now();
M.parse('*a '.repeat(30000) + '**'.repeat(30000) + '['.repeat(30000) + '`'.repeat(30000));
assert.ok(Date.now() - t0 < 2000, 'slow on pathological input');

// render(): DOM only through text nodes, never a link, never raw markup
class N {
  constructor(tag) { this.tag = tag; this.children = []; this.className = ''; this.textContent = ''; this.listeners = {}; }
  append(...c) { this.children.push(...c); }
  addEventListener(n, f) { this.listeners[n] = f; }
}
const doc = { createElement: (t) => new N(t), createTextNode: (s) => ({ text: s }), createDocumentFragment: () => new N('#frag') };
const hostile = '<script>alert(1)</script> <img src=x onerror=1> [click](javascript:alert(1)) **<b>x</b>**\n```\n<i>raw</i>\n```';
const copied = [];
const f = M.render(doc, hostile, (s) => copied.push(s), 'Копировать');
const walk = (n, f2) => { f2(n); (n.children || []).forEach((c) => walk(c, f2)); };
const tags = new Set();
walk(f, (n) => n.tag && tags.add(n.tag));
assert.deepEqual([...tags].sort(), ['#frag', 'button', 'code', 'div', 'p', 'pre', 'span', 'strong']); // no a / img / script elements
walk(f, (n) => assert.ok(!('innerHTML' in n)));
const btn = f.children[1].children[1];
assert.equal(btn.tag, 'button');
btn.listeners.click();
assert.deepEqual(copied, ['<i>raw</i>']);

console.log('md-lite: all checks passed');
