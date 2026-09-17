import { api } from '../api.js';
import {
  addJob, applyLibraryScanJobSnapshot, beginLibraryScanJob, findJob,
  initialLibraryScanProgress, markLibraryScanJobUnavailable, removeJob,
  restoreLibraryScanJob, setLibraryScanPollingFailure, state, updateJob,
} from '../state.js';
import {
  clearServerJobs, createServerJobPoller, forgetServerJob, loadServerJobs,
  rememberServerJob,
} from './server-jobs.js';

const PHASES = new Set(['queued', 'discovering', 'planning', 'importing', 'fetching_covers', 'complete']);
const STATES = new Set(['queued', 'running', 'succeeded', 'failed', 'cancelled']);
const SCAN_ID = /^scan_[A-Za-z0-9_-]+$/u;
const UTF8_ENCODER = new TextEncoder();

function statusUrlFor(id) {
  return `/api/admin/library/scans/${encodeURIComponent(id)}`;
}

export function validateLibraryScanCreateResponse(response) {
  const job = response?.job;
  if (!SCAN_ID.test(job?.id || '') || job?.state !== 'queued' || job?.phase !== 'queued') {
    throw new Error('Teatro returned an invalid library scan job summary.');
  }
  const expected = statusUrlFor(job.id);
  if (response?.status_url !== expected) {
    throw new Error('Teatro returned an invalid library scan status URL.');
  }
  return { job, statusUrl: expected };
}

function validCount(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

export function validateLibraryScanSnapshot(snapshot, expectedId) {
  if (!snapshot || snapshot.id !== expectedId || !STATES.has(snapshot.state) || !PHASES.has(snapshot.phase)) {
    throw new Error('Teatro returned an invalid library scan job resource.');
  }
  if (snapshot.progress !== null) {
    const progress = snapshot.progress;
    if (progress?.kind !== 'batches'
      || !validCount(progress.current)
      || !validCount(progress.total)
      || progress.current > progress.total
      || !Number.isFinite(progress.percent)
      || progress.percent < 0
      || progress.percent > 100) {
      throw new Error('Teatro returned invalid library scan progress.');
    }
  }
  if (snapshot.state === 'succeeded') {
    const result = snapshot.result;
    if (snapshot.phase !== 'complete' || snapshot.progress !== null || snapshot.error !== null
      || !result
      || !validCount(result.scanned_file_count)
      || !validCount(result.imported_rom_count)
      || !validCount(result.imported_file_count)
      || !validCount(result.generated_manifest_count)
      || !validCount(result.cover_downloaded_count)
      || !validCount(result.cover_not_downloaded_count)
      || !Array.isArray(result.cover_warnings)
      || result.cover_warnings.length !== result.cover_not_downloaded_count
      || result.imported_rom_count !== result.cover_downloaded_count + result.cover_not_downloaded_count
      || !validCount(result.already_indexed_file_count)
      || !validCount(result.not_imported_file_count)
      || !validCount(result.unimported_file_count)
      || !Array.isArray(result.imported_roms)
      || typeof result.imported_roms_truncated !== 'boolean'
      || !Array.isArray(result.unimported_files)
      || result.unimported_files.length !== result.unimported_file_count
      || result.scanned_file_count !== result.imported_file_count + result.unimported_file_count
      || result.unimported_files.filter(({ disposition }) => disposition === 'not_imported').length !== result.not_imported_file_count
      || result.unimported_files.filter(({ disposition }) => disposition === 'already_indexed').length !== result.already_indexed_file_count) {
      throw new Error('Teatro returned an invalid library scan result.');
    }
    for (const warning of result.cover_warnings) {
      if (!Number.isSafeInteger(warning?.rom_id)
        || warning.rom_id < 1
        || typeof warning.rom_name !== 'string'
        || UTF8_ENCODER.encode(warning.rom_name).byteLength > 4 * 1024
        || typeof warning.reason !== 'string'
        || UTF8_ENCODER.encode(warning.reason).byteLength > 4 * 1024) {
        throw new Error('Teatro returned an unsafe library scan cover warning.');
      }
    }
    for (const file of result.unimported_files) {
      if (!['already_indexed', 'not_imported'].includes(file?.disposition)
        || typeof file.relative_path !== 'string'
        || file.relative_path.startsWith('/')
        || /^[A-Za-z]:[\\/]/u.test(file.relative_path)
        || UTF8_ENCODER.encode(file.relative_path).byteLength > 4 * 1024
        || typeof file.reason_code !== 'string'
        || UTF8_ENCODER.encode(file.reason_code).byteLength > 128
        || typeof file.reason !== 'string'
        || UTF8_ENCODER.encode(file.reason).byteLength > 4 * 1024) {
        throw new Error('Teatro returned an unsafe library scan file result.');
      }
    }
  } else if (snapshot.state === 'cancelled') {
    if (snapshot.result !== null || snapshot.progress !== null || snapshot.error !== null) {
      throw new Error('Teatro returned an invalid cancelled library scan result.');
    }
  } else if (snapshot.state === 'failed') {
    if (snapshot.result !== null || snapshot.progress !== null
      || typeof snapshot.error?.code !== 'string'
      || typeof snapshot.error?.message !== 'string') {
      throw new Error('Teatro returned an invalid failed library scan result.');
    }
  } else if (snapshot.result !== null || snapshot.error !== null) {
    throw new Error('Teatro returned terminal data for an active library scan.');
  }
  return snapshot;
}

export class LibraryScanController {
  constructor(context) {
    this.context = Object.freeze({ ...context });
    this.request = context.request || api;
    this.launchInFlight = false;
    this.poller = createServerJobPoller({
      ...context,
      request: this.request,
      getJob: findJob,
      shouldPoll: (appJobId) => this.shouldPoll(appJobId),
      onSnapshot: (...args) => this.onSnapshot(...args),
      onError: (...args) => this.onPollError(...args),
    });

    if (state.auth?.header) {
      for (const record of loadServerJobs('library-scan')) {
        const existing = state.jobs.find((job) => job.serverJobId === record.id);
        const job = existing || restoreLibraryScanJob(record.id, record.statusUrl);
        this.poller.watch(job.id, { immediate: true });
      }
    } else {
      clearServerJobs('library-scan');
    }
  }

  bind() {
    const button = this.context.app.querySelector('[data-action="scan-library"]');
    if (button) {
      button.disabled = this.launchInFlight;
      button.addEventListener('click', this.launch);
    }
    for (const job of state.jobs) {
      if (job.type === 'library-scan') this.poller.watch(job.id);
    }
  }

  shouldPoll(appJobId) {
    const job = findJob(appJobId);
    return Boolean(
      state.auth?.header
      && job?.type === 'library-scan'
      && job.serverJobId
      && !job.progress?.unavailable
      && !['succeeded', 'failed', 'cancelled'].includes(job.progress?.jobState),
    );
  }

  launch = async () => {
    if (this.launchInFlight) return;
    if (!globalThis.confirm('Scan the managed library for new game files? Sidecars and rejected files are preserved. Import may generate a playlist or rename a complete multi-disc folder to match its playlist.')) return;

    this.launchInFlight = true;
    const button = this.context.app.querySelector('[data-action="scan-library"]');
    if (button) button.disabled = true;
    const appJob = addJob({
      type: 'library-scan',
      title: 'Managed library scan',
      detail: 'Indexing new files in place',
      state: 'queued',
      statusText: 'Submitting scan',
      progress: initialLibraryScanProgress(),
    });
    this.context.setNotice('Library scan launched. Track it in Jobs.');
    this.context.render();

    try {
      const created = validateLibraryScanCreateResponse(await this.request('/api/admin/library/scans', {
        method: 'POST',
      }));
      beginLibraryScanJob(appJob.id, created.job, created.statusUrl);
      rememberServerJob({ type: 'library-scan', id: created.job.id, statusUrl: created.statusUrl });
      this.context.jobChanged?.(appJob.id);
      this.poller.watch(appJob.id, { immediate: true });
    } catch (error) {
      updateJob(appJob.id, {
        state: 'failed',
        statusText: 'Scan launch failed',
        error: error?.message || String(error),
        progress: { ...findJob(appJob.id)?.progress, active: false },
      });
      this.context.setError(error);
      this.context.render();
    } finally {
      this.launchInFlight = false;
      const currentButton = this.context.app.querySelector('[data-action="scan-library"]');
      if (currentButton) currentButton.disabled = false;
    }
  };

  async onSnapshot(appJobId, snapshot, serverJobId) {
    validateLibraryScanSnapshot(snapshot, serverJobId);
    applyLibraryScanJobSnapshot(appJobId, snapshot);
    this.context.jobChanged?.(appJobId);
    if (!['succeeded', 'failed', 'cancelled'].includes(snapshot.state)) return false;

    forgetServerJob('library-scan', serverJobId);
    if (snapshot.state === 'cancelled') {
      updateJob(appJobId, { state: 'cancelled', statusText: 'Scan cancelled' });
      this.context.render();
      return true;
    }
    if (snapshot.state === 'failed') {
      this.context.setError(new Error(snapshot.error?.message || 'Library scan failed.'));
      this.context.render();
      return true;
    }

    let refreshError = null;
    try {
      await Promise.all([
        this.context.loadPlatforms(), this.context.loadStats(), this.context.loadRoms(),
      ]);
    } catch (error) {
      refreshError = error;
    }
    const result = snapshot.result;
    updateJob(appJobId, {
      state: 'succeeded',
      statusText: refreshError ? 'Scan complete; library refresh failed' : 'Scan complete',
      error: refreshError ? `Library refresh failed: ${refreshError.message}` : '',
      result,
    });
    if (refreshError) {
      this.context.setError(new Error(`The scan completed, but refreshing the library failed: ${refreshError.message}`));
    } else if (result.not_imported_file_count || result.cover_not_downloaded_count) {
      this.context.setNotice(`Scan finished: ${result.imported_rom_count} game${result.imported_rom_count === 1 ? '' : 's'} imported; ${result.cover_downloaded_count} covers downloaded; ${result.not_imported_file_count} files and ${result.cover_not_downloaded_count} covers need review in Jobs.`);
    } else {
      this.context.setNotice(`Scan finished: ${result.imported_rom_count} game${result.imported_rom_count === 1 ? '' : 's'} imported; ${result.cover_downloaded_count} covers downloaded; ${result.already_indexed_file_count} files were already indexed.`);
    }
    this.context.render();
    return true;
  }

  async onPollError(appJobId, error, failures) {
    const job = findJob(appJobId);
    if (!job) return false;
    if (error?.status === 404) {
      forgetServerJob('library-scan', job.serverJobId);
      markLibraryScanJobUnavailable(
        appJobId,
        'This in-memory scan job is unavailable because it expired or Teatro restarted. Refresh the library before deciding whether to scan again.',
      );
      this.context.render();
      return false;
    }
    if (error?.status === 401 || error?.status === 403 || !state.auth?.header) {
      forgetServerJob('library-scan', job.serverJobId);
      markLibraryScanJobUnavailable(appJobId, 'Admin access to this library scan is no longer available.');
      this.context.render();
      return false;
    }
    setLibraryScanPollingFailure(
      appJobId,
      'Connection to Teatro was interrupted. Retrying this existing scan without creating another.',
      failures,
    );
    this.context.jobChanged?.(appJobId);
    return true;
  }

  cancelJob = async (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'library-scan' || !job.serverJobId || !['queued', 'running'].includes(job.state)) return;
    this.poller.stop(appJobId, { remove: true });
    const snapshot = validateLibraryScanSnapshot(
      await this.request(job.statusUrl, { method: 'DELETE' }),
      job.serverJobId,
    );
    if (snapshot.state !== 'cancelled') {
      await this.onSnapshot(appJobId, snapshot, job.serverJobId);
      return;
    }
    applyLibraryScanJobSnapshot(appJobId, snapshot);
    forgetServerJob('library-scan', job.serverJobId);
    updateJob(appJobId, { state: 'cancelled', statusText: 'Scan cancelled' });
    await Promise.all([
      this.context.loadPlatforms(), this.context.loadStats(), this.context.loadRoms(),
    ]).catch((error) => this.context.setError(error));
    this.context.setNotice('Library scan cancelled.');
    this.context.render();
  };

  stopWatching = (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'library-scan') return;
    this.poller.stop(appJobId, { remove: true });
    forgetServerJob('library-scan', job.serverJobId);
    removeJob(appJobId);
    this.context.setNotice('Stopped watching the library scan. This does not cancel server processing.');
    this.context.render();
  };

  stopAll({ clearStorage = true } = {}) {
    this.poller.stopAll();
    if (clearStorage) clearServerJobs('library-scan');
  }
}
