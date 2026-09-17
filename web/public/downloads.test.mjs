import test from 'node:test';
import assert from 'node:assert/strict';

import { downloadArchive, downloadFile, submitDownloadTicket } from './downloads.js';

function captureForms(t) {
  const original = globalThis.document;
  const submitted = [];
  globalThis.document = {
    body: { append() {} },
    createElement(tagName) {
      assert.ok(['form', 'input'].includes(tagName), 'downloads must not create Blob or protected anchors');
      if (tagName === 'input') return {};
      return {
        append(input) { this.input = input; },
        submit() {
          submitted.push({ method: this.method, action: this.action, hidden: this.hidden, input: this.input });
        },
        remove() {},
      };
    },
  };
  t.after(() => { globalThis.document = original; });
  return submitted;
}

for (const kind of ['file', 'archive']) {
  test(`${kind} downloads exchange session auth for a native single-use form`, async (t) => {
    const forms = captureForms(t);
    const requests = [];
    const ticket = `teatro_dl_${'A'.repeat(43)}`;
    t.mock.method(globalThis, 'fetch', async (path, options) => {
      requests.push({ path, options });
      return new Response(JSON.stringify({ ticket, expires_in_seconds: 60 }), {
        headers: { 'content-type': 'application/json' },
      });
    });
    const sizes = kind === 'file' ? [0, 64 * 1024 * 1024 - 1, 64 * 1024 * 1024, 64 * 1024 * 1024 + 1, undefined] : [0];
    for (const size of sizes) {
      const notifications = [];
      const result = await (kind === 'file' ? downloadFile : downloadArchive)({
        romId: 8,
        file: { id: 9, file_name: 'Game #1.bin', file_size_bytes: size },
        authorization: 'Bearer teatro_session_test',
        notify: (message, level) => notifications.push(level),
      });
      assert.deepEqual(result, { mode: 'browser' });
      assert.deepEqual(notifications, ['progress', 'success']);
      assert.deepEqual(forms.at(-1), {
        method: 'post', action: `/api/downloads/${kind}`, hidden: true,
        input: { type: 'hidden', name: 'ticket', value: ticket },
      });
    }
    assert.equal(requests.length, sizes.length, 'only ticket JSON is fetched, never file bytes');
    for (const { path, options } of requests) {
      assert.equal(path, kind === 'file' ? '/api/roms/8/files/9/download-ticket' : '/api/roms/8/archive-ticket');
      assert.equal(options.method, 'POST');
      assert.equal(options.headers.get('Authorization'), 'Bearer teatro_session_test');
    }
  });
}

test('failed authorization and invalid tickets never submit a download', async (t) => {
  const forms = captureForms(t);
  const options = { romId: 1, file: { id: 2, file_name: 'game.bin' } };
  await assert.rejects(downloadFile(options), { status: 401 });
  t.mock.method(globalThis, 'fetch', async () => new Response(JSON.stringify({
    error: { code: 'unauthorized', message: 'Session expired' },
  }), { status: 401, headers: { 'content-type': 'application/json' } }));
  await assert.rejects(downloadFile({ ...options, authorization: 'Bearer expired' }), { status: 401 });
  for (const ticket of ['', 'bad', `teatro_dl_${'A'.repeat(42)}`, `teatro_dl_${'/'.repeat(43)}`]) {
    assert.throws(() => submitDownloadTicket(ticket, 'file'), /invalid download ticket/);
  }
  assert.throws(() => submitDownloadTicket(`teatro_dl_${'A'.repeat(43)}`, 'https://other.example'), /invalid download ticket/);
  assert.deepEqual(forms, []);
});
