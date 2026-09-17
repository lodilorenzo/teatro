import { api } from '../api.js';
import { clearAuth } from '../auth.js';
import { state } from '../state.js';
import {
  backgroundTransfersSupported, startBackgroundTransfer, waitForBackgroundTransfer,
} from './background-transfer.js';

const MAX_PERSISTED_UPLOAD_BYTES = 1024 * 1024 * 1024;

export function createUploadTransfer(context) {
  const dependencies = Object.freeze({ ...context });

  async function uploadWithProgress(form, options) {
    const uploadBytes = Number(options?.totalBytes || options?.fileSize || 0);
    // ponytail: IndexedDB duplicates selected files; keep huge uploads direct until browsers can persist file handles.
    if (options?.durable && uploadBytes <= MAX_PERSISTED_UPLOAD_BYTES && backgroundTransfersSupported()) {
      if (!state.auth?.header) throw new Error('Not signed in');
      dependencies.updateProgress({
        active: true,
        fileName: options.fileName || 'Uploading in background',
        loaded: 0,
        total: Number(options.totalBytes || options.fileSize || 0),
        percent: 0,
        speed: 0,
      });
      await startBackgroundTransfer(form, {
        ...options,
        authorization: state.auth.header,
      });
      try {
        return await waitForBackgroundTransfer(options.jobId, (progress) => {
          if (!progress) return;
          dependencies.updateProgress({
            ...progress,
            active: true,
            fileName: options.fileName || 'Uploading in background',
          });
        });
      } catch (error) {
        if (error?.status === 401) clearAuth();
        throw error;
      }
    }

    return new Promise((resolve, reject) => {
      if (!state.auth?.header) {
        reject(new Error('Not signed in'));
        return;
      }
      const xhr = new XMLHttpRequest();
      const startedAt = options?.batchStartedAt || performance.now();
      const completedBytes = options?.completedBytes || 0;
      const fileSize = options?.fileSize || 0;
      const totalBytes = options?.totalBytes || fileSize;
      xhr.open('POST', options?.url || '/api/admin/uploads');
      xhr.setRequestHeader('Accept', 'application/json');
      xhr.setRequestHeader('Authorization', state.auth.header);
      xhr.upload.addEventListener('progress', (event) => {
        const seconds = Math.max((performance.now() - startedAt) / 1000, 0.001);
        const loaded = Math.min(totalBytes, completedBytes + Math.min(fileSize || event.loaded, event.loaded));
        dependencies.updateProgress({
          active: true,
          fileName: options?.fileName || 'Uploading',
          fileIndex: options?.fileIndex || 0,
          fileCount: options?.fileCount || 0,
          loaded,
          total: totalBytes,
          percent: totalBytes > 0 ? Math.min(100, (loaded / totalBytes) * 100) : 0,
          speed: loaded / seconds,
        });
      });
      xhr.upload.addEventListener('load', () => options?.onUploadComplete?.());
      xhr.addEventListener('load', () => {
        let body = null;
        try {
          body = xhr.responseText ? JSON.parse(xhr.responseText) : null;
        } catch (_) {}
        if (xhr.status >= 200 && xhr.status < 300) {
          if (options?.expectedStatus && xhr.status !== options.expectedStatus) {
            const error = new Error(`Unexpected ${xhr.status} response from upload endpoint.`);
            error.status = xhr.status;
            reject(error);
            return;
          }
          const loaded = Math.min(totalBytes, completedBytes + fileSize);
          dependencies.updateProgress({
            active: true,
            loaded,
            total: totalBytes,
            percent: totalBytes > 0 ? Math.min(100, (loaded / totalBytes) * 100) : 100,
          });
          resolve(options?.includeResponseMetadata ? {
            body,
            status: xhr.status,
            location: xhr.getResponseHeader?.('Location') || null,
          } : body);
          return;
        }
        if (xhr.status === 401) clearAuth();
        const error = new Error(body?.error?.message || `${xhr.status} ${xhr.statusText || 'Upload failed'}`);
        error.status = xhr.status;
        error.code = body?.error?.code || null;
        reject(error);
      });
      xhr.addEventListener('error', () => reject(new Error('Upload failed due to a network error.')));
      xhr.addEventListener('abort', () => reject(new Error('Upload was aborted.')));
      xhr.send(form);
    });
  }

  async function autoApplyIgdbMetadata(upload) {
    if (!upload?.rom?.id || !upload?.rom?.name) {
      return { applied: false, reason: 'missing uploaded game details' };
    }
    if (!state.igdbStatus?.configured) await dependencies.loadIgdbStatus();
    if (!state.igdbStatus?.configured) {
      return { applied: false, reason: 'IGDB credentials are not configured' };
    }
    const platform = upload.rom.platform_slug;
    if (!platform) return { applied: false, reason: 'missing uploaded game platform' };
    const candidates = await api(`/api/admin/igdb/search?q=${encodeURIComponent(upload.rom.name)}&limit=10&platform=${encodeURIComponent(platform)}&require_platform_match=true`);
    const selected = candidates?.[0];
    if (!selected) {
      return { applied: false, reason: 'no IGDB candidate was found on the selected or any platform' };
    }
    const applied = await api(`/api/admin/roms/${encodeURIComponent(upload.rom.id)}/metadata/igdb`, {
      method: 'POST',
      body: JSON.stringify({ match: selected, cache_cover: true }),
    });
    return { applied: true, rom: applied.rom, selected };
  }

  return { autoApplyIgdbMetadata, uploadWithProgress };
}
