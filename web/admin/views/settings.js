import { MAX_ROM_PAGE_SIZE, state } from '../state.js';
import { renderIgdbStatus } from './metadata.js';
import { renderRommSource } from './romm-source.js';
import { renderServerCleanup } from './server-cleanup.js';

export function renderSettings() {
  return `
    <section class="page-header">
      <div>
        <h1>Settings</h1>
      </div>
    </section>
    <section class="settings-bento">
      <div class="settings-bento-column">
        <div class="card">
          <h2 class="section-title">Library display</h2>
          <form id="library-display-settings-form">
            <label><span>Games per page</span><input name="rom_page_size" type="number" min="1" max="${MAX_ROM_PAGE_SIZE}" value="${state.romLimit}" aria-describedby="library-page-size-hint" required /></label>
            <p class="hint" id="library-page-size-hint">Applies to the admin Library in this browser.</p>
            <div class="form-footer"><button class="primary" type="submit">Save display</button></div>
          </form>
        </div>
        <div class="card">
          <h2 class="section-title">IGDB</h2>
          ${renderIgdbStatus(true)}
        </div>
      </div>
      <div class="settings-bento-column">
        <div class="card">
          <h2 class="section-title">RomM connection</h2>
          <p class="hint">Connect to a RomM server to copy games into Teatro. The remote library stays unchanged.</p>
          ${renderRommSource()}
        </div>
        <div class="card danger-card">
          <h2 class="section-title">Delete library content</h2>
          ${renderServerCleanup()}
        </div>
      </div>
    </section>`;
}
