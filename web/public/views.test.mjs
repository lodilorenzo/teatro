import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  renderError,
  renderGameDetail,
  renderLoading,
  renderLogin,
  renderNotFound,
  renderPlatformPage,
  renderPlatforms,
  renderSearchPage,
  renderShell,
} from './views.js';

const populated = { id: 1, slug: 'genesis', display_name: 'Sega Genesis', rom_count: 2 };
const empty = { id: 2, slug: 'nes', display_name: 'Nintendo Entertainment System', rom_count: 0 };
const game = {
  id: 7,
  name: 'Sonic <The Hedgehog>',
  platform_id: 1,
  platform_slug: 'genesis',
  platform_display_name: 'Sega Genesis',
  regions: ['USA'],
  metadatum: { release_year: 1991, genres: ['Platform'], developers: ['Sonic Team'] },
  summary: 'Fast & bright.',
  fs_size_bytes: 1536,
  path_cover_small: 'covers/sonic-small.jpg',
  path_cover_large: 'covers/sonic-large.jpg',
  files: [{ id: 11, file_name: 'sonic.bin', file_size_bytes: 1536, role: 'content', launchable: true }],
};

test('login is public-facing, safe, and links to the admin console', () => {
  const output = renderLogin({ error: '<bad password>', origin: 'http://10.0.0.2:4440' });
  assert.match(output, /Sign in to Teatro/);
  assert.match(output, /id="login-form"/);
  assert.match(output, /<label class="checkbox"><input name="remember_me" type="checkbox" \/> Remember me for 30 days<\/label>/);
  assert.doesNotMatch(output, /name="remember_me"[^>]*checked/);
  assert.match(output, /href="\/admin"/);
  assert.match(output, /&lt;bad password&gt;/);
  assert.doesNotMatch(output, /<bad password>/);
});

test('platform landing omits empty platforms and shows five of the ten latest games', () => {
  const recentGames = Array.from({ length: 10 }, (_, index) => ({
    ...game, id: index + 1, name: `Recent game ${index + 1}`,
  }));
  const output = renderPlatforms([empty, populated], recentGames);
  assert.match(output, /class="home-hero"/);
  assert.match(output, /<h1>Library<\/h1>/);
  assert.match(output, /data-action="surprise"[^>]*>[\s\S]*Random game/);
  assert.doesNotMatch(output, /home-search-form/);
  assert.match(output, /Sega Genesis/);
  assert.match(output, /#\/platform\/1\/genesis/);
  assert.match(output, /src="\/public\/platform-icons\/genesis\.png"/);
  assert.doesNotMatch(output, /Nintendo Entertainment System/);
  assert.match(output, /2 games/);
  assert.match(output, /id="recent-games-heading">Recently added/);
  assert.equal((output.match(/data-recent-game=/g) || []).length, 10);
  assert.equal((output.match(/class="recent-game-platform-icon"/g) || []).length, 10);
  assert.match(output, /class="recent-game-platform-icon"[^>]*><img class="platform-icon" src="\/public\/platform-icons\/genesis\.png"/);
  assert.equal((output.match(/data-recent-game="[5-9]" hidden/g) || []).length, 5);
  assert.match(output, /data-action="recent-previous"[^>]*disabled/);
  assert.match(output, /data-action="recent-next"/);
  assert.match(output, /Page 1 of 2/);
});

test('platform landing renders storage and uptime when stats are provided', () => {
  const output = renderPlatforms([populated], [], {
    storage_used_bytes: 612 * 1024 ** 3,
    storage_library_bytes: 200 * 1024 ** 3,
    storage_other_bytes: 412 * 1024 ** 3,
    storage_free_bytes: 412 * 1024 ** 3,
    storage_total_bytes: 1024 * 1024 ** 3,
    uptime_seconds: 18 * 86400 + 4 * 3600,
  });
  assert.match(output, /<h2>Storage<\/h2>/);
  assert.match(output, /viewBox="0 0 100 1" width="100%" height="6"/);
  assert.match(output, /class="other" width="40\.234375"[^>]*fill="#5f596b"/);
  assert.match(output, /class="library" x="40\.234375" width="19\.53125"[^>]*fill="#d65353"/);
  assert.match(output, /aria-label="[^"]*other, [^"]*Teatro library, [^"]*free"/);
  assert.ok(output.indexOf('class="other"') < output.indexOf('class="library"'));
  assert.doesNotMatch(output, /style="width:/);
  assert.match(output, /uptime<\/span><b>18d 04h<\/b>/);
});

test('platform landing shows placeholders when stats are unavailable', () => {
  const output = renderPlatforms([populated], []);
  assert.match(output, /<svg class="home-storage-bar" viewBox="0 0 100 1" width="100%" height="6"[^>]*aria-label="Storage unavailable">/);
  assert.match(output, /library<\/span><b class="warm">—<\/b>/);
  assert.match(output, /other<\/span><b class="other">—<\/b>/);
  assert.match(output, /uptime<\/span><b>—<\/b>/);
});

test('platform cards retain initials when no slug is provided', () => {
  const output = renderPlatforms([{
    id: 3, display_name: 'Example Platform', rom_count: 1,
  }]);
  assert.match(output, /platform-card-emblem">EP<i>/);
  assert.doesNotMatch(output, /platform-icons\//);
});

test('platform and search pages render game cards with pagination-safe routes', () => {
  const page = { items: [game], total: 30, limit: 24, offset: 0 };
  const platformOutput = renderPlatformPage({ platform: populated, page, route: { query: '', page: 1 } });
  assert.match(platformOutput, /Sonic &lt;The Hedgehog&gt;/);
  assert.match(platformOutput, /src="\/public\/platform-icons\/genesis\.png"/);
  assert.match(platformOutput, /data-cover-path="covers\/sonic-large.jpg"/);
  assert.doesNotMatch(platformOutput, /data-cover-path="covers\/sonic-small.jpg"/);
  assert.match(platformOutput, /#\/game\/7/);
  assert.match(platformOutput, /page=2/);
  assert.match(platformOutput, /data-library-view="grid" aria-pressed="true"/);
  assert.doesNotMatch(platformOutput, /platform-search-form|recent-game-platform-icon|game-grid list-view/);

  const platformSearchOutput = renderPlatformPage({
    platform: populated, page, route: { query: 'Sonic', page: 1 },
  });
  assert.match(platformSearchOutput, /class="recent-game-platform-icon"[^>]*><img class="platform-icon" src="\/public\/platform-icons\/genesis\.png"/);

  const searchOutput = renderSearchPage({
    page, route: { name: 'search', query: 'Sonic', page: 1 }, view: 'list',
  });
  assert.match(searchOutput, /across all platforms/);
  assert.match(searchOutput, /Results for &quot;Sonic&quot;/);
  assert.match(searchOutput, /data-library-view="list" aria-pressed="true"/);
  assert.match(searchOutput, /class="game-grid list-view"/);
  assert.match(searchOutput, /class="recent-game-platform-icon"[^>]*><img class="platform-icon" src="\/public\/platform-icons\/genesis\.png"/);
  assert.doesNotMatch(searchOutput, /page-search-form/);
});

test('public grid and list cards place rating below the year without duplicating it in play modes', () => {
  for (const view of ['grid', 'list']) {
    for (const render of [
      (page) => renderSearchPage({ page, route: { query: '' }, view }),
      (page) => renderPlatformPage({ platform: populated, page, route: { query: '' }, view }),
    ]) {
      const output = render({ items: [{ ...game, metadatum: {
        ...game.metadatum, total_rating: 83.5, game_modes: ['Co-operative'],
      } }], total: 1, limit: 24, offset: 0 });
      assert.match(output, /class="game-card-stats"><span>1991<\/span><span class="game-card-rating" title="Rating">83\.5 \/ 100<\/span>/);
      assert.equal((output.match(/83\.5 \/ 100/g) || []).length, 1);
      assert.match(output, /class="game-card-facts">Co-operative<\/p>/);
      for (const metadatum of [{ total_rating: 0 }, { rating: 70 }, { release_year: 1991 }]) {
        const markup = render({ items: [{ ...game, metadatum }], total: 1, limit: 24, offset: 0 });
        if (metadatum.release_year) assert.doesNotMatch(markup, /class="game-card-rating"/);
        else assert.match(markup, new RegExp(`class="game-card-rating" title="Rating">${metadatum.total_rating ?? metadatum.rating} / 100`));
      }
    }
  }
});

test('filter-only browsing exposes labeled controls, applied values and preserved pagination', () => {
  const route = { name: 'search', query: '', page: 1,
    filters: { game_mode: 'cooperative', genre: '<unsafe>', rating_min: '0', size_max: '1048576', sort: 'rating', order: 'desc' } };
  const output = renderSearchPage({ page: { items: [game], total: 30, limit: 24, offset: 0,
    ranges: { year: [1990, 2000], rating: [0, 90.5], size: [0, 1048576] } }, route });
  assert.match(output, /Browse games/);
  assert.match(output, /class="catalog-filters" open/);
  assert.match(output, /name="game_mode"[\s\S]*value="cooperative" selected/);
  assert.match(output, /value="rating" selected/);
  assert.match(output, /data-sort-order="desc" aria-pressed="true" aria-label="Descending order"/);
  assert.match(output, /data-sort-order="asc" aria-pressed="false" aria-label="Ascending order"/);
  assert.match(output, /class="icon sort-asc"/);
  assert.match(output, /class="icon sort-desc"/);
  assert.ok(output.indexOf('class="catalog-sort"') < output.indexOf('data-library-view="grid"'));
  assert.ok(output.indexOf('data-library-view="grid"') < output.indexOf('data-library-view="list"'));
  assert.ok(output.indexOf('class="catalog-sort"') < output.indexOf('id="catalog-controls"'));
  assert.doesNotMatch(output, /Apply sort|Apply filters|filter-help|<select name="order"/);
  assert.ok(output.indexOf('class="button subtle-button catalog-clear"') < output.indexOf('name="sort"'));
  assert.ok(output.indexOf('name="sort"') < output.indexOf('data-sort-order="asc"'));
  assert.ok(output.indexOf('data-sort-order="desc"') < output.indexOf('data-library-view="grid"'));
  assert.match(output, /class="catalog-field"><span>Genre<\/span><input name="genre"/);
  assert.match(output, /class="catalog-field"><span>Minimum<span class="visually-hidden"> release year/);
  assert.match(output, /aria-label="Clear filters" title="Clear filters"/);
  assert.match(output, /name="rating_min"[^>]*value="0"/);
  assert.match(output, /name="size_max"[^>]*value="1"/);
  assert.match(output, /value="&lt;unsafe&gt;"/);
  assert.doesNotMatch(output, /<unsafe>/);
  assert.match(output, /game_mode=cooperative[^"\s]*page=2/);
  assert.doesNotMatch(output, /Ranges use games matching the other applied filters/);
  assert.equal((output.match(/type="range"/g) || []).length, 6);
  assert.match(output, /data-range="year"[\s\S]*type="range" min="1990" max="2000"/);
  assert.match(output, /aria-label="Minimum release year"/);
  assert.match(output, /data-range-value data-active="false" data-default="1990"/);
  const emptyRanges = renderSearchPage({ page: { items: [], total: 0 }, route: { query: '' } });
  assert.match(emptyRanges, /No known values with other filters/);
  assert.doesNotMatch(emptyRanges, /catalog-clear|Clear filters/);
  const sortOnly = renderSearchPage({ page: { items: [], total: 0 }, route: { query: '', filters: { sort: 'rating', order: 'desc' } } });
  assert.doesNotMatch(sortOnly, /catalog-clear|Clear filters/);
  assert.equal((emptyRanges.match(/type="range"[^>]* disabled/g) || []).length, 6);
  const detail = renderGameDetail({ ...game, metadatum: { total_rating: 0, romm: { age_ratings: ['PEGI 12'], game_modes: ['Co-operative'], tags: ['Family'] } } });
  assert.match(detail, /0 \/ 100/);
  assert.doesNotMatch(detail, /Age classifications|PEGI 12/);
  assert.doesNotMatch(output, /age_rating|Age classification|value="age"/);
  assert.match(detail, /Play modes<\/dt><dd>Co-operative/);
});

test('game detail renders metadata and authenticated file download controls', () => {
  const singleFileOutput = renderGameDetail(game);
  assert.match(singleFileOutput, /Download · 1\.5 KiB/);
  assert.doesNotMatch(singleFileOutput, /data-download-archive/);

  const output = renderGameDetail({
    ...game,
    files: [
      ...game.files,
      { id: 12, file_name: 'track.bin', file_size_bytes: 2048, role: 'track', launchable: false },
    ],
  });
  assert.doesNotMatch(output, /About this game|Ready when you are/);
  assert.match(output, /Fast &amp; bright\./);
  assert.match(output, /class="platform-pill"[^>]*><span class="detail-platform-icon"[^>]*><img class="platform-icon" src="\/public\/platform-icons\/genesis\.png"/);
  assert.match(output, /data-download-archive/);
  assert.match(output, /data-download-file="11"/);
  assert.match(output, /data-download-file="12"/);
  assert.match(output, /Download ZIP · 3\.5 KiB/);
  assert.match(output, /Download ZIP includes all files/);
  assert.doesNotMatch(singleFileOutput, /Download ZIP includes/);
  assert.match(output, /data-cover-path="covers\/sonic-large.jpg"/);
  assert.doesNotMatch(output, /Sonic <The Hedgehog>/);
});

test('shell includes one contextual search, session controls, counts, and admin link only for admins', () => {
  const admin = renderShell({
    user: { username: 'captain', role: 'admin' }, platforms: [populated], route: { name: 'platforms' }, content: '<section>Body</section>',
  });
  assert.doesNotMatch(admin, /main-nav|<span>Platforms<\/span>/);
  assert.match(admin, /class="brand" href="#\/"/);
  assert.match(admin, /id="header-search-form"/);
  assert.equal((admin.match(/role="search"/g) || []).length, 1);
  assert.match(admin, /placeholder="Search all games…"/);
  assert.match(admin, /data-action="logout"/);
  const browse = admin.match(/<a class="icon-button browse-link"[^>]*>[\s\S]*?<\/a>/)?.[0];
  assert.ok(browse);
  assert.match(browse, /href="#\/search" aria-label="Browse &amp; filter" title="Browse &amp; filter"/);
  assert.match(browse, /<svg class="icon" aria-hidden="true"/);
  assert.doesNotMatch(browse, />Browse &amp; filter</);
  assert.match(admin, /class="admin-link" href="\/admin"/);
  assert.match(admin, /aria-label="Open Teatro admin"/);
  assert.match(admin, />Admin<\/span>/);
  assert.match(admin, /2 games/);

  const reader = renderShell({
    user: { username: 'reader', role: 'readonly' }, platforms: [populated], route: { name: 'platforms' }, content: '',
  });
  assert.doesNotMatch(reader, /class="admin-link"/);

  const platform = renderShell({
    user: { username: 'reader', role: 'readonly' },
    platforms: [populated],
    route: { name: 'platform', platformId: 1, query: 'Sonic' },
    content: '',
  });
  assert.match(platform, /value="Sonic"/);
  assert.match(platform, /placeholder="Search Sega Genesis…"/);
});

test('pages keep one main heading without decorative introductions', () => {
  const page = { items: [game], total: 1, limit: 24, offset: 0 };
  const views = [
    renderLogin(), renderPlatforms([populated], [game]), renderGameDetail(game),
    renderPlatformPage({ platform: populated, page, route: { query: '' } }),
    renderPlatformPage({ platform: populated, page, route: { query: '<title>' } }),
    renderSearchPage({ page, route: { query: '' } }),
    renderSearchPage({ page, route: { query: '<title>' } }),
    renderLoading(), renderError('<offline>'), renderNotFound(),
  ];
  for (const markup of views) {
    assert.equal((markup.match(/<h1[ >]/g) || []).length, 1);
    assert.doesNotMatch(markup, /class="eyebrow"|<title>|<offline>/);
  }
  const css = readFileSync(new URL('./app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /[✦◆♦◇]|rotate\(45deg\)/);
  assert.match(css, /--brand: #d65353/);
});
