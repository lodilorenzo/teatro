import { api } from '../api.js';
import { isTextManifestFile, slugifyTitle } from '../dom.js';
import {
  addJob, findJob, resetUploadWorkflow, setUploadFiles, setUploadPlatformId,
  setUploadPreview, state, updateJob, updateJobProgress,
} from '../state.js';
import {
  renderUploadFileList, renderUploadPlanPanel, uploadSelectionLabel,
} from '../views/upload-renderers.js';
import { createUploadTransfer } from './upload-transfer.js';
import {
  listBackgroundTransfers, removeBackgroundTransfer, waitForBackgroundTransfer,
} from './background-transfer.js';

export class UploadController {
  constructor(context) {
    this.context = Object.freeze({ ...context });
    void this.restoreBackgroundJobs();
  }

  async restoreBackgroundJobs() {
    const existingActive = new Set(state.jobs
      .filter((job) => job.type === 'upload' && ['queued', 'running'].includes(job.state))
      .map(({ id }) => id));
    try {
      const records = await listBackgroundTransfers('upload');
      const retained = new Set(records.map(({ id }) => id));
      for (const record of records) {
        let job = findJob(record.id);
        if (!job) {
          job = addJob({
            id: record.id,
            type: 'upload',
            title: record.title,
            detail: record.detail,
            state: 'running',
            statusText: 'Uploading grouped batch',
            createdAt: record.createdAt,
            progress: {
              active: true,
              fileName: 'Uploading grouped batch',
              loaded: Number(record.progress?.loaded || 0),
              total: record.totalBytes,
              percent: Number(record.progress?.percent || 0),
              speed: Number(record.progress?.speed || 0),
            },
          });
        }
        if (job.state === 'queued' || job.state === 'running') {
          const platform = record.context?.platform;
          void this.runUploadJob(job.id, null, platform, record.totalBytes, { resume: true });
        }
      }
      for (const job of state.jobs) {
        if (job.type === 'upload'
          && existingActive.has(job.id)
          && ['queued', 'running'].includes(job.state)
          && !retained.has(job.id)) {
          updateJob(job.id, {
            state: 'failed',
            statusText: 'Upload interrupted',
            error: 'The background upload is no longer available.',
            progress: { ...job.progress, active: false },
          });
        }
      }
      this.context.render();
    } catch (error) {
      for (const job of state.jobs) {
        if (job.type === 'upload'
          && existingActive.has(job.id)
          && ['queued', 'running'].includes(job.state)) {
          updateJob(job.id, {
            state: 'failed',
            statusText: 'Upload interrupted',
            error: error?.message || String(error),
            progress: { ...job.progress, active: false },
          });
        }
      }
      this.context.render();
    }
  }

  createTransfer(jobId) {
    if (this.context.uploadTransfer) return this.context.uploadTransfer;
    return createUploadTransfer({
      loadIgdbStatus: this.context.loadIgdbStatus,
      updateProgress: (progress) => this.updateProgress(jobId, progress),
    });
  }

  bindDropZone() {
    const dropZone = this.context.app.querySelector('#upload-drop-zone');
    const fileInput = this.context.app.querySelector('#upload-file-input');
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
    this.context.app.querySelectorAll('[data-remove-upload-file]').forEach((button) => {
      button.addEventListener('click', () => this.removeSelectedFile(Number(button.dataset.removeUploadFile)));
    });
  }

  bindPlanControls() {
    this.context.app.querySelectorAll('[data-planned-title]').forEach((input) => {
      input.addEventListener('input', (event) => this.onPlannedTitleInput(event));
    });
  }

  onPlannedTitleInput(event) {
    const input = event.currentTarget;
    const rom = applyPlannedTitle(state.uploadPlan, input.dataset.plannedTitle, input.value);
    if (!rom) return;
    this.context.app.querySelectorAll('[data-planned-slug]').forEach((element) => {
      if (element.dataset.plannedSlug === rom.plan_id) element.textContent = rom.slug;
    });
    this.context.app.querySelectorAll('[data-planned-group-title]').forEach((element) => {
      if (element.dataset.plannedGroupTitle === rom.plan_id) element.textContent = rom.title;
    });
    this.syncFinalizeButton();
  }

  syncFinalizeButton() {
    const button = this.context.app.querySelector('[data-action="finalize-upload-plan"]');
    if (!button) return;
    button.disabled = !state.uploadPlan
      || Boolean(state.uploadPlan.errors?.length)
      || !plannedTitlesAreValid();
  }

  appendSelectedFiles(files) {
    if (!files.length) return;
    this.clearFileInput();
    this.updateSelectedFiles([...state.uploadSelectedFiles, ...files]);
  }

  removeSelectedFile(index) {
    if (!Number.isInteger(index)) return;
    this.clearFileInput();
    this.updateSelectedFiles(state.uploadSelectedFiles.filter((_, current) => current !== index));
  }

  clearSelectedFiles = () => {
    this.clearFileInput();
    this.updateSelectedFiles([]);
  };

  clearFileInput() {
    const input = this.context.app.querySelector('#upload-file-input');
    if (input) input.value = '';
  }

  clearPlan() {
    if (!state.uploadPlan && !state.uploadPreviewLoading) return;
    setUploadPreview(false, null);
    const panel = this.context.app.querySelector('#upload-plan-panel');
    if (panel) panel.innerHTML = renderUploadPlanPanel();
    this.syncFinalizeButton();
  }

  onPlatformChange = (event) => {
    setUploadPlatformId(event.currentTarget.value);
    this.clearPlan();
    this.context.render();
    this.context.app.querySelector('.platform-picker summary')?.focus();
  };

  updateSelectedFiles(files) {
    setUploadFiles(files);
    const app = this.context.app;
    const label = app.querySelector('#upload-file-name');
    const list = app.querySelector('#upload-file-list');
    const clearButton = app.querySelector('[data-action="clear-upload-list"]');
    if (label) label.textContent = uploadSelectionLabel(files);
    if (list) {
      list.innerHTML = renderUploadFileList(files);
      this.bindQueueControls();
    }
    if (clearButton) clearButton.disabled = !files.length;
    const panel = app.querySelector('#upload-plan-panel');
    if (panel) panel.innerHTML = renderUploadPlanPanel();
    this.syncFinalizeButton();
  }

  onPreviewPlan = async (event) => {
    event?.preventDefault?.();
    const form = this.context.app.querySelector('#upload-form');
    if (!form) return;
    try {
      setUploadPreview(true, null);
      this.context.render();
      const plan = await this.buildAndFetchPlan(form);
      const existingWarnings = (plan.warnings || [])
        .filter((warning) => warning.code === 'file_already_exists' && warning.file_name);
      const existingFileNames = [...new Set(existingWarnings.map((warning) => warning.file_name))];
      if (existingFileNames.length) {
        const existing = new Set(existingFileNames);
        setUploadFiles(state.uploadSelectedFiles.filter((file) => !existing.has(file.name)));
      }
      setUploadPreview(false, plan);
      const otherWarningCount = (plan.warnings?.length || 0) - existingWarnings.length;
      if (existingFileNames.length) {
        const message = existingFileNames.length === 1
          ? `“${existingFileNames[0]}” already exists for this platform and was removed from the upload list.`
          : `${existingFileNames.length} files already exist for this platform and were removed from the upload list.`;
        this.context.setNotice(otherWarningCount ? `${message} Review ${otherWarningCount} other warning(s).` : message);
      }
      if (plan.errors?.length) {
        this.context.setError(new Error(`Upload plan has ${plan.errors.length} blocking error(s). Review the plan before launching.`));
      } else if (!existingFileNames.length && plan.warnings?.length) {
        this.context.setNotice(`Upload plan ready with ${plan.warnings.length} warning(s).`);
      } else if (!existingFileNames.length) {
        this.context.setNotice('Upload plan ready. Review grouping, then launch the job when ready.');
      }
    } catch (error) {
      setUploadPreview(false, null);
      this.context.setError(error);
    }
    this.context.render();
  };

  async buildAndFetchPlan(form) {
    const { platform } = this.formContext(form);
    const files = state.uploadSelectedFiles;
    if (!platform) throw new Error('Select a platform before previewing.');
    if (!files.length) throw new Error('Select or drop one or more game files before previewing.');

    const requestFiles = [];
    for (const file of files) {
      const requestFile = { file_name: file.name, file_size_bytes: file.size };
      if (isTextManifestFile(file.name) && file.size <= 1024 * 1024) {
        requestFile.manifest_contents = await file.text();
      }
      requestFiles.push(requestFile);
    }
    return api('/api/admin/uploads/preview', {
      method: 'POST',
      body: JSON.stringify({ platform_id: platform.id, files: requestFiles }),
    });
  }

  formContext(form) {
    const platformId = String(form.elements.platform_id?.value || state.uploadPlatformId || '');
    setUploadPlatformId(platformId);
    return { platform: state.platforms.find((item) => String(item.id) === platformId) };
  }

  updateProgress(jobId, progress) {
    if (!findJob(jobId)) return;
    updateJobProgress(jobId, { ...progress, active: true });
    this.context.jobChanged?.(jobId);
  }

  onSubmit = async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const files = [...state.uploadSelectedFiles];
    const { platform } = this.formContext(form);
    if (!platform || !files.length) {
      this.context.setError(new Error(platform
        ? 'Select or drop one or more game files before uploading.'
        : 'Select a platform before uploading.'));
      this.context.render();
      return;
    }
    if (!state.uploadPlan) {
      await this.onPreviewPlan(event);
      return;
    }
    if (state.uploadPlan.errors?.length || !plannedTitlesAreValid()) {
      this.context.setError(new Error(state.uploadPlan.errors?.length
        ? 'Resolve upload plan errors before launching.'
        : 'Every planned game must have a title before launching.'));
      this.context.render();
      return;
    }

    const plan = state.uploadPlan;
    const totalBytes = files.reduce((total, file) => total + Number(file.size || 0), 0);
    const batchForm = this.buildBatchForm(form, platform, files, plan);
    const romCount = plan.roms.length;
    const title = romCount === 1 ? plan.roms[0].title : `${romCount} planned games`;
    const job = addJob({
      type: 'upload',
      title,
      detail: `${platform.display_name} · ${files.length} file${files.length === 1 ? '' : 's'}`,
      state: 'running',
      statusText: 'Uploading grouped batch',
      progress: {
        active: true,
        fileName: 'Uploading grouped batch',
        fileIndex: 0,
        fileCount: files.length,
        percent: 0,
        loaded: 0,
        total: totalBytes,
        speed: 0,
      },
    });

    this.clearFileInput();
    resetUploadWorkflow();
    this.context.setNotice(`Upload job “${title}” launched. Track it in Jobs.`);
    this.context.render();
    void this.runUploadJob(job.id, batchForm, platform, totalBytes);
  };

  async runUploadJob(jobId, batchForm, platform, totalBytes, { resume = false } = {}) {
    const metadataMatches = [];
    const metadataFailures = [];
    const transfer = this.createTransfer(jobId);
    try {
      const job = findJob(jobId);
      const batch = resume
        ? await waitForBackgroundTransfer(jobId, (progress) => {
          if (progress) this.updateProgress(jobId, {
            ...progress,
            active: true,
            fileName: 'Uploading grouped batch',
          });
        })
        : await transfer.uploadWithProgress(batchForm, {
          durable: true,
          jobId,
          jobType: 'upload',
          jobTitle: job?.title,
          jobDetail: job?.detail,
          context: { platform, totalBytes },
          url: '/api/admin/upload-batches',
          fileName: 'Uploading grouped batch',
          fileSize: totalBytes,
          totalBytes,
          batchStartedAt: performance.now(),
        });
      if (!findJob(jobId)) return;
      const uploadedRoms = batch?.roms || [];
      updateJob(jobId, { statusText: 'Applying optional IGDB metadata' });
      this.context.jobChanged?.(jobId);
      await this.applyMetadataToBatch(jobId, transfer, uploadedRoms, totalBytes, metadataMatches, metadataFailures);
      if (!findJob(jobId)) return;

      let refreshError = null;
      updateJob(jobId, { statusText: 'Refreshing library data' });
      this.context.jobChanged?.(jobId);
      try {
        await Promise.all([
          this.context.loadPlatforms(), this.context.loadStats(), this.context.loadRoms(),
        ]);
      } catch (error) {
        refreshError = error;
      }
      if (!findJob(jobId)) return;

      const message = this.uploadSuccessMessage(
        batch, platform, uploadedRoms, metadataMatches, metadataFailures,
      );
      updateJob(jobId, {
        state: 'succeeded',
        statusText: refreshError ? 'Upload complete; library refresh failed' : 'Upload complete',
        progress: {
          ...findJob(jobId)?.progress,
          active: false,
          loaded: totalBytes,
          total: totalBytes,
          percent: 100,
          speed: 0,
        },
        result: { batch, roms: uploadedRoms },
        error: refreshError ? `Library refresh failed: ${refreshError.message}` : '',
      });
      if (refreshError) {
        this.context.setError(new Error(`${message} Refreshing the library failed: ${refreshError.message}`));
      } else {
        this.context.setNotice(message);
      }
    } catch (error) {
      if (findJob(jobId) && findJob(jobId)?.state !== 'cancelled') {
        updateJob(jobId, {
          state: 'failed',
          statusText: 'Upload failed',
          error: error?.message || String(error),
          progress: { ...findJob(jobId)?.progress, active: false },
        });
        this.context.setError(error);
      }
    }
    await removeBackgroundTransfer(jobId);
    this.context.render();
  }

  cancelJob = async (jobId) => {
    const job = findJob(jobId);
    if (!job || job.type !== 'upload' || !['queued', 'running'].includes(job.state)) return;
    try {
      await api(`/api/admin/background-transfers/${encodeURIComponent(jobId)}?operation=upload`, { method: 'DELETE' });
    } catch (error) {
      if (error?.status !== 404) throw error;
    }
    await removeBackgroundTransfer(jobId);
    updateJob(jobId, {
      state: 'cancelled',
      statusText: 'Upload cancelled',
      progress: { ...job.progress, active: false, speed: 0 },
    });
    this.context.setNotice('Upload cancelled.');
    this.context.render();
  };

  buildBatchForm(form, platform, files, plan = state.uploadPlan) {
    const batchForm = new FormData(form);
    for (const name of ['platform_id', 'title', 'name', 'planned_title', 'file', 'files']) batchForm.delete(name);
    batchForm.set('platform_slug', platform.slug);
    for (const rom of plan.roms) {
      batchForm.append('planned_title', JSON.stringify({ plan_id: rom.plan_id, title: rom.title }));
    }
    for (const file of files) batchForm.append('file', file, file.name);
    return batchForm;
  }

  async applyMetadataToBatch(jobId, transfer, roms, totalBytes, matches, failures) {
    for (const [index, rom] of roms.entries()) {
      if (!findJob(jobId)) return;
      this.updateProgress(jobId, {
        active: true,
        fileName: `Matching IGDB metadata for ${rom.name}`,
        fileIndex: index + 1,
        fileCount: roms.length,
        loaded: totalBytes,
        total: totalBytes,
        percent: 100,
        speed: 0,
      });
      try {
        const match = await transfer.autoApplyIgdbMetadata({ rom });
        if (match.applied) {
          roms[index] = match.rom;
          matches.push({ rom: match.rom, selected: match.selected });
        } else if (match.reason) {
          failures.push(`${rom.name}: ${match.reason}`);
        }
      } catch (error) {
        failures.push(`${rom.name}: ${error.message || error}`);
      }
    }
    if (failures.length) console.warn('Automatic IGDB metadata matching skipped or failed for some uploads', failures);
  }

  uploadSuccessMessage(batch, platform, roms, matches, failures) {
    const warnings = batch?.warnings?.length ? ` ${batch.warnings.length} ingest warning(s) returned.` : '';
    const metadata = matches.length
      ? ` Auto-applied IGDB metadata to ${matches.length} game${matches.length === 1 ? '' : 's'}.`
      : state.igdbStatus?.configured
        ? ' No IGDB metadata match was applied.'
        : ' IGDB credentials are not configured, so metadata matching was skipped.';
    const failed = failures.length
      ? ` ${failures.length} metadata lookup(s) skipped or failed: ${failures.slice(0, 3).join('; ')}${failures.length > 3 ? '; …' : ''}.`
      : '';
    return `Uploaded ${roms.length} planned game${roms.length === 1 ? '' : 's'} to ${platform.display_name}.${warnings}${metadata}${failed}`;
  }
}

export function applyPlannedTitle(plan, planId, title) {
  const rom = plan?.roms?.find((candidate) => candidate.plan_id === planId);
  if (!rom) return null;
  const previousTitle = rom.title;
  const previousSlug = rom.slug;
  rom.title = String(title ?? '');
  rom.slug = slugifyTitle(rom.title.trim());
  for (const group of rom.groups || []) {
    if (group.display_name === previousTitle) group.display_name = rom.title;
    if (group.group_key === previousSlug) group.group_key = rom.slug;
    else if (group.group_key?.startsWith(`${previousSlug}:`)) {
      group.group_key = `${rom.slug}${group.group_key.slice(previousSlug.length)}`;
    }
  }
  return rom;
}

export function plannedTitlesAreValid(plan = state.uploadPlan) {
  return Boolean(plan?.roms?.length)
    && plan.roms.every((rom) => String(rom.title || '').trim());
}
