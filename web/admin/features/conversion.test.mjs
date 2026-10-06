import test from 'node:test';
import assert from 'node:assert/strict';
import { ConversionController, readDroppedFiles } from './conversion.js';
import { activeJobCancellationUrl } from './jobs.js';
import { UploadController } from './upload.js';
import { decodeStoredServerJobs } from './server-jobs.js';
import { addJob, findJob, state } from '../state.js';
import { renderConversionImport, renderConversionProgress } from '../views/conversion.js';

test('folder preparation is opt-in, escaped and Jobs never requests approval', () => {
  const keys = ['conversionStatus', 'platforms', 'uploadPlatformId', 'conversionFiles', 'conversionTitle'];
  const before = Object.fromEntries(keys.map((key) => [key,state[key]]));
  try {
    state.platforms = [{ id: 1, slug: 'wiiu', display_name: 'Nintendo Wii U' }];
    state.uploadPlatformId = '1';
    state.conversionFiles = [{file:new Blob(['data']),path:'Game/content/data'}];
    state.conversionTitle = '<game>';
    state.conversionStatus = null;
    assert.equal(renderConversionImport(), '');
    state.conversionStatus = { enabled:true };
    assert.match(renderConversionImport(), /Compress and import WUA/);
    assert.match(renderConversionImport(), /&lt;game&gt;/);
    const job = {id:'local',state:'running',progress:{phase:'compressing'},proposal:{revision:2,output_bytes:200}};
    assert.match(renderConversionProgress(job), /Compressing WUA/);
    assert.doesNotMatch(renderConversionProgress(job), /data-conversion-confirm|Publish larger|confirmation/);
  } finally { Object.assign(state, before); }
});

test('conversion progress distinguishes measured transfer/compression from processing', () => {
  const job = { id:'progress', state:'running', progress:{phase:'uploading', uploadProgress:{current:512, total:1024, percent:50, speed:256}} };
  let markup = renderConversionProgress(job);
  assert.match(markup, /Uploading folders/);
  assert.match(markup, /value="50"/);
  assert.match(markup, /512 B \/ 1\.0 KiB/);
  assert.match(markup, /256 B\/s/);
  assert.match(markup, /aria-labelledby="conversion-progress-status"/);
  job.progress = { phase:'compressing', jobProgress:{current:256, total:1024, percent:25} };
  markup = renderConversionProgress(job);
  assert.match(markup, /Compressing WUA/);
  assert.match(markup, /value="25"/);
  assert.match(markup, /input processed/);
  for (const phase of ['inspecting', 'queued_conversion', 'finalizing_wua', 'verifying', 'publishing', 'matching_igdb_metadata']) {
    job.progress = { phase };
    markup = renderConversionProgress(job);
    assert.match(markup, /<progress max="100"  aria-labelledby/);
    assert.doesNotMatch(markup, /<progress[^>]*value=/);
    assert.match(markup, /aria-busy="true"/);
    assert.match(markup, /Progress is not measurable/);
  }
  for (const state of ['succeeded', 'cancelled', 'failed', 'unavailable']) {
    job.state = state; job.progress = {phase:'compressing', jobProgress:{current:100, total:100, percent:100}};
    assert.doesNotMatch(renderConversionProgress(job), /<progress/);
    assert.match(renderConversionProgress(job), /aria-busy="false"/);
  }
});

class FakeConversionXhr {
  constructor() { this.listeners = new Map(); this.upload = { addEventListener:(name, handler) => this.listeners.set(`upload:${name}`, handler) }; FakeConversionXhr.last = this; }
  open(method, url) { this.method = method; this.url = url; }
  setRequestHeader(name, value) { (this.headers ||= {})[name] = value; }
  addEventListener(name, handler) { this.listeners.set(name, handler); }
  send(body) { this.body = body; }
  emit(name, event = {}) { this.listeners.get(name)?.(event); }
  abort() { this.aborted = true; this.emit('abort'); this.emit('loadend'); }
}

test('Wii U uploads report bytes until receipt, then switch to server compression and processing', async () => {
  const keys = ['auth','jobs','screen','platforms','uploadPlatformId','conversionFiles','conversionTitle','conversionTitleOrigin','conversionInspection','conversionInspecting'];
  const previous = Object.fromEntries(keys.map((key) => [key,state[key]]));
  const oldXhr = globalThis.XMLHttpRequest;
  globalThis.XMLHttpRequest = FakeConversionXhr;
  Object.assign(state, {platforms:[{id:1,slug:'wiiu'}],uploadPlatformId:'1',auth:{header:'Bearer test'}, jobs:[], conversionTitle:'Base game', conversionInspecting:false,
    conversionFiles:[{file:new File(['x'.repeat(100)], 'app.xml'), path:'Game/code/app.xml'}], conversionInspection:{titles:[],errors:[]} });
  const changed = [];
  const controller = new ConversionController({app:{querySelector:()=>null}, render:()=>{}, jobChanged:(id)=>changed.push(id)});
  controller.poller.watch = () => {};
  try {
    const pending = controller.launch();
    const jobId = state.jobs[0].id;
    const xhr = FakeConversionXhr.last;
    assert.equal(xhr.url, '/api/admin/conversion-imports');
    assert.equal(xhr.headers.Authorization, 'Bearer test');
    assert.equal(xhr.body.getAll('files')[0].name, 'Game/code/app.xml');
    xhr.emit('upload:progress', {lengthComputable:true, loaded:150, total:300});
    assert.equal(findJob(jobId).progress.uploadProgress.percent, 50);
    assert.ok(changed.includes(jobId));
    xhr.emit('upload:load');
    assert.deepEqual(findJob(jobId).progress, {phase:'inspecting'});
    xhr.emit('upload:progress', {loaded:300});
    assert.equal(findJob(jobId).progress.phase, 'inspecting', 'Late upload callbacks must not undo server processing');
    xhr.status = 202; xhr.responseText = '{"id":"conv_test","status_url":"/api/admin/conversion-imports/conv_test"}';
    xhr.emit('load'); xhr.emit('loadend');
    await pending;
    assert.equal(findJob(jobId).serverJobId, 'conv_test');
    await controller.onSnapshot(jobId, {id:'conv_test',state:'running',phase:'compressing',progress:{current:25,total:100,percent:25}}, 'conv_test');
    assert.match(renderConversionProgress(findJob(jobId)), /value="25"/);
    await controller.onSnapshot(jobId, {id:'conv_test',state:'running',phase:'finalizing_wua',progress:null}, 'conv_test');
    assert.match(renderConversionProgress(findJob(jobId)), /Finalizing WUA/);
    assert.doesNotMatch(renderConversionProgress(findJob(jobId)), /<progress[^>]*value=/);
  } finally { controller.stopAll(); Object.assign(state,previous); globalThis.XMLHttpRequest = oldXhr; }
});

test('Wii U upload cancellation aborts the request and stops processing indicators', async () => {
  const keys = ['auth','jobs','screen','platforms','uploadPlatformId','conversionFiles','conversionTitle','conversionTitleOrigin','conversionInspection','conversionInspecting'];
  const previous = Object.fromEntries(keys.map((key) => [key,state[key]]));
  const oldXhr = globalThis.XMLHttpRequest;
  globalThis.XMLHttpRequest = FakeConversionXhr;
  Object.assign(state, {platforms:[{id:1,slug:'wiiu'}],uploadPlatformId:'1',auth:{header:'Bearer test'}, jobs:[], conversionTitle:'Game', conversionInspecting:false,
    conversionFiles:[{file:new File(['data'],'file'), path:'Game/content/file'}], conversionInspection:{titles:[],errors:[]} });
  const controller = new ConversionController({app:{querySelector:()=>null}, render:()=>{}});
  try {
    const pending = controller.launch();
    const jobId = state.jobs[0].id;
    await controller.cancelJob(jobId);
    await pending;
    assert.equal(FakeConversionXhr.last.aborted,true);
    assert.equal(findJob(jobId).state,'cancelled');
    assert.doesNotMatch(renderConversionProgress(findJob(jobId)), /<progress/);
  } finally { controller.stopAll(); Object.assign(state,previous); globalThis.XMLHttpRequest = oldXhr; }
});

test('conversion batches queue beyond server capacity and advance after success, failure or cancellation', async () => {
  const before = { auth: state.auth, jobs: state.jobs, screen: state.screen, platforms: state.platforms, xhr: globalThis.XMLHttpRequest };
  Object.assign(state, { auth: { header: 'Bearer test' }, jobs: [], platforms: [{slug:'psx'}] });
  globalThis.XMLHttpRequest = FakeConversionXhr;
  const controller = new ConversionController({ app: { querySelector: () => null }, render() {} });
  controller.poller.watch = () => {};
  try {
    const uploads = Array.from({length:6}, (_, at) => controller.launchFiles({title:`Game ${at}`, platform:'psx', files:[{file:new File(['disc'], `Game ${at}.iso`), path:`Game ${at}.iso`}]}));
    const jobs = [...state.jobs].reverse();
    const first = jobs.find((job) => job.state === 'running');
    const queued = jobs.filter((job) => job.state === 'queued');
    assert.equal(queued.length, 5);
    assert.match(renderConversionProgress(queued[0]), /Keep this page open/);
    const firstXhr = FakeConversionXhr.last;
    await controller.cancelJob(queued[0].id);
    assert.equal(FakeConversionXhr.last, firstXhr, 'Cancelling a queued job must not start an overlapping upload');
    firstXhr.status = 202; firstXhr.responseText = '{"id":"conv_first","status_url":"/api/admin/conversion-imports/conv_first"}';
    firstXhr.emit('load'); firstXhr.emit('loadend');
    await Promise.all(uploads);
    assert.equal(FakeConversionXhr.last, firstXhr, 'The next upload waits for conversion, not just receipt');
    await controller.onSnapshot(first.id, {id:'conv_first', state:'succeeded', phase:'complete'}, 'conv_first');
    assert.notEqual(FakeConversionXhr.last, firstXhr);
    const rejected = FakeConversionXhr.last;
    rejected.status = 400; rejected.responseText = '{"error":{"message":"Bad image"}}';
    rejected.emit('load'); rejected.emit('loadend');
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.notEqual(FakeConversionXhr.last, rejected, 'An upload failure must release the queue');
    for (let at = 0; at < 3; at++) {
      const current = state.jobs.find((job) => job.state === 'running');
      assert.ok(current);
      const xhr = FakeConversionXhr.last;
      const serverId = `conv_remaining${at}`;
      xhr.status = 202; xhr.responseText = JSON.stringify({id:serverId,status_url:`/api/admin/conversion-imports/${serverId}`});
      xhr.emit('load'); xhr.emit('loadend');
      await new Promise((resolve) => setTimeout(resolve, 0));
      await controller.onSnapshot(current.id, {id:serverId, state:at === 0 ? 'failed' : 'succeeded', phase:'complete'}, serverId);
    }
    assert.equal(state.jobs.filter((job) => ['running','queued'].includes(job.state)).length, 0);
    assert.equal(controller.pendingUploads.size, 0);
    assert.equal(controller.activeConversionId, null);
  } finally {
    controller.stopAll(); state.auth = before.auth; state.jobs = before.jobs; state.screen = before.screen; state.platforms = before.platforms; globalThis.XMLHttpRequest = before.xhr;
  }
});

test('folder drops retain nested paths and read every directory batch', async () => {
  const file = (name) => ({ name, isFile: true, file: (resolve) => resolve(new Blob([name])) });
  const directory = (name, ...batches) => ({ name, isDirectory: true, createReader: () => {
    let index = 0;
    return { readEntries: (resolve) => resolve(batches[index++] || []) };
  } });
  const root = directory('Family',
    [directory('Game', [directory('code', [file('app.xml')])])],
    [directory('Update', [directory('content', [file('patch.bin')])])]);
  const transfer = { items: [{ webkitGetAsEntry: () => root }] };
  const entries = await readDroppedFiles(transfer);
  assert.deepEqual(entries.map((entry) => entry.path), ['Family/Game/code/app.xml', 'Family/Update/content/patch.bin']);
  assert.equal(await entries[0].file.text(), 'app.xml');
  await assert.rejects(readDroppedFiles(transfer, 1), /Too many files/);
  const bare = new File(['data'], 'bare.wua');
  assert.deepEqual(await readDroppedFiles({ files: [bare] }), [{file:bare,path:'bare.wua'}]);
  await assert.rejects(readDroppedFiles({ files: [bare, bare] }, 1), /Too many files/);
  await assert.rejects(readDroppedFiles({ items: [{ webkitGetAsEntry: () => directory('Empty') }] }), /contain no files/);
  await assert.rejects(readDroppedFiles({ items: [{ webkitGetAsEntry: () => ({ name: 'Unreadable', isDirectory: true, createReader: () => ({ readEntries: (_, reject) => reject(new Error('Denied')) }) }) }] }), /Denied/);
});

test('shared Wii U drop routes normal files and refuses mixed imports without replacing selections', async () => {
  const keys = ['uploadSelectedFiles','uploadPlan','uploadCompressRvz','uploadPreviewLoading','conversionFiles','conversionInspection','conversionInspecting','conversionTitle','conversionTitleOrigin'];
  const before = Object.fromEntries(keys.map((key) => [key,state[key]]));
  const errors = [];
  const context = {app:{querySelector:()=>null,querySelectorAll:()=>[]},render:()=>{},setError:(error)=>errors.push(error.message)};
  const conversion = new ConversionController(context);
  const upload = new UploadController(context);
  const options = {onFiles:(files)=>upload.appendSelectedFiles(files)};
  const file = new File(['archive'],'Game.wua');
  const folder = {file:new Blob(['data']),path:'Game/content/data'};
  try {
    Object.assign(state,{uploadSelectedFiles:[],conversionFiles:[],conversionInspecting:false});
    await conversion.selectFolders(Promise.resolve([{file,path:file.name}]),options);
    assert.deepEqual(state.uploadSelectedFiles,[file]);
    await conversion.selectFolders(Promise.resolve([folder]),options);
    assert.match(errors.at(-1),/Clear the game files/);
    assert.deepEqual(state.uploadSelectedFiles,[file]);
    assert.deepEqual(state.conversionFiles,[]);
    upload.clearSelectedFiles();
    await conversion.selectFolders(Promise.resolve([folder,{file,path:file.name}]),options);
    assert.match(errors.at(-1),/separate jobs/);
    assert.deepEqual(state.uploadSelectedFiles,[]);
    state.conversionFiles = [folder];
    await conversion.selectFolders(Promise.resolve([{file,path:file.name}]),options);
    assert.match(errors.at(-1),/Clear the decrypted folders/);
    assert.deepEqual(state.conversionFiles,[folder]);
    assert.deepEqual(state.uploadSelectedFiles,[]);
    let finish;
    const pending = conversion.selectFolders(new Promise((resolve)=>{finish=resolve;}),options);
    conversion.clearSelection();
    finish([{file,path:file.name}]);
    await pending;
    assert.deepEqual(state.uploadSelectedFiles,[],'A cleared pending drop cannot restore normal files either');
  } finally {conversion.stopAll();Object.assign(state,before);}
});

test('clearing a pending folder read cannot restore a stale selection', async () => {
  const before = { conversionFiles: state.conversionFiles, conversionInspection: state.conversionInspection, conversionInspecting: state.conversionInspecting, conversionTitle: state.conversionTitle, conversionTitleOrigin: state.conversionTitleOrigin };
  state.conversionFiles = [];
  const handlers = new Map();
  const app = { querySelector: (selector) => selector === '[data-clear-conversion]' ? { addEventListener: (_, handler) => handlers.set(selector, handler) } : null, querySelectorAll: () => [] };
  const controller = new ConversionController({ app, render: () => {}, setError: (error) => { throw error; } });
  try {
    controller.bind();
    let finish;
    const pending = controller.selectFolders(new Promise((resolve) => { finish = resolve; }));
    assert.equal(state.conversionInspecting, true);
    handlers.get('[data-clear-conversion]')();
    finish([{ file: new Blob(['data']), path: 'Game/code/app.xml' }]);
    await pending;
    assert.deepEqual(state.conversionFiles, []);
    assert.equal(state.conversionInspecting, false);
    assert.equal(state.conversionInspection, null);
  } finally { controller.stopAll(); Object.assign(state, before); }
});

test('conversion storage and requests never trust a foreign saved status URL', async () => {
  const previous = {jobs:state.jobs,auth:state.auth,fetch:globalThis.fetch};
  state.jobs = []; state.auth = {header:'Bearer test-token'};
  const calls = [];
  globalThis.fetch = async (url, options) => { calls.push([url,options]); return new Response('{}',{status:202,headers:{'Content-Type':'application/json'}}); };
  const record = {type:'conversion-import',id:'conv_test',statusUrl:'/api/admin/conversion-imports/conv_test'};
  assert.equal(decodeStoredServerJobs(JSON.stringify([record])).length,1);
  assert.equal(decodeStoredServerJobs(JSON.stringify([{...record,statusUrl:'https://foreign.test/steal'}])).length,0);
  const job = addJob({type:'conversion-import',title:'Title',state:'running',serverJobId:'conv_test',statusUrl:'https://foreign.test/steal'});
  const controller = new ConversionController({app:{querySelector:()=>null},render:()=>{},setError:(error)=>{throw error;}});
  try {
    assert.equal(calls.length,0,'Preparing/polling alone never submits a conversion');
    assert.equal(activeJobCancellationUrl(job),record.statusUrl);
    await controller.cancelJob(job.id);
    assert.equal(calls[0][0],record.statusUrl);
    assert.equal(calls[0][1].method,'DELETE');
    assert.equal(findJob(job.id).state,'cancelled');
  } finally { controller.stopAll(); state.jobs=previous.jobs; state.auth=previous.auth; globalThis.fetch=previous.fetch; }
});
