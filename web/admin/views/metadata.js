import { attr, coverPath, html } from '../dom.js';
import { state } from '../state.js';

export function renderIgdbStatus(includeCredentials = false) {
  const status = state.igdbSettings || state.igdbStatus;
  if (!status) return '<p class="muted">IGDB status unavailable.</p>';
  return `<dl class="status-list">
    <div><dt>Status</dt><dd>${status.configured ? 'Configured' : 'Not configured'}</dd></div>
    <div><dt>Cached token</dt><dd>${status.token_cached ? 'Available' : 'None'}</dd></div>
    <div><dt>Client ID</dt><dd>${status.client_id_configured ? 'Set' : 'Not set'}</dd></div>
    <div><dt>Client secret</dt><dd>${status.client_secret_configured ? `Set via ${html(status.client_secret_source || 'configuration')}` : 'Missing'}</dd></div>
  </dl>
  ${includeCredentials ? `<div class="settings-subsection">${renderIgdbSettingsForm()}</div>` : ''}`;
}

function renderIgdbSettingsForm() {
  const settings = state.igdbSettings;
  if (settings?.configured) {
    return `
      <div class="credential-status">
        <p class="notice">Client ID is set.</p>
        <div class="form-footer"><button class="danger" type="button" data-action="clear-igdb-credentials">Clear credentials</button></div>
      </div>`;
  }

  const secretConfigured = Boolean(settings?.client_secret_configured);
  const secretPlaceholder = secretConfigured
    ? `Leave blank to keep existing ${settings?.client_secret_source || 'configured'} secret`
    : 'Enter Twitch/IGDB client secret';
  return `
    <form id="igdb-settings-form">
      <div class="form-grid">
        <label><span>Client ID</span><input name="client_id" placeholder="Twitch application client ID" required /></label>
        <label><span>Client secret</span><input name="client_secret" type="password" placeholder="${attr(secretPlaceholder)}" ${secretConfigured ? '' : 'required'} autocomplete="new-password" /></label>
        <p class="hint wide">The secret cannot be viewed after saving. It is stored in the database. Keep backups private.</p>
      </div>
      <div class="form-footer"><button class="primary" type="submit">Save IGDB credentials</button></div>
    </form>`;
}

export function renderIgdbSearch(rom) {
  const disabled = !state.igdbStatus?.configured;
  const detectedTitle = rom.metadatum?.filename?.clean_title;
  const query = !coverPath(rom) && typeof detectedTitle === 'string' && detectedTitle.trim()
    ? detectedTitle.trim()
    : rom.name;
  return `
    ${disabled ? '<div class="notice">IGDB credentials are not configured on the server.</div>' : ''}
    <form id="igdb-search-form" data-rom-id="${rom.id}" class="filters">
      <label><span>Game title</span><input name="q" value="${attr(query)}" ${disabled ? 'disabled' : ''} /></label>
      <label><span>Max results</span><input name="limit" type="number" min="1" max="25" value="10" ${disabled ? 'disabled' : ''} /></label>
      <button type="submit" ${disabled ? 'disabled' : ''}>Search platform</button>
      <button type="submit" data-search-all-platforms="true" ${disabled ? 'disabled' : ''}>Search all platforms</button>
    </form>
    <p id="igdb-feedback" class="${state.igdbFeedback?.error ? 'error' : 'muted'}" role="${state.igdbFeedback?.error ? 'alert' : 'status'}">${html(state.igdbFeedback?.message || '')}</p>
    ${renderIgdbResults(rom.id)}`;
}

function renderIgdbResults(romId) {
  if (!state.igdbResults.length) return '';
  return `<div class="candidate-list">${state.igdbResults.map((candidate, index) => `
    <div class="candidate">
      ${renderIgdbCandidateCover(candidate)}
      <div>
        <strong>${html(candidate.name)} ${candidate.release_year ? `<span class="muted">(${html(candidate.release_year)})</span>` : ''}</strong>
        <p>${html(candidate.summary || candidate.storyline || 'No summary returned.')}</p>
        <div class="chips">${[...(candidate.genres || []), ...(candidate.developers || [])].slice(0, 8).map((item) => `<span class="chip">${html(item)}</span>`).join('')}</div>
      </div>
      <button class="primary" data-apply-igdb="${index}" data-rom-id="${romId}">Use match</button>
    </div>`).join('')}</div>`;
}

function renderIgdbCandidateCover(candidate) {
  if (!candidate.cover_url) return '<div class="candidate-cover cover-placeholder">No cover</div>';
  return `<img class="candidate-cover" src="${attr(candidate.cover_url)}" alt="Cover preview for ${attr(candidate.name)}" loading="lazy" referrerpolicy="no-referrer" />`;
}
