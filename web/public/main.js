import {
  clearBackgroundTransfers, resumeBackgroundTransfers,
} from '/admin/features/background-transfer.js';
import { clearCoverImages, syncCoverImages } from './shared.js';
import { apiRequest } from './api.js';
import { createSession, revokeSession, clearSavedAuth, loadSavedAuth, saveAuth } from './auth.js';
import { PAGE_SIZE, contextualSearchHash, gameHash, parseRoute, romListPath } from './catalog.js';
import { downloadArchive, downloadFile } from './downloads.js';
import { captureScroll, restoreScroll, scrollToTop } from './scroll.js';
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

const app = document.getElementById('app');
const state = {
  auth: loadSavedAuth(),
  user: null,
  platforms: [],
  recentGames: [],
  stats: null,
  route: parseRoute(window.location.hash),
  libraryView: 'grid',
  game: null,
};
let navigationRequest = 0;
let toastTimer = null;
// A navigation that stays on the same route (a retry, a repeated search, a hash re-entry) keeps the
// reader where they were. The snapshot is taken once per navigation so the intermediate loading
// screen cannot overwrite it with its own, necessarily empty, offsets.
let pendingScroll = null;
void resumeBackgroundTransfers(state.auth?.header).catch(() => null);

async function bootstrap() {
  if (!state.auth) {
    renderLoginPage();
    return;
  }

  renderContent(renderLoading('Opening your library…'));
  const authorization = state.auth.header;
  try {
    const [user, platforms, recentPage, stats] = await Promise.all([
      apiRequest('/api/users/me', authorization),
      apiRequest('/api/platforms', authorization),
      apiRequest(romListPath({ limit: 10, sort: 'recent' }), authorization),
      apiRequest('/api/stats', authorization).catch(() => null),
    ]);
    if (state.auth?.header !== authorization) return;
    state.user = user;
    state.platforms = platforms;
    state.recentGames = recentPage.items || [];
    state.stats = stats;
    await navigate();
  } catch (error) {
    if (state.auth?.header !== authorization) return;
    if (error.status === 401) endSession('Your saved sign-in has expired. Please sign in again.');
    else renderContent(renderError(error));
  }
}

function scrollPageKey(route) {
  return [
    'public', route?.name || 'unknown', route?.platformId ?? '', route?.query || '',
    route?.page ?? '', route?.gameId ?? '',
  ].join(':');
}

function renderLoginPage(error = '') {
  pendingScroll = null;
  scrollToTop({ root: app });
  clearCoverImages();
  app.className = '';
  app.innerHTML = renderLogin({ error, origin: window.location.origin });
  app.querySelector('#login-form')?.addEventListener('submit', onLogin);
}

async function onLogin(event) {
  event.preventDefault();
  const formElement = event.currentTarget;
  const submit = formElement.querySelector('button[type="submit"]');
  const form = new FormData(formElement);
  const username = String(form.get('username') || '').trim();
  const password = String(form.get('password') || '');
  if (!username) {
    renderLoginPage('Enter your Teatro username.');
    return;
  }
  const rememberMe = form.has('remember_me');
  submit.disabled = true;
  submit.textContent = 'Opening library…';

  try {
    const { auth: candidate, user } = await createSession(username, password, rememberMe);
    state.auth = candidate;
    state.user = user;
    saveAuth(candidate, rememberMe);
    void resumeBackgroundTransfers(candidate.header).catch(() => null);
    await bootstrap();
  } catch (error) {
    renderLoginPage(error.message || 'Sign-in failed.');
  }
}

async function navigate() {
  if (!state.auth) {
    renderLoginPage();
    return;
  }
  if (!state.user) return;

  const requestId = ++navigationRequest;
  const route = parseRoute(window.location.hash);
  pendingScroll = captureScroll(scrollPageKey(state.route), { root: app });
  state.route = route;
  state.game = null;

  if (route.name === 'platforms') {
    renderContent(renderPlatforms(state.platforms, state.recentGames, state.stats), true);
    return;
  }
  if (route.name === 'notFound') {
    renderContent(renderNotFound(), true);
    return;
  }
  if (route.name === 'search' && !route.query) {
    renderContent(renderSearchPage({ page: emptyPage(), route, view: state.libraryView }), true);
    return;
  }

  renderContent(renderLoading(route.name === 'game' ? 'Loading game…' : 'Loading games…'));
  const authorization = state.auth.header;
  try {
    if (route.name === 'platform') {
      const platform = state.platforms.find((item) => Number(item.id) === route.platformId);
      if (!platform) {
        if (requestId === navigationRequest) renderContent(renderNotFound(), true);
        return;
      }
      const page = await apiRequest(romListPath({
        platformId: platform.id,
        query: route.query,
        page: route.page,
      }), authorization);
      if (requestId !== navigationRequest) return;
      renderContent(renderPlatformPage({ platform, page, route, view: state.libraryView }), true);
      return;
    }

    if (route.name === 'search') {
      const page = await apiRequest(romListPath({ query: route.query, page: route.page }), authorization);
      if (requestId !== navigationRequest) return;
      renderContent(renderSearchPage({ page, route, view: state.libraryView }), true);
      return;
    }

    if (route.name === 'game') {
      const game = await apiRequest(`/api/roms/${encodeURIComponent(route.gameId)}`, authorization);
      if (requestId !== navigationRequest) return;
      state.game = game;
      renderContent(renderGameDetail(game), true);
      return;
    }

    renderContent(renderNotFound(), true);
  } catch (error) {
    if (requestId !== navigationRequest) return;
    if (error.status === 401) {
      endSession('Your sign-in has expired. Please sign in again.');
      return;
    }
    if (error.status === 404 && route.name === 'game') {
      renderContent(renderNotFound(), true);
      return;
    }
    renderContent(renderError(error), true);
  }
}

function renderContent(content, focusMain = false) {
  app.className = '';
  app.innerHTML = renderShell({
    user: state.user,
    platforms: state.platforms,
    route: state.route,
    content,
  });
  restoreScroll(pendingScroll, scrollPageKey(state.route), { root: app });
  bindShellEvents();
  syncCoverImages(app, state.auth?.header);
  if (focusMain) app.querySelector('#main-content')?.focus({ preventScroll: true });
}

function bindShellEvents() {
  app.querySelector('[data-action="logout"]')?.addEventListener('click', async () => {
    try {
      await revokeSession(state.auth);
      endSession();
    } catch (error) {
      showToast(`Could not sign out: ${error.message}`, 'error');
    }
  });
  app.querySelector('[data-action="retry"]')?.addEventListener('click', bootstrap);
  app.querySelector('[data-action="surprise"]')?.addEventListener('click', onSurprise);
  bindRecentCarousel();

  app.querySelector('#header-search-form')?.addEventListener('submit', (event) => {
    event.preventDefault();
    const query = String(new FormData(event.currentTarget).get('q') || '').trim();
    goTo(contextualSearchHash(state.route, state.platforms, query));
  });

  const viewButtons = [...app.querySelectorAll('[data-library-view]')];
  viewButtons.forEach((button) => {
    button.addEventListener('click', () => {
      state.libraryView = button.dataset.libraryView === 'list' ? 'list' : 'grid';
      app.querySelector('.game-grid')?.classList.toggle('list-view', state.libraryView === 'list');
      viewButtons.forEach((candidate) => candidate.setAttribute(
        'aria-pressed', String(candidate.dataset.libraryView === state.libraryView),
      ));
    });
  });

  app.querySelectorAll('[data-download-file]').forEach((button) => {
    button.addEventListener('click', () => onDownloadFile(button));
  });
  app.querySelector('[data-download-archive]')?.addEventListener('click', (event) => onDownloadArchive(event.currentTarget));
}

function bindRecentCarousel() {
  const carousel = app.querySelector('[data-recent-carousel]');
  if (!carousel) return;
  const games = [...carousel.querySelectorAll('[data-recent-game]')];
  const previous = app.querySelector('[data-action="recent-previous"]');
  const next = app.querySelector('[data-action="recent-next"]');
  const status = app.querySelector('[data-recent-status]');
  if (!previous || !next || !status) return;
  const pageCount = Math.ceil(games.length / 5);
  let page = 0;

  const showPage = (target) => {
    page = Math.max(0, Math.min(pageCount - 1, target));
    games.forEach((game, index) => { game.hidden = Math.floor(index / 5) !== page; });
    previous.disabled = page === 0;
    next.disabled = page === pageCount - 1;
    status.textContent = `Page ${page + 1} of ${pageCount}`;
  };
  previous.addEventListener('click', () => showPage(page - 1));
  next.addEventListener('click', () => showPage(page + 1));
}

async function onDownloadFile(button) {
  const game = state.game;
  const file = game?.files?.find((item) => String(item.id) === button.dataset.downloadFile);
  if (!game || !file || !state.auth) return;
  button.disabled = true;
  try {
    await downloadFile({
      romId: game.id,
      file,
      authorization: state.auth.header,
      notify: showToast,
    });
  } catch (error) {
    handleDownloadError(error);
  } finally {
    if (button.isConnected) button.disabled = false;
  }
}

async function onDownloadArchive(button) {
  const game = state.game;
  if ((game?.files?.length || 0) < 2 || !state.auth) return;
  button.disabled = true;
  try {
    await downloadArchive({
      romId: game.id,
      authorization: state.auth.header,
      notify: showToast,
    });
  } catch (error) {
    handleDownloadError(error);
  } finally {
    if (button.isConnected) button.disabled = false;
  }
}

function handleDownloadError(error) {
  if (error?.name === 'AbortError') return;
  if (error?.status === 401) {
    endSession('Your sign-in expired before the download started. Please sign in again.');
    return;
  }
  showToast(error?.message || 'The download could not be started.', 'error');
}

function showToast(message, kind = 'info') {
  const region = document.getElementById('toast-region');
  if (!region) return;
  clearTimeout(toastTimer);
  const toast = document.createElement('div');
  toast.className = `toast ${kind}`;
  toast.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  toast.textContent = message;
  region.replaceChildren(toast);
  if (kind !== 'progress') {
    toastTimer = setTimeout(() => {
      if (toast.isConnected) toast.remove();
    }, 6500);
  }
}

function goTo(hash) {
  if (window.location.hash === hash) navigate();
  else window.location.hash = hash;
}

async function onSurprise(event) {
  const button = event.currentTarget;
  if (button.disabled || !state.auth) return;
  const pool = state.platforms.filter((platform) => Number(platform.rom_count) > 0);
  if (!pool.length) {
    showToast('No games are available yet.', 'error');
    return;
  }
  button.disabled = true;
  try {
    const platform = pool[Math.floor(Math.random() * pool.length)];
    const count = Number(platform.rom_count) || 0;
    const pages = Math.max(1, Math.ceil(count / PAGE_SIZE));
    const page = Math.floor(Math.random() * pages) + 1;
    const result = await apiRequest(
      romListPath({ platformId: platform.id, page, limit: PAGE_SIZE }),
      state.auth.header,
    );
    const items = result.items || [];
    if (!items.length) {
      button.disabled = false;
      showToast('Could not pick a game. Try again.', 'error');
      return;
    }
    const pick = items[Math.floor(Math.random() * items.length)];
    goTo(gameHash(pick.id));
  } catch (error) {
    button.disabled = false;
    showToast(error.message || 'Could not pick a game.', 'error');
  }
}

function endSession(message = '') {
  navigationRequest += 1;
  state.auth = null;
  state.user = null;
  state.platforms = [];
  state.recentGames = [];
  state.game = null;
  void clearBackgroundTransfers();
  clearSavedAuth();
  clearCoverImages();
  renderLoginPage(message);
}

function emptyPage() {
  return { items: [], total: 0, limit: PAGE_SIZE, offset: 0 };
}

window.addEventListener('hashchange', navigate);
window.addEventListener('pageshow', (event) => {
  if (!event.persisted) return;
  state.auth = loadSavedAuth();
  if (!state.auth) endSession();
  else void bootstrap();
});
document.addEventListener('keydown', (event) => {
  if (!state.auth || event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return;
  if (['INPUT', 'TEXTAREA', 'SELECT'].includes(document.activeElement?.tagName)) return;
  event.preventDefault();
  document.getElementById('header-search')?.focus();
});

bootstrap();
