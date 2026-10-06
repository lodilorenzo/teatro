import { attr, formatBytes, formatSpeed, html } from '../dom.js';
import { isJobActive, state } from '../state.js';

export function renderConversionImport() {
  const platform = state.platforms.find((item) => String(item.id) === state.uploadPlatformId);
  if (platform?.slug !== 'wiiu' || !state.conversionStatus?.enabled) return '';
  const files = state.conversionFiles;
  const inspection = state.conversionInspection;
  const busy = state.conversionInspecting;
  if (!files.length && !busy) return '';
  return `<section class="conversion-folders" aria-labelledby="conversion-heading">
    <h3 id="conversion-heading">Wii U folders</h3>
    <form id="conversion-form">
      <div id="conversion-selection" role="status" aria-live="polite" aria-busy="${busy}">
        <p class="hint conversion-summary">${busy ? 'Reading folders and checking title information…' : `${html(files.length)} files selected${files.length ? ` · ${html(formatBytes(files.reduce((total, entry) => total + entry.file.size, 0)))}` : ''}.`}</p>
        ${inspection ? `<ul class="conversion-flags" aria-label="Detected Wii U content">${[['base', 'Game'], ['update', 'Update'], ['dlc', 'DLC']].map(([kind, label]) => `<li class="${inspection.titles.some((title) => title.kind === kind) ? 'detected' : ''}">${label}: ${inspection.titles.some((title) => title.kind === kind) ? 'detected' : 'not detected'}</li>`).join('')}</ul>
          <ul class="conversion-title-list">${inspection.titles.map((title) => `<li><div><strong>${html({base:'Game', update:'Update', dlc:'DLC'}[title.kind] || 'Unrecognized folder')}</strong><span>${html(title.root || 'code/content/meta')}${title.titleId ? ` · <code>${html(title.titleId)}</code> · version ${html(title.version)}` : ''}</span>${title.error ? `<span class="warning">${html(title.error)}</span>` : ''}</div><button type="button" class="ghost" data-remove-conversion-folder="${attr(title.root)}" aria-label="Remove ${attr(title.root || 'title folder')}" ${busy ? 'disabled' : ''}>Remove</button></li>`).join('')}</ul>
          ${inspection.errors.map((error) => `<p class="warning">${html(error)}</p>`).join('')}` : ''}
      </div>
      <label>Game title<input id="conversion-title" name="title" required maxlength="512" value="${attr(state.conversionTitle || '')}" /></label>
      <p class="hint">Experimental folder-to-WUA import. Compression, decoded-content verification and import run automatically, even if the archive is larger than the inputs. Local originals are unchanged. Cemu launch qualification is pending. Only files inside code/content/meta are accepted; relative paths must be ASCII.</p>
      <div class="form-footer"><button type="button" class="ghost" data-clear-conversion ${files.length || busy ? '' : 'disabled'}>Clear folders</button><button type="submit" ${!files.length || busy || !inspection || inspection.errors.length || inspection.titles.some((title) => title.error) ? 'disabled' : ''}>Compress and import WUA</button></div>
    </form>
  </section>`;
}
export function renderConversionProgress(job) {
  const progress = job.progress || {};
  const phase = progress.phase || 'queued';
  const disc = ['rvz', 'chd'].includes(job.outputFormat);
  const sevenZip = job.outputFormat === '7z';
  const format = sevenZip ? '7z' : disc ? job.outputFormat.toUpperCase() : 'WUA';
  const active = isJobActive(job);
  const [label, detail] = {
    queued: ['Preparing import', 'Waiting for the import to start.'],
    queued_upload: ['Waiting to upload', 'Another conversion is running. Keep this page open; these local files will upload automatically when it finishes.'],
    uploading: [sevenZip ? 'Uploading ROM' : disc ? 'Uploading disc files' : 'Uploading folders', sevenZip ? 'Sending the uncompressed ROM to Teatro.' : disc ? 'Sending the disc files to Teatro. Compression has not started.' : 'Sending folder data to Teatro. Compression has not started.'],
    inspecting: [sevenZip ? 'Checking ROM' : disc ? 'Inspecting disc files' : 'Inspecting title files', sevenZip ? 'Checking that this is an uncompressed single-file ROM for the selected non-disc platform.' : disc ? 'Checking disc identity and structure for the selected platform. Wrong-platform and legacy images are rejected.' : 'Upload received. Teatro is checking the game, update and DLC folders.'],
    queued_conversion: ['Waiting for compression', 'The import will start when the compression worker is available.'],
    compressing: [`Compressing ${format}`, 'Processing input data into the archive. Finalization and verification follow.'],
    finalizing_wua: ['Finalizing WUA', 'All input data has been processed. The compressor is finishing and saving the archive.'],
    compressing_and_verifying: ['Compressing and verifying CHD discs', 'Each disc is compressed separately. Decoded tracks and every byte are checked before the CHDs, playlist and companions are imported.'],
    verifying: [`Verifying ${format}`, sevenZip ? 'Comparing the decoded ROM with the uploaded original.' : disc ? 'Comparing every decoded disc byte with the uploaded logical disc data.' : 'Checking every decoded file against the original folder data.'],
    publishing: [`Importing ${format} into the library`, 'Saving the verified archive and its library record.'],
    matching_igdb_metadata: ['Matching IGDB metadata', 'Searching for game metadata and downloading available covers.'],
  }[phase] || ['Processing import', 'Teatro is working on this import.'];
  const title = active ? label : {succeeded:'Import complete', failed:'Import failed', cancelled:'Import cancelled', unavailable:'Import unavailable'}[job.state] || job.statusText || label;
  const bytes = phase === 'uploading' ? progress.uploadProgress : progress.jobProgress;
  const percent = bytes ? Math.min(100, Math.max(0, Number(bytes.percent) || 0)) : null;
  const working = active;
  const statusId = `conversion-${String(job.id).replace(/[^A-Za-z0-9_-]/gu, '-').slice(0, 96)}-status`;
  return `<div class="upload-progress conversion-progress" aria-busy="${working}" data-conversion-phase="${attr(phase)}">
      <div id="${attr(statusId)}" role="status" aria-live="polite"><strong>${html(title)}</strong>${active ? `<p class="hint">${html(detail)}</p>` : ''}</div>
      ${working ? `<progress max="100" ${bytes ? `value="${attr(percent)}"` : ''} aria-labelledby="${attr(statusId)}"></progress>
        ${bytes ? `<div class="upload-progress-labels"><span>${html(Math.round(percent))}%</span><span>${html(formatBytes(bytes.current))} / ${html(formatBytes(bytes.total))}${['compressing', 'compressing_and_verifying'].includes(phase) ? ' input processed' : ''}</span>${phase === 'uploading' ? `<span>${html(formatSpeed(bytes.speed || 0))}</span>` : ''}</div>` : '<p class="hint">Progress is not measurable for this step.</p>'}` : ''}
      ${progress.pollingError ? `<p class="warning">${html(progress.pollingError)} Processing may still be running on the server.</p>` : ''}
    </div>`;
}
