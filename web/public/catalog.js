export const PAGE_SIZE = 24;
const MAX_PAGE = 10_000;
export const FILTER_KEYS = ['game_mode', 'year_min', 'year_max', 'rating_min', 'rating_max',
  'genre', 'developer', 'publisher', 'region', 'language', 'tag', 'size_min', 'size_max'];
export const SORT_FIELDS = [['name', 'Title'], ['recent', 'Date added'], ['year', 'Release year'],
  ['rating', 'Rating'], ['size', 'Total file size']];

export function catalogFilters(values = {}) {
  const filters = {};
  for (const key of [...FILTER_KEYS, 'sort', 'order']) {
    const value = String(values[key] ?? '').trim().slice(0, 200);
    if (value) filters[key] = value;
  }
  return filters;
}

export function catalogSortFilters(filters = {}, sort = filters.sort || 'name', order = filters.order || (sort === 'recent' ? 'desc' : 'asc')) {
  return catalogFilters({ ...filters, sort, order });
}

export function bindCatalogFilters(form, onApply) {
  if (!form) return;
  let timer;
  const apply = (report = false) => {
    clearTimeout(timer);
    if (!form.isConnected) return;
    if (form.querySelector('.dual-range input:active')) {
      timer = setTimeout(() => apply(report), 50);
      return;
    }
    if (!form.checkValidity()) {
      if (report) form.reportValidity();
      return;
    }
    const filters = readCatalogForm(form, report);
    if (filters) onApply(filters);
  };
  form.addEventListener('input', (event) => {
    for (const input of form.querySelectorAll('input')) input.setCustomValidity('');
    clearTimeout(timer);
    if (event.target.type !== 'range') timer = setTimeout(apply, 350);
  });
  form.addEventListener('change', () => apply());
  form.addEventListener('submit', (event) => { event.preventDefault(); apply(true); });
  form.addEventListener('keydown', (event) => {
    if (event.key === 'Enter' && event.target.matches('input:not([type=range])')) {
      event.preventDefault();
      apply(true);
    }
  });
}

export function preserveCatalogForm(form) {
  if (!form) return null;
  const open = form.querySelector('details').open;
  const controls = [...form.querySelectorAll('input, select, button')];
  const focusIndex = controls.indexOf(form.ownerDocument.activeElement);
  const start = controls[focusIndex]?.selectionStart, end = controls[focusIndex]?.selectionEnd;
  const fields = [...form.querySelectorAll('[name]')].map((input) => ({ name: input.name,
    value: input.value, active: input.dataset.active, disabled: input.disabled,
    divisor: Number(input.closest('[data-range]')?.dataset.divisor || 1) }));
  return (next) => {
    if (!next) return false;
    next.querySelector('details').open = open;
    let dirty = false;
    for (const field of fields) {
      const input = next.elements.namedItem(field.name);
      let value = field.value;
      if (field.active === 'false' && (value !== '' || field.disabled)) value = input.dataset.default;
      else if (field.name.startsWith('size_') && value !== '') {
        value = String(Math.ceil(Number(value) * field.divisor / Number(input.closest('[data-range]').dataset.divisor)));
      }
      dirty ||= input.value !== value || input.dataset.active !== field.active;
      input.value = value;
      if (field.active != null) input.dataset.active = field.active;
    }
    const focused = next.querySelectorAll('input, select, button')[focusIndex];
    focused?.focus({ preventScroll: true });
    if (start != null) focused?.setSelectionRange(start, end);
    return dirty;
  };
}

export function readCatalogForm(form, report = true) {
  const values = Object.fromEntries(new FormData(form));
  for (const key of ['rating_min', 'rating_max', 'size_min', 'size_max']) {
    if (values[key] != null && values[key] !== '') values[key] = String(Math.ceil(Number(values[key])));
  }
  for (const key of ['year', 'rating', 'size']) {
    const min = values[`${key}_min`];
    const max = values[`${key}_max`];
    if (min !== '' && max !== '' && Number(min) > Number(max)) {
      const input = form.elements.namedItem(`${key}_max`);
      input.setCustomValidity('To must be greater than or equal to From.');
      if (report) input.reportValidity();
      return null;
    }
  }
  for (const input of form.querySelectorAll('[data-range-value]')) {
    if (input.dataset.active !== 'true') delete values[input.name];
  }
  const divisor = Number(form.querySelector('[data-range="size"]')?.dataset.divisor || 1024 ** 2);
  for (const key of ['size_min', 'size_max']) {
    if (values[key] != null && values[key] !== '') values[key] = String(Math.round(Number(values[key]) * divisor));
  }
  return catalogFilters(values);
}

export function catalogRange(name, bounds, filters = {}) {
  const available = Array.isArray(bounds) && bounds.length === 2 && bounds.every(Number.isFinite);
  const largest = Math.max(...(available ? bounds : [0]), Number(filters.size_min || 0), Number(filters.size_max || 0));
  const divisor = name !== 'size' ? 1 : largest >= 1024 ** 3 ? 1024 ** 3 : largest >= 1024 ** 2 ? 1024 ** 2 : largest >= 1024 ? 1024 : 1;
  const unit = name !== 'size' ? '' : divisor === 1024 ** 3 ? 'GiB' : divisor === 1024 ** 2 ? 'MiB' : divisor === 1024 ? 'KiB' : 'B';
  const displayValue = (value) => name === 'year' ? Number(value) : Math.ceil(Number(value) / divisor);
  const defaults = available ? bounds.map(displayValue) : ['', ''];
  const values = ['min', 'max'].map((bound, index) => filters[`${name}_${bound}`] != null
    ? displayValue(filters[`${name}_${bound}`]) : defaults[index]);
  const endpoints = [...defaults, ...values].filter((value) => value !== '' && Number.isFinite(value));
  return { available, divisor, unit, defaults, values,
    min: endpoints.length ? Math.min(...endpoints) : 0,
    max: endpoints.length ? Math.max(...endpoints) : 0 };
}

export function bindCatalogRanges(form) {
  if (!form) return;
  for (const field of form.querySelectorAll('[data-range]')) {
    const numbers = [...field.querySelectorAll('[data-range-value]')];
    const sliders = [...field.querySelectorAll('input[type=range]')];
    const update = () => {
      const [low, high] = numbers.map((input) => Number(input.value));
      const min = Number(sliders[0].min), max = Number(sliders[0].max);
      const percent = (value) => max > min ? Math.max(0, Math.min(100, (value - min) / (max - min) * 100)) : 0;
      sliders.forEach((slider, index) => {
        slider.value = numbers[index].value || slider.min;
        slider.setAttribute('aria-valuetext', `${numbers[index].value} ${field.dataset.unit || ''}`.trim());
      });
      field.style.setProperty('--range-low', `${percent(low)}%`);
      field.style.setProperty('--range-high', `${percent(high)}%`);
      sliders[0].style.zIndex = low >= (min + max) / 2 ? '3' : '2';
      sliders[1].style.zIndex = low >= (min + max) / 2 ? '2' : '3';
    };
    field.addEventListener('input', (event) => {
      const sliderIndex = sliders.indexOf(event.target);
      if (sliderIndex !== -1) {
        const value = Math.ceil(Number(event.target.value));
        const other = Number(numbers[1 - sliderIndex].value);
        numbers[sliderIndex].value = sliderIndex === 0 ? Math.min(value, other) : Math.max(value, other);
        numbers[sliderIndex].dataset.active = 'true';
      } else if (numbers.includes(event.target)) {
        if (field.dataset.range !== 'year' && event.target.value !== '') {
          event.target.value = String(Math.ceil(Number(event.target.value)));
        }
        event.target.dataset.active = String(event.target.value !== '');
      }
      update();
    });
    field.querySelector('[data-range-reset]').addEventListener('click', () => {
      numbers.forEach((input) => { input.value = input.dataset.default; input.dataset.active = 'false'; input.setCustomValidity(''); });
      update();
      form.dispatchEvent(new Event('change'));
    });
    update();
  }
}

function appendFilters(params, filters) {
  for (const [key, value] of Object.entries(catalogFilters(filters))) params.set(key, value);
}

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
  const values = Object.fromEntries(url.searchParams);
  if (values.year) values.year_min = values.year_max = values.year;
  if (values.rating) values.rating_min = values.rating_max = values.rating;
  const filters = catalogFilters(values);
  const filterRoute = Object.keys(filters).length ? { filters } : {};

  if (parts.length === 0) return { name: 'platforms' };
  if (parts[0] === 'search' && parts.length === 1) return { name: 'search', query, page, ...filterRoute };
  if (parts[0] === 'platform' && parts.length >= 2) {
    const platformId = parsePositiveInteger(parts[1]);
    if (platformId) {
      return {
        name: 'platform',
        platformId,
        slug: String(parts[2] || ''),
        query,
        page,
        ...filterRoute,
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
  appendFilters(params, options.filters);
  if (page > 1) params.set('page', String(page));
  const queryString = params.toString();
  const suffix = queryString ? `?${queryString}` : '';
  return `#/platform/${encodeURIComponent(platform.id)}/${encodeURIComponent(platform.slug || '')}${suffix}`;
}

export function gameHash(gameId) {
  return `#/game/${encodeURIComponent(gameId)}`;
}

export function searchHash(query, page = 1, filters = {}) {
  const params = new URLSearchParams();
  const normalized = String(query || '').trim();
  if (normalized) params.set('q', normalized);
  appendFilters(params, filters);
  const parsedPage = parsePage(page);
  if (parsedPage > 1) params.set('page', String(parsedPage));
  const queryString = params.toString();
  return `#/search${queryString ? `?${queryString}` : ''}`;
}

export function contextualSearchHash(route, platforms, query) {
  const platform = route?.name === 'platform'
    ? (platforms || []).find((item) => Number(item.id) === route.platformId)
    : null;
  if (platform) return platformHash(platform, { query, filters: route.filters });
  return String(query || '').trim() || Object.keys(route?.filters || {}).length
    ? searchHash(query, 1, route?.filters) : '#/';
}

export function romListPath({
  platformId = null, query = '', page = 1, limit = PAGE_SIZE, sort = '', filters = {}, includeRanges = false,
} = {}) {
  const safeLimit = Math.max(1, Math.min(100, Number.parseInt(limit, 10) || PAGE_SIZE));
  const safePage = parsePage(page);
  const params = new URLSearchParams({
    limit: String(safeLimit),
    offset: String((safePage - 1) * safeLimit),
  });
  if (includeRanges) params.set('include_ranges', 'true');
  if (platformId) params.set('platform_ids', String(platformId));
  appendFilters(params, filters);
  if (sort) params.set('sort', sort);
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
