// Both browser UIs repaint a page by replacing its markup wholesale, which resets every scroll
// offset to the top. The helpers below capture the offsets just before a repaint and reapply them
// afterwards, so an update caused by polling, a job snapshot, or a saved form never yanks the page
// out from under the reader. A repaint that lands on a different page scrolls to the top instead,
// which is what a reader expects from a fresh page.

// Containers that scroll inside the page opt in with `data-scroll-key`. The key only has to be
// unique within one page, and an unknown key is ignored rather than guessed at.
const CONTAINER_SELECTOR = '[data-scroll-key]';

function scrollElementTo(element, top, left) {
  // `behavior: instant` defeats the `scroll-behavior: smooth` rule; restoring a position is not a
  // movement the reader should watch.
  if (typeof element?.scrollTo === 'function') element.scrollTo({ top, left, behavior: 'instant' });
  else if (element) {
    element.scrollTop = top;
    element.scrollLeft = left;
  }
}

function containers(root) {
  return root?.querySelectorAll?.(CONTAINER_SELECTOR) || [];
}

export function captureScroll(pageKey, { root, view = globalThis } = {}) {
  const positions = [];
  for (const element of containers(root)) {
    const key = element.dataset?.scrollKey;
    if (!key) continue;
    positions.push({ key, top: element.scrollTop || 0, left: element.scrollLeft || 0 });
  }
  return {
    pageKey,
    page: { top: view?.scrollY || 0, left: view?.scrollX || 0 },
    positions,
  };
}

export function scrollToTop({ root, view = globalThis } = {}) {
  for (const element of containers(root)) scrollElementTo(element, 0, 0);
  scrollElementTo(view, 0, 0);
}

/// Reapplies a snapshot when the repaint stayed on the same page. A repaint that moved to another
/// page starts at the top, and a missing snapshot leaves the page alone.
export function restoreScroll(snapshot, pageKey, { root, view = globalThis } = {}) {
  if (!snapshot) return false;
  if (snapshot.pageKey !== pageKey) {
    scrollToTop({ root, view });
    return false;
  }

  const wanted = new Map(snapshot.positions.map((position) => [position.key, position]));
  for (const element of containers(root)) {
    const position = wanted.get(element.dataset?.scrollKey);
    // A container that did not exist in the snapshot is new markup and belongs at its top.
    scrollElementTo(element, position?.top || 0, position?.left || 0);
  }
  scrollElementTo(view, snapshot.page.top, snapshot.page.left);
  return true;
}
