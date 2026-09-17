import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import { initialGogImportProgress, state } from '../state.js';
import { renderDashboard } from './dashboard.js';
import { renderGogImport, renderGogImportProgress } from './gog-import.js';
import {
  renderCoverLarge, renderCoverThumb, renderRomDetail, renderRomEditForm,
} from './library-renderers.js';
import { renderJobs } from './jobs.js';
import { renderLibrary } from './library.js';
import { renderIgdbSearch, renderIgdbStatus } from './metadata.js';
import { renderRommBrowse } from './romm-browse.js';
import { renderSettings } from './settings.js';
import { renderNotificationStack, sideNavButton } from './shared.js';
import { renderImport, renderUpload } from './upload.js';

test('sidebar notifications stack loading, notices, and escaped errors', () => {
  const previousLoading = state.loading;
  const previousNotifications = state.notifications;
  state.loading = true;
  state.notifications = [
    { id: 41, kind: 'notice', message: 'Library refreshed.' },
    { id: 42, kind: 'error', message: '<offline>' },
  ];

  try {
    const markup = renderNotificationStack();
    assert.match(markup, /class="notification-stack"/);
    assert.match(markup, /Refreshing library…/);
    assert.equal((markup.match(/class="notification-icon"><svg class="icon" aria-hidden="true"/g) || []).length, 3);
    assert.match(markup, /data-notification-id="41"[^>]*role="status"/);
    assert.match(markup, /data-notification-id="42"[^>]*role="alert"/);
    assert.match(markup, /&lt;offline&gt;/);
    assert.doesNotMatch(markup, /<offline>/);
  } finally {
    state.loading = previousLoading;
    state.notifications = previousNotifications;
  }
});

test('sidebar navigation puts all running activity on Jobs', () => {
  const previousScreen = state.screen;
  const previousJobs = state.jobs;

  try {
    state.screen = 'jobs';
    state.jobs = [
      { id: 'upload_1', type: 'upload', state: 'running' },
      { id: 'gog_1', type: 'gog-import', state: 'running' },
    ];
    const jobsMarkup = sideNavButton('jobs', 'list-todo', 'Jobs', '2 active');
    assert.match(jobsMarkup, /class="active has-activity"/);
    assert.match(jobsMarkup, /aria-busy="true" aria-label="Jobs: 2 jobs in progress"/);
    assert.match(jobsMarkup, /<small>2 active<\/small>/);
    assert.match(jobsMarkup, /class="nav-activity" aria-hidden="true"/);

    const importMarkup = sideNavButton('upload', 'upload', 'Import');
    assert.doesNotMatch(importMarkup, /has-activity|aria-busy|nav-activity/);

    state.jobs = state.jobs.map((job) => ({ ...job, state: 'succeeded' }));
    const idleMarkup = sideNavButton('jobs', 'list-todo', 'Jobs');
    assert.doesNotMatch(idleMarkup, /has-activity|aria-busy|nav-activity/);
  } finally {
    state.screen = previousScreen;
    state.jobs = previousJobs;
  }
});

test('dashboard lists only platforms that contain ROMs', () => {
  const previousStats = state.stats;
  state.stats = {
    total_roms: 1,
    total_files: 1,
    total_file_bytes: 1024,
    platforms_with_roms: 1,
    library_roots: [],
    platforms: [
      {
        id: 1,
        slug: 'genesis',
        display_name: 'Sega Genesis',
        rom_count: 1,
        file_count: 1,
        total_file_bytes: 1024,
      },
      {
        id: 2,
        slug: 'empty-platform',
        display_name: 'Empty Platform',
        rom_count: 0,
        file_count: 0,
        total_file_bytes: 0,
      },
    ],
  };

  try {
    const markup = renderDashboard();
    assert.match(markup, /class="platform-stat"[^>]*><span aria-hidden="true"><img class="rom-platform-icon" src="\/public\/platform-icons\/genesis\.png"/);
    assert.match(markup, /Sega Genesis/);
    assert.doesNotMatch(markup, /Upload game|data-nav="upload"/);
    assert.doesNotMatch(markup, /Empty Platform/);
  } finally {
    state.stats = previousStats;
  }
});

test('settings exposes the browser-local admin library page size', () => {
  const previousLimit = state.romLimit;
  state.romLimit = 50;

  try {
    const markup = renderSettings();
    assert.match(markup, /<section class="settings-bento">/);
    assert.equal((markup.match(/class="settings-bento-column"/g) || []).length, 2);
    assert.doesNotMatch(markup, /wide-card/);
    assert.match(markup, /id="library-display-settings-form"/);
    assert.match(markup, /name="rom_page_size"[^>]*min="1"[^>]*max="100"[^>]*value="50"/);
    assert.match(markup, /Applies to the admin Library in this browser\./);
  } finally {
    state.romLimit = previousLimit;
  }
});

test('library platform filter includes only platforms containing ROMs', () => {
  const previousPlatforms = state.platforms;
  const previousStats = state.stats;
  const previousPlatformFilter = state.platformFilter;
  const previousMissingCoverFilter = state.missingCoverFilter;
  state.platforms = [
    { id: 11, slug: 'empty', display_name: 'Empty System', rom_count: 0 },
    { id: 12, slug: 'occupied', display_name: 'Occupied System', rom_count: 3 },
  ];
  state.stats = { ...previousStats, library_roots: [] };
  state.platformFilter = '12';
  state.missingCoverFilter = true;

  try {
    const markup = renderLibrary();
    assert.match(markup, /data-action="scan-library"><svg class="icon" aria-hidden="true"[^>]*><path d="M10\.7 20H4/);
    assert.doesNotMatch(markup, /Import games|data-nav="upload"/);
    assert.match(markup, /<option value="">All platforms<\/option>/);
    assert.match(markup, /value="12"[^>]*selected[^>]*>Occupied System \(occupied\)<\/option>/);
    assert.match(markup, /name="missing_cover"[^>]*checked[^>]*> Missing cover/);
    assert.match(markup, /data-action="clear-library-filter"[^>]*>Clear filters<\/button>/);
    assert.doesNotMatch(markup, /Empty System|value="11"|Apply filter/);
  } finally {
    state.platforms = previousPlatforms;
    state.stats = previousStats;
    state.platformFilter = previousPlatformFilter;
    state.missingCoverFilter = previousMissingCoverFilter;
  }
});

test('library list and grid use the same numbered pagination after the results', () => {
  const previous = { roms: state.roms, romTotal: state.romTotal, romLimit: state.romLimit,
    romOffset: state.romOffset, libraryView: state.libraryView, selectedRom: state.selectedRom };
  Object.assign(state, { roms: [{ id: 7, name: 'Page four game', platform_slug: 'nes' }],
    romTotal: 750, romLimit: 75, romOffset: 225, selectedRom: null });
  try {
    let pagination;
    for (const view of ['list', 'grid']) {
      state.libraryView = view;
      const markup = renderLibrary();
      const nav = markup.match(/<nav class="pagination"[\s\S]*?<\/nav>/)?.[0];
      assert.ok(nav);
      assert.match(nav, /aria-label="Page 4" aria-current="page" data-page-offset="225"/);
      assert.match(nav, /aria-label="Previous page" data-page-offset="150"/);
      assert.match(nav, /aria-label="Next page" data-page-offset="300"/);
      assert.ok(markup.indexOf(nav) > markup.indexOf('Page four game'));
      if (pagination) assert.equal(nav, pagination);
      pagination = nav;
    }
  } finally {
    Object.assign(state, previous);
  }
});

test('library switches between list and grid views', () => {
  const previous = {
    roms: state.roms, romTotal: state.romTotal, libraryView: state.libraryView,
    selectedRom: state.selectedRom, editingRomId: state.editingRomId, stats: state.stats,
  };
  state.roms = [
    {
      id: 7, name: 'Grid Game', slug: 'grid-game', platform_slug: 'nes',
      platform_display_name: 'Nintendo Entertainment System', fs_size_bytes: 1024,
      path_cover_small: 'covers/grid-small.jpg', path_cover_large: 'covers/grid-large.jpg',
    },
    {
      id: 8, name: 'Second Game', slug: 'second-game', platform_slug: 'sgb',
      platform_display_name: 'Nintendo Super Game Boy', fs_size_bytes: 2048,
    },
  ];
  state.romTotal = 2;
  state.selectedRom = state.roms[0];
  state.editingRomId = null;
  state.stats = { ...state.stats, library_roots: [] };

  try {
    state.libraryView = 'grid';
    const gridMarkup = renderLibrary();
    assert.match(gridMarkup, /data-library-view="grid" aria-pressed="true"/);
    assert.match(gridMarkup, /<section class="card library-list-card">/);
    assert.doesNotMatch(gridMarkup, /class="grid two"/);
    assert.match(gridMarkup, /class="rom-grid-card active" data-rom-id="7"/);
    assert.match(gridMarkup, /class="romm-drawer library-detail-drawer"[^>]*aria-labelledby="library-detail-title"/);
    assert.match(gridMarkup, /data-action="close-rom-detail" aria-label="Close game details"/);
    assert.match(gridMarkup, /class="rom-grid-cover"><img class="cover-thumb"[^>]*data-cover-path="covers\/grid-large\.jpg"[^>]*><img class="rom-platform-icon rom-grid-platform-icon" src="\/public\/platform-icons\/nes\.png" alt=""/);
    assert.match(gridMarkup, /class="rom-platform-icon rom-grid-platform-icon" src="\/public\/platform-icons\/sgb\.png" alt=""/);
    assert.doesNotMatch(gridMarkup, /<table>/);

    state.libraryView = 'list';
    const listMarkup = renderLibrary();
    assert.match(listMarkup, /data-library-view="list" aria-pressed="true"/);
    assert.match(listMarkup, /<table>/);
    assert.match(listMarkup, /<td><img class="cover-thumb"[^>]*data-cover-path="covers\/grid-small\.jpg"/);
    assert.match(listMarkup, /class="rom-platform-icon" src="\/public\/platform-icons\/nes\.png" alt="Nintendo Entertainment System"/);
    assert.match(listMarkup, /class="rom-platform-icon" src="\/public\/platform-icons\/sgb\.png" alt="Nintendo Super Game Boy"/);
    assert.doesNotMatch(listMarkup, /Nintendo Entertainment System<br>|<small class="mono">nes<\/small>/);
    assert.doesNotMatch(listMarkup, /class="rom-grid"/);

    state.editingRomId = 7;
    const editMarkup = renderLibrary();
    assert.match(editMarkup, /class="romm-drawer library-detail-drawer"/);
    assert.match(editMarkup, /Edit game/);
    assert.match(editMarkup, /id="rom-edit-form" data-rom-id="7"/);
    assert.doesNotMatch(editMarkup, /modal-backdrop|modal-card/);
  } finally {
    Object.assign(state, previous);
  }
});

test('game covers show the placeholder before downloaded artwork loads', () => {
  const rom = {
    name: 'Test Game',
    path_cover_large: 'library/test-game/cover-large.jpg',
    path_cover_small: 'library/test-game/cover-small.jpg',
  };

  for (const markup of [renderCoverThumb(rom), renderCoverLarge(rom)]) {
    assert.match(markup, /src="\/admin\/game-cover-placeholder\.jpg"/);
    assert.match(markup, /data-cover-path=/);
  }

  const missingCover = renderCoverLarge({ name: 'Missing Cover' });
  assert.match(missingCover, /src="\/admin\/game-cover-placeholder\.jpg"/);
  assert.doesNotMatch(missingCover, /data-cover-path=/);
});

test('coverless game IGDB search uses the detected filename title', () => {
  const rom = {
    id: 7,
    name: 'Great Volleyball (USA, Europe).zip',
    metadatum: { filename: { clean_title: 'Great Volleyball' } },
  };

  assert.match(renderIgdbSearch(rom), /name="q" value="Great Volleyball"/);
  assert.match(
    renderIgdbSearch({ ...rom, path_cover_small: 'covers/game.jpg' }),
    /name="q" value="Great Volleyball \(USA, Europe\)\.zip"/,
  );
});

test('drawer ROM editor accepts supported cover image formats', () => {
  const previousRom = state.selectedRom;
  const previousEditingRomId = state.editingRomId;
  state.selectedRom = {
    id: 7, name: 'Cover Game', platform_id: 1, regions: [], metadatum: {},
  };
  state.editingRomId = 7;

  try {
    const markup = renderRomEditForm();
    assert.match(markup, /id="library-detail-title">Cover Game<\/h3>/);
    assert.match(markup, /id="rom-edit-form"/);
    assert.doesNotMatch(markup, /modal-backdrop|modal-card/);
    assert.match(markup, /name="cover" type="file"/);
    assert.match(markup, /accept="image\/jpeg,image\/png,image\/webp"/);
    assert.match(markup, /up to 10 MiB/);
    assert.match(markup, /IGDB match/);
    assert.match(markup, /id="igdb-search-form"/);
  } finally {
    state.selectedRom = previousRom;
    state.editingRomId = previousEditingRomId;
  }
});

test('game detail puts one package ZIP below the cover and shows file lists without group names', () => {
  const previousFiles = state.selectedRomFiles;
  const files = [
    {
      id: 11, file_name: 'Game.cue', original_file_name: 'Game.cue',
      relative_path: 'psx/Game.cue', file_size_bytes: 32, role: 'descriptor', launchable: true,
    },
    {
      id: 12, file_name: 'Game.bin', original_file_name: 'Game.bin',
      relative_path: 'psx/Game.bin', file_size_bytes: 1024, role: 'track', launchable: false,
    },
  ];
  const rom = {
    id: 7, name: 'Package Game', platform_slug: 'psx', platform_display_name: 'PlayStation',
    regions: [], metadatum: {}, files,
  };
  state.selectedRomFiles = {
    groups: [{
      kind: 'track_set', display_name: 'Package Game', group_key: 'package-game',
      launchable: true, files,
    }],
    ungrouped_files: [], dependencies: [], warnings: [],
  };

  try {
    const markup = renderRomDetail(rom);
    const coverIndex = markup.indexOf('class="cover-large"');
    const packageIndex = markup.indexOf('data-download-package="7"');
    assert.ok(coverIndex >= 0 && packageIndex > coverIndex);
    assert.match(markup, /data-download-package="7"[^>]*>Download<\/button>/);
    assert.match(markup, /class="detail-cover"><img class="cover-large"[^>]*><img class="rom-platform-icon rom-grid-platform-icon" src="\/public\/platform-icons\/psx\.png" alt=""/);
    assert.doesNotMatch(markup, /detail-game-title/);
    const singleFileMarkup = renderRomDetail({ ...rom, files: [files[0]] });
    assert.match(singleFileMarkup, /data-download-rom="7"[^>]*>Download<\/button>/);
    assert.equal((markup.match(/data-download-rom=/g) || []).length, 2);
    assert.doesNotMatch(markup, /IGDB match|id="igdb-search-form"|package-game|class="file-group-heading"|class="file-list"/);
  } finally {
    state.selectedRomFiles = previousFiles;
  }
});

test('Import combines ROM uploads and GOG setup imports on one page', () => {
  const markup = renderImport();
  assert.equal((markup.match(/<h1>/g) || []).length, 1);
  assert.match(markup, /class="import-columns"/);
  assert.equal((markup.match(/class="hint preparation-notice"/g) || []).length, 1);
  assert.match(markup, /class="import-notice"/);
  assert.match(markup, /id="upload-form"/);
  assert.match(markup, /id="gog-import-form"/);
  assert.ok(markup.indexOf('import-notice') < markup.indexOf('id="upload-form"'));
  assert.ok(markup.indexOf('id="gog-import-form"') < markup.indexOf('gog-import-warning'));
  assert.equal((markup.match(/class="warning gog-import-warning"/g) || []).length, 1);
  assert.ok(markup.indexOf('id="upload-form"') < markup.indexOf('id="gog-import-form"'));
  assert.equal((markup.match(/class="card import-card"/g) || []).length, 2);
  assert.equal((markup.match(/class="import-info"/g) || []).length, 2);
  assert.equal((markup.match(/class="page-header"/g) || []).length, 1);
  assert.match(markup, /<h2 class="section-title">Game files<\/h2>/);
  assert.match(markup, /<h2 class="section-title">GOG installer<\/h2>/);
  assert.match(markup, /<summary aria-label="About game upload">/);
  assert.match(markup, /<summary aria-label="About GOG setup import">/);
  assert.doesNotMatch(markup, /<details class="import-info" open/);
  assert.match(markup, /Some GOG installers need extra components/);
  assert.doesNotMatch(markup, /legally entitled|does not contact GOG/);
});

test('upload renderer explains the Windows archive install contract', () => {
  const markup = renderUpload();
  assert.match(markup, /For Windows, upload/);
  assert.match(markup, /prebuilt <code>\.zip<\/code> or <code>\.7z<\/code>/);
});

test('upload platform picker shows platform icons', () => {
  const previousPlatforms = state.platforms;
  const previousPlatformId = state.uploadPlatformId;
  state.platforms = [
    { id: 1, slug: 'nes', display_name: 'Nintendo Entertainment System' },
    { id: 2, slug: 'sgb', display_name: 'Nintendo Super Game Boy' },
  ];
  state.uploadPlatformId = '2';

  try {
    const markup = renderUpload();
    assert.match(markup, /class="platform-picker"/);
    assert.match(markup, /src="\/public\/platform-icons\/nes\.png"/);
    assert.match(markup, /src="\/public\/platform-icons\/sgb\.png"/);
    assert.match(markup, /name="platform_id" value="2" checked/);
    assert.doesNotMatch(markup, /<select name="platform_id"/);
  } finally {
    state.platforms = previousPlatforms;
    state.uploadPlatformId = previousPlatformId;
  }
});

test('upload page only prepares jobs and contains no running progress', () => {
  const markup = renderUpload();
  assert.match(markup, /Follow progress in Jobs/);
  assert.match(markup, /Upload games/);
  assert.match(markup, /For Windows, upload one prebuilt <code>\.zip<\/code> or <code>\.7z<\/code> archive per game\. Teatro stores the archive unchanged\./);
  assert.doesNotMatch(markup, /<progress|upload-progress-current/);
});

test('Jobs renders upload transfer speed and byte progress', () => {
  const previousJobs = state.jobs;
  state.jobs = [{
    id: 'upload_test', type: 'upload', title: 'Planned Game', detail: 'NES · 1 file',
    state: 'running', statusText: 'Uploading grouped batch', createdAt: Date.now(),
    progress: { active: true, percent: 50, loaded: 512, total: 1024, speed: 256 },
  }];
  try {
    const markup = renderJobs();
    assert.match(markup, /Planned Game/);
    assert.match(markup, /value="50"/);
    assert.match(markup, /256 B\/s/);
    assert.match(markup, /512 B \/ 1\.0 KiB/);
  } finally {
    state.jobs = previousJobs;
  }
});

test('Jobs keeps running work above queued work', () => {
  const previousJobs = state.jobs;
  state.jobs = [
    {
      id: 'queued', type: 'upload', title: 'Queued Game', detail: '', state: 'queued',
      createdAt: 3, progress: {},
    },
    {
      id: 'finished', type: 'upload', title: 'Finished Game', detail: '', state: 'succeeded',
      createdAt: 2, completedAt: 4, progress: {},
    },
    {
      id: 'running', type: 'upload', title: 'Running Game', detail: '', state: 'running',
      createdAt: 1, progress: {},
    },
  ];
  try {
    const markup = renderJobs();
    assert.ok(markup.indexOf('Running Game') < markup.indexOf('Queued Game'));
    assert.ok(markup.indexOf('Queued Game') < markup.indexOf('Finished Game'));
    assert.deepEqual(state.jobs.map(({ id }) => id), ['queued', 'finished', 'running']);
  } finally {
    state.jobs = previousJobs;
  }
});

test('Jobs shows how long completed work took', () => {
  const previousJobs = state.jobs;
  state.jobs = [{
    id: 'scan_done', type: 'library-scan', title: 'Managed library scan', detail: '',
    state: 'succeeded', createdAt: 1_000, completedAt: 66_000,
    progress: { active: false, phase: 'complete' }, result: {},
  }];
  try {
    assert.match(renderJobs(), /Duration 1m 5s/);
  } finally {
    state.jobs = previousJobs;
  }
});

test('upload uses compact collapsible groups with editable planned titles', () => {
  const previousPlan = state.uploadPlan;
  state.uploadPlan = {
    platform_slug: 'psx',
    errors: [],
    warnings: [],
    roms: [{
      plan_id: 'rom-1',
      title: 'Detected Game',
      slug: 'detected-game',
      regions: [],
      groups: [{ key: 'group-1', kind: 'single', display_name: 'Detected Game' }],
      files: [{
        key: 'file-1', group_key: 'group-1', original_file_name: 'Detected Game.bin',
        file_size_bytes: 1024, role: 'content', launchable: true,
      }],
      dependencies: [{
        parent_file_key: 'file-1', child_file_key: 'software-managed-child',
        dependency_kind: 'required',
      }],
    }],
  };

  try {
    const markup = renderUpload();
    assert.match(markup, /data-planned-title="rom-1"/);
    assert.match(markup, /value="Detected Game"/);
    assert.match(markup, /<details class="plan-group">\s*<summary>/);
    assert.match(markup, /Detected Game\.bin/);
    assert.doesNotMatch(markup, /<details class="plan-group"[^>]*open/);
    assert.doesNotMatch(markup, /dependency-details|dependency rows|software-managed-child/);
    assert.doesNotMatch(markup, /Separate ROMs|upload_mode|Title override/);
  } finally {
    state.uploadPlan = previousPlan;
  }
});

test('active jobs never disable either preparation page', () => {
  const previousJobs = state.jobs;
  const previousPlan = state.uploadPlan;
  const previousStatus = state.gogImportStatus;
  state.jobs = [{ id: 'upload_active', type: 'upload', state: 'running' }];
  state.uploadPlan = {
    errors: [], warnings: [],
    roms: [{ plan_id: 'rom-1', title: 'Ready Game', slug: 'ready-game', groups: [], files: [] }],
  };
  state.gogImportStatus = {
    enabled: true, configured: true, bundled_extractor: true, timeout_seconds: 30,
    max_extracted_bytes: 1024, max_extracted_files: 10,
  };
  try {
    const upload = renderUpload();
    assert.match(upload, /data-action="finalize-upload-plan" type="submit" >Upload games/);
    const gog = renderGogImport();
    assert.doesNotMatch(gog, /experimental/i);
    assert.match(gog, /id="gog-import-file-input"[^>]*multiple  \/>/);
    assert.match(gog, /id="gog-import-submit"[^>]*type="submit" >Import installer/);
  } finally {
    state.jobs = previousJobs;
    state.uploadPlan = previousPlan;
    state.gogImportStatus = previousStatus;
  }
});

test('GOG importer renders an escaped editable suggestion with accessible review hints', () => {
  const previousStatus = state.gogImportStatus;
  const previousTitle = state.gogImportTitle;
  const previousOrigin = state.gogImportTitleOrigin;
  const previousConfidence = state.gogImportTitleConfidence;
  state.gogImportStatus = {
    enabled: true, configured: true, bundled_extractor: true, timeout_seconds: 30,
    max_extracted_bytes: 1024, max_extracted_files: 10,
  };
  state.gogImportTitle = '\"><script>suggestion</script>';
  state.gogImportTitleOrigin = 'inferred';
  state.gogImportTitleConfidence = 'high';

  try {
    let markup = renderGogImport();
    assert.doesNotMatch(markup, /GOG importer enabled with/);
    assert.match(markup, /Current limits: 10 extracted entries, 1\.0 KiB total extracted data, and 30s per extraction phase\./);
    assert.match(markup, /name="title"[^>]*value="&quot;&gt;&lt;script&gt;suggestion&lt;\/script&gt;"/);
    assert.doesNotMatch(markup, /<script>suggestion/);
    assert.match(markup, /aria-describedby="gog-import-title-hint"/);
    assert.match(markup, /id="gog-import-title-hint"[^>]*data-title-origin="inferred"[^>]*data-title-confidence="high"/);
    assert.match(markup, /Suggested from the filename\. Check the title before importing\./);
    assert.doesNotMatch(markup, /readonly/);

    state.gogImportTitleConfidence = 'medium';
    markup = renderGogImport();
    assert.match(markup, /filename is ambiguous/);
    assert.match(markup, /Check the suggested title carefully/);

    state.gogImportTitleOrigin = 'manual';
    state.gogImportTitleConfidence = null;
    markup = renderGogImport();
    assert.match(markup, /Used for the game title and ZIP filename/);
    assert.doesNotMatch(markup, /Suggested from the filename/);
  } finally {
    state.gogImportStatus = previousStatus;
    state.gogImportTitle = previousTitle;
    state.gogImportTitleOrigin = previousOrigin;
    state.gogImportTitleConfidence = previousConfidence;
  }
});

test('GOG preparation stays clear while Jobs renders byte progress and escaped output', () => {
  const previousStatus = state.gogImportStatus;
  state.gogImportStatus = {
    enabled: true, configured: true, bundled_extractor: true, timeout_seconds: 30,
    max_extracted_bytes: 1024, max_extracted_files: 10,
  };
  const progress = {
    ...initialGogImportProgress(),
    active: true,
    serverProcessing: true,
    jobId: 'gog_test',
    jobState: 'running',
    phase: 'create_windows_zip',
    jobProgress: { kind: 'bytes', current: 512, total: 1024, percent: 50 },
    phaseEvents: [
      { seq: 1, kind: 'phase', phase: 'validate_extracted_tree', stream: null, text: 'Validating extracted tree' },
      { seq: 2, kind: 'phase', phase: 'create_windows_zip', stream: null, text: 'Creating Windows ZIP' },
    ],
    transcript: [
      { seq: 3, kind: 'output', phase: 'extract_installer_payload', stream: 'stdout', text: '<img src=x onerror=alert(1)>' },
    ],
    outputTruncated: true,
  };
  try {
    const preparation = renderGogImport();
    assert.match(preparation, /accept="\.exe,\.bin"/);
    assert.match(preparation, /Import installer/);
    assert.match(preparation, /Follow progress in Jobs/);
    assert.match(preparation, /Some GOG installers need extra components/);
    assert.doesNotMatch(preparation, /Creating Windows ZIP|Sanitized innoextract output|<progress/);

    const markup = renderGogImportProgress(progress, 'gog-card');
    assert.match(markup, /data-gog-phase="create_windows_zip"/);
    assert.match(markup, /Creating Windows ZIP/);
    assert.match(markup, /id="gog-card-current"[^>]*role="status" aria-live="polite"/);
    assert.match(markup, /aria-labelledby="gog-card-current gog-card-percent" value="50"/);
    assert.match(markup, /512 B \/ 1\.0 KiB/);
    assert.match(markup, /Earlier or oversized output was discarded/);
    assert.match(markup, /&lt;img src=x onerror=alert\(1\)&gt;/);
    assert.doesNotMatch(markup, /<img src=x/);
    assert.match(markup, /aria-label="Observed server-side import phases"/);
    assert.match(markup, /tabindex="0" aria-label="Sanitized innoextract output" aria-live="off"/);
    assert.equal((markup.match(/aria-live="polite"/g) || []).length, 1);
  } finally {
    state.gogImportStatus = previousStatus;
  }
});

test('GOG Jobs keeps finalization indeterminate and shows stable failures', () => {
  const progress = {
    ...initialGogImportProgress(),
    active: true,
    serverProcessing: true,
    jobId: 'gog_test',
    jobState: 'running',
    phase: 'finalize_windows_zip',
    jobProgress: null,
  };
  let markup = renderGogImportProgress(progress, 'gog-finalize');
  assert.match(markup, /Finalizing Windows ZIP/);
  assert.match(markup, /aria-labelledby="gog-finalize-current gog-finalize-percent"><\/progress>/);
  assert.match(markup, /Progress unavailable for this step/);

  markup = renderGogImportProgress({
    ...progress,
    active: false,
    jobState: 'failed',
    phase: 'validate_sipario_zip',
    jobError: { code: 'unprocessable_entity', message: 'Generated ZIP failed <validation>' },
  }, 'gog-failed');
  assert.match(markup, /Import failed during validating windows zip/);
  assert.match(markup, /Generated ZIP failed &lt;validation&gt;/);
});

test('rendered views do not contain CSP-blocked inline styles', () => {
  const previous = {
    platforms: state.platforms,
    rommSource: state.rommSource,
    rommPlatforms: state.rommPlatforms,
    rommBrowse: state.rommBrowse,
    rommSelected: state.rommSelected,
    rommDrawer: state.rommDrawer,
  };
  // The RomM screen is rendered in both states, and with a selection and an open drawer, so every
  // branch of the newest view is covered by this guard.
  const unconfigured = renderRommBrowse();
  state.rommSource = {
    enabled: true,
    configured: true,
    base_url: 'https://romm.example/',
    username: 'teatro',
    auth_mode: 'token',
    secret_configured: true,
    plaintext_http: false,
  };
  state.platforms = [{ id: 3, slug: 'snes', display_name: 'SNES' }];
  state.rommPlatforms = [{ id: 4, slug: 'snes', name: 'Super Nintendo', rom_count: 3 }];
  state.rommBrowse = {
    ...state.rommBrowse,
    loaded: true,
    loading: false,
    total: 2,
    items: [{
      id: 7, name: 'Remote Game', platform_slug: 'snes', platform_name: 'Super Nintendo',
      file_count: 1, file_size_bytes: 1024, has_cover: true, already_present: false,
      target_platform_id: 3, target_platform_name: 'SNES',
    }],
  };
  state.rommSelected = [{
    id: 7, name: 'Remote Game', platformName: 'Super Nintendo', fileCount: 1, sizeBytes: 1024,
    targetPlatformId: '3', targetPlatformName: 'SNES',
    titleOverride: null, platformOverride: null, fileIds: null,
  }];
  state.rommDrawer = {
    romId: 7,
    loading: false,
    rom: state.rommBrowse.items[0],
    files: [{ id: 11, file_name: 'game.sfc', file_size_bytes: 1024, hash_signals: ['sha256'] }],
    totalSizeBytes: 1024,
    title: 'Remote Game',
    platformId: '3',
    fileIds: [11],
  };
  const configured = renderRommBrowse();
  Object.assign(state, previous);

  const markup = `${renderDashboard()}${renderUpload()}${renderGogImport()}${renderJobs()}${renderSettings()}${unconfigured}${configured}`;
  assert.match(configured, /class="romm-destination-icon"[^>]*><img class="rom-platform-icon rom-grid-platform-icon" src="\/public\/platform-icons\/snes\.png" alt=""/);
  assert.doesNotMatch(markup, /\sstyle\s*=/i);
  assert.doesNotMatch(markup, /Windows \(x86\)/);
});

test('admin pages use one task heading and preserve warnings and field labels', () => {
  for (const render of [renderDashboard, renderLibrary, renderImport, renderUpload,
    renderGogImport, renderJobs, renderSettings, renderRommBrowse]) {
    const markup = render();
    assert.equal((markup.match(/<h1[ >]/g) || []).length, 1);
    const header = markup.match(/<section class="page-header">[\s\S]*?<\/section>/)?.[0];
    assert.ok(header);
    assert.doesNotMatch(header, /class="eyebrow"/);
  }
  assert.match(renderLibrary(), /<details class="library-roots-card">\s*<summary>/);
  const settings = renderSettings();
  assert.match(settings, /permanently delete managed games and their managed files/);
  assert.match(settings, /<span>Type DELETE ALL to confirm<\/span>/);
  const css = readFileSync(new URL('../app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /[✦◆♦◇]|rotate\(45deg\)/);
  assert.match(css, /--brand: #d65353/);
});

test('IGDB status uses escaped facts rather than nested statistic cards', () => {
  const previous = state.igdbSettings;
  state.igdbSettings = { configured: true, client_id_configured: true,
    client_secret_configured: true, client_id: 'test-client-id',
    client_id_source: '<source>', client_secret_source: '<secret>' };
  try {
    const markup = renderIgdbStatus(true);
    assert.match(markup, /<dl class="status-list">/);
    assert.match(markup, />Set<\/dd>/);
    assert.match(markup, /&lt;secret&gt;/);
    assert.doesNotMatch(markup, /<source>|<secret>|test-client-id|class="card"/);
  } finally {
    state.igdbSettings = previous;
  }
});
