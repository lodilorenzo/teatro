// Requires an existing Playwright installation and Chromium. No project dependency is added.
// TEATRO_PLAYWRIGHT_MODULE=/path/to/playwright/index.mjs node scripts/test-browser-downloads.mjs
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdtemp, open, rm, stat } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const { chromium } = await import(process.env.TEATRO_PLAYWRIGHT_MODULE || 'playwright');
const binary = resolve(process.env.TEATRO_TEST_BINARY || 'target/debug/teatro');
const root = await mkdtemp(join(tmpdir(), 'teatro-download-check-'));
const listener = createServer();
await new Promise((resolve) => listener.listen(0, '127.0.0.1', resolve));
const port = listener.address().port;
await new Promise((resolve) => listener.close(resolve));
const origin = `http://127.0.0.1:${port}`;
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('TEATRO_')));
Object.assign(env, {
  TEATRO_BIND_ADDR: `127.0.0.1:${port}`, TEATRO_DATA_DIR: join(root, 'data'),
  TEATRO_DATABASE_URL: `sqlite://${join(root, 'test.sqlite3')}`,
  TEATRO_DEFAULT_LIBRARY_ROOT: join(root, 'roms'), TEATRO_ASSET_ROOT: join(root, 'assets'),
});
const server = spawn(binary, ['serve'], { env, cwd: root, stdio: 'ignore' });
const exited = new Promise((resolve, reject) => {
  server.once('exit', resolve);
  server.once('error', reject);
});
let browser;
async function digest(path) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest('hex');
}

try {
  for (let attempt = 0; ; attempt++) {
    if (await fetch(`${origin}/healthz`).then((r) => r.ok).catch(() => false)) break;
    assert.ok(attempt < 100 && server.exitCode === null, 'isolated server did not start');
    await sleep(100);
  }
  const setup = await fetch(`${origin}/api/setup`, {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ username: 'admin', password: 'synthetic-password' }),
  });
  assert.equal(setup.status, 201);
  const games = [];
  for (const size of [64 * 1024 * 1024 + 1, 64 * 1024 * 1024 - 1, 64 * 1024 * 1024]) {
    const form = new FormData();
    form.set('platform_slug', 'genesis');
    form.append('file', new Blob(['synthetic']), `Boundary-${size}.bin`);
    const upload = await fetch(`${origin}/api/admin/upload-batches`, {
      method: 'POST', headers: { Authorization: `Basic ${Buffer.from('admin:synthetic-password').toString('base64')}` },
      body: form,
    });
    assert.equal(upload.status, 201);
    const game = (await upload.json()).roms[0];
    const file = game.files[0];
    const relative = execFileSync('python3', ['-c', `
import sqlite3, sys
with sqlite3.connect(sys.argv[1]) as db:
    path, = db.execute('SELECT relative_path FROM rom_files WHERE id = ?', (sys.argv[2],)).fetchone()
    db.execute('UPDATE rom_files SET file_size_bytes = ? WHERE id = ?', (sys.argv[3], sys.argv[2]))
    print(path)
`, join(root, 'test.sqlite3'), String(file.id), String(size)], { encoding: 'utf8' }).trim();
    const path = join(root, 'roms', relative);
    const handle = await open(path, 'w');
    await handle.truncate(size);
    await handle.write('synthetic start', 0);
    await handle.write('synthetic end', size - 13);
    await handle.close();
    games.push({ id: game.id, file, size, hash: await digest(path) });
  }

  browser = await chromium.launch({ headless: true });
  const failures = [];
  for (const section of ['public', 'admin']) {
    const context = await browser.newContext({ acceptDownloads: true });
    const page = await context.newPage();
    page.setDefaultTimeout(10_000);
    const unauthorized = [];
    page.on('response', (response) => {
      if (response.status() === 401) unauthorized.push(new URL(response.url()).pathname);
    });
    await page.goto(`${origin}${section === 'admin' ? '/admin' : '/'}`);
    await page.locator('#login-form [name=username]').fill('admin');
    await page.locator('#login-form [name=password]').fill('synthetic-password');
    await page.locator('#login-form button[type=submit]').click();
    await page.locator('[data-action=logout]').waitFor();
    if (section === 'admin') await page.getByRole('button', { name: 'Library', exact: true }).click();
    for (const game of games) {
      try {
        if (section === 'public') await page.goto(`${origin}/#/game/${game.id}`);
        else await page.locator(`button[data-rom-id="${game.id}"]`).first().click();
        const selector = section === 'public'
          ? `[data-download-file="${game.file.id}"]`
          : `[data-download-rom="${game.id}"][data-file-id="${game.file.id}"]`;
        const [download] = await Promise.all([
          page.waitForEvent('download', { timeout: 10_000 }),
          page.locator(selector).first().click(),
        ]);
        const destination = join(root, `${section}-${game.size}.bin`);
        await download.saveAs(destination);
        assert.equal(download.url(), `${origin}/api/downloads/file`);
        assert.equal(download.suggestedFilename(), game.file.file_name);
        assert.equal((await stat(destination)).size, game.size);
        assert.equal(await digest(destination), game.hash);
        await rm(destination);
        console.log(`${section}: ${game.size} bytes, filename and SHA-256 match; native file ticket download`);
      } catch (error) {
        failures.push(`${section} ${game.size}: ${error.message}`);
      } finally {
        if (section === 'admin') await page.locator('button[data-action=close-rom-detail]').click();
      }
    }
    if (unauthorized.length) failures.push(`${section}: unauthenticated requests ${unauthorized.join(', ')}`);
    await context.close();
  }
  console.log(`Chromium ${browser.version()}, ${process.platform}/${process.arch}, fresh contexts without Basic credentials`);
  assert.deepEqual(failures, []);
} finally {
  await browser?.close();
  server.kill('SIGTERM');
  await exited;
  await rm(root, { recursive: true, force: true });
}
