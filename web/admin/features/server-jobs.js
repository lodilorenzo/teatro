import { api } from '../api.js';

export const SERVER_JOB_STORAGE_KEY = 'teatro.admin.server-jobs.v1';
const SERVER_JOB_POLL_INTERVAL_MS = 750;
const SERVER_JOB_MAX_BACKOFF_MS = 12_000;
const STORED_JOB_LIMIT = 64;
const ID_PATTERN = /^(?:gog|scan|romm)_[A-Za-z0-9_-]+$/u;
const STATUS_URLS = {
  'library-scan': (id) => `/api/admin/library/scans/${encodeURIComponent(id)}`,
  'gog-import': (id) => `/api/admin/gog-imports/${encodeURIComponent(id)}`,
  'romm-import': (id) => `/api/admin/sources/romm/imports/${encodeURIComponent(id)}`,
};

function validRecord(record) {
  const statusUrl = STATUS_URLS[record?.type];
  if (!statusUrl) return false;
  if (typeof record.id !== 'string' || !ID_PATTERN.test(record.id) || record.id.length > 128) return false;
  return record.statusUrl === statusUrl(record.id);
}

export function decodeStoredServerJobs(raw) {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(validRecord).slice(0, STORED_JOB_LIMIT);
  } catch (_) {
    return [];
  }
}

function loadAll() {
  try {
    return decodeStoredServerJobs(globalThis.sessionStorage?.getItem(SERVER_JOB_STORAGE_KEY));
  } catch (_) {
    return [];
  }
}

function saveAll(records) {
  try {
    const valid = records.filter(validRecord).slice(0, STORED_JOB_LIMIT);
    if (valid.length) globalThis.sessionStorage?.setItem(SERVER_JOB_STORAGE_KEY, JSON.stringify(valid));
    else globalThis.sessionStorage?.removeItem(SERVER_JOB_STORAGE_KEY);
  } catch (_) {
    // Browser reattachment is best effort; server processing never depends on it.
  }
}

export function loadServerJobs(type) {
  return loadAll().filter((record) => record.type === type);
}

export function rememberServerJob(record) {
  if (!validRecord(record)) return;
  saveAll([record, ...loadAll().filter(({ type, id }) => type !== record.type || id !== record.id)]);
}

export function forgetServerJob(type, id) {
  saveAll(loadAll().filter((record) => record.type !== type || record.id !== id));
}

export function clearServerJobs(type) {
  saveAll(loadAll().filter((record) => record.type !== type));
}

export function createServerJobPoller(context) {
  const request = context.request || api;
  const schedule = context.setTimeout || ((callback, delay) => globalThis.setTimeout(callback, delay));
  const cancel = context.clearTimeout || ((timer) => globalThis.clearTimeout(timer));
  const runtimes = new Map();

  function runtimeFor(appJobId) {
    let runtime = runtimes.get(appJobId);
    if (!runtime) {
      runtime = { timer: null, inFlight: false, generation: 0 };
      runtimes.set(appJobId, runtime);
    }
    return runtime;
  }

  function stop(appJobId, { remove = false } = {}) {
    const runtime = runtimes.get(appJobId);
    if (!runtime) return;
    if (runtime.timer !== null) cancel(runtime.timer);
    runtime.timer = null;
    runtime.generation += 1;
    if (remove) runtimes.delete(appJobId);
  }

  function current(appJobId, serverJobId, generation) {
    const runtime = runtimes.get(appJobId);
    const job = context.getJob(appJobId);
    return Boolean(runtime && runtime.generation === generation && job?.serverJobId === serverJobId);
  }

  function watch(appJobId, { immediate = false } = {}) {
    if (!context.shouldPoll(appJobId)) return;
    const runtime = runtimeFor(appJobId);
    if (runtime.timer !== null || runtime.inFlight) return;
    const generation = runtime.generation;
    runtime.timer = schedule(() => {
      runtime.timer = null;
      void poll(appJobId, generation);
    }, immediate ? 0 : SERVER_JOB_POLL_INTERVAL_MS);
  }

  async function poll(appJobId, generation) {
    const runtime = runtimeFor(appJobId);
    const job = context.getJob(appJobId);
    if (!job || !context.shouldPoll(appJobId) || runtime.inFlight || generation !== runtime.generation) return;
    const serverJobId = job.serverJobId;
    runtime.inFlight = true;
    try {
      const snapshot = await request(context.requestUrl?.(job) || job.statusUrl);
      if (!current(appJobId, serverJobId, generation)) return;
      const terminal = await context.onSnapshot(
        appJobId,
        snapshot,
        serverJobId,
        () => current(appJobId, serverJobId, generation),
      );
      if (terminal) {
        stop(appJobId, { remove: true });
      } else {
        runtime.inFlight = false;
        watch(appJobId);
      }
    } catch (error) {
      if (!current(appJobId, serverJobId, generation)) return;
      const failures = Number(context.getJob(appJobId)?.progress?.pollFailureCount || 0) + 1;
      const retry = await context.onError(appJobId, error, failures);
      if (retry === false) stop(appJobId, { remove: true });
      else {
        const delay = Math.min(
          SERVER_JOB_MAX_BACKOFF_MS,
          SERVER_JOB_POLL_INTERVAL_MS * (2 ** Math.min(failures, 4)),
        );
        const next = runtimeFor(appJobId);
        const nextGeneration = next.generation;
        next.timer = schedule(() => {
          next.timer = null;
          void poll(appJobId, nextGeneration);
        }, delay);
      }
    } finally {
      runtime.inFlight = false;
    }
  }

  function stopAll() {
    for (const appJobId of [...runtimes.keys()]) stop(appJobId, { remove: true });
  }

  return { watch, stop, stopAll };
}
