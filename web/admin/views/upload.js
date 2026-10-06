import { icon } from '../../public/icons.js';
import { html } from '../dom.js';
import { canCompressChd, canCompressRvz, canCompressSevenZip, chdFormat, state } from '../state.js';
import { renderConversionImport } from './conversion.js';
import { renderGogImport } from './gog-import.js';
import { platformPicker } from './shared.js';
import {
  renderUploadFileList, renderUploadPlanPanel, uploadSelectionLabel,
} from './upload-renderers.js';

export function renderImport() {
  return `
    <section class="page-header">
      <div>
        <h1>Import</h1>
      </div>
    </section>
    <div class="import-columns">
      <div class="import-notice">${renderUploadPreparationNotice()}</div>
      <div class="import-section import-upload-section">${renderUpload({ embedded: true })}</div>
      <div class="import-section import-gog-section">${renderGogImport({ embedded: true })}</div>
    </div>`;
}

function renderUploadPreparationNotice() {
  return '<p class="hint preparation-notice">Follow progress in Jobs. You can prepare another import while it runs.</p>';
}

export function renderUpload({ embedded = false } = {}) {
  const plan = state.uploadPlan;
  const rvzAvailable = canCompressRvz();
  const chd = chdFormat();
  const chdAvailable = canCompressChd();
  const platform = state.platforms.find((item) => String(item.id) === state.uploadPlatformId);
  const isWii = platform?.slug === 'wii';
  const wiiuFolders = platform?.slug === 'wiiu' && state.conversionStatus?.enabled;
  const folderSelection = wiiuFolders && (state.conversionFiles.length || state.conversionInspecting);
  const compressRvz = state.uploadCompressRvz && rvzAvailable;
  const compressChd = state.uploadCompressChd && chdAvailable;
  const sevenZipAvailable = canCompressSevenZip();
  const compressSevenZip = state.uploadCompressSevenZip && sevenZipAvailable;
  const compress = compressRvz || compressChd || compressSevenZip;
  const description = 'Upload game files together with their playlists and track files. Review the detected games before uploading. Teatro keeps supplied playlists and can create <code>.m3u</code> playlists for multi-disc games.';
  const planHasErrors = Boolean(plan?.errors?.length);
  const planHasInvalidTitles = Boolean(
    plan && (!plan.roms?.length || plan.roms.some((rom) => !String(rom.title || '').trim())),
  );
  return `
    ${embedded ? '' : `<section class="page-header">
      <div>
        <h1>Upload games</h1>
      </div>
    </section>`}
    ${embedded ? '' : renderUploadPreparationNotice()}
    <section class="card${embedded ? ' import-card' : ''}">
      ${embedded ? `<header class="import-card-header">
        <h2 class="section-title">Game files</h2>
        <details class="import-info">
          <summary aria-label="About game upload">${icon('info')}</summary>
          <div class="import-info-popup"><p>${description}</p></div>
        </details>
      </header>` : ''}
      <form id="upload-form">
        <div class="form-grid">
          <div class="wide"><span class="field-label">Platform</span>${platformPicker('platform_id', state.uploadPlatformId)}<small class="hint">For Windows, upload one prebuilt <code>.zip</code> or <code>.7z</code> archive per game. Teatro stores the archive unchanged.</small></div>
          <div class="wide">
            <label id="upload-drop-zone" class="drop-zone" for="upload-file-input">
              <input id="upload-file-input" class="drop-input" name="file" type="file" multiple ${wiiuFolders ? 'aria-describedby="conversion-folder-hint"' : ''} />
              <span class="drop-zone-icon">${icon('upload')}</span>
              <strong>${wiiuFolders ? 'Drop game files or decrypted Wii U folders here' : 'Drop game files here'}</strong>
              <span>or choose files</span>
              <small id="upload-file-name" class="mono">${html(folderSelection ? `${state.conversionFiles.length} folder files selected` : uploadSelectionLabel())}</small>
            </label>
            <div id="upload-file-list" class="upload-file-list">${renderUploadFileList()}</div>
            <div class="upload-file-actions">
              <button class="ghost" type="button" data-action="clear-upload-list" ${state.uploadSelectedFiles.length ? '' : 'disabled'}>Clear files</button>
              ${wiiuFolders ? `<button class="ghost" type="button" data-add-conversion-folder ${state.conversionInspecting ? 'disabled' : ''}>Add decrypted folder</button><input id="conversion-folder" type="file" webkitdirectory multiple hidden aria-describedby="conversion-folder-hint" />` : ''}
            </div>
            ${wiiuFolders ? '<p class="hint" id="conversion-folder-hint">Choose files normally, or add a decrypted game folder with <code>code/content/meta</code>. Add its matching update and DLC here too, or drop a parent folder containing all three. Import game files and folders as separate jobs.</p>' : ''}
          </div>
        </div>
        ${folderSelection ? '' : `${rvzAvailable ? `<label class="checkbox"><input id="upload-compress-rvz" type="checkbox" aria-describedby="upload-rvz-hint" ${compress ? 'checked' : ''} /> Compress to <code>.rvz</code> for Dolphin</label>
          <p class="hint" id="upload-rvz-hint">Select one ${isWii ? '.iso or single-file .wbfs Wii' : 'plain .iso or .gcm GameCube'} disc image. Existing RVZ and legacy WIA, GCZ and NKit images are not recompressed. Experimental: once started, compression, verification and import run automatically. Dolphin launch qualification is pending.</p>` : ''}
        ${chd ? `<label class="checkbox"><input id="upload-compress-chd" type="checkbox" aria-describedby="upload-chd-hint" ${compressChd ? 'checked' : ''} ${chdAvailable ? '' : 'disabled'} /> Compress to <code>.chd</code> for ${html(chd.emulator)}</label>
          <p class="hint" id="upload-chd-hint">Choose compression now, then review files to validate compatibility and detect disc sets and separate games. ${platform.slug === 'psp' ? 'Select plain ISO or 2048-byte IMG images.' : 'Select raw 2352-byte BINARY CUE/BIN discs or single-data-track IMG images' + (['psx', 'ps2'].includes(platform.slug) ? ', or compatible ISO images.' : '.')} Teatro queues one job per detected game. Each disc becomes its own CHD; M3U entries point to the compressed discs and SBI companions stay attached. ISO/IMG conversion cannot restore missing audio; mixed-track IMG images need their CUE. Keep this page open until queued uploads have started. INDEX 00, POSTGAP and track flags are unsupported. Experimental: verification and import run automatically, including larger outputs. Emulator qualification is pending.</p>` : ''}
        ${sevenZipAvailable ? `<label class="checkbox"><input id="upload-compress-seven-zip" type="checkbox" aria-describedby="upload-seven-zip-hint" ${compressSevenZip ? 'checked' : ''} /> Compress uncompressed ROMs to <code>.7z</code></label>
          <p class="hint" id="upload-seven-zip-hint">Teatro creates and verifies one archive per uncompressed ROM before import. Already-compressed files are uploaded unchanged. Review files before launching. Keep this page open until queued uploads have started. Your local originals are unchanged.</p>` : ''}
        <div id="upload-plan-panel">${compressRvz ? '<p class="hint">The image will be inspected, compressed and verified before import. Larger verified outputs are also imported automatically. Your local original is unchanged.</p>' : renderUploadPlanPanel()}</div>
        <div class="form-footer">
          <button type="button" data-action="preview-upload-plan" ${state.uploadPreviewLoading || compressRvz ? 'disabled' : ''}>${state.uploadPreviewLoading ? 'Reviewing…' : 'Review files'}</button>
          <button class="primary" data-action="finalize-upload-plan" type="submit" ${!compressRvz && (!plan || planHasErrors || planHasInvalidTitles) ? 'disabled' : ''}>${compress ? `Compress and import ${compressChd ? 'CHD' : compressSevenZip ? '7z' : 'RVZ'}` : 'Upload games'}</button>
        </div>`}
      </form>
      ${renderConversionImport()}
    </section>`;
}
