import { attr, html } from '../dom.js';
import { state } from '../state.js';

// Every remote string below is untrusted: it comes from a server Teatro does not control. It is
// escaped through the shared helpers and rendered at a bounded length.
const MAX_DISPLAY_CHARACTERS = 160;

export function bounded(value) {
  const text = String(value ?? '');
  return text.length > MAX_DISPLAY_CHARACTERS
    ? `${text.slice(0, MAX_DISPLAY_CHARACTERS)}…`
    : text;
}

export function formatTimestamp(value) {
  if (!value) return null;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? bounded(value) : parsed.toLocaleString();
}

/// A saved source shows what Teatro stored instead of its inputs. The secret is never part of the
/// status payload, so only its presence is reported.
function renderConnectionSummary() {
  const source = state.rommSource;
  const saved = formatTimestamp(source.updated_at);
  return `
    <dl class="romm-connection">
      <div><dt>Base URL</dt><dd><code>${html(bounded(source.base_url || ''))}</code></dd></div>
      <div><dt>Username</dt><dd>${html(bounded(source.username || '—'))}</dd></div>
      <div><dt>Authentication</dt><dd>${source.auth_mode === 'basic' ? 'HTTP Basic' : 'OAuth2 token'}</dd></div>
      <div><dt>Password or API key</dt><dd>${source.secret_configured ? 'Stored, hidden' : 'Not stored'}</dd></div>
      <div><dt>Transport</dt><dd>${source.plaintext_http ? 'HTTP, not encrypted' : 'HTTPS'}</dd></div>
      ${saved ? `<div><dt>Saved</dt><dd>${html(saved)}</dd></div>` : ''}
    </dl>
    <div class="form-footer">
      <button class="primary" type="button" data-nav="romm">Browse RomM</button>
      <button type="button" data-action="test-romm-source">Test connection</button>
      <button type="button" data-action="edit-romm-source">Change connection</button>
      <button class="danger" type="button" data-action="clear-romm-source">Remove source</button>
    </div>`;
}

function renderSettingsForm() {
  const source = state.rommSource;
  const configured = Boolean(source?.configured);
  const secretPlaceholder = source?.secret_configured
    ? 'Leave blank to keep the stored secret'
    : 'RomM password or API key';
  return `
    <form id="romm-source-settings-form">
      <div class="form-grid">
        <label><span>RomM base URL</span><input name="base_url" value="${attr(source?.base_url || '')}" placeholder="http://192.168.1.10:8080" required /></label>
        <label><span>Username</span><input name="username" value="${attr(source?.username || '')}" placeholder="RomM username" autocomplete="off" required /></label>
        <label><span>Password or API key</span><input name="secret" type="password" placeholder="${attr(secretPlaceholder)}" ${source?.secret_configured ? '' : 'required'} autocomplete="new-password" /></label>
        <label><span>Authentication</span><select name="auth_mode">
          <option value="token" ${source?.auth_mode === 'basic' ? '' : 'selected'}>OAuth2 token (default)</option>
          <option value="basic" ${source?.auth_mode === 'basic' ? 'selected' : ''}>HTTP Basic</option>
        </select></label>
        <label class="checkbox wide"><input type="checkbox" name="acknowledge_plaintext_http" /> <span>I understand that an <code>http://</code> RomM server carries credentials and ROM bytes across the network unencrypted.</span></label>
        <p class="hint wide">The secret cannot be viewed after saving and is never logged. It is stored in the database. Keep backups private.</p>
      </div>
      <div class="form-footer">
        <button class="primary" type="submit">Save connection</button>
        ${configured ? '<button type="button" data-action="cancel-romm-source-edit">Cancel</button>' : ''}
        ${!configured && source?.base_url ? '<button class="danger" type="button" data-action="clear-romm-source">Remove source</button>' : ''}
      </div>
    </form>`;
}

export function renderConnectionStatus() {
  const source = state.rommSource;
  if (!source) return '';
  const test = source.lastTest;
  const parts = [];
  if (source.plaintext_http) {
    parts.push('<div class="warning">This connection uses HTTP. Credentials and game files travel unencrypted.</div>');
  }
  if (test) {
    const tone = test.result === 'reachable' ? 'notice' : test.result === 'unauthorized' ? 'warning' : 'error';
    parts.push(`<div class="${tone}">${html(bounded(test.message))}</div>`);
  }
  return parts.join('');
}

/// The Settings card owns the connection only. Browsing and importing live on their own screen so
/// the routine act — picking games — is never nested inside a credential form.
export function renderRommSource() {
  const source = state.rommSource;
  if (!source) {
    return '<p class="muted">The RomM source is disabled on this server. Set <code>TEATRO_ROMM_SOURCE_ENABLED=true</code> to enable it.</p>';
  }

  const editing = !source.configured || state.rommSourceEditing;
  return `
    ${renderConnectionStatus()}
    ${editing ? renderSettingsForm() : renderConnectionSummary()}
    ${source.configured || editing ? '' : '<p class="hint">Save a base URL, username, and secret to browse the remote library.</p>'}`;
}
