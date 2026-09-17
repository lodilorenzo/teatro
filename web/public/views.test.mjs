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

  const searchOutput = renderSearchPage({
    page, route: { name: 'search', query: 'Sonic', page: 1 }, view: 'list',
  });
  assert.match(searchOutput, /across all platforms/);
  assert.match(searchOutput, /Results for &quot;Sonic&quot;/);
  assert.match(searchOutput, /data-library-view="list" aria-pressed="true"/);
  assert.match(searchOutput, /class="game-grid list-view"/);
  assert.doesNotMatch(searchOutput, /page-search-form/);
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
