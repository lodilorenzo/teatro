import test from 'node:test';
import assert from 'node:assert/strict';

import { UploadController, applyPlannedTitle, plannedTitlesAreValid } from './upload.js';
import { state } from '../state.js';

test('planned title edits update title, slug, folder keys, and matching group labels', () => {
  const plan = {
    roms: [{
      plan_id: 'rom-1',
      title: 'Final Fantasy VII',
      slug: 'final-fantasy-vii',
      groups: [
        {
          display_name: 'Final Fantasy VII',
          group_key: 'final-fantasy-vii:playlist',
        },
        {
          display_name: 'Disc 1',
          group_key: 'final-fantasy-vii:disc-1',
        },
      ],
      files: [{ original_file_name: 'Final Fantasy VII (USA).m3u' }],
    }],
  };

  const rom = applyPlannedTitle(plan, 'rom-1', 'Final Fantasy VII Remastered');

  assert.equal(rom.title, 'Final Fantasy VII Remastered');
  assert.equal(rom.slug, 'final-fantasy-vii-remastered');
  assert.equal(rom.groups[0].display_name, 'Final Fantasy VII Remastered');
  assert.equal(rom.groups[0].group_key, 'final-fantasy-vii-remastered:playlist');
  assert.equal(rom.groups[1].display_name, 'Disc 1');
  assert.equal(rom.groups[1].group_key, 'final-fantasy-vii-remastered:disc-1');
  assert.equal(rom.files[0].original_file_name, 'Final Fantasy VII (USA).m3u');
  assert.equal(plannedTitlesAreValid(plan), true);

  applyPlannedTitle(plan, 'rom-1', '   ');
  assert.equal(plannedTitlesAreValid(plan), false);
});

test('upload preview removes files whose exact names already exist for the selected platform', async () => {
  const previousFetch = globalThis.fetch;
  const previousAuth = state.auth;
  const previousPlatforms = state.platforms;
  const previousFiles = state.uploadSelectedFiles;
  const previousPlan = state.uploadPlan;
  const previousPreviewLoading = state.uploadPreviewLoading;
  const previousPlatformId = state.uploadPlatformId;
  const form = { elements: { platform_id: { value: '7' } } };
  const requests = [];
  const notices = [];
  const errors = [];
  const plan = {
    platform_id: 7,
    platform_slug: 'ps2',
    roms: [{ plan_id: 'rom-1', title: 'Fresh Game', slug: 'fresh-game', groups: [], files: [] }],
    warnings: [{
      code: 'file_already_exists',
      message: 'file already exists for this platform and was skipped',
      file_name: 'Already There.bin',
    }],
    errors: [],
  };
  globalThis.fetch = async (path, options) => {
    requests.push([path, JSON.parse(options.body)]);
    return {
      status: 200,
      ok: true,
      headers: { get: () => 'application/json' },
      json: async () => plan,
    };
  };
  state.auth = { header: 'Basic test' };
  state.platforms = [{ id: 7, slug: 'ps2', display_name: 'PlayStation 2' }];
  state.uploadPlatformId = '7';
  state.uploadSelectedFiles = [
    { name: 'Already There.bin', size: 8 },
    { name: 'Fresh Game.bin', size: 5 },
  ];
  state.uploadPlan = null;
  state.uploadPreviewLoading = false;
  const controller = new UploadController({
    app: {
      querySelector: (selector) => (selector === '#upload-form' ? form : null),
      querySelectorAll: () => [],
    },
    render: () => {},
    setNotice: (message) => notices.push(message),
    setError: (error) => errors.push(error?.message || String(error)),
  });

  try {
    await controller.onPreviewPlan({ preventDefault() {} });

    assert.deepEqual(state.uploadSelectedFiles.map((file) => file.name), ['Fresh Game.bin']);
    assert.equal(state.uploadPlan, plan);
    assert.match(notices.at(-1), /“Already There\.bin” already exists for this platform.*removed from the upload list/);
    assert.equal(errors.length, 0);
    assert.deepEqual(requests[0][1].files.map((file) => file.file_name), [
      'Already There.bin', 'Fresh Game.bin',
    ]);
  } finally {
    globalThis.fetch = previousFetch;
    state.auth = previousAuth;
    state.platforms = previousPlatforms;
    state.uploadSelectedFiles = previousFiles;
    state.uploadPlan = previousPlan;
    state.uploadPreviewLoading = previousPreviewLoading;
    state.uploadPlatformId = previousPlatformId;
  }
});

test('launching an upload snapshots the plan, clears preparation, and retains the job', async () => {
  const previousFormData = globalThis.FormData;
  const previousPlatforms = state.platforms;
  const previousFiles = state.uploadSelectedFiles;
  const previousPlan = state.uploadPlan;
  const previousJobs = state.jobs;
  const previousPlatformId = state.uploadPlatformId;
  const previousScreen = state.screen;
  const fields = [];
  class FakeFormData {
    delete(name) { fields.push(['delete', name]); }
    set(name, value) { fields.push(['set', name, value]); }
    append(name, value, fileName) { fields.push(['append', name, value, fileName]); }
  }
  globalThis.FormData = FakeFormData;
  state.platforms = [{ id: 7, slug: 'nes', display_name: 'Nintendo Entertainment System' }];
  state.uploadPlatformId = '7';
  state.uploadSelectedFiles = [{ name: 'Planned Game.nes', size: 4 }];
  state.uploadPlan = {
    errors: [], warnings: [],
    roms: [{ plan_id: 'rom-1', title: 'Planned Game', slug: 'planned-game', groups: [] }],
  };
  state.jobs = [];
  state.screen = 'upload';
  let releaseUpload;
  const uploadGate = new Promise((resolve) => { releaseUpload = resolve; });
  const notices = [];
  const errors = [];
  const app = { querySelector: () => null, querySelectorAll: () => [] };
  const controller = new UploadController({
    app,
    render: () => {},
    jobChanged: () => {},
    setNotice: (message) => notices.push(message),
    setError: (error) => errors.push(error?.message || String(error)),
    loadPlatforms: async () => {},
    loadStats: async () => {},
    loadRoms: async () => {},
    loadIgdbStatus: async () => {},
    uploadTransfer: {
      uploadWithProgress: async () => uploadGate,
      autoApplyIgdbMetadata: async () => ({ applied: false }),
    },
  });

  try {
    await controller.onSubmit({
      preventDefault() {},
      currentTarget: { elements: { platform_id: { value: '7' } } },
    });
    assert.equal(state.uploadSelectedFiles.length, 0);
    assert.equal(state.uploadPlan, null);
    assert.equal(state.jobs.length, 1);
    assert.equal(state.jobs[0].title, 'Planned Game');
    assert.equal(state.jobs[0].state, 'running');
    assert.match(notices.at(-1), /launched\. Track it in Jobs/);
    assert.ok(fields.some(([operation, name, value]) => operation === 'set' && name === 'platform_slug' && value === 'nes'));

    releaseUpload({
      roms: [{ id: 9, name: 'Planned Game' }],
      warnings: [],
    });
    await new Promise((resolve) => setImmediate(resolve));
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(state.jobs[0].state, 'succeeded');
    assert.equal(state.jobs[0].result.roms[0].id, 9);
    assert.equal(state.screen, 'upload', 'background completion does not interrupt preparation');
    assert.equal(errors.length, 0);
  } finally {
    globalThis.FormData = previousFormData;
    state.platforms = previousPlatforms;
    state.uploadSelectedFiles = previousFiles;
    state.uploadPlan = previousPlan;
    state.jobs = previousJobs;
    state.uploadPlatformId = previousPlatformId;
    state.screen = previousScreen;
  }
});
