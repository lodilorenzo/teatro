import { clearCoverImages } from '../../public/shared.js';
import { api } from '../api.js';
import { replaceRom, setIgdbResults, setIgdbSettings, state } from '../state.js';

export function createIgdbController(context) {
  const dependencies = Object.freeze({ ...context });

  function setFeedback(message, error = false) {
    state.igdbFeedback = { message, error };
    // Update progress without repainting the editor and discarding unsaved fields.
    const feedback = dependencies.app?.querySelector('#igdb-feedback');
    if (feedback) {
      feedback.className = error ? 'error' : 'muted';
      feedback.setAttribute('role', error ? 'alert' : 'status');
      feedback.textContent = message;
    }
  }

  return {
    async onSettingsSubmit(event) {
      event.preventDefault();
      const form = new FormData(event.currentTarget);
      const clientId = String(form.get('client_id') || '').trim();
      const clientSecret = String(form.get('client_secret') || '').trim();
      const payload = { client_id: clientId };
      if (clientSecret) payload.client_secret = clientSecret;
      try {
        setIgdbSettings(await api('/api/admin/igdb/settings', {
          method: 'PATCH',
          body: JSON.stringify(payload),
        }));
        await dependencies.loadIgdbStatus();
        dependencies.setNotice('IGDB credentials saved. The secret remains write-only.');
      } catch (error) {
        dependencies.setError(error);
      }
      dependencies.render();
    },

    async onSettingsClear() {
      try {
        setIgdbSettings(await api('/api/admin/igdb/settings', { method: 'DELETE' }));
        await dependencies.loadIgdbStatus();
        dependencies.setNotice(state.igdbSettings.configured
          ? 'Stored IGDB credentials cleared. Environment credentials are still active.'
          : 'IGDB credentials cleared.');
      } catch (error) {
        dependencies.setError(error);
      }
      dependencies.render();
    },

    async onSearchSubmit(event) {
      event.preventDefault();
      const form = new FormData(event.currentTarget);
      const q = String(form.get('q') || '').trim();
      const limit = String(form.get('limit') || '10');
      if (!q) return;
      const selectionId = state.romSelectionRequestId;
      setIgdbResults([]);
      setFeedback('Searching IGDB…');
      try {
        const searchAllPlatforms = event.submitter?.dataset.searchAllPlatforms === 'true';
        const platform = searchAllPlatforms ? null : state.selectedRom?.platform_slug;
        const platformQuery = platform ? `&platform=${encodeURIComponent(platform)}` : '';
        const results = await api(`/api/admin/igdb/search?q=${encodeURIComponent(q)}&limit=${encodeURIComponent(limit)}${platformQuery}`);
        if (selectionId !== state.romSelectionRequestId) return;
        setIgdbResults(results);
        setFeedback(results.length
          ? `Found ${results.length} IGDB candidate(s)${searchAllPlatforms ? ' across all platforms' : ''}.`
          : 'No matching games found on IGDB. Try a different search query.');
      } catch (error) {
        if (selectionId !== state.romSelectionRequestId) return;
        setFeedback(error?.message || String(error), true);
      }
      dependencies.render();
    },

    async apply(romId, index) {
      const candidate = state.igdbResults[index];
      if (!candidate) return;
      const selectionId = state.romSelectionRequestId;
      setFeedback('Applying IGDB metadata…');
      try {
        const applied = await api(`/api/admin/roms/${encodeURIComponent(romId)}/metadata/igdb`, {
          method: 'POST',
          body: JSON.stringify({ match: candidate, cache_cover: true }),
        });
        if (selectionId !== state.romSelectionRequestId) return;
        replaceRom(applied.rom);
        setIgdbResults([]);
        clearCoverImages();
        setFeedback(`Applied IGDB metadata to ${applied.rom.name}.`);
      } catch (error) {
        if (selectionId !== state.romSelectionRequestId) return;
        setFeedback(error?.message || String(error), true);
      }
      dependencies.render();
    },
  };
}
