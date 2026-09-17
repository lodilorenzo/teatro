import { attr, formatBytes, html } from '../dom.js';
import { state } from '../state.js';
import { platformSelectFromList } from './shared.js';

function renderSidecarCleanup() {
  const preview = state.sidecarCleanupPreview;
  if (state.sidecarCleanupLoading) return '<p class="hint" role="status">Checking the managed library for sidecars...</p>';
  if (!preview) return '<p class="hint">Preview unindexed text, image, document, OS metadata, and Zone.Identifier files before deleting any of them.</p>';
  if (!preview.files.length) return '<div class="notice">No unindexed sidecars were found.</div>';

  return `
    <p class="warning">Review all ${html(preview.file_count)} files below. Cleanup deletes only this ${html(formatBytes(preview.total_file_bytes))} preview through recoverable trash. Indexed files, configured assets, and folders are kept.</p>
    ${preview.truncated ? '<p class="hint">The preview is limited to 256 files. Preview again after this batch to check for more.</p>' : ''}
    <div class="table-wrap sidecar-cleanup-preview"><table><thead><tr><th>File</th><th>Size</th></tr></thead><tbody>${preview.files.map((file) => `
      <tr><td><code>${html(file.relative_path)}</code></td><td>${html(formatBytes(file.file_size_bytes))}</td></tr>`).join('')}</tbody></table></div>
    <form id="sidecar-cleanup-form" class="form-grid">
      <label class="wide"><span>Type DELETE SIDECARS to confirm</span><input name="confirm" placeholder="Type DELETE SIDECARS" required /></label>
      <div class="form-footer wide"><button class="danger" type="submit">Delete previewed sidecars</button></div>
    </form>`;
}

export function renderServerCleanup() {
  const platforms = (state.platforms || []).filter((platform) => Number(platform.rom_count || 0) > 0);
  const selected = platforms[0];
  return `
    <div class="cleanup-section">
      <section>
        <h3>Unindexed sidecars</h3>
        ${renderSidecarCleanup()}
        <div class="form-footer"><button type="button" data-action="preview-sidecar-cleanup" ${state.sidecarCleanupLoading ? 'disabled' : ''}>${state.sidecarCleanupPreview ? 'Refresh sidecar preview' : 'Preview sidecar cleanup'}</button></div>
      </section>
      <section class="cleanup-clear-form">
        <p class="warning">These actions permanently delete managed games and their managed files from disk. Users, API tokens, IGDB credentials, and server configuration are kept.</p>
        <form id="bulk-delete-platform-form" class="form-grid">
          <label class="wide"><span>Platform</span>${platformSelectFromList('platform_id', platforms, selected?.id || '', false)}</label>
          <label class="wide"><span>Type the platform code to confirm</span><input name="confirm" placeholder="${attr(selected?.slug || 'platform-slug')}" ${platforms.length ? 'required' : 'disabled'} /></label>
          <p class="hint wide">Only games and managed files for the selected platform are removed.</p>
          <div class="form-footer wide"><button class="danger" type="submit" ${platforms.length ? '' : 'disabled'}>Delete platform games</button></div>
        </form>
        <form id="clear-server-form" class="form-grid cleanup-clear-form">
          <label class="wide"><span>Type DELETE ALL to confirm</span><input name="confirm" placeholder="Type DELETE ALL" required /></label>
          <p class="hint wide">Deletes every game and its managed files across all platforms.</p>
          <div class="form-footer wide"><button class="danger" type="submit" ${Number(state.stats?.total_roms || 0) ? '' : 'disabled'}>Delete all games</button></div>
        </form>
      </section>
    </div>`;
}
