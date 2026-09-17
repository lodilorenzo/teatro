import test from 'node:test';
import assert from 'node:assert/strict';

import { clearCoverImages, syncCoverImages } from '../../public/shared.js';
import { clearRomSelection, selectRom, state } from '../state.js';
import { renderRomEditForm } from '../views/library-renderers.js';
import { createIgdbController } from './igdb.js';

test('IGDB feedback stays in the game editor and resets with the selection', async () => {
  const previous = { ...state };
  const previousFetch = globalThis.fetch;
  const previousFormData = globalThis.FormData;
  const feedback = { setAttribute(name, value) { this[name] = value; } };
  let markup = '';
  let respond;
  globalThis.FormData = class {
    get(name) { return name === 'q' ? 'Sonic' : '10'; }
  };
  globalThis.fetch = () => new Promise((resolve) => { respond = resolve; });
  state.auth = { header: 'Basic test' };
  state.igdbStatus = { configured: true };
  selectRom({ id: 7, name: 'Sonic', platform_slug: 'genesis' });
  state.editingRomId = 7;
  const controller = createIgdbController({
    app: { querySelector: (selector) => selector === '#igdb-feedback' ? feedback : null },
    render: () => { markup = renderRomEditForm(); },
    setNotice: assert.fail,
    setError: assert.fail,
  });
  const response = (body, status = 200) => new Response(JSON.stringify(body), {
    status, headers: { 'content-type': 'application/json' },
  });
  const search = () => controller.onSearchSubmit({ preventDefault() {}, currentTarget: {} });

  try {
    assert.match(renderRomEditForm(), /id="igdb-feedback"[^>]*role="status"><\/p>/);
    let pending = search();
    assert.equal(feedback.textContent, 'Searching IGDB…');
    assert.equal(feedback.role, 'status');
    assert.equal(markup, '', 'progress must not repaint unsaved editor fields');
    respond(response([]));
    await pending;
    assert.match(markup, /id="igdb-feedback"[^>]*role="status">No matching games found on IGDB/);
    assert.doesNotMatch(markup, /Search results will appear here/);

    pending = search();
    respond(response([{ id: 123, name: 'Sonic' }]));
    await pending;
    assert.match(markup, /Found 1 IGDB candidate/);
    assert.match(markup, /data-apply-igdb="0"/);

    pending = controller.apply(7, 0);
    assert.equal(feedback.textContent, 'Applying IGDB metadata…');
    respond(response({ error: { message: 'IGDB <offline>' } }, 502));
    await pending;
    assert.match(markup, /id="igdb-feedback" class="error" role="alert">IGDB &lt;offline&gt;/);
    assert.doesNotMatch(markup, /<offline>/);
    assert.match(markup, /data-apply-igdb="0"/, 'failed applies can be retried');

    pending = search();
    respond(response({ error: { message: 'Search unavailable' } }, 502));
    await pending;
    assert.match(markup, /role="alert">Search unavailable/);
    assert.doesNotMatch(markup, /data-apply-igdb/);

    selectRom({ id: 8, name: 'Another game' });
    assert.equal(state.igdbFeedback, null);
    pending = search();
    clearRomSelection();
    respond(response([]));
    await pending;
    assert.equal(state.igdbFeedback, null, 'late responses must not restore closed feedback');
  } finally {
    globalThis.fetch = previousFetch;
    globalThis.FormData = previousFormData;
    Object.assign(state, previous);
  }
});

test('IGDB search scopes to the selected ROM platform unless all platforms is explicit', async () => {
  const previous = {
    fetch: globalThis.fetch,
    FormData: globalThis.FormData,
    igdbFeedback: state.igdbFeedback,
    auth: state.auth,
    selectedRom: state.selectedRom,
    igdbResults: state.igdbResults,
  };
  let requestedPath = '';
  globalThis.FormData = class {
    get(name) { return name === 'q' ? 'Sonic' : '10'; }
  };
  globalThis.fetch = async (path) => {
    requestedPath = String(path);
    return new Response('[]', { headers: { 'content-type': 'application/json' } });
  };
  state.auth = { header: 'Basic test' };
  state.selectedRom = { id: 7, platform_slug: 'genesis' };
  const controller = createIgdbController({
    render: () => {},
    setNotice: () => {},
    setError: assert.fail,
  });

  try {
    await controller.onSearchSubmit({ preventDefault() {}, currentTarget: {} });
    assert.equal(requestedPath, '/api/admin/igdb/search?q=Sonic&limit=10&platform=genesis');

    await controller.onSearchSubmit({
      preventDefault() {},
      currentTarget: {},
      submitter: { dataset: { searchAllPlatforms: 'true' } },
    });
    assert.equal(requestedPath, '/api/admin/igdb/search?q=Sonic&limit=10');
  } finally {
    globalThis.fetch = previous.fetch;
    globalThis.FormData = previous.FormData;
    Object.assign(state, {
      auth: previous.auth,
      igdbFeedback: previous.igdbFeedback,
      selectedRom: previous.selectedRom,
      igdbResults: previous.igdbResults,
    });
  }
});

test('applying IGDB metadata rerenders details and reloads a replaced cover at the same path', async () => {
  const previous = {
    fetch: globalThis.fetch,
    URL: globalThis.URL,
    IntersectionObserver: globalThis.IntersectionObserver,
    igdbFeedback: state.igdbFeedback,
    auth: state.auth,
    roms: state.roms,
    selectedRom: state.selectedRom,
    igdbResults: state.igdbResults,
  };
  const coverPath = 'library/nes/7-game/cover-large.jpg';
  let coverFetches = 0;
  let renderCount = 0;
  globalThis.IntersectionObserver = undefined;
  globalThis.URL = {
    createObjectURL: () => `blob:cover-${coverFetches}`,
    revokeObjectURL: () => {},
  };
  globalThis.fetch = async (path) => {
    if (String(path).startsWith('/api/')) {
      return new Response(JSON.stringify({
        rom: {
          id: 7,
          name: 'Updated Game',
          metadatum: { release_year: 1994, genres: ['Action'] },
          path_cover_large: coverPath,
          path_cover_small: coverPath,
        },
        cached_covers: [{ resource_path: coverPath }],
      }), { headers: { 'content-type': 'application/json' } });
    }
    coverFetches += 1;
    return { ok: true, blob: async () => new Blob([`cover-${coverFetches}`]) };
  };
  state.auth = { header: 'Basic test' };
  state.selectedRom = { id: 7, name: 'Old Game', path_cover_large: coverPath };
  state.roms = [state.selectedRom];
  state.igdbResults = [{ id: 123, name: 'Updated Game' }];
  const image = () => ({ dataset: { coverPath }, isConnected: true, src: 'placeholder' });
  const controller = createIgdbController({
    render: () => { renderCount += 1; },
    setNotice: () => {},
    setError: assert.fail,
  });

  try {
    clearCoverImages();
    syncCoverImages({ querySelectorAll: () => [image()] }, state.auth.header);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(coverFetches, 1);

    await controller.apply(7, 0);
    assert.equal(state.selectedRom.name, 'Updated Game');
    assert.equal(state.selectedRom.metadatum.release_year, 1994);
    assert.equal(state.igdbResults.length, 0);
    assert.equal(state.igdbFeedback.message, 'Applied IGDB metadata to Updated Game.');
    assert.equal(renderCount, 1);

    syncCoverImages({ querySelectorAll: () => [image()] }, state.auth.header);
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(coverFetches, 2);
  } finally {
    clearCoverImages();
    globalThis.fetch = previous.fetch;
    globalThis.URL = previous.URL;
    globalThis.IntersectionObserver = previous.IntersectionObserver;
    Object.assign(state, {
      auth: previous.auth,
      igdbFeedback: previous.igdbFeedback,
      roms: previous.roms,
      selectedRom: previous.selectedRom,
      igdbResults: previous.igdbResults,
    });
  }
});
