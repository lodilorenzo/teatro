import test from 'node:test';
import assert from 'node:assert/strict';

import { api } from './api.js';
import { state } from './state.js';

test('admin API supplies JSON defaults and clears auth on shared 401 errors', async () => {
  const previous = { auth: state.auth, user: state.user, fetch: globalThis.fetch };
  let request;
  let calls = 0;
  state.auth = { header: 'Basic test' };
  state.user = { id: 1, username: 'admin' };
  globalThis.fetch = async (path, options) => {
    request = { path, options };
    calls += 1;
    if (calls === 1) {
      return new Response(JSON.stringify({ enabled: true }), {
        headers: { 'content-type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({
      error: { code: 'unauthorized', message: 'Authentication failed. Sign in again.' },
    }), { status: 401, headers: { 'content-type': 'application/json' } });
  };

  try {
    assert.deepEqual(await api('/api/admin/example', {
      method: 'POST', body: '{"enabled":true}',
    }), { enabled: true });
    assert.equal(request.options.headers.get('Authorization'), 'Basic test');
    assert.equal(request.options.headers.get('Content-Type'), 'application/json');

    await assert.rejects(api('/api/admin/example'), (error) => error.status === 401
      && error.code === 'unauthorized'
      && error.message === 'Authentication failed. Sign in again.');
    assert.equal(state.auth, null);
    assert.equal(state.user, null);
  } finally {
    state.auth = previous.auth;
    state.user = previous.user;
    globalThis.fetch = previous.fetch;
  }
});
