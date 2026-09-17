export const GOG_IMPORT_TRANSCRIPT_LIMIT = 512;
export const LIBRARY_VIEW_STORAGE_KEY = 'teatro.admin.library-view.v1';
export const ROM_PAGE_SIZE_STORAGE_KEY = 'teatro.admin.rom-page-size.v1';
export const MAX_ROM_PAGE_SIZE = 100;
export const UPLOAD_PLATFORM_STORAGE_KEY = 'teatro.admin.upload-platform.v1';
export const NOTIFICATION_DURATION_MS = Object.freeze({
  notice: 6_000,
  error: 10_000,
});
const NOTIFICATION_STACK_LIMIT = 5;
export const JOB_LIST_LIMIT = 50;
export const JOB_STORAGE_KEY = 'teatro.admin.jobs.v1';
// Internal API page size; the RomM screen automatically fetches every page.
export const ROMM_BROWSE_PAGE_SIZE = 48;
const GOG_IMPORT_PHASE_EVENT_LIMIT = 64;
let nextNotificationId = 0;
let nextJobId = 0;

const DEFAULT_ROM_PAGE_SIZE = 75;

function normalizeRomPageSize(pageSize) {
  const normalized = Number(pageSize);
  return Number.isSafeInteger(normalized) && normalized >= 1 && normalized <= MAX_ROM_PAGE_SIZE
    ? normalized
    : DEFAULT_ROM_PAGE_SIZE;
}

function loadSavedRomPageSize() {
  try {
    return normalizeRomPageSize(globalThis.localStorage?.getItem(ROM_PAGE_SIZE_STORAGE_KEY));
  } catch (_) {
    return DEFAULT_ROM_PAGE_SIZE;
  }
}

function loadSavedLibraryView() {
  try {
    return globalThis.localStorage?.getItem(LIBRARY_VIEW_STORAGE_KEY) === 'list' ? 'list' : 'grid';
  } catch (_) {
    return 'grid';
  }
}

function normalizeUploadPlatformId(platformId) {
  const normalized = String(platformId ?? '').trim();
  return /^\d{1,20}$/.test(normalized) ? normalized : '';
}

function loadSavedUploadPlatformId() {
  try {
    return normalizeUploadPlatformId(
      globalThis.localStorage?.getItem(UPLOAD_PLATFORM_STORAGE_KEY),
    );
  } catch (_) {
    return '';
  }
}

function loadSavedJobs() {
  try {
    const parsed = JSON.parse(globalThis.sessionStorage?.getItem(JOB_STORAGE_KEY) || '[]');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((job) => job
      && typeof job.id === 'string'
      && ['upload', 'gog-import', 'library-scan'].includes(job.type)
      && typeof job.state === 'string')
      .slice(0, JOB_LIST_LIMIT);
  } catch (_) {
    return [];
  }
}

function persistJobs() {
  try {
    if (state.jobs.length) globalThis.sessionStorage?.setItem(JOB_STORAGE_KEY, JSON.stringify(state.jobs));
    else globalThis.sessionStorage?.removeItem(JOB_STORAGE_KEY);
  } catch (_) {
    // Browser job history is best effort; active server jobs retain their own reattachment data.
  }
}

export function initialLibraryScanProgress() {
  return {
    active: true,
    jobState: 'queued',
    phase: 'queued',
    jobProgress: null,
    pollingError: '',
    pollFailureCount: 0,
    unavailable: false,
    reattached: false,
  };
}

export function initialGogImportProgress() {
  return {
    active: false,
    serverProcessing: false,
    fileName: '',
    percent: 0,
    loaded: 0,
    total: 0,
    speed: 0,
    jobId: null,
    statusUrl: null,
    jobState: null,
    phase: null,
    jobProgress: null,
    eventCursor: 0,
    phaseEvents: [],
    transcript: [],
    outputTruncated: false,
    pollingError: '',
    pollFailureCount: 0,
    result: null,
    jobError: null,
    reattached: false,
    unavailable: false,
    postProcessing: false,
  };
}

export const state = {
  auth: null,
  user: null,
  screen: 'dashboard',
  platforms: [],
  stats: null,
  sidecarCleanupPreview: null,
  sidecarCleanupLoading: false,
  igdbStatus: null,
  igdbSettings: null,
  roms: [],
  romTotal: 0,
  romOffset: 0,
  romLimit: loadSavedRomPageSize(),
  romRequestId: 0,
  romSelectionRequestId: 0,
  selectedRom: null,
  selectedRomFiles: null,
  editingRomId: null,
  igdbResults: [],
  igdbFeedback: null,
  search: '',
  platformFilter: '',
  missingCoverFilter: false,
  libraryView: loadSavedLibraryView(),
  loading: false,
  notice: '',
  error: '',
  notifications: [],
  uploadSelectedFiles: [],
  uploadPlatformId: loadSavedUploadPlatformId(),
  uploadPlan: null,
  uploadPreviewLoading: false,
  jobs: loadSavedJobs(),
  gogImportStatus: null,
  gogImportTitle: '',
  gogImportTitleOrigin: 'empty',
  gogImportTitleConfidence: null,
  gogImportSelectedFiles: [],
  rommSource: null,
  // A saved source hides its inputs behind a connection summary until the admin chooses to edit.
  rommSourceEditing: false,
  rommSourceDetailOpen: false,
  rommPlatforms: [],
  rommBrowse: initialRommBrowse(),
  // Selection is the import: each entry is a bounded snapshot of one remote game plus the
  // operator's optional overrides, so it survives a filter change and a repaint.
  rommSelected: [],
  rommHideExisting: true,
  // The drawer is the only place a single game is edited, and it holds a working copy until commit.
  rommDrawer: null,
};

function initialRommBrowse() {
  return {
    platformId: '',
    search: '',
    total: 0,
    items: [],
    loading: false,
    refreshing: false,
    loaded: false,
  };
}

export function initialRommImportProgress() {
  return {
    active: true,
    jobState: 'queued',
    phase: 'queued',
    jobProgress: null,
    pollingError: '',
    pollFailureCount: 0,
    unavailable: false,
    reattached: false,
  };
}

export function setAuth(auth) {
  state.auth = auth || null;
}

export function setUser(user) {
  state.user = user;
}

export function setPlatforms(platforms) {
  state.platforms = platforms || [];
}

export function setStats(stats) {
  state.stats = stats;
}

export function setIgdbStatus(status) {
  state.igdbStatus = status;
}

export function setIgdbSettings(settings) {
  state.igdbSettings = settings;
}

export function setGogImportStatus(status) {
  state.gogImportStatus = status;
}

export function setIgdbResults(results) {
  state.igdbResults = results || [];
}

export function beginRomPageRequest() {
  state.romRequestId += 1;
  return state.romRequestId;
}

export function isCurrentRomPageRequest(requestId) {
  return requestId === state.romRequestId;
}

export function setLoading(loading) {
  state.loading = Boolean(loading);
}

function syncCurrentFeedback() {
  const current = state.notifications.at(-1);
  state.notice = current?.kind === 'notice' ? current.message : '';
  state.error = current?.kind === 'error' ? current.message : '';
}

function addNotification(kind, message) {
  const normalizedMessage = String(message || '').trim();
  if (!normalizedMessage) {
    clearFeedback();
    return null;
  }

  const notification = {
    id: ++nextNotificationId,
    kind,
    message: normalizedMessage,
    expiresAt: Date.now() + NOTIFICATION_DURATION_MS[kind],
  };
  state.notifications = [...state.notifications, notification].slice(-NOTIFICATION_STACK_LIMIT);
  syncCurrentFeedback();
  return notification;
}

export function showNotice(message) {
  return addNotification('notice', message);
}

export function showError(error) {
  return addNotification('error', error?.message || String(error || 'Unknown error'));
}

export function dismissNotification(notificationId) {
  state.notifications = state.notifications.filter(({ id }) => id !== notificationId);
  syncCurrentFeedback();
}

export function clearFeedback() {
  state.notifications = [];
  state.notice = '';
  state.error = '';
}

export function setScreen(screen) {
  invalidateRomSelection();
  state.screen = screen;
  state.editingRomId = null;
  state.error = '';
}

export function setLibraryFilter(search, platformFilter, missingCoverFilter) {
  state.search = search;
  state.platformFilter = platformFilter;
  state.missingCoverFilter = Boolean(missingCoverFilter);
  state.romOffset = 0;
  clearRomSelection();
}

export function setRomOffset(offset) {
  clearRomSelection();
  state.romOffset = Number(offset || 0);
}

export function setLibraryView(view) {
  state.libraryView = view === 'list' ? 'list' : 'grid';
  try {
    globalThis.localStorage?.setItem(LIBRARY_VIEW_STORAGE_KEY, state.libraryView);
  } catch (_) {
    // Browser preferences are best effort and must never block the library.
  }
  return state.libraryView;
}

export function setRomPageSize(pageSize) {
  const normalized = normalizeRomPageSize(pageSize);
  state.romLimit = normalized;
  state.romOffset = 0;
  try {
    globalThis.localStorage?.setItem(ROM_PAGE_SIZE_STORAGE_KEY, String(normalized));
  } catch (_) {
    // Browser preferences are best effort and must never block the library.
  }
  return normalized;
}

export function setEditingRom(romId) {
  state.editingRomId = romId || null;
}

export function setRomPage(page) {
  state.roms = page.items || [];
  state.romTotal = page.total ?? state.roms.length;
}

export function clearRomSelection() {
  invalidateRomSelection();
  state.selectedRom = null;
  state.selectedRomFiles = null;
  state.editingRomId = null;
  state.igdbResults = [];
  state.igdbFeedback = null;
}

export function selectRom(rom, files = null) {
  state.selectedRom = rom;
  state.selectedRomFiles = files;
  state.editingRomId = null;
  state.igdbResults = [];
  state.igdbFeedback = null;
  state.screen = 'library';
  state.error = '';
}

export function replaceRom(updated) {
  state.selectedRom = updated;
  state.roms = state.roms.map((rom) => (rom.id === updated.id ? updated : rom));
}

export function setUploadPlatformId(platformId) {
  const normalized = normalizeUploadPlatformId(platformId);
  state.uploadPlatformId = normalized;
  try {
    if (normalized) globalThis.localStorage?.setItem(UPLOAD_PLATFORM_STORAGE_KEY, normalized);
    else globalThis.localStorage?.removeItem(UPLOAD_PLATFORM_STORAGE_KEY);
  } catch (_) {
    // Browser preferences are best effort and must never block an upload.
  }
}

export function setUploadFiles(files) {
  state.uploadSelectedFiles = [...files];
  state.uploadPlan = null;
  state.uploadPreviewLoading = false;
}

export function setUploadPreview(loading, plan = null) {
  state.uploadPreviewLoading = Boolean(loading);
  state.uploadPlan = plan;
}

function createJobId(type) {
  nextJobId += 1;
  const prefix = type === 'gog-import'
    ? 'gog'
    : type === 'library-scan' ? 'scan' : type === 'romm-import' ? 'romm' : 'upload';
  const random = new Uint32Array(4);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(random);
    return `${prefix}_${[...random].map((value) => value.toString(16).padStart(8, '0')).join('')}`;
  }
  return `${prefix}_${Date.now().toString(36)}_${nextJobId.toString(36)}`;
}

export function isJobActive(job) {
  return job?.state === 'queued' || job?.state === 'running';
}

export function activeJobCount() {
  return state.jobs.filter(isJobActive).length;
}

export function findJob(jobId) {
  return state.jobs.find((job) => job.id === jobId) || null;
}

function boundJobHistory(jobs) {
  let retainedFinished = 0;
  return jobs.filter((job) => {
    if (isJobActive(job)) return true;
    retainedFinished += 1;
    return retainedFinished <= JOB_LIST_LIMIT;
  });
}

export function addJob(input) {
  const createdAt = Number(input?.createdAt || Date.now());
  const job = {
    id: String(input?.id || createJobId(input?.type)),
    type: ['gog-import', 'library-scan', 'romm-import'].includes(input?.type)
      ? input.type
      : 'upload',
    title: String(input?.title || 'Untitled job'),
    detail: String(input?.detail || ''),
    state: input?.state || 'queued',
    statusText: String(input?.statusText || ''),
    createdAt,
    updatedAt: Number(input?.updatedAt || createdAt),
    completedAt: Number(input?.completedAt || 0) || null,
    progress: { ...(input?.progress || {}) },
    result: input?.result ?? null,
    error: input?.error ? String(input.error) : '',
    serverJobId: input?.serverJobId ? String(input.serverJobId) : null,
    statusUrl: input?.statusUrl ? String(input.statusUrl) : null,
  };
  state.jobs = boundJobHistory([job, ...state.jobs.filter(({ id }) => id !== job.id)]);
  persistJobs();
  return job;
}

export function updateJob(jobId, patch) {
  let updated = null;
  state.jobs = boundJobHistory(state.jobs.map((job) => {
    if (job.id !== jobId) return job;
    const updatedAt = Date.now();
    const nextState = patch?.state || job.state;
    updated = {
      ...job,
      ...patch,
      id: job.id,
      updatedAt,
      completedAt: job.completedAt
        || (['succeeded', 'failed', 'cancelled'].includes(nextState) ? updatedAt : null),
      progress: patch?.progress ? { ...patch.progress } : job.progress,
    };
    return updated;
  }));
  persistJobs();
  return updated;
}

export function updateJobProgress(jobId, progress) {
  const job = findJob(jobId);
  if (!job) return null;
  return updateJob(jobId, { progress: { ...job.progress, ...progress } });
}

export function removeJob(jobId) {
  const previousLength = state.jobs.length;
  state.jobs = state.jobs.filter((job) => job.id !== jobId);
  persistJobs();
  return state.jobs.length !== previousLength;
}

export function clearFinishedJobs() {
  state.jobs = state.jobs.filter(isJobActive);
  persistJobs();
}

export function resetUploadWorkflow() {
  state.uploadSelectedFiles = [];
  state.uploadPlan = null;
  state.uploadPreviewLoading = false;
}

export function setGogImportTitleManual(title) {
  state.gogImportTitle = String(title || '');
  state.gogImportTitleOrigin = 'manual';
  state.gogImportTitleConfidence = null;
}

export function commitGogImportTitle(title) {
  const value = String(title || '');
  if (state.gogImportTitleOrigin === 'inferred' && state.gogImportTitle === value) return;
  setGogImportTitleManual(value);
}

export function syncGogImportTitleSuggestion(suggestion) {
  if (state.gogImportTitleOrigin === 'manual') return false;
  const title = typeof suggestion?.title === 'string' ? suggestion.title : '';
  const confidence = suggestion?.confidence === 'high' || suggestion?.confidence === 'medium'
    ? suggestion.confidence
    : null;
  if (title && confidence && suggestion?.source === 'gog_setup_filename') {
    state.gogImportTitle = title;
    state.gogImportTitleOrigin = 'inferred';
    state.gogImportTitleConfidence = confidence;
  } else {
    state.gogImportTitle = '';
    state.gogImportTitleOrigin = 'empty';
    state.gogImportTitleConfidence = null;
  }
  return true;
}

export function setGogImportFiles(files) {
  state.gogImportSelectedFiles = [...files];
}

export function beginGogImportUpload(appJobId, totalBytes) {
  updateJob(appJobId, {
    state: 'running',
    progress: {
      ...initialGogImportProgress(),
      active: true,
      fileName: 'Uploading GOG setup files',
      total: Number(totalBytes || 0),
    },
  });
}

export function beginGogImportJob(appJobId, job, statusUrl, { reattached = false } = {}) {
  const progress = {
    ...initialGogImportProgress(),
    active: true,
    serverProcessing: true,
    jobId: String(job?.id || ''),
    statusUrl: String(statusUrl || ''),
    jobState: job?.state || null,
    phase: job?.phase || null,
    jobProgress: job?.progress ?? null,
    eventCursor: Number(job?.last_event_seq || 0),
    outputTruncated: Boolean(job?.output_truncated),
    reattached: Boolean(reattached),
  };
  return updateJob(appJobId, {
    state: 'running',
    serverJobId: progress.jobId,
    statusUrl: progress.statusUrl,
    progress,
  });
}

export function restoreGogImportJob(jobId, statusUrl) {
  const appJob = addJob({
    type: 'gog-import',
    title: 'GOG setup import',
    detail: 'Reattached server job',
    state: 'running',
  });
  beginGogImportJob(appJob.id, { id: jobId }, statusUrl, { reattached: true });
  return findJob(appJob.id);
}

export function applyGogImportJobSnapshot(appJobId, snapshot) {
  const appJob = findJob(appJobId);
  if (!appJob) return null;
  const previous = appJob.progress || initialGogImportProgress();
  const previousCursor = Number(previous.eventCursor || 0);
  const phaseEvents = [...(previous.phaseEvents || [])];
  const transcript = [...(previous.transcript || [])];
  const incomingSequences = new Set();

  const events = Array.isArray(snapshot?.events)
    ? [...snapshot.events].sort((left, right) => Number(left?.seq || 0) - Number(right?.seq || 0))
    : [];
  for (const event of events) {
    const seq = Number(event?.seq);
    if (!Number.isSafeInteger(seq) || seq <= previousCursor || incomingSequences.has(seq)) continue;
    incomingSequences.add(seq);
    const normalized = {
      seq,
      kind: event?.kind === 'phase' ? 'phase' : 'output',
      phase: String(event?.phase || snapshot?.phase || ''),
      stream: event?.stream === 'stderr' ? 'stderr' : event?.stream === 'stdout' ? 'stdout' : null,
      text: String(event?.text || ''),
    };
    if (normalized.kind === 'phase') phaseEvents.push(normalized);
    else transcript.push(normalized);
  }

  let browserTruncated = false;
  if (transcript.length > GOG_IMPORT_TRANSCRIPT_LIMIT) {
    transcript.splice(0, transcript.length - GOG_IMPORT_TRANSCRIPT_LIMIT);
    browserTruncated = true;
  }
  if (phaseEvents.length > GOG_IMPORT_PHASE_EVENT_LIMIT) {
    phaseEvents.splice(0, phaseEvents.length - GOG_IMPORT_PHASE_EVENT_LIMIT);
  }

  const nextCursor = Number(snapshot?.next_event_seq);
  const terminal = ['succeeded', 'failed', 'cancelled'].includes(snapshot?.state);
  const progress = {
    ...previous,
    active: !terminal,
    serverProcessing: true,
    jobState: snapshot?.state || previous.jobState,
    phase: snapshot?.phase || previous.phase,
    jobProgress: snapshot?.progress ?? null,
    eventCursor: Number.isSafeInteger(nextCursor)
      ? Math.max(previousCursor, nextCursor)
      : previousCursor,
    phaseEvents,
    transcript,
    outputTruncated: previous.outputTruncated
      || Boolean(snapshot?.output_truncated)
      || browserTruncated,
    pollingError: '',
    pollFailureCount: 0,
    result: snapshot?.result ?? null,
    jobError: snapshot?.error ?? null,
    reattached: false,
    unavailable: false,
    postProcessing: false,
  };
  return updateJob(appJobId, {
    state: snapshot?.state === 'failed'
      ? 'failed'
      : snapshot?.state === 'cancelled' ? 'cancelled' : 'running',
    progress,
    result: snapshot?.result ?? appJob.result,
    error: snapshot?.error?.message || '',
  });
}

export function beginLibraryScanJob(appJobId, job, statusUrl, { reattached = false } = {}) {
  const progress = {
    ...initialLibraryScanProgress(),
    jobState: job?.state || 'queued',
    phase: job?.phase || 'queued',
    jobProgress: job?.progress ?? null,
    reattached: Boolean(reattached),
  };
  return updateJob(appJobId, {
    state: progress.jobState,
    serverJobId: String(job?.id || ''),
    statusUrl: String(statusUrl || ''),
    progress,
  });
}

export function restoreLibraryScanJob(jobId, statusUrl) {
  const appJob = addJob({
    type: 'library-scan',
    title: 'Managed library scan',
    detail: 'Reattached server job',
    state: 'running',
  });
  beginLibraryScanJob(appJob.id, { id: jobId }, statusUrl, { reattached: true });
  return findJob(appJob.id);
}

export function applyLibraryScanJobSnapshot(appJobId, snapshot) {
  const job = findJob(appJobId);
  if (!job) return null;
  const terminal = ['succeeded', 'failed', 'cancelled'].includes(snapshot?.state);
  return updateJob(appJobId, {
    state: snapshot?.state || job.state,
    statusText: snapshot?.state === 'succeeded'
      ? 'Scan complete'
      : snapshot?.state === 'failed'
        ? 'Scan failed'
        : snapshot?.state === 'cancelled' ? 'Scan cancelled' : 'Scanning managed library',
    result: snapshot?.result ?? job.result,
    error: snapshot?.error?.message || '',
    progress: {
      ...job.progress,
      active: !terminal,
      jobState: snapshot?.state || job.progress?.jobState,
      phase: snapshot?.phase || job.progress?.phase,
      jobProgress: snapshot?.progress ?? null,
      pollingError: '',
      pollFailureCount: 0,
      unavailable: false,
      reattached: false,
    },
  });
}

export function setLibraryScanPollingFailure(appJobId, message, failureCount) {
  return updateJobProgress(appJobId, {
    pollingError: String(message || ''),
    pollFailureCount: Number(failureCount || 0),
  });
}

export function markLibraryScanJobUnavailable(appJobId, message) {
  const job = findJob(appJobId);
  if (!job) return null;
  return updateJob(appJobId, {
    state: 'unavailable',
    error: String(message || ''),
    progress: {
      ...job.progress,
      active: false,
      jobState: null,
      jobProgress: null,
      pollingError: String(message || ''),
      unavailable: true,
    },
  });
}

export function setGogImportPollingFailure(appJobId, message, failureCount) {
  return updateJobProgress(appJobId, {
    pollingError: String(message || ''),
    pollFailureCount: Number(failureCount || 0),
  });
}

export function markGogImportJobUnavailable(appJobId, message) {
  const job = findJob(appJobId);
  if (!job) return null;
  return updateJob(appJobId, {
    state: 'unavailable',
    error: String(message || ''),
    progress: {
      ...job.progress,
      active: false,
      serverProcessing: true,
      jobState: null,
      jobProgress: null,
      pollingError: String(message || ''),
      unavailable: true,
      postProcessing: false,
    },
  });
}

export function setGogImportPostProcessing(appJobId, active) {
  const job = findJob(appJobId);
  if (!job) return null;
  return updateJob(appJobId, {
    state: active ? 'running' : job.state,
    progress: {
      ...job.progress,
      active: Boolean(active),
      postProcessing: Boolean(active),
    },
  });
}

export function resetGogImportWorkflow() {
  state.gogImportTitle = '';
  state.gogImportTitleOrigin = 'empty';
  state.gogImportTitleConfidence = null;
  state.gogImportSelectedFiles = [];
}

export function setRommSourceStatus(status) {
  state.rommSource = status;
  // An unconfigured source has nothing to summarize, so its inputs are always shown.
  if (!status?.configured) state.rommSourceEditing = false;
}

export function setRommSourceEditing(editing) {
  state.rommSourceEditing = Boolean(editing);
}

export function setRommPlatforms(platforms) {
  state.rommPlatforms = Array.isArray(platforms) ? platforms : [];
}

export function setRommBrowse(patch) {
  state.rommBrowse = { ...state.rommBrowse, ...patch };
}

export function resetRommBrowse() {
  state.rommPlatforms = [];
  state.rommBrowse = initialRommBrowse();
  state.rommSelected = [];
  state.rommDrawer = null;
}

export function setRommHideExisting(hide) {
  state.rommHideExisting = Boolean(hide);
}

export function setRommSourceDetailOpen(open) {
  state.rommSourceDetailOpen = Boolean(open);
}

/// The browsed page is filtered in the browser only for the advisory duplicate marker; every other
/// filter is a server request, so this never hides a game the remote server did not return.
export function visibleRommItems() {
  if (!state.rommHideExisting) return state.rommBrowse.items;
  return state.rommBrowse.items.filter((rom) => !rom.already_present);
}

/// A game Teatro just imported is marked in the browsed pool so the list stops offering it without
/// spending another remote request. The advisory marker matches what a fresh browse would report.
export function markRommItemImported(remoteRomId, reason) {
  state.rommBrowse = {
    ...state.rommBrowse,
    items: state.rommBrowse.items.map((rom) => (rom.id === remoteRomId
      ? { ...rom, already_present: true, already_present_reason: String(reason || '') }
      : rom)),
  };
  state.rommSelected = state.rommSelected.filter(({ id }) => id !== remoteRomId);
}

export function findRommSelection(remoteRomId) {
  return state.rommSelected.find(({ id }) => id === remoteRomId) || null;
}

/// One selection entry carries only the fields the action bar and the import need, so a later
/// filter change cannot strand a queued game without its target.
function rommSelectionSnapshot(rom) {
  return {
    id: rom.id,
    name: String(rom.name || ''),
    platformName: String(rom.platform_name || rom.platform_slug || ''),
    fileCount: Number(rom.file_count || 0),
    sizeBytes: Number(rom.file_size_bytes || 0),
    targetPlatformId: rom.target_platform_id ? String(rom.target_platform_id) : '',
    targetPlatformName: String(rom.target_platform_name || ''),
    // Overrides stay null until an operator commits the drawer.
    titleOverride: null,
    platformOverride: null,
    fileIds: null,
  };
}

export function toggleRommSelection(rom) {
  if (!rom || !Number.isSafeInteger(rom.id) || rom.already_present) return false;
  if (findRommSelection(rom.id)) {
    state.rommSelected = state.rommSelected.filter(({ id }) => id !== rom.id);
    return true;
  }
  state.rommSelected = [...state.rommSelected, rommSelectionSnapshot(rom)];
  return true;
}

export function selectRommItems(roms) {
  const selected = new Set(state.rommSelected.map(({ id }) => id));
  const added = roms.filter((rom) => rom
    && Number.isSafeInteger(rom.id)
    && !rom.already_present
    && !selected.has(rom.id));
  state.rommSelected = [...state.rommSelected, ...added.map(rommSelectionSnapshot)];
  return added.length;
}

export function clearRommSelections() {
  state.rommSelected = [];
}

export function updateRommSelectionEntry(remoteRomId, patch) {
  state.rommSelected = state.rommSelected.map((entry) => (
    entry.id === remoteRomId ? { ...entry, ...patch } : entry
  ));
  return findRommSelection(remoteRomId);
}

export function rommSelectionPlatformId(entry) {
  return entry?.platformOverride || entry?.targetPlatformId || '';
}

export function rommSelectionTotals() {
  const platforms = new Set();
  let files = 0;
  let bytes = 0;
  let unmatched = 0;
  for (const entry of state.rommSelected) {
    const platformId = rommSelectionPlatformId(entry);
    if (platformId) platforms.add(platformId);
    else unmatched += 1;
    files += entry.fileIds ? entry.fileIds.length : entry.fileCount;
    bytes += entry.sizeBytes;
  }
  return { games: state.rommSelected.length, files, bytes, platforms: platforms.size, unmatched };
}

/// Opens the per-game drawer on a working copy. Nothing reaches the selection entry until commit,
/// so cancelling a drawer leaves the queued import exactly as it was.
export function openRommDrawer(remoteRomId) {
  const entry = findRommSelection(remoteRomId);
  state.rommDrawer = {
    romId: remoteRomId,
    loading: true,
    rom: null,
    files: [],
    totalSizeBytes: 0,
    title: entry?.titleOverride ?? '',
    platformId: entry ? rommSelectionPlatformId(entry) : '',
    fileIds: entry?.fileIds ? [...entry.fileIds] : null,
  };
  return state.rommDrawer;
}

export function loadRommDrawerDetail(detail) {
  if (!state.rommDrawer) return null;
  const files = Array.isArray(detail?.files) ? detail.files : [];
  const drawer = state.rommDrawer;
  state.rommDrawer = {
    ...drawer,
    loading: false,
    rom: detail?.rom || null,
    files,
    totalSizeBytes: Number(detail?.total_size_bytes || 0),
    title: drawer.title || String(detail?.rom?.name || ''),
    platformId: drawer.platformId
      || (detail?.rom?.target_platform_id ? String(detail.rom.target_platform_id) : ''),
    // No stored choice means every file is imported, which is what an untouched card already does.
    fileIds: drawer.fileIds
      ? drawer.fileIds.filter((id) => files.some((file) => file.id === id))
      : files.map(({ id }) => id),
  };
  return state.rommDrawer;
}

export function updateRommDrawer(patch) {
  if (!state.rommDrawer) return null;
  state.rommDrawer = { ...state.rommDrawer, ...patch };
  return state.rommDrawer;
}

export function toggleRommDrawerFile(fileId, checked) {
  const drawer = state.rommDrawer;
  if (!drawer) return null;
  const selected = new Set(drawer.fileIds || []);
  if (checked) selected.add(fileId);
  else selected.delete(fileId);
  return updateRommDrawer({
    fileIds: drawer.files.map(({ id }) => id).filter((id) => selected.has(id)),
  });
}

export function closeRommDrawer() {
  state.rommDrawer = null;
}

export function beginRommImportJob(appJobId, job, statusUrl, { reattached = false } = {}) {
  const progress = {
    ...initialRommImportProgress(),
    jobState: job?.state || 'queued',
    phase: job?.phase || 'queued',
    jobProgress: job?.progress ?? null,
    reattached: Boolean(reattached),
  };
  return updateJob(appJobId, {
    state: progress.jobState,
    serverJobId: String(job?.id || ''),
    statusUrl: String(statusUrl || ''),
    progress,
  });
}

export function restoreRommImportJob(jobId, statusUrl) {
  const appJob = addJob({
    type: 'romm-import',
    title: 'RomM import',
    detail: 'Reattached server job',
    state: 'running',
  });
  beginRommImportJob(appJob.id, { id: jobId }, statusUrl, { reattached: true });
  return findJob(appJob.id);
}

export function applyRommImportJobSnapshot(appJobId, snapshot) {
  const job = findJob(appJobId);
  if (!job) return null;
  const terminal = ['succeeded', 'failed', 'cancelled'].includes(snapshot?.state);
  const skipped = snapshot?.result?.outcome === 'already_present';
  return updateJob(appJobId, {
    state: snapshot?.state || job.state,
    statusText: snapshot?.state === 'succeeded'
      ? (skipped ? 'Already in Teatro' : 'Import complete')
      : snapshot?.state === 'failed'
        ? 'Import failed'
        : snapshot?.state === 'cancelled' ? 'Import cancelled' : 'Importing from RomM',
    result: snapshot?.result ?? job.result,
    error: snapshot?.error?.message || '',
    progress: {
      ...job.progress,
      active: !terminal,
      jobState: snapshot?.state || job.progress?.jobState,
      phase: snapshot?.phase || job.progress?.phase,
      jobProgress: snapshot?.progress ?? null,
      pollingError: '',
      pollFailureCount: 0,
      unavailable: false,
      reattached: false,
    },
  });
}

export function setRommImportPollingFailure(appJobId, message, failureCount) {
  return updateJobProgress(appJobId, {
    pollingError: String(message || ''),
    pollFailureCount: Number(failureCount || 0),
  });
}

export function markRommImportJobUnavailable(appJobId, message) {
  const job = findJob(appJobId);
  if (!job) return null;
  return updateJob(appJobId, {
    state: 'unavailable',
    error: String(message || ''),
    progress: {
      ...job.progress,
      active: false,
      jobState: null,
      jobProgress: null,
      pollingError: String(message || ''),
      unavailable: true,
    },
  });
}

export function resetSession() {
  invalidateRomSelection();
  state.auth = null;
  state.user = null;
  state.screen = 'dashboard';
  clearFeedback();
  clearRomSelection();
  resetUploadWorkflow();
  resetGogImportWorkflow();
  state.sidecarCleanupPreview = null;
  state.sidecarCleanupLoading = false;
  state.rommSource = null;
  state.rommSourceEditing = false;
  state.rommSourceDetailOpen = false;
  state.rommHideExisting = true;
  resetRommBrowse();
  state.jobs = [];
  persistJobs();
}

export function invalidateRomSelection() {
  state.romSelectionRequestId += 1;
}

export function beginRomSelection() {
  invalidateRomSelection();
  return state.romSelectionRequestId;
}

export function isCurrentRomSelection(requestId) {
  return requestId === state.romSelectionRequestId;
}
