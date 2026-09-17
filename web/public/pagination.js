import { pageCount } from './catalog.js';
import { icon } from './icons.js';
import { attr } from './shared.js';

export function renderPagination(page, hrefForPage = null) {
  const limit = Math.max(1, Number(page.limit) || 1);
  const current = Math.max(1, Math.floor(Number(page.offset || 0) / limit) + 1);
  const pages = pageCount(page.total, limit);
  if (pages <= 1) return '';

  function control(number, label, className, disabled = false) {
    const numbered = className === 'pagination-number';
    const accessibleLabel = numbered ? `Page ${number}` : `${number < current ? 'Previous' : 'Next'} page`;
    const attributes = `class="pagination-control ${className}" aria-label="${accessibleLabel}"${numbered && number === current ? ' aria-current="page"' : ''}`;
    if (disabled) return `<button type="button" ${attributes} disabled>${label}</button>`;
    return hrefForPage
      ? `<a ${attributes} href="${attr(hrefForPage(number))}">${label}</a>`
      : `<button type="button" ${attributes} data-page-offset="${(number - 1) * limit}">${label}</button>`;
  }

  const numbers = paginationWindow(current, pages).map((value) => value === '…'
    ? '<span class="pagination-gap" aria-hidden="true">…</span>'
    : control(value, value, 'pagination-number')).join('');
  return `<nav class="pagination" aria-label="Game pages">
    ${control(current - 1, `${icon('back')} Previous`, 'pagination-step', current <= 1)}
    <div class="pagination-pages">${numbers}</div>
    ${control(current + 1, `Next ${icon('arrow')}`, 'pagination-step', current >= pages)}
  </nav>`;
}

function paginationWindow(current, pages) {
  if (pages <= 7) return Array.from({ length: pages }, (_, index) => index + 1);
  const values = new Set([1, pages, current - 1, current, current + 1].filter((page) => page >= 1 && page <= pages));
  const sorted = [...values].sort((left, right) => left - right);
  const result = [];
  for (const value of sorted) {
    if (result.length && value - result[result.length - 1] > 1) result.push('…');
    result.push(value);
  }
  return result;
}
