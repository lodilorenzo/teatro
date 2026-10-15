import test from 'node:test';
import assert from 'node:assert/strict';

import {
  catalogRange,
  catalogSortFilters,
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
  assert.equal(contextualSearchHash({ name: 'search' }, [], '   '), '#/');
  assert.equal(
    romListPath({ platformId: 5, query: 'Sonic & Knuckles', page: 2 }),
    '/api/roms?limit=24&offset=24&platform_ids=5&search=Sonic+%26+Knuckles',
  );
  assert.equal(
    romListPath({ limit: 10, sort: 'recent' }),
    '/api/roms?limit=10&offset=0&sort=recent',
  );
});

test('metadata filters and sort survive routes, pagination and contextual title searches', () => {
  const filters = { game_mode: 'cooperative', year_min: '1991', year_max: '1991',
    rating_min: '80.5', developer: 'Studio & Co', size_max: '1048576', sort: 'rating', order: 'desc' };
  const hash = searchHash('', 3, filters);
  const route = parseRoute(hash);
  assert.deepEqual(route, { name: 'search', query: '', page: 3, filters });
  const path = new URL(romListPath({ ...route, filters: route.filters }), 'http://test.invalid');
  assert.equal(path.searchParams.get('offset'), '48');
  for (const [key, value] of Object.entries(filters)) assert.equal(path.searchParams.get(key), value);
  const platform = { id: 5, slug: 'genesis' };
  const platformRoute = parseRoute(platformHash(platform, { filters, page: 2 }));
  assert.deepEqual(platformRoute.filters, filters);
  assert.deepEqual(parseRoute(contextualSearchHash(platformRoute, [platform], 'Sonic')).filters, filters);
  assert.equal(parseRoute(contextualSearchHash(platformRoute, [platform], 'Sonic')).page, 1);
  assert.equal(parseRoute(contextualSearchHash(route, [], '')).name, 'search');
  assert.deepEqual(parseRoute('#/search?year=1991&rating=80').filters,
    { year_min: '1991', year_max: '1991', rating_min: '80', rating_max: '80' });
  assert.deepEqual(parseRoute('#/search?genre=Platform&unexpected=ignored&age_rating=PEGI+12').filters, { genre: 'Platform' });
});

test('instant sorting preserves applied filters without mutating them', () => {
  const filters = { game_mode: 'cooperative', rating_min: '80', sort: 'rating', order: 'desc' };
  assert.deepEqual(catalogSortFilters(filters, 'size', 'asc'), { ...filters, sort: 'size', order: 'asc' });
  assert.equal(filters.sort, 'rating');
  assert.equal(filters.order, 'desc');
  assert.deepEqual(catalogSortFilters({ game_mode: 'cooperative' }, 'recent'),
    { game_mode: 'cooperative', sort: 'recent', order: 'desc' });
  assert.deepEqual(catalogSortFilters(), { sort: 'name', order: 'asc' });
});

test('range defaults use complete facet bounds, preserve selections and choose readable size units', () => {
  assert.deepEqual(catalogRange('year', [1991, 1995]).values, [1991, 1995]);
  assert.deepEqual(catalogRange('rating', [0, 90.5], { rating_min: '0' }).values, [0, 91]);
  const selected = catalogRange('rating', [70, 90], { rating_min: '60', rating_max: '95' });
  assert.deepEqual([selected.min, selected.max, ...selected.values], [60, 95, 60, 95]);
  assert.deepEqual(catalogRange('size', [1024, 22528]).values, [1, 22]);
  assert.deepEqual(catalogRange('rating', [80.01, 90.5]).defaults, [81, 91]);
  assert.deepEqual(catalogRange('rating', [0, 100], { rating_min: '80.1', rating_max: '90.5' }).values, [81, 91]);
  assert.deepEqual(catalogRange('rating', [0, 0]).values, [0, 0]);
  assert.deepEqual(catalogRange('size', [1025, 22529]).defaults, [2, 23]);
  assert.deepEqual(catalogRange('size', [1024, 22528], { size_min: '1025', size_max: '22529' }).values, [2, 23]);
  assert.deepEqual(catalogRange('size', [1024 ** 3 + 1, 3 * 1024 ** 3 + 1]).values, [2, 4]);
  assert.equal(catalogRange('size', [1024, 22528]).unit, 'KiB');
  assert.equal(catalogRange('size', [0, 1024 ** 3]).unit, 'GiB');
  assert.equal(catalogRange('year', null).available, false);
  assert.deepEqual(catalogRange('year', null).values, ['', '']);
  assert.deepEqual(catalogRange('year', [1991, 1991]).values, [1991, 1991]);
  assert.match(romListPath({ includeRanges: true }), /include_ranges=true/);
});

test('display helpers handle metadata, initials, sizes, and page bounds', () => {
  assert.equal(platformInitials({ display_name: 'Sony PlayStation' }), 'SP');
  assert.equal(platformInitials({ display_name: 'Arcade' }), 'AR');
  assert.equal(formatBytes(1536), '1.5 KiB');
  assert.equal(pageCount(49, 24), 3);
  assert.deepEqual(metadataList({ metadatum: { genres: ['Action', '', null] } }, 'genres'), ['Action']);
});
