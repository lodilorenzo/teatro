import { attr, formatBytes, formatSpeed, html } from '../dom.js';
import { isJobActive, state } from '../state.js';
import { renderGogImportProgress } from './gog-import.js';

function statusLabel(job) {
  if (job.state === 'succeeded') return 'Complete';
  if (job.state === 'failed') return 'Failed';
  if (job.state === 'cancelled') return 'Cancelled';
  if (job.state === 'unavailable') return 'Unavailable';
  if (job.progress?.postProcessing) return 'Finishing';
  if (job.progress?.serverProcessing) return job.progress.jobState === 'queued' ? 'Queued' : 'Running';
  return job.state === 'queued' ? 'Queued' : 'Running';
}

export function jobListPriority(job) {
  if (!isJobActive(job)) return 2;
  if (!job.progress?.postProcessing
    && (job.state === 'queued'
      || (job.progress?.serverProcessing && job.progress.jobState === 'queued'))) return 1;
  return 0;
}

function jobTypeLabel(job) {
  if (job.type === 'gog-import') return 'GOG setup import';
  if (job.type === 'library-scan') return 'Managed library scan';
  if (job.type === 'romm-import') return 'RomM import';
  return 'Library upload';
}

function safeJobDomId(jobId, suffix) {
  return `${String(jobId).replace(/[^A-Za-z0-9_-]/gu, '-').slice(0, 96)}-${suffix}`;
}

function formatJobDuration(job) {
  const milliseconds = Number(job.completedAt) - Number(job.createdAt);
  if (!Number.isFinite(milliseconds) || milliseconds < 0) return '';
  if (milliseconds < 1000) return '<1s';
  const seconds = Math.round(milliseconds / 1000);
  const parts = [
    [Math.floor(seconds / 3600), 'h'],
    [Math.floor((seconds % 3600) / 60), 'm'],
    [seconds % 60, 's'],
  ];
  return parts.filter(([value]) => value).map(([value, unit]) => `${value}${unit}`).join(' ');
}

function renderUploadJobProgress(job) {
  const progress = job.progress || {};
  const percent = Number.isFinite(Number(progress.percent))
    ? Math.min(100, Math.max(0, Number(progress.percent)))
    : 0;
  const currentId = safeJobDomId(job.id, 'current');
  const percentId = safeJobDomId(job.id, 'percent');
  const terminal = !isJobActive(job);
  return `
    <div class="upload-progress" aria-busy="${attr(isJobActive(job))}">
      <div id="${attr(currentId)}" class="upload-progress-current" role="status" aria-live="polite">${html(job.statusText || progress.fileName || 'Preparing upload')}</div>
      <progress max="100" value="${attr(percent)}" aria-labelledby="${attr(currentId)} ${attr(percentId)}" ${terminal ? 'hidden' : ''}></progress>
      <div class="upload-progress-labels">
        <span id="${attr(percentId)}">${html(Math.round(percent))}%</span>
        <span>${html(formatSpeed(progress.speed))}</span>
        <span>${html(formatBytes(progress.loaded))} / ${html(formatBytes(progress.total))}</span>
      </div>
    </div>`;
}

function renderLibraryScanProgress(job) {
  const progress = job.progress || {};
  const phase = {
    queued: 'Queued', discovering: 'Discovering files', planning: 'Planning batches',
    importing: 'Importing batches', fetching_covers: 'Downloading IGDB covers', complete: 'Complete',
  }[progress.phase] || 'Scanning managed library';
  const batches = progress.jobProgress;
  const polling = progress.pollingError
    ? `<div class="warning">${html(progress.pollingError)}</div>`
    : '';
  return `<div class="upload-progress" aria-busy="${attr(isJobActive(job))}">
    <div class="upload-progress-current" role="status" aria-live="polite">${html(phase)}</div>
    ${batches ? `<progress max="100" value="${attr(batches.percent)}"></progress>
      <div class="upload-progress-labels"><span>${html(Math.round(batches.percent))}%</span><span>${html(batches.current)} / ${html(batches.total)} batches</span></div>` : ''}
    ${polling}
  </div>`;
}

function renderRommImportProgress(job) {
  const progress = job.progress || {};
  const phase = {
    queued: 'Queued', resolving: 'Resolving remote game', checking: 'Checking for duplicates',
    downloading: 'Downloading from RomM', verifying: 'Verifying downloaded files',
    ingesting: 'Importing into the library', finalizing: 'Finalizing', complete: 'Complete',
  }[progress.phase] || 'Importing from RomM';
  const bytes = progress.jobProgress;
  const polling = progress.pollingError
    ? `<div class="warning">${html(progress.pollingError)}</div>`
    : '';
  return `<div class="upload-progress" aria-busy="${attr(isJobActive(job))}">
    <div class="upload-progress-current" role="status" aria-live="polite">${html(phase)}</div>
    ${bytes ? `<progress max="100" value="${attr(bytes.percent)}"></progress>
      <div class="upload-progress-labels"><span>${html(Math.round(bytes.percent))}%</span><span>${html(formatBytes(bytes.current))} / ${html(formatBytes(bytes.total))}</span></div>` : ''}
    ${polling}
  </div>`;
}

/// A declared hash the download did not reproduce is reported, not hidden: the import was allowed
/// to complete, so the operator is told exactly which value disagreed and which one Teatro kept.
function renderRommHashConflicts(job) {
  const conflicts = job.result?.hash_conflicts || [];
  if (!conflicts.length) return '';
  return `<div class="job-result warning">
      ${html(conflicts.length)} declared hash${conflicts.length === 1 ? '' : 'es'} did not match the downloaded bytes. Teatro imported the file it actually received and recorded the hashes it computed, so the library now describes the local copy. Verify the game, or delete it and re-import if the remote server was mid-write.
    </div>
    <ul class="scan-result-list romm-hash-conflicts">${conflicts.map((conflict) => `
      <li>
        <code>${html(conflict.file_name)}</code>
        <span>${html(conflict.algorithm)} declared <code class="romm-hash-declared">${html(conflict.declared)}</code></span>
        <span>${html(conflict.algorithm)} computed <code>${html(conflict.computed)}</code></span>
      </li>`).join('')}</ul>`;
}

function renderRommImportResult(job) {
  const result = job.result || {};
  const signals = result.hash_signals?.length
    ? `Duplicate rules used ${html(result.hash_signals.join(', '))} hashes.`
    : 'The remote server published no hashes, so the filename and title rules carried the duplicate check.';
  if (result.outcome === 'already_present') {
    const duplicate = result.duplicate || {};
    return `<div class="job-result warning">Skipped as already present: ${html(duplicate.detail || 'a matching game is already in Teatro')} Nothing was imported${result.downloaded_bytes ? ` after downloading ${html(formatBytes(result.downloaded_bytes))}` : ' and no bytes were transferred'}. Matched rule: ${html(duplicate.rule || 'unknown')}.</div>
      ${renderRommHashConflicts(job)}`;
  }
  const conflicts = result.hash_conflicts?.length || 0;
  return `<div class="job-result ${conflicts ? 'warning' : 'notice'}">Imported ${html(result.file_count || 0)} file${result.file_count === 1 ? '' : 's'} (${html(formatBytes(result.downloaded_bytes || 0))}) as “${html(result.title || job.title)}”. ${signals}</div>
    ${renderRommHashConflicts(job)}`;
}

function resultRom(job) {
  if (job.type === 'gog-import') return job.result?.rom || null;
  if (job.type === 'library-scan') return null;
  return job.result?.roms?.at?.(-1) || null;
}

function renderScanFileList(files, key, label, className) {
  if (!files.length) return '';
  return `<details data-job-details="${attr(key)}">
    <summary>${html(label)} (${html(files.length)})</summary>
    <ul class="scan-result-list ${attr(className)}">${files.map((file) => `<li><code>${html(file.relative_path)}</code><span>${html(file.reason)}</span></li>`).join('')}</ul>
  </details>`;
}

function renderScanCoverWarnings(warnings) {
  if (!warnings.length) return '';
  return `<details data-job-details="scan-cover-warnings">
    <summary>Cover download warnings (${html(warnings.length)})</summary>
    <ul class="scan-result-list warning">${warnings.map((warning) => `<li><code>${html(warning.rom_name)}</code><span>${html(warning.reason)}</span></li>`).join('')}</ul>
  </details>`;
}

function renderResult(job) {
  if (job.state !== 'succeeded') return '';
  if (job.type === 'gog-import') {
    const summary = job.result?.import;
    return `<div class="job-result notice">${summary
      ? `Published a ${html(formatBytes(summary.archive_bytes))} Windows ZIP from ${html(summary.extracted_file_count)} extracted files.`
      : 'The Windows archive was published successfully.'}</div>`;
  }
  if (job.type === 'romm-import') return renderRommImportResult(job);
  if (job.type === 'library-scan') {
    const result = job.result || {};
    const indexed = (result.unimported_files || []).filter(({ disposition }) => disposition === 'already_indexed');
    const rejected = (result.unimported_files || []).filter(({ disposition }) => disposition === 'not_imported');
    const coverWarnings = result.cover_warnings || [];
    const importedGameCount = result.imported_rom_count || 0;
    return `<div class="job-result ${rejected.length || coverWarnings.length ? 'warning' : 'notice'}">Imported ${html(importedGameCount)} game${importedGameCount === 1 ? '' : 's'} and ${html(result.imported_file_count || 0)} source files; downloaded ${html(result.cover_downloaded_count || 0)} IGDB covers; ${html(indexed.length)} files were already indexed; ${html(rejected.length)} files were not imported; ${html(result.generated_manifest_count || 0)} playlists were generated.</div>
      ${renderScanCoverWarnings(coverWarnings)}
      ${renderScanFileList(rejected, 'scan-not-imported', 'Files not imported', 'warning')}
      ${renderScanFileList(indexed, 'scan-already-indexed', 'Already indexed files', 'indexed')}`;
  }
  const romCount = job.result?.roms?.length || 0;
  const warnings = job.result?.batch?.warnings?.length || 0;
  return `<div class="job-result notice">Published ${html(romCount)} planned game${romCount === 1 ? '' : 's'}${warnings ? ` with ${html(warnings)} ingest warning${warnings === 1 ? '' : 's'}` : ''}.</div>`;
}

function renderActions(job) {
  const actions = [];
  if (resultRom(job)) {
    actions.push(`<button type="button" data-view-job-rom="${attr(job.id)}">View in library</button>`);
  }
  if (isJobActive(job) && (job.type !== 'library-scan' || job.serverJobId)) {
    actions.push(`<button class="danger" type="button" data-cancel-job="${attr(job.id)}">Cancel</button>`);
  }
  if (!isJobActive(job)) {
    actions.push(`<button class="ghost" type="button" data-dismiss-job="${attr(job.id)}">Dismiss</button>`);
  }
  if (!actions.length) return '';
  return `<div class="job-actions">${actions.join('')}</div>`;
}

export function renderJobCardBody(job) {
  const typeLabel = jobTypeLabel(job);
  const status = statusLabel(job);
  const created = new Date(job.createdAt).toLocaleString();
  const duration = formatJobDuration(job);
  return `
    <div class="job-heading">
      <div>
        <p class="eyebrow">${html(typeLabel)}</p>
        <h3>${html(job.title)}</h3>
        <p class="job-detail">${html(job.detail)}${job.detail ? ' · ' : ''}Started ${html(created)}${duration ? ` · Duration ${html(duration)}` : ''}</p>
      </div>
      <span class="job-status ${attr(job.state)}">${html(status)}</span>
    </div>
    ${job.type === 'gog-import'
      ? renderGogImportProgress(job.progress || {}, job.id)
      : job.type === 'library-scan' ? renderLibraryScanProgress(job)
        : job.type === 'romm-import' ? renderRommImportProgress(job) : renderUploadJobProgress(job)}
    ${job.error ? `<div class="${job.state === 'succeeded' ? 'warning' : 'error'} job-error">${html(job.error)}</div>` : ''}
    ${renderResult(job)}
    ${renderActions(job)}`;
}

function renderJobCard(job) {
  return `<article class="card job-card ${attr(job.type)}" data-job-shell="${attr(job.id)}" data-job-priority="${jobListPriority(job)}">${renderJobCardBody(job)}</article>`;
}

export function renderJobs() {
  const jobs = [...state.jobs].sort((left, right) => jobListPriority(left) - jobListPriority(right));
  const active = jobs.filter(isJobActive);
  const finished = jobs.filter((job) => !isJobActive(job));
  return `
    <section class="page-header">
      <div>
        <h1>Jobs</h1>
      </div>
      ${finished.length ? '<button class="ghost" type="button" data-action="clear-finished-jobs">Clear finished</button>' : ''}
    </section>
    ${active.length ? '<p class="hint">You can switch admin sections while jobs run. Leaving admin or reloading cancels active jobs.</p>' : ''}
    <section class="jobs-summary" aria-label="Job summary">
      <div><span>Active</span><strong>${html(active.length)}</strong></div>
      <div><span>Finished</span><strong>${html(finished.length)}</strong></div>
    </section>
    ${jobs.length
      ? `<div class="job-list">${jobs.map(renderJobCard).join('')}</div>`
      : '<div class="empty-state"><div><h3>No jobs yet</h3><p>Imports and library scans appear here.</p></div></div>'}`;
}
