import { api } from '../api.js';
import { formatBytes } from '../dom.js';
import {
  addJob, applyGogImportJobSnapshot, beginGogImportJob, beginGogImportUpload,
  commitGogImportTitle, findJob, initialGogImportProgress, markGogImportJobUnavailable,
  removeJob, resetGogImportWorkflow, restoreGogImportJob, setGogImportFiles,
  setGogImportPollingFailure, setGogImportPostProcessing, setGogImportTitleManual,
  state, syncGogImportTitleSuggestion, updateJob, updateJobProgress,
} from '../state.js';
import {
  GOG_IMPORT_PHASE_LABELS, gogImportSelectionLabel, gogImportTitleHint,
  renderGogImportFileList,
} from '../views/gog-import.js';
import { suggestGogTitleFromSelection } from './gog-title.js';
import {
  clearServerJobs, createServerJobPoller, forgetServerJob, loadServerJobs,
  rememberServerJob,
} from './server-jobs.js';
import { createUploadTransfer } from './upload-transfer.js';
import {
  listBackgroundTransfers, removeBackgroundTransfer, waitForBackgroundTransfer,
} from './background-transfer.js';

const GOG_IMPORT_MAX_RESPONSE_EVENTS = 512;
const GOG_IMPORT_MAX_EVENT_TEXT_BYTES = 4 * 1024;
const GOG_IMPORT_MAX_JOB_ID_BYTES = 128;
const GOG_SETUP_FILE_NAME_MAX_BYTES = 255;
const UTF8_ENCODER = new TextEncoder();
const GOG_IMPORT_STATES = new Set(['queued', 'running', 'succeeded', 'failed', 'cancelled']);
const GOG_IMPORT_EVENT_KINDS = new Set(['phase', 'output']);
const GOG_IMPORT_OUTPUT_STREAMS = new Set(['stdout', 'stderr']);
const GOG_IMPORT_ID = /^gog_[A-Za-z0-9_-]+$/u;

export function validateGogSetupSelection(files) {
  if (!files.length) throw new Error('Select one setup .exe and every matching .bin part.');
  const names = new Set();
  let executables = 0;
  for (const file of files) {
    const name = String(file?.name || '');
    if (!name
      || name.trim() !== name
      || UTF8_ENCODER.encode(name).byteLength > GOG_SETUP_FILE_NAME_MAX_BYTES
      || name.includes('/')
      || name.includes('\\')
      || /[\u0000-\u001f\u007f-\u009f]/u.test(name)) {
      throw new Error('Setup filenames must be flat names no longer than 255 UTF-8 bytes.');
    }
    const lowerName = name.toLocaleLowerCase('en-US');
    if (names.has(lowerName)) throw new Error('Setup filenames must be unique case-insensitively.');
    names.add(lowerName);
    if (lowerName.endsWith('.exe')) executables += 1;
    else if (!lowerName.endsWith('.bin')) throw new Error('Only setup .exe and .bin files are accepted.');
  }
  if (executables !== 1) throw new Error('Select exactly one setup .exe file.');
  return true;
}

function statusUrlFor(jobId) {
  return `/api/admin/gog-imports/${encodeURIComponent(jobId)}`;
}

function validJobId(jobId) {
  return typeof jobId === 'string'
    && UTF8_ENCODER.encode(jobId).byteLength <= GOG_IMPORT_MAX_JOB_ID_BYTES
    && GOG_IMPORT_ID.test(jobId);
}

export function validateGogImportCreateResponse(response) {
  if (response?.status !== 202) throw new Error('Teatro did not accept the GOG import as a background job.');
  const body = response?.body;
  const job = body?.job;
  if (!validJobId(job?.id) || job?.state !== 'queued' || job?.phase !== 'queued') {
    throw new Error('Teatro returned an invalid GOG import job summary.');
  }
  const expectedUrl = statusUrlFor(job.id);
  if (body?.status_url !== expectedUrl || response?.location !== expectedUrl) {
    throw new Error('Teatro returned an invalid GOG import status URL.');
  }
  return { job, statusUrl: expectedUrl };
}

function validateProgress(progress) {
  if (progress === null) return;
  if (!progress || !Number.isFinite(progress.percent) || progress.percent < 0 || progress.percent > 100) {
    throw new Error('Teatro returned invalid GOG import progress.');
  }
  if (progress.kind === 'percent') {
    if (progress.current !== null || progress.total !== null) {
      throw new Error('Teatro returned invalid percentage progress.');
    }
    return;
  }
  if (progress.kind === 'bytes'
    && Number.isSafeInteger(progress.current)
    && Number.isSafeInteger(progress.total)
    && progress.current >= 0
    && progress.current <= progress.total) {
    const expectedPercent = progress.total === 0
      ? 100
      : Math.min(100, Math.max(0, (progress.current / progress.total) * 100));
    if (Math.abs(progress.percent - expectedPercent) <= Number.EPSILON * 100) return;
  }
  throw new Error('Teatro returned invalid byte progress.');
}

export function validateGogImportJobSnapshot(snapshot, expectedJobId) {
  if (!snapshot || snapshot.id !== expectedJobId || !GOG_IMPORT_STATES.has(snapshot.state)) {
    throw new Error('Teatro returned an invalid GOG import job resource.');
  }
  if (!Object.hasOwn(GOG_IMPORT_PHASE_LABELS, snapshot.phase)) {
    throw new Error('Teatro returned an unknown GOG import phase.');
  }
  validateProgress(snapshot.progress);
  if (!Array.isArray(snapshot.events)
    || snapshot.events.length > GOG_IMPORT_MAX_RESPONSE_EVENTS
    || !Number.isSafeInteger(snapshot.next_event_seq)
    || snapshot.next_event_seq < 0
    || typeof snapshot.output_truncated !== 'boolean') {
    throw new Error('Teatro returned an invalid GOG import event cursor.');
  }
  let previousSequence = 0;
  for (const event of snapshot.events) {
    if (!Number.isSafeInteger(event?.seq)
      || event.seq <= previousSequence
      || event.seq > snapshot.next_event_seq
      || !GOG_IMPORT_EVENT_KINDS.has(event.kind)
      || !Object.hasOwn(GOG_IMPORT_PHASE_LABELS, event.phase)
      || typeof event.text !== 'string'
      || UTF8_ENCODER.encode(event.text).byteLength > GOG_IMPORT_MAX_EVENT_TEXT_BYTES) {
      throw new Error('Teatro returned an invalid GOG import event.');
    }
    if (event.kind === 'phase' && event.stream !== null) {
      throw new Error('Teatro returned an invalid GOG import phase event.');
    }
    if (event.kind === 'output' && !GOG_IMPORT_OUTPUT_STREAMS.has(event.stream)) {
      throw new Error('Teatro returned an invalid GOG import output event.');
    }
    previousSequence = event.seq;
  }
  if (snapshot.state === 'succeeded'
    && (snapshot.phase !== 'complete'
      || snapshot.progress !== null
      || !snapshot.result
      || snapshot.error !== null)) {
    throw new Error('Teatro returned an invalid successful GOG import result.');
  }
  if (snapshot.state === 'failed'
    && (snapshot.progress !== null
      || snapshot.result !== null
      || typeof snapshot.error?.code !== 'string'
      || typeof snapshot.error?.message !== 'string')) {
    throw new Error('Teatro returned an invalid failed GOG import result.');
  }
  if (snapshot.state === 'cancelled'
    && (snapshot.progress !== null || snapshot.result !== null || snapshot.error !== null)) {
    throw new Error('Teatro returned invalid cancelled data for a GOG import.');
  }
  if ((snapshot.state === 'queued' || snapshot.state === 'running')
    && (snapshot.result !== null || snapshot.error !== null)) {
    throw new Error('Teatro returned invalid terminal data for an active GOG import.');
  }
  return snapshot;
}

export class GogImportController {
  constructor(context) {
    this.context = Object.freeze({ ...context });
    this.request = context.request || api;
    this.transfers = new Map();
    this.poller = createServerJobPoller({
      ...context,
      request: this.request,
      getJob: findJob,
      shouldPoll: (appJobId) => this.shouldPoll(appJobId),
      requestUrl: (job) => `${job.statusUrl}?after=${encodeURIComponent(Number(job.progress?.eventCursor || 0))}`,
      onSnapshot: (...args) => this.onSnapshot(...args),
      onError: (...args) => this.onPollError(...args),
    });

    if (state.auth?.header) {
      for (const record of loadServerJobs('gog-import')) {
        const existing = state.jobs.find((job) => job.serverJobId === record.id);
        const appJob = existing || restoreGogImportJob(record.id, record.statusUrl);
        this.poller.watch(appJob.id, { immediate: true });
      }
    } else {
      clearServerJobs('gog-import');
    }
    void this.restoreBackgroundJobs();
  }

  async restoreBackgroundJobs() {
    const existingActive = new Set(state.jobs
      .filter((job) => job.type === 'gog-import'
        && ['queued', 'running'].includes(job.state)
        && !job.serverJobId)
      .map(({ id }) => id));
    try {
      const records = await listBackgroundTransfers('gog-import');
      const retained = new Set(records.map(({ id }) => id));
      for (const record of records) {
        let job = findJob(record.id);
        if (job?.serverJobId) {
          void removeBackgroundTransfer(record.id);
          continue;
        }
        if (!job) {
          job = addJob({
            id: record.id,
            type: 'gog-import',
            title: record.title,
            detail: record.detail,
            state: 'running',
            statusText: 'Uploading setup files',
            createdAt: record.createdAt,
            progress: {
              ...initialGogImportProgress(),
              active: true,
              fileName: 'Uploading GOG setup files',
              loaded: Number(record.progress?.loaded || 0),
              total: record.totalBytes,
              percent: Number(record.progress?.percent || 0),
              speed: Number(record.progress?.speed || 0),
            },
          });
        }
        if (job.state === 'queued' || job.state === 'running') {
          void this.launchJob(job.id, null, record.totalBytes, { resume: true });
        }
      }
      for (const job of state.jobs) {
        if (job.type === 'gog-import'
          && existingActive.has(job.id)
          && ['queued', 'running'].includes(job.state)
          && !job.serverJobId
          && !retained.has(job.id)) {
          updateJob(job.id, {
            state: 'failed',
            statusText: 'Import launch interrupted',
            error: 'The background setup upload is no longer available.',
            progress: { ...job.progress, active: false, serverProcessing: false },
          });
        }
      }
      this.context.render();
    } catch (error) {
      for (const job of state.jobs) {
        if (job.type === 'gog-import'
          && existingActive.has(job.id)
          && ['queued', 'running'].includes(job.state)
          && !job.serverJobId) {
          updateJob(job.id, {
            state: 'failed',
            statusText: 'Import launch interrupted',
            error: error?.message || String(error),
            progress: { ...job.progress, active: false, serverProcessing: false },
          });
        }
      }
      this.context.render();
    }
  }

  createTransfer(appJobId) {
    if (this.context.uploadTransfer) return this.context.uploadTransfer;
    return createUploadTransfer({
      loadIgdbStatus: this.context.loadIgdbStatus,
      updateProgress: (progress) => this.updateProgress(appJobId, progress),
    });
  }

  bind() {
    this.bindDropZone();
    this.bindQueueControls();
    this.bindTitleControl();
    this.context.app.querySelector('[data-action="clear-gog-import-list"]')
      ?.addEventListener('click', this.clearSelectedFiles);
    this.context.app.querySelector('#gog-import-form')?.addEventListener('submit', this.onSubmit);
    for (const job of state.jobs) {
      if (job.type === 'gog-import') this.poller.watch(job.id);
    }
  }

  bindTitleControl() {
    this.context.app.querySelector('#gog-import-form input[name="title"]')
      ?.addEventListener('input', this.onTitleInput);
  }

  syncTitleControl() {
    const input = this.context.app.querySelector('#gog-import-form input[name="title"]');
    const hint = this.context.app.querySelector('#gog-import-title-hint');
    if (input && input.value !== state.gogImportTitle) input.value = state.gogImportTitle;
    if (hint) {
      hint.textContent = gogImportTitleHint();
      hint.dataset.titleOrigin = state.gogImportTitleOrigin;
      hint.dataset.titleConfidence = state.gogImportTitleConfidence || '';
    }
  }

  syncTitleSuggestion(files) {
    syncGogImportTitleSuggestion(suggestGogTitleFromSelection(files));
    this.syncTitleControl();
  }

  onTitleInput = (event) => {
    setGogImportTitleManual(event.currentTarget?.value || '');
    this.syncTitleControl();
  };

  bindDropZone() {
    const dropZone = this.context.app.querySelector('#gog-import-drop-zone');
    const fileInput = this.context.app.querySelector('#gog-import-file-input');
    if (!dropZone || !fileInput) return;
    fileInput.addEventListener('change', () => this.appendSelectedFiles([...fileInput.files]));
    for (const eventName of ['dragenter', 'dragover']) {
      dropZone.addEventListener(eventName, (event) => {
        event.preventDefault();
        event.stopPropagation();
        dropZone.classList.add('drag-over');
      });
    }
    for (const eventName of ['dragleave', 'dragend']) {
      dropZone.addEventListener(eventName, (event) => {
        event.preventDefault();
        event.stopPropagation();
        dropZone.classList.remove('drag-over');
      });
    }
    dropZone.addEventListener('drop', (event) => {
      event.preventDefault();
      event.stopPropagation();
      dropZone.classList.remove('drag-over');
      const files = [...(event.dataTransfer?.files || [])];
      if (files.length) this.appendSelectedFiles(files);
    });
  }

  bindQueueControls() {
    this.context.app.querySelectorAll('[data-remove-gog-file]').forEach((button) => {
      button.addEventListener('click', () => this.removeSelectedFile(Number(button.dataset.removeGogFile)));
    });
  }

  appendSelectedFiles(files) {
    if (!files.length) return;
    this.clearFileInput();
    this.updateSelectedFiles([...state.gogImportSelectedFiles, ...files]);
  }

  removeSelectedFile(index) {
    if (!Number.isInteger(index)) return;
    this.clearFileInput();
    this.updateSelectedFiles(state.gogImportSelectedFiles.filter((_, current) => current !== index));
  }

  clearSelectedFiles = () => {
    this.clearFileInput();
    this.updateSelectedFiles([]);
  };

  clearFileInput() {
    const input = this.context.app.querySelector('#gog-import-file-input');
    if (input) input.value = '';
  }

  updateSelectedFiles(files) {
    setGogImportFiles(files);
    this.syncTitleSuggestion(files);
    const label = this.context.app.querySelector('#gog-import-file-name');
    const list = this.context.app.querySelector('#gog-import-file-list');
    const clearButton = this.context.app.querySelector('[data-action="clear-gog-import-list"]');
    if (label) label.textContent = gogImportSelectionLabel(files);
    if (list) {
      list.innerHTML = renderGogImportFileList(files);
      this.bindQueueControls();
    }
    if (clearButton) clearButton.disabled = !files.length;
  }

  updateProgress(appJobId, progress) {
    if (!findJob(appJobId)) return;
    updateJobProgress(appJobId, { ...progress, active: true });
    this.context.jobChanged?.(appJobId);
  }

  acceptCreatedJob(appJobId, response) {
    const created = validateGogImportCreateResponse(response);
    if (!state.auth?.header || !findJob(appJobId)) {
      throw new Error('Teatro accepted the import job, but this tab can no longer retain it. The server job continues.');
    }
    beginGogImportJob(appJobId, created.job, created.statusUrl);
    rememberServerJob({ type: 'gog-import', id: created.job.id, statusUrl: created.statusUrl });
    void removeBackgroundTransfer(appJobId);
    this.context.jobChanged?.(appJobId);
    this.poller.watch(appJobId, { immediate: true });
    return created;
  }

  shouldPoll(appJobId) {
    const job = findJob(appJobId);
    const progress = job?.progress;
    return Boolean(
      state.auth?.header
      && job?.type === 'gog-import'
      && job.serverJobId
      && !progress?.unavailable
      && progress?.jobState !== 'succeeded'
      && progress?.jobState !== 'failed'
      && progress?.jobState !== 'cancelled',
    );
  }

  async onSnapshot(appJobId, snapshot, serverJobId, isCurrent) {
    validateGogImportJobSnapshot(snapshot, serverJobId);
    applyGogImportJobSnapshot(appJobId, snapshot);
    this.context.jobChanged?.(appJobId);
    if (!['succeeded', 'failed', 'cancelled'].includes(snapshot.state)) return false;

    forgetServerJob('gog-import', serverJobId);
    if (snapshot.state === 'cancelled') {
      updateJob(appJobId, {
        state: 'cancelled', statusText: 'Import cancelled',
        progress: { ...findJob(appJobId)?.progress, active: false, jobState: 'cancelled' },
      });
      this.context.render();
      return true;
    }
    await this.handleTerminal(snapshot, appJobId, isCurrent);
    return true;
  }

  handleAuthenticationLoss() {
    this.stopAll({ removeJobs: false, clearStorage: true });
    for (const job of state.jobs.filter(({ type }) => type === 'gog-import')) {
      if (job.state === 'running' || job.state === 'queued') {
        markGogImportJobUnavailable(job.id, 'Authentication was lost while watching this job.');
      }
    }
    this.context.render();
  }

  onPollError(appJobId, error, failures) {
    const job = findJob(appJobId);
    if (!job) return false;
    if (!state.auth?.header || error?.status === 401) {
      this.handleAuthenticationLoss();
      return false;
    }
    if (error?.status === 404) {
      forgetServerJob('gog-import', job.serverJobId);
      markGogImportJobUnavailable(
        appJobId,
        'This in-memory import job is unavailable because it expired or Teatro restarted. Refresh the library before deciding whether to retry.',
      );
      this.transfers.delete(appJobId);
      this.context.render();
      return false;
    }
    if (error?.status === 403) {
      forgetServerJob('gog-import', job.serverJobId);
      markGogImportJobUnavailable(appJobId, 'Admin access to this import job is no longer available.');
      this.transfers.delete(appJobId);
      this.context.setError(new Error('Admin access is required to monitor GOG imports.'));
      this.context.render();
      return false;
    }

    setGogImportPollingFailure(
      appJobId,
      'Connection to Teatro was interrupted. Retrying this existing job without creating another import.',
      failures,
    );
    this.context.jobChanged?.(appJobId);
    return true;
  }

  async handleTerminal(snapshot, appJobId, isCurrent) {
    if (snapshot.state === 'failed') {
      const message = snapshot.error?.message || 'GOG setup import failed.';
      updateJob(appJobId, { state: 'failed', statusText: 'Import failed', error: message });
      this.transfers.delete(appJobId);
      this.context.setError(new Error(message));
      this.context.render();
      return;
    }

    setGogImportPostProcessing(appJobId, true);
    this.context.jobChanged?.(appJobId);
    const response = snapshot.result;
    let selected = response?.rom || null;
    let metadataNotice = '';
    let refreshError = null;
    const transfer = this.transfers.get(appJobId) || this.createTransfer(appJobId);
    try {
      const metadata = await transfer.autoApplyIgdbMetadata(response);
      if (metadata.applied) selected = metadata.rom;
      else if (metadata.reason) metadataNotice = ` IGDB metadata skipped: ${metadata.reason}.`;
    } catch (error) {
      metadataNotice = ' Automatic IGDB metadata matching failed non-fatally.';
      console.warn('Automatic IGDB metadata matching failed after GOG import', error);
    }
    if (!state.auth?.header) {
      this.handleAuthenticationLoss();
      return;
    }
    if (!isCurrent()) return;

    try {
      await Promise.all([
        this.context.loadPlatforms(), this.context.loadStats(), this.context.loadRoms(),
      ]);
    } catch (error) {
      refreshError = error;
    }
    if (!state.auth?.header) {
      this.handleAuthenticationLoss();
      return;
    }
    if (!isCurrent()) return;

    const current = findJob(appJobId);
    const title = selected?.name || current?.title || 'GOG game';
    const summary = response?.import;
    updateJob(appJobId, {
      state: 'succeeded',
      statusText: refreshError ? 'Import complete; library refresh failed' : 'Import complete',
      result: { ...response, rom: selected },
      error: refreshError ? `Library refresh failed: ${refreshError.message}` : '',
      progress: {
        ...current?.progress,
        active: false,
        postProcessing: false,
        jobState: 'succeeded',
        result: response,
      },
    });
    this.transfers.delete(appJobId);
    if (refreshError) {
      this.context.setError(new Error(`Imported ${title}, but refreshing the library failed: ${refreshError.message}`));
    } else {
      const message = summary
        ? `Imported ${title} as a ${formatBytes(summary.archive_bytes)} Windows ZIP from ${summary.extracted_file_count} extracted files.`
        : `Imported ${title} as a Windows ZIP.`;
      this.context.setNotice(`${message}${metadataNotice}`);
    }
    this.context.render();
  }

  cancelJob = async (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'gog-import' || !['queued', 'running'].includes(job.state)) return;
    this.poller.stop(appJobId, { remove: true });
    if (!job.serverJobId) {
      try {
        await this.request(`/api/admin/background-transfers/${encodeURIComponent(appJobId)}?operation=gog`, { method: 'DELETE' });
      } catch (error) {
        if (error?.status !== 404) throw error;
      }
      await removeBackgroundTransfer(appJobId);
      updateJob(appJobId, {
        state: 'cancelled',
        statusText: 'Import cancelled',
        progress: { ...job.progress, active: false, serverProcessing: false },
      });
    } else {
      const snapshot = validateGogImportJobSnapshot(await this.request(job.statusUrl, { method: 'DELETE' }), job.serverJobId);
      if (snapshot.state !== 'cancelled') {
        await this.onSnapshot(appJobId, snapshot, job.serverJobId, () => true);
        return;
      }
      applyGogImportJobSnapshot(appJobId, snapshot);
      forgetServerJob('gog-import', job.serverJobId);
      updateJob(appJobId, { state: 'cancelled', statusText: 'Import cancelled' });
    }
    this.transfers.delete(appJobId);
    this.context.setNotice('GOG import cancelled.');
    this.context.render();
  };

  stopWatching = (appJobId) => {
    const job = findJob(appJobId);
    if (!job || job.type !== 'gog-import') return;
    this.poller.stop(appJobId, { remove: true });
    if (job.serverJobId) forgetServerJob('gog-import', job.serverJobId);
    this.transfers.delete(appJobId);
    removeJob(appJobId);
    this.context.setNotice('Stopped watching the GOG import. This does not cancel server processing.');
    this.context.render();
  };

  stopAll({ removeJobs = false, clearStorage = true } = {}) {
    this.poller.stopAll();
    this.transfers.clear();
    if (clearStorage) clearServerJobs('gog-import');
    if (removeJobs) {
      for (const job of [...state.jobs]) {
        if (job.type === 'gog-import') removeJob(job.id);
      }
    }
  }

  reset({ render = true } = {}) {
    this.stopAll({ removeJobs: true, clearStorage: true });
    resetGogImportWorkflow();
    if (render) this.context.render();
  }

  onSubmit = async (event) => {
    event.preventDefault();
    const title = String(event.currentTarget.elements.title?.value || '').trim();
    const files = [...state.gogImportSelectedFiles];
    try {
      if (!state.gogImportStatus?.enabled || !state.gogImportStatus?.configured) {
        throw new Error('The GOG setup importer is not enabled and configured.');
      }
      if (!title) throw new Error('Enter the game name.');
      validateGogSetupSelection(files);
      commitGogImportTitle(title);
      const totalBytes = files.reduce((sum, file) => sum + Number(file.size || 0), 0);
      const form = new FormData();
      form.set('title', title);
      for (const file of files) form.append('file', file, file.name);

      const appJob = addJob({
        type: 'gog-import',
        title,
        detail: `${files.length} setup file${files.length === 1 ? '' : 's'}`,
        state: 'running',
        statusText: 'Uploading setup files',
        progress: initialGogImportProgress(),
      });
      beginGogImportUpload(appJob.id, totalBytes);
      this.clearFileInput();
      resetGogImportWorkflow();
      this.context.setNotice(`GOG import job “${title}” launched. Track it in Jobs.`);
      this.context.render();
      void this.launchJob(appJob.id, form, totalBytes);
    } catch (error) {
      this.context.setError(error);
      this.context.render();
    }
  };

  async launchJob(appJobId, form, totalBytes, { resume = false } = {}) {
    const transfer = this.createTransfer(appJobId);
    this.transfers.set(appJobId, transfer);
    try {
      const job = findJob(appJobId);
      const response = resume
        ? await waitForBackgroundTransfer(appJobId, (progress) => {
          if (progress) this.updateProgress(appJobId, {
            ...progress,
            active: true,
            fileName: 'Uploading GOG setup files',
          });
        })
        : await transfer.uploadWithProgress(form, {
          durable: true,
          jobId: appJobId,
          jobType: 'gog-import',
          jobTitle: job?.title,
          jobDetail: job?.detail,
          url: '/api/admin/gog-imports',
          fileName: 'Uploading GOG setup files',
          fileSize: totalBytes,
          totalBytes,
          batchStartedAt: performance.now(),
          expectedStatus: 202,
          includeResponseMetadata: true,
          onUploadComplete: () => this.updateProgress(appJobId, {
            active: true,
            fileName: 'Upload complete — waiting for Teatro to admit the import job',
            loaded: totalBytes,
            total: totalBytes,
            percent: 100,
            speed: 0,
          }),
        });
      this.acceptCreatedJob(appJobId, response);
    } catch (error) {
      const job = findJob(appJobId);
      if (job && job.state !== 'cancelled') {
        updateJob(appJobId, {
          state: 'failed',
          statusText: 'Import launch failed',
          error: error?.message || String(error),
          progress: { ...job.progress, active: false, serverProcessing: false },
        });
        this.transfers.delete(appJobId);
        void removeBackgroundTransfer(appJobId);
        this.context.setError(error);
        this.context.render();
      }
    }
  }
}
