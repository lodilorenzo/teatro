import test from 'node:test';
import assert from 'node:assert/strict';

import {
  RommSourceController, validateRommImportCreateResponse, validateRommImportSnapshot,
} from './romm-source.js';
import { clearRommCovers, syncRommCovers } from './romm-covers.js';
import { SERVER_JOB_STORAGE_KEY, decodeStoredServerJobs } from './server-jobs.js';
import {
  ROMM_BROWSE_PAGE_SIZE, clearRommSelections, selectRommItems, setRommBrowse,
  setRommHideExisting, setRommPlatforms,
  setRommSourceEditing, setRommSourceStatus, state, toggleRommSelection,
} from '../state.js';
import { renderJobCardBody } from '../views/jobs.js';
import { renderRommBrowse } from '../views/romm-browse.js';
import { renderRommSource } from '../views/romm-source.js';

function storage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

function created(id = 'romm_test') {
  return {
    job: { id, state: 'queued', phase: 'queued', progress: null },
    status_url: `/api/admin/sources/romm/imports/${id}`,
  };
}

function succeeded(id = 'romm_test', overrides = {}) {
  return {
    id,
    state: 'succeeded',
    phase: 'complete',
    progress: null,
    result: {
      outcome: 'imported',
      remote_rom_id: 7,
      title: 'Example Game',
      platform_id: 3,
      platform_slug: 'snes',
      file_count: 1,
      downloaded_bytes: 1024,
      hash_signals: ['sha256'],
      imported_rom_ids: [12],
      imported_rom_name: 'Example Game',
      duplicate: null,
      ...overrides,
    },
    error: null,
  };
}

function remoteRom(id, overrides = {}) {
  return {
    id,
    name: `Remote Game ${id}`,
    platform_slug: 'snes',
    platform_name: 'Super Nintendo',
    file_count: 1,
    file_size_bytes: 1024,
    has_cover: false,
    already_present: false,
    already_present_reason: null,
    target_platform_id: 3,
    target_platform_name: 'Super Nintendo Entertainment System',
    ...overrides,
  };
}

function configuredSource(overrides = {}) {
  setRommSourceStatus({
    enabled: true,
    configured: true,
    base_url: 'https://romm.example/',
    username: 'teatro',
    auth_mode: 'token',
    secret_configured: true,
    plaintext_http: false,
    index_refreshed_at: '2026-08-06T00:00:00Z',
    index_game_count: 1,
    index_platform_count: 1,
    lastTest: { result: 'reachable', message: 'Connected to RomM.' },
    ...overrides,
  });
}

function preserve() {
  return {
    auth: state.auth,
    jobs: state.jobs,
    screen: state.screen,
    platforms: state.platforms,
    rommSource: state.rommSource,
    rommSourceEditing: state.rommSourceEditing,
    rommSourceDetailOpen: state.rommSourceDetailOpen,
    rommPlatforms: state.rommPlatforms,
    rommBrowse: state.rommBrowse,
    rommSelected: state.rommSelected,
    rommHideExisting: state.rommHideExisting,
    rommDrawer: state.rommDrawer,
  };
}

function restore(previous) {
  Object.assign(state, previous);
}

function noopContext(overrides = {}) {
  return {
    app: { querySelector: () => null, querySelectorAll: () => [] },
    render: () => {},
    setNotice: () => {},
    setError: () => {},
    jobChanged: () => {},
    loadPlatforms: async () => {},
    loadStats: async () => {},
    loadRoms: async () => {},
    setTimeout: () => 1,
    clearTimeout: () => {},
    ...overrides,
  };
}

function flush() {
  return new Promise((resolve) => { setTimeout(resolve, 0); });
}

function rommJobs() {
  return state.jobs.filter(({ type }) => type === 'romm-import');
}

test('RomM import create and terminal resources are validated before use', () => {
  assert.equal(validateRommImportCreateResponse(created()).job.id, 'romm_test');
  assert.throws(
    () => validateRommImportCreateResponse({ ...created(), status_url: '/api/admin/evil' }),
    /invalid RomM import status URL/,
  );
  assert.equal(validateRommImportSnapshot(succeeded(), 'romm_test').state, 'succeeded');
  assert.throws(
    () => validateRommImportSnapshot(succeeded('romm_test', { outcome: 'sideloaded' }), 'romm_test'),
    /invalid RomM import result/,
  );
  assert.throws(
    () => validateRommImportSnapshot({ ...succeeded(), phase: 'teleporting' }, 'romm_test'),
    /invalid RomM import job resource/,
  );
});

test('a declared-hash conflict is a reported warning on a completed import', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = storage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  clearRommSelections();
  configuredSource();

  const conflicts = [
    { file_name: 'game.sfc', algorithm: 'sha256', declared: 'a'.repeat(64), computed: 'd'.repeat(64) },
    { file_name: 'game.sfc', algorithm: 'crc32', declared: '0000ffff', computed: '1234abcd' },
  ];
  // The snapshot is still validated: a report that is not Teatro-shaped is refused.
  assert.equal(
    validateRommImportSnapshot(succeeded('romm_test', { hash_conflicts: conflicts }), 'romm_test').state,
    'succeeded',
  );
  assert.equal(validateRommImportSnapshot(succeeded(), 'romm_test').state, 'succeeded');
  assert.throws(
    () => validateRommImportSnapshot(succeeded('romm_test', {
      hash_conflicts: [{ file_name: 'game.sfc', algorithm: 'sha256', declared: 'not hex', computed: 'd'.repeat(64) }],
    }), 'romm_test'),
    /invalid RomM import hash report/,
  );
  assert.throws(
    () => validateRommImportSnapshot(succeeded('romm_test', {
      hash_conflicts: [{ file_name: 'game.sfc', algorithm: 'rot13', declared: 'a'.repeat(64), computed: 'd'.repeat(64) }],
    }), 'romm_test'),
    /invalid RomM import hash report/,
  );

  const notices = [];
  const errors = [];
  const controller = new RommSourceController(noopContext({
    request: async (path) => (/\/roms\/\d+$/u.test(path)
      ? { rom: remoteRom(7), files: [{ id: 11, file_name: 'game.sfc', file_size_bytes: 1024, hash_signals: [] }], total_size_bytes: 1024 }
      : created()),
    setNotice: (message) => notices.push(message),
    setError: (error) => errors.push(error?.message || String(error)),
  }));

  setRommBrowse({ loaded: true, total: 1, items: [remoteRom(7)] });
  toggleRommSelection(remoteRom(7));
  await controller.startBatchImport();
  await flush();
  const job = rommJobs()[0];
  await controller.onSnapshot(job.id, succeeded('romm_test', { hash_conflicts: conflicts }), 'romm_test');

  // The import succeeded, so this is a warning rather than an error.
  assert.deepEqual(errors, []);
  assert.match(notices.at(-1), /2 hash warnings/);
  assert.match(notices.at(-1), /stored the hashes it computed/);

  const card = renderJobCardBody(state.jobs.find(({ id }) => id === job.id));
  assert.match(card, /2 declared hashes did not match the downloaded bytes/);
  assert.match(card, /sha256 declared/);
  assert.match(card, new RegExp(`${'a'.repeat(64)}`));
  assert.match(card, new RegExp(`${'d'.repeat(64)}`));
  assert.match(card, /crc32 declared/);
  assert.match(card, /0000ffff/);
  assert.match(card, /1234abcd/);
  // The card is styled as a warning, not as a plain success.
  assert.match(card, /job-result warning/);

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('a batch admits imports up to the concurrency ceiling and queues the rest', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  const fakeStorage = storage();
  globalThis.sessionStorage = fakeStorage;
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  clearRommSelections();
  configuredSource();

  const requested = [];
  const controller = new RommSourceController(noopContext({
    request: async (path) => {
      requested.push(path);
      const detail = path.match(/^\/api\/admin\/sources\/romm\/roms\/(\d+)$/u);
      if (detail) {
        return {
          rom: remoteRom(Number(detail[1])),
          files: [{ id: Number(detail[1]) * 100, file_name: 'game.sfc', file_size_bytes: 1024, hash_signals: [] }],
          total_size_bytes: 1024,
        };
      }
      return created(`romm_${requested.filter((entry) => entry.endsWith('/imports')).length}`);
    },
  }));

  const items = Array.from({ length: 52 }, (_, index) => remoteRom(index + 7));
  setRommBrowse({ loaded: true, total: items.length, items });
  selectRommItems(items);
  assert.equal(state.rommSelected.length, 52);

  await controller.startBatchImport();
  await flush();

  // Every selected game is visible in Jobs immediately, but only two hold a remote download.
  assert.equal(rommJobs().length, 52);
  assert.equal(state.rommSelected.length, 0);
  const admitted = rommJobs().filter(({ serverJobId }) => serverJobId);
  assert.equal(admitted.length, 2);
  const waiting = rommJobs().find(({ serverJobId }) => !serverJobId);
  assert.equal(waiting.statusText, 'Waiting for an import slot');
  // Files were never dictated by the browser: each admission resolved them from the remote detail.
  assert.equal(requested.filter((path) => /\/roms\/\d+$/u.test(path)).length, 2);

  // Finishing one admitted import frees its slot for the next queued game.
  await controller.onSnapshot(
    admitted[0].id,
    succeeded(admitted[0].serverJobId, { remote_rom_id: 7 }),
    admitted[0].serverJobId,
  );
  await flush();
  assert.equal(rommJobs().filter(({ serverJobId }) => serverJobId).length, 3);
  assert.deepEqual(
    decodeStoredServerJobs(fakeStorage.getItem(SERVER_JOB_STORAGE_KEY)).map(({ type }) => type),
    ['romm-import', 'romm-import'],
  );

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('a committed drawer override replaces the resolved files, platform, and title', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = storage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  state.platforms = [{ id: 5, slug: 'psx', display_name: 'PlayStation' }];
  clearRommSelections();
  configuredSource();

  const bodies = [];
  const controller = new RommSourceController(noopContext({
    request: async (path, options) => {
      if (options?.method === 'POST') {
        bodies.push(JSON.parse(options.body));
        return created('romm_override');
      }
      return {
        rom: remoteRom(21, { name: 'Disc Game', target_platform_id: null, target_platform_name: '' }),
        files: [
          { id: 1, file_name: 'disc1.chd', file_size_bytes: 8, hash_signals: ['md5'] },
          { id: 2, file_name: 'disc2.chd', file_size_bytes: 8, hash_signals: [] },
        ],
        total_size_bytes: 16,
      };
    },
  }));

  const rom = remoteRom(21, { name: 'Disc Game', file_count: 2, target_platform_id: null, target_platform_name: '' });
  setRommBrowse({ loaded: true, total: 1, items: [rom] });

  await controller.openDrawer(21);
  assert.equal(state.rommDrawer.loading, false);
  // An untouched drawer offers every remote file.
  assert.deepEqual(state.rommDrawer.fileIds, [1, 2]);
  // An unmatched platform cannot be committed, so the drawer stays open and nothing is queued.
  controller.commitDrawer();
  assert.notEqual(state.rommDrawer, null);
  assert.equal(state.rommSelected.length, 0);
  assert.match(renderRommBrowse(), /data-action="romm-commit-drawer" disabled/);

  // Committing an unselected game selects it: adjusting a target is how an operator asks for it.
  state.rommDrawer.platformId = '5';
  state.rommDrawer.title = 'Disc Game (Rev 1)';
  state.rommDrawer.fileIds = [2];
  controller.commitDrawer();

  assert.equal(state.rommSelected.length, 1);
  assert.deepEqual(state.rommSelected[0].fileIds, [2]);
  assert.equal(state.rommSelected[0].platformOverride, '5');
  assert.equal(state.rommSelected[0].titleOverride, 'Disc Game (Rev 1)');

  await controller.startBatchImport();
  await flush();
  assert.deepEqual(bodies, [{
    remote_rom_id: 21,
    remote_file_ids: [2],
    platform_id: 5,
    title: 'Disc Game (Rev 1)',
  }]);

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('a refused duplicate is reported as a plain skip and marks its browsed card', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = storage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  clearRommSelections();
  configuredSource();

  const notices = [];
  const errors = [];
  const controller = new RommSourceController(noopContext({
    request: async (path) => (/\/roms\/\d+$/u.test(path)
      ? { rom: remoteRom(7), files: [{ id: 11, file_name: 'game.sfc', file_size_bytes: 1024, hash_signals: [] }], total_size_bytes: 1024 }
      : created()),
    setNotice: (message) => notices.push(message),
    setError: (error) => errors.push(error),
  }));

  setRommBrowse({ loaded: true, total: 1, items: [remoteRom(7)] });
  toggleRommSelection(remoteRom(7));
  await controller.startBatchImport();
  await flush();
  const job = rommJobs()[0];

  await controller.onSnapshot(job.id, succeeded('romm_test', {
    outcome: 'already_present',
    downloaded_bytes: 0,
    imported_rom_ids: [],
    duplicate: {
      rule: 'content_hash',
      detail: 'The sha256 of a selected file matches “Example Game”.',
      existing_rom_id: 12,
      existing_rom_name: 'Example Game',
    },
  }), 'romm_test');

  assert.equal(errors.length, 0);
  assert.match(notices.at(-1), /already in Teatro/);
  const card = renderJobCardBody(state.jobs.find(({ id }) => id === job.id));
  assert.match(card, /Skipped as already present/);
  assert.match(card, /content_hash/);
  // The browsed card stops offering the game without spending another remote request.
  assert.equal(state.rommBrowse.items[0].already_present, true);

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('untrusted remote strings are escaped and an existing game cannot be selected', () => {
  const previous = preserve();
  state.screen = 'romm';
  state.platforms = [{ id: 3, slug: 'snes', display_name: 'SNES' }];
  clearRommSelections();
  setRommHideExisting(false);
  configuredSource({ base_url: 'http://192.168.1.9:8080/', plaintext_http: true });
  setRommPlatforms([{ id: 4, slug: 'snes', name: '<b>SNES</b>', rom_count: 12 }]);
  setRommBrowse({
    loaded: true,
    total: 1,
    items: [remoteRom(7, {
      name: '<img src=x onerror=alert(1)>',
      already_present: true,
      already_present_reason: '<script>pwn()</script>',
    })],
  });

  const markup = renderRommBrowse();
  assert.equal(markup.includes('<img src=x'), false);
  assert.equal(markup.includes('<script>pwn()'), false);
  // The remote platform list is one dropdown, and its untrusted names are escaped like every other
  // remote string.
  assert.match(markup, /<select data-romm-platform/);
  assert.equal(markup.includes('<b>SNES</b>'), false);
  assert.match(markup, /&lt;b&gt;SNES&lt;\/b&gt; \(12\)/);
  assert.match(markup, /&lt;img src=x/);
  // The advisory duplicate marker disables the selection control; enforcement is server-side.
  assert.match(markup, /data-romm-toggle="7" aria-pressed="false" disabled/);
  assert.match(markup, /In Teatro/);
  // A game already in Teatro offers no drawer, so nothing invites an override that cannot run.
  assert.equal(markup.includes('data-romm-drawer="7"'), false);
  assert.match(markup, /Plaintext HTTP/);
  assert.match(markup, /class="romm-list"/);
  assert.doesNotMatch(markup, /class="romm-grid"/);
  assert.equal(markup.includes('Games per load'), false);
  assert.equal(markup.includes('data-romm-page-size'), false);

  // Selection refuses the same game programmatically, not only in the markup.
  assert.equal(toggleRommSelection(state.rommBrowse.items[0]), false);
  assert.equal(state.rommSelected.length, 0);

  restore(previous);
});

test('the batch bar totals a selection, warns on unmatched targets, and accepts the full listing', () => {
  const previous = preserve();
  state.screen = 'romm';
  state.platforms = [{ id: 3, slug: 'snes', display_name: 'SNES' }];
  clearRommSelections();
  setRommHideExisting(true);
  configuredSource();

  const items = [
    remoteRom(1, { file_count: 2, file_size_bytes: 2048 }),
    remoteRom(2, { target_platform_id: null, target_platform_name: '' }),
    remoteRom(3, { already_present: true, already_present_reason: 'present' }),
  ];
  setRommBrowse({ loaded: true, total: 3, items });

  // The advisory duplicate is hidden by default, so "shown" counts what can be acted on.
  let markup = renderRommBrowse();
  assert.match(markup, /2 shown/);

  toggleRommSelection(items[0]);
  toggleRommSelection(items[1]);
  markup = renderRommBrowse();
  assert.match(markup, /2 games selected/);
  assert.match(markup, /3 files/);
  assert.match(markup, /1 game needs a target platform/);
  assert.match(markup, /data-action="romm-import" disabled/);
  assert.match(markup, /Choose a platform/);

  clearRommSelections();
  const fullListing = Array.from({ length: 75 }, (_, index) => remoteRom(index + 100));
  assert.equal(selectRommItems(fullListing), 75);
  assert.equal(state.rommSelected.length, 75);
  assert.match(renderRommBrowse(), /75 selected/);

  clearRommSelections();
  restore(previous);
});

test('the settings card keeps the connection only and points at the RomM section', () => {
  const previous = preserve();
  setRommSourceEditing(false);
  setRommSourceStatus({
    enabled: true,
    configured: true,
    base_url: 'https://romm.example/',
    username: 'teatro',
    auth_mode: 'basic',
    secret_configured: true,
    plaintext_http: false,
    updated_at: '2026-07-25T10:00:00Z',
  });

  const summary = renderRommSource();
  assert.match(summary, /Browse RomM/);
  assert.match(summary, /romm\.example/);
  assert.match(summary, /HTTP Basic/);
  assert.match(summary, /Stored, hidden/);
  assert.match(summary, /data-nav="romm"/);
  assert.equal(summary.includes('name="base_url"'), false);
  assert.equal(summary.includes('name="secret"'), false);
  // Browsing no longer lives in Settings.
  assert.equal(summary.includes('romm-grid'), false);
  assert.equal(summary.includes('data-romm-toggle'), false);

  setRommSourceEditing(true);
  const editing = renderRommSource();
  assert.match(editing, /name="base_url"/);
  assert.match(editing, /value="https:\/\/romm\.example\/"/);
  assert.match(editing, /Leave blank to keep the stored secret/);
  assert.match(editing, /data-action="cancel-romm-source-edit"/);

  // An unconfigured source has no summary to fall back to, so its inputs stay visible.
  setRommSourceStatus({ enabled: true, configured: false, auth_mode: 'token', secret_configured: false, plaintext_http: false });
  assert.equal(state.rommSourceEditing, false);
  assert.match(renderRommSource(), /name="base_url"/);

  restore(previous);
});

test('the RomM screen refuses to browse until a connection is saved', () => {
  const previous = preserve();
  state.screen = 'romm';
  setRommSourceStatus({ enabled: true, configured: false, auth_mode: 'token', secret_configured: false, plaintext_http: false });
  const unconfigured = renderRommBrowse();
  assert.match(unconfigured, /No RomM server configured/);
  assert.match(unconfigured, /data-nav="settings"/);
  assert.equal(unconfigured.includes('romm-searchbar'), false);

  setRommSourceStatus(null);
  const disabled = renderRommBrowse();
  assert.match(disabled, /TEATRO_ROMM_SOURCE_ENABLED/);
  assert.equal(disabled.includes('data-nav="settings"'), false);

  restore(previous);
});

test('an unreachable RomM server blocks the list without automatic reconnects', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = storage();
  state.auth = { header: 'Basic test' };
  state.screen = 'romm';
  configuredSource({ lastTest: undefined });
  setRommPlatforms([{ id: 4, slug: 'snes', name: 'Super Nintendo', rom_count: 1 }]);
  setRommBrowse({ items: [remoteRom(7)], total: 1, loading: false, loaded: false });

  const requested = [];
  const errors = [];
  const controller = new RommSourceController(noopContext({
    request: async (path) => {
      requested.push(path);
      if (path.endsWith('/test')) {
        return { result: 'unreachable', message: 'The configured RomM server did not respond.' };
      }
      throw new Error(`unexpected request: ${path}`);
    },
    setError: (error) => errors.push(error?.message || String(error)),
  }));

  controller.ensureBrowsed();
  await flush();
  assert.deepEqual(requested, ['/api/admin/sources/romm/test']);
  assert.equal(state.rommSource.lastTest.result, 'unreachable');
  assert.deepEqual(state.rommPlatforms, []);
  assert.deepEqual(state.rommBrowse.items, []);
  assert.equal(state.rommBrowse.loaded, false);
  assert.deepEqual(errors, ['The configured RomM server did not respond.']);
  const markup = renderRommBrowse();
  assert.match(markup, /RomM connection failed/);
  assert.match(markup, /The configured RomM server did not respond/);
  assert.doesNotMatch(markup, /romm-searchbar|data-romm-cover/);

  controller.ensureBrowsed();
  await flush();
  assert.equal(requested.length, 1);

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('RomM browse controls automatically load every indexed game', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = storage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  configuredSource();
  setRommBrowse({ platformId: '', search: '', items: [], loaded: false, total: 0 });

  const requested = [];
  const timers = [];
  const controller = new RommSourceController(noopContext({
    request: async (path) => {
      requested.push(path);
      const offset = Number(new URL(`https://teatro.test${path}`).searchParams.get('offset'));
      if (path.includes('search=chrono')) {
        const items = offset === 0
          ? Array.from({ length: ROMM_BROWSE_PAGE_SIZE }, (_, index) => remoteRom(index + 1))
          : [remoteRom(ROMM_BROWSE_PAGE_SIZE + 1)];
        return { items, total: ROMM_BROWSE_PAGE_SIZE + 1, limit: ROMM_BROWSE_PAGE_SIZE, offset };
      }
      return { items: [remoteRom(1)], total: 1, limit: ROMM_BROWSE_PAGE_SIZE, offset };
    },
    setTimeout: (callback) => {
      timers.push(callback);
      return timers.length;
    },
    clearTimeout: (timer) => { timers[timer - 1] = null; },
  }));

  // Three keystrokes leave one pending search, and the debounce only fires the last value.
  controller.onSearchInput({ currentTarget: { value: 'chr' } });
  controller.onSearchInput({ currentTarget: { value: 'chro' } });
  controller.onSearchInput({ currentTarget: { value: 'chrono ' } });
  assert.equal(requested.length, 0);
  timers.filter(Boolean).forEach((callback) => callback());
  await flush();
  assert.equal(requested.length, 2);
  assert.match(requested[0], /search=chrono/);
  assert.match(requested[0], new RegExp(`limit=${ROMM_BROWSE_PAGE_SIZE}`));
  assert.match(requested[0], /offset=0/);
  assert.match(requested[1], new RegExp(`offset=${ROMM_BROWSE_PAGE_SIZE}`));
  assert.equal(state.rommBrowse.search, 'chrono');
  assert.equal(state.rommBrowse.items.length, ROMM_BROWSE_PAGE_SIZE + 1);

  controller.selectPlatform('4');
  await flush();
  assert.match(requested[2], /platform_id=4/);
  assert.match(requested[2], /offset=0/);
  assert.match(requested[3], /platform_id=4/);
  assert.match(requested[3], new RegExp(`offset=${ROMM_BROWSE_PAGE_SIZE}`));
  // Re-selecting the platform already applied is not a new request.
  controller.selectPlatform('4');
  await flush();
  assert.equal(requested.length, 4);

  // A repaint that lands mid-typing keeps the field focused, its caret, and later keystrokes.
  const field = {
    id: 'romm-search',
    value: 'chrono tr',
    selectionStart: 9,
    focused: false,
    range: null,
    focus() { this.focused = true; },
    setSelectionRange(start, end) { this.range = [start, end]; },
  };
  const previousDocument = globalThis.document;
  globalThis.document = { activeElement: field };
  const focusController = new RommSourceController(noopContext({
    app: { querySelector: (selector) => (selector === '#romm-search' ? field : null), querySelectorAll: () => [] },
    render: () => { field.value = 'chrono'; },
    request: async () => ({ items: [], total: 0, limit: 24, offset: 0 }),
  }));
  focusController.renderKeepingSearchFocus();
  globalThis.document = previousDocument;
  assert.equal(field.value, 'chrono tr');
  assert.equal(field.focused, true);
  assert.deepEqual(field.range, [9, 9]);
  focusController.stopAll({ clearStorage: false });

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  restore(previous);
});

test('browse cards carry a cover placeholder that only the authenticated hydrator can fill', async () => {
  const previous = preserve();
  state.screen = 'romm';
  clearRommSelections();
  setRommHideExisting(true);
  configuredSource();
  setRommBrowse({
    loaded: true,
    total: 2,
    items: [
      remoteRom(7, { name: 'With Cover', has_cover: true, cover_url: '/api/admin/sources/romm/roms/7/cover' }),
      remoteRom(8, { name: 'No Cover', has_cover: false, cover_url: null }),
    ],
  });

  const markup = renderRommBrowse();
  // Both rows render an image so the list keeps one shape; only the game the remote server has a
  // cover for is marked for hydration.
  assert.equal(markup.match(/class="romm-cover"/gu).length, 2);
  assert.match(markup, /data-romm-cover="7"/);
  assert.equal(markup.includes('data-romm-cover="8"'), false);
  const firstRow = markup.slice(markup.indexOf('data-romm-toggle="7"'), markup.indexOf('</article>'));
  assert.ok(firstRow.indexOf('class="romm-check"') < firstRow.indexOf('class="romm-cover"'));
  assert.ok(firstRow.indexOf('class="romm-cover"') < firstRow.indexOf('class="romm-card-copy"'));
  assert.match(firstRow, /<small>Size<\/small><strong>1\.0 KiB<\/strong>/);
  assert.match(firstRow, /<small>Destination<\/small>\s*<strong>Super Nintendo Entertainment System<\/strong>/);
  assert.match(firstRow, /class="romm-edit-button"[^>]*aria-label="Edit title, platform, and files for With Cover"/);
  assert.equal(firstRow.includes('Title, platform &amp; files…'), false);
  // The proxy URL is rebuilt from the remote id, never taken from the response body.
  assert.equal(markup.includes('/api/admin/sources/romm/roms/7/cover'), false);

  const requested = [];
  const previousFetch = globalThis.fetch;
  const previousUrl = globalThis.URL;
  globalThis.fetch = async (path, options) => {
    requested.push([path, options?.headers?.Authorization]);
    return { ok: true, blob: async () => ({}) };
  };
  globalThis.URL = { createObjectURL: () => 'blob:romm-cover', revokeObjectURL: () => {} };
  const image = {
    dataset: { rommCover: '7' },
    isConnected: true,
    src: '/admin/game-cover-placeholder.jpg',
  };

  try {
    clearRommCovers();
    // Without a credential nothing is requested: the proxy route is admin-only.
    syncRommCovers({ querySelectorAll: () => [image] }, null);
    await flush();
    assert.deepEqual(requested, []);
    assert.equal(image.src, '/admin/game-cover-placeholder.jpg');

    syncRommCovers({ querySelectorAll: () => [image] }, 'Basic test');
    await flush();
    assert.deepEqual(requested, [['/api/admin/sources/romm/roms/7/cover', 'Basic test']]);
    assert.equal(image.src, 'blob:romm-cover');

    // A second pass reuses the cached object URL instead of refetching.
    syncRommCovers({ querySelectorAll: () => [image] }, 'Basic test');
    await flush();
    assert.equal(requested.length, 1);
  } finally {
    clearRommCovers();
    globalThis.fetch = previousFetch;
    globalThis.URL = previousUrl;
  }

  restore(previous);
});
