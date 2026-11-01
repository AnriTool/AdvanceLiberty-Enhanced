/* Offline schema and editor interaction regression checks; no browser needed. */
'use strict';
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const html = fs.readFileSync(path.join(__dirname, '..', 'font-editor.html'), 'utf8');
const scripts = [...html.matchAll(/<script[^>]*>([\s\S]*?)<\/script>/g)].map(m => m[1]);
const elements = new Map();
function element() {
  return { value: '', checked: false, disabled: false, hidden: false, style: {}, dataset: {},
    parentElement: { hidden: false }, classList: { toggle() {} },
    append() {}, replaceChildren() {}, setAttribute() {}, addEventListener() {},
    getContext() { return new Proxy({}, { get: (obj, key) => obj[key] || (() => {}) }); },
    querySelectorAll() { return []; } };
}
for (const [, id] of html.matchAll(/id="([^"]+)"/g)) elements.set(id, element());
const $ = id => elements.get(id);
$('zoom').value = '4'; $('theme').value = 'gold'; $('preview-text').value = 'A AA';
const c = { console, document: {
  getElementById: $, createElement: element, querySelectorAll: () => [],
  addEventListener() {}, activeElement: { tagName: 'BODY' }
}, localStorage: { getItem: () => null, setItem() {} }, setTimeout: () => 1, clearTimeout() {} };
vm.createContext(c);
vm.runInContext(scripts[0] + '\nglobalThis.F = FontData;', c);
const F = c.F;
const pack = F.fresh();
pack.font.aliases = { a: 'A' };
for (const key of F.variants) pack[key] = F.variant(pack);
assert.equal(F.validate(pack).length, 0);
assert.equal(JSON.stringify(F.fragment(F.normalize(pack, F.fresh()))), JSON.stringify(F.fragment(pack)));
const resized = F.setAlphabet(pack, 'AB');
for (const key of F.variants) assert.ok(resized[key].glyphs.B);
for (const key of F.variants) {
  const bad = F.clone(pack); bad[key].glyphs = {};
  assert.ok(F.validate(bad).some(e => e.startsWith(key + ':')));
  bad[key] = F.clone(pack.font);
  assert.ok(F.validate(bad).some(e => e.includes('shared aliases')));
}
const noVariants = F.normalize(F.fresh(), pack);
assert.ok(F.variants.every(key => !noVariants[key]));
vm.runInContext(scripts[1], c);
const exported = () => JSON.parse($('json').value);
for (const key of F.variants) {
  $('font-variant').value = key; $('font-variant').onchange();
  assert.ok($('variant-enable-label').hidden === false);
  assert.equal($('variant-enabled').checked, false);
  assert.ok($('dimension-hint').textContent.includes('Uses font'));
  assert.ok(!exported()[key]);
  $('variant-enabled').checked = true; $('variant-enabled').onchange();
  assert.ok(exported()[key]);
  $('width').value = '2'; $('height').value = '1'; $('shadow').checked = false;
  $('apply-size').onclick();
  $('rows').value = '.#'; $('apply-rows').onclick();
  assert.deepEqual(exported()[key].glyphs.A, ['.#']);
  assert.equal(exported().font.width, 5, 'variant editing must not resize fallback font');
  $('undo').onclick();
  assert.deepEqual(exported()[key].glyphs.A, ['..']);
  $('redo').onclick();
  assert.deepEqual(exported()[key].glyphs.A, ['.#']);
  $('variant-enabled').checked = false; $('variant-enabled').onchange();
  assert.ok(!exported()[key]);
  assert.ok($('dimension-hint').textContent.includes('Uses font'));
}
$('color-tab').onclick();
assert.equal($('font-variant').parentElement.hidden, true);
$('color-enabled').checked = true; $('color-enabled').onchange();
assert.equal(exported().color_font.width, 16);
$('mono-tab').onclick();
assert.equal($('font-variant').parentElement.hidden, false);
console.log('PASS font schema, variant fallback, independent editing, undo/redo and color-font compatibility');
