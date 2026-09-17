const DB_NAME = 'teatro-admin-background-transfers-v1';
const STORE_NAME = 'transfers';
const PAYLOAD_STORE_NAME = 'payloads';
const RETRY_INTERVAL_MS = 750;
const RETRY_WAIT_BUDGET_MS = 15 * 60 * 1000;
const PROGRESS_POLL_INTERVAL_MS = 250;
const PROGRESS_WRITE_INTERVAL_MS = 100;
const OWNER_STORAGE_KEY = 'teatro.admin.background-transfer-owner.v1';
const active = new Map();
const pendingStarts = new Set();
let fallbackOwnerId = '';

function supported() {
  return Boolean(globalThis.indexedDB);
}

function ownerId() {
  try {
    const saved = globalThis.sessionStorage?.getItem(OWNER_STORAGE_KEY);
    if (saved) return saved;
    const random = new Uint32Array(4);
    const entropy = globalThis.crypto?.getRandomValues
      ? [...globalThis.crypto.getRandomValues(random)]
        .map((value) => value.toString(16).padStart(8, '0')).join('')
      : `${Date.now().toString(36)}_${Math.random().toString(36).slice(2)}`;
    const created = `owner_${entropy}`;
    globalThis.sessionStorage?.setItem(OWNER_STORAGE_KEY, created);
    return created;
  } catch (_) {
    if (!fallbackOwnerId) fallbackOwnerId = `owner_${Date.now().toString(36)}`;
    return fallbackOwnerId;
  }
}

function openDatabase() {
  if (!supported()) return Promise.reject(new Error('This browser cannot persist background uploads.'));
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 2);
    request.onupgradeneeded = () => {
      if (!request.result.objectStoreNames.contains(STORE_NAME)) {
        request.result.createObjectStore(STORE_NAME, { keyPath: 'id' });
      }
      if (!request.result.objectStoreNames.contains(PAYLOAD_STORE_NAME)) {
        request.result.createObjectStore(PAYLOAD_STORE_NAME, { keyPath: 'id' });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error || new Error('Could not open background upload storage.'));
  });
}

async function transact(storeName, mode, operation) {
  const database = await openDatabase();
  try {
    return await new Promise((resolve, reject) => {
      const transaction = database.transaction(storeName, mode);
      const request = operation(transaction.objectStore(storeName));
      let result;
      request.onsuccess = () => { result = request.result; };
      request.onerror = () => reject(request.error || new Error('Background upload storage failed.'));
      transaction.oncomplete = () => resolve(result);
      transaction.onabort = () => reject(transaction.error || new Error('Background upload storage was aborted.'));
    });
  } finally {
    database.close();
  }
}

async function getRecord(id) {
  const record = await transact(STORE_NAME, 'readonly', (store) => store.get(id));
  return record?.ownerId === ownerId() ? record : null;
}

function putRecord(record) {
  record.updatedAt = Date.now();
  return transact(STORE_NAME, 'readwrite', (store) => store.put(record)).then(() => record);
}

function getPayload(id) {
  return transact(PAYLOAD_STORE_NAME, 'readonly', (store) => store.get(id));
}

function removePayload(id) {
  return transact(PAYLOAD_STORE_NAME, 'readwrite', (store) => store.delete(id));
}

async function putNewTransfer(record, payload) {
  const database = await openDatabase();
  try {
    await new Promise((resolve, reject) => {
      const transaction = database.transaction([STORE_NAME, PAYLOAD_STORE_NAME], 'readwrite');
      transaction.objectStore(STORE_NAME).put(record);
      transaction.objectStore(PAYLOAD_STORE_NAME).put(payload);
      transaction.oncomplete = resolve;
      transaction.onabort = () => reject(transaction.error || new Error('Background upload storage was aborted.'));
    });
  } finally {
    database.close();
  }
}

function formEntries(form) {
  return [...form.entries()].map(([name, value]) => ({
    name,
    value,
    fileName: typeof value === 'string' ? null : value.name,
  }));
}

function buildForm(entries) {
  const form = new FormData();
  for (const entry of entries || []) {
    if (typeof entry.value === 'string') form.append(entry.name, entry.value);
    else form.append(entry.name, entry.value, entry.fileName);
  }
  return form;
}

export function calculateTransferProgress(previousLoaded, totalBytes, event, elapsedMs) {
  const total = Number(totalBytes || event.total || 0);
  const ratio = event.lengthComputable && event.total > 0
    ? Math.min(1, event.loaded / event.total)
    : total > 0 ? Math.min(1, event.loaded / total) : 0;
  const observedLoaded = total * ratio;
  const loaded = Math.max(Number(previousLoaded || 0), observedLoaded);
  return {
    loaded,
    total,
    percent: total > 0 ? Math.min(100, (loaded / total) * 100) : ratio * 100,
    speed: observedLoaded / Math.max(elapsedMs / 1000, 0.001),
  };
}

function sleep(delay) {
  return new Promise((resolve) => setTimeout(resolve, delay));
}

function sendRequest(record, entries, authorization, signal, onProgress) {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    const startedAt = Date.now();
    let lastPublishedAt = 0;
    const abort = () => xhr.abort();
    const cleanup = () => signal.removeEventListener('abort', abort);
    signal.addEventListener('abort', abort, { once: true });
    xhr.open('POST', record.url);
    xhr.setRequestHeader('Accept', 'application/json');
    xhr.setRequestHeader('Authorization', authorization);
    xhr.setRequestHeader('X-Teatro-Transfer-Id', record.id);
    xhr.upload.addEventListener('progress', (event) => {
      const now = Date.now();
      if (now - lastPublishedAt < PROGRESS_WRITE_INTERVAL_MS && event.loaded !== event.total) return;
      lastPublishedAt = now;
      onProgress(calculateTransferProgress(
        record.progress?.loaded,
        record.totalBytes,
        event,
        now - startedAt,
      ));
    });
    xhr.addEventListener('load', () => {
      cleanup();
      resolve({
        status: xhr.status,
        statusText: xhr.statusText,
        text: xhr.responseText,
        location: xhr.getResponseHeader('Location'),
        retryAfter: xhr.getResponseHeader('Retry-After'),
      });
    });
    xhr.addEventListener('error', () => {
      cleanup();
      reject(new Error('Background upload failed due to a network error.'));
    });
    xhr.addEventListener('abort', () => {
      cleanup();
      reject(new Error('Background upload was interrupted.'));
    });
    xhr.send(buildForm(entries));
  });
}

function retryAfterDelay(value, now = Date.now()) {
  const text = String(value || '').trim();
  let delay = RETRY_INTERVAL_MS;
  if (/^\d+$/.test(text)) {
    delay = Number(text) * 1000;
  } else if (text) {
    const retryAt = Date.parse(text);
    if (Number.isFinite(retryAt)) delay = retryAt - now;
  }
  return Number.isFinite(delay)
    ? Math.max(RETRY_INTERVAL_MS, delay)
    : RETRY_WAIT_BUDGET_MS + 1;
}

async function run(id, authorization, signal) {
  for (;;) {
    const record = await getRecord(id);
    if (!record || record.state !== 'running' || signal.aborted) return;
    const pendingRetryDelay = Math.max(0, Number(record.retryAt || 0) - Date.now());
    if (pendingRetryDelay > 0) {
      await sleep(pendingRetryDelay);
      if (signal.aborted) return;
      continue;
    }
    let progressWrites = Promise.resolve();
    try {
      const payload = record.entries ? record : await getPayload(id);
      if (!payload?.entries?.length) {
        record.state = 'failed';
        record.error = 'Background upload payload is unavailable.';
        await putRecord(record);
        return;
      }
      if (signal.aborted) return;
      const response = await sendRequest(record, payload.entries, authorization, signal, (progress) => {
        record.progress = progress;
        const snapshot = { ...record, progress: { ...progress } };
        progressWrites = progressWrites.then(() => putRecord(snapshot)).catch(() => {});
      });
      await progressWrites;
      let body = null;
      try {
        body = response.text ? JSON.parse(response.text) : null;
      } catch (_) {
        body = null;
      }
      let terminalMessage = '';
      if (response.status === 409 && body?.error?.code === 'transfer_in_progress') {
        const delay = retryAfterDelay(response.retryAfter);
        const retryWaitMs = Math.max(0, Number(record.retryWaitMs) || 0);
        if (delay <= RETRY_WAIT_BUDGET_MS - retryWaitMs) {
          record.retryWaitMs = retryWaitMs + delay;
          record.retryAt = Date.now() + delay;
          await putRecord(record);
          await sleep(delay);
          if (signal.aborted) return;
          continue;
        }
        terminalMessage = 'Automatic waiting stopped after the 15-minute retry budget. Check Jobs and the library before starting another upload; the earlier request may have completed.';
      }
      if (response.status < 200
        || response.status >= 300
        || (record.expectedStatus && response.status !== record.expectedStatus)) {
        record.state = 'failed';
        record.status = response.status;
        record.code = body?.error?.code || null;
        record.error = terminalMessage
          || body?.error?.message
          || `${response.status} ${response.statusText || 'Upload failed'}`;
      } else {
        record.state = 'succeeded';
        record.status = response.status;
        record.progress = {
          loaded: record.totalBytes,
          total: record.totalBytes,
          percent: 100,
          speed: 0,
        };
        record.response = record.includeResponseMetadata ? {
          body,
          status: response.status,
          location: response.location,
        } : body;
      }
      delete record.entries;
      delete record.retryWaitMs;
      delete record.retryAt;
      await putRecord(record);
      await removePayload(id).catch(() => null);
      return;
    } catch (_) {
      await progressWrites;
      if (signal.aborted) return;
      // A full-page navigation cancels this page's request. The next Teatro page resumes it.
      await sleep(RETRY_INTERVAL_MS);
      if (signal.aborted) return;
    }
  }
}

function ensureRunning(id, authorization) {
  if (!authorization || active.has(id)) return;
  const controller = new AbortController();
  const promise = run(id, authorization, controller.signal)
    .catch(() => {})
    .finally(() => active.delete(id));
  active.set(id, { controller, promise });
}

export function startBackgroundTransfer(form, options) {
  const started = (async () => {
    if (!supported()) throw new Error('This browser cannot keep uploads running across page navigation.');
    const existing = await getRecord(options.jobId);
    if (existing) {
      ensureRunning(existing.id, options.authorization);
      return existing;
    }
    const now = Date.now();
    const record = {
      id: options.jobId,
      ownerId: ownerId(),
      type: options.jobType,
      title: String(options.jobTitle || 'Background transfer'),
      detail: String(options.jobDetail || ''),
      totalBytes: Number(options.totalBytes || 0),
      context: options.context || null,
      url: options.url,
      expectedStatus: options.expectedStatus || null,
      includeResponseMetadata: Boolean(options.includeResponseMetadata),
      state: 'running',
      status: 0,
      code: null,
      error: '',
      response: null,
      retryWaitMs: 0,
      retryAt: 0,
      progress: {
        loaded: 0,
        total: Number(options.totalBytes || 0),
        percent: 0,
        speed: 0,
      },
      createdAt: now,
      updatedAt: now,
    };
    await putNewTransfer(record, {
      id: options.jobId,
      ownerId: ownerId(),
      entries: formEntries(form),
    });
    ensureRunning(record.id, options.authorization);
    return record;
  })();
  pendingStarts.add(started);
  void started.finally(() => pendingStarts.delete(started));
  return started;
}

export async function resumeBackgroundTransfers(authorization) {
  if (!supported() || !authorization) return [];
  const records = await listBackgroundTransfers();
  for (const record of records) {
    if (record.state === 'running') ensureRunning(record.id, authorization);
  }
  return records;
}

export async function settleBackgroundTransferStarts() {
  await Promise.allSettled([...pendingStarts]);
}

function getBackgroundTransfer(id) {
  return getRecord(id);
}

export async function listBackgroundTransfers(type = null) {
  if (!supported()) return [];
  const records = await transact(STORE_NAME, 'readonly', (store) => store.getAll());
  return records
    .filter((record) => record.ownerId === ownerId() && (!type || record.type === type))
    .sort((left, right) => right.createdAt - left.createdAt);
}

export function removeBackgroundTransfer(id) {
  active.get(id)?.controller.abort();
  return Promise.all([
    transact(STORE_NAME, 'readwrite', (store) => store.delete(id)),
    removePayload(id),
  ]).catch(() => null);
}

export async function clearBackgroundTransfers() {
  for (const { controller } of active.values()) controller.abort();
  const records = await listBackgroundTransfers().catch(() => []);
  await Promise.all(records.flatMap(({ id }) => [
    transact(STORE_NAME, 'readwrite', (store) => store.delete(id)).catch(() => null),
    removePayload(id).catch(() => null),
  ]));
}

export async function waitForBackgroundTransfer(id, onProgress = null) {
  for (;;) {
    const record = await getBackgroundTransfer(id);
    if (!record) throw new Error('Background transfer is no longer available.');
    onProgress?.(record.progress || null);
    if (record.state === 'succeeded') return record.response;
    if (record.state === 'failed') {
      const error = new Error(record.error || 'Background transfer failed.');
      error.status = record.status || 0;
      error.code = record.code || null;
      throw error;
    }
    await sleep(PROGRESS_POLL_INTERVAL_MS);
  }
}

export { supported as backgroundTransfersSupported };
