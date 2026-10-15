import { api } from '../api.js';
import { attr, html, slugifyTitle } from '../dom.js';
import { state } from '../state.js';
import { renderPagination } from '../../public/pagination.js';

const PAGE_SIZE = 50;

export function createCollectionsController({ app, render, setNotice, setError }) {
  let collections = [];
  let editor = null;
  let page = { items: [], total: 0, limit: PAGE_SIZE, offset: 0 };
  let busy = false;
  let generation = 0;

  async function load() {
    const auth = state.auth;
    const result = await api('/api/collections');
    if (auth === state.auth) collections = result;
  }

  async function loadGames() {
    const params = new URLSearchParams({ limit: String(PAGE_SIZE), offset: String(editor.offset), search: editor.search });
    const activeEditor = editor;
    const current = generation;
    const result = await api(`/api/roms?${params}`);
    if (current === generation && editor === activeEditor) page = result;
  }

  async function run(action) {
    if (busy) return;
    busy = true;
    render();
    const current = ++generation;
    try { await action(() => current === generation); }
    catch (error) { if (current === generation) setError(error); }
    finally {
      if (current === generation) { busy = false; render(); }
    }
  }

  function renderScreen() {
    if (!editor) return `
      <section class="page-header"><h1>Collections</h1><button class="primary" data-collection-new ${busy ? 'disabled' : ''}>Create collection</button></section>
      <p class="hint">Group games and share a dedicated URL. Regular collections require sign-in.</p>
      <div class="collection-list">${collections.length ? collections.map((collection) => `
        <div class="collection-row">
          <div><h2>${html(collection.name)}</h2><p>${collection.rom_count} games · ${collection.lan_party_zone ? 'LAN Party Zone' : 'Sign-in required'}</p><a class="text-link" href="/${attr(collection.slug)}" target="_blank" rel="noopener">/${html(collection.slug)}</a></div>
          <div class="actions"><button data-collection-edit="${collection.id}" ${busy ? 'disabled' : ''}>Edit<span class="visually-hidden"> ${html(collection.name)}</span></button><button class="danger" data-collection-delete="${collection.id}" ${busy ? 'disabled' : ''}>Delete<span class="visually-hidden"> ${html(collection.name)}</span></button></div>
        </div>`).join('') : '<p>No collections yet. Create one to choose games for your players.</p>'}</div>`;
    return `
      <section class="page-header"><h1>${editor.id ? 'Edit collection' : 'Create collection'}</h1></section>
      <form id="collection-form" class="collection-editor">
        <fieldset ${busy ? 'disabled' : ''}>
          <label><span>Name</span><input name="name" value="${attr(editor.name)}" maxlength="100" required /></label>
          <label><span>URL name</span><input name="slug" value="${attr(editor.slug)}" maxlength="80" pattern="[a-z0-9]+(-[a-z0-9]+)*" required aria-describedby="collection-url-help" /></label>
          <p id="collection-url-help" class="hint">${html(window.location.origin)}/<span data-collection-url>${html(editor.slug || 'collection-name')}</span>. Changing this name changes the shared URL.</p>
          <label class="checkbox"><input name="lan_party_zone" type="checkbox" ${editor.lan_party_zone ? 'checked' : ''} aria-describedby="collection-access-help" /> LAN Party Zone</label>
          <p id="collection-access-help" class="collection-access-help">Anyone who can reach this server and URL can browse and download the selected games without signing in when LAN Party Zone is enabled. This is not restricted to the local network.</p>
          <div class="actions"><button class="primary" type="submit">${busy ? 'Saving…' : 'Save collection'}</button><button type="button" data-collection-cancel>Cancel</button><span data-collection-count role="status">${editor.ids.size} selected</span></div>
        </fieldset>
      </form>
      <form id="collection-search" class="filters"><label><span>Find games</span><input name="search" value="${attr(editor.search)}" maxlength="200" type="search" placeholder="Search game titles" /></label><button ${busy ? 'disabled' : ''}>Search</button></form>
      <section aria-label="Choose games" class="collection-picker" aria-busy="${busy}">
        <p class="hint">Selections stay checked across searches and pages. Up to 10,000 games per collection.</p>
        ${page.items.length ? page.items.map((game) => `<label class="collection-game"><input type="checkbox" data-collection-game="${game.id}" ${editor.ids.has(game.id) ? 'checked' : ''} ${busy ? 'disabled' : ''} /><span><strong>${html(game.name)}</strong><small>${html(game.platform_display_name)}</small></span></label>`).join('') : '<p>No games found. Try another title or import games into the library.</p>'}
        ${renderPagination(page)}
      </section>`;
  }

  function bind() {
    app.querySelector('[data-collection-new]')?.addEventListener('click', () => run(async () => {
      editor = { name: '', slug: '', lan_party_zone: false, ids: new Set(), search: '', offset: 0, slugEdited: false };
      await loadGames();
    }));
    app.querySelectorAll('[data-collection-edit]').forEach((button) => button.addEventListener('click', () => run(async (current) => {
      const collection = collections.find((item) => String(item.id) === button.dataset.collectionEdit);
      const detail = await api(`/api/collections/${encodeURIComponent(collection.slug)}`);
      if (!current()) return;
      editor = { ...collection, ids: new Set(detail.rom_ids), search: '', offset: 0, slugEdited: true };
      await loadGames();
    })));
    app.querySelectorAll('[data-collection-delete]').forEach((button) => button.addEventListener('click', () => {
      const collection = collections.find((item) => String(item.id) === button.dataset.collectionDelete);
      if (!globalThis.confirm(`Delete "${collection.name}"? Its shared URL will stop working. Game files will not be deleted.`)) return;
      void run(async (current) => {
        await api(`/api/admin/collections/${collection.id}`, { method: 'DELETE' });
        if (!current()) return;
        await load();
        if (!current()) return;
        setNotice('Collection deleted. Game files were kept.');
      });
    }));
    app.querySelector('[data-collection-cancel]')?.addEventListener('click', () => { editor = null; render(); });
    const form = app.querySelector('#collection-form');
    form?.addEventListener('input', (event) => {
      const input = event.target;
      if (input.name === 'name') {
        editor.name = input.value;
        if (!editor.slugEdited) {
          editor.slug = input.value ? slugifyTitle(input.value).slice(0, 80).replace(/-+$/, '') : '';
          form.elements.slug.value = editor.slug;
        }
      }
      if (input.name === 'slug') { editor.slug = input.value; editor.slugEdited = true; }
      if (input.name === 'lan_party_zone') editor.lan_party_zone = input.checked;
      form.querySelector('[data-collection-url]').textContent = editor.slug || 'collection-name';
    });
    form?.addEventListener('submit', (event) => {
      event.preventDefault();
      void run(async (current) => {
        await api(`/api/admin/collections${editor.id ? `/${editor.id}` : ''}`, {
          method: editor.id ? 'PUT' : 'POST',
          body: JSON.stringify({ name: editor.name, slug: editor.slug, lan_party_zone: editor.lan_party_zone, rom_ids: [...editor.ids] }),
        });
        if (!current()) return;
        editor = null;
        await load();
        if (!current()) return;
        setNotice('Collection saved. Its URL is ready to share.');
      });
    });
    app.querySelector('#collection-search')?.addEventListener('input', (event) => { editor.search = event.target.value; });
    app.querySelector('#collection-search')?.addEventListener('submit', (event) => {
      event.preventDefault();
      void run(async () => { editor.offset = 0; await loadGames(); });
    });
    app.querySelectorAll('[data-collection-game]').forEach((input) => input.addEventListener('change', () => {
      const id = Number(input.dataset.collectionGame);
      if (input.checked && editor.ids.size >= 10_000) {
        input.checked = false;
        setError(new Error('Select at most 10,000 games.'));
        render();
        return;
      }
      if (input.checked) editor.ids.add(id);
      else editor.ids.delete(id);
      app.querySelector('[data-collection-count]').textContent = `${editor.ids.size} selected`;
    }));
    app.querySelectorAll('.collection-picker [data-page-offset]').forEach((button) => button.addEventListener('click', () => run(async () => {
      editor.offset = Number(button.dataset.pageOffset);
      await loadGames();
    })));
  }

  return { load, render: renderScreen, bind, reset() { generation++; collections = []; editor = null; page = { items: [], total: 0, limit: PAGE_SIZE, offset: 0 }; busy = false; } };
}
