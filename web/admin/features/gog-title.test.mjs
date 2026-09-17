import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

import {
  suggestGogTitleFromFileName,
  suggestGogTitleFromSelection,
} from './gog-title.js';

const fixture = JSON.parse(readFileSync(
  new URL('../../../tests/fixtures/gog-title-filenames.json', import.meta.url),
  'utf8',
));
const file = (name) => ({ name, size: 1 });

for (const entry of fixture.cases) {
  test(`GOG title fixture: ${entry.id}`, () => {
    assert.deepEqual(suggestGogTitleFromFileName(entry.filename), entry.expected);
  });
}

test('GOG title selection is order-independent and reads names only', () => {
  const contentAccesses = [];
  const setup = {
    name: 'setup_the_witcher_adventure_game_1.2.5a_(12082).exe',
    get size() { contentAccesses.push('size'); return 1; },
    arrayBuffer() { contentAccesses.push('arrayBuffer'); throw new Error('must not read content'); },
    bytes() { contentAccesses.push('bytes'); throw new Error('must not read content'); },
    slice() { contentAccesses.push('slice'); throw new Error('must not read content'); },
    stream() { contentAccesses.push('stream'); throw new Error('must not read content'); },
    text() { contentAccesses.push('text'); throw new Error('must not read content'); },
  };
  const expected = {
    title: 'The Witcher Adventure Game',
    confidence: 'high',
    source: 'gog_setup_filename',
  };

  assert.deepEqual(suggestGogTitleFromSelection([
    file('setup_game-2.bin'), setup, file('setup_game-1.bin'),
  ]), expected);
  assert.deepEqual(suggestGogTitleFromSelection([
    setup, file('setup_game-1.bin'), file('setup_game-2.bin'),
  ]), expected);
  assert.deepEqual(contentAccesses, []);
});

test('GOG title selection rejects ambiguous and invalid bundles without throwing', () => {
  assert.equal(suggestGogTitleFromSelection([]), null);
  assert.equal(suggestGogTitleFromSelection([file('setup_game-1.bin')]), null);
  assert.equal(suggestGogTitleFromSelection([
    file('setup_game.exe'), file('setup_other.exe'),
  ]), null);
  assert.equal(suggestGogTitleFromSelection([
    file('setup_game.exe'), file('readme.txt'),
  ]), null);
  assert.equal(suggestGogTitleFromSelection([
    file('setup_game.exe'), file('SETUP_GAME.EXE'),
  ]), null);
  assert.equal(suggestGogTitleFromSelection([
    file('setup_game.exe'), file('setup_game-1.bin'), file('SETUP_GAME-1.BIN'),
  ]), null);
  assert.equal(suggestGogTitleFromSelection({ [Symbol.iterator]: () => { throw new Error('bad input'); } }), null);
});

test('GOG title inference enforces filename, metadata, token, and regex bounds', () => {
  const acceptedStem = 'a'.repeat(245);
  const acceptedName = `setup_${acceptedStem}.exe`;
  assert.equal(new TextEncoder().encode(acceptedName).byteLength, 255);
  assert.equal(suggestGogTitleFromFileName(acceptedName)?.title, `A${'a'.repeat(244)}`);

  const rejectedName = `setup_${'a'.repeat(246)}.exe`;
  assert.equal(new TextEncoder().encode(rejectedName).byteLength, 256);
  assert.equal(suggestGogTitleFromFileName(rejectedName), null);
  assert.equal(suggestGogTitleFromFileName(`setup_${'😀'.repeat(200)}.exe`), null);

  const eightMetadataTokens = `${Array.from({ length: 8 }, () => '_(x64)').join('')}`;
  assert.equal(
    suggestGogTitleFromFileName(`setup_bounded_game${eightMetadataTokens}.exe`)?.title,
    'Bounded Game',
  );
  assert.equal(
    suggestGogTitleFromFileName(`setup_bounded_game${eightMetadataTokens}_(x64).exe`),
    null,
  );

  const sixtyFourTokens = Array.from({ length: 64 }, () => 'a').join('_');
  assert.ok(suggestGogTitleFromFileName(`setup_${sixtyFourTokens}.exe`));
  const sixtyFiveTokens = Array.from({ length: 65 }, () => 'a').join('_');
  assert.equal(suggestGogTitleFromFileName(`setup_${sixtyFiveTokens}.exe`), null);

  const adversarial = `setup_${'_'.repeat(100)}${'-'.repeat(100)}${'('.repeat(20)}.exe`;
  assert.doesNotThrow(() => suggestGogTitleFromFileName(adversarial));

  const excessiveSelection = [file('setup_game.exe'), ...Array.from(
    { length: 256 }, (_, index) => file(`setup_game-${index}.bin`),
  )];
  assert.equal(suggestGogTitleFromSelection(excessiveSelection), null);
});

test('GOG title inference never strips bare numbers or unrecognized terminal syntax', () => {
  assert.deepEqual(suggestGogTitleFromFileName('setup_game_2.exe'), {
    title: 'Game 2', confidence: 'medium', source: 'gog_setup_filename',
  });
  assert.deepEqual(suggestGogTitleFromFileName('setup_game_v2.exe'), {
    title: 'Game V2', confidence: 'medium', source: 'gog_setup_filename',
  });
  assert.deepEqual(suggestGogTitleFromFileName('setup_game_1.2.3_(edition).exe'), {
    title: 'Game 1.2.3 (Edition)', confidence: 'medium', source: 'gog_setup_filename',
  });
  assert.deepEqual(suggestGogTitleFromFileName('setup_game_1.2.3.4.5.6.7.exe'), {
    title: 'Game 1.2.3.4.5.6.7', confidence: 'medium', source: 'gog_setup_filename',
  });
});
