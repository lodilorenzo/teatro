import test from 'node:test';
import assert from 'node:assert/strict';
import { canCompressRvz as canCompressGameCube, resetUploadWorkflow, setUploadFiles, setUploadPlatformId, state } from '../state.js';
import { UploadController } from './upload.js';
import { renderUpload } from '../views/upload.js';
import { renderConversionProgress } from '../views/conversion.js';
import { renderJobCardBody } from '../views/jobs.js';

const keys = ['platforms', 'uploadPlatformId', 'uploadSelectedFiles', 'uploadCompressRvz', 'uploadPlan', 'uploadPreviewLoading', 'conversionStatus'];
function prepare() {
  const before = Object.fromEntries(keys.map((key) => [key, state[key]]));
  Object.assign(state, { platforms: [{ id: 1, slug: 'gc', display_name: 'GameCube' }, { id: 2, slug: 'wii', display_name: 'Wii' }],
    uploadPlatformId: '1', uploadCompressRvz: false, uploadPlan: null,
    conversionStatus: { enabled: true, formats: [{ platform_slug: 'gc', output: 'rvz' }] } });
  return () => Object.assign(state, before);
}

test('GameCube checkbox appears only for one compatible ISO/GCM and resets on changed inputs', () => {
  const restore = prepare();
  try {
    for (const name of ['Game.ISO', 'Game.gcm']) {
      setUploadFiles([{ name, size: 100 }]);
      assert.equal(canCompressGameCube(), true);
      assert.match(renderUpload(), /id="upload-compress-rvz"/);
      assert.doesNotMatch(renderUpload(), /id="upload-compress-rvz"[^>]*disabled/);
      state.uploadCompressRvz = true;
      assert.match(renderUpload(), /Compress and import RVZ/);
      assert.match(renderUpload(), /id="upload-compress-rvz"[^>]*checked/);
      assert.doesNotMatch(renderUpload(), /data-action="finalize-upload-plan"[^>]*disabled/);
    }
    for (const files of [[], [{ name: 'Game.rvz' }], [{ name: 'Game.gcz' }], [{ name: 'Game.nkit.iso' }], [{ name: 'Game.iso' }, { name: 'Disc2.iso' }]]) {
      setUploadFiles(files);
      assert.equal(state.uploadCompressRvz, false);
      assert.equal(canCompressGameCube(), false);
      assert.doesNotMatch(renderUpload(), /upload-compress-rvz|upload-rvz-hint/);
    }
    setUploadFiles([{ name: 'Game.iso' }]);
    state.uploadCompressRvz = true;
    setUploadPlatformId('2');
    assert.equal(state.uploadCompressRvz, false);
    assert.doesNotMatch(renderUpload(), /upload-compress-rvz/);
    setUploadPlatformId('1');
    state.conversionStatus.enabled = false;
    assert.equal(canCompressGameCube(), false);
    assert.doesNotMatch(renderUpload(), /upload-compress-rvz|upload-rvz-hint/);
    state.uploadCompressRvz = true;
    resetUploadWorkflow();
    assert.equal(state.uploadCompressRvz, false);
  } finally { restore(); }
});

test('checked GameCube upload uses staged conversion, preserves edited title, and clears selection', async () => {
  const restore = prepare();
  const requests = [];
  try {
    const file = new File(['disc'], 'Game.iso');
    setUploadFiles([file]);
    state.uploadCompressRvz = true;
    state.uploadPlan = { roms: [{ title: 'Edited title' }] };
    const controller = new UploadController({ app: { querySelector: () => null, querySelectorAll: () => [] }, render() {},
      setError(error) { throw error; }, launchConversion: async (selection) => requests.push(selection) });
    controller.runUploadJob = () => assert.fail('Ordinary upload must not run');
    controller.onPreviewPlan = () => assert.fail('Conversion must inspect uploaded bytes, not filenames');
    await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { platform_id: { value: '1' } } } });
    assert.deepEqual(requests, [{ files: [{ file, path: 'Game.iso' }], title: 'Edited title', platform: 'gc' }]);
    assert.deepEqual(state.uploadSelectedFiles, []);
    assert.equal(state.uploadCompressRvz, false);
    assert.equal(state.uploadPlan, null);
  } finally { restore(); }
});

test('RVZ Jobs shows verification without confirmation controls', () => {
  const job = { id: 'rvz-job', type: 'conversion-import', state: 'running', outputFormat: 'rvz', progress: { phase: 'verifying' } };
  let markup = renderConversionProgress(job);
  assert.doesNotMatch(markup, /data-conversion-confirm|Publish larger|confirmation/);
  assert.match(markup, /Verifying RVZ/);
  assert.match(markup, /every decoded disc byte/);
  assert.doesNotMatch(markup, /WUA|Cemu/);
  markup = renderJobCardBody({ ...job, state: 'succeeded', result: { title: 'Disc', file_name: 'Disc.rvz', input_bytes: 1024, output_bytes: 512 } });
  assert.match(markup, /Dolphin conversion import/);
  assert.match(markup, /512 B RVZ from/);
  assert.doesNotMatch(markup, /WUA|Wii U/);
});
