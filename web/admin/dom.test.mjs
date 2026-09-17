import test from 'node:test';
import assert from 'node:assert/strict';

import { coverPath, html, parseCsv, slugifyTitle } from './dom.js';

test('html escapes markup and quotes', () => {
  assert.equal(html(`<a title="x">Tom & 'Ada'</a>`), '&lt;a title=&quot;x&quot;&gt;Tom &amp; &#39;Ada&#39;&lt;/a&gt;');
});

test('parseCsv trims and removes empty values', () => {
  assert.deepEqual(parseCsv('US, Europe, , Japan'), ['US', 'Europe', 'Japan']);
});

test('slugifyTitle matches the server upload slug format', () => {
  assert.equal(slugifyTitle(' Final Fantasy VII: Rebirth! '), 'final-fantasy-vii-rebirth');
  assert.equal(slugifyTitle('日本語'), 'rom');
});

test('cover paths prefer the requested size and fall back to the other', () => {
  const rom = { path_cover_small: 'small.jpg', path_cover_large: 'large.jpg' };
  assert.equal(coverPath(rom), 'small.jpg');
  assert.equal(coverPath(rom, 'large'), 'large.jpg');
  assert.equal(coverPath({ path_cover_small: 'small.jpg' }, 'large'), 'small.jpg');
});
