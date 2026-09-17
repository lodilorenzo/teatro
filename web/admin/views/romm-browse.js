import { icon } from '../../public/icons.js';
import { attr, formatBytes, html } from '../dom.js';
import {
  findRommSelection, rommSelectionPlatformId, rommSelectionTotals, state, visibleRommItems,
} from '../state.js';
import { bounded, formatTimestamp, renderConnectionStatus } from './romm-source.js';
import { renderPlatformIcon } from './shared.js';

const COVER_PLACEHOLDER_PATH = '/admin/game-cover-placeholder.jpg';

/// The connection is one line. Its stored detail stays behind a disclosure because configuring a
/// server is rare and browsing is not.
function renderConnectionStrip() {
  const source = state.rommSource;
  const platforms = source.index_refreshed_at
    ? Number(source.index_platform_count || 0)
    : state.rommPlatforms.length;
  const remoteTotal = source.index_refreshed_at
    ? Number(source.index_game_count || 0)
    : (state.rommBrowse.loaded ? state.rommBrowse.total : null);
  const facts = source.lastTest?.result === 'reachable' ? [
    remoteTotal === null ? null : `${remoteTotal} game${remoteTotal === 1 ? '' : 's'}`,
    platforms ? `${platforms} platform${platforms === 1 ? '' : 's'}` : null,
  ].filter(Boolean).join(' · ') : '';
  const saved = formatTimestamp(source.updated_at);
  const indexed = formatTimestamp(source.index_refreshed_at);
  const connection = {
    reachable: 'Connected', unauthorized: 'Authentication failed', unreachable: 'Connection failed',
  }[source.lastTest?.result] || 'Not checked';
  return `
    <div class="romm-status">
      <span class="romm-status-dot ${source.plaintext_http ? 'insecure' : ''}" aria-hidden="true"></span>
      <span class="romm-status-host mono">${html(bounded(source.base_url || ''))}</span>
      <span class="romm-status-meta">${connection}${source.plaintext_http ? ' · Plaintext HTTP' : ''}${facts ? ` · ${html(facts)}` : ''}</span>
      <span class="romm-status-spacer"></span>
      <button class="ghost" type="button" data-action="romm-toggle-connection" aria-expanded="${state.rommSourceDetailOpen ? 'true' : 'false'}" aria-controls="romm-connection-detail">Connection details</button>
    </div>
    <div class="romm-connection-detail" id="romm-connection-detail" ${state.rommSourceDetailOpen ? '' : 'hidden'}>
      ${renderConnectionStatus()}
      <dl class="romm-connection">
        <div><dt>Base URL</dt><dd><code>${html(bounded(source.base_url || ''))}</code></dd></div>
        <div><dt>Username</dt><dd>${html(bounded(source.username || '—'))}</dd></div>
        <div><dt>Authentication</dt><dd>${source.auth_mode === 'basic' ? 'HTTP Basic' : 'OAuth2 token'}</dd></div>
        <div><dt>Password or API key</dt><dd>${source.secret_configured ? 'Stored, hidden' : 'Not stored'}</dd></div>
        <div><dt>Transport</dt><dd>${source.plaintext_http ? 'HTTP, not encrypted' : 'HTTPS'}</dd></div>
        ${saved ? `<div><dt>Saved</dt><dd>${html(saved)}</dd></div>` : ''}
        ${indexed ? `<div><dt>List updated</dt><dd>${html(indexed)}</dd></div>` : '<div><dt>Remote index</dt><dd>Not downloaded yet</dd></div>'}
      </dl>
      <div class="form-footer">
        <button type="button" data-action="test-romm-source">Test connection</button>
        <button type="button" data-nav="settings">Change connection in Settings</button>
      </div>
    </div>`;
}

/// One dropdown instead of a rail: the remote platform list runs to hundreds of entries, and a
/// filter should not outgrow the list it filters.
function renderPlatformFilter() {
  const active = String(state.rommBrowse.platformId || '');
  const options = state.rommPlatforms.map((platform) => `
    <option value="${attr(platform.id)}" ${String(platform.id) === active ? 'selected' : ''}>
      ${html(bounded(platform.name || platform.slug || 'Unknown platform'))} (${html(platform.rom_count)})
    </option>`).join('');
  return `
    <label class="romm-platform-filter">
      <span class="field-label">Remote platform</span>
      <select data-romm-platform ${state.rommPlatforms.length ? '' : 'disabled'}>
        <option value="" ${active ? '' : 'selected'}>All platforms</option>
        ${options}
      </select>
    </label>`;
}

/// The cover is loaded later by the authenticated hydrator, keyed by the remote game id rather
/// than by any URL the remote server supplied. A game without a remote cover keeps the
/// placeholder, so every row in the list has the same shape.
function renderRemoteCover(rom) {
  const hydrate = rom.has_cover && Number.isSafeInteger(rom.id)
    ? ` data-romm-cover="${attr(rom.id)}"`
    : '';
  return `<img class="romm-cover" src="${COVER_PLACEHOLDER_PATH}" alt="" loading="lazy"${hydrate} />`;
}

function renderCard(rom) {
  const entry = findRommSelection(rom.id);
  const selected = Boolean(entry);
  const platformId = entry
    ? rommSelectionPlatformId(entry)
    : (rom.target_platform_id ? String(rom.target_platform_id) : '');
  const destinationPlatform = state.platforms.find(({ id }) => String(id) === platformId);
  const platformName = destinationPlatform?.display_name
    || (entry?.platformOverride ? '' : String(rom.target_platform_name || ''));
  const platformIcon = destinationPlatform ? renderPlatformIcon(destinationPlatform, true) : '';
  const title = entry?.titleOverride || bounded(rom.name);
  const sourcePlatform = bounded(rom.platform_name || rom.platform_slug || 'Unknown platform');
  const size = rom.file_size_bytes ? formatBytes(rom.file_size_bytes) : 'Unknown';
  const fileCount = entry?.fileIds ? entry.fileIds.length : Number(rom.file_count || 0);

  const target = rom.already_present ? '' : `
    <span class="romm-card-fact romm-card-target ${platformId ? '' : 'needs-pick'}">
      <small>Destination</small>
      <strong>${platformIcon ? `<span class="romm-destination-icon" aria-hidden="true">${platformIcon}</span>` : ''}${platformId ? html(bounded(platformName) || 'Chosen platform') : 'Choose a platform'}</strong>
    </span>`;
  const action = rom.already_present
    ? `<span class="romm-badge in-library" title="${attr(bounded(rom.already_present_reason || ''))}">In Teatro</span>`
    : `<button type="button" class="romm-edit-button" data-romm-drawer="${attr(rom.id)}" aria-label="Edit title, platform, and files for ${attr(title)}" title="Edit title, platform, and files">${icon('pencil')}</button>`;

  return `
    <article class="romm-card ${selected ? 'selected' : ''} ${rom.already_present ? 'already' : ''}">
      <button type="button" class="romm-card-open" data-romm-toggle="${attr(rom.id)}" aria-pressed="${selected}" ${rom.already_present ? 'disabled' : ''}>
        <span class="romm-check" aria-hidden="true">✓</span>
        ${renderRemoteCover(rom)}
        <span class="romm-card-copy">
          <strong class="romm-card-title">${html(title)}</strong>
          <span class="romm-card-details">
            <span class="romm-card-fact"><small>RomM platform</small><strong>${html(sourcePlatform)}</strong></span>
            <span class="romm-card-fact"><small>Files</small><strong>${html(fileCount)}</strong></span>
            <span class="romm-card-fact"><small>Size</small><strong>${html(size)}</strong></span>
            ${target}
          </span>
        </span>
      </button>
      <span class="romm-card-actions">${action}</span>
    </article>`;
}

function renderList() {
  const browse = state.rommBrowse;
  const connection = state.rommSource?.lastTest;
  if (browse.refreshing) return '<p class="muted">Updating the RomM game list…</p>';
  if (browse.loading && !connection) return '<p class="muted">Checking the RomM connection…</p>';
  if (connection && connection.result !== 'reachable') {
    return `<div class="empty-state"><div>
      <h3>RomM connection failed</h3>
      <p>${html(bounded(connection.message || 'The configured RomM server did not respond.'))}</p>
      <div class="actions romm-empty-actions"><button type="button" data-action="test-romm-source">Test connection</button></div>
    </div></div>`;
  }
  if (browse.loading) return '<p class="muted">Loading RomM games…</p>';
  if (state.rommSource?.index_refreshed_at === null) {
    return '<div class="empty-state"><div><h3>No saved game list</h3><p>Refresh remote list to load games from RomM.</p></div></div>';
  }
  const items = visibleRommItems();
  if (!items.length) {
    const hidden = browse.items.length - items.length;
    return `<div class="empty-state"><div>
      <h3>No remote games</h3>
      <p>${browse.items.length
    ? `${hidden} browsed game${hidden === 1 ? ' is' : 's are'} already in Teatro and hidden by the filter above.`
    : 'The RomM server returned no games for this filter.'}</p>
    </div></div>`;
  }
  return `<div class="romm-list">${items.map(renderCard).join('')}</div>`;
}

function renderActionBar() {
  if (!state.rommSelected.length) return '';
  const totals = rommSelectionTotals();
  const detail = [
    `${totals.files} file${totals.files === 1 ? '' : 's'}`,
    totals.bytes ? formatBytes(totals.bytes) : null,
    totals.platforms ? `${totals.platforms} platform${totals.platforms === 1 ? '' : 's'}` : null,
  ].filter(Boolean).join(' · ');
  return `
    <div class="romm-actionbar" role="region" aria-label="Selected imports">
      <div class="romm-actionbar-summary">
        <strong>${html(totals.games)} game${totals.games === 1 ? '' : 's'} selected</strong>
        <small>${html(detail)}</small>
      </div>
      ${totals.unmatched ? `<p class="romm-warn">${icon('triangle-alert')}<span>${html(totals.unmatched)} game${totals.unmatched === 1 ? ' needs' : 's need'} a target platform</span></p>` : ''}
      <span class="romm-status-spacer"></span>
      <button class="ghost" type="button" data-action="romm-clear-selection">Cancel</button>
      <button class="primary" type="button" data-action="romm-import" ${totals.unmatched ? 'disabled' : ''}>Import ${html(totals.games)} game${totals.games === 1 ? '' : 's'}</button>
    </div>`;
}

function renderDrawerFiles() {
  const drawer = state.rommDrawer;
  if (!drawer.files.length) return '<p class="muted">The remote server published no files for this game.</p>';
  return `<ul class="romm-filelist">${drawer.files.map((file) => `
    <li>
      <input type="checkbox" data-romm-file="${attr(file.id)}" ${drawer.fileIds?.includes(file.id) ? 'checked' : ''} aria-label="${attr(bounded(file.file_name))}" />
      <span>
        <code>${html(bounded(file.file_name))}</code>
        <small>${file.file_size_bytes ? `${html(formatBytes(file.file_size_bytes))} · ` : ''}${file.hash_signals?.length ? `Remote hashes: ${html(file.hash_signals.join(', '))}` : 'The remote server published no hash for this file.'}</small>
      </span>
    </li>`).join('')}</ul>`;
}

function renderDrawer() {
  const drawer = state.rommDrawer;
  if (!drawer) return '';
  const platformOptions = state.platforms.map((platform) => `
    <option value="${attr(platform.id)}" ${String(drawer.platformId) === String(platform.id) ? 'selected' : ''}>
      ${html(platform.display_name || platform.slug)}
    </option>`).join('');
  const blocked = drawer.rom?.already_present;
  return `
    <div class="romm-drawer-backdrop" data-action="romm-close-drawer"></div>
    <aside class="romm-drawer" role="dialog" aria-modal="true" aria-labelledby="romm-drawer-title">
      <div class="romm-drawer-head">
        <div>
          <p class="eyebrow">Import options</p>
          <h3 id="romm-drawer-title">${html(drawer.title || bounded(drawer.rom?.name || 'Remote game'))}</h3>
        </div>
        <button class="ghost" type="button" data-action="romm-close-drawer" aria-label="Close">✕</button>
      </div>
      <div class="romm-drawer-body" data-scroll-key="romm-drawer">
        ${drawer.loading ? '<p class="muted">Loading remote files…</p>' : `
          ${blocked ? `<div class="warning">${html(bounded(drawer.rom.already_present_reason || 'This game already exists in Teatro.'))} Teatro refuses duplicate imports; delete the local game first if you want to re-import it.</div>` : ''}
          <label><span>Title in Teatro</span><input data-romm-title value="${attr(drawer.title)}" /></label>
          <label><span>Destination platform</span><select data-romm-target-platform>
            <option value="">${drawer.platformId ? 'Choose a platform' : 'No match. Choose a platform'}</option>
            ${platformOptions}
          </select></label>
          <div class="romm-drawer-files">
            <p class="field-label">Files</p>
            ${renderDrawerFiles()}
          </div>`}
      </div>
      <div class="romm-drawer-foot">
        <button class="ghost" type="button" data-action="romm-close-drawer">Cancel</button>
        <button class="primary" type="button" data-action="romm-commit-drawer" ${drawer.loading || blocked || !drawer.platformId || !drawer.fileIds?.length ? 'disabled' : ''}>Keep changes</button>
      </div>
    </aside>`;
}

export function renderRommBrowse() {
  const source = state.rommSource;
  if (!source?.configured) {
    return `
      <section class="page-header">
        <div>
          <h1>RomM</h1>
        </div>
      </section>
      <div class="empty-state"><div>
        <h3>No RomM server configured</h3>
        <p>${source ? 'Save a base URL, username, and secret in Settings to browse a remote library.' : 'The RomM source is disabled on this server. Set <code>TEATRO_ROMM_SOURCE_ENABLED=true</code> to enable it.'}</p>
        ${source ? '<div class="actions romm-empty-actions"><button class="primary" type="button" data-nav="settings">Open Settings</button></div>' : ''}
      </div></div>`;
  }

  return `
    <section class="page-header">
      <div>
        <h1>RomM</h1>
      </div>
      <div class="actions">
        <button class="ghost icon-label-button" type="button" data-action="romm-refresh" ${state.rommBrowse.refreshing ? 'disabled' : ''}>${icon('refresh-cw')} ${state.rommBrowse.refreshing ? 'Refreshing list…' : 'Refresh remote list'}</button>
      </div>
    </section>
    ${renderConnectionStrip()}
    <p class="hint">Browsing a saved list. Refresh remote list to check for changes. Imports leave the remote library unchanged.</p>
    ${source.lastTest && source.lastTest.result !== 'reachable' ? renderList() : `
      <div class="romm-searchbar">
        <label class="romm-search">
          <span class="field-label">Search games</span>
          <input id="romm-search" type="search" value="${attr(state.rommBrowse.search)}" placeholder="Game title" autocomplete="off" />
        </label>
        ${renderPlatformFilter()}
      </div>
      <div class="romm-listhead">
        <span>${html(visibleRommItems().length)} shown${state.rommSelected.length ? ` · ${html(state.rommSelected.length)} selected` : ''}</span>
        <span class="actions">
          <button class="ghost" type="button" data-action="romm-select-shown">Select all shown</button>
          <button class="ghost" type="button" data-action="romm-clear-selection" ${state.rommSelected.length ? '' : 'disabled'}>Clear selection</button>
          <label class="romm-toggle">
            <input type="checkbox" data-action="romm-hide-existing" ${state.rommHideExisting ? 'checked' : ''} />
            Hide games already in Teatro
          </label>
        </span>
      </div>
      ${renderList()}
      ${renderActionBar()}
      ${renderDrawer()}`}`;
}
