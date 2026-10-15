import test from 'node:test';
import assert from 'node:assert/strict';
import { apiRequest } from './api.js';
import { renderCollections, renderGameDetail, renderSearchPage, renderShell } from './views.js';

const collection = { id: 1, name: 'Friday <games>', slug: 'friday-games', lan_party_zone: true, rom_count: 2 };

test('collection links escape names and guest navigation stays inside its collection', () => {
  const listing = renderCollections([collection]);
  assert.match(listing, /href="\/friday-games"/);
  assert.match(listing, /Friday &lt;games&gt;/);
  assert.match(listing, /LAN Party Zone/);
  const output = renderSearchPage({ collection, page: { items: [], total: 0 }, route: { name: 'platforms' } });
  assert.match(output, /<h1 id="search-heading">Friday &lt;games&gt;<\/h1>/);
  assert.match(output, /No sign-in required/);
  const shell = renderShell({ user: null, collection, platforms: [], content: output, route: { name: 'platforms' } });
  assert.match(shell, /Search Friday &lt;games&gt;/);
  assert.match(shell, /Sign in<\/a>/);
  assert.doesNotMatch(shell, /data-action="logout"|Signed in account|Read-only/);
  const detail = renderGameDetail({ name: 'Selected', platform_id: 7, platform_slug: 'nes', files: [], regions: [] }, collection);
  assert.match(detail, /Back to Friday &lt;games&gt;/);
  assert.doesNotMatch(detail, /#\/platform\//);
});

test('anonymous requests are explicit, omit credentials and retain normal login requirements', async () => {
  const originalFetch = globalThis.fetch;
  let requests = 0;
  globalThis.fetch = async (path, options) => {
    requests++;
    assert.equal(path, '/api/collections/friday-games');
    assert.equal(options.headers.has('Authorization'), false);
    assert.equal(options.credentials, 'omit');
    return Response.json(collection);
  };
  try {
    await assert.rejects(apiRequest('/api/roms', null), (error) => error.status === 401);
    assert.equal(requests, 0);
    assert.deepEqual(await apiRequest('/api/collections/friday-games', null, { allowAnonymous: true }), collection);
    assert.equal(requests, 1);
  } finally {
    globalThis.fetch = originalFetch;
  }
});
