const UTF8_ENCODER = new TextEncoder();
const MAX_SETUP_FILE_NAME_BYTES = 255;
const MAX_SETUP_FILE_NAME_CODE_UNITS = 512;
const MAX_SELECTION_FILES = 256;
const MAX_PARENT_METADATA_TOKENS = 8;
const MAX_DISPLAY_TOKENS = 64;
const MAX_TITLE_BYTES = 512;
const CONTROL_CHARACTERS = /[\u0000-\u001f\u007f-\u009f]/u;
const ARCHITECTURE_METADATA = /^(?:32bit|64bit|x86|x64)$/iu;
const GOG_BUILD_METADATA = /^\d{4,12}$/u;
const DOTTED_VERSION = /_[vV]?\d{1,6}(?:\.\d{1,6}){1,5}(?:[A-Za-z][A-Za-z0-9]{0,7})?$/u;
const ISO_RELEASE = /_((?:19|20)\d{2})-(\d{2})-(\d{2})(?:_([0-9A-Fa-f]{6,16}))?$/u;
const EXPLICIT_RELEASE = /_(?:build|version)_([A-Za-z0-9][A-Za-z0-9._-]{0,63})$/iu;
const PARENTHESIZED_SUFFIX = /_\(([^()]*)\)$/u;
const CASED_LETTER = /\p{L}/u;
const LETTER_OR_NUMBER = /[\p{L}\p{N}]/u;
const EDGE_PUNCTUATION = /^[^\p{L}\p{N}]+|[^\p{L}\p{N}]+$/gu;

const LOWERCASE_TITLE_WORDS = new Set([
  'a', 'an', 'and', 'as', 'at', 'but', 'by', 'for', 'from', 'in', 'nor', 'of', 'on', 'or',
  'the', 'to', 'via', 'vs', 'with',
]);
const ROMAN_NUMERALS = new Set([
  'i', 'ii', 'iii', 'iv', 'v', 'vi', 'vii', 'viii', 'ix', 'x',
  'xi', 'xii', 'xiii', 'xiv', 'xv', 'xvi', 'xvii', 'xviii', 'xix', 'xx',
]);

function utf8Length(value) {
  return UTF8_ENCODER.encode(value).byteLength;
}

function safeSetupFileName(fileName) {
  if (typeof fileName !== 'string'
    || !fileName
    || fileName.length > MAX_SETUP_FILE_NAME_CODE_UNITS
    || fileName.trim() !== fileName
    || fileName.includes('/')
    || fileName.includes('\\')
    || CONTROL_CHARACTERS.test(fileName)) return false;
  return utf8Length(fileName) <= MAX_SETUP_FILE_NAME_BYTES;
}

function isLeapYear(year) {
  return year % 4 === 0 && (year % 100 !== 0 || year % 400 === 0);
}

function validIsoDate(yearText, monthText, dayText) {
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const days = [31, isLeapYear(year) ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return month >= 1 && month <= 12 && day >= 1 && day <= days[month - 1];
}

function stripParenthesizedMetadata(stem) {
  let current = stem;
  let stripped = 0;
  while (stripped < MAX_PARENT_METADATA_TOKENS) {
    const match = current.match(PARENTHESIZED_SUFFIX);
    if (!match || (!ARCHITECTURE_METADATA.test(match[1]) && !GOG_BUILD_METADATA.test(match[1]))) {
      break;
    }
    current = current.slice(0, match.index);
    stripped += 1;
  }

  const overflow = current.match(PARENTHESIZED_SUFFIX);
  if (stripped === MAX_PARENT_METADATA_TOKENS
    && overflow
    && (ARCHITECTURE_METADATA.test(overflow[1]) || GOG_BUILD_METADATA.test(overflow[1]))) {
    return null;
  }
  return { stem: current, stripped };
}

function stripReleaseSuffix(stem) {
  const explicit = stem.match(EXPLICIT_RELEASE);
  if (explicit && /\d/u.test(explicit[1])) {
    return stem.slice(0, explicit.index);
  }

  const release = stem.match(ISO_RELEASE);
  if (release && validIsoDate(release[1], release[2], release[3])) {
    return stem.slice(0, release.index);
  }

  const dotted = stem.match(DOTTED_VERSION);
  if (dotted) return stem.slice(0, dotted.index);
  return null;
}

function casedLetters(value) {
  return [...value].filter((character) => CASED_LETTER.test(character)
    && character.toLocaleLowerCase('en-US') !== character.toLocaleUpperCase('en-US'));
}

function isAllLowercaseWord(value) {
  const letters = casedLetters(value);
  return letters.length > 0
    && letters.every((character) => character === character.toLocaleLowerCase('en-US'));
}

function displayTokenCore(value) {
  return value.replace(EDGE_PUNCTUATION, '').toLocaleLowerCase('en-US');
}

function formatDisplayPart(part, index, total) {
  if (!isAllLowercaseWord(part)) return part;
  const core = displayTokenCore(part);
  if (ROMAN_NUMERALS.has(core)) return part.toLocaleUpperCase('en-US');
  if (index > 0 && index < total - 1 && LOWERCASE_TITLE_WORDS.has(core)) return part;
  return part.replace(CASED_LETTER, (character) => character.toLocaleUpperCase('en-US'));
}

function displayTitleFromSlug(slug) {
  let normalized;
  try {
    normalized = slug.normalize('NFC');
  } catch (_) {
    return null;
  }
  normalized = normalized
    .replace(/_+/gu, ' ')
    .replace(/\s+/gu, ' ')
    .replace(/-{2,}/gu, '-')
    .replace(/^[\s_-]+|[\s_-]+$/gu, '');
  if (!normalized || CONTROL_CHARACTERS.test(normalized)) return null;

  const words = normalized.split(' ');
  const displayParts = words.flatMap((word) => word.split('-'))
    .filter((part) => LETTER_OR_NUMBER.test(part));
  if (!displayParts.length || displayParts.length > MAX_DISPLAY_TOKENS) return null;

  let displayIndex = 0;
  const title = words.map((word) => word.split('-').map((part) => {
    if (!LETTER_OR_NUMBER.test(part)) return part;
    const formatted = formatDisplayPart(part, displayIndex, displayParts.length);
    displayIndex += 1;
    return formatted;
  }).join('-')).join(' ').trim();

  if (!title || CONTROL_CHARACTERS.test(title) || utf8Length(title) > MAX_TITLE_BYTES) return null;
  return title;
}

export function suggestGogTitleFromFileName(fileName) {
  if (!safeSetupFileName(fileName)) return null;
  const lowerName = fileName.toLocaleLowerCase('en-US');
  if (!lowerName.endsWith('.exe')) return null;

  const stemWithPrefix = fileName.slice(0, -4);
  if (!stemWithPrefix.toLocaleLowerCase('en-US').startsWith('setup_')) return null;
  let stem = stemWithPrefix.slice('setup_'.length);
  if (!stem) return null;

  const metadata = stripParenthesizedMetadata(stem);
  if (!metadata) return null;
  stem = metadata.stem;
  let recognizedSuffixes = metadata.stripped;

  const releaseStem = stripReleaseSuffix(stem);
  if (releaseStem !== null) {
    stem = releaseStem;
    recognizedSuffixes += 1;
  }

  const title = displayTitleFromSlug(stem);
  if (!title) return null;
  return {
    title,
    confidence: recognizedSuffixes > 0 ? 'high' : 'medium',
    source: 'gog_setup_filename',
  };
}

export function suggestGogTitleFromSelection(files) {
  const selected = [];
  try {
    for (const file of files || []) {
      if (selected.length >= MAX_SELECTION_FILES) return null;
      selected.push(file);
    }
  } catch (_) {
    return null;
  }
  if (!selected.length) return null;

  const names = new Set();
  let executableName = null;
  for (const file of selected) {
    const name = typeof file?.name === 'string' ? file.name : '';
    if (!safeSetupFileName(name)) return null;
    const lowerName = name.toLocaleLowerCase('en-US');
    if (names.has(lowerName)) return null;
    names.add(lowerName);
    if (lowerName.endsWith('.exe')) {
      if (executableName !== null) return null;
      executableName = name;
    } else if (!lowerName.endsWith('.bin')) {
      return null;
    }
  }

  return executableName === null ? null : suggestGogTitleFromFileName(executableName);
}
