import test from 'node:test';
import assert from 'node:assert/strict';

import {
  GOG_IMPORT_TRANSCRIPT_LIMIT,
  JOB_LIST_LIMIT,
  JOB_STORAGE_KEY,
  LIBRARY_VIEW_STORAGE_KEY,
  NOTIFICATION_DURATION_MS,
  ROM_PAGE_SIZE_STORAGE_KEY,
  UPLOAD_PLATFORM_STORAGE_KEY,
  activeJobCount,
  addJob,
  applyGogImportJobSnapshot,
  beginGogImportJob,
  beginRomSelection,
  clearFeedback,
  clearFinishedJobs,
  commitGogImportTitle,
  dismissNotification,
  findJob,
  invalidateRomSelection,
  isCurrentRomSelection,
  removeJob,
  resetGogImportWorkflow,
  setGogImportFiles,
  setGogImportTitleManual,
  setUploadFiles,
  setUploadPreview,
  showError,
  showNotice,
  state,
  syncGogImportTitleSuggestion,
  updateJob,
  updateJobProgress,
} from './state.js';

test('named actions stack timed feedback and keep preparation transitions consistent', () => {
  clearFeedback();
  const previousJobs = state.jobs;
  const errorStartedAt = Date.now();
  const errorNotification = showError(new Error('failed'));
  assert.equal(state.error, 'failed');
  assert.equal(state.notice, '');
  assert.equal(errorNotification.kind, 'error');
  assert.ok(errorNotification.expiresAt >= errorStartedAt + NOTIFICATION_DURATION_MS.error);
  assert.ok(errorNotification.expiresAt <= Date.now() + NOTIFICATION_DURATION_MS.error);

  const noticeNotification = showNotice('ready');
  assert.equal(state.notice, 'ready');
  assert.equal(state.error, '');
  assert.equal(state.notifications.length, 2);
  dismissNotification(noticeNotification.id);
  assert.equal(state.error, 'failed');
  dismissNotification(errorNotification.id);
  assert.equal(state.notifications.length, 0);

  setUploadPreview(false, { roms: [{ plan_id: 'rom-1' }] });
  setUploadFiles([{ name: 'game.rom', size: 4 }]);
  assert.equal(state.uploadSelectedFiles.length, 1);
  assert.equal(state.uploadPlan, null);
  assert.equal(state.uploadPreviewLoading, false);

  setGogImportTitleManual('Private Game');
  setGogImportFiles([{ name: 'setup.exe', size: 5 }]);
  resetGogImportWorkflow();
  assert.equal(state.gogImportTitle, '');
  assert.equal(state.gogImportTitleOrigin, 'empty');
  assert.equal(state.gogImportTitleConfidence, null);
  assert.equal(state.gogImportSelectedFiles.length, 0);
  assert.equal(state.jobs, previousJobs, 'clearing preparation does not clear jobs');
});

test('application jobs retain concurrent work and bounded recent results', () => {
  const previousJobs = state.jobs;
  state.jobs = [];
  try {
    const upload = addJob({ type: 'upload', title: 'Upload One', state: 'running' });
    const gog = addJob({ type: 'gog-import', title: 'Import One', state: 'running' });
    assert.equal(activeJobCount(), 2);
    assert.equal(state.jobs.length, 2);
    assert.equal(findJob(upload.id).title, 'Upload One');

    updateJobProgress(upload.id, { percent: 42, loaded: 42, total: 100 });
    assert.equal(findJob(upload.id).progress.percent, 42);
    updateJob(upload.id, { state: 'succeeded', result: { roms: [{ id: 7 }] } });
    const completedAt = findJob(upload.id).completedAt;
    assert.ok(completedAt >= findJob(upload.id).createdAt);
    updateJob(upload.id, { statusText: 'Still complete' });
    assert.equal(findJob(upload.id).completedAt, completedAt);
    assert.equal(activeJobCount(), 1);

    clearFinishedJobs();
    assert.deepEqual(state.jobs.map(({ id }) => id), [gog.id]);
    assert.equal(removeJob(gog.id), true);
    assert.equal(state.jobs.length, 0);

    for (let index = 0; index < JOB_LIST_LIMIT + 5; index += 1) {
      addJob({ type: 'upload', title: `Job ${index}`, state: 'succeeded' });
    }
    assert.equal(state.jobs.length, JOB_LIST_LIMIT);

    state.jobs = [];
    for (let index = 0; index < JOB_LIST_LIMIT + 5; index += 1) {
      addJob({ type: 'upload', title: `Active ${index}`, state: 'running' });
    }
    assert.equal(state.jobs.length, JOB_LIST_LIMIT + 5, 'active jobs are never evicted');
  } finally {
    state.jobs = previousJobs;
  }
});

test('job mutations persist navigation-safe history in session storage', async () => {
  const previousStorage = globalThis.sessionStorage;
  const previousJobs = state.jobs;
  const values = new Map();
  globalThis.sessionStorage = {
    getItem: (key) => values.get(key) || null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };
  state.jobs = [];
  try {
    const job = addJob({ type: 'upload', title: 'Persistent upload', state: 'running' });
    assert.equal(JSON.parse(values.get(JOB_STORAGE_KEY))[0].id, job.id);
    updateJob(job.id, { state: 'succeeded' });
    assert.equal(JSON.parse(values.get(JOB_STORAGE_KEY))[0].state, 'succeeded');
    const reloaded = await import(`./state.js?jobs=${Date.now()}`);
    assert.equal(reloaded.state.jobs[0].title, 'Persistent upload');
    assert.equal(reloaded.state.jobs[0].state, 'succeeded');
    removeJob(job.id);
    assert.equal(values.has(JOB_STORAGE_KEY), false);
  } finally {
    state.jobs = previousJobs;
    globalThis.sessionStorage = previousStorage;
  }
});

test('GOG title ownership makes manual edits terminal until preparation reset', () => {
  resetGogImportWorkflow();
  assert.equal(syncGogImportTitleSuggestion({
    title: 'Inferred Game', confidence: 'high', source: 'gog_setup_filename',
  }), true);
  assert.equal(state.gogImportTitle, 'Inferred Game');
  assert.equal(state.gogImportTitleOrigin, 'inferred');
  assert.equal(state.gogImportTitleConfidence, 'high');
  commitGogImportTitle('Inferred Game');
  assert.equal(state.gogImportTitleOrigin, 'inferred');

  setGogImportTitleManual('Operator Game');
  assert.equal(syncGogImportTitleSuggestion({
    title: 'Ignored Game', confidence: 'high', source: 'gog_setup_filename',
  }), false);
  assert.equal(state.gogImportTitle, 'Operator Game');
  setGogImportTitleManual('');
  assert.equal(syncGogImportTitleSuggestion(null), false);
  assert.equal(state.gogImportTitleOrigin, 'manual');

  resetGogImportWorkflow();
  assert.equal(state.gogImportTitleOrigin, 'empty');
  assert.equal(syncGogImportTitleSuggestion(null), true);
});

test('upload platform preference survives workflow resets and module reloads', async () => {
  const previousStorage = globalThis.localStorage;
  const values = new Map([[UPLOAD_PLATFORM_STORAGE_KEY, '42']]);
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };

  try {
    const freshState = await import(`./state.js?upload-platform=${Date.now()}`);
    assert.equal(freshState.state.uploadPlatformId, '42');
    freshState.resetUploadWorkflow();
    assert.equal(freshState.state.uploadPlatformId, '42');
    freshState.setUploadPlatformId('77');
    assert.equal(values.get(UPLOAD_PLATFORM_STORAGE_KEY), '77');
    freshState.setUploadPlatformId('');
    assert.equal(values.has(UPLOAD_PLATFORM_STORAGE_KEY), false);
  } finally {
    globalThis.localStorage = previousStorage;
  }
});

test('admin library view defaults to grid and survives reloads', async () => {
  const previousStorage = globalThis.localStorage;
  const previousView = state.libraryView;
  const values = new Map();
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
  };

  try {
    const freshState = await import(`./state.js?library-view=${Date.now()}`);
    assert.equal(freshState.state.libraryView, 'grid');
    assert.equal(freshState.setLibraryView('list'), 'list');
    assert.equal(values.get(LIBRARY_VIEW_STORAGE_KEY), 'list');

    const reloadedState = await import(`./state.js?library-view-reload=${Date.now()}`);
    assert.equal(reloadedState.state.libraryView, 'list');
  } finally {
    state.libraryView = previousView;
    globalThis.localStorage = previousStorage;
  }
});

test('admin library page size survives reloads and stays within the browser limit', async () => {
  const previousStorage = globalThis.localStorage;
  const values = new Map([[ROM_PAGE_SIZE_STORAGE_KEY, '50']]);
  globalThis.localStorage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
  };

  try {
    const freshState = await import(`./state.js?rom-page-size=${Date.now()}`);
    assert.equal(freshState.state.romLimit, 50);
    freshState.state.romOffset = 50;
    assert.equal(freshState.setRomPageSize(100), 100);
    assert.equal(freshState.state.romOffset, 0);
    assert.equal(values.get(ROM_PAGE_SIZE_STORAGE_KEY), '100');
    assert.equal(freshState.setRomPageSize(101), 75);
  } finally {
    globalThis.localStorage = previousStorage;
  }
});

test('GOG snapshots merge into their own jobs and bound each transcript tail', () => {
  const previousJobs = state.jobs;
  state.jobs = [];
  try {
    const firstJob = addJob({ type: 'gog-import', title: 'First', state: 'running' });
    const secondJob = addJob({ type: 'gog-import', title: 'Second', state: 'running' });
    beginGogImportJob(firstJob.id, {
      id: 'gog_first', state: 'queued', phase: 'queued', progress: null,
      last_event_seq: 0, output_truncated: false,
    }, '/api/admin/gog-imports/gog_first');
    beginGogImportJob(secondJob.id, {
      id: 'gog_second', state: 'queued', phase: 'queued', progress: null,
      last_event_seq: 0, output_truncated: false,
    }, '/api/admin/gog-imports/gog_second');

    const first = {
      id: 'gog_first', state: 'running', phase: 'extract_installer_payload',
      progress: { kind: 'percent', current: null, total: null, percent: 42.5 },
      events: [
        { seq: 1, kind: 'phase', phase: 'extract_installer_payload', stream: null, text: 'Extracting installer payload' },
        { seq: 2, kind: 'output', phase: 'extract_installer_payload', stream: 'stdout', text: '42.5%' },
      ],
      next_event_seq: 2, output_truncated: false, result: null, error: null,
    };
    applyGogImportJobSnapshot(firstJob.id, first);
    applyGogImportJobSnapshot(firstJob.id, first);
    assert.equal(findJob(firstJob.id).progress.eventCursor, 2);
    assert.equal(findJob(firstJob.id).progress.transcript.length, 1);
    assert.equal(findJob(secondJob.id).progress.eventCursor, 0);

    const output = Array.from({ length: 600 }, (_, index) => ({
      seq: index + 3,
      kind: 'output',
      phase: 'extract_installer_payload',
      stream: index % 2 ? 'stderr' : 'stdout',
      text: `line ${index}`,
    }));
    applyGogImportJobSnapshot(firstJob.id, {
      ...first, events: output, next_event_seq: 602, output_truncated: false,
    });
    assert.equal(findJob(firstJob.id).progress.transcript.length, GOG_IMPORT_TRANSCRIPT_LIMIT);
    assert.equal(findJob(firstJob.id).progress.transcript.at(-1).seq, 602);
    assert.equal(findJob(firstJob.id).progress.outputTruncated, true);
  } finally {
    state.jobs = previousJobs;
  }
});

test('new selections and invalidation make older responses stale', () => {
  const first = beginRomSelection();
  assert.equal(isCurrentRomSelection(first), true);
  const second = beginRomSelection();
  assert.equal(isCurrentRomSelection(first), false);
  assert.equal(isCurrentRomSelection(second), true);
  invalidateRomSelection();
  assert.equal(isCurrentRomSelection(second), false);
});
