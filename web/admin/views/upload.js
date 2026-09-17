import { icon } from '../../public/icons.js';
import { html } from '../dom.js';
import { state } from '../state.js';
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
              <input id="upload-file-input" class="drop-input" name="file" type="file" multiple />
              <span class="drop-zone-icon">${icon('upload')}</span>
              <strong>Drop game files here</strong>
              <span>or choose files</span>
              <small id="upload-file-name" class="mono">${html(uploadSelectionLabel())}</small>
            </label>
            <div id="upload-file-list" class="upload-file-list">${renderUploadFileList()}</div>
            <div class="upload-file-actions">
              <button class="ghost" type="button" data-action="clear-upload-list" ${state.uploadSelectedFiles.length ? '' : 'disabled'}>Clear files</button>
            </div>
          </div>
        </div>
        <div id="upload-plan-panel">${renderUploadPlanPanel()}</div>
        <div class="form-footer">
          <button type="button" data-action="preview-upload-plan" ${state.uploadPreviewLoading ? 'disabled' : ''}>${state.uploadPreviewLoading ? 'Reviewing…' : 'Review files'}</button>
          <button class="primary" data-action="finalize-upload-plan" type="submit" ${!plan || planHasErrors || planHasInvalidTitles ? 'disabled' : ''}>Upload games</button>
        </div>
      </form>
    </section>`;
}
