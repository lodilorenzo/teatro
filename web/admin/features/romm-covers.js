/// Remote cover thumbnails for the RomM browse list.
///
/// The cover route is an authenticated admin proxy, so a plain `<img src>` cannot load it: the
/// browser sends no Authorization header. Each cover is fetched with the admin credential and
/// handed to its image as an object URL, mirroring how the library loads Teatro's own covers.
/// Nothing here trusts a URL from the remote server; every request is built from the remote game
/// id Teatro already validated as an integer.

const COVER_CONCURRENCY = 4;
const coverUrls = new Map();
const pendingUrls = new Map();
let generation = 0;
let activeIds = new Set();
let activeLoads = 0;
let queue = [];

function rommCoverPath(remoteRomId) {
  return `/api/admin/sources/romm/roms/${encodeURIComponent(remoteRomId)}/cover`;
}

/// Loads the covers for the currently rendered browse list and releases the rest.
export function syncRommCovers(root, authorization) {
  const currentGeneration = ++generation;
  queue = [];

  const images = [...root.querySelectorAll('img[data-romm-cover]')];
  const ids = [...new Set(images.map((image) => image.dataset.rommCover).filter(Boolean))];
  activeIds = new Set(ids);
  for (const [id, url] of coverUrls) {
    if (activeIds.has(id)) continue;
    URL.revokeObjectURL(url);
    coverUrls.delete(id);
  }
  if (!authorization) return;

  const imagesById = new Map(ids.map((id) => [id, []]));
  for (const image of images) {
    const id = image.dataset.rommCover;
    if (imagesById.has(id)) imagesById.get(id).push(image);
  }
  for (const id of ids) {
    queue.push({
      id, images: imagesById.get(id), authorization, generation: currentGeneration,
    });
  }
  pumpQueue();
}

export function clearRommCovers() {
  generation += 1;
  queue = [];
  activeIds = new Set();
  for (const url of coverUrls.values()) URL.revokeObjectURL(url);
  coverUrls.clear();
}

function pumpQueue() {
  while (activeLoads < COVER_CONCURRENCY && queue.length) {
    const task = queue.shift();
    if (task.generation !== generation || !activeIds.has(task.id)) continue;
    activeLoads += 1;
    fetchCover(task.id, task.authorization, task.generation)
      .then((url) => {
        if (!url || task.generation !== generation) return;
        for (const image of task.images) {
          if (image.isConnected && image.dataset.rommCover === task.id) image.src = url;
        }
      })
      .finally(() => {
        activeLoads -= 1;
        pumpQueue();
      });
  }
}

async function fetchCover(id, authorization, requestedGeneration) {
  if (coverUrls.has(id)) return coverUrls.get(id);
  const key = `${requestedGeneration}:${id}`;
  if (pendingUrls.has(key)) return pendingUrls.get(key);

  const pending = (async () => {
    let objectUrl = null;
    try {
      // A remote server with no cover for this game answers 404; that is an ordinary outcome and
      // simply leaves the placeholder in place.
      const response = await fetch(rommCoverPath(id), {
        headers: { Authorization: authorization },
      });
      if (!response.ok) return null;
      objectUrl = URL.createObjectURL(await response.blob());
      if (requestedGeneration !== generation || !activeIds.has(id)) {
        URL.revokeObjectURL(objectUrl);
        return null;
      }
      coverUrls.set(id, objectUrl);
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
