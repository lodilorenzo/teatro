import test from 'node:test';
import assert from 'node:assert/strict';

import { ApiRequestError, apiRequest } from './api.js';

test('public library API requests carry the active authorization header', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async (path, options) => {
    assert.equal(path, '/api/platforms');
    assert.equal(options.headers.get('Authorization'), 'Basic encoded');
    assert.equal(options.headers.get('Accept'), 'application/json');
    return new Response(JSON.stringify([{ id: 1 }]), {
      headers: { 'content-type': 'application/json' },
    });
  };

  try {
    assert.deepEqual(await apiRequest('/api/platforms', 'Basic encoded'), [{ id: 1 }]);
  } finally {
    globalThis.fetch = originalFetch;
  }
});

test('public library API errors retain status and stable server code', async () => {
  const originalFetch = globalThis.fetch;
  globalThis.fetch = async () => new Response(JSON.stringify({
    error: { code: 'unauthorized', message: 'Sign in again.' },
  }), {
    status: 401,
    headers: { 'content-type': 'application/json' },
  });

  try {
    await assert.rejects(
      apiRequest('/api/users/me', 'Basic invalid'),
      (error) => error instanceof ApiRequestError
        && error.status === 401
        && error.code === 'unauthorized'
        && error.message === 'Sign in again.',
    );
  } finally {
    globalThis.fetch = originalFetch;
  }
});
