import { icon } from '../../public/icons.js';
import { attr, formatBytes, formatSpeed, html } from '../dom.js';
import { state } from '../state.js';

export const GOG_IMPORT_PHASE_LABELS = Object.freeze({
  queued: 'Queued',
  validate_request: 'Validating request',
  resolve_windows_target: 'Resolving Windows target',
  verify_innoextract_hash: 'Verifying innoextract',
  prepare_private_workspace: 'Preparing private workspace',
  stage_setup_files: 'Staging setup files',
  probe_innoextract_version: 'Checking innoextract version',
  probe_installer_data_version: 'Checking installer data version',
  test_installer_integrity: 'Testing installer integrity',
  extract_installer_payload: 'Extracting installer payload',
  validate_extracted_tree: 'Validating extracted tree',
  create_windows_zip: 'Creating Windows ZIP',
  finalize_windows_zip: 'Finalizing Windows ZIP',
  validate_sipario_zip: 'Validating Windows ZIP',
  stage_generated_archive: 'Staging generated archive',
  publish_windows_archive: 'Publishing Windows archive',
  cleanup_uploaded_setup_staging: 'Cleaning uploaded setup staging',
  complete: 'GOG setup import complete',
});

function gogImportPhaseLabel(phase) {
  return GOG_IMPORT_PHASE_LABELS[phase] || 'Processing GOG setup import';
}

export function gogImportTitleHint() {
  if (state.gogImportTitleOrigin === 'inferred') {
    return state.gogImportTitleConfidence === 'high'
      ? 'Suggested from the filename. Check the title before importing.'
      : 'The filename is ambiguous. Check the suggested title carefully.';
  }
  return 'Used for the game title and ZIP filename. Platform: Windows.';
}

export function gogImportSelectionLabel(files = state.gogImportSelectedFiles) {
  if (!files.length) return 'No setup files selected';
  const total = files.reduce((sum, file) => sum + Number(file.size || 0), 0);
  return `${files.length} setup file${files.length === 1 ? '' : 's'} · ${formatBytes(total)}`;
}

export function renderGogImportFileList(files = state.gogImportSelectedFiles) {
  if (!files.length) return '';
  return `<ul data-scroll-key="gog-import-file-list">${files.map((file, index) => `
    <li>
      <span class="mono">${html(file.name)}</span>
      <small>${html(formatBytes(file.size))}</small>
      <button class="ghost upload-file-remove" type="button" data-remove-gog-file="${attr(index)}">Remove</button>
    </li>`).join('')}</ul>`;
}

function boundedPercent(value) {
  const percent = Number(value);
  return Number.isFinite(percent) ? Math.min(100, Math.max(0, percent)) : 0;
}

function jobDomId(jobId, suffix) {
  const safeJobId = String(jobId || 'gog-job').replace(/[^A-Za-z0-9_-]/gu, '-').slice(0, 96);
  return `${safeJobId}-${suffix}`;
}

function phaseRows(progress) {
  const rows = [];
  const phases = new Set();
  for (const event of progress.phaseEvents || []) {
    if (!event?.phase || phases.has(event.phase)) continue;
    phases.add(event.phase);
    rows.push({ phase: event.phase, text: event.text || gogImportPhaseLabel(event.phase) });
  }
  if (progress.phase && !phases.has(progress.phase)) {
    rows.push({ phase: progress.phase, text: gogImportPhaseLabel(progress.phase) });
  }
  return rows;
}

function renderPhaseHistory(progress) {
  if (!progress.jobId) return '';
  const rows = phaseRows(progress);
  if (!rows.length) return '';
  return `
    <ol class="gog-import-server-phases" aria-label="Observed server-side import phases">
      ${rows.map((row) => {
        const current = row.phase === progress.phase;
        const failed = current && progress.jobState === 'failed';
        const className = failed ? 'failed' : current ? 'current' : 'completed';
        const stateLabel = failed ? 'Failed' : current ? 'Current' : 'Completed';
        return `<li class="${className}" data-gog-phase="${attr(row.phase)}"><span>${html(row.text)}</span><small>${stateLabel}</small></li>`;
      }).join('')}
    </ol>`;
}

function renderTranscript(progress, jobId) {
  if (!progress.jobId) return '';
  const entries = progress.transcript || [];
  const transcriptId = jobDomId(jobId, 'transcript');
  return `
    <details class="gog-import-output" data-job-details="transcript" ${entries.length ? 'open' : ''}>
      <summary><code>innoextract</code> output <span>(${html(entries.length)} retained)</span></summary>
      ${progress.outputTruncated ? '<div class="warning gog-import-output-truncated">Earlier or oversized output was discarded to keep this view bounded.</div>' : ''}
      <pre id="${attr(transcriptId)}" data-job-transcript tabindex="0" aria-label="Sanitized innoextract output" aria-live="off">${entries.length
        ? entries.map((event) => `<span class="gog-import-output-entry ${attr(event.stream || 'stdout')}"><span class="gog-import-output-stream">${html(event.stream || 'stdout')}</span> ${html(event.text)}</span>`).join('')
        : '<span class="gog-import-output-empty">No extractor output has been received for this job.</span>'}</pre>
    </details>`;
}

function jobPresentation(progress) {
  if (!progress.serverProcessing) {
    return {
      label: progress.fileName || 'Uploading GOG setup files',
      determinate: true,
      percent: boundedPercent(progress.percent),
      percentLabel: `${Math.round(boundedPercent(progress.percent))}%`,
      secondary: formatSpeed(progress.speed),
      bytes: `${formatBytes(progress.loaded)} / ${formatBytes(progress.total)}`,
      message: 'Uploading the selected offline setup files to Teatro.',
    };
  }

  const jobProgress = progress.jobProgress;
  const determinate = jobProgress?.kind === 'percent' || jobProgress?.kind === 'bytes';
  const percent = determinate ? boundedPercent(jobProgress.percent) : 0;
  let label = gogImportPhaseLabel(progress.phase);
  let percentLabel = determinate ? `${Math.round(percent)}%` : 'Working';
  let secondary = 'Processing on the server';
  let bytes = 'Progress unavailable for this step';
  let message = 'You can keep working in admin while this runs.';

  if (jobProgress?.kind === 'bytes') {
    bytes = `${formatBytes(jobProgress.current)} / ${formatBytes(jobProgress.total)}`;
    secondary = 'Measured bytes';
  } else if (jobProgress?.kind === 'percent') {
    bytes = 'Estimated progress';
    secondary = 'Extractor-reported percentage';
  }
  if (progress.reattached) {
    label = 'Restoring job status';
    secondary = 'Checking Teatro for current status';
    message = 'Checking the saved job ID. Selected files and previous output were not saved.';
  }
  if (progress.pollingError && !progress.unavailable) {
    secondary = 'Connection interrupted. Retrying…';
    message = 'Could not get an update. Retrying without starting another import.';
  }
  if (progress.postProcessing) {
    label = 'Import complete. Refreshing library…';
    percentLabel = 'Complete';
    secondary = 'Updating metadata';
    bytes = 'Windows ZIP added to the library';
    message = 'The game is imported. Updating its library details.';
  } else if (progress.jobState === 'succeeded') {
    label = gogImportPhaseLabel('complete');
    percentLabel = 'Complete';
    secondary = 'Import complete';
    bytes = 'Windows ZIP added to the library';
    message = 'Teatro completed and published the import.';
  } else if (progress.jobState === 'cancelled') {
    label = 'Import cancelled';
    percentLabel = 'Cancelled';
    secondary = 'Server processing stopped';
    bytes = 'No further import phases will run';
    message = 'The job was cancelled by an administrator.';
  } else if (progress.jobState === 'failed') {
    label = `Import failed during ${gogImportPhaseLabel(progress.phase).toLowerCase()}`;
    percentLabel = 'Failed';
    secondary = 'No partial game was published';
    bytes = 'Check the error and output below';
    message = 'The server job stopped at the displayed phase.';
  } else if (progress.unavailable) {
    label = 'Import job status is unavailable';
    percentLabel = 'Unavailable';
    secondary = 'Could not restore job status';
    bytes = 'The server may have restarted or the job may have expired';
    message = 'Check the library before importing again. The game may already have been added.';
  }

  return { label, determinate, percent, percentLabel, secondary, bytes, message };
}

export function renderGogImportProgress(progress, jobId = 'gog-import-job') {
  const presentation = jobPresentation(progress || {});
  const progressValue = presentation.determinate ? ` value="${attr(presentation.percent)}"` : '';
  const terminal = progress?.unavailable
    || (!progress?.postProcessing && ['succeeded', 'failed', 'cancelled'].includes(progress?.jobState));
  const progressHidden = terminal ? ' hidden' : '';
  const currentId = jobDomId(jobId, 'current');
  const percentId = jobDomId(jobId, 'percent');

  return `
    <div class="upload-progress${progress?.serverProcessing ? ' server-processing' : ''}" aria-busy="${attr(Boolean(progress?.active))}">
      <div id="${attr(currentId)}" class="upload-progress-current" role="status" aria-live="polite">${html(presentation.label)}</div>
      <progress max="100" aria-labelledby="${attr(currentId)} ${attr(percentId)}"${progressValue}${progressHidden}></progress>
      <div class="upload-progress-labels">
        <span id="${attr(percentId)}">${html(presentation.percentLabel)}</span>
        <span>${html(presentation.secondary)}</span>
        <span>${html(presentation.bytes)}</span>
      </div>
      ${progress?.serverProcessing ? `
        <div class="gog-import-server-work">
          ${progress.active ? '<span class="server-working-spinner" aria-hidden="true"></span>' : ''}
          <div><span>${html(presentation.message)}</span></div>
        </div>
        ${progress.pollingError ? `<div class="warning gog-import-poll-warning">${html(progress.pollingError)}</div>` : ''}
        ${progress.jobError?.message ? `<div class="error gog-import-job-error">${html(progress.jobError.message)}</div>` : ''}
        ${renderPhaseHistory(progress)}
        ${renderTranscript(progress, jobId)}` : ''}
    </div>`;
}

function renderGogImportPreparationNotice() {
  return '<p class="hint preparation-notice">Follow progress in Jobs. You can prepare another import while it runs.</p>';
}

export function renderGogImportWarning() {
  return `<div class="warning gog-import-warning">
    Some GOG installers need extra components or installation steps. Extracted games may not run without them. Failed imports are not added to the library.
  </div>`;
}

export function renderGogImport({ embedded = false } = {}) {
  const status = state.gogImportStatus;
  const available = Boolean(status?.enabled && status?.configured);
  const limits = status
    ? ` Current limits: ${html(status.max_extracted_files)} extracted entries, ${html(formatBytes(status.max_extracted_bytes))} total extracted data, and ${html(status.timeout_seconds)}s per extraction phase.`
    : '';
  const description = `Choose one GOG offline setup <code>.exe</code> you own and all matching <code>.bin</code> parts. Teatro tests and extracts them with <code>innoextract</code>, then adds a ZIP to the Windows library.${limits}`;
  const statusMessage = !status
    ? '<div class="warning">Importer status is unavailable.</div>'
    : available
      ? ''
      : '<div class="warning">The importer is unavailable. Enable GOG import on the server and configure a hash-pinned <code>innoextract</code>. The Docker image includes it.</div>';

  return `
    ${embedded ? '' : `<section class="page-header">
      <div>
        <h1>Import GOG installer</h1>
        <p>${description}</p>
      </div>
    </section>`}
    ${statusMessage}
    ${embedded ? '' : renderGogImportPreparationNotice()}
    <section class="card${embedded ? ' import-card' : ''}">
      ${embedded ? `<header class="import-card-header">
        <h2 class="section-title">GOG installer</h2>
        <details class="import-info">
          <summary aria-label="About GOG setup import">${icon('info')}</summary>
          <div class="import-info-popup"><p>${description}</p></div>
        </details>
      </header>` : ''}
      <form id="gog-import-form">
        <div class="form-grid">
          <label class="wide"><span>Game title</span><input name="title" maxlength="512" value="${attr(state.gogImportTitle)}" aria-describedby="gog-import-title-hint" required ${available ? '' : 'disabled'} /><small id="gog-import-title-hint" class="hint" data-title-origin="${attr(state.gogImportTitleOrigin)}" data-title-confidence="${attr(state.gogImportTitleConfidence || '')}">${html(gogImportTitleHint())}</small></label>
          <div class="wide">
            <span class="field-label">Offline setup files</span>
            <label id="gog-import-drop-zone" class="drop-zone" for="gog-import-file-input">
              <input id="gog-import-file-input" class="drop-input" name="file" type="file" accept=".exe,.bin" multiple ${available ? '' : 'disabled'} />
              <span class="drop-zone-icon">${icon('package-open')}</span>
              <strong>Drop the setup .exe and all .bin parts here</strong>
              <span>or choose setup files</span>
              <small id="gog-import-file-name" class="mono">${html(gogImportSelectionLabel())}</small>
            </label>
            <div id="gog-import-file-list" class="upload-file-list">${renderGogImportFileList()}</div>
            <div class="upload-file-actions">
              <button class="ghost" type="button" data-action="clear-gog-import-list" ${state.gogImportSelectedFiles.length ? '' : 'disabled'}>Clear setup files</button>
            </div>
          </div>
        </div>
        ${renderGogImportWarning()}
        <div class="form-footer">
          <button id="gog-import-submit" class="primary" type="submit" ${available ? '' : 'disabled'}>Import installer</button>
        </div>
      </form>
    </section>`;
}
