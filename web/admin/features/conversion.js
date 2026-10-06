import { api } from '../api.js';
import { addJob, findJob, isJobActive, setScreen, state, updateJob } from '../state.js';
import { clearServerJobs, createServerJobPoller, forgetServerJob, loadServerJobs, rememberServerJob } from './server-jobs.js';
import { createUploadTransfer } from './upload-transfer.js';

const TYPE = 'conversion-import';
const terminal = (value) => ['succeeded', 'failed', 'cancelled', 'unavailable'].includes(value);
const urlFor = (id) => `/api/admin/conversion-imports/${encodeURIComponent(id)}`;

export class ConversionController {
  constructor(context) {
    this.context = context;
    this.uploads = new Map();
    this.pendingUploads = new Map();
    this.activeConversionId = null;
    this.selectionRevision = 0;
    this.poller = createServerJobPoller({
      getJob: findJob,
      requestUrl: (job) => urlFor(job.serverJobId),
      shouldPoll: (id) => Boolean(state.auth?.header && findJob(id)?.type === TYPE && findJob(id)?.serverJobId && !terminal(findJob(id)?.state)),
      onSnapshot: (id, snapshot, serverId) => this.onSnapshot(id, snapshot, serverId),
      onError: (id, error, failures) => {
        if ([401, 403, 404].includes(error?.status)) {
          updateJob(id, { state: 'unavailable', error: 'This import is unavailable or the server restarted. Upload the files again.' });
          forgetServerJob(TYPE, findJob(id)?.serverJobId);
          this.finishConversion(id);
          this.context.jobChanged?.(id);
          return false;
        }
        updateJob(id, { progress: { ...findJob(id)?.progress, pollingError: error?.message || 'Could not read progress', pollFailureCount: failures } });
        this.context.jobChanged?.(id);
        return true;
      },
    });
    if (!state.auth?.header) clearServerJobs(TYPE);
    else for (const record of loadServerJobs(TYPE)) {
      const job = state.jobs.find((job) => job.serverJobId === record.id) || addJob({ type:TYPE, title:'Conversion import', state:'running', serverJobId:record.id, statusUrl:record.statusUrl });
      this.poller.watch(job.id, { immediate:true });
    }
  }
  bind() {
    const app = this.context.app;
    app.querySelector('[data-add-conversion-folder]')?.addEventListener('click', () => app.querySelector('#conversion-folder')?.click());
    app.querySelector('#conversion-folder')?.addEventListener('change', (event) => {
      const files = Array.from(event.target.files || [], (file) => ({ file, path: file.webkitRelativePath || file.name }));
      if (files.length) void this.selectFolders(Promise.resolve(files));
    });
    app.querySelector('#conversion-title')?.addEventListener('input', (event) => {
      state.conversionTitle = event.target.value; state.conversionTitleOrigin = 'manual';
    });
    app.querySelector('[data-clear-conversion]')?.addEventListener('click', () => {
      this.clearSelection();
      this.context.render();
    });
    app.querySelectorAll('[data-remove-conversion-folder]').forEach((button) => button.addEventListener('click', () => {
      const files = state.conversionFiles.filter((entry) => selectionRoot(entry.path) !== button.dataset.removeConversionFolder);
      void this.selectFolders(Promise.resolve(files), { replace: true });
    }));
    app.querySelector('#conversion-form')?.addEventListener('submit', (event) => { event.preventDefault(); void this.launch(); });
  }
  clearSelection() {
    this.selectionRevision++;
    state.conversionFiles = []; state.conversionInspection = null; state.conversionInspecting = false;
    state.conversionTitle = ''; state.conversionTitleOrigin = 'empty';
  }
  async selectFolders(readFiles, { replace = false, onFiles = null } = {}) {
    const revision = ++this.selectionRevision;
    state.conversionInspecting = true;
    this.context.render();
    try {
      const incoming = await readFiles;
      if (revision !== this.selectionRevision) return;
      if (onFiles && incoming.every((entry) => !entry.path.includes('/'))) {
        state.conversionInspecting = false;
        onFiles(incoming.map((entry) => entry.file));
        return;
      }
      if (onFiles && incoming.some((entry) => !entry.path.includes('/'))) throw new Error('Import game files and decrypted folders as separate jobs.');
      if (state.uploadSelectedFiles.length) throw new Error('Clear the game files before adding decrypted folders. Import files and folders as separate jobs.');
      const files = replace ? incoming : [...state.conversionFiles, ...incoming];
      if (files.length > (state.conversionStatus?.max_files || 20_000)) throw new Error('Too many files. Select one game with at most one matching update and DLC.');
      state.conversionFiles = files;
      const inspection = files.length ? await inspectConversionFolders(files) : null;
      if (revision !== this.selectionRevision) return;
      state.conversionInspection = inspection;
      if (state.conversionTitleOrigin !== 'manual') {
        const base = inspection?.titles.find((title) => title.kind === 'base');
        state.conversionTitle = base?.gameTitle || base?.root.split('/').pop() || '';
        state.conversionTitleOrigin = state.conversionTitle ? 'inferred' : 'empty';
      }
    } catch (error) {
      if (revision === this.selectionRevision) this.context.setError(error);
    } finally {
      if (revision === this.selectionRevision) { state.conversionInspecting = false; this.context.render(); }
    }
  }
  async launch() {
    const inspection = state.conversionInspection;
    const platform = state.platforms.find((item) => String(item.id) === state.uploadPlatformId);
    if (platform?.slug !== 'wiiu' || !state.conversionTitle.trim() || !state.conversionFiles.length || state.conversionInspecting || !inspection || inspection.errors.length || inspection.titles.some((title) => title.error)) return;
    const files = state.conversionFiles;
    const title = state.conversionTitle;
    state.conversionFiles = []; state.conversionInspection = null;
    state.conversionTitle = ''; state.conversionTitleOrigin = 'empty';
    await this.launchFiles({ files, title, platform: 'wiiu' });
  }
  async launchFiles({ files, title, platform }) {
    const format = state.conversionStatus?.formats?.find((item) => item.platform_slug === platform)?.output
      || (platform === 'wiiu' ? 'wua' : ['gc', 'wii'].includes(platform) ? 'rvz' : 'chd');
    const platformName = state.platforms.find((item) => item.slug === platform)?.display_name || platform;
    const job = addJob({ type:TYPE, title, outputFormat:format, detail:platform === 'wiiu' ? 'Wii U folders → WUA' : `${platformName} ${format === '7z' ? 'ROM' : `disc${files.length > 1 ? ' set' : ''}`} → ${format.toUpperCase()}`,  state:'queued', statusText:'Waiting to upload', progress:{phase:'queued_upload'} });
    this.pendingUploads.set(job.id, { files, title, platform });
    setScreen('jobs'); this.context.render();
    return this.startNextUpload();
  }
  async startNextUpload() {
    if (this.activeConversionId || !this.pendingUploads.size) return;
    const [id, selection] = this.pendingUploads.entries().next().value;
    this.pendingUploads.delete(id);
    this.activeConversionId = id;
    await this.uploadFiles(findJob(id), selection);
  }
  finishConversion(id) {
    if (this.activeConversionId !== id) return;
    this.activeConversionId = null;
    void this.startNextUpload();
  }
  async uploadFiles(job, { files, title, platform }) {
    const body = new FormData();
    body.append('title', title.trim());
    body.append('platform_slug', platform);
    for (const { file, path } of files) body.append('files', file, path);
    const totalBytes = files.reduce((total, entry) => total + entry.file.size, 0);
    updateJob(job.id, { state:'running', statusText:'Uploading inputs', progress:{phase:'uploading', uploadProgress:{current:0, total:totalBytes, percent:0, speed:0}} });
    const abort = new AbortController();
    this.uploads.set(job.id, abort);
    this.context.render();
    try {
      const transfer = createUploadTransfer({
        updateProgress: (progress) => {
          const current = findJob(job.id);
          if (!isJobActive(current) || current.progress?.phase !== 'uploading') return;
          updateJob(job.id, { progress: { ...current.progress, uploadProgress: {
            current:progress.loaded, total:progress.total, percent:progress.percent, speed:progress.speed || 0,
          } } });
          this.context.jobChanged?.(job.id);
        },
      });
      const created = await transfer.uploadWithProgress(body, {
        url:'/api/admin/conversion-imports', expectedStatus:202, signal:abort.signal,
        fileSize:totalBytes, totalBytes,
        onUploadComplete: () => {
          if (!isJobActive(findJob(job.id))) return;
          updateJob(job.id, { statusText:'Inspecting title files', progress:{phase:'inspecting'} });
          this.context.jobChanged?.(job.id);
        },
      });
      if (!/^conv_[A-Za-z0-9_-]+$/u.test(created?.id || '') || created.status_url !== urlFor(created.id)) throw new Error('Invalid conversion job response');
      updateJob(job.id, { serverJobId:created.id, statusUrl:created.status_url, statusText:'Inspecting title files', progress:{phase:'inspecting'} });
      rememberServerJob({ type:TYPE, id:created.id, statusUrl:created.status_url });
      this.poller.watch(job.id, { immediate:true });
    } catch (error) {
      updateJob(job.id, { state:abort.signal.aborted ? 'cancelled' : 'failed', statusText:abort.signal.aborted ? 'Upload cancelled' : 'Upload failed', error:abort.signal.aborted ? '' : error.message });
    } finally {
      this.uploads.delete(job.id);
      if (terminal(findJob(job.id)?.state)) this.finishConversion(job.id);
      this.context.render();
    }
  }
  async onSnapshot(id, snapshot, serverId) {
    if (snapshot?.id !== serverId || !['queued','running','succeeded','failed','cancelled'].includes(snapshot.state)) throw new Error('Invalid conversion job snapshot');
    const previous = findJob(id);
    const changed = previous?.state !== snapshot.state || previous?.progress?.phase !== snapshot.phase || previous?.progress?.pollingError || JSON.stringify(previous?.progress?.jobProgress) !== JSON.stringify(snapshot.progress);
    updateJob(id, { state:snapshot.state, outputFormat:snapshot.output_format || snapshot.result?.output_format || previous?.outputFormat || snapshot.result?.file_name?.split('.').pop(), result:snapshot.result, error:snapshot.error?.message || '', progress:{phase:snapshot.phase, jobProgress:snapshot.progress, pollFailureCount:0}, statusText:snapshot.phase.replaceAll('_',' ') });
    if (changed) this.context.jobChanged?.(id);
    if (!terminal(snapshot.state)) return false;
    forgetServerJob(TYPE, serverId);
    this.finishConversion(id);
    if (snapshot.state === 'succeeded') await Promise.all([this.context.loadStats?.(), this.context.loadRoms?.()]);
    return true;
  }
  async cancelJob(id) {
    const job = findJob(id);
    if (!job || !isJobActive(job)) return;
    this.pendingUploads.delete(id);
    if (job.serverJobId) await api(urlFor(job.serverJobId), { method:'DELETE' });
    else this.uploads.get(id)?.abort();
    this.poller.stop(id, {remove:true});
    forgetServerJob(TYPE, job.serverJobId);
    updateJob(id, { state:'cancelled', proposal:null, statusText:'Cancelled; the server will join its writer before removing private inputs' });
    this.finishConversion(id);
    this.context.render();
  }
  stopAll() {
    this.selectionRevision++; this.poller.stopAll();
    for (const id of this.pendingUploads.keys()) updateJob(id, {state:'cancelled', statusText:'Queued upload cancelled'});
    this.pendingUploads.clear(); this.activeConversionId = null;
    for (const abort of this.uploads.values()) abort.abort();
    clearServerJobs(TYPE);
  }
}

// Capture entries before the drop event returns; DataTransfer is protected afterwards.
export async function readDroppedFiles(transfer, maxFiles = 20_000) {
  const entries = Array.from(transfer?.items || [], (item) => item.webkitGetAsEntry?.()).filter(Boolean);
  if (!entries.length) {
    const files = Array.from(transfer?.files || [], (file) => ({ file, path: file.webkitRelativePath || file.name }));
    if (!files.length) throw new Error('This browser could not read the dropped files. Use the file or folder picker instead.');
    if (files.length > maxFiles) throw new Error(`Too many files. The limit is ${maxFiles}.`);
    return files;
  }
  const files = [];
  async function walk(entry, prefix = '', depth = 0) {
    if (depth > 32) throw new Error('Folder nesting exceeds the 32-level limit.');
    const path = prefix + entry.name;
    if (entry.isFile) {
      if (files.length >= maxFiles) throw new Error(`Too many files. The limit is ${maxFiles}.`);
      const file = await new Promise((resolve, reject) => entry.file(resolve, reject));
      files.push({ file, path });
    } else if (entry.isDirectory) {
      const reader = entry.createReader();
      // Chromium returns directory entries in batches, not all in the first read.
      while (true) {
        const children = await new Promise((resolve, reject) => reader.readEntries(resolve, reject));
        if (!children.length) break;
        for (const child of children) await walk(child, `${path}/`, depth + 1);
      }
    }
  }
  for (const entry of entries) await walk(entry);
  if (!files.length) throw new Error('The dropped folders contain no files. Choose a decrypted code/content/meta title folder.');
  return files;
}

function selectionRoot(path) {
  const parts = path.split('/');
  const at = parts.findIndex((part) => ['code', 'content', 'meta'].includes(part));
  return parts.slice(0, at < 0 ? -1 : at).join('/');
}

export function conversionTitleHeader(xml) {
  const document = new DOMParser().parseFromString(xml, 'application/xml');
  if (document.querySelector('parsererror')) throw new Error('Title XML is invalid.');
  const id = document.querySelector('title_id')?.textContent.trim().toLowerCase();
  const versionNode = document.querySelector('title_version');
  const encoding = versionNode?.getAttribute('type');
  const raw = versionNode?.textContent.trim();
  const hex = encoding === 'hexBinary';
  if (!/^[0-9a-f]{16}$/u.test(id || '') || !raw || ![null, 'unsignedInt', 'hexBinary'].includes(encoding)
    || !(hex ? /^[0-9a-f]+$/iu : /^\d+$/u).test(raw)) throw new Error('Title ID or version is missing or invalid.');
  const version = Number.parseInt(raw, hex ? 16 : 10);
  if (!Number.isSafeInteger(version) || version > 0xffffffff) throw new Error('Title version is invalid.');
  const titleNodes = [document.querySelector('longname_en'), ...document.documentElement.children];
  const gameTitle = titleNodes.filter((node) => node?.tagName.startsWith('longname_'))
    .map((node) => node.textContent.trim().replace(/\s+/gu, ' '))
    .find((name) => name && new TextEncoder().encode(name).length <= 512 && !/[\u0000-\u001f\u007f-\u009f]/u.test(name)) || '';
  return { id, version, gameTitle };
}

export async function inspectConversionFolders(files) {
  const groups = new Map();
  const paths = new Map();
  const errors = new Set();
  for (const entry of files) {
    const root = selectionRoot(entry.path);
    if (!groups.has(root)) groups.set(root, new Map());
    groups.get(root).set(entry.path.slice(root ? root.length + 1 : 0), entry.file);
    const key = entry.path.toLowerCase();
    if (paths.has(key)) errors.add('Duplicate or case-colliding paths. Remove the affected folder and rename it before adding it again.');
    paths.set(key, entry.path);
    if (!/^[\x20-\x7e]+$/u.test(entry.path) || entry.path.length > 240) errors.add('Use ASCII relative paths of at most 240 characters. Rename the affected folders or files.');
  }
  const titles = [];
  for (const [root, entries] of groups) {
    const title = { root, kind: null, titleId: null, version: null, error: '' };
    titles.push(title);
    try {
      const readHeader = async (path) => {
        const file = entries.get(path);
        if (!file) throw new Error(`Missing ${path}. Select the complete title folder.`);
        if (file.size > 1024 * 1024) throw new Error(`${path} exceeds 1 MiB.`);
        return conversionTitleHeader(await file.text());
      };
      const header = await readHeader('code/app.xml');
      title.titleId = header.id; title.version = header.version;
      title.kind = { '00050000': 'base', '0005000e': 'update', '0005000c': 'dlc' }[header.id.slice(0, 8)] || null;
      if (!title.kind) throw new Error('Unsupported Wii U title type. Select a game, update or DLC folder.');
      const meta = await readHeader('meta/meta.xml');
      if (title.kind === 'base') title.gameTitle = meta.gameTitle;
      if ((meta.id !== header.id && !(title.kind === 'update' && meta.id === `00050000${header.id.slice(8)}`)) || meta.version !== header.version) throw new Error('code/app.xml and meta/meta.xml disagree about the title ID or version.');
      if (!entries.has('code/cos.xml') || ![...entries.keys()].some((path) => path.startsWith('content/'))) throw new Error('Missing code/cos.xml or content files. Select the complete code/content/meta folder.');
      if ([...entries.keys()].some((path) => !/^(code|content|meta)\/.+/u.test(path))) throw new Error('Files outside code/content/meta are not accepted. Remove them from the selected folder.');
    } catch (error) { title.error = error.message || 'Could not read title metadata. Choose the folder again.'; }
  }
  if (!titles.some((title) => title.kind === 'base')) errors.add('Add the base game folder before uploading. Updates and DLC cannot be imported on their own.');
  const families = new Set(titles.filter((title) => title.kind).map((title) => title.titleId.slice(8)));
  if (families.size > 1) errors.add('These folders belong to different games. Select one matching game, update and DLC family.');
  const kinds = titles.map((title) => title.kind).filter(Boolean);
  if (titles.length > 3 || new Set(kinds).size !== kinds.length) errors.add('Select only one game, one update and one DLC folder.');
  return { titles, errors: [...errors] };
}
