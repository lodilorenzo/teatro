import test from 'node:test';
import assert from 'node:assert/strict';

import { activeJobCancellationUrl, JobsController } from './jobs.js';
import {
  addJob, findJob, initialGogImportProgress, state, updateJob,
} from '../state.js';

test('Jobs controller dismisses finished work but retains active work', () => {
  const previousJobs = state.jobs;
  state.jobs = [];
  const renders = [];
  const controller = new JobsController({
    app: { querySelector: () => null, querySelectorAll: () => [] },
    render: () => renders.push('render'),
    setError: () => {},
  });
  try {
    const active = addJob({ type: 'upload', title: 'Active', state: 'running' });
    const finished = addJob({ type: 'upload', title: 'Finished', state: 'succeeded' });
    controller.dismiss(active.id);
    assert.ok(findJob(active.id));
    controller.dismiss(finished.id);
    assert.equal(findJob(finished.id), null);
    assert.equal(renders.length, 1);
  } finally {
    state.jobs = previousJobs;
  }
});

test('Jobs controller confirms and dispatches actual cancellation', async () => {
  const previousJobs = state.jobs;
  const previousConfirm = globalThis.confirm;
  state.jobs = [];
  globalThis.confirm = () => true;
  const cancelled = [];
  const controller = new JobsController({
    app: { querySelector: () => null, querySelectorAll: () => [] },
    render: () => {},
    setError: () => {},
    cancelJob: async (jobId) => cancelled.push(jobId),
  });
  try {
    const active = addJob({ type: 'upload', title: 'Active', state: 'running' });
    controller.cancel(active.id);
    await new Promise((resolve) => setImmediate(resolve));
    assert.deepEqual(cancelled, [active.id]);
  } finally {
    state.jobs = previousJobs;
    globalThis.confirm = previousConfirm;
  }
});

test('active jobs map to same-origin cancellation endpoints', () => {
  assert.equal(activeJobCancellationUrl({
    id: 'upload_1', type: 'upload', state: 'running',
  }), '/api/admin/background-transfers/upload_1?operation=upload');
  assert.equal(activeJobCancellationUrl({
    id: 'gog_upload', type: 'gog-import', state: 'running', serverJobId: null,
  }), '/api/admin/background-transfers/gog_upload?operation=gog');
  assert.equal(activeJobCancellationUrl({
    id: 'local', type: 'gog-import', state: 'running', serverJobId: 'gog_server',
  }), '/api/admin/gog-imports/gog_server');
  assert.equal(activeJobCancellationUrl({
    id: 'local', type: 'library-scan', state: 'queued', serverJobId: 'scan_server',
  }), '/api/admin/library/scans/scan_server');
  assert.equal(activeJobCancellationUrl({
    id: 'upload_done', type: 'upload', state: 'succeeded',
  }), null);
});

test('Jobs controller cancels every active job before navigation', async () => {
  const previousJobs = state.jobs;
  state.jobs = [];
  const cancelled = [];
  const controller = new JobsController({
    app: { querySelector: () => null, querySelectorAll: () => [] },
    render: () => {},
    setError: () => {},
    cancelJob: async (jobId) => {
      cancelled.push(jobId);
      updateJob(jobId, { state: 'cancelled' });
    },
  });
  try {
    const first = addJob({ type: 'upload', title: 'First', state: 'running' });
    const second = addJob({ type: 'library-scan', title: 'Second', state: 'queued' });
    addJob({ type: 'gog-import', title: 'Done', state: 'succeeded' });
    await controller.cancelAll();
    assert.deepEqual(new Set(cancelled), new Set([first.id, second.id]));
  } finally {
    state.jobs = previousJobs;
  }
});

test('Jobs refresh rerenders when a queued job becomes active', () => {
  const previousJobs = state.jobs;
  const previousScreen = state.screen;
  state.jobs = [];
  state.screen = 'jobs';
  const job = addJob({ type: 'upload', title: 'Now active', state: 'running' });
  const shell = { dataset: { jobShell: job.id, jobPriority: '1' } };
  let renderCount = 0;
  const controller = new JobsController({
    app: {
      querySelectorAll: (selector) => (selector === '[data-job-shell]' ? [shell] : []),
    },
    render: () => { renderCount += 1; },
    setError: () => {},
  });

  try {
    controller.refreshJob(job.id);
    assert.equal(renderCount, 1);
  } finally {
    state.jobs = previousJobs;
    state.screen = previousScreen;
  }
});

test('Jobs refresh preserves transcript scroll and disclosure state', () => {
  const previousJobs = state.jobs;
  const previousScreen = state.screen;
  state.jobs = [];
  state.screen = 'jobs';
  const job = addJob({
    type: 'gog-import', title: 'Import', state: 'running', serverJobId: 'gog_test',
    progress: {
      ...initialGogImportProgress(), active: true, serverProcessing: true,
      jobId: 'gog_test', jobState: 'running', phase: 'extract_installer_payload',
      transcript: [{ seq: 1, stream: 'stdout', text: 'line' }],
    },
  });
  const oldDetails = { dataset: { jobDetails: 'transcript' }, open: false };
  const newDetails = { dataset: { jobDetails: 'transcript' }, open: true };
  const oldTranscript = { scrollTop: 120, scrollHeight: 1000, clientHeight: 100 };
  const newTranscript = { scrollTop: 0, scrollHeight: 1600, clientHeight: 100 };
  let replaced = false;
  const shell = {
    dataset: { jobShell: job.id, jobPriority: '0' },
    querySelector(selector) {
      if (selector === '[data-job-transcript]') return replaced ? newTranscript : oldTranscript;
      return null;
    },
    querySelectorAll(selector) {
      if (selector === 'details[data-job-details]') return [replaced ? newDetails : oldDetails];
      return [];
    },
    set innerHTML(_value) { replaced = true; },
  };
  const app = {
    querySelector: () => null,
    querySelectorAll: (selector) => (selector === '[data-job-shell]' ? [shell] : []),
  };
  const controller = new JobsController({ app, render: () => {}, setError: () => {} });

  try {
    controller.refreshJob(job.id);
    assert.equal(newDetails.open, false);
    assert.equal(newTranscript.scrollTop, 120);
  } finally {
    state.jobs = previousJobs;
    state.screen = previousScreen;
  }
});
