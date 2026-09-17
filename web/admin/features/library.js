import { downloadFile as downloadSharedFile, submitDownloadTicket } from '../../public/downloads.js';
import { api } from '../api.js';
import { clearAuth } from '../auth.js';
import { parseCsv } from '../dom.js';
import {
  beginRomSelection, clearRomSelection, isCurrentRomSelection, replaceRom, selectRom,
  setEditingRom, setLibraryFilter, setLibraryView, setRomOffset, setRomPageSize, state,
} from '../state.js';

export function createLibraryController(context) {
  const dependencies = Object.freeze({ ...context });
  let filterVersion = 0;
  let searchFilterTimeout;

  async function refreshLibrary() {
    await Promise.all([
      dependencies.loadPlatforms(), dependencies.loadStats(), dependencies.loadRoms(),
    ]);
  }

  async function selectRomById(romId) {
    const requestId = beginRomSelection();
    try {
      const [rom, files] = await Promise.all([
        api(`/api/roms/${encodeURIComponent(romId)}`),
        api(`/api/admin/roms/${encodeURIComponent(romId)}/files`).catch(() => null),
      ]);
      if (!isCurrentRomSelection(requestId)) return;
      selectRom(rom, files);
    } catch (error) {
      if (!isCurrentRomSelection(requestId)) return;
      dependencies.setError(error);
    }
    dependencies.render();
    dependencies.app.querySelector('button[data-action="close-rom-detail"]')?.focus();
  }

  async function applyFilter(formElement, version) {
    const form = new FormData(formElement);
    setLibraryFilter(
      String(form.get('search') || ''),
      String(form.get('platform') || ''),
      form.get('missing_cover') === 'on',
    );
    try {
      await dependencies.loadRoms();
    } catch (error) {
      if (version === filterVersion) dependencies.setError(error);
    }
    if (version !== filterVersion) return;
    const searchInput = formElement.querySelector('input[name="search"]');
    const restoreSearchFocus = formElement.ownerDocument?.activeElement === searchInput;
    const selectionStart = searchInput?.selectionStart;
    const selectionEnd = searchInput?.selectionEnd;
    dependencies.render();
    if (restoreSearchFocus) {
      const renderedSearchInput = dependencies.app.querySelector('#library-filter-form input[name="search"]');
      renderedSearchInput?.focus();
      if (selectionStart != null) renderedSearchInput?.setSelectionRange(selectionStart, selectionEnd);
    }
  }

  function onFilterSubmit(event) {
    event.preventDefault();
    clearTimeout(searchFilterTimeout);
    return applyFilter(event.currentTarget, ++filterVersion);
  }

  function onSearchInput(event) {
    clearTimeout(searchFilterTimeout);
    const form = event.currentTarget.form;
    const version = ++filterVersion;
    searchFilterTimeout = setTimeout(() => {
      if (form.isConnected) applyFilter(form, version);
    }, 250);
  }

  function onPlatformChange(event) {
    clearTimeout(searchFilterTimeout);
    return applyFilter(event.currentTarget.form, ++filterVersion);
  }

  function onClearFilter(event) {
    clearTimeout(searchFilterTimeout);
    const form = event.currentTarget.form;
    form.elements.search.value = '';
    form.elements.platform.value = '';
    form.elements.missing_cover.checked = false;
    return applyFilter(form, ++filterVersion);
  }

  async function onPageSizeSubmit(event) {
    event.preventDefault();
    const pageSize = setRomPageSize(new FormData(event.currentTarget).get('rom_page_size'));
    try {
      await dependencies.loadRoms();
      dependencies.setNotice(`Admin Library will show ${pageSize} titles per page.`);
    } catch (error) {
      dependencies.setError(error);
    }
    dependencies.render();
  }

  async function onEditSubmit(event) {
    event.preventDefault();
    const formElement = event.currentTarget;
    const form = new FormData(formElement);
    const yearText = String(form.get('release_year') || '').trim();
    const cover = form.get('cover');
    const hasCover = cover && typeof cover === 'object' && cover.size > 0;
    const payload = {
      name: String(form.get('name') || '').trim(),
      platform_id: Number(form.get('platform_id')),
      summary: String(form.get('summary') || '').trim() || null,
      regions: parseCsv(String(form.get('regions') || '')),
      genres: parseCsv(String(form.get('genres') || '')),
      developers: parseCsv(String(form.get('developers') || '')),
      publishers: parseCsv(String(form.get('publishers') || '')),
      release_year: yearText ? Number(yearText) : null,
    };
    try {
      let updated = await api(`/api/admin/roms/${encodeURIComponent(formElement.dataset.romId)}`, {
        method: 'PATCH', body: JSON.stringify(payload),
      });
      replaceRom(updated);
      if (hasCover) {
        updated = await api(`/api/admin/roms/${encodeURIComponent(formElement.dataset.romId)}/cover`, {
          method: 'PUT', headers: { 'Content-Type': cover.type || 'application/octet-stream' }, body: cover,
        });
        replaceRom(updated);
      }
      setEditingRom(null);
      dependencies.setNotice(`Saved ${updated.name}${hasCover ? ' and its cover' : ''}.`);
    } catch (error) {
      dependencies.setError(error);
    }
    dependencies.render();
  }

  async function onDeleteSubmit(event) {
    event.preventDefault();
    const romId = event.currentTarget.dataset.romId;
    const name = state.selectedRom?.name || 'this game';
    const packageWarning = (state.selectedRom?.files?.length || 0) > 1
      ? ' If this package uses a managed game folder, that complete folder—including sidecars—will be removed; the platform folder will remain.'
      : '';
    if (!confirm(`Delete ${name} and its managed file(s) from disk?${packageWarning}`)) return;
    try {
      await api(`/api/admin/roms/${encodeURIComponent(romId)}`, { method: 'DELETE' });
      clearRomSelection();
      await refreshLibrary();
      dependencies.setNotice(`Deleted ${name} and its managed file(s).`);
    } catch (error) {
      dependencies.setError(error);
    }
    dependencies.render();
  }

  async function downloadPackage(button) {
    const rom = state.selectedRom;
    if (!rom || String(rom.id) !== button.dataset.downloadPackage || (rom.files?.length || 0) < 2 || !state.auth?.header) return;

    button.disabled = true;
    dependencies.setNotice('Preparing package ZIP on Teatro…');
    dependencies.render();
    button = dependencies.app.querySelector('[data-download-package]') || button;
    button.disabled = true;
    try {
      const response = await api(`/api/roms/${encodeURIComponent(rom.id)}/archive-ticket`, {
        method: 'POST',
      });
      submitDownloadTicket(response.ticket);
      dependencies.setNotice('Package download started in your browser.');
    } catch (error) {
      dependencies.setError(error);
    } finally {
      if (button.isConnected) button.disabled = false;
      dependencies.render();
    }
  }

  async function downloadFile(romId, fileId) {
    const rom = state.selectedRom;
    const file = rom?.files?.find((item) => String(item.id) === String(fileId));
    if (!file || String(rom.id) !== String(romId) || !state.auth?.header) return;

    try {
      await downloadSharedFile({
        romId,
        file,
        authorization: state.auth.header,
        notify: (message) => {
          dependencies.setNotice(message);
          dependencies.render();
        },
      });
    } catch (error) {
      if (error?.status === 401) clearAuth();
      dependencies.setError(error);
      dependencies.render();
    }
  }

  function bind() {
    const app = dependencies.app;
    app.querySelector('#library-display-settings-form')?.addEventListener('submit', onPageSizeSubmit);
    const filterForm = app.querySelector('#library-filter-form');
    filterForm?.addEventListener('submit', onFilterSubmit);
    filterForm?.querySelector('input[name="search"]')?.addEventListener('input', onSearchInput);
    filterForm?.querySelector('select[name="platform"]')?.addEventListener('change', onPlatformChange);
    filterForm?.querySelector('input[name="missing_cover"]')?.addEventListener('change', onPlatformChange);
    filterForm?.querySelector('[data-action="clear-library-filter"]')?.addEventListener('click', onClearFilter);
    app.querySelectorAll('[data-library-view]').forEach((button) => {
      button.addEventListener('click', () => { setLibraryView(button.dataset.libraryView); dependencies.render(); });
    });
    app.querySelectorAll('[data-page-offset]').forEach((button) => {
      button.addEventListener('click', async () => {
        setRomOffset(button.dataset.pageOffset);
        await dependencies.loadRoms().catch(dependencies.setError);
        dependencies.render();
      });
    });
    app.querySelector('#rom-edit-form')?.addEventListener('submit', onEditSubmit);
    const closeRomDetail = () => {
      clearRomSelection();
      dependencies.render();
    };
    app.querySelectorAll('[data-action="close-rom-detail"]').forEach((element) => {
      element.addEventListener('click', closeRomDetail);
    });
    app.querySelector('.library-detail-drawer')?.addEventListener('keydown', (event) => {
      if (event.key === 'Escape') closeRomDetail();
    });
    app.querySelector('[data-action="open-rom-edit"]')?.addEventListener('click', () => {
      if (state.selectedRom?.id) setEditingRom(state.selectedRom.id);
      dependencies.render();
      dependencies.app.querySelector('#rom-edit-form input[name="name"]')?.focus();
    });
    app.querySelectorAll('[data-action="close-rom-edit"]').forEach((button) => {
      button.addEventListener('click', () => { setEditingRom(null); dependencies.render(); });
    });
    app.querySelector('#delete-rom-form')?.addEventListener('submit', onDeleteSubmit);
    app.querySelectorAll('button[data-rom-id]:not([data-apply-igdb])').forEach((button) => {
      button.addEventListener('click', () => selectRomById(button.dataset.romId));
    });
    app.querySelector('[data-download-package]')?.addEventListener(
      'click', (event) => downloadPackage(event.currentTarget),
    );
    app.querySelectorAll('[data-download-rom]').forEach((button) => {
      button.addEventListener('click', () => downloadFile(
        button.dataset.downloadRom, button.dataset.fileId,
      ));
    });
  }

  return { bind, selectRomById };
}
