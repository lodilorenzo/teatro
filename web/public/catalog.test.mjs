import test from 'node:test';
import assert from 'node:assert/strict';

import {
  contextualSearchHash,
  gameHash,
  pageCount,
  parseRoute,
  platformHash,
  platformInitials,
  populatedPlatforms,
  romListPath,
  searchHash,
  totalGames,
} from './catalog.js';
import { formatBytes, metadataList } from './shared.js';

test('catalog exposes only populated platforms in display-name order', () => {
  const platforms = [
    { id: 2, slug: 'zeta', display_name: 'Zeta', rom_count: 3 },
    { id: 3, slug: 'empty', display_name: 'Empty', rom_count: 0 },
    { id: 1, slug: 'alpha', display_name: 'alpha', rom_count: 2 },
  ];

  assert.deepEqual(populatedPlatforms(platforms).map(({ slug }) => slug), ['alpha', 'zeta']);
  assert.equal(totalGames(platforms), 5);
  assert.deepEqual(platforms.map(({ slug }) => slug), ['zeta', 'empty', 'alpha']);
});

test('hash routes validate ids and preserve search pagination', () => {
  assert.deepEqual(parseRoute(''), { name: 'platforms' });
  assert.deepEqual(parseRoute('#/platform/42/sony-playstation?q=Ridge+Racer&page=3'), {
    name: 'platform', platformId: 42, slug: 'sony-playstation', query: 'Ridge Racer', page: 3,
  });
  assert.deepEqual(parseRoute('#/game/7'), { name: 'game', gameId: 7 });
  assert.deepEqual(parseRoute('#/search?q=Sonic&page=2'), { name: 'search', query: 'Sonic', page: 2 });
  assert.deepEqual(parseRoute('#/search?q=Sonic&page=999999'), { name: 'search', query: 'Sonic', page: 10_000 });
  assert.deepEqual(parseRoute('#/game/not-a-number'), { name: 'notFound' });
  assert.deepEqual(parseRoute('#/platform/-1/nope'), { name: 'notFound' });
  assert.deepEqual(parseRoute('#/platform/1/%E0%A4%A'), { name: 'notFound' });
});

test('hash and API path builders encode user and server values', () => {
  const platform = { id: 5, slug: 'my platform' };
  assert.equal(platformHash(platform, { query: 'Sonic & Knuckles', page: 2 }), '#/platform/5/my%20platform?q=Sonic+%26+Knuckles&page=2');
  assert.equal(gameHash(9), '#/game/9');
  assert.equal(searchHash('Ridge Racer', 4), '#/search?q=Ridge+Racer&page=4');
  assert.equal(
    contextualSearchHash(
      { name: 'platform', platformId: 5 },
      [{ id: 5, slug: 'my platform' }],
      'Sonic & Knuckles',
    ),
    '#/platform/5/my%20platform?q=Sonic+%26+Knuckles',
  );
  assert.equal(contextualSearchHash({ name: 'platforms' }, [], 'Sonic'), '#/search?q=Sonic');
  assert.equal(
    romListPath({ platformId: 5, query: 'Sonic & Knuckles', page: 2 }),
    '/api/roms?limit=24&offset=24&platform_ids=5&search=Sonic+%26+Knuckles',
  );
  assert.equal(
    romListPath({ limit: 10, sort: 'recent' }),
    '/api/roms?limit=10&offset=0&sort=recent',
  );
});

test('display helpers handle metadata, initials, sizes, and page bounds', () => {
  assert.equal(platformInitials({ display_name: 'Sony PlayStation' }), 'SP');
  assert.equal(platformInitials({ display_name: 'Arcade' }), 'AR');
  assert.equal(formatBytes(1536), '1.5 KiB');
  assert.equal(pageCount(49, 24), 3);
  assert.deepEqual(metadataList({ metadatum: { genres: ['Action', '', null] } }, 'genres'), ['Action']);
});
