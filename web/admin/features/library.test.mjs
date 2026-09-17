import test from 'node:test';
import assert from 'node:assert/strict';

import { state } from '../state.js';
import { createLibraryController } from './library.js';

test('library filters apply automatically, preserve exact search input and focus, and clear', async () => {
  const previousFormData = globalThis.FormData;
  const previousFilterState = {
    search: state.search,
    platformFilter: state.platformFilter,
    missingCoverFilter: state.missingCoverFilter,
    romOffset: state.romOffset,
    romSelectionRequestId: state.romSelectionRequestId,
    selectedRom: state.selectedRom,
    selectedRomFiles: state.selectedRomFiles,
    editingRomId: state.editingRomId,
    igdbResults: state.igdbResults,
  };
  const listeners = {};
  const searchInput = {
    value: '  sonic  ',
    selectionStart: 5,
    selectionEnd: 5,
    addEventListener: (type, listener) => { listeners[`search:${type}`] = listener; },
  };
  const platformSelect = {
    value: '',
    addEventListener: (type, listener) => { listeners[`platform:${type}`] = listener; },
  };
  const missingCoverInput = {
    checked: false,
    addEventListener: (type, listener) => { listeners[`cover:${type}`] = listener; },
  };
  const clearButton = {
    addEventListener: (type, listener) => { listeners[`clear:${type}`] = listener; },
  };
  const ownerDocument = { activeElement: searchInput };
  const form = {
    isConnected: true,
    ownerDocument,
    elements: { search: searchInput, platform: platformSelect, missing_cover: missingCoverInput },
    addEventListener: (type, listener) => { listeners[`form:${type}`] = listener; },
    querySelector: (selector) => ({
      'input[name="search"]': searchInput,
      'select[name="platform"]': platformSelect,
      'input[name="missing_cover"]': missingCoverInput,
      '[data-action="clear-library-filter"]': clearButton,
    })[selector] || null,
  };
  searchInput.form = form;
  platformSelect.form = form;
  missingCoverInput.form = form;
  clearButton.form = form;
  globalThis.FormData = class {
    constructor(source) { this.source = source; }
    get(name) {
      const element = this.source.elements[name];
      return name === 'missing_cover' ? (element.checked ? 'on' : null) : element.value;
    }
  };
  const loads = [];
  let renderCount = 0;
  let focusCount = 0;
  let restoredSelection;
  const renderedSearchInput = {
    focus: () => { focusCount += 1; },
    setSelectionRange: (...selection) => { restoredSelection = selection; },
  };
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => ({
        '#library-filter-form': form,
        '#library-filter-form input[name="search"]': renderedSearchInput,
      })[selector] || null,
      querySelectorAll: () => [],
    },
    loadRoms: async () => {
      loads.push([state.search, state.platformFilter, state.missingCoverFilter]);
    },
    render: () => { renderCount += 1; },
    setError: assert.fail,
  });

  try {
    controller.bind();
    listeners['search:input']({ currentTarget: searchInput });
    await new Promise((resolve) => setTimeout(resolve, 300));

    assert.equal(focusCount, 1);
    assert.deepEqual(restoredSelection, [5, 5]);

    platformSelect.value = '3';
    ownerDocument.activeElement = platformSelect;
    await listeners['platform:change']({ currentTarget: platformSelect });

    missingCoverInput.checked = true;
    ownerDocument.activeElement = missingCoverInput;
    await listeners['cover:change']({ currentTarget: missingCoverInput });

    ownerDocument.activeElement = clearButton;
    await listeners['clear:click']({ currentTarget: clearButton });

    assert.deepEqual(loads, [
      ['  sonic  ', '', false], ['  sonic  ', '3', false],
      ['  sonic  ', '3', true], ['', '', false],
    ]);
    assert.equal(searchInput.value, '');
    assert.equal(platformSelect.value, '');
    assert.equal(missingCoverInput.checked, false);
    assert.equal(renderCount, 4);
    assert.equal(focusCount, 1);
  } finally {
    globalThis.FormData = previousFormData;
    Object.assign(state, previousFilterState);
  }
});

test('library view controls switch the rendered layout', () => {
  const previousView = state.libraryView;
  const listeners = {};
  const buttons = ['list', 'grid'].map((view) => ({
    dataset: { libraryView: view },
    addEventListener: (type, listener) => { listeners[`${view}:${type}`] = listener; },
  }));
  let renderCount = 0;
  const controller = createLibraryController({
    app: {
      querySelector: () => null,
      querySelectorAll: (selector) => (selector === '[data-library-view]' ? buttons : []),
    },
    render: () => { renderCount += 1; },
  });

  try {
    controller.bind();
    listeners['grid:click']();
    assert.equal(state.libraryView, 'grid');
    listeners['list:click']();
    assert.equal(state.libraryView, 'list');
    assert.equal(renderCount, 2);
  } finally {
    state.libraryView = previousView;
  }
});

test('library detail drawer closes from its backdrop, button, or Escape key', () => {
  const previous = {
    selectedRom: state.selectedRom,
    selectedRomFiles: state.selectedRomFiles,
    editingRomId: state.editingRomId,
    igdbResults: state.igdbResults,
  };
  const listeners = {};
  const closeElements = ['backdrop', 'button'].map((name) => ({
    addEventListener: (type, listener) => { listeners[`${name}:${type}`] = listener; },
  }));
  const drawer = {
    addEventListener: (type, listener) => { listeners[`drawer:${type}`] = listener; },
  };
  let renderCount = 0;
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => (selector === '.library-detail-drawer' ? drawer : null),
      querySelectorAll: (selector) => (selector === '[data-action="close-rom-detail"]' ? closeElements : []),
    },
    render: () => { renderCount += 1; },
  });

  try {
    state.selectedRom = { id: 7, name: 'Drawer Game' };
    state.selectedRomFiles = { groups: [] };
    controller.bind();

    listeners['backdrop:click']();
    assert.equal(state.selectedRom, null);
    assert.equal(renderCount, 1);

    state.selectedRom = { id: 7, name: 'Drawer Game' };
    listeners['drawer:keydown']({ key: 'Enter' });
    assert.notEqual(state.selectedRom, null);
    listeners['drawer:keydown']({ key: 'Escape' });
    assert.equal(state.selectedRom, null);

    state.selectedRom = { id: 7, name: 'Drawer Game' };
    listeners['button:click']();
    assert.equal(state.selectedRom, null);
    assert.equal(renderCount, 3);
  } finally {
    Object.assign(state, previous);
  }
});

test('library page-size setting reloads the first page', async () => {
  const previousFormData = globalThis.FormData;
  const previousStorage = globalThis.localStorage;
  const previousLimit = state.romLimit;
  const previousOffset = state.romOffset;
  let submit;
  let loadedPage;
  let notice;
  const form = {
    elements: { rom_page_size: { value: '50' } },
    addEventListener: (type, listener) => { if (type === 'submit') submit = listener; },
  };
  globalThis.FormData = class {
    constructor(source) { this.source = source; }
    get(name) { return this.source.elements[name].value; }
  };
  globalThis.localStorage = { setItem: () => {} };
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => (selector === '#library-display-settings-form' ? form : null),
      querySelectorAll: () => [],
    },
    loadRoms: async () => { loadedPage = [state.romLimit, state.romOffset]; },
    render: () => {},
    setNotice: (message) => { notice = message; },
    setError: assert.fail,
  });

  try {
    state.romOffset = 75;
    controller.bind();
    await submit({ preventDefault() {}, currentTarget: form });
    assert.deepEqual(loadedPage, [50, 0]);
    assert.match(notice, /50 titles per page/);
  } finally {
    globalThis.FormData = previousFormData;
    globalThis.localStorage = previousStorage;
    state.romLimit = previousLimit;
    state.romOffset = previousOffset;
  }
});

test('ROM edit uploads a selected cover after saving details', async () => {
  const previousFormData = globalThis.FormData;
  const previousFetch = globalThis.fetch;
  const previousState = {
    auth: state.auth, selectedRom: state.selectedRom, editingRomId: state.editingRomId,
    roms: state.roms,
  };
  const cover = { size: 12, type: 'image/png' };
  const values = {
    name: 'Cover Game', platform_id: '1', summary: '', regions: '', release_year: '',
    genres: '', developers: '', publishers: '', cover,
  };
  let submit;
  const form = {
    dataset: { romId: '7' }, values,
    addEventListener: (type, listener) => { if (type === 'submit') submit = listener; },
  };
  globalThis.FormData = class {
    constructor(source) { this.source = source; }
    get(name) { return this.source.values[name]; }
  };
  const requests = [];
  globalThis.fetch = async (path, options) => {
    requests.push({ path, options });
    const uploaded = path.endsWith('/cover');
    return new Response(JSON.stringify({
      id: 7, name: 'Cover Game', platform_id: 1, regions: [], metadatum: {},
      path_cover_large: uploaded ? 'library/cover.png' : null,
      path_cover_small: uploaded ? 'library/cover.png' : null,
    }), { headers: { 'content-type': 'application/json' } });
  };
  state.auth = { header: 'Basic test' };
  state.selectedRom = { id: 7, name: 'Cover Game' };
  state.editingRomId = 7;
  state.roms = [state.selectedRom];
  let notice;
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => (selector === '#rom-edit-form' ? form : null),
      querySelectorAll: () => [],
    },
    render: () => {},
    setNotice: (message) => { notice = message; },
    setError: assert.fail,
  });

  try {
    controller.bind();
    await submit({ preventDefault() {}, currentTarget: form });
    assert.equal(requests[0].path, '/api/admin/roms/7');
    assert.equal(requests[0].options.method, 'PATCH');
    assert.equal(requests[1].path, '/api/admin/roms/7/cover');
    assert.equal(requests[1].options.method, 'PUT');
    assert.equal(requests[1].options.body, cover);
    assert.equal(requests[1].options.headers.get('Content-Type'), 'image/png');
    assert.equal(state.selectedRom.path_cover_large, 'library/cover.png');
    assert.match(notice, /and its cover/);
  } finally {
    globalThis.FormData = previousFormData;
    globalThis.fetch = previousFetch;
    Object.assign(state, previousState);
  }
});

test('package deletion warns that the complete game folder will be removed', async () => {
  const previousRom = state.selectedRom;
  const previousConfirm = globalThis.confirm;
  let submit;
  let prompt;
  const form = {
    dataset: { romId: '7' },
    addEventListener: (type, listener) => { if (type === 'submit') submit = listener; },
  };
  globalThis.confirm = (message) => { prompt = message; return false; };
  state.selectedRom = {
    id: 7,
    name: 'Package Game',
    files: [{ id: 1 }, { id: 2 }],
  };
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => (selector === '#delete-rom-form' ? form : null),
      querySelectorAll: () => [],
    },
    render: () => {},
    setError: assert.fail,
  });

  try {
    controller.bind();
    await submit({ preventDefault() {}, currentTarget: form });
    assert.match(prompt, /complete folder—including sidecars/);
    assert.match(prompt, /platform folder will remain/);
  } finally {
    state.selectedRom = previousRom;
    globalThis.confirm = previousConfirm;
  }
});

test('package downloads exchange Basic auth for a normal one-time form download', async () => {
  const previousAuth = state.auth;
  const previousRom = state.selectedRom;
  const previousFetch = globalThis.fetch;
  const previousDocument = globalThis.document;
  const ticket = `teatro_dl_${'A'.repeat(43)}`;
  let click;
  let request;
  let renderCount = 0;
  let submitted;
  const button = {
    dataset: { downloadPackage: '7' },
    disabled: false,
    isConnected: true,
    addEventListener: (_event, listener) => { click = listener; },
  };

  globalThis.fetch = async (path, options) => {
    request = { path, options, notice: notices.at(-1), renderCount };
    return new Response(JSON.stringify({ ticket, expires_in_seconds: 60 }), {
      headers: { 'content-type': 'application/json', 'cache-control': 'private, no-store' },
    });
  };
  globalThis.document = {
    body: { append: () => {} },
    createElement(tagName) {
      if (tagName === 'input') return {};
      const form = {
        append(input) { this.input = input; },
        submit() {
          submitted = {
            method: this.method, action: this.action, hidden: this.hidden, input: this.input,
          };
        },
        remove() {},
      };
      return form;
    },
  };
  state.auth = { header: 'Basic test' };
  state.selectedRom = {
    id: 7,
    name: 'Package Game',
    files: [{ id: 1 }, { id: 2 }],
  };
  const notices = [];
  const errors = [];
  const controller = createLibraryController({
    app: {
      querySelector: (selector) => (selector === '[data-download-package]' ? button : null),
      querySelectorAll: () => [],
    },
    render: () => { renderCount += 1; },
    setNotice: (message) => notices.push(message),
    setError: (error) => errors.push(error?.message || String(error)),
  });

  try {
    controller.bind();
    await click({ currentTarget: button });

    assert.equal(request.path, '/api/roms/7/archive-ticket');
    assert.equal(request.options.method, 'POST');
    assert.equal(request.options.headers.get('Authorization'), 'Basic test');
    assert.equal(request.notice, 'Preparing package ZIP on Teatro…');
    assert.equal(request.renderCount, 1);
    assert.deepEqual(submitted, {
      method: 'post',
      action: '/api/downloads/archive',
      hidden: true,
      input: { type: 'hidden', name: 'ticket', value: ticket },
    });
    assert.equal(button.disabled, false);
    assert.equal(renderCount, 2);
    assert.deepEqual(errors, []);
    assert.match(notices.at(-1), /download started in your browser/);
  } finally {
    state.auth = previousAuth;
    state.selectedRom = previousRom;
    globalThis.fetch = previousFetch;
    globalThis.document = previousDocument;
  }
});
