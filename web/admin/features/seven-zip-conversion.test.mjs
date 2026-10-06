import test from 'node:test';
import assert from 'node:assert/strict';
import { canCompressSevenZip, setUploadFiles, setUploadPlatformId, state } from '../state.js';
import { UploadController } from './upload.js';
import { ConversionController } from './conversion.js';
import { renderUpload } from '../views/upload.js';
import { renderJobs } from '../views/jobs.js';

const keys = ['platforms', 'conversionStatus', 'uploadPlatformId', 'uploadSelectedFiles', 'uploadPlan', 'uploadCompressSevenZip', 'uploadCompressChd', 'uploadCompressRvz', 'uploadPreviewLoading', 'jobs', 'screen', 'auth'];
function prepare() {
  const before = Object.fromEntries(keys.map((key) => [key, state[key]]));
  state.platforms = [{ id: 1, slug: 'nes', display_name: 'NES' }, { id: 2, slug: 'psx' }, { id: 3, slug: 'win' }];
  state.conversionStatus = { enabled: true, formats: [{ platform_slug: 'nes', output: '7z', input_extensions: ['nes', 'unf', 'unif'] }] };
  state.auth = null;
  state.jobs = [];
  setUploadPlatformId('1'); setUploadFiles([]);
  return () => Object.assign(state, before);
}
const planFor = (files) => ({ errors: [], roms: files.map((file, i) => ({ plan_id: `rom-${i}`, title: `Edited title ${i}`, files: [{ original_file_name: file.name }] })) });

test('7z is opt-in and excludes archives, disc platforms, empty files and invalid grouping', () => {
  const restore = prepare();
  try {
    for (const [name, expected] of [['Game.NES', true], ['Game.zip', false], ['Game.7z', false], ['Game.rar', false], ['Game.nes.gz', false], ['Game.iso', false], ['Game.m3u', false], ['Game.nsz', false]]) {
      setUploadFiles([{ name, size: 10 }]);
      assert.equal(canCompressSevenZip(), expected, name);
      assert.equal(renderUpload().includes('id="upload-compress-seven-zip"'), expected);
    }
    setUploadFiles([{ name: 'Game.nes', size: 0 }]); assert.equal(canCompressSevenZip(), false);
    setUploadFiles([{ name: 'Game.nes', size: 10 }]);
    assert.doesNotMatch(renderUpload(), /id="upload-compress-seven-zip"[^>]*checked/u);
    state.uploadCompressSevenZip = true;
    assert.match(renderUpload(), /id="upload-compress-seven-zip"[^>]*checked/u);
    assert.match(renderUpload(), /data-action="finalize-upload-plan"[^>]*disabled/u);
    assert.match(renderUpload(), /Compress and import 7z/u);
    state.uploadPlan = { errors: [{}], roms: [] }; assert.equal(canCompressSevenZip(), false);
    state.uploadPlan = { errors: [], roms: [{ files: [{ original_file_name: 'Game.nes' }, { original_file_name: 'Other.nes' }] }] };
    assert.equal(canCompressSevenZip(), false);
    setUploadFiles([{ name: 'Game.nes', size: 10 }]); assert.equal(state.uploadCompressSevenZip, false);
    state.uploadCompressSevenZip = true; setUploadPlatformId('2');
    assert.equal(state.uploadCompressSevenZip, false); assert.equal(canCompressSevenZip(), false);
    setUploadPlatformId('3'); assert.equal(canCompressSevenZip(), false);
    setUploadPlatformId('1'); state.conversionStatus.enabled = false;
    assert.equal(canCompressSevenZip(), false);
  } finally { restore(); }
});

test('7z checkbox preserves compatible review and queues each raw ROM with its edited title', async () => {
  const restore = prepare();
  const originalFetch = globalThis.fetch;
  try {
    const files = [new File(['rom'], 'One.nes'), new File(['rom'], 'Two.NES')];
    setUploadFiles(files); state.uploadCompressSevenZip = true;
    const reviewed = planFor(files);
    state.auth = { header: 'Basic test' };
    globalThis.fetch = async () => new Response(JSON.stringify(reviewed), { headers: { 'Content-Type': 'application/json' } });
    const form = { elements: { platform_id: { value: '1' } } };
    const launched = [];
    const controls = {};
    const controller = new UploadController({
      app: { querySelector: (selector) => selector === '#upload-form' ? form : controls[selector], querySelectorAll: () => [] },
      render() {}, setNotice() {}, setError(error) { throw error; },
      launchConversion: async (selection) => launched.push(selection),
    });
    await controller.onPreviewPlan();
    assert.equal(state.uploadCompressSevenZip, true);
    await controller.onSubmit({ preventDefault() {}, currentTarget: form });
    assert.deepEqual(launched.map((selection) => [selection.platform, selection.title, selection.files[0].path]), [['nes', 'Edited title 0', 'One.nes'], ['nes', 'Edited title 1', 'Two.NES']]);
    assert.equal(state.uploadCompressSevenZip, false);
    assert.deepEqual(state.uploadSelectedFiles, []);
    controls['#upload-compress-seven-zip'] = { addEventListener(_name, callback) { this.change = callback; }, focus() { this.focused = true; } };
    setUploadFiles(files);
    controller.bindPlanControls();
    controls['#upload-compress-seven-zip'].change({ currentTarget: { checked: true } });
    assert.equal(state.uploadCompressSevenZip, true);
    assert.equal(controls['#upload-compress-seven-zip'].focused, true);
  } finally { globalThis.fetch = originalFetch; restore(); }
});

test('mixed selections compress raw ROMs and upload existing archives unchanged', async () => {
  const restore = prepare();
  const OriginalFormData = globalThis.FormData;
  const originalFetch = globalThis.fetch;
  try {
    const files = [new File(['rom'], 'Raw.nes'), new File(['archive'], 'Packed.7z')];
    setUploadFiles(files); state.uploadPlan = planFor(files); state.uploadCompressSevenZip = true;
    state.auth = { header: 'Basic test' };
    globalThis.fetch = async (_url, options) => {
      assert.deepEqual(JSON.parse(options.body).files.map((file) => file.file_name), ['Packed.7z']);
      return new Response(JSON.stringify(planFor([files[1]])), { headers: { 'Content-Type': 'application/json' } });
    };
    // Node FormData has no HTML form constructor. Keep native multipart behavior otherwise.
    globalThis.FormData = class extends OriginalFormData { constructor() { super(); } };
    const conversions = [];
    let uploaded;
    const controller = new UploadController({ app: { querySelector: () => null }, render() {}, setNotice() {}, setError(error) { throw error; }, launchConversion: async (selection) => conversions.push(selection) });
    controller.runUploadJob = async (_id, body) => { uploaded = body; };
    await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { platform_id: { value: '1' } } } });
    assert.deepEqual(conversions.map((selection) => selection.files[0].file.name), ['Raw.nes']);
    assert.deepEqual(uploaded.getAll('file').map((file) => file.name), ['Packed.7z']);
    assert.deepEqual(uploaded.getAll('planned_title').map(JSON.parse), [{ plan_id: 'rom-0', title: 'Edited title 1' }]);
  } finally { globalThis.FormData = OriginalFormData; globalThis.fetch = originalFetch; restore(); }
});

test('7z jobs use ROM labels, not Wii U folder or disc labels', async () => {
  const restore = prepare();
  try {
    const controller = new ConversionController({ render() {} });
    controller.startNextUpload = async () => {};
    await controller.launchFiles({ files: [{ file: new File(['rom'], 'Game.nes'), path: 'Game.nes' }], title: 'Game', platform: 'nes' });
    const job = state.jobs[0];
    assert.equal(job.outputFormat, '7z');
    assert.match(job.detail, /NES ROM/u);
    for (const phase of ['uploading', 'inspecting', 'compressing', 'verifying']) {
      job.progress = { phase };
      assert.match(renderJobs(), /7z ROM import/u);
      assert.doesNotMatch(renderJobs(), /Wii U|WUA|disc files|Uploading folders|game, update and DLC/u);
    }
  } finally { restore(); }
});
