import test from 'node:test';
import assert from 'node:assert/strict';

import { icon } from './icons.js';

test('shared Lucide renderer emits local decorative SVGs and escapes classes', () => {
  const markup = icon('layout-dashboard', 'nav<icon');
  assert.match(markup, /^<svg class="icon nav&lt;icon" aria-hidden="true"/);
  assert.match(markup, /<rect width="7" height="9" x="3" y="3" rx="1"\/>/);
  assert.doesNotMatch(markup, /script|href=/);
  for (const name of ['external-link', 'folder-search', 'info', 'refresh-cw', 'logout']) {
    assert.notEqual(icon(name), icon('unknown'), `${name} must not use the fallback`);
  }
});
