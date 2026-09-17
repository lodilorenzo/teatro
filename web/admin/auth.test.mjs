import test from 'node:test';
import assert from 'node:assert/strict';

import { clearAuth, loadSavedAuth as loadAdminAuth, saveAuth as saveAdminAuth } from './auth.js';
import { clearSavedAuth, loadSavedAuth as loadPublicAuth, saveAuth as savePublicAuth } from '../public/auth.js';
import { state } from './state.js';

function storage() {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
  };
}

const auth = { username: 'admin', header: 'Bearer teatro_session_test', expiresAt: Date.now() + 86400000 };

test('sign-in and sign-out are shared in both directions, with opt-in restart persistence', () => {
  const previous = { sessionStorage: globalThis.sessionStorage, localStorage: globalThis.localStorage };
  const previousState = { auth: state.auth, user: state.user, romSelectionRequestId: state.romSelectionRequestId };
  globalThis.sessionStorage = storage();
  globalThis.localStorage = storage();
  try {
    for (const save of [saveAdminAuth, savePublicAuth]) {
      for (const remember of [false, true]) {
        save(auth, remember);
        assert.deepEqual(loadAdminAuth(), auth);
        assert.deepEqual(loadPublicAuth(), auth);
        assert.equal(localStorage.getItem('teatro.auth.v2') !== null, remember);
        sessionStorage.clear(); // New tab or browser restart.
        assert.deepEqual(loadAdminAuth(), remember ? auth : null);
        assert.deepEqual(loadPublicAuth(), remember ? auth : null);
        save(auth, remember);
        clearAuth();
        assert.equal(loadPublicAuth(), null);
        save(auth, remember);
        clearSavedAuth();
        assert.equal(loadAdminAuth(), null);
      }
    }
    savePublicAuth(auth, true);
    saveAdminAuth({ ...auth, username: 'other' });
    assert.equal(localStorage.getItem('teatro.auth.v2'), null);
    sessionStorage.clear();
    assert.equal(loadPublicAuth(), null, 'unchecked sign-in must not revive an older remembered account');
  } finally {
    Object.assign(globalThis, previous);
    Object.assign(state, previousState);
  }
});

test('expired, malformed and legacy credentials are discarded; unavailable storage is tolerated', () => {
  const previous = { sessionStorage: globalThis.sessionStorage, localStorage: globalThis.localStorage };
  globalThis.sessionStorage = storage();
  globalThis.localStorage = storage();
  try {
    for (const value of ['{', '{}', JSON.stringify({ ...auth, expiresAt: 1 }),
      JSON.stringify({ ...auth, header: 'Basic password' })]) {
      sessionStorage.setItem('teatro.auth.v2', value);
      localStorage.setItem('teatro.auth.v2', value);
      assert.equal(loadPublicAuth(), null);
    }
    sessionStorage.setItem('teatro.auth.v1', JSON.stringify({ username: 'admin', header: 'Basic password' }));
    assert.equal(loadPublicAuth(), null);
    assert.equal(sessionStorage.getItem('teatro.auth.v1'), null);
    globalThis.localStorage = {
      getItem() { throw new Error('blocked'); },
      setItem() { throw new Error('blocked'); },
      removeItem() { throw new Error('blocked'); },
    };
    savePublicAuth(auth, true);
    assert.deepEqual(loadPublicAuth(), auth, 'blocked persistence falls back to tab storage');
    clearSavedAuth();
    assert.equal(loadPublicAuth(), null);
    globalThis.sessionStorage = globalThis.localStorage;
    assert.doesNotThrow(() => savePublicAuth(auth));
    assert.equal(loadPublicAuth(), null);
    assert.doesNotThrow(clearSavedAuth);
  } finally {
    Object.assign(globalThis, previous);
  }
});
