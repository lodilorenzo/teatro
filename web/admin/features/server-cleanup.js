import { api } from '../api.js';
import { clearRomSelection, state } from '../state.js';

const UTF8_ENCODER = new TextEncoder();

function safeRelativePath(value) {
  return typeof value === 'string'
    && value.length > 0
    && !value.startsWith('/')
    && !/^[A-Za-z]:[\\/]/u.test(value)
    && UTF8_ENCODER.encode(value).byteLength <= 4 * 1024;
}

export function validateSidecarCleanupPreview(preview) {
  if (!preview
    || !Number.isSafeInteger(preview.file_count)
    || preview.file_count < 0
    || !Number.isSafeInteger(preview.total_file_bytes)
    || preview.total_file_bytes < 0
    || typeof preview.truncated !== 'boolean'
    || !Array.isArray(preview.files)
    || preview.files.length !== preview.file_count
    || preview.files.length > 256) {
    throw new Error('Teatro returned an invalid sidecar cleanup preview.');
  }
  const paths = new Set();
  let total = 0;
  for (const file of preview.files) {
    if (!safeRelativePath(file?.relative_path)
      || paths.has(file.relative_path)
      || !Number.isSafeInteger(file.file_size_bytes)
      || file.file_size_bytes < 0
      || typeof file.fingerprint !== 'string'
      || !/^\d+:\d+$/u.test(file.fingerprint)
      || file.fingerprint.length > 128) {
      throw new Error('Teatro returned an unsafe sidecar cleanup target.');
    }
    paths.add(file.relative_path);
    total += file.file_size_bytes;
  }
  if (!Number.isSafeInteger(total) || total !== preview.total_file_bytes) {
    throw new Error('Teatro returned an invalid sidecar cleanup size.');
  }
  return preview;
}

export function validateSidecarCleanupResult(result, expected) {
  if (!result
    || !Number.isSafeInteger(result.deleted_file_count)
    || !Number.isSafeInteger(result.deleted_file_bytes)
    || !Array.isArray(result.deleted_files)
    || result.deleted_file_count !== expected.file_count
    || result.deleted_file_bytes !== expected.total_file_bytes
    || result.deleted_files.length !== expected.file_count
    || result.deleted_files.some((path, index) => path !== expected.files[index].relative_path)) {
    throw new Error('Teatro returned an invalid sidecar cleanup result.');
  }
  return result;
}

export function createServerCleanupController(context) {
  const dependencies = Object.freeze({ ...context });
  const request = dependencies.request || api;

  async function refreshLibrary() {
    clearRomSelection();
    await Promise.all([
      dependencies.loadPlatforms(), dependencies.loadStats(), dependencies.loadRoms(),
    ]);
  }

  async function previewSidecars() {
    if (state.sidecarCleanupLoading) return;
    state.sidecarCleanupLoading = true;
    dependencies.render();
    try {
      state.sidecarCleanupPreview = validateSidecarCleanupPreview(
        await request('/api/admin/library/sidecars'),
      );
    } catch (error) {
      state.sidecarCleanupPreview = null;
      dependencies.setError(error);
    } finally {
      state.sidecarCleanupLoading = false;
      dependencies.render();
    }
  }

  async function onSidecarCleanup(event) {
    event.preventDefault();
    const preview = state.sidecarCleanupPreview;
    const confirmation = String(new FormData(event.currentTarget).get('confirm') || '').trim();
    if (!preview?.files?.length || confirmation !== 'DELETE SIDECARS') {
      dependencies.setError(new Error('Confirmation must exactly match DELETE SIDECARS.'));
      dependencies.render();
      return;
    }
    if (!globalThis.confirm(`Delete the ${preview.file_count} previewed sidecar files? Folders, indexed files, and configured assets will remain.`)) return;
    try {
      const result = validateSidecarCleanupResult(await request('/api/admin/library/sidecars', {
        method: 'DELETE',
        body: JSON.stringify({ confirm: confirmation, files: preview.files }),
      }), preview);
      state.sidecarCleanupPreview = null;
      dependencies.setNotice(`Deleted ${result.deleted_file_count} sidecar file${result.deleted_file_count === 1 ? '' : 's'} through recoverable cleanup.`);
    } catch (error) {
      if (error?.status === 409) state.sidecarCleanupPreview = null;
      dependencies.setError(error);
    }
    dependencies.render();
  }

  async function onPlatformDelete(event) {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const platformId = String(form.get('platform_id') || '');
    const platform = state.platforms.find((candidate) => String(candidate.id) === platformId);
    const confirmation = String(form.get('confirm') || '').trim();
    if (!platform || confirmation !== platform.slug) {
      dependencies.setError(new Error(platform
        ? `Confirmation must exactly match ${platform.slug}.`
        : 'Select a platform with games to delete.'));
      dependencies.render();
      return;
    }
    if (!confirm(`Permanently delete all ${platform.display_name} games and their managed files?`)) return;
    try {
      const result = await request(`/api/admin/platforms/${encodeURIComponent(platform.id)}/roms?confirm=${encodeURIComponent(confirmation)}`, { method: 'DELETE' });
      await refreshLibrary();
      const count = result.deleted_rom_count || 0;
      dependencies.setNotice(`Deleted ${count} game${count === 1 ? '' : 's'} for ${platform.display_name}.`);
    } catch (error) {
      dependencies.setError(error);
    }
    dependencies.render();
  }

  async function onClearAll(event) {
    event.preventDefault();
    const confirmation = String(new FormData(event.currentTarget).get('confirm') || '').trim();
    if (confirmation !== 'DELETE ALL') {
      dependencies.setError(new Error('Confirmation must exactly match DELETE ALL.'));
      dependencies.render();
      return;
    }
    if (!confirm('Permanently delete every game and managed game file on this server?')) return;
    try {
      const result = await request(`/api/admin/roms?confirm=${encodeURIComponent(confirmation)}`, { method: 'DELETE' });
      await refreshLibrary();
      const count = result.deleted_rom_count || 0;
      dependencies.setNotice(`Cleared ${count} game${count === 1 ? '' : 's'} from the server library.`);
    } catch (error) {
      dependencies.setError(error);
    }
    dependencies.render();
  }

  function bind() {
    dependencies.app.querySelector('[data-action="preview-sidecar-cleanup"]')?.addEventListener('click', previewSidecars);
    dependencies.app.querySelector('#sidecar-cleanup-form')?.addEventListener('submit', onSidecarCleanup);
    dependencies.app.querySelector('#bulk-delete-platform-form')?.addEventListener('submit', onPlatformDelete);
    dependencies.app.querySelector('#clear-server-form')?.addEventListener('submit', onClearAll);
  }

  return { bind };
}
