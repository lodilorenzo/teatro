import { icon } from '../public/icons.js';
import { captureScroll, restoreScroll } from '../public/scroll.js';
import { VERSION_LABEL, syncCoverImages } from '../public/shared.js';
import { syncRommCovers } from './features/romm-covers.js';
import { api } from './api.js';
import { clearAuth, loadSavedAuth, saveAuth } from './auth.js';
import { createSession, revokeSession } from '../public/auth.js';
import { html } from './dom.js';
import {
  clearBackgroundTransfers, resumeBackgroundTransfers, settleBackgroundTransferStarts,
} from './features/background-transfer.js';
import { GogImportController } from './features/gog-import.js';
import { createIgdbController } from './features/igdb.js';
import { activeJobCancellationUrl, JobsController } from './features/jobs.js';
import { createLibraryController } from './features/library.js';
import { LibraryScanController } from './features/library-scan.js';
import { RommSourceController } from './features/romm-source.js';
import { createServerCleanupController } from './features/server-cleanup.js';
import { UploadController } from './features/upload.js';
import {
  beginRomPageRequest, dismissNotification, isCurrentRomPageRequest, isJobActive, resetSession,
  setAuth, setGogImportStatus, setIgdbSettings, setIgdbStatus, setLoading, setPlatforms, setRomPage,
  setScreen, setStats, setUser, showError, showNotice, state,
} from './state.js';
import { renderDashboard } from './views/dashboard.js';
import { renderJobs } from './views/jobs.js';
import { renderLibrary } from './views/library.js';
import { renderSettings } from './views/settings.js';
import { renderRommBrowse } from './views/romm-browse.js';
import {
  jobsSummary, renderNotificationStack, rommSummary, sideNavButton, statsSummary,
} from './views/shared.js';
import { renderImport } from './views/upload.js';

const app = document.getElementById('app');
const notificationTimers = new Map();
const LEAVE_WARNING = 'Ongoing jobs will be cancelled and their progress will be lost if you leave the Admin page.';
let navigationApproved = false;
let exitCancellationStarted = false;
setAuth(loadSavedAuth());
void resumeBackgroundTransfers(state.auth?.header).catch(() => null);

function syncNotificationTimers() {
  const activeIds = new Set(state.notifications.map(({ id }) => id));
  for (const [notificationId, timeoutId] of notificationTimers) {
    if (activeIds.has(notificationId)) continue;
    clearTimeout(timeoutId);
    notificationTimers.delete(notificationId);
  }

  for (const notification of state.notifications) {
    if (notificationTimers.has(notification.id)) continue;
    const timeoutId = setTimeout(() => {
      notificationTimers.delete(notification.id);
      dismissNotification(notification.id);
      app.querySelector(`[data-notification-id="${notification.id}"]`)?.remove?.();
    }, Math.max(0, notification.expiresAt - Date.now()));
    timeoutId?.unref?.();
    notificationTimers.set(notification.id, timeoutId);
  }
}

async function loadPlatforms() {
  setPlatforms(await api('/api/platforms'));
}

async function loadStats() {
  setStats(await api('/api/admin/stats'));
}

async function loadIgdbStatus() {
  setIgdbStatus(await api('/api/admin/igdb/status').catch(() => null));
}

async function loadIgdbSettings() {
  setIgdbSettings(await api('/api/admin/igdb/settings').catch(() => null));
}

async function loadGogImportStatus() {
  setGogImportStatus(await api('/api/admin/gog-import/status').catch(() => null));
}

async function loadRommSourceStatus() {
  await rommSourceController?.loadStatus();
}

async function loadRoms() {
  const requestId = beginRomPageRequest();
  const params = new URLSearchParams({
    limit: String(state.romLimit), offset: String(state.romOffset),
  });
  if (state.platformFilter) params.set('platform_ids', state.platformFilter);
  if (state.search) params.set('search', state.search);
  if (state.missingCoverFilter) params.set('missing_cover', 'true');
  const page = await api(`/api/roms?${params}`);
  if (isCurrentRomPageRequest(requestId)) setRomPage(page);
}

async function refreshAll() {
  setLoading(true);
  render();
  try {
    setUser(await api('/api/users/me'));
    if (state.user.role !== 'admin') {
      window.location.replace('/');
      return;
    }
    await Promise.all([
      loadPlatforms(), loadStats(), loadIgdbStatus(), loadIgdbSettings(),
      loadGogImportStatus(), loadRommSourceStatus(), loadRoms(),
    ]);
  } catch (error) {
    showError(error);
  } finally {
    setLoading(false);
    render();
  }
}

let gogImportController = null;
let libraryScanController = null;
let rommSourceController = null;
const jobsController = new JobsController({
  app,
  render,
  setError: showError,
  cancelJob: (jobId) => {
    const job = state.jobs.find(({ id }) => id === jobId);
    if (job?.type === 'library-scan') return libraryScanController?.cancelJob(jobId);
    if (job?.type === 'gog-import') return gogImportController?.cancelJob(jobId);
    if (job?.type === 'romm-import') return rommSourceController?.cancelJob(jobId);
    return uploadController?.cancelJob(jobId);
  },
});
const controllerContext = {
  app,
  render,
  setNotice: showNotice,
  setError: showError,
  loadPlatforms,
  loadStats,
  loadRoms,
  loadIgdbStatus,
  jobChanged: (jobId) => jobsController.refreshJob(jobId),
};
const uploadController = new UploadController(controllerContext);
gogImportController = new GogImportController(controllerContext);
libraryScanController = new LibraryScanController(controllerContext);
const libraryController = createLibraryController(controllerContext);
const cleanupController = createServerCleanupController(controllerContext);
const igdbController = createIgdbController(controllerContext);
rommSourceController = new RommSourceController(controllerContext);

/// The scroll snapshot is keyed by the page a repaint lands on. Every re-render of the same admin
/// screen keeps its offsets; moving to another screen starts at the top.
function scrollPageKey() {
  return state.auth ? `admin:${state.screen}` : 'admin:login';
}

function beginRender() {
  return captureScroll(scrollPageKey(), { root: app });
}

function endRender(snapshot) {
  restoreScroll(snapshot, scrollPageKey(), { root: app });
}

function renderLogin() {
  const snapshot = beginRender();
  const loginError = [...state.notifications].reverse().find(({ kind }) => kind === 'error');
  const loginErrorMessage = loginError?.message || state.error;
  app.className = '';
  app.innerHTML = `
    <div class="login-page">
      <header class="login-header">
        <div class="brand">
          <img class="brand-mark" src="/admin/favicon.svg?icon=teatro-controller" alt="" />
          <div class="brand-name">
            <strong>Teatro Admin</strong>
            <span class="version-label" title="Beta / release-hardening">${html(VERSION_LABEL)}</span>
          </div>
        </div>
        <a class="text-link" href="/">Player library ${icon('arrow')}</a>
      </header>
      <main class="login-shell">
        <section class="login-panel" aria-labelledby="sign-in-heading">
          <h1 id="sign-in-heading">Sign in to admin</h1>
          <p class="muted">Use your Teatro admin account.</p>
          ${loginErrorMessage ? `<div class="error" role="alert" ${loginError ? `data-notification-id="${loginError.id}"` : ''}>${html(loginErrorMessage)}</div>` : ''}
          <form id="login-form">
            <label><span>Username</span><input name="username" autocomplete="username" required autofocus /></label>
            <label><span>Password</span><input name="password" type="password" autocomplete="current-password" required /></label>
            <label class="checkbox"><input name="remember_me" type="checkbox" /> Remember me for 30 days</label>
            <button class="primary" type="submit">Sign in</button>
          </form>
          <p class="login-security">${icon('shield')}<span>Your password is not saved. Use Remember me only on a trusted device.</span></p>
          <small class="server-address">Server <code>${html(window.location.origin)}/admin</code></small>
        </section>
      </main>
      <footer class="login-footer"><span>Teatro Admin</span></footer>
    </div>`;
  endRender(snapshot);
  document.getElementById('login-form')?.addEventListener('submit', async (event) => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const username = String(form.get('username') || '').trim();
    const submit = event.currentTarget.querySelector('button[type="submit"]');
    submit.disabled = true;
    try {
      const rememberMe = form.has('remember_me');
      const { auth } = await createSession(username, String(form.get('password') || ''), rememberMe);
      saveAuth(auth, rememberMe);
      void resumeBackgroundTransfers(auth.header).catch(() => null);
      await refreshAll();
    } catch (error) {
      showError(error);
      render();
    } finally {
      submit.disabled = false;
    }
  });
  syncNotificationTimers();
}

function render() {
  if (!state.auth) {
    renderLogin();
    return;
  }
  const snapshot = beginRender();
  const screenHtml = {
    library: renderLibrary,
    upload: renderImport,
    jobs: renderJobs,
    settings: renderSettings,
    romm: renderRommBrowse,
    dashboard: renderDashboard,
  }[state.screen]?.() || renderDashboard();

  app.className = '';
  app.innerHTML = `
    <div class="desktop-shell"><div class="desktop-workspace">
      <aside class="sidebar">
        <div class="sidebar-brand"><img class="brand-mark" src="/admin/favicon.svg?icon=teatro-controller" alt="" /><div><strong>Teatro Admin</strong><span class="version-label" title="Beta / release-hardening">${html(VERSION_LABEL)}</span></div></div>
        <div class="sidebar-session">
          <span class="session-pill">${html(state.user?.username || state.auth.username || 'signed in')}</span>
          <div class="actions compact-actions"><button class="ghost" type="button" data-action="public-library" aria-label="Open player library" title="Player library">${icon('external-link')}</button><button class="ghost" type="button" data-action="refresh" aria-label="Refresh data" title="Refresh">${icon('refresh-cw')}</button><button class="danger" type="button" data-action="logout" aria-label="Sign out" title="Sign out">${icon('logout')}</button></div>
        </div>
        <nav class="side-nav" aria-label="Admin sections">
          ${sideNavButton('dashboard', 'layout-dashboard', 'Dashboard', statsSummary())}
          ${sideNavButton('library', 'library', 'Library')}
          ${sideNavButton('upload', 'upload', 'Import')}
          ${state.rommSource?.configured ? sideNavButton('romm', 'download', 'RomM', rommSummary()) : ''}
          ${sideNavButton('jobs', 'list-todo', 'Jobs', jobsSummary())}
          ${sideNavButton('settings', 'settings', 'Settings')}
        </nav>
        ${renderNotificationStack()}
      </aside>
      <main class="screen" data-scroll-key="screen">
        ${screenHtml}
      </main>
    </div></div>`;
  endRender(snapshot);
  bindEvents();
  syncCoverImages(app, state.auth?.header);
  syncRommCovers(app, state.auth?.header);
  syncNotificationTimers();
}

function bindEvents() {
  app.querySelectorAll('[data-nav]').forEach((button) => {
    button.addEventListener('click', () => { setScreen(button.dataset.nav); render(); });
  });
  app.querySelector('[data-action="logout"]')?.addEventListener('click', async () => {
    try {
      await revokeSession(state.auth);
    } catch (error) {
      showError(new Error(`Could not sign out: ${error.message}`));
      render();
      return;
    }
    gogImportController.stopAll({ removeJobs: false, clearStorage: true });
    libraryScanController.stopAll({ clearStorage: true });
    rommSourceController.stopAll({ clearStorage: true });
    void clearBackgroundTransfers();
    clearAuth();
    resetSession();
    render();
  });
  app.querySelector('[data-action="refresh"]')?.addEventListener('click', refreshAll);
  app.querySelector('[data-action="public-library"]')?.addEventListener('click', async () => {
    const active = state.jobs.filter(isJobActive).length;
    if (active && !globalThis.confirm(`${LEAVE_WARNING}\n\n${active} active job${active === 1 ? '' : 's'} will be cancelled.`)) return;
    try {
      await settleBackgroundTransferStarts();
      if (active) await jobsController.cancelAll();
    } catch (error) {
      showError(new Error(`Could not cancel all active jobs: ${error?.message || error}`));
      render();
      return;
    }
    navigationApproved = true;
    window.location.assign('/');
  });

  uploadController.bindDropZone();
  uploadController.bindQueueControls();
  uploadController.bindPlanControls();
  app.querySelector('[data-action="clear-upload-list"]')?.addEventListener('click', uploadController.clearSelectedFiles);
  app.querySelector('#upload-form')?.addEventListener('submit', uploadController.onSubmit);
  app.querySelector('[data-action="preview-upload-plan"]')?.addEventListener('click', uploadController.onPreviewPlan);
  app.querySelectorAll('#upload-form input[name="platform_id"]').forEach((input) => {
    input.addEventListener('change', uploadController.onPlatformChange);
  });

  gogImportController.bind();
  jobsController.bind();
  libraryController.bind();
  libraryScanController.bind();
  rommSourceController.bind();
  cleanupController.bind();
  app.querySelector('#igdb-search-form')?.addEventListener('submit', igdbController.onSearchSubmit);
  app.querySelector('#igdb-settings-form')?.addEventListener('submit', igdbController.onSettingsSubmit);
  app.querySelector('[data-action="clear-igdb-credentials"]')?.addEventListener('click', igdbController.onSettingsClear);
  app.querySelectorAll('[data-apply-igdb]').forEach((button) => {
    button.addEventListener('click', () => igdbController.apply(button.dataset.romId, Number(button.dataset.applyIgdb)));
  });
}

function cancelActiveJobsOnExit() {
  if (exitCancellationStarted) return;
  exitCancellationStarted = true;
  const authorization = state.auth?.header;
  if (authorization) {
    for (const job of state.jobs.filter(isJobActive)) {
      const url = activeJobCancellationUrl(job);
      if (!url) continue;
      void fetch(url, {
        method: 'DELETE',
        headers: { Accept: 'application/json', Authorization: authorization },
        keepalive: true,
      }).catch(() => null);
    }
  }
  void clearBackgroundTransfers();
}

window.addEventListener('beforeunload', (event) => {
  if (navigationApproved || !state.jobs.some(isJobActive)) return;
  event.preventDefault();
  event.returnValue = LEAVE_WARNING;
});
window.addEventListener('pagehide', () => {
  if (!navigationApproved && state.jobs.some(isJobActive)) cancelActiveJobsOnExit();
});
window.addEventListener('pageshow', (event) => {
  navigationApproved = false;
  exitCancellationStarted = false;
  if (event.persisted) {
    setAuth(loadSavedAuth());
    if (state.auth) void refreshAll();
    else { clearAuth(); resetSession(); render(); }
  }
});

if (state.auth) refreshAll();
else renderLogin();
