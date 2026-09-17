import { icon } from '../../public/icons.js';
import { attr, formatBytes, html } from '../dom.js';
import { activeJobCount, state } from '../state.js';

export function visibleRoms() {
  return state.roms;
}

function navActivityLabel(screen) {
  const active = activeJobCount();
  if (screen === 'jobs' && active) return `${active} job${active === 1 ? '' : 's'} in progress`;
  return '';
}

export function sideNavButton(screen, iconName, label, meta = '') {
  const activityLabel = navActivityLabel(screen);
  const classes = [state.screen === screen ? 'active' : '', activityLabel ? 'has-activity' : '']
    .filter(Boolean)
    .join(' ');
  const activityAttributes = activityLabel
    ? ` aria-busy="true" aria-label="${attr(`${label}: ${activityLabel}`)}"`
    : '';
  const activity = activityLabel
    ? '<span class="nav-activity" aria-hidden="true"></span>'
    : '';
  const secondary = meta ? `<small>${html(meta)}</small>` : '';
  return `<button type="button" data-nav="${screen}" class="${classes}"${activityAttributes}><span class="nav-icon">${icon(iconName)}</span><span class="nav-label">${html(label)}</span>${secondary}${activity}</button>`;
}

export function renderNotificationStack() {
  if (!state.loading && !state.notifications.length) return '';

  const notifications = state.notifications.map((notification) => {
    const isError = notification.kind === 'error';
    return `<div class="sidebar-notification ${isError ? 'error' : 'notice'}" data-notification-id="${notification.id}" role="${isError ? 'alert' : 'status'}" aria-atomic="true">
      <span class="notification-icon">${icon(isError ? 'triangle-alert' : 'circle-check')}</span>
      <span>${html(notification.message)}</span>
    </div>`;
  }).join('');

  return `<div class="notification-stack" data-scroll-key="notifications" aria-label="Notifications">
    ${state.loading ? `<div class="sidebar-notification loading-notification notice" role="status"><span class="notification-icon">${icon('loader-circle')}</span><span>Refreshing library…</span></div>` : ''}
    ${notifications}
  </div>`;
}

export function statsSummary() {
  return state.stats ? `${state.stats.total_roms} game${state.stats.total_roms === 1 ? '' : 's'}` : 'Loading';
}

export function jobsSummary() {
  const active = activeJobCount();
  if (active) return `${active} active`;
  return state.jobs.length ? `${state.jobs.length} recent` : 'Idle';
}

export function rommSummary() {
  if (state.rommSelected.length) {
    return `${state.rommSelected.length} selected`;
  }
  return state.rommBrowse.loaded ? `${state.rommBrowse.total} remote` : 'Remote source';
}

export function statCard(label, value) {
  return `<div class="stat"><p class="stat-label">${html(label)}</p><p class="stat-value">${html(value)}</p></div>`;
}

export function renderRootTable(roots) {
  if (!roots.length) return '<div class="empty-state">No library folders configured.</div>';
  return `<div class="table-wrap"><table><thead><tr><th>Name</th><th>Files</th><th>Size</th><th>Writable</th></tr></thead><tbody>${roots.map((root) => `
    <tr><td><strong>${html(root.name)}</strong><br><small class="mono">${html(root.root_path)}</small></td><td>${html(root.file_count)}</td><td>${formatBytes(root.total_file_bytes)}</td><td>${root.writable ? 'Yes' : 'No'}</td></tr>`).join('')}</tbody></table></div>`;
}

export function renderPlatformStats(platforms) {
  if (!platforms.length) return '<div class="empty-state">No games yet.</div>';
  return `<div class="table-wrap"><table><thead><tr><th>Platform</th><th>Games</th><th>Files</th><th>Size</th></tr></thead><tbody>${platforms.map((platform) => `
    <tr><td><div class="platform-stat"><span aria-hidden="true">${renderPlatformIcon(platform)}</span><span><strong>${html(platform.display_name)}</strong><br><small class="mono">${html(platform.slug)}</small></span></div></td><td>${html(platform.rom_count)}</td><td>${html(platform.file_count)}</td><td>${formatBytes(platform.total_file_bytes)}</td></tr>`).join('')}</tbody></table></div>`;
}

export function renderPlatformIcon(platform, overlay = false) {
  const platformSlug = String(platform.platform_slug || platform.slug || '');
  if (!platformSlug) return '';
  const platformName = String(platform.platform_display_name || platform.display_name || platformSlug);
  const className = overlay ? ' rom-grid-platform-icon' : '';
  return `<img class="rom-platform-icon${className}" src="${attr(`/public/platform-icons/${encodeURIComponent(platformSlug)}.png`)}" alt="${overlay ? '' : attr(platformName)}" loading="lazy" />`;
}

export function platformSelect(name, selected, includeAll = false) {
  return platformSelectFromList(name, state.platforms, selected, includeAll);
}

export function platformPicker(name, selected) {
  const selectedPlatform = state.platforms.find((platform) => (
    String(selected) === String(platform.id) || String(selected) === platform.slug
  )) || state.platforms[0];
  if (!selectedPlatform) return '<div class="platform-picker disabled">No platforms available</div>';

  const option = (platform) => `
    <label class="platform-picker-option">
      <input type="radio" name="${attr(name)}" value="${attr(platform.id)}" ${platform === selectedPlatform ? 'checked' : ''} />
      <span class="platform-picker-icon" aria-hidden="true">${renderPlatformIcon(platform)}</span>
      <span><strong>${html(platform.display_name)}</strong><small class="mono">${html(platform.slug)}</small></span>
    </label>`;
  return `<details class="platform-picker">
    <summary>
      <span class="platform-picker-icon" aria-hidden="true">${renderPlatformIcon(selectedPlatform)}</span>
      <span><strong>${html(selectedPlatform.display_name)}</strong><small class="mono">${html(selectedPlatform.slug)}</small></span>
    </summary>
    <div class="platform-picker-menu" role="radiogroup" aria-label="Platform">${state.platforms.map(option).join('')}</div>
  </details>`;
}

export function platformSelectFromList(name, platforms, selected, includeAll = false) {
  const options = platforms.map((platform) => `
    <option value="${attr(platform.id)}" data-slug="${attr(platform.slug)}" ${String(selected) === String(platform.id) || String(selected) === platform.slug ? 'selected' : ''}>${html(platform.display_name)} (${html(platform.slug)})</option>`).join('');
  return `<select name="${attr(name)}" ${platforms.length ? '' : 'disabled'}>${includeAll ? '<option value="">All platforms</option>' : ''}${options}${platforms.length ? '' : '<option value="">No platforms with games</option>'}</select>`;
}
