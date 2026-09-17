import { api } from '../api.js';
import {
  ROMM_BROWSE_PAGE_SIZE, addJob,
  applyRommImportJobSnapshot, beginRommImportJob,
  clearRommSelections, closeRommDrawer, findJob, findRommSelection, initialRommImportProgress,
  loadRommDrawerDetail, markRommImportJobUnavailable, markRommItemImported, openRommDrawer,
  removeJob, resetRommBrowse, restoreRommImportJob, rommSelectionPlatformId, rommSelectionTotals,
  selectRommItems, setRommBrowse, setRommHideExisting, setRommImportPollingFailure,
  setRommPlatforms, setRommSourceDetailOpen, setRommSourceEditing, setRommSourceStatus, state,
  toggleRommDrawerFile, toggleRommSelection, updateJob, updateRommDrawer, updateRommSelectionEntry,
  visibleRommItems,
} from '../state.js';
import {
  clearServerJobs, createServerJobPoller, forgetServerJob, loadServerJobs, rememberServerJob,
} from './server-jobs.js';

const PHASES = new Set([
  'queued', 'resolving', 'checking', 'downloading', 'verifying', 'ingesting', 'finalizing',
  'complete',
]);
const STATES = new Set(['queued', 'running', 'succeeded', 'failed', 'cancelled']);
const IMPORT_ID = /^romm_[A-Za-z0-9_-]+$/u;
// A batch is admitted a few games at a time: Teatro is a guest on the remote server, and each
// running import holds one streaming download open.
const IMPORT_CONCURRENCY = 2;
const SEARCH_DEBOUNCE_MS = 300;

function statusUrlFor(id) {
  return `/api/admin/sources/romm/imports/${encodeURIComponent(id)}`;
}

export function validateRommImportCreateResponse(response) {
  const job = response?.job;
  if (!IMPORT_ID.test(job?.id || '') || job?.state !== 'queued' || job?.phase !== 'queued') {
    throw new Error('Teatro returned an invalid RomM import job summary.');
  }
  const expected = statusUrlFor(job.id);
  if (response?.status_url !== expected) {
    throw new Error('Teatro returned an invalid RomM import status URL.');
  }
  return { job, statusUrl: expected };
}

function validCount(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

// A hash conflict is a warning attached to a completed import, so an absent list is normal. The
// values are lowercase hex from the server; anything else means the snapshot is not Teatro's.
const HEX_HASH = /^[0-9a-f]{8,128}$/u;
const HASH_ALGORITHMS = new Set(['crc32', 'md5', 'sha1', 'sha256']);

function validateHashConflicts(conflicts) {
  if (conflicts === null || conflicts === undefined) return;
  if (!Array.isArray(conflicts)) {
    throw new Error('Teatro returned an invalid RomM import hash report.');
  }
  for (const conflict of conflicts) {
    if (typeof conflict?.file_name !== 'string'
      || !HASH_ALGORITHMS.has(conflict.algorithm)
      || !HEX_HASH.test(conflict.declared || '')
      || !HEX_HASH.test(conflict.computed || '')) {
      throw new Error('Teatro returned an invalid RomM import hash report.');
    }
  }
}

export function validateRommImportSnapshot(snapshot, expectedId) {
  if (!snapshot || snapshot.id !== expectedId
    || !STATES.has(snapshot.state) || !PHASES.has(snapshot.phase)) {
    throw new Error('Teatro returned an invalid RomM import job resource.');
  }
  if (snapshot.progress !== null && snapshot.progress !== undefined) {
    const progress = snapshot.progress;
    if (progress?.kind !== 'bytes'
      || !validCount(progress.current)
      || !validCount(progress.total)
      || progress.current > progress.total
      || !Number.isFinite(progress.percent)
      || progress.percent < 0
      || progress.percent > 100) {
      throw new Error('Teatro returned invalid RomM import progress.');
    }
  }
  if (snapshot.state === 'succeeded') {
    const result = snapshot.result;
    if (!['imported', 'already_present'].includes(result?.outcome)
      || typeof result.title !== 'string'
      || !validCount(result.file_count)
      || !validCount(result.downloaded_bytes)
      || !Array.isArray(result.imported_rom_ids)) {
      throw new Error('Teatro returned an invalid RomM import result.');
    }
    validateHashConflicts(result.hash_conflicts);
  } else if (snapshot.state === 'failed'
    && (typeof snapshot.error?.code !== 'string' || typeof snapshot.error?.message !== 'string')) {
    throw new Error('Teatro returned an invalid failed RomM import result.');
  }
  return snapshot;
}

export class RommSourceController {
  constructor(context) {
    this.context = Object.freeze({ ...context });
    this.request = context.request || api;
    this.setTimeout = context.setTimeout || ((...args) => globalThis.setTimeout(...args));
    this.clearTimeout = context.clearTimeout || ((...args) => globalThis.clearTimeout(...args));
    this.searchTimer = null;
    this.browseRequestId = 0;
    this.batchInFlight = false;
    // Queued games wait in the browser until a slot frees, so one click cannot open fifty
    // simultaneous downloads against the remote server.
    this.importQueue = [];
    this.runningImports = new Set();
    this.poller = createServerJobPoller({
      ...context,
      request: this.request,
      getJob: findJob,
      shouldPoll: (appJobId) => this.shouldPoll(appJobId),
      onSnapshot: (...args) => this.onSnapshot(...args),
      onError: (...args) => this.onPollError(...args),
    });

    if (state.auth?.header) {
      for (const record of loadServerJobs('romm-import')) {
        const existing = state.jobs.find((job) => job.serverJobId === record.id);
        const job = existing || restoreRommImportJob(record.id, record.statusUrl);
        this.poller.watch(job.id, { immediate: true });
      }
    } else {
      clearServerJobs('romm-import');
    }
  }

  /// Loads the source status. A disabled server answers 404 on every RomM route.
  async loadStatus() {
    const status = await this.request('/api/admin/sources/romm/status').catch(() => null);
    setRommSourceStatus(status);
    if (!status?.configured) resetRommBrowse();
    return status;
  }

  bind(root = this.context.app) {
    root.querySelector('#romm-source-settings-form')?.addEventListener('submit', this.onSaveSettings);
    root.querySelector('[data-action="test-romm-source"]')?.addEventListener('click', this.onTest);
    root.querySelector('[data-action="clear-romm-source"]')?.addEventListener('click', this.onClear);
    root.querySelector('[data-action="edit-romm-source"]')?.addEventListener('click', () => {
      setRommSourceEditing(true);
      this.context.render();
    });
    root.querySelector('[data-action="cancel-romm-source-edit"]')?.addEventListener('click', () => {
      setRommSourceEditing(false);
      this.context.render();
    });
    root.querySelector('[data-action="romm-toggle-connection"]')?.addEventListener('click', () => {
      setRommSourceDetailOpen(!state.rommSourceDetailOpen);
      this.context.render();
    });
    root.querySelector('[data-action="romm-refresh"]')?.addEventListener('click', this.onRefresh);
    root.querySelector('#romm-search')?.addEventListener('input', this.onSearchInput);
    root.querySelector('[data-romm-platform]')?.addEventListener('change', (event) => {
      this.selectPlatform(event.currentTarget.value);
    });
    root.querySelectorAll('[data-romm-toggle]').forEach((button) => {
      button.addEventListener('click', () => this.toggleCard(Number(button.dataset.rommToggle)));
    });
    root.querySelectorAll('[data-romm-drawer]').forEach((button) => {
      button.addEventListener('click', () => void this.openDrawer(Number(button.dataset.rommDrawer)));
    });
    root.querySelector('[data-action="romm-select-shown"]')?.addEventListener('click', this.selectShown);
    root.querySelectorAll('[data-action="romm-clear-selection"]').forEach((button) => {
      button.addEventListener('click', () => {
        clearRommSelections();
        this.context.render();
      });
    });
    root.querySelector('[data-action="romm-hide-existing"]')?.addEventListener('change', (event) => {
      setRommHideExisting(event.currentTarget.checked);
      this.context.render();
    });
    root.querySelectorAll('[data-action="romm-close-drawer"]').forEach((element) => {
      element.addEventListener('click', () => {
        closeRommDrawer();
        this.context.render();
      });
    });
    root.querySelector('[data-action="romm-commit-drawer"]')?.addEventListener('click', this.commitDrawer);
    root.querySelectorAll('[data-romm-file]').forEach((input) => {
      input.addEventListener('change', () => {
        toggleRommDrawerFile(Number(input.dataset.rommFile), input.checked);
        this.context.render();
      });
    });
    root.querySelector('[data-romm-target-platform]')?.addEventListener('change', (event) => {
      updateRommDrawer({ platformId: event.currentTarget.value });
      this.context.render();
    });
    root.querySelector('[data-romm-title]')?.addEventListener('input', (event) => {
      updateRommDrawer({ title: event.currentTarget.value });
    });
    root.querySelector('[data-action="romm-import"]')?.addEventListener('click', this.startBatchImport);

    for (const job of state.jobs) {
      if (job.type === 'romm-import' && job.serverJobId) this.poller.watch(job.id);
    }
    if (state.screen === 'romm') this.ensureBrowsed();
  }

  onSaveSettings = async (event) => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const secret = String(form.get('secret') || '').trim();
    const payload = {
      base_url: String(form.get('base_url') || '').trim(),
      username: String(form.get('username') || '').trim(),
      auth_mode: String(form.get('auth_mode') || 'token'),
      acknowledge_plaintext_http: form.get('acknowledge_plaintext_http') === 'on',
    };
    if (secret) payload.secret = secret;

    try {
      setRommSourceStatus(await this.request('/api/admin/sources/romm/settings', {
        method: 'PATCH',
        body: JSON.stringify(payload),
      }));
      setRommSourceEditing(false);
      const indexed = !state.rommSource?.configured
        || state.rommSource.index_refreshed_at
        || await this.refreshRemoteIndex({ announce: false });
      this.context.setNotice(indexed
        ? 'RomM source saved. The complete remote game index is stored locally.'
        : 'RomM source saved, but its remote game index could not be downloaded.');
    } catch (error) {
      this.context.setError(error);
    }
    this.context.render();
  };

  onClear = async () => {
    if (!globalThis.confirm('Remove the configured RomM source and its stored credentials?')) return;
    try {
      setRommSourceStatus(await this.request('/api/admin/sources/romm/settings', { method: 'DELETE' }));
      resetRommBrowse();
      this.context.setNotice('RomM source removed, including its stored credentials.');
    } catch (error) {
      this.context.setError(error);
    }
    this.context.render();
  };

  onTest = async () => {
    if (await this.testConnection()) {
      await this.loadPlatforms();
      if (state.screen === 'romm') await this.loadRoms();
    }
    this.context.render();
  };

  connectionFailed(error) {
    const message = error?.message || String(error) || 'The RomM server could not be reached.';
    setRommSourceStatus({
      ...state.rommSource,
      lastTest: { result: 'unreachable', message },
    });
    resetRommBrowse();
    this.context.setError(new Error(message));
  }

  async testConnection() {
    let result;
    try {
      result = await this.request('/api/admin/sources/romm/test', { method: 'POST' });
    } catch (error) {
      this.connectionFailed(error);
      return false;
    }

    setRommSourceStatus({ ...state.rommSource, lastTest: result });
    if (result?.result === 'reachable') return true;

    resetRommBrowse();
    this.context.setError(new Error(result?.message || 'The RomM connection failed.'));
    return false;
  }

  async loadPlatforms() {
    try {
      setRommPlatforms(await this.request('/api/admin/sources/romm/platforms'));
    } catch (error) {
      setRommPlatforms([]);
      this.context.setError(error);
    }
  }

  /// A browse repaint lands while the operator is still typing, so the live search field keeps its
  /// focus, its caret, and any characters typed after the request left.
  renderKeepingSearchFocus() {
    const active = globalThis.document?.activeElement;
    const focused = active?.id === 'romm-search';
    const value = focused ? active.value : null;
    const caret = focused ? active.selectionStart : null;
    this.context.render();
    if (!focused) return;
    const field = this.context.app?.querySelector?.('#romm-search');
    if (!field) return;
    if (value !== null && field.value !== value) field.value = value;
    field.focus();
    if (Number.isInteger(caret)) field.setSelectionRange?.(caret, caret);
  }

  /// Opening the RomM screen checks the remote once before reading the stored snapshot. A failed
  /// check remains terminal until the operator explicitly tests or refreshes the connection.
  ensureBrowsed() {
    const browse = state.rommBrowse;
    if (!state.rommSource?.configured || browse.loaded || browse.loading) return;
    const connection = state.rommSource.lastTest?.result;
    if (connection && connection !== 'reachable') return;
    if (connection === 'reachable') {
      void this.loadRoms();
      return;
    }
    void this.connectAndBrowse();
  }

  async connectAndBrowse() {
    setRommBrowse({ loading: true });
    this.context.render();
    if (!await this.testConnection()) {
      this.context.render();
      return;
    }
    await this.loadPlatforms();
    await this.loadRoms();
  }

  onRefresh = async () => {
    await this.refreshRemoteIndex();
  };

  async refreshRemoteIndex({ announce = true } = {}) {
    if (state.rommBrowse.refreshing) return false;
    const { platformId, search } = state.rommBrowse;
    this.browseRequestId += 1;
    setRommBrowse({ refreshing: true });
    this.context.render();
    try {
      if (!await this.testConnection()) return false;
      const status = await this.request('/api/admin/sources/romm/refresh', { method: 'POST' });
      setRommSourceStatus({ ...status, lastTest: state.rommSource?.lastTest });
      resetRommBrowse();
      setRommBrowse({ platformId, search, refreshing: true });
      await this.loadPlatforms();
      await this.loadRoms();
      if (announce) {
        this.context.setNotice(`Remote index refreshed: ${state.rommSource.index_game_count || 0} games.`);
      }
      return true;
    } catch (error) {
      this.connectionFailed(error);
      return false;
    } finally {
      setRommBrowse({ refreshing: false });
      this.context.render();
    }
  }

  onSearchInput = (event) => {
    const search = String(event.currentTarget.value || '');
    if (this.searchTimer) this.clearTimeout(this.searchTimer);
    this.searchTimer = this.setTimeout(() => {
      this.searchTimer = null;
      if (search.trim() === state.rommBrowse.search) return;
      setRommBrowse({ search: search.trim() });
      void this.loadRoms();
    }, SEARCH_DEBOUNCE_MS);
  };

  selectPlatform(platformId) {
    const next = String(platformId ?? '');
    if (next === String(state.rommBrowse.platformId)) return;
    setRommBrowse({ platformId: next });
    void this.loadRoms();
  }

  async loadRoms() {
    if (state.rommSource?.lastTest?.result !== 'reachable') return;
    const { platformId, search } = state.rommBrowse;
    const requestId = ++this.browseRequestId;
    setRommBrowse({ items: [], total: 0, loading: true });
    this.renderKeepingSearchFocus();

    try {
      const items = [];
      const seen = new Set();
      let offset = 0;
      let total = 0;
      do {
        const params = new URLSearchParams({
          limit: String(ROMM_BROWSE_PAGE_SIZE),
          offset: String(offset),
        });
        if (platformId) params.set('platform_id', platformId);
        if (search) params.set('search', search);

        const page = await this.request(`/api/admin/sources/romm/roms?${params}`);
        if (requestId !== this.browseRequestId) return;
        const pageItems = Array.isArray(page?.items) ? page.items : [];
        total = Math.max(0, Number(page?.total) || 0);
        if (!pageItems.length && offset < total) {
          throw new Error('Teatro returned an incomplete RomM index page.');
        }
        for (const rom of pageItems) {
          if (!seen.has(rom?.id)) {
            seen.add(rom?.id);
            items.push(rom);
          }
        }
        offset += pageItems.length;
      } while (offset < total);

      setRommBrowse({ items, total, loading: false, loaded: true });
    } catch (error) {
      if (requestId !== this.browseRequestId) return;
      setRommBrowse({ items: [], total: 0, loading: false, loaded: true });
      this.context.setError(error);
    }
    this.renderKeepingSearchFocus();
  }

  toggleCard(remoteRomId) {
    const rom = state.rommBrowse.items.find(({ id }) => id === remoteRomId);
    if (!rom) return;
    toggleRommSelection(rom);
    this.context.render();
  }

  selectShown = () => {
    selectRommItems(visibleRommItems());
    this.context.render();
  };

  /// The drawer is the only place a single game is edited. Its remote files are fetched once, on
  /// demand, rather than for every card in the grid.
  async openDrawer(remoteRomId) {
    if (!Number.isSafeInteger(remoteRomId)) return;
    openRommDrawer(remoteRomId);
    this.context.render();
    try {
      const detail = await this.request(`/api/admin/sources/romm/roms/${encodeURIComponent(remoteRomId)}`);
      if (state.rommDrawer?.romId === remoteRomId) loadRommDrawerDetail(detail);
    } catch (error) {
      closeRommDrawer();
      this.context.setError(error);
    }
    this.context.render();
  }

  /// Committing the drawer also selects the game: adjusting a target is how an operator says they
  /// want it.
  commitDrawer = () => {
    const drawer = state.rommDrawer;
    if (!drawer || drawer.loading || !drawer.platformId || !drawer.fileIds?.length) return;
    const rom = state.rommBrowse.items.find(({ id }) => id === drawer.romId);
    if (!findRommSelection(drawer.romId) && rom) toggleRommSelection(rom);
    const title = drawer.title.trim();
    updateRommSelectionEntry(drawer.romId, {
      titleOverride: title && title !== String(rom?.name || '') ? title : null,
      platformOverride: String(drawer.platformId),
      fileIds: [...drawer.fileIds],
    });
    closeRommDrawer();
    this.context.render();
  };

  startBatchImport = async () => {
    const totals = rommSelectionTotals();
    if (this.batchInFlight || !totals.games || totals.unmatched) return;
    this.batchInFlight = true;

    const queued = state.rommSelected.map((entry) => {
      const appJob = addJob({
        type: 'romm-import',
        title: entry.titleOverride || entry.name,
        detail: 'Importing from RomM',
        state: 'queued',
        statusText: 'Waiting for an import slot',
        progress: initialRommImportProgress(),
      });
      return { appJobId: appJob.id, entry };
    });
    this.importQueue.push(...queued);
    clearRommSelections();
    this.context.setNotice(`Queued ${queued.length} RomM import${queued.length === 1 ? '' : 's'}. Track ${queued.length === 1 ? 'it' : 'them'} in Jobs.`);
    this.context.render();
    this.batchInFlight = false;
    this.pumpImports();
  };

  pumpImports() {
    while (this.runningImports.size < IMPORT_CONCURRENCY && this.importQueue.length) {
      const next = this.importQueue.shift();
      this.runningImports.add(next.appJobId);
      void this.admitImport(next);
    }
  }

  releaseImportSlot(appJobId) {
    if (this.runningImports.delete(appJobId)) this.pumpImports();
  }

  /// One admission: Teatro resolves the remote file list server-side when the operator never chose
  /// files, then names those file IDs explicitly. The browser still dictates no URL, path, or size.
  async admitImport({ appJobId, entry }) {
    if (!findJob(appJobId)) {
      this.releaseImportSlot(appJobId);
      return;
    }
    updateJob(appJobId, { statusText: 'Submitting import' });
    this.context.jobChanged?.(appJobId);

    try {
      const fileIds = await this.resolveFileIds(entry);
      const title = entry.titleOverride?.trim();
      const created = validateRommImportCreateResponse(await this.request('/api/admin/sources/romm/imports', {
        method: 'POST',
        body: JSON.stringify({
          remote_rom_id: entry.id,
          remote_file_ids: fileIds,
          platform_id: Number(rommSelectionPlatformId(entry)),
          title: title || undefined,
        }),
      }));
      beginRommImportJob(appJobId, created.job, created.statusUrl);
      rememberServerJob({ type: 'romm-import', id: created.job.id, statusUrl: created.statusUrl });
      this.context.jobChanged?.(appJobId);
      this.poller.watch(appJobId, { immediate: true });
    } catch (error) {
      updateJob(appJobId, {
        state: 'failed',
        statusText: 'Import launch failed',
        error: error?.message || String(error),
        progress: { ...findJob(appJobId)?.progress, active: false },
      });
      this.context.setError(error);
      this.context.render();
      this.releaseImportSlot(appJobId);
    }
  }

  async resolveFileIds(entry) {
    if (entry.fileIds?.length) return entry.fileIds;
    const detail = await this.request(`/api/admin/sources/romm/roms/${encodeURIComponent(entry.id)}`);
    const fileIds = (Array.isArray(detail?.files) ? detail.files : [])
      .map(({ id }) => id)
      .filter((id) => Number.isSafeInteger(id));
    if (!fileIds.length) {
      throw new Error('The remote server published no importable files for this game.');
    }
    return fileIds;
  }

  shouldPoll(appJobId) {
    const job = findJob(appJobId);
    return Boolean(
      state.auth?.header
      && job?.type === 'romm-import'
      && job.serverJobId
      && !job.progress?.unavailable
      && !['succeeded', 'failed', 'cancelled'].includes(job.progress?.jobState),
    );
  }

  async onSnapshot(appJobId, snapshot, serverJobId) {
    validateRommImportSnapshot(snapshot, serverJobId);
    applyRommImportJobSnapshot(appJobId, snapshot);
    this.context.jobChanged?.(appJobId);
    if (!['succeeded', 'failed', 'cancelled'].includes(snapshot.state)) return false;

    forgetServerJob('romm-import', serverJobId);
    this.releaseImportSlot(appJobId);
    if (snapshot.state === 'cancelled') {
      updateJob(appJobId, { state: 'cancelled', statusText: 'Import cancelled' });
      this.context.render();
      return true;
    }
    if (snapshot.state === 'failed') {
      this.context.setError(new Error(snapshot.error?.message || 'RomM import failed.'));
      this.context.render();
      return true;
    }

    const result = snapshot.result;
    if (result.outcome === 'already_present') {
      // A refused duplicate is a normal, plainly-worded outcome rather than an error.
      markRommItemImported(result.remote_rom_id, 'Teatro refused this import as already present.');
      this.context.setNotice(`“${result.title}” is already in Teatro; nothing was imported.`);
      this.context.render();
      return true;
    }
    markRommItemImported(result.remote_rom_id, 'Imported into Teatro from this browser session.');

    let refreshError = null;
    try {
      await Promise.all([
        this.context.loadPlatforms(), this.context.loadStats(), this.context.loadRoms(),
      ]);
    } catch (error) {
      refreshError = error;
    }
    const conflicts = result.hash_conflicts?.length || 0;
    if (refreshError) {
      updateJob(appJobId, { error: `Library refresh failed: ${refreshError.message}` });
      this.context.setError(new Error(`The import completed, but refreshing the library failed: ${refreshError.message}`));
    } else if (conflicts) {
      // The bytes landed and Teatro stored the digests it computed, so this is a warning about the
      // remote server's claim rather than a failed import.
      this.context.setNotice(`Imported “${result.title}” with ${conflicts} hash warning${conflicts === 1 ? '' : 's'}: the downloaded bytes did not match what RomM declared. Teatro stored the hashes it computed — see Jobs.`);
    } else {
      this.context.setNotice(`Imported “${result.title}” (${result.file_count} file${result.file_count === 1 ? '' : 's'}) into Teatro.`);
    }
    this.context.render();
    return true;
  }

  async onPollError(appJobId, error, failures) {
    const job = findJob(appJobId);
    if (!job) return false;
    if (error?.status === 404) {
      forgetServerJob('romm-import', job.serverJobId);
      markRommImportJobUnavailable(
        appJobId,
        'This in-memory import job is unavailable because it expired or Teatro restarted. Refresh the library to check whether the game arrived.',
      );
      this.releaseImportSlot(appJobId);
      this.context.render();
      return false;
    }
    if (error?.status === 401 || error?.status === 403 || !state.auth?.header) {
      forgetServerJob('romm-import', job.serverJobId);
      markRommImportJobUnavailable(appJobId, 'Admin access to this RomM import is no longer available.');
      this.releaseImportSlot(appJobId);
      this.context.render();
      return false;
    }
    setRommImportPollingFailure(
      appJobId,
      'Connection to Teatro was interrupted. Retrying this existing import without creating another.',
      failures,
    );
    this.context.jobChanged?.(appJobId);
    return true;
  }

  cancelJob = async (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'romm-import' || !['queued', 'running'].includes(job.state)) return;
    // A game still waiting for a slot has no server job to cancel; dropping it from the queue is
    // the whole cancellation.
    if (!job.serverJobId) {
      this.importQueue = this.importQueue.filter((queued) => queued.appJobId !== appJobId);
      this.releaseImportSlot(appJobId);
      updateJob(appJobId, {
        state: 'cancelled',
        statusText: 'Import cancelled before submission',
        progress: { ...job.progress, active: false },
      });
      this.context.setNotice('Queued RomM import cancelled.');
      this.context.render();
      return;
    }
    this.poller.stop(appJobId, { remove: true });
    const snapshot = validateRommImportSnapshot(
      await this.request(job.statusUrl, { method: 'DELETE' }),
      job.serverJobId,
    );
    if (snapshot.state !== 'cancelled') {
      await this.onSnapshot(appJobId, snapshot, job.serverJobId);
      return;
    }
    applyRommImportJobSnapshot(appJobId, snapshot);
    forgetServerJob('romm-import', job.serverJobId);
    updateJob(appJobId, { state: 'cancelled', statusText: 'Import cancelled' });
    this.releaseImportSlot(appJobId);
    this.context.setNotice('RomM import cancelled.');
    this.context.render();
  };

  stopWatching = (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'romm-import') return;
    this.poller.stop(appJobId, { remove: true });
    forgetServerJob('romm-import', job.serverJobId);
    this.importQueue = this.importQueue.filter((queued) => queued.appJobId !== appJobId);
    this.releaseImportSlot(appJobId);
    removeJob(appJobId);
    this.context.render();
  };

  stopAll({ clearStorage = true } = {}) {
    this.poller.stopAll();
    if (this.searchTimer) this.clearTimeout(this.searchTimer);
    this.searchTimer = null;
    this.browseRequestId += 1;
    this.importQueue = [];
    this.runningImports.clear();
    if (clearStorage) clearServerJobs('romm-import');
  }
}
