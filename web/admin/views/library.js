import { icon } from '../../public/icons.js';
import { renderPagination } from '../../public/pagination.js';
import { attr } from '../dom.js';
import { state } from '../state.js';
import {
  renderRomDetail, renderRomEditForm, renderRomGrid, renderRomTable,
} from './library-renderers.js';
import { platformSelectFromList, renderRootTable, visibleRoms } from './shared.js';

export function renderLibrary() {
  const roms = visibleRoms();
  const platformsWithRoms = state.platforms
    .filter((platform) => Number(platform.rom_count) > 0);
  return `
    <section class="page-header">
      <div>
        <h1>Library</h1>
      </div>
      <div class="actions"><button class="icon-label-button" type="button" data-action="scan-library">${icon('folder-search')}<span>Scan library</span></button></div>
    </section>
    <details class="library-roots-card">
      <summary>Library folders and scanning</summary>
      <p class="hint">Scan adds new games from the default library folder without moving files. It does not repair missing files or update existing games.</p>
      ${renderRootTable(state.stats?.library_roots || [])}
    </details>
    <form class="filters" id="library-filter-form">
      <label><span>Search</span><input name="search" value="${attr(state.search)}" placeholder="Game title" /></label>
      <label><span>Platform</span>${platformSelectFromList('platform', platformsWithRoms, state.platformFilter, true)}</label>
      <label class="checkbox"><input name="missing_cover" type="checkbox" ${state.missingCoverFilter ? 'checked' : ''} /> Missing cover</label>
      <button type="button" data-action="clear-library-filter" ${state.search || state.platformFilter || state.missingCoverFilter ? '' : 'disabled'}>Clear filters</button>
    </form>
    <section class="card library-list-card">
      <div class="library-list-heading">
        <p class="muted">${roms.length} of ${state.romTotal} games</p>
        <div class="view-toggle" role="group" aria-label="Library view">
          <button type="button" data-library-view="list" aria-pressed="${state.libraryView === 'list'}">List</button>
          <button type="button" data-library-view="grid" aria-pressed="${state.libraryView === 'grid'}">Grid</button>
        </div>
      </div>
      ${state.libraryView === 'grid' ? renderRomGrid(roms) : renderRomTable(roms)}
      ${renderPagination({ total: state.romTotal, limit: state.romLimit, offset: state.romOffset })}
    </section>
    ${state.selectedRom ? `
      <div class="romm-drawer-backdrop" data-action="close-rom-detail"></div>
      <aside class="romm-drawer library-detail-drawer" role="dialog" aria-modal="true" aria-labelledby="library-detail-title">
        <div class="romm-drawer-head">
          <p class="eyebrow">${state.editingRomId ? 'Edit game' : 'Game details'}</p>
          <button class="ghost" type="button" data-action="close-rom-detail" aria-label="Close game details">✕</button>
        </div>
        <div class="romm-drawer-body" data-scroll-key="library-detail-drawer">
          ${state.editingRomId ? renderRomEditForm() : renderRomDetail(state.selectedRom)}
        </div>
      </aside>` : ''}`;
}
