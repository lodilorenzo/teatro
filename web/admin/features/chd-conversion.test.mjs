import test from 'node:test';
import assert from 'node:assert/strict';
import { canCompressChd, chdFormat, setUploadFiles, setUploadPlatformId, state } from '../state.js';
import { UploadController, chdInputsCompatible } from './upload.js';
import { renderUpload } from '../views/upload.js';
import { renderJobs } from '../views/jobs.js';

const plan = (files, errors = []) => ({ errors, roms: [{ plan_id: 'rom-1', title: 'Edited title', files: files.map((name) => ({ original_file_name: name, launchable: /\.(cue|iso|m3u)$/u.test(name) })) }] });
const keys = ['platforms', 'conversionStatus', 'uploadPlatformId', 'uploadSelectedFiles', 'uploadPlan', 'uploadCompressChd', 'uploadCompressRvz', 'uploadPreviewLoading', 'jobs'];
function prepare() {
  const before = Object.fromEntries(keys.map((key) => [key, state[key]]));
  state.platforms = [{ id: 1, slug: 'psx' }, { id: 2, slug: 'ps2' }, { id: 3, slug: 'psp' }, { id: 4, slug: 'dreamcast' }];
  state.conversionStatus = { enabled: true, formats: ['psx', 'ps2', 'psp'].map((platform_slug) => ({ platform_slug, output: 'chd', emulator: 'Test emulator' })) };
  setUploadPlatformId('1'); setUploadFiles([]);
  return () => Object.assign(state, before);
}
test('CHD eligibility starts with supported inputs and still rejects invalid reviewed games', () => {
  const restore = prepare();
  try {
    assert.ok(chdFormat());
    assert.match(renderUpload(), /id="upload-compress-chd"[^>]*disabled/u);
    for (const [platform, names, compatible] of [
      ['1', ['Game.cue', 'Game.bin'], true],
      ['1', ['Game.cue'], false],
      ['1', ['Game.chd'], false],
      ['1', ['Game.bin'], false],
      ['1', ['Game.m3u', 'Game.sbi'], false],
      ['1', ['Game.iso', 'Game.chd'], false],
      ['1', ['Game.iso'], true],
      ['1', ['Game.IMG'], true],
      ['2', ['Game.img'], true],
      ['3', ['Game.img'], true],
      ['2', ['Game.iso'], true],
      ['3', ['Game.iso'], true],
      ['3', ['Game.cso'], false],
      ['3', ['Game.cue', 'Game.bin'], false],
      ['4', ['Game.gdi', 'Game.bin'], false],
      ['1', ['Game.ccd', 'Game.img'], false],
      ['1', ['Game.img', 'Game.sub'], false],
      ['1', ['Game (Disc 1).cue', 'disc1.bin', 'Game (Disc 2).cue', 'disc2.bin', 'Game.m3u', 'Game (Disc 1).sbi'], true],
    ]) {
      setUploadPlatformId(platform); setUploadFiles(names.map((name) => ({ name, size: 10 })));
      assert.equal(canCompressChd(), compatible || names.join() === 'Game.cue', `Before review: ${names.join(', ')}`);
      if (canCompressChd()) {
        state.uploadCompressChd = true;
        assert.doesNotMatch(renderUpload(), /data-action="preview-upload-plan"[^>]*disabled/u);
        assert.match(renderUpload(), /data-action="finalize-upload-plan"[^>]*disabled/u);
      }
      state.uploadCompressChd = false;
      state.uploadPlan = plan(names, names.length === 1 && names[0].endsWith('.cue') ? [{}] : []);
      assert.equal(canCompressChd(), compatible, names.join(', '));
      assert.equal(state.uploadCompressChd, false);
    }
    setUploadPlatformId('1'); setUploadFiles([{ name: 'Game.cue' }, { name: 'Game.bin' }]);
    state.uploadPlan = plan(['Game.cue', 'Game.bin']); state.uploadCompressChd = true;
    setUploadPlatformId('2'); assert.equal(state.uploadCompressChd, false);
    state.uploadCompressChd = true; setUploadFiles([]); assert.equal(state.uploadCompressChd, false);
    state.uploadPlan = plan(['Game.iso']); setUploadFiles([{ name: 'Game.iso' }]);
    state.uploadPlan = plan(['Game.iso']); state.conversionStatus.enabled = false;
    assert.equal(canCompressChd(), false);
  } finally { restore(); }
});

test('review preserves an early compression choice and clears it for incompatible tracks', async () => {
  const restore = prepare();
  const beforeFetch = globalThis.fetch;
  const beforeAuth = state.auth;
  try {
    state.auth = { header: 'Basic test' };
    const form = { elements: { platform_id: { value: '1' } } };
    const errors = [];
    let reviewed;
    globalThis.fetch = async () => new Response(JSON.stringify(reviewed), { status: 200, headers: { 'Content-Type': 'application/json' } });
    const controller = new UploadController({ app: { querySelector: (selector) => selector === '#upload-form' ? form : null }, render() {}, setNotice() {}, setError(error) { errors.push(error); } });
    for (const compatible of [true, false]) {
      const contents = 'FILE "Game.bin" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n' + (compatible ? '' : ' FLAGS PRE\n');
      setUploadFiles([new File([contents], 'Game.cue'), new File([new Uint8Array(2352)], 'Game.bin')]);
      state.uploadCompressChd = true;
      reviewed = plan(['Game.cue', 'Game.bin']);
      await controller.onPreviewPlan();
      assert.equal(state.uploadCompressChd, compatible);
      assert.equal(canCompressChd(), compatible);
      assert.doesNotMatch(renderUpload(), /data-action="preview-upload-plan"[^>]*disabled/u);
    }
    assert.deepEqual(errors, []);
  } finally { globalThis.fetch = beforeFetch; state.auth = beforeAuth; restore(); }
});

test('separate games and disc sets use reviewed grouping without uploading generated playlists', async () => {
  const restore = prepare();
  try {
    const names = ['Other.iso', 'Game (Disc 2).img', 'Game (Disc 1).iso', 'Single.img'];
    setUploadFiles(names.map((name) => ({ name, size: 2048 })));
    const discSet = plan(names.slice(1, 3)).roms[0];
    discSet.files.push({ original_file_name: 'Game.m3u', metadata: { source: 'generated' } });
    state.uploadPlan = { errors: [], roms: [discSet, { ...plan([names[0]]).roms[0], title: 'Other' }, { ...plan([names[3]]).roms[0], title: 'Single' }] };
    assert.equal(canCompressChd(), true);
    state.uploadCompressChd = true;
    const launched = [];
    const controller = new UploadController({ app: { querySelector: () => null }, render() {}, setError(error) { throw error; }, launchConversion: async (input) => launched.push(input) });
    await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { platform_id: { value: '1' } } } });
    assert.deepEqual(launched.map((input) => input.files.map((entry) => entry.path)), [names.slice(1, 3), [names[0]], [names[3]]]);
    assert.deepEqual(launched.map((input) => input.title), ['Edited title', 'Other', 'Single']);
    assert.equal(state.uploadCompressChd, false);
    assert.deepEqual(state.uploadSelectedFiles, []);
    setUploadFiles([{ name: 'Unpaired.bin', size: 2352 }]);
    state.uploadPlan = plan(['Unpaired.bin']);
    assert.equal(canCompressChd(), false);
    setUploadFiles([{ name: 'Game.iso', size: 2048 }]);
    state.uploadPlan = { errors: [], roms: [plan(['Game.iso']).roms[0], plan(['Unpaired.bin']).roms[0]] };
    assert.equal(canCompressChd(), false, 'Every planned game must be compatible');
  } finally { restore(); }
});

test('review disables CHD for missing, truncated and unsupported CUE tracks', () => {
  const cue = 'FILE "Game.bin" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n';
  const inputs = (contents, size = 2352) => [{ file_name: 'Game.cue', manifest_contents: contents }, { file_name: 'Game.bin', file_size_bytes: size }];
  assert.equal(chdInputsCompatible(inputs(cue)), true);
  for (const text of [cue + ' INDEX 00 00:00:00\n', cue + ' POSTGAP 00:00:01\n', cue + ' FLAGS PRE\n', cue.replace('MODE1/2352', 'MODE1/2048'), cue.replace('BINARY', 'WAVE'), cue.replace('Game.bin', '../Game.bin')]) {
    assert.equal(chdInputsCompatible(inputs(text)), false, text);
  }
  assert.equal(chdInputsCompatible(inputs(cue, 2353)), false);
  assert.equal(chdInputsCompatible(inputs(cue).slice(0, 1)), false);
  for (const [name, bytes, expected] of [['Game.iso', 2048, true], ['Game.img', 2352, true], ['Game.img', 2048, true], ['Game.iso', 2049, false], ['Game.img', 2353, false], ['Game.img', 0, false]]) {
    assert.equal(chdInputsCompatible([{ file_name: name, file_size_bytes: bytes }]), expected, name);
  }
  const restore = prepare();
  try {
    setUploadFiles([{ name: 'Game.cue' }, { name: 'Game.bin' }]);
    state.uploadPlan = { ...plan(['Game.cue', 'Game.bin']), chd_inputs_compatible: false };
    assert.equal(canCompressChd(), false);
  } finally { restore(); }
});

test('CHD submission sends every disc, track, playlist and sidecar once without subsequent approval', async () => {
  const restore = prepare();
  try {
    const names = ['Game (Disc 1).cue', 'disc1.bin', 'Game (Disc 2).cue', 'disc2.bin', 'Game.m3u', 'Game (Disc 1).sbi'];
    setUploadFiles(names.map((name) => ({ name, size: 10 })));
    state.uploadPlan = plan(names); state.uploadCompressChd = true;
    const launched = [];
    const controller = new UploadController({ app: { querySelector: () => null }, render() {}, setError(error) { throw error; }, launchConversion: async (input) => launched.push(input) });
    assert.match(renderUpload(), /Compress and import CHD/u);
    await controller.onSubmit({ preventDefault() {}, currentTarget: { elements: { platform_id: { value: '1' } } } });
    assert.equal(launched.length, 1);
    assert.equal(launched[0].platform, 'psx');
    assert.equal(launched[0].title, 'Edited title');
    assert.deepEqual(launched[0].files.map((entry) => entry.path), names);
    assert.equal(state.uploadCompressChd, false); assert.deepEqual(state.uploadSelectedFiles, []);
    state.jobs = [{ id: 'chd_job', type: 'conversion-import', outputFormat: 'chd', title: 'Game', state: 'running', progress: { phase: 'compressing_and_verifying' } }];
    const jobs = renderJobs();
    assert.match(jobs, /Compressing and verifying CHD discs/u);
    assert.match(jobs, /CHD conversion import/u);
    assert.doesNotMatch(jobs, /Confirm|awaiting_review|Wii U|WUA/u);
    state.jobs[0].state = 'succeeded';
    state.jobs[0].result = { output_format: 'chd', file_name: 'Game.m3u', title: 'Game', input_bytes: 10, output_bytes: 12 };
    assert.match(renderJobs(), /CHD disc set/u);
    assert.match(renderJobs(), /larger than the inputs/u);
  } finally { restore(); }
});
