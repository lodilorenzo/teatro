import {
  coverPath,
  catalogRange,
  FILTER_KEYS,
  SORT_FIELDS,
  formatCount,
  gameHash,
  platformAccent,
  platformHash,
  platformInitials,
  populatedPlatforms,
  searchHash,
  totalGames,
} from './catalog.js';
import { icon } from './icons.js';
import { renderPagination } from './pagination.js';
import { VERSION_LABEL, attr, html, formatBytes, metadataList, metadataText } from './shared.js';

const COVER_PLACEHOLDER = '/public/game-cover-placeholder.jpg';

export function renderLogin({ error = '', origin = '' } = {}) {
  return `
    <div class="login-page">
      <header class="login-header">
        ${brandLink(false)}
        <a class="text-link" href="/admin">Admin ${icon('arrow')}</a>
      </header>
      <main class="login-main">
        <section class="login-card" aria-labelledby="sign-in-heading">
          <h1 id="sign-in-heading">Sign in to Teatro</h1>
          ${error ? `<div class="alert error-alert" role="alert">${icon('shield')}<span>${html(error)}</span></div>` : ''}
          <form id="login-form" class="login-form">
            <label><span>Username</span><input name="username" autocomplete="username" maxlength="128" required autofocus /></label>
            <label><span>Password</span><input name="password" type="password" autocomplete="current-password" maxlength="1024" required /></label>
            <label class="checkbox"><input name="remember_me" type="checkbox" /> Remember me for 30 days</label>
            <button class="button primary-button wide-button" type="submit">Sign in</button>
          </form>
          <div class="login-security">${icon('shield')}<span>Your password is not saved. Use Remember me only on a trusted device.</span></div>
          <small class="server-address">Server <code>${html(origin)}</code></small>
        </section>
      </main>
      <footer class="login-footer"><span>Teatro</span><span>Self-hosted game library</span></footer>
    </div>`;
}

export function renderShell({ user, platforms, route, content, collection = null }) {
  const gameTotal = totalGames(platforms);
  const platformTotal = populatedPlatforms(platforms).length;
  const searchPlatform = route?.name === 'platform'
    ? platforms.find((platform) => Number(platform.id) === route.platformId)
    : null;
  const searchLabel = collection ? `Search ${collection.name}` : searchPlatform
    ? `Search ${searchPlatform.display_name || searchPlatform.name}`
    : 'Search all games';
  const searchValue = ['platform', 'search'].includes(route?.name) ? route.query : '';
  return `
    <a class="skip-link" href="#main-content">Skip to library</a>
    <div class="storefront-shell">
      <header class="site-header">
        <div class="header-inner">
          ${brandLink(true)}
          <form id="header-search-form" class="header-search" role="search">
            <label class="visually-hidden" for="header-search">${html(searchLabel)}</label>
            ${icon('search')}
            <input id="header-search" name="q" type="search" value="${attr(searchValue)}" maxlength="200" placeholder="${attr(searchLabel)}…" autocomplete="off" />
            <kbd>/</kbd>
          </form>
          <a class="icon-button browse-link" href="#/search" aria-label="Browse &amp; filter" title="Browse &amp; filter">${icon('funnel')}</a>
          <div class="header-session">
            ${user?.role === 'admin' ? `<a class="admin-link" href="/admin" aria-label="Open Teatro admin" title="Admin">${icon('shield')}<span>Admin</span></a>` : ''}
            ${user ? `<div class="user-chip" title="Signed in account">${icon('user')}<span><strong>${html(user?.username || 'Player')}</strong><small>${user?.role === 'admin' ? 'Admin' : 'Read-only'}</small></span></div>
            <button class="icon-button" type="button" data-action="logout" aria-label="Sign out" title="Sign out">${icon('logout')}</button>` : '<a class="text-link" href="/">Sign in</a>'}
          </div>
        </div>
      </header>
      <main id="main-content" class="site-main" tabindex="-1">${content}</main>
      <footer class="site-footer">
        <div><strong>Teatro</strong></div>
        <div>${collection ? `${html(collection.name)} · ${formatCount(collection.rom_count, 'game')}` : `${formatCount(gameTotal, 'game')} · ${formatCount(platformTotal, 'platform')}`}</div>
      </footer>
      <div id="toast-region" class="toast-region" aria-live="polite" aria-atomic="true"></div>
    </div>`;
}

function formatUptime(seconds) {
  const total = Math.max(0, Math.floor(Number(seconds) || 0));
  const days = Math.floor(total / 86400);
  const hours = Math.floor((total % 86400) / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  if (days > 0) return `${days}d ${String(hours).padStart(2, '0')}h`;
  if (hours > 0) return `${hours}h ${String(minutes).padStart(2, '0')}m`;
  return `${minutes}m`;
}

export function renderPlatforms(platforms, recentGames = [], stats = null, collections = null) {
  const visible = populatedPlatforms(platforms);
  const games = totalGames(visible);
  const host = typeof window !== 'undefined' ? window.location.host : '';
  const hasStorage = stats && Number(stats.storage_total_bytes) > 0;
  const storageTotal = hasStorage ? Number(stats.storage_total_bytes) : 1;
  const librarySize = hasStorage ? Number(stats.storage_library_bytes) : 0;
  const otherSize = hasStorage ? Number(stats.storage_other_bytes) : 0;
  const freeSize = hasStorage ? Number(stats.storage_free_bytes) : 0;
  const otherPercent = hasStorage ? (otherSize / storageTotal) * 100 : 0;
  const libraryPercent = hasStorage ? (librarySize / storageTotal) * 100 : 0;
  const storageLabel = hasStorage
    ? `${formatBytes(otherSize)} other, ${formatBytes(librarySize)} Teatro library, ${formatBytes(freeSize)} free`
    : 'Storage unavailable';
  return `
    <section class="home-hero">
      <div class="home-panel">
        <h1>Library</h1>
        <div class="home-panel-duo"><span>games</span><strong>${games.toLocaleString()}</strong></div>
        <div class="home-panel-duo"><span>platforms</span><strong>${visible.length.toLocaleString()}</strong></div>
      </div>
      <div class="home-panel">
        <h2>Storage</h2>
        <svg class="home-storage-bar" viewBox="0 0 100 1" width="100%" height="6" preserveAspectRatio="none" role="img" aria-label="${attr(storageLabel)}">
          <rect class="free" width="100" height="1" fill="#0d0a0b" />
          <rect class="other" width="${attr(otherPercent)}" height="1" fill="#5f596b" />
          <rect class="library" x="${attr(otherPercent)}" width="${attr(libraryPercent)}" height="1" fill="#d65353" />
        </svg>
        <div class="home-panel-kv"><span>library</span><b class="warm">${hasStorage ? html(formatBytes(librarySize)) : '—'}</b></div>
        <div class="home-panel-kv"><span>other</span><b class="other">${hasStorage ? html(formatBytes(otherSize)) : '—'}</b></div>
        <div class="home-panel-kv"><span>free</span><b class="free">${hasStorage ? html(formatBytes(freeSize)) : '—'}</b></div>
      </div>
      <div class="home-panel">
        <h2>Server</h2>
        <div class="home-panel-kv"><span>host</span><b>${host ? html(host) : '—'}</b></div>
        <div class="home-panel-kv"><span>uptime</span><b>${stats ? html(formatUptime(stats.uptime_seconds)) : '—'}</b></div>
        <span class="home-online">online</span>
      </div>
      <button type="button" class="home-surprise" data-action="surprise" aria-label="Open a random game">
        ${icon('shuffle')}<strong>Random game</strong>
      </button>
    </section>
    ${renderRecentGames(recentGames)}
    ${collections ? renderCollections(collections) : ''}
    <section class="platform-section" aria-labelledby="platform-heading">
      <div class="section-heading">
        <h2 id="platform-heading">Platforms</h2>
      </div>
      ${visible.length ? `<div class="platform-grid">${visible.map(renderPlatformCard).join('')}</div>` : renderEmpty(
        'No games yet',
        'Ask an administrator to add games, then refresh the library.',
        'folder',
      )}
    </section>`;
}

export function renderCollections(collections) {
  return `<section class="platform-section" aria-labelledby="collections-heading">
    <div class="section-heading"><h2 id="collections-heading">Collections</h2></div>
    ${collections.length ? `<div class="platform-grid">${collections.map((collection) => `<a class="platform-card" href="/${attr(collection.slug)}">
      <span class="platform-card-emblem">${icon('folder')}</span>
      <span class="platform-card-copy"><strong>${html(collection.name)}</strong><small>${formatCount(collection.rom_count, 'game')}${collection.lan_party_zone ? ' · LAN Party Zone' : ''}</small></span>
      <span class="platform-card-arrow">${icon('arrow')}</span></a>`).join('')}</div>` : '<p>No collections yet. An administrator can create one in Admin.</p>'}
  </section>`;
}

export function renderPlatformPage({ platform, page, route, view = 'grid' }) {
  const query = route.query || '';
  const title = platform.display_name || platform.name;
  return `
    ${renderBreadcrumbs([
      ['Library', '#/'],
      [title, ''],
    ])}
    <section class="platform-hero ${platformAccent(platform)}">
      <div class="platform-emblem">${renderPlatformIcon(platform)}<span></span></div>
      <div><h1>${html(title)}</h1><p>${formatCount(platform.rom_count, 'game')}</p></div>
    </section>
    <section class="games-section" aria-labelledby="games-heading">
      <div class="games-toolbar">
        <h2 id="games-heading"${query ? '' : ' class="visually-hidden"'}>${query ? `Results for &quot;${html(query)}&quot;` : 'Games'}</h2>
        <div class="games-toolbar-controls">${renderSortControls(route)}${renderViewToggle(view)}</div>
      </div>
      ${renderCatalogControls(route, page.ranges)}
      <p class="catalog-count" role="status">${formatCount(page.total, 'game')} found</p>
      ${renderGameResults(page, (targetPage) => platformHash(platform, { query, filters: route.filters, page: targetPage }), {
        emptyTitle: 'No matching games',
        emptyMessage: 'Try another title or remove a filter. Games with missing metadata do not match that filter.',
      }, view, Boolean(query))}
    </section>`;
}

export function renderSearchPage({ page, route, view = 'grid', collection = null }) {
  const query = route.query || '';
  return `
    ${renderBreadcrumbs(collection ? [['Library', '/'], [collection.name, '#/']] : [['Library', '#/'], ['Search', '']])}
    <section class="search-results" aria-labelledby="search-heading">
      <div class="search-results-heading">
        <div><h1 id="search-heading">${query ? `Results for &quot;${html(query)}&quot;` : collection ? html(collection.name) : 'Browse games'}</h1><p role="status">${formatCount(page.total, 'game')}${collection ? collection.lan_party_zone ? ' · LAN Party Zone · No sign-in required' : ' · Collection' : ' across all platforms'}</p></div>
        <div class="games-toolbar-controls">${renderSortControls(route)}${renderViewToggle(view)}</div>
      </div>
      ${renderCatalogControls(route, page.ranges)}
      ${renderGameResults(page, (targetPage) => searchHash(query, targetPage, route.filters), {
        emptyTitle: 'No matching games',
        emptyMessage: 'Try another title or remove a filter. Games with missing metadata do not match that filter.',
      }, view, true)}
    </section>`;
}

function renderCatalogControls(route, ranges = {}) {
  const filters = route.filters || {};
  const count = FILTER_KEYS.filter((key) => filters[key]).length;
  const select = (name, label, options, value) => `<label class="catalog-field"><span>${label}</span><select name="${name}">${options.map(([key, text]) => `<option value="${attr(key)}"${key === value ? ' selected' : ''}>${html(text)}</option>`).join('')}</select></label>`;
  const text = (name, label, placeholder = '') => `<label class="catalog-field"><span>${label}</span><input name="${name}" value="${attr(filters[name] || '')}" maxlength="200" placeholder="${attr(placeholder)}" /></label>`;
  const range = (name, label) => {
    const state = catalogRange(name, ranges[name], filters);
    const step = '1';
    const limit = name === 'year' ? 9999 : name === 'rating' ? 100 : Number.MAX_SAFE_INTEGER / state.divisor;
    const disabled = !state.available || state.min === state.max;
    return `<fieldset data-range="${name}" data-divisor="${state.divisor}" data-unit="${state.unit}" aria-labelledby="${name}-range-label">
      <legend><span id="${name}-range-label">${label}${state.unit ? `, ${state.unit}` : ''}</span><button class="range-reset" type="button" data-range-reset aria-label="Reset ${label.toLowerCase()} filter">Reset</button></legend>
      <div class="dual-range"><span class="range-rail" aria-hidden="true"><span></span></span>${['min', 'max'].map((bound, index) => `<input type="range" min="${state.min}" max="${state.max}" step="${step}" value="${state.values[index] || state.min}" aria-label="${bound === 'min' ? 'Minimum' : 'Maximum'} ${label.toLowerCase()}"${disabled ? ' disabled' : ''} />`).join('')}</div>
      <div class="range-values">${['min', 'max'].map((bound, index) => `${index ? '<span aria-hidden="true">to</span>' : ''}<label class="catalog-field"><span>${bound === 'min' ? 'Minimum' : 'Maximum'}<span class="visually-hidden"> ${label.toLowerCase()}, exact value${state.unit ? `, ${state.unit}` : ''}</span></span><input name="${name}_${bound}" type="number" min="${name === 'year' ? 1 : 0}" max="${limit}" step="${step}" value="${attr(state.values[index])}" data-range-value data-active="${filters[`${name}_${bound}`] != null}" data-default="${attr(state.defaults[index])}"${!state.available && state.values[index] === '' ? ' disabled' : ''} /></label>`).join('')}</div>
      ${!state.available ? '<small class="range-empty">No known values with other filters</small>' : ''}
    </fieldset>`;
  };
  return `<form id="catalog-controls" class="catalog-controls">
    <details class="catalog-filters"${count ? ' open' : ''}>
      <summary>Filter games${count ? ` · ${count} active` : ''}</summary>
      <div class="numeric-filters">
        ${range('year', 'Release year')}
        ${range('rating', 'Rating / 100')}
        ${range('size', 'File size')}
      </div>
      <div class="filter-fields">
        ${select('game_mode', 'Play mode', [['', 'Any play mode'], ['multiplayer', 'Multiplayer + co-op'], ['cooperative', 'Cooperative'], ['Single player', 'Single player'], ['Split screen', 'Split screen']], filters.game_mode || '')}
        ${text('genre', 'Genre', 'e.g. Platform')}
        ${text('developer', 'Developer')}
        ${text('publisher', 'Publisher')}
        ${text('region', 'Region', 'e.g. USA')}
        ${text('language', 'Language', 'e.g. English')}
        ${text('tag', 'Tag')}
      </div>
    </details>
  </form>`;
}

export function renderGameDetail(game, collection = null) {
  const genres = metadataList(game, 'genres');
  const developers = metadataList(game, 'developers');
  const publishers = metadataList(game, 'publishers');
  const releaseYear = metadataText(game, 'release_year');
  const files = Array.isArray(game.files) ? game.files : [];
  const totalSize = files.reduce((total, file) => total + Number(file.file_size_bytes || 0), 0);
  const platform = {
    id: game.platform_id,
    slug: game.platform_slug,
    display_name: game.platform_display_name,
  };
  const largeCover = coverPath(game, 'large');
  const coverData = largeCover ? ` data-cover-path="${attr(largeCover)}"` : '';
  return `
    ${renderBreadcrumbs([
      [collection?.name || 'Library', '#/'],
      [game.platform_display_name, collection ? '' : platformHash(platform)],
      [game.name, ''],
    ])}
    <article class="game-detail">
      <aside class="detail-cover-column">
        <div class="detail-cover-wrap">
          <img class="detail-cover" src="${COVER_PLACEHOLDER}" alt="${attr(largeCover ? `Cover for ${game.name}` : `No cover available for ${game.name}`)}"${coverData} />
          <span class="cover-corner top-left"></span><span class="cover-corner bottom-right"></span>
        </div>
        <a class="back-link" href="${collection ? '#/' : attr(platformHash(platform))}">${icon('back')} Back to ${html(collection?.name || game.platform_display_name)}</a>
      </aside>
      <div class="detail-content">
        <div class="detail-title-block">
          <a class="platform-pill" href="${collection ? '#/' : attr(platformHash(platform))}"><span class="detail-platform-icon" aria-hidden="true">${renderPlatformIcon(platform)}</span>${html(game.platform_display_name)}</a>
          <h1>${html(game.name)}</h1>
          <div class="detail-tags">
            ${releaseYear ? `<span>${html(releaseYear)}</span>` : ''}
            ${(game.regions || []).map((region) => `<span>${html(region)}</span>`).join('')}
            ${genres.slice(0, 4).map((genre) => `<span>${html(genre)}</span>`).join('')}
          </div>
        </div>
        <div class="detail-summary">
          <p>${html(game.summary || 'No description available.')}</p>
        </div>
        ${renderDetailFacts({ game, releaseYear, developers, publishers, genres })}
        <section class="download-panel" aria-labelledby="download-heading">
          <div class="download-panel-heading">
            <div><h2 id="download-heading">Files</h2><p>${formatCount(files.length, 'file')} · ${formatBytes(totalSize)}</p></div>
            ${renderPrimaryDownload(game, files, totalSize)}
          </div>
          ${files.length ? `<div class="download-file-list">${files.map((file) => renderDownloadFile(game, file)).join('')}</div>` : '<div class="alert">No downloadable files are registered for this game.</div>'}
          ${files.length > 1 ? '<p class="download-help">Download ZIP includes all files. You can also download files individually.</p>' : ''}
        </section>
      </div>
    </article>`;
}

export function renderLoading(label = 'Loading library…') {
  return `<div class="loading-view" role="status" aria-live="polite">
    <div class="loading-indicator" aria-hidden="true"></div><h1>${html(label)}</h1>
    <div class="skeleton-grid">${Array.from({ length: 8 }, () => '<span></span>').join('')}</div>
  </div>`;
}

export function renderError(error) {
  return `<section class="error-view" role="alert">
    <h1>Could not load the library</h1>
    <p>${html(error?.message || error || 'Teatro could not load this part of the library.')}</p>
    <div class="error-actions"><button class="button primary-button" type="button" data-action="retry">Try again</button><a class="button subtle-button" href="#/">Back to platforms</a></div>
  </section>`;
}

export function renderNotFound() {
  return `<section class="error-view">
    <h1>Page not found</h1>
    <p>This page may have moved or been deleted.</p><a class="button primary-button" href="#/">Browse platforms ${icon('arrow')}</a>
  </section>`;
}

function brandLink(linked) {
  const content = `<img class="brand-symbol" src="/public/favicon.svg?icon=teatro-controller" alt="" /><span class="brand-name"><strong>Teatro</strong><small>Game library</small><span class="version-label" title="Beta / release-hardening">${html(VERSION_LABEL)}</span></span>`;
  return linked ? `<a class="brand" href="#/" aria-label="Teatro library home">${content}</a>` : `<div class="brand">${content}</div>`;
}

function renderPlatformIcon(platform) {
  const slug = String(platform?.slug || '');
  if (!slug) return html(platformInitials(platform));
  return `<img class="platform-icon" src="${attr(`/public/platform-icons/${encodeURIComponent(slug)}.png`)}" alt="" loading="lazy" />`;
}

function renderPlatformCard(platform) {
  const title = platform.display_name || platform.name;
  return `<a class="platform-card ${platformAccent(platform)}" href="${attr(platformHash(platform))}">
    <span class="platform-card-emblem">${renderPlatformIcon(platform)}<i></i></span>
    <span class="platform-card-copy"><strong>${html(title)}</strong><small>${formatCount(platform.rom_count, 'game')}</small></span>
    <span class="platform-card-arrow">${icon('arrow')}</span>
  </a>`;
}

function renderRecentGames(games) {
  const items = (games || []).slice(0, 10);
  if (!items.length) return '';
  const pages = Math.ceil(items.length / 5);
  return `
    <section class="recent-games-section" aria-labelledby="recent-games-heading">
      <div class="section-heading">
        <h2 id="recent-games-heading">Recently added</h2>
        <div class="carousel-controls">
          <button class="icon-button" type="button" data-action="recent-previous" aria-controls="recent-games-carousel" aria-label="Show previous games" disabled>${icon('back')}</button>
          <span data-recent-status role="status" aria-live="polite">Page 1 of ${pages}</span>
          <button class="icon-button" type="button" data-action="recent-next" aria-controls="recent-games-carousel" aria-label="Show next games" ${pages === 1 ? 'disabled' : ''}>${icon('arrow')}</button>
        </div>
      </div>
      <div id="recent-games-carousel" class="recent-games-grid" data-recent-carousel role="group" aria-roledescription="carousel" aria-label="Recently added games">
        ${items.map((game, index) => renderGameCard(game, index)).join('')}
      </div>
    </section>`;
}

function renderGameResults(page, hrefForPage, empty, view, showPlatformIcon = false) {
  const items = page?.items || [];
  if (!items.length) return renderEmpty(empty.emptyTitle, empty.emptyMessage, 'search');
  return `<div class="game-grid${view === 'list' ? ' list-view' : ''}">${items.map((game) => renderGameCard(game, null, showPlatformIcon)).join('')}</div>${renderPagination(page, hrefForPage)}`;
}

function renderViewToggle(view) {
  return `<div class="view-toggle" role="group" aria-label="Library view">
    <button class="button subtle-button" type="button" data-library-view="grid" aria-pressed="${view !== 'list'}">Grid</button>
    <button class="button subtle-button" type="button" data-library-view="list" aria-pressed="${view === 'list'}">List</button>
  </div>`;
}

function renderSortControls(route) {
  const sort = route.filters?.sort || 'name';
  const order = route.filters?.order || (sort === 'recent' ? 'desc' : 'asc');
  const clear = { sort, order };
  const href = route.name === 'platform' || route.platformId
    ? platformHash({ id: route.platformId, slug: route.slug || '' }, { query: route.query, filters: clear })
    : searchHash(route.query, 1, clear);
  return `<div class="catalog-sort">
    ${FILTER_KEYS.some((key) => route.filters?.[key]) ? `<a class="button subtle-button catalog-clear" href="${attr(href)}" aria-label="Clear filters" title="Clear filters">${icon('funnel-x')}</a>` : ''}
    <label><span class="visually-hidden">Sort by</span><select name="sort">${SORT_FIELDS.map(([value, label]) => `<option value="${value}"${value === sort ? ' selected' : ''}>${html(label)}</option>`).join('')}</select></label>
    <div class="sort-direction" role="group" aria-label="Sort direction">${[['asc', 'Ascending'], ['desc', 'Descending']].map(([value, label]) => `<button class="button subtle-button" type="button" data-sort-order="${value}" aria-pressed="${value === order}" aria-label="${label} order" title="${label === 'Ascending' ? 'Sort lowest to highest, A to Z' : 'Sort highest to lowest, Z to A'}">${icon(value === 'asc' ? 'arrow-down-narrow-wide' : 'arrow-down-wide-narrow', `sort-${value}`)}</button>`).join('')}</div>
  </div>`;
}

function renderGameCard(game, recentIndex = null, showPlatformIcon = false) {
  const largeCover = coverPath(game, 'large');
  const coverData = largeCover ? ` data-cover-path="${attr(largeCover)}"` : '';
  const year = metadataText(game, 'release_year');
  const genres = metadataList(game, 'genres');
  const rating = gameRating(game);
  const facts = metadataList(game, 'game_modes').join(' · ');
  const isRecent = Number.isInteger(recentIndex);
  const carouselAttributes = isRecent
    ? ` data-recent-game="${recentIndex}"${recentIndex >= 5 ? ' hidden' : ''}`
    : '';
  const platformIcon = isRecent || showPlatformIcon ? `<span class="recent-game-platform-icon" aria-hidden="true">${renderPlatformIcon({
    slug: game.platform_slug,
    display_name: game.platform_display_name,
  })}</span>` : '';
  return `<article class="game-card"${carouselAttributes}>
    <a class="game-cover-link" href="${attr(gameHash(game.id))}" aria-label="View ${attr(game.name)}">
      <img class="game-cover" src="${COVER_PLACEHOLDER}" alt="" loading="lazy"${coverData} />
      ${platformIcon}
      <span class="game-card-action">View game ${icon('arrow')}</span>
    </a>
    <div class="game-card-body">
      <div class="game-card-kicker"><span>${html(game.platform_display_name)}</span>${year || rating ? `<div class="game-card-stats">${year ? `<span>${html(year)}</span>` : ''}${rating ? `<span class="game-card-rating" title="Rating">${html(rating)}</span>` : ''}</div>` : ''}</div>
      <h3><a href="${attr(gameHash(game.id))}">${html(game.name)}</a></h3>
      <div class="game-card-meta"><span>${genres.length ? html(genres[0]) : 'Game'}</span><span>${formatBytes(game.fs_size_bytes || 0)}</span></div>
      ${facts ? `<p class="game-card-facts">${html(facts)}</p>` : ''}
    </div>
  </article>`;
}

function renderBreadcrumbs(items) {
  return `<nav class="breadcrumbs" aria-label="Breadcrumb">${items.map(([label, href], index) => {
    const content = href ? `<a href="${attr(href)}">${html(label)}</a>` : `<span aria-current="page">${html(label)}</span>`;
    return `${index ? icon('chevron') : ''}${content}`;
  }).join('')}</nav>`;
}

function renderDetailFacts({ game, releaseYear, developers, publishers, genres }) {
  const rows = [
    ['Platform', game.platform_display_name],
    ['Released', releaseYear],
    ['Developer', developers.join(', ')],
    ['Publisher', publishers.join(', ')],
    ['Genres', genres.join(', ')],
    ['Rating', gameRating(game)],
    ['Play modes', metadataList(game, 'game_modes').join(', ')],
    ['Tags', metadataList(game, 'tags').join(', ')],
    ['Languages', metadataList(game, 'languages').join(', ')],
    ['Regions', (game.regions || []).join(', ')],
  ].filter(([, value]) => value);
  if (!rows.length) return '';
  return `<dl class="detail-facts">${rows.map(([label, value]) => `<div><dt>${html(label)}</dt><dd>${html(value)}</dd></div>`).join('')}</dl>`;
}

function gameRating(game) {
  const metadata = game?.metadatum || {};
  const value = [metadata.total_rating, metadata.rating, metadata.aggregated_rating]
    .find((value) => typeof value === 'number' && Number.isFinite(value));
  return value === undefined ? '' : `${Math.round(value * 10) / 10} / 100`;
}

function renderPrimaryDownload(game, files, totalSize) {
  if (!files.length) return '<button class="button primary-button" type="button" disabled>Unavailable</button>';
  const label = `${icon('download')} ${files.length > 1 ? 'Download ZIP' : 'Download'} · ${formatBytes(totalSize)}`;
  if (files.length === 1) {
    return `<button class="button primary-button download-primary" type="button" data-download-file="${files[0].id}" data-rom-id="${game.id}">${label}</button>`;
  }
  return `<button class="button primary-button download-primary" type="button" data-download-archive data-rom-id="${game.id}">${label}</button>`;
}

function renderDownloadFile(game, file) {
  const role = String(file.role || 'file').replaceAll('_', ' ');
  return `<div class="download-file">
    <span class="download-file-icon">${icon('file')}</span>
    <span class="download-file-name"><strong>${html(file.file_name)}</strong><small>${html(role)}${file.launchable ? ' · launch file' : ''}</small></span>
    <span class="download-file-size">${formatBytes(file.file_size_bytes)}</span>
    <button class="button file-download-button" type="button" data-download-file="${file.id}" data-rom-id="${game.id}" aria-label="Download ${attr(file.file_name)}">${icon('download')}<span>Download</span></button>
  </div>`;
}

function renderEmpty(title, message, iconName) {
  return `<div class="empty-state"><span class="empty-icon">${icon(iconName)}</span><h3>${html(title)}</h3><p>${html(message)}</p></div>`;
}
