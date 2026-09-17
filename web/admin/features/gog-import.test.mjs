import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  GogImportController,
  validateGogImportCreateResponse,
  validateGogImportJobSnapshot,
  validateGogSetupSelection,
} from './gog-import.js';
import { SERVER_JOB_STORAGE_KEY } from './server-jobs.js';
import {
  addJob, findJob, resetGogImportWorkflow, state,
} from '../state.js';

const file = (name, size = 1) => ({ name, size });
const jobContract = JSON.parse(readFileSync(
  new URL('../../../tests/fixtures/gog-import-job-contract.json', import.meta.url),
  'utf8',
));
const sortedKeys = (value) => Object.keys(value).sort();

class FakeScheduler {
  constructor() {
    this.nextId = 1;
    this.tasks = new Map();
    this.delays = [];
  }

  setTimeout = (callback, delay) => {
    const id = this.nextId;
    this.nextId += 1;
    this.tasks.set(id, callback);
    this.delays.push(delay);
    return id;
  };

  clearTimeout = (id) => this.tasks.delete(id);

  runNext() {
    const next = this.tasks.entries().next().value;
    assert.ok(next, 'expected a scheduled poll');
    const [id, callback] = next;
    this.tasks.delete(id);
    callback();
  }
}

const app = {
  querySelector: () => null,
  querySelectorAll: () => [],
};

function fakeStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    values,
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };
}

async function settle() {
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
}

function createdJob(id = 'gog_test') {
  const statusUrl = `/api/admin/gog-imports/${id}`;
  return {
    status: 202,
    location: statusUrl,
    body: {
      job: {
        id, state: 'queued', phase: 'queued', progress: null,
        output_truncated: false, last_event_seq: 0,
      },
      status_url: statusUrl,
    },
  };
}

function runningJob(id = 'gog_test', cursor = 2) {
  return {
    id,
    state: 'running',
    phase: 'extract_installer_payload',
    progress: { kind: 'percent', current: null, total: null, percent: 42.5 },
    events: [
      { seq: cursor - 1, kind: 'phase', phase: 'extract_installer_payload', stream: null, text: 'Extracting installer payload' },
      { seq: cursor, kind: 'output', phase: 'extract_installer_payload', stream: 'stdout', text: '42.5%' },
    ],
    next_event_seq: cursor,
    output_truncated: false,
    result: null,
    error: null,
  };
}

function setupBlob(name = 'setup.exe', contents = 'setup') {
  const blob = new Blob([contents]);
  Object.defineProperty(blob, 'name', { value: name });
  return blob;
}

function controllerContext(overrides = {}) {
  const scheduler = overrides.scheduler || new FakeScheduler();
  const feedback = { notices: [], errors: [] };
  const context = {
    app,
    render: () => {},
    jobChanged: () => {},
    setNotice: (message) => feedback.notices.push(message),
    setError: (error) => feedback.errors.push(error?.message || String(error)),
    loadPlatforms: async () => {},
    loadStats: async () => {},
    loadRoms: async () => {},
    loadIgdbStatus: async () => {},
    request: async () => runningJob(),
    uploadTransfer: {
      uploadWithProgress: async () => createdJob(),
      autoApplyIgdbMetadata: async () => ({ applied: false }),
    },
    setTimeout: scheduler.setTimeout,
    clearTimeout: scheduler.clearTimeout,
    ...overrides,
  };
  delete context.scheduler;
  return { context, scheduler, feedback };
}

function preserveState() {
  return {
    auth: state.auth,
    status: state.gogImportStatus,
    files: state.gogImportSelectedFiles,
    title: state.gogImportTitle,
    origin: state.gogImportTitleOrigin,
    confidence: state.gogImportTitleConfidence,
    jobs: state.jobs,
    screen: state.screen,
  };
}

function restoreState(previous) {
  state.auth = previous.auth;
  state.gogImportStatus = previous.status;
  state.gogImportSelectedFiles = previous.files;
  state.gogImportTitle = previous.title;
  state.gogImportTitleOrigin = previous.origin;
  state.gogImportTitleConfidence = previous.confidence;
  state.jobs = previous.jobs;
  state.screen = previous.screen;
}

test('GOG setup selection requires one exe and only bin sidecars', () => {
  assert.equal(validateGogSetupSelection([
    file('setup_game.exe'), file('setup_game-1.bin'), file('setup_game-2.bin'),
  ]), true);
  assert.throws(() => validateGogSetupSelection([]), /Select one setup/);
  assert.throws(() => validateGogSetupSelection([file('setup.exe'), file('patch.exe')]), /exactly one setup/);
  assert.throws(() => validateGogSetupSelection([file('setup.exe'), file('readme.txt')]), /Only setup/);
  assert.throws(() => validateGogSetupSelection([file('Setup.exe'), file('setup.EXE')]), /unique case-insensitively/);
  assert.throws(() => validateGogSetupSelection([file('../setup.exe')]), /flat names/);
  assert.throws(() => validateGogSetupSelection([file(' setup.exe')]), /255 UTF-8 bytes/);
  assert.throws(() => validateGogSetupSelection([file(`setup_${'a'.repeat(246)}.exe`)]), /255 UTF-8 bytes/);
});

test('GOG file changes synchronize only empty or inferred titles', () => {
  const previous = preserveState();
  const titleInput = {
    value: '', listeners: {},
    addEventListener(name, listener) { this.listeners[name] = listener; },
  };
  const hint = { textContent: '', dataset: {} };
  const titleApp = {
    querySelector(selector) {
      if (selector === '#gog-import-form input[name="title"]') return titleInput;
      if (selector === '#gog-import-title-hint') return hint;
      return null;
    },
    querySelectorAll: () => [],
  };

  resetGogImportWorkflow();
  state.jobs = [];
  try {
    const { context } = controllerContext({ app: titleApp });
    const controller = new GogImportController(context);
    controller.bind();
    controller.appendSelectedFiles([file('setup_quake_2.exe')]);
    assert.equal(state.gogImportTitle, 'Quake 2');
    assert.equal(state.gogImportTitleOrigin, 'inferred');
    assert.equal(state.gogImportTitleConfidence, 'medium');
    assert.match(hint.textContent, /filename is ambiguous/);

    titleInput.value = 'Operator title';
    titleInput.listeners.input({ currentTarget: titleInput });
    controller.updateSelectedFiles([file('setup_terraria.exe')]);
    assert.equal(state.gogImportTitle, 'Operator title');
    titleInput.value = '';
    titleInput.listeners.input({ currentTarget: titleInput });
    controller.updateSelectedFiles([file('setup_the_witcher_3.exe')]);
    assert.equal(state.gogImportTitle, '', 'manual blanking is retained');
  } finally {
    restoreState(previous);
  }
});

test('GOG submission rejects a manually blank title before creating a job', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = fakeStorage();
  resetGogImportWorkflow();
  state.jobs = [];
  state.auth = { header: 'Basic test' };
  state.gogImportStatus = { enabled: true, configured: true };
  state.gogImportSelectedFiles = [file('setup_quake_2.exe')];
  state.gogImportTitleOrigin = 'manual';
  let uploadCalls = 0;
  try {
    const { context, feedback } = controllerContext({
      uploadTransfer: {
        uploadWithProgress: async () => { uploadCalls += 1; },
        autoApplyIgdbMetadata: async () => ({ applied: false }),
      },
    });
    const controller = new GogImportController(context);
    await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { title: { value: '' } } } });
    assert.equal(uploadCalls, 0);
    assert.equal(state.jobs.length, 0);
    assert.equal(feedback.errors.at(-1), 'Enter the game name.');
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('GOG asynchronous job JSON contract remains frozen for polling', () => {
  const { contract } = jobContract;
  assert.equal(contract.create_http_status, 202);
  assert.deepEqual(contract.job_states, ['queued', 'running', 'succeeded', 'failed', 'cancelled']);
  assert.deepEqual(contract.progress_kinds, ['percent', 'bytes']);
  assert.equal(contract.limits.browser_poll_interval_ms, 750);
  assert.equal(contract.limits.browser_transcript_entries, 512);
  assert.deepEqual(sortedKeys(jobContract.create_response), ['job', 'status_url']);
  const statusKeys = [
    'created_at', 'error', 'events', 'id', 'next_event_seq', 'output_truncated',
    'phase', 'progress', 'result', 'state', 'updated_at',
  ];
  for (const name of ['running_percent_response', 'running_bytes_response', 'succeeded_response', 'failed_response']) {
    assert.deepEqual(sortedKeys(jobContract[name]), statusKeys);
  }
  assert.deepEqual(jobContract.running_bytes_response.progress, {
    kind: 'bytes', current: 2147483648, total: 4294967296, percent: 50,
  });
});

test('GOG create and polling resources are validated and bounded before use', () => {
  assert.equal(validateGogImportCreateResponse(createdJob()).job.id, 'gog_test');
  assert.throws(
    () => validateGogImportCreateResponse({ ...createdJob(), location: '/api/admin/gog-imports/gog_other' }),
    /invalid GOG import status URL/,
  );
  assert.throws(
    () => validateGogImportCreateResponse(createdJob(`gog_${'x'.repeat(129)}`)),
    /invalid GOG import job summary/,
  );
  assert.equal(validateGogImportJobSnapshot(runningJob(), 'gog_test').next_event_seq, 2);
  assert.throws(
    () => validateGogImportJobSnapshot({
      ...runningJob(), progress: { kind: 'bytes', current: 1, total: 4, percent: 30 },
    }, 'gog_test'),
    /invalid byte progress/,
  );
  assert.throws(
    () => validateGogImportJobSnapshot({
      ...runningJob(),
      events: [{ seq: 1, kind: 'output', phase: 'extract_installer_payload', stream: 'stdout', text: 'x'.repeat(4097) }],
    }, 'gog_test'),
    /invalid GOG import event/,
  );
});

test('launching immediately clears preparation, notifies, and retains the running job', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  const storage = fakeStorage();
  let releaseUpload;
  const uploadGate = new Promise((resolve) => { releaseUpload = resolve; });
  globalThis.sessionStorage = storage;
  state.auth = { header: 'Basic test' };
  state.gogImportStatus = { enabled: true, configured: true };
  state.jobs = [];
  resetGogImportWorkflow();
  state.gogImportSelectedFiles = [setupBlob()];
  state.gogImportTitle = 'Private Game';
  state.gogImportTitleOrigin = 'manual';

  try {
    const { context, scheduler, feedback } = controllerContext({
      uploadTransfer: {
        uploadWithProgress: async () => uploadGate,
        autoApplyIgdbMetadata: async () => ({ applied: false }),
      },
    });
    const controller = new GogImportController(context);
    await controller.onSubmit({
      preventDefault() {}, currentTarget: { elements: { title: { value: 'Private Game' } } },
    });

    assert.equal(state.gogImportSelectedFiles.length, 0);
    assert.equal(state.gogImportTitle, '');
    assert.equal(state.jobs.length, 1);
    assert.equal(state.jobs[0].state, 'running');
    assert.equal(state.jobs[0].progress.serverProcessing, false);
    assert.match(feedback.notices.at(-1), /launched\. Track it in Jobs/);

    releaseUpload(createdJob('gog_private'));
    await settle();
    assert.equal(state.jobs[0].serverJobId, 'gog_private');
    assert.equal(state.jobs[0].progress.serverProcessing, true);
    const stored = JSON.parse(storage.values.get(SERVER_JOB_STORAGE_KEY));
    assert.deepEqual(stored, [{
      type: 'gog-import', id: 'gog_private', statusUrl: '/api/admin/gog-imports/gog_private',
    }]);
    assert.equal(JSON.stringify(stored).includes('Private Game'), false);
    assert.equal(scheduler.tasks.size, 1);
    controller.reset({ render: false });
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('multiple GOG imports retain separate jobs, cursors, and pollers', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  const storage = fakeStorage();
  let createCount = 0;
  const requests = [];
  globalThis.sessionStorage = storage;
  state.auth = { header: 'Basic test' };
  state.gogImportStatus = { enabled: true, configured: true };
  state.jobs = [];
  resetGogImportWorkflow();

  try {
    const { context, scheduler } = controllerContext({
      uploadTransfer: {
        uploadWithProgress: async () => {
          createCount += 1;
          return createdJob(`gog_job_${createCount}`);
        },
        autoApplyIgdbMetadata: async () => ({ applied: false }),
      },
      request: async (path) => {
        requests.push(path);
        const id = path.includes('gog_job_1') ? 'gog_job_1' : 'gog_job_2';
        return runningJob(id);
      },
    });
    const controller = new GogImportController(context);
    for (const title of ['First Game', 'Second Game']) {
      state.gogImportSelectedFiles = [setupBlob(`setup_${title.replace(' ', '_')}.exe`)];
      state.gogImportTitle = title;
      state.gogImportTitleOrigin = 'manual';
      await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { title: { value: title } } } });
      await settle();
    }
    assert.equal(state.jobs.length, 2);
    assert.equal(scheduler.tasks.size, 2);
    scheduler.runNext();
    scheduler.runNext();
    await settle();
    assert.deepEqual(requests.sort(), [
      '/api/admin/gog-imports/gog_job_1?after=0',
      '/api/admin/gog-imports/gog_job_2?after=0',
    ]);
    assert.ok(state.jobs.every((job) => job.progress.eventCursor === 2));
    assert.ok(state.jobs.every((job) => job.progress.transcript.length === 1));
    assert.equal(scheduler.tasks.size, 2);

    controller.stopWatching(state.jobs.find(({ serverJobId }) => serverJobId === 'gog_job_1').id);
    assert.deepEqual(state.jobs.map(({ serverJobId }) => serverJobId), ['gog_job_2']);
    assert.equal(scheduler.tasks.size, 1);
    controller.reset({ render: false });
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('stored server jobs reattach as a list and reset clears all pollers', () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  const storage = fakeStorage({
    [SERVER_JOB_STORAGE_KEY]: JSON.stringify([
      {
        type: 'gog-import', id: 'gog_saved_one',
        statusUrl: '/api/admin/gog-imports/gog_saved_one',
      },
      {
        type: 'gog-import', id: 'gog_saved_two',
        statusUrl: '/api/admin/gog-imports/gog_saved_two',
      },
    ]),
  });
  globalThis.sessionStorage = storage;
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  try {
    const { context, scheduler } = controllerContext();
    const controller = new GogImportController(context);
    assert.equal(state.jobs.length, 2);
    assert.deepEqual(state.jobs.map(({ serverJobId }) => serverJobId).sort(), ['gog_saved_one', 'gog_saved_two']);
    assert.ok(state.jobs.every((job) => job.progress.reattached));
    controller.bind();
    controller.bind();
    assert.equal(scheduler.tasks.size, 2, 'rebinding must not duplicate pollers');
    controller.reset({ render: false });
    assert.equal(scheduler.tasks.size, 0);
    assert.equal(storage.values.has(SERVER_JOB_STORAGE_KEY), false);
    assert.equal(state.jobs.length, 0);
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('polling advances only its job cursor and backs off transient failures', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = fakeStorage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  let requestCount = 0;
  const requests = [];
  try {
    const { context, scheduler } = controllerContext({
      request: async (path) => {
        requests.push(path);
        requestCount += 1;
        if (requestCount === 1) return runningJob('gog_test');
        throw new Error('temporary network failure');
      },
    });
    const controller = new GogImportController(context);
    const appJob = addJob({ type: 'gog-import', title: 'Test', state: 'running' });
    controller.acceptCreatedJob(appJob.id, createdJob());
    scheduler.runNext();
    await settle();
    assert.equal(findJob(appJob.id).progress.eventCursor, 2);
    assert.equal(requests.at(-1), '/api/admin/gog-imports/gog_test?after=0');
    assert.equal(scheduler.delays.at(-1), 750);
    scheduler.runNext();
    await settle();
    assert.equal(requests.at(-1), '/api/admin/gog-imports/gog_test?after=2');
    assert.match(findJob(appJob.id).progress.pollingError, /Retrying this existing job/);
    assert.equal(scheduler.delays.at(-1), 1500);
    controller.reset({ render: false });
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('terminal success updates its job without navigating away from Jobs', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  const storage = fakeStorage();
  const succeeded = structuredClone(jobContract.succeeded_response);
  succeeded.id = 'gog_test';
  let metadataCalls = 0;
  let refreshCalls = 0;
  globalThis.sessionStorage = storage;
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  state.screen = 'jobs';
  try {
    const { context, scheduler, feedback } = controllerContext({
      request: async () => succeeded,
      uploadTransfer: {
        uploadWithProgress: async () => createdJob(),
        autoApplyIgdbMetadata: async () => {
          metadataCalls += 1;
          return { applied: false, reason: 'no IGDB candidate was found on the selected or any platform' };
        },
      },
      loadPlatforms: async () => { refreshCalls += 1; },
      loadStats: async () => { refreshCalls += 1; },
      loadRoms: async () => { refreshCalls += 1; },
    });
    const controller = new GogImportController(context);
    const appJob = addJob({ type: 'gog-import', title: 'Example Game', state: 'running' });
    controller.acceptCreatedJob(appJob.id, createdJob());
    scheduler.runNext();
    await settle();

    assert.equal(metadataCalls, 1);
    assert.equal(refreshCalls, 3);
    assert.equal(findJob(appJob.id).state, 'succeeded');
    assert.equal(findJob(appJob.id).result.rom.id, 42);
    assert.equal(state.screen, 'jobs');
    assert.match(feedback.notices.at(-1), /Imported Example Game/);
    assert.match(feedback.notices.at(-1), /IGDB metadata skipped: no IGDB candidate was found/);
    assert.equal(storage.values.has(SERVER_JOB_STORAGE_KEY), false);
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});

test('terminal failures and missing resources remain visible as finished jobs', async () => {
  const previous = preserveState();
  const previousStorage = globalThis.sessionStorage;
  globalThis.sessionStorage = fakeStorage();
  state.auth = { header: 'Basic test' };
  state.jobs = [];
  const failed = structuredClone(jobContract.failed_response);
  failed.id = 'gog_failed';
  try {
    const { context, scheduler, feedback } = controllerContext({ request: async () => failed });
    const controller = new GogImportController(context);
    const appJob = addJob({ type: 'gog-import', title: 'Failed Game', state: 'running' });
    controller.acceptCreatedJob(appJob.id, createdJob('gog_failed'));
    scheduler.runNext();
    await settle();
    assert.equal(findJob(appJob.id).state, 'failed');
    assert.equal(findJob(appJob.id).error, failed.error.message);
    assert.equal(feedback.errors.at(-1), failed.error.message);

    const missing = new Error('not found');
    missing.status = 404;
    const missingController = new GogImportController({
      ...context,
      request: async () => { throw missing; },
      setTimeout: scheduler.setTimeout,
      clearTimeout: scheduler.clearTimeout,
    });
    const missingJob = addJob({ type: 'gog-import', title: 'Missing Game', state: 'running' });
    missingController.acceptCreatedJob(missingJob.id, createdJob('gog_missing'));
    scheduler.runNext();
    await settle();
    assert.equal(findJob(missingJob.id).state, 'unavailable');
    assert.match(findJob(missingJob.id).error, /expired or Teatro restarted/);
  } finally {
    globalThis.sessionStorage = previousStorage;
    restoreState(previous);
  }
});
