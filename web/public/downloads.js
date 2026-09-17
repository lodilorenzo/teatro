import { apiRequest } from './api.js';

export async function downloadFile({ romId, file, authorization, notify = () => {} }) {
  notify(`Preparing ${file.file_name}…`, 'progress');
  const response = await apiRequest(
    `/api/roms/${encodeURIComponent(romId)}/files/${encodeURIComponent(file.id)}/download-ticket`,
    authorization,
    { method: 'POST' },
  );
  submitDownloadTicket(response.ticket, 'file');
  notify(`${file.file_name} download started in your browser.`, 'success');
  return { mode: 'browser' };
}

export async function downloadArchive({ romId, authorization, notify = () => {} }) {
  notify('Preparing package ZIP on Teatro…', 'progress');
  const response = await apiRequest(
    `/api/roms/${encodeURIComponent(romId)}/archive-ticket`,
    authorization,
    { method: 'POST' },
  );
  submitDownloadTicket(response.ticket);
  notify('Package download started in your browser.', 'success');
  return { mode: 'browser' };
}

export function submitDownloadTicket(ticket, kind = 'archive') {
  if (!['archive', 'file'].includes(kind)
    || !/^teatro_dl_[A-Za-z0-9_-]{43}$/.test(String(ticket || ''))) {
    throw new Error('Teatro returned an invalid download ticket.');
  }
  const form = document.createElement('form');
  form.method = 'post';
  form.action = kind === 'file' ? '/api/downloads/file' : '/api/downloads/archive';
  form.hidden = true;
  const input = document.createElement('input');
  input.type = 'hidden';
  input.name = 'ticket';
  input.value = ticket;
  form.append(input);
  document.body.append(form);
  form.submit();
  form.remove();
}
