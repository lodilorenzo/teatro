import {
  attr, encodeResourcePath, formatBytes, html, metadataList, metadataText,
} from '../public/shared.js';

export {
  attr, encodeResourcePath, formatBytes, html, metadataList, metadataText,
};

export function formatSpeed(bytesPerSecond) {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatRole(role) {
  return String(role || 'file').replaceAll('_', ' ');
}

export function slugifyTitle(title) {
  return String(title || '')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '') || 'rom';
}

function manifestExtension(fileName) {
  return String(fileName || '').split('.').pop()?.toLowerCase() || '';
}

export function isTextManifestFile(fileName) {
  return ['m3u', 'cue', 'gdi'].includes(manifestExtension(fileName));
}

export function csv(values) {
  return Array.isArray(values) ? values.join(', ') : '';
}

export function parseCsv(value) {
  return value
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean);
}

export function coverPath(rom, size = 'small') {
  if (!rom) return '';
  return size === 'large'
    ? rom.path_cover_large || rom.path_cover_small || ''
    : rom.path_cover_small || rom.path_cover_large || '';
}
