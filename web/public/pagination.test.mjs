import test from 'node:test';
import assert from 'node:assert/strict';
import { renderPagination } from './pagination.js';

const controls = (markup) => [...markup.matchAll(/<(?:a|button)\b[^>]*>/g)].map(([tag]) => tag);

test('public and admin pagination share numbered pages and boundary states', () => {
  for (const href of [null, (page) => `#/search?q=Sonic&Knuckles&page=${page}`]) {
    assert.equal(renderPagination({ total: 0, limit: 24, offset: 0 }, href), '');
    assert.equal(renderPagination({ total: 24, limit: 24, offset: 0 }, href), '');
    const first = renderPagination({ total: 49, limit: 24, offset: 0 }, href);
    assert.match(first, /<nav class="pagination" aria-label="Game pages">/);
    assert.match(controls(first)[0], /aria-label="Previous page" disabled/);
    assert.match(first, /aria-label="Page 1" aria-current="page"/);
    assert.equal(controls(first).length, 5);
    if (href) {
      assert.match(first, /href="#\/search\?q=Sonic&amp;Knuckles&amp;page=2"/);
      assert.doesNotMatch(first, /data-page-offset/);
    } else {
      assert.match(first, /aria-label="Page 2" data-page-offset="24"/);
      assert.match(first, /aria-label="Next page" data-page-offset="24"/);
    }
    const last = renderPagination({ total: 49, limit: 24, offset: 48 }, href);
    assert.match(controls(last).at(-1), /aria-label="Next page" disabled/);
    assert.match(last, /aria-label="Page 3" aria-current="page"/);
  }
});

test('large catalogs keep first, last, and nearby pages without rendering every page', () => {
  const markup = renderPagination({ total: 1_000_000, limit: 100, offset: 49_900 });
  assert.match(markup, /aria-label="Page 1"/);
  assert.match(markup, /aria-label="Page 500" aria-current="page" data-page-offset="49900"/);
  assert.match(markup, /aria-label="Page 10000" data-page-offset="999900"/);
  assert.equal((markup.match(/class="pagination-gap"/g) || []).length, 2);
  assert.equal(controls(markup).length, 7);
  assert.match(markup, /aria-label="Previous page" data-page-offset="49800"/);
  assert.match(markup, /aria-label="Next page" data-page-offset="50000"/);
});
