export const PAGE_SIZE = 24;
const MAX_PAGE = 10_000;

export function populatedPlatforms(platforms) {
  return [...(platforms || [])]
    .filter((platform) => Number(platform.rom_count) > 0)
    .sort((left, right) => String(left.display_name || left.name || '').localeCompare(
      String(right.display_name || right.name || ''),
      undefined,
      { sensitivity: 'base' },
    ));
}

export function totalGames(platforms) {
  return populatedPlatforms(platforms)
    .reduce((total, platform) => total + Number(platform.rom_count || 0), 0);
}

export function platformInitials(platform) {
  const words = String(platform?.display_name || platform?.name || platform?.slug || '?')
    .replace(/[^\p{L}\p{N}]+/gu, ' ')
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  if (!words.length) return '?';
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return `${words[0][0]}${words[words.length - 1][0]}`.toUpperCase();
}

export function platformAccent(platform) {
  const value = String(platform?.slug || platform?.id || 'teatro');
  let hash = 0;
  for (const character of value) hash = ((hash * 31) + character.codePointAt(0)) >>> 0;
  return `accent-${hash % 6}`;
}

export function parseRoute(hash) {
  const raw = String(hash || '').replace(/^#/, '') || '/';
  const url = new URL(raw.startsWith('/') ? raw : `/${raw}`, 'http://teatro.invalid');
  let parts;
  try {
    parts = url.pathname.split('/').filter(Boolean).map(decodeURIComponent);
  } catch (_) {
    return { name: 'notFound' };
  }
  const page = parsePage(url.searchParams.get('page'));
  const query = String(url.searchParams.get('q') || '').trim().slice(0, 200);

  if (parts.length === 0) return { name: 'platforms' };
  if (parts[0] === 'search' && parts.length === 1) return { name: 'search', query, page };
  if (parts[0] === 'platform' && parts.length >= 2) {
    const platformId = parsePositiveInteger(parts[1]);
    if (platformId) {
      return {
        name: 'platform',
        platformId,
        slug: String(parts[2] || ''),
        query,
        page,
      };
    }
  }
  if (parts[0] === 'game' && parts.length === 2) {
    const gameId = parsePositiveInteger(parts[1]);
    if (gameId) return { name: 'game', gameId };
  }
  return { name: 'notFound' };
}

export function platformHash(platform, options = {}) {
  const params = new URLSearchParams();
  const query = String(options.query || '').trim();
  const page = parsePage(options.page);
  if (query) params.set('q', query);
  if (page > 1) params.set('page', String(page));
  const queryString = params.toString();
  const suffix = queryString ? `?${queryString}` : '';
  return `#/platform/${encodeURIComponent(platform.id)}/${encodeURIComponent(platform.slug || '')}${suffix}`;
}

export function gameHash(gameId) {
  return `#/game/${encodeURIComponent(gameId)}`;
}

export function searchHash(query, page = 1) {
  const params = new URLSearchParams();
  const normalized = String(query || '').trim();
  if (normalized) params.set('q', normalized);
  const parsedPage = parsePage(page);
  if (parsedPage > 1) params.set('page', String(parsedPage));
  const queryString = params.toString();
  return `#/search${queryString ? `?${queryString}` : ''}`;
}

export function contextualSearchHash(route, platforms, query) {
  const platform = route?.name === 'platform'
    ? (platforms || []).find((item) => Number(item.id) === route.platformId)
    : null;
  return platform ? platformHash(platform, { query }) : searchHash(query);
}

export function romListPath({
  platformId = null, query = '', page = 1, limit = PAGE_SIZE, sort = '',
} = {}) {
  const safeLimit = Math.max(1, Math.min(100, Number.parseInt(limit, 10) || PAGE_SIZE));
  const safePage = parsePage(page);
  const params = new URLSearchParams({
    limit: String(safeLimit),
    offset: String((safePage - 1) * safeLimit),
  });
  if (platformId) params.set('platform_ids', String(platformId));
  if (sort === 'recent') params.set('sort', 'recent');
  const normalizedQuery = String(query || '').trim();
  if (normalizedQuery) params.set('search', normalizedQuery);
  return `/api/roms?${params}`;
}

export function coverPath(game, size = 'small') {
  if (!game) return '';
  return size === 'large'
    ? game.path_cover_large || game.path_cover_small || ''
    : game.path_cover_small || game.path_cover_large || '';
}

export function formatCount(value, singular, plural = `${singular}s`) {
  const count = Number(value || 0);
  return `${count.toLocaleString()} ${count === 1 ? singular : plural}`;
}

export function pageCount(total, limit = PAGE_SIZE) {
  return Math.max(1, Math.ceil(Math.max(0, Number(total || 0)) / limit));
}

function parsePage(value) {
  const parsed = Number.parseInt(value, 10);
  return Number.isSafeInteger(parsed) && parsed > 0 ? Math.min(parsed, MAX_PAGE) : 1;
}

function parsePositiveInteger(value) {
  if (!/^\d+$/.test(String(value || ''))) return null;
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : null;
}
