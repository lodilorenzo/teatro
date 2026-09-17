import { attr, csv, formatBytes, formatRole, html } from '../dom.js';
import { state } from '../state.js';

export function renderUploadPlanPanel() {
  if (state.uploadPreviewLoading) {
    return '<p class="hint" role="status">Checking filenames and playlists…</p>';
  }
  if (!state.uploadPlan) {
    return '<p class="hint">Review files to check titles, disc groups, and warnings before uploading.</p>';
  }

  const plan = state.uploadPlan;
  return `<div class="upload-plan">
    <div class="upload-plan-header">
      <div>
        <h3>${html(plan.roms?.length || 0)} game${plan.roms?.length === 1 ? '' : 's'} to upload</h3>
        <p class="hint">Check the titles below. To change disc groups, update the files and review again.</p>
      </div>
      <div class="chips">
        <span class="chip">${html((plan.roms || []).reduce((total, rom) => total + (rom.files?.length || 0), 0))} files</span>
        <span class="chip">${html((plan.roms || []).reduce((total, rom) => total + (rom.groups?.length || 0), 0))} groups</span>
      </div>
    </div>
    ${renderPlanMessages('error', plan.errors || [])}
    ${renderPlanMessages('warning', plan.warnings || [])}
    <div class="plan-rom-list">${(plan.roms || []).map(renderPlanRom).join('') || '<div class="empty-state">No games were planned.</div>'}</div>
  </div>`;
}

function renderPlanMessages(kind, messages) {
  if (!messages.length) return '';
  const className = kind === 'error' ? 'plan-message error' : 'plan-message notice';
  return `<div class="${className}"><strong>${kind === 'error' ? 'Blocking errors' : 'Warnings'}</strong><ul>${messages.map((message) => `
    <li>${message.file_name ? `<span class="mono">${html(message.file_name)}</span>: ` : ''}${html(message.message || message.code)}</li>`).join('')}</ul></div>`;
}

function renderPlanRom(rom) {
  const generatedFiles = (rom.files || []).filter((file) => file.metadata?.source === 'generated');
  return `<article class="plan-rom-card">
    <div class="plan-rom-heading">
      <div class="plan-title-block">
        <label class="plan-title-edit"><span>Game title</span><input data-planned-title="${attr(rom.plan_id)}" value="${attr(rom.title)}" required /></label>
        <small class="mono">Storage slug: <span data-planned-slug="${attr(rom.plan_id)}">${html(rom.slug)}</span>${rom.regions?.length ? ` · ${html(csv(rom.regions))}` : ''}</small>
      </div>
      <div class="chips"><span class="chip">${html(rom.groups?.length || 0)} groups</span><span class="chip">${html(rom.files?.length || 0)} files</span>${generatedFiles.length ? '<span class="chip generated-chip">generated m3u</span>' : ''}</div>
    </div>
    <div class="plan-groups">${(rom.groups || []).map((group) => renderPlanGroup(group, rom)).join('')}</div>
  </article>`;
}

function renderPlanGroup(group, rom) {
  const files = (rom.files || []).filter((file) => file.group_key === group.key);
  const discLabel = group.disc_index ? `Disc ${group.disc_index}${group.disc_count ? ` of ${group.disc_count}` : ''}` : group.disc_count ? `${group.disc_count} discs` : '';
  const titleAttribute = group.display_name === rom.title ? ` data-planned-group-title="${attr(rom.plan_id)}"` : '';
  return `<details class="plan-group">
    <summary><span class="plan-group-summary"><strong${titleAttribute}>${html(group.display_name)}</strong><span class="chips"><span class="chip">${html(group.kind)}</span>${discLabel ? `<span class="chip">${html(discLabel)}</span>` : ''}<span class="chip">${html(files.length)} files</span></span></span></summary>
    <div class="plan-file-table">${files.map(renderPlanFile).join('') || '<small class="muted">No files in this group.</small>'}</div>
  </details>`;
}

function renderPlanFile(file) {
  const generated = file.metadata?.source === 'generated';
  return `<div class="plan-file-row ${generated ? 'generated' : ''}">
    <span title="${attr(file.original_file_name)}"><strong>${html(file.original_file_name)}</strong><small>${html(formatBytes(file.file_size_bytes || 0))}</small></span>
    <span class="chip">${html(formatRole(file.role))}</span>
    ${file.launchable ? '<span class="chip launchable-chip">launch</span>' : '<span class="chip dependency-chip">dependency</span>'}
    ${generated ? '<span class="chip generated-chip">generated</span>' : ''}
  </div>`;
}

export function uploadSelectionLabel(files = state.uploadSelectedFiles) {
  if (!files.length) return 'No files selected';
  const totalBytes = files.reduce((total, file) => total + file.size, 0);
  if (files.length === 1) return `${files[0].name} · ${formatBytes(totalBytes)}`;
  return `${files.length} files selected · ${formatBytes(totalBytes)}`;
}

export function renderUploadFileList(files = state.uploadSelectedFiles) {
  if (!files.length) return '';
  return `<ul data-scroll-key="upload-file-list">${files.map((file, index) => `
    <li><span title="${attr(file.name)}">${html(file.name)}</span><small>${html(formatBytes(file.size))}</small><button class="ghost upload-file-remove" type="button" data-remove-upload-file="${index}" aria-label="Remove ${attr(file.name)} from upload queue">Remove</button></li>`).join('')}</ul>`;
}
