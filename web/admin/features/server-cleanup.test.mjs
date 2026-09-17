import test from 'node:test';
import assert from 'node:assert/strict';

import {
  validateSidecarCleanupPreview, validateSidecarCleanupResult,
} from './server-cleanup.js';
import { state } from '../state.js';
import { renderServerCleanup } from '../views/server-cleanup.js';

function preview() {
  return {
    file_count: 1,
    total_file_bytes: 4,
    truncated: false,
    files: [{
      relative_path: 'psx/<script>.txt',
      file_size_bytes: 4,
      fingerprint: '4:123456789',
    }],
  };
}

test('sidecar cleanup validates and escapes the exact preview', () => {
  const value = preview();
  assert.equal(validateSidecarCleanupPreview(value), value);
  assert.equal(validateSidecarCleanupResult({
    deleted_file_count: 1,
    deleted_file_bytes: 4,
    deleted_files: ['psx/<script>.txt'],
  }, value).deleted_file_count, 1);

  const previous = state.sidecarCleanupPreview;
  state.sidecarCleanupPreview = value;
  const markup = renderServerCleanup();
  state.sidecarCleanupPreview = previous;
  assert.match(markup, /psx\/&lt;script&gt;\.txt/);
  assert.match(markup, /DELETE SIDECARS/);
  assert.doesNotMatch(markup, /psx\/<script>/);
});

test('sidecar cleanup rejects unsafe or mismatched responses', () => {
  assert.throws(
    () => validateSidecarCleanupPreview({
      ...preview(), files: [{ ...preview().files[0], relative_path: '/private/secret.txt' }],
    }),
    /unsafe sidecar cleanup target/,
  );
  assert.throws(
    () => validateSidecarCleanupResult({
      deleted_file_count: 1,
      deleted_file_bytes: 4,
      deleted_files: ['psx/other.txt'],
    }, preview()),
    /invalid sidecar cleanup result/,
  );
});
