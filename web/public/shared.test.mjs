import test from 'node:test';
import assert from 'node:assert/strict';

import {
  basicAuth, clearCoverImages, syncCoverImages, uniqueCoverPaths,
} from './shared.js';

test('cover loading deduplicates repeated resource paths', () => {
  const images = [
    { dataset: { coverPath: 'covers/game.jpg' } },
    { dataset: { coverPath: 'covers/game.jpg' } },
    { dataset: { coverPath: 'covers/other.jpg' } },
    { dataset: { coverPath: '' } },
  ];

  assert.deepEqual(uniqueCoverPaths(images), [
    'covers/game.jpg',
    'covers/other.jpg',
  ]);
});

test('basic auth encodes Unicode credentials as UTF-8', () => {
  const credentials = 'béacon:pässword';
  assert.equal(
    basicAuth('béacon', 'pässword'),
    `Basic ${Buffer.from(credentials, 'utf8').toString('base64')}`,
  );
});

test('cover loading waits until an image approaches the viewport', async () => {
  const observers = [];
  let fetchCount = 0;
  class TestIntersectionObserver {
    constructor(callback) {
      this.callback = callback;
      observers.push(this);
    }

    observe() {}

    unobserve() {}

    disconnect() {}
  }

  const previousObserver = globalThis.IntersectionObserver;
  const previousFetch = globalThis.fetch;
  const previousUrl = globalThis.URL;
  globalThis.IntersectionObserver = TestIntersectionObserver;
  globalThis.fetch = async () => {
    fetchCount += 1;
    return { ok: true, blob: async () => ({}) };
  };
  globalThis.URL = {
    createObjectURL: () => 'blob:cover',
    revokeObjectURL: () => {},
  };
  clearCoverImages();
  const image = {
    dataset: { coverPath: 'covers/lazy.jpg' },
    isConnected: true,
    src: '/admin/game-cover-placeholder.jpg',
  };

  try {
    syncCoverImages({ querySelectorAll: () => [image] }, 'Basic test');
    assert.equal(fetchCount, 0);
    assert.equal(image.src, '/admin/game-cover-placeholder.jpg');
    observers[0].callback(
      [{ isIntersecting: true, target: image }],
      observers[0],
    );
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(fetchCount, 1);
    assert.equal(image.src, 'blob:cover');
  } finally {
    clearCoverImages();
    globalThis.IntersectionObserver = previousObserver;
    globalThis.fetch = previousFetch;
    globalThis.URL = previousUrl;
  }
});
