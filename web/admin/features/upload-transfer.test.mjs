import test from 'node:test';
import assert from 'node:assert/strict';

import { state } from '../state.js';
import { createUploadTransfer } from './upload-transfer.js';

class FakeEventTarget {
  constructor() {
    this.listeners = new Map();
  }

  addEventListener(name, listener) {
    this.listeners.set(name, listener);
  }

  emit(name, event = {}) {
    this.listeners.get(name)?.(event);
  }
}

class FakeXmlHttpRequest extends FakeEventTarget {
  constructor() {
    super();
    this.upload = new FakeEventTarget();
    this.status = 0;
    this.responseText = '';
    FakeXmlHttpRequest.last = this;
  }

  open() {}

  setRequestHeader() {}

  getResponseHeader(name) {
    return name.toLowerCase() === 'location' ? '/api/admin/gog-imports/gog_test' : null;
  }

  send() {}
}

test('automatic metadata search asks the server to try the selected platform first', async () => {
  const previous = { fetch: globalThis.fetch, auth: state.auth, igdbStatus: state.igdbStatus };
  let requestedPath = '';
  globalThis.fetch = async (path) => {
    requestedPath = String(path);
    return new Response('[]', { headers: { 'content-type': 'application/json' } });
  };
  state.auth = { header: 'Basic test' };
  state.igdbStatus = { configured: true };

  try {
    const transfer = createUploadTransfer({ loadIgdbStatus: async () => {} });
    const result = await transfer.autoApplyIgdbMetadata({
      rom: { id: 7, name: 'Sonic the Hedgehog', platform_slug: 'genesis' },
    });
    assert.equal(
      requestedPath,
      '/api/admin/igdb/search?q=Sonic%20the%20Hedgehog&limit=10&platform=genesis&require_platform_match=true',
    );
    assert.equal(result.applied, false);
    assert.match(result.reason, /selected or any platform/);
  } finally {
    globalThis.fetch = previous.fetch;
    state.auth = previous.auth;
    state.igdbStatus = previous.igdbStatus;
  }
});

test('direct upload applies an all-platform fallback candidate returned by the server', async () => {
  const previous = { fetch: globalThis.fetch, auth: state.auth, igdbStatus: state.igdbStatus };
  const requests = [];
  globalThis.fetch = async (path, options = {}) => {
    requests.push([String(path), options.method || 'GET']);
    if (String(path).includes('/igdb/search')) {
      return new Response('[{"id":123,"name":"Fallback Match"}]', {
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response('{"rom":{"id":7,"name":"Fallback Match","platform_slug":"genesis"}}', {
      headers: { 'content-type': 'application/json' },
    });
  };
  state.auth = { header: 'Basic test' };
  state.igdbStatus = { configured: true };

  try {
    const transfer = createUploadTransfer({ loadIgdbStatus: async () => {} });
    const result = await transfer.autoApplyIgdbMetadata({
      rom: { id: 7, name: 'Missing Scoped Match', platform_slug: 'genesis' },
    });
    assert.equal(result.applied, true);
    assert.equal(result.selected.id, 123);
    assert.deepEqual(requests, [
      ['/api/admin/igdb/search?q=Missing%20Scoped%20Match&limit=10&platform=genesis&require_platform_match=true', 'GET'],
      ['/api/admin/roms/7/metadata/igdb', 'POST'],
    ]);
  } finally {
    globalThis.fetch = previous.fetch;
    state.auth = previous.auth;
    state.igdbStatus = previous.igdbStatus;
  }
});

test('upload progress is self-contained for background Jobs entries', async () => {
  const previousAuth = state.auth;
  const previousXmlHttpRequest = globalThis.XMLHttpRequest;
  const previousPerformance = globalThis.performance;
  state.auth = { header: 'Basic test' };
  globalThis.XMLHttpRequest = FakeXmlHttpRequest;
  globalThis.performance = { now: () => 2000 };

  const transitions = [];
  try {
    const transfer = createUploadTransfer({
      updateProgress: (progress) => transitions.push(progress),
    });
    const pending = transfer.uploadWithProgress(new FormData(), {
      url: '/api/admin/upload-batches',
      fileName: 'Uploading background job',
      fileSize: 100,
      totalBytes: 100,
      batchStartedAt: 1000,
    });
    const xhr = FakeXmlHttpRequest.last;
    xhr.upload.emit('progress', { loaded: 40 });
    assert.deepEqual(transitions[0], {
      active: true,
      fileName: 'Uploading background job',
      fileIndex: 0,
      fileCount: 0,
      loaded: 40,
      total: 100,
      percent: 40,
      speed: 40,
    });
    xhr.status = 201;
    xhr.responseText = '{"roms":[]}';
    xhr.emit('load');
    await pending;
  } finally {
    state.auth = previousAuth;
    globalThis.XMLHttpRequest = previousXmlHttpRequest;
    globalThis.performance = previousPerformance;
  }
});

test('large durable uploads start directly instead of copying the file into IndexedDB', async () => {
  const previousAuth = state.auth;
  const previousXmlHttpRequest = globalThis.XMLHttpRequest;
  const previousIndexedDb = globalThis.indexedDB;
  state.auth = { header: 'Basic test' };
  globalThis.XMLHttpRequest = FakeXmlHttpRequest;
  globalThis.indexedDB = {};
  FakeXmlHttpRequest.last = null;

  try {
    const transfer = createUploadTransfer({ updateProgress: () => {} });
    const pending = transfer.uploadWithProgress(new FormData(), {
      durable: true,
      jobId: 'upload_large',
      url: '/api/admin/upload-batches',
      totalBytes: 35_074_585_924,
    });
    const xhr = FakeXmlHttpRequest.last;
    assert.ok(xhr, 'the network request starts without waiting for IndexedDB');
    xhr.status = 201;
    xhr.responseText = '{"roms":[]}';
    xhr.emit('load');
    await pending;
  } finally {
    state.auth = previousAuth;
    globalThis.XMLHttpRequest = previousXmlHttpRequest;
    if (previousIndexedDb === undefined) delete globalThis.indexedDB;
    else globalThis.indexedDB = previousIndexedDb;
  }
});

test('upload completion accepts the asynchronous 202 job response with metadata', async () => {
  const previousAuth = state.auth;
  const previousXmlHttpRequest = globalThis.XMLHttpRequest;
  const previousPerformance = globalThis.performance;
  state.auth = { header: 'Basic test' };
  globalThis.XMLHttpRequest = FakeXmlHttpRequest;
  globalThis.performance = { now: () => 1000 };

  const transitions = [];
  try {
    const transfer = createUploadTransfer({
      updateProgress: (progress) => transitions.push(progress),
    });
    const pending = transfer.uploadWithProgress(new FormData(), {
      url: '/api/admin/gog-imports',
      fileSize: 10,
      totalBytes: 10,
      expectedStatus: 202,
      includeResponseMetadata: true,
      onUploadComplete: () => transitions.push({ serverProcessing: true }),
    });

    const xhr = FakeXmlHttpRequest.last;
    xhr.upload.emit('load');
    assert.deepEqual(transitions, [{ serverProcessing: true }]);

    xhr.status = 202;
    xhr.responseText = '{"job":{"id":"gog_test"}}';
    xhr.emit('load');
    const response = await pending;
    assert.equal(transitions.at(-1).percent, 100);
    assert.deepEqual(response, {
      body: { job: { id: 'gog_test' } },
      status: 202,
      location: '/api/admin/gog-imports/gog_test',
    });
  } finally {
    state.auth = previousAuth;
    globalThis.XMLHttpRequest = previousXmlHttpRequest;
    globalThis.performance = previousPerformance;
  }
});
