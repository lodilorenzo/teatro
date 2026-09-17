import test from 'node:test';
import assert from 'node:assert/strict';

import { captureScroll, restoreScroll, scrollToTop } from './scroll.js';

function element(key, { top = 0, left = 0 } = {}) {
  return {
    dataset: { scrollKey: key },
    scrollTop: top,
    scrollLeft: left,
    scrollTo({ top: nextTop, left: nextLeft, behavior }) {
      this.behavior = behavior;
      this.scrollTop = nextTop;
      this.scrollLeft = nextLeft;
    },
  };
}

function root(elements) {
  return { querySelectorAll: () => elements };
}

function view(top = 0, left = 0) {
  return {
    scrollY: top,
    scrollX: left,
    scrollTo({ top: nextTop, left: nextLeft, behavior }) {
      this.behavior = behavior;
      this.scrollY = nextTop;
      this.scrollX = nextLeft;
    },
  };
}

test('a repaint of the same page restores the page and container offsets', () => {
  const before = [element('screen', { top: 640 }), element('notifications', { top: 24, left: 8 })];
  const page = view(120);
  const snapshot = captureScroll('admin:library', { root: root(before), view: page });

  // The repaint replaces the markup, so the restored elements start at the top.
  const after = [element('screen'), element('notifications')];
  const restoredView = view(0);
  assert.equal(restoreScroll(snapshot, 'admin:library', { root: root(after), view: restoredView }), true);
  assert.equal(after[0].scrollTop, 640);
  assert.equal(after[1].scrollTop, 24);
  assert.equal(after[1].scrollLeft, 8);
  assert.equal(restoredView.scrollY, 120);
  // Restoring a position is never animated.
  assert.equal(after[0].behavior, 'instant');
  assert.equal(restoredView.behavior, 'instant');
});

test('a repaint that lands on another page starts at the top', () => {
  const snapshot = captureScroll('admin:library', {
    root: root([element('screen', { top: 900 })]),
    view: view(300),
  });

  const after = [element('screen', { top: 900 })];
  const restoredView = view(300);
  assert.equal(restoreScroll(snapshot, 'admin:settings', { root: root(after), view: restoredView }), false);
  assert.equal(after[0].scrollTop, 0);
  assert.equal(restoredView.scrollY, 0);
});

test('containers absent from the snapshot open at their top and a missing snapshot changes nothing', () => {
  const snapshot = captureScroll('public:game::::7', { root: root([]), view: view(0) });
  const after = [element('rom-edit-modal', { top: 400 })];
  restoreScroll(snapshot, 'public:game::::7', { root: root(after), view: view(0) });
  assert.equal(after[0].scrollTop, 0);

  const untouched = [element('screen', { top: 55 })];
  const untouchedView = view(55);
  assert.equal(restoreScroll(null, 'public:platforms', { root: root(untouched), view: untouchedView }), false);
  assert.equal(untouched[0].scrollTop, 55);
  assert.equal(untouchedView.scrollY, 55);
});

test('elements without a scroll API fall back to direct assignment', () => {
  const plain = { dataset: { scrollKey: 'screen' }, scrollTop: 42, scrollLeft: 7 };
  scrollToTop({ root: root([plain]), view: view(10) });
  assert.equal(plain.scrollTop, 0);
  assert.equal(plain.scrollLeft, 0);
});
