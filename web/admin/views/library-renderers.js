import {
  attr, coverPath, csv, formatBytes, formatRole, html, metadataList, metadataText,
} from '../dom.js';
import { state } from '../state.js';
import { renderIgdbSearch } from './metadata.js';
import { platformSelect, renderPlatformIcon } from './shared.js';

const COVER_PLACEHOLDER_PATH = '/admin/game-cover-placeholder.jpg';

export function renderRomTable(roms) {
  if (!roms.length) return '<div class="empty-state">No games match this view.</div>';
  return `<div class="table-wrap"><table><thead><tr><th>Cover</th><th>Title</th><th>Platform</th><th>Size</th></tr></thead><tbody>${roms.map((rom) => {
    const active = state.selectedRom?.id === rom.id;
    return `<tr class="rom-row ${active ? 'active' : ''}">
      <td>${renderCoverThumb(rom)}</td>
      <td><button class="link-button" data-rom-id="${rom.id}"><strong>${html(rom.name)}</strong></button><br><small class="mono">${html(rom.fs_name || rom.slug)}</small></td>
      <td>${renderPlatformIcon(rom)}</td>
      <td>${formatBytes(rom.fs_size_bytes || 0)}</td>
    </tr>`;
  }).join('')}</tbody></table></div>`;
}

export function renderRomGrid(roms) {
  if (!roms.length) return '<div class="empty-state">No games match this view.</div>';
  return `<div class="rom-grid">${roms.map((rom) => {
    const active = state.selectedRom?.id === rom.id;
    return `<button type="button" class="rom-grid-card ${active ? 'active' : ''}" data-rom-id="${rom.id}">
      <span class="rom-grid-cover">${renderCoverThumb(rom, 'large')}${renderPlatformIcon(rom, true)}</span>
      <span><strong>${html(rom.name)}</strong><small>${html(rom.platform_display_name)}</small><small class="mono">${formatBytes(rom.fs_size_bytes || 0)}</small></span>
    </button>`;
  }).join('')}</div>`;
}

export function renderCoverThumb(rom, size = 'small') {
  const path = coverPath(rom, size);
  const coverData = path ? ` data-cover-path="${attr(path)}"` : '';
  return `<img class="cover-thumb" src="${COVER_PLACEHOLDER_PATH}" alt="" loading="lazy"${coverData} />`;
}

export function renderRomDetail(rom) {
  const genres = metadataList(rom, 'genres');
  const developers = metadataList(rom, 'developers');
  const publishers = metadataList(rom, 'publishers');
  const releaseYear = metadataText(rom, 'release_year');
  return `
    <div class="detail-layout">
      <div class="detail-media">
        <div class="detail-cover">${renderCoverLarge(rom)}${renderPlatformIcon(rom, true)}</div>
        ${renderPackageDownload(rom)}
      </div>
      <div class="detail-body">
        <div class="detail-heading">
          <div><h3 id="library-detail-title">${html(rom.name)}</h3></div>
          <button class="primary" type="button" data-action="open-rom-edit">Edit details</button>
        </div>
        <div class="summary-text muted">${html(rom.summary || 'No summary has been saved yet.')}</div>
        ${renderRomMetadataFields(rom, { genres, developers, publishers, releaseYear })}
      </div>
    </div>
    <section class="detail-section"><h3>Files</h3>${renderRomFileGroups(state.selectedRomFiles, rom)}</section>
    <section class="detail-section">
      <form id="delete-rom-form" data-rom-id="${rom.id}" class="form-grid">
        <p class="warning wide">Deleting this game permanently removes its managed files from disk.</p>
        <div class="form-footer wide"><button class="danger" type="submit">Delete game and files</button></div>
      </form>
    </section>`;
}

function renderPackageDownload(rom) {
  const files = rom.files || [];
  if (!files.length) return '<button class="package-download" type="button" disabled>Package unavailable</button>';
  if (files.length === 1) {
    const file = files[0];
    return `<button class="primary package-download" type="button" data-download-rom="${rom.id}" data-file-id="${file.id}">Download</button>`;
  }
  return `<button class="primary package-download" type="button" data-download-package="${rom.id}">Download</button>`;
}

function renderRomFileGroups(groupDetails, rom) {
  if (!groupDetails) return '<small class="muted">Grouped file details are loading or unavailable.</small>';
  const filesById = new Map();
  for (const group of groupDetails.groups || []) {
    for (const file of group.files || []) filesById.set(file.id, file);
  }
  for (const file of groupDetails.ungrouped_files || []) filesById.set(file.id, file);

  const dependencies = groupDetails.dependencies || [];
  return `<div class="rom-file-groups">
    ${(groupDetails.warnings || []).length ? `<div class="notice">${groupDetails.warnings.map(html).join('<br>')}</div>` : ''}
    ${(groupDetails.groups || []).map((group) => renderRomFileGroup(group, rom)).join('') || '<div class="empty-state">No file groups.</div>'}
    ${(groupDetails.ungrouped_files || []).length ? `<div class="file-group-card"><div class="file-group-heading"><strong>Ungrouped files</strong></div>${groupDetails.ungrouped_files.map((file) => renderAdminFileRow(file, rom)).join('')}</div>` : ''}
    ${dependencies.length ? `<details class="dependency-details"><summary>${html(dependencies.length)} file dependencies</summary><ul class="dependency-list">${dependencies.map((dependency) => `
      <li><span class="mono">${html(filesById.get(dependency.parent_file_id)?.file_name || dependency.parent_file_id)}</span> → <span class="mono">${html(filesById.get(dependency.child_file_id)?.file_name || dependency.child_file_id)}</span> <span class="chip">${html(dependency.dependency_kind)}</span></li>`).join('')}</ul></details>` : ''}
  </div>`;
}

function renderRomFileGroup(group, rom) {
  return `<article class="file-group-card">
    <div class="admin-file-list">${(group.files || []).map((file) => renderAdminFileRow(file, rom)).join('') || '<small class="muted">No files in this group.</small>'}</div>
  </article>`;
}

function renderAdminFileRow(file, rom) {
  return `<div class="admin-file-row">
    <span title="${attr(file.relative_path)}"><strong>${html(file.file_name)}</strong><small>${html(file.original_file_name || file.relative_path)} · ${html(formatBytes(file.file_size_bytes))}</small></span>
    <span class="chip">${html(formatRole(file.role))}</span>
    ${file.launchable ? '<span class="chip launchable-chip">launch</span>' : '<span class="chip dependency-chip">dependency</span>'}
    <button data-download-rom="${rom.id}" data-file-id="${file.id}">Download</button>
  </div>`;
}

function renderRomMetadataFields(rom, metadata) {
  const fields = [
    ['Platform', [rom.platform_display_name, rom.platform_slug ? `(${rom.platform_slug})` : ''].filter(Boolean).join(' ')],
    ['Release year', metadata.releaseYear],
    ['Regions', csv(rom.regions || [])],
    ['Genres', csv(metadata.genres)],
    ['Developers', csv(metadata.developers)],
    ['Publishers', csv(metadata.publishers)],
  ];
  return `<dl class="detail-metadata">${fields.map(([label, value]) => `<div><dt>${html(label)}</dt><dd>${html(value || '—')}</dd></div>`).join('')}</dl>`;
}

export function renderRomEditForm() {
  const rom = state.editingRomId && String(state.selectedRom?.id) === String(state.editingRomId)
    ? state.selectedRom : null;
  if (!rom) return '';
  const genres = metadataList(rom, 'genres');
  const developers = metadataList(rom, 'developers');
  const publishers = metadataList(rom, 'publishers');
  const releaseYear = metadataText(rom, 'release_year');
  return `
    <div class="detail-heading">
      <div><h3 id="library-detail-title">${html(rom.name)}</h3></div>
    </div>
    <form id="rom-edit-form" data-rom-id="${rom.id}">
      <div class="form-grid">
        <label><span>Title</span><input name="name" value="${attr(rom.name)}" required autofocus /></label>
        <label><span>Platform</span>${platformSelect('platform_id', rom.platform_id, false)}</label>
        <label class="wide"><span>Cover image</span><input name="cover" type="file" accept="image/jpeg,image/png,image/webp" /><small>JPEG, PNG, or WebP; up to 10 MiB.</small></label>
        <label class="wide"><span>Summary</span><textarea name="summary" placeholder="Short description">${html(rom.summary || '')}</textarea></label>
        <label><span>Regions</span><input name="regions" value="${attr(csv(rom.regions || []))}" placeholder="World, USA" /></label>
        <label><span>Release year</span><input name="release_year" type="number" min="0" max="9999" value="${attr(releaseYear)}" /></label>
        <label><span>Genres</span><input name="genres" value="${attr(csv(genres))}" placeholder="Action, Platform" /></label>
        <label><span>Developers</span><input name="developers" value="${attr(csv(developers))}" placeholder="Studio" /></label>
        <label><span>Publishers</span><input name="publishers" value="${attr(csv(publishers))}" placeholder="Publisher" /></label>
      </div>
      <div class="form-footer"><button type="button" data-action="close-rom-edit">Cancel</button><button class="primary" type="submit">Save details</button></div>
    </form>
    <section class="settings-subsection"><h3>IGDB match</h3>${renderIgdbSearch(rom)}</section>`;
}

export function renderCoverLarge(rom) {
  const path = rom.path_cover_large || rom.path_cover_small;
  const altText = path ? `Cover for ${rom.name}` : `No cover available for ${rom.name}`;
  const coverData = path ? ` data-cover-path="${attr(path)}"` : '';
  return `<img class="cover-large" src="${COVER_PLACEHOLDER_PATH}" alt="${attr(altText)}" loading="lazy"${coverData} />`;
}
