import test from 'node:test';
import assert from 'node:assert/strict';

import {
  LibraryScanController, validateLibraryScanCreateResponse,
  validateLibraryScanSnapshot,
} from './library-scan.js';
import { createServerJobPoller, SERVER_JOB_STORAGE_KEY } from './server-jobs.js';
import { state } from '../state.js';
import { renderJobCardBody } from '../views/jobs.js';

function storage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

function created(id = 'scan_test') {
  return {
    job: {
      id, state: 'queued', phase: 'queued', progress: null,
    },
    status_url: `/api/admin/library/scans/${id}`,
  };
}

function succeeded(id = 'scan_test') {
  return {
    id,
    state: 'succeeded',
    phase: 'complete',
    progress: null,
    result: {
      scanned_file_count: 3,
      already_indexed_file_count: 1,
      imported_rom_count: 1,
      imported_file_count: 1,
      generated_manifest_count: 0,
      cover_downloaded_count: 0,
      cover_not_downloaded_count: 1,
      cover_warnings: [{
        rom_id: 5, rom_name: '<img onerror=alert(1)>', reason: '<script>cover failed</script>',
      }],
      not_imported_file_count: 1,
      unimported_file_count: 2,
      imported_roms: [{ id: 5, name: 'Example', platform_slug: 'psx', file_count: 1 }],
      imported_roms_truncated: false,
      unimported_files: [
        {
          relative_path: 'psx/<script>.cue', disposition: 'not_imported',
          reason_code: 'missing_disc_file', reason: '<b>Disc 2 is missing</b>',
        },
        {
          relative_path: 'nes/Existing.nes', disposition: 'already_indexed',
          reason_code: 'already_indexed', reason: 'Already indexed.',
        },
      ],
    },
    error: null,
  };
}

function preserve() {
  return { auth: state.auth, jobs: state.jobs, screen: state.screen };
}

function restore(previous) {
  state.auth = previous.auth;
  state.jobs = previous.jobs;
  state.screen = previous.screen;
}

test('library scan create and terminal resources are validated before use', () => {
  assert.equal(validateLibraryScanCreateResponse(created()).job.id, 'scan_test');
  assert.equal(validateLibraryScanSnapshot(succeeded(), 'scan_test').state, 'succeeded');
  assert.throws(
    () => validateLibraryScanSnapshot({ ...succeeded(), result: { ...succeeded().result, unimported_file_count: 1 } }, 'scan_test'),
    /invalid library scan result/,
  );
  assert.throws(
    () => validateLibraryScanSnapshot({ ...succeeded(), result: { ...succeeded().result, unimported_files: [{
      relative_path: '/private/root/game.rom', disposition: 'not_imported', reason_code: 'bad', reason: 'bad',
    }], scanned_file_count: 2, already_indexed_file_count: 0, unimported_file_count: 1, not_imported_file_count: 1 } }, 'scan_test'),
    /unsafe library scan file result/,
  );
});

test('one scan click admits one server job and stores only opaque reattachment data', async () => {
  const previous = preserve();
  const previousStorage = globalThis.sessionStorage;
  const previousConfirm = globalThis.confirm;
  const fakeStorage = storage();
  globalThis.sessionStorage = fakeStorage;
  globalThis.confirm = () => true;
  state.auth = { header: 'Basic test' };
  state.jobs = [];

  let resolveRequest;
  let requestCount = 0;
  const request = () => {
    requestCount += 1;
    return new Promise((resolve) => { resolveRequest = resolve; });
  };
  const button = {
    disabled: false,
    addEventListener(_name, listener) { this.listener = listener; },
  };
  const app = {
    querySelector: (selector) => selector === '[data-action="scan-library"]' ? button : null,
  };
  const scheduled = [];
  const controller = new LibraryScanController({
    app,
    request,
    render: () => {},
    setNotice: () => {},
    setError: () => {},
    jobChanged: () => {},
    loadPlatforms: async () => {},
    loadStats: async () => {},
    loadRoms: async () => {},
    setTimeout: (callback, delay) => { scheduled.push({ callback, delay }); return scheduled.length; },
    clearTimeout: () => {},
  });
  controller.bind();
  const first = button.listener();
  const second = button.listener();
  assert.equal(requestCount, 1);
  assert.equal(state.jobs.filter(({ type }) => type === 'library-scan').length, 1);

  resolveRequest(created());
  await Promise.all([first, second]);
  const stored = JSON.parse(fakeStorage.getItem(SERVER_JOB_STORAGE_KEY));
  assert.deepEqual(stored, [{
    type: 'library-scan', id: 'scan_test', statusUrl: '/api/admin/library/scans/scan_test',
  }]);
  assert.equal(JSON.stringify(stored).includes('result'), false);
  assert.equal(scheduled[0].delay, 0);

  controller.stopAll({ clearStorage: true });
  globalThis.sessionStorage = previousStorage;
  globalThis.confirm = previousConfirm;
  restore(previous);
});

test('scan polling requests the latest snapshot without an event cursor', async () => {
  const scheduled = [];
  const requested = [];
  const job = {
    id: 'scan-app', serverJobId: 'scan_test', statusUrl: '/api/admin/library/scans/scan_test',
  };
  const poller = createServerJobPoller({
    request: async (url) => { requested.push(url); return succeeded(); },
    setTimeout: (callback) => { scheduled.push(callback); return scheduled.length; },
    clearTimeout: () => {},
    getJob: () => job,
    shouldPoll: () => true,
    onSnapshot: async () => true,
    onError: async () => false,
  });

  poller.watch(job.id, { immediate: true });
  scheduled.shift()();
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(requested, ['/api/admin/library/scans/scan_test']);
  poller.stopAll();
});

test('completed scan cards escape and retain every rejected path and reason', () => {
  const snapshot = succeeded();
  const output = renderJobCardBody({
    id: 'scan-app',
    type: 'library-scan',
    title: 'Managed library scan',
    detail: 'Completed',
    state: 'succeeded',
    createdAt: Date.now(),
    progress: { phase: 'complete', active: false },
    result: snapshot.result,
    error: '',
  });
  assert.match(output, /psx\/&lt;script&gt;\.cue/);
  assert.match(output, /&lt;b&gt;Disc 2 is missing&lt;\/b&gt;/);
  assert.match(output, /nes\/Existing\.nes/);
  assert.match(output, /&lt;img onerror=alert\(1\)&gt;/);
  assert.match(output, /&lt;script&gt;cover failed&lt;\/script&gt;/);
  assert.doesNotMatch(output, /<script>|<b>Disc 2|<img onerror/);
});
