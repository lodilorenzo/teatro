import test from 'node:test';
import assert from 'node:assert/strict';

import {
  calculateTransferProgress, removeBackgroundTransfer, resumeBackgroundTransfers,
  startBackgroundTransfer, waitForBackgroundTransfer,
} from './background-transfer.js';

function fakeIndexedDb({ beforeGet = () => null } = {}) {
  const stores = new Map();
  const clone = (value) => value === undefined ? undefined : structuredClone(value);
  return {
    stores,
    open() {
      const openRequest = {};
      queueMicrotask(() => {
        const database = {
          objectStoreNames: { contains: (name) => stores.has(name) },
          createObjectStore(name) { stores.set(name, new Map()); },
          close() {},
          transaction(names) {
            const transaction = {
              objectStore(name) {
                const records = stores.get(name);
                const request = (result) => {
                  const operation = {};
                  queueMicrotask(() => {
                    operation.result = clone(result);
                    operation.onsuccess?.();
                    queueMicrotask(() => transaction.oncomplete?.());
                  });
                  return operation;
                };
                return {
                  get(key) {
                    const result = records.get(key);
                    const wait = beforeGet(name, key);
                    if (!wait) return request(result);
                    const operation = {};
                    void Promise.resolve(wait).then(() => {
                      operation.result = clone(result);
                      operation.onsuccess?.();
                      queueMicrotask(() => transaction.oncomplete?.());
                    });
                    return operation;
                  },
                  getAll: () => request([...records.values()]),
                  put(value) {
                    records.set(value.id, clone(value));
                    return request(value.id);
                  },
                  delete(key) {
                    records.delete(key);
                    return request(undefined);
                  },
                };
              },
            };
            return transaction;
          },
        };
        openRequest.result = database;
        openRequest.onupgradeneeded?.();
        openRequest.onsuccess?.();
      });
      return openRequest;
    },
  };
}

function installTransferHarness(t, responses, databaseOptions = {}) {
  const previous = {
    indexedDB: globalThis.indexedDB,
    sessionStorage: globalThis.sessionStorage,
    XMLHttpRequest: globalThis.XMLHttpRequest,
  };
  const indexedDB = fakeIndexedDb(databaseOptions);
  const attempts = [];
  const session = new Map([
    ['teatro.admin.background-transfer-owner.v1', 'owner_test'],
  ]);

  class FakeEventTarget {
    constructor() { this.listeners = new Map(); }
    addEventListener(name, listener) { this.listeners.set(name, listener); }
    removeEventListener(name, listener) {
      if (this.listeners.get(name) === listener) this.listeners.delete(name);
    }
    emit(name, event = {}) { this.listeners.get(name)?.(event); }
  }

  class FakeXmlHttpRequest extends FakeEventTarget {
    constructor() {
      super();
      this.upload = new FakeEventTarget();
      this.headers = new Map();
      this.aborted = false;
    }
    open(method, url) { this.method = method; this.url = url; }
    setRequestHeader(name, value) { this.headers.set(name.toLowerCase(), value); }
    getResponseHeader(name) { return this.response?.headers?.[name.toLowerCase()] ?? null; }
    send() {
      const response = responses.shift();
      assert.ok(response, 'unexpected transfer attempt');
      attempts.push({ id: this.headers.get('x-teatro-transfer-id'), url: this.url });
      this.response = response;
      queueMicrotask(() => {
        if (this.aborted) return;
        this.status = response.status;
        this.statusText = response.statusText || '';
        this.responseText = response.text || '';
        this.emit('load');
      });
    }
    abort() {
      if (this.aborted) return;
      this.aborted = true;
      this.emit('abort');
    }
  }

  globalThis.indexedDB = indexedDB;
  globalThis.sessionStorage = {
    getItem: (key) => session.get(key) ?? null,
    setItem: (key, value) => session.set(key, String(value)),
  };
  globalThis.XMLHttpRequest = FakeXmlHttpRequest;
  t.after(() => {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
  });
  return { attempts, indexedDB };
}

function response(status, code, message, retryAfter) {
  return {
    status,
    statusText: status === 409 ? 'Conflict' : 'Created',
    text: JSON.stringify(code ? { error: { code, message } } : { roms: [] }),
    headers: retryAfter === undefined ? {} : { 'retry-after': retryAfter },
  };
}

function start(id) {
  const form = new FormData();
  form.set('platform_slug', 'genesis');
  return startBackgroundTransfer(form, {
    jobId: id,
    jobType: 'upload',
    url: '/api/admin/upload-batches',
    authorization: 'Bearer test',
  });
}

test('background upload progress advances and never regresses after navigation retry', () => {
  const halfway = calculateTransferProgress(0, 1_000, {
    lengthComputable: true, loaded: 500, total: 1_100,
  }, 1_000);
  assert.equal(halfway.percent, 500 / 1_100 * 100);
  assert.equal(halfway.loaded, 500 / 1_100 * 1_000);
  assert.ok(halfway.speed > 0);

  const retried = calculateTransferProgress(halfway.loaded, 1_000, {
    lengthComputable: true, loaded: 100, total: 1_100,
  }, 500);
  assert.equal(retried.loaded, halfway.loaded);
  assert.equal(retried.percent, halfway.percent);
});

test('permanent conflicts fail once with the server message and remove payload bytes', async (t) => {
  const id = 'upload_permanent_conflict';
  const harness = installTransferHarness(t, [
    response(409, 'conflict', 'No available filename remains'),
  ]);
  await start(id);
  await assert.rejects(waitForBackgroundTransfer(id), {
    status: 409,
    code: 'conflict',
    message: 'No available filename remains',
  });
  assert.deepEqual(harness.attempts.map(({ id: attemptId }) => attemptId), [id]);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
  await removeBackgroundTransfer(id);
});

test('only transfer_in_progress retries with the same ID and honors Retry-After', async (t) => {
  const id = 'upload_running_retry';
  const harness = installTransferHarness(t, [
    response(409, 'transfer_in_progress', 'still running', '1'),
    response(201),
  ]);
  const startedAt = Date.now();
  await start(id);
  assert.deepEqual(await waitForBackgroundTransfer(id), { roms: [] });
  assert.ok(Date.now() - startedAt >= 900);
  assert.deepEqual(harness.attempts.map(({ id: attemptId }) => attemptId), [id, id]);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
  await removeBackgroundTransfer(id);
});

test('running replies exhaust the persisted 15-minute wait budget', async (t) => {
  const id = 'upload_retry_exhausted';
  const harness = installTransferHarness(t, [
    response(409, 'transfer_in_progress', 'still running', '0'),
    response(409, 'transfer_in_progress', 'still running', '901'),
  ]);
  await start(id);
  await assert.rejects(waitForBackgroundTransfer(id), {
    status: 409,
    code: 'transfer_in_progress',
    message: /15-minute retry budget/,
  });
  assert.deepEqual(harness.attempts.map(({ id: attemptId }) => attemptId), [id, id]);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
  await removeBackgroundTransfer(id);
});

test('malformed conflict bodies and Retry-After values stay bounded', async (t) => {
  const id = 'upload_malformed_retry';
  const malformed = response(409, 'transfer_in_progress', 'still running', 'not-a-delay');
  const malformedBody = response(409);
  malformedBody.text = '{';
  const harness = installTransferHarness(t, [malformed, malformedBody]);
  await start(id);
  await assert.rejects(waitForBackgroundTransfer(id), {
    status: 409,
    code: null,
    message: '409 Conflict',
  });
  assert.equal(harness.attempts.length, 2);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
  await removeBackgroundTransfer(id);
});

test('reattachment waits for the persisted Retry-After deadline', async (t) => {
  const id = 'upload_reattached_retry';
  const harness = installTransferHarness(t, [response(201)]);
  const now = Date.now();
  harness.indexedDB.stores.set('transfers', new Map([[id, {
    id,
    ownerId: 'owner_test',
    state: 'running',
    url: '/api/admin/upload-batches',
    retryWaitMs: 1_000,
    retryAt: now + 1_000,
    createdAt: now,
  }]]));
  harness.indexedDB.stores.set('payloads', new Map([[id, {
    id,
    ownerId: 'owner_test',
    entries: [{ name: 'platform_slug', value: 'genesis', fileName: null }],
  }]]));

  await resumeBackgroundTransfers('Bearer test');
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(harness.attempts.length, 0);
  assert.deepEqual(await waitForBackgroundTransfer(id), { roms: [] });
  assert.ok(Date.now() - now >= 900);
  assert.equal(harness.attempts.length, 1);
  await removeBackgroundTransfer(id);
});

test('cancelling during the payload read prevents the first submission', async (t) => {
  const id = 'upload_cancel_read';
  let releasePayloadRead;
  let markPayloadReadStarted;
  let blocked = false;
  const payloadReadStarted = new Promise((resolve) => { markPayloadReadStarted = resolve; });
  const payloadRead = new Promise((resolve) => { releasePayloadRead = resolve; });
  const harness = installTransferHarness(t, [response(201)], {
    beforeGet(storeName) {
      if (storeName !== 'payloads' || blocked) return null;
      blocked = true;
      markPayloadReadStarted();
      return payloadRead;
    },
  });

  await start(id);
  await payloadReadStarted;
  const cancellation = removeBackgroundTransfer(id);
  releasePayloadRead();
  await cancellation;
  await new Promise((resolve) => setTimeout(resolve, 50));
  assert.equal(harness.attempts.length, 0);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
});

test('cancelling during Retry-After does not submit again', async (t) => {
  const id = 'upload_cancel_retry';
  const harness = installTransferHarness(t, [
    response(409, 'transfer_in_progress', 'still running', '1'),
  ]);
  await start(id);
  while (harness.attempts.length === 0) await new Promise((resolve) => setTimeout(resolve, 5));
  await removeBackgroundTransfer(id);
  await new Promise((resolve) => setTimeout(resolve, 1_050));
  assert.equal(harness.attempts.length, 1);
  assert.equal(harness.indexedDB.stores.get('payloads').has(id), false);
});
