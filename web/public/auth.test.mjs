import test from 'node:test';
import assert from 'node:assert/strict';
import { createSession, revokeSession } from './auth.js';

test('login exchanges the password for a session and logout revokes only that session', async () => {
  const previous = globalThis.fetch;
  const calls = [];
  let status = 201;
  globalThis.fetch = async (path, options) => {
    calls.push({ path, ...options });
    return new Response(JSON.stringify(status === 201 ? {
      user: { id: 1, username: 'admin', role: 'admin' }, token: 'teatro_session_test', expires_at: 2000000000,
    } : { error: { message: 'Failed' } }), { status, headers: { 'content-type': 'application/json' } });
  };
  try {
    for (const remember of [false, true]) {
      const { auth, user } = await createSession('admin', 'password', remember);
      assert.deepEqual(auth, { username: 'admin', header: 'Bearer teatro_session_test', expiresAt: 2000000000000 });
      assert.equal(user.role, 'admin');
      const call = calls.at(-1);
      assert.equal(call.path, '/api/auth/session');
      assert.equal(call.method, 'POST');
      assert.equal(call.headers.get('Authorization'), `Basic ${btoa('admin:password')}`);
      assert.deepEqual(JSON.parse(call.body), { remember_me: remember });
      await revokeSession(auth);
      assert.equal(calls.at(-1).method, 'DELETE');
      assert.equal(calls.at(-1).headers.get('Authorization'), auth.header);
    }
    status = 401;
    await assert.rejects(createSession('admin', 'wrong'), (error) => error.status === 401);
    await assert.doesNotReject(revokeSession({ header: 'Bearer teatro_session_expired' }));
    status = 500;
    await assert.rejects(revokeSession({ header: 'Bearer teatro_session_test' }), /Failed/);
  } finally {
    globalThis.fetch = previous;
  }
});
