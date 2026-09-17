export { VERSION_LABEL } from './version.js';

export function html(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}

export function attr(value) {
  return html(value);
}

export function formatBytes(bytes) {
  const value = Number(bytes || 0);
  if (!Number.isFinite(value) || value <= 0) return '0 B';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`;
}

export function metadataList(item, key) {
  const value = item?.metadatum?.[key];
  return Array.isArray(value)
    ? value.filter((entry) => entry !== null && entry !== undefined && String(entry)).map(String)
    : [];
}

export function metadataText(item, key) {
  const value = item?.metadatum?.[key];
  return value === null || value === undefined ? '' : String(value);
}

export function encodeResourcePath(path) {
  return String(path || '').split('/').map(encodeURIComponent).join('/');
}

export function basicAuth(username, password) {
  const bytes = new TextEncoder().encode(`${username}:${password}`);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return `Basic ${btoa(binary)}`;
}

const COVER_CONCURRENCY = 4;
const coverUrls = new Map();
const pendingUrls = new Map();
let generation = 0;
let activePaths = new Set();
let observer = null;
let activeLoads = 0;
let queue = [];

export function uniqueCoverPaths(images) {
  return [...new Set(images.map((image) => image.dataset.coverPath).filter(Boolean))];
}

export function syncCoverImages(root, authorization) {
  const currentGeneration = ++generation;
  observer?.disconnect();
  observer = null;
  queue = [];

  const images = [...root.querySelectorAll('img[data-cover-path]')];
  const paths = uniqueCoverPaths(images);
  activePaths = new Set(paths);
  for (const [path, url] of coverUrls) {
    if (activePaths.has(path)) continue;
    URL.revokeObjectURL(url);
    coverUrls.delete(path);
  }
  if (!authorization) return;

  const imagesByPath = new Map(paths.map((path) => [path, []]));
  for (const image of images) {
    const path = image.dataset.coverPath;
    if (imagesByPath.has(path)) imagesByPath.get(path).push(image);
  }

  const enqueue = (path) => {
    if (queue.some((task) => task.generation === currentGeneration && task.path === path)) return;
    queue.push({
      path,
      images: imagesByPath.get(path),
      authorization,
      generation: currentGeneration,
    });
    pumpQueue();
  };

  if (typeof IntersectionObserver !== 'function') {
    for (const path of paths) enqueue(path);
    return;
  }

  const enqueued = new Set();
  observer = new IntersectionObserver((entries, activeObserver) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      activeObserver.unobserve(entry.target);
      const path = entry.target.dataset.coverPath;
      if (!path || enqueued.has(path)) continue;
      enqueued.add(path);
      enqueue(path);
    }
  }, { rootMargin: '200px' });
  for (const image of images) observer.observe(image);
}

export function clearCoverImages() {
  generation += 1;
  observer?.disconnect();
  observer = null;
  queue = [];
  activePaths = new Set();
  for (const url of coverUrls.values()) URL.revokeObjectURL(url);
  coverUrls.clear();
}

function pumpQueue() {
  while (activeLoads < COVER_CONCURRENCY && queue.length) {
    const task = queue.shift();
    if (task.generation !== generation || !activePaths.has(task.path)) continue;
    activeLoads += 1;
    fetchCover(task.path, task.authorization, task.generation)
      .then((url) => {
        if (!url || task.generation !== generation) return;
        for (const image of task.images) {
          if (image.isConnected && image.dataset.coverPath === task.path) image.src = url;
        }
      })
      .finally(() => {
        activeLoads -= 1;
        pumpQueue();
      });
  }
}

async function fetchCover(path, authorization, requestedGeneration) {
  if (coverUrls.has(path)) return coverUrls.get(path);
  const key = `${requestedGeneration}:${path}`;
  if (pendingUrls.has(key)) return pendingUrls.get(key);

  const pending = (async () => {
    let objectUrl = null;
    try {
      const response = await fetch(`/assets/romm/resources/${encodeResourcePath(path)}`, {
        headers: { Authorization: authorization },
      });
      if (!response.ok) return null;
      objectUrl = URL.createObjectURL(await response.blob());
      if (requestedGeneration !== generation || !activePaths.has(path)) {
        URL.revokeObjectURL(objectUrl);
        return null;
      }
      coverUrls.set(path, objectUrl);
      return objectUrl;
    } catch (_) {
      if (objectUrl) URL.revokeObjectURL(objectUrl);
      return null;
    } finally {
      pendingUrls.delete(key);
    }
  })();
  pendingUrls.set(key, pending);
  return pending;
}
