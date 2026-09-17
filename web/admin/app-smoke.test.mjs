import test from 'node:test';
import assert from 'node:assert/strict';

import { showError, state } from './state.js';
import { renderGogImport } from './views/gog-import.js';
import { renderJobs } from './views/jobs.js';
import { renderLibrary } from './views/library.js';
import { renderUpload } from './views/upload.js';

test('admin DOM smoke covers login errors, list selection, and upload planning', async () => {
  const app = {
    className: '',
    innerHTML: '',
    querySelector: () => null,
    querySelectorAll: () => [],
  };
  globalThis.window = { addEventListener: () => {}, location: { origin: 'http://teatro.test' } };
  globalThis.document = {
    getElementById(id) {
      return id === 'app' ? app : null;
    },
  };
  globalThis.sessionStorage = {
    getItem: () => null,
    setItem: () => {},
    removeItem: () => {},
  };

  showError(new Error('<offline>'));
  await import(`./main.js?smoke=${Date.now()}`);
  assert.match(app.innerHTML, /id="login-form"/);
  assert.match(app.innerHTML, /<label class="checkbox"><input name="remember_me" type="checkbox" \/> Remember me for 30 days<\/label>/);
  assert.doesNotMatch(app.innerHTML, /name="remember_me"[^>]*checked/);
  assert.match(app.innerHTML, /&lt;offline&gt;/);

  state.error = '';
  state.roms = [{
    id: 7,
    name: 'Selected Game',
    slug: 'selected-game',
    platform_slug: 'nes',
    platform_display_name: 'Nintendo Entertainment System',
    files: [],
  }];
  state.romTotal = 1;
  state.selectedRom = state.roms[0];
  state.selectedRomFiles = { groups: [], ungrouped_files: [], dependencies: [], warnings: [] };
  app.innerHTML = renderLibrary();
  assert.match(app.innerHTML, /data-rom-id="7"/);
  assert.match(app.innerHTML, /Game details/);

  state.uploadPlan = {
    platform_slug: 'nes',
    errors: [],
    warnings: [],
    roms: [{
      plan_id: 'rom-1', title: 'Planned Game', slug: 'planned-game', regions: [],
      groups: [], files: [], dependencies: [],
    }],
  };
  app.innerHTML = renderUpload();
  assert.match(app.innerHTML, /data-planned-title="rom-1"/);
  assert.match(app.innerHTML, /Upload games/);
  assert.match(app.innerHTML, /Follow progress in Jobs/);

  state.gogImportStatus = {
    enabled: true, configured: true, max_extracted_files: 1000,
    max_extracted_bytes: 1024, timeout_seconds: 30,
  };
  app.innerHTML = renderGogImport();
  assert.match(app.innerHTML, /id="gog-import-form"/);
  assert.match(app.innerHTML, /Import installer/);

  state.jobs = [{
    id: 'upload_smoke', type: 'upload', title: 'Planned Game', detail: 'NES · 1 file',
    state: 'running', statusText: 'Uploading grouped batch', createdAt: Date.now(),
    progress: { active: true, percent: 25, loaded: 1, total: 4, speed: 1 },
  }];
  app.innerHTML = renderJobs();
  assert.match(app.innerHTML, /<h1>Jobs<\/h1>/);
  assert.match(app.innerHTML, /Planned Game/);
});
