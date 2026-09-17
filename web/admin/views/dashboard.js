import { formatBytes } from '../dom.js';
import { state } from '../state.js';
import { renderIgdbStatus } from './metadata.js';
import { renderPlatformStats, renderRootTable, statCard } from './shared.js';

export function renderDashboard() {
  const stats = state.stats;
  const platformsWithRoms = (stats?.platforms || [])
    .filter((platform) => Number(platform.rom_count) > 0);
  return `
    <section class="page-header">
      <div>
        <h1>Dashboard</h1>
      </div>
    </section>
    <section class="stats-grid" aria-label="Library totals">
      ${statCard('Games', stats?.total_roms ?? 0)}
      ${statCard('Files', stats?.total_files ?? 0)}
      ${statCard('Library size', formatBytes(stats?.total_file_bytes ?? 0))}
      ${statCard('Platforms', stats?.platforms_with_roms ?? 0)}
    </section>
    <section class="grid two dashboard-section">
      <div class="card">
        <h2 class="section-title">Library folders</h2>
        ${renderRootTable(stats?.library_roots || [])}
      </div>
      <div class="card">
        <h2 class="section-title">IGDB</h2>
        ${renderIgdbStatus()}
      </div>
    </section>
    <section class="card dashboard-section">
      <h2 class="section-title">Platforms</h2>
      ${renderPlatformStats(platformsWithRoms)}
    </section>`;
}
