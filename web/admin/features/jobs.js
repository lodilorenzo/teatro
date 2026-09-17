import { api } from '../api.js';
import {
  clearFinishedJobs, findJob, isJobActive, removeJob, selectRom, state,
} from '../state.js';
import { jobListPriority, renderJobCardBody } from '../views/jobs.js';

export function activeJobCancellationUrl(job) {
  if (!isJobActive(job)) return null;
  if (job.type === 'library-scan' && job.serverJobId) {
    return `/api/admin/library/scans/${encodeURIComponent(job.serverJobId)}`;
  }
  if (job.type === 'gog-import' && job.serverJobId) {
    return `/api/admin/gog-imports/${encodeURIComponent(job.serverJobId)}`;
  }
  if (job.type === 'romm-import' && job.serverJobId) {
    return `/api/admin/sources/romm/imports/${encodeURIComponent(job.serverJobId)}`;
  }
  if (job.type === 'upload' || job.type === 'gog-import') {
    const operation = job.type === 'upload' ? 'upload' : 'gog';
    return `/api/admin/background-transfers/${encodeURIComponent(job.id)}?operation=${operation}`;
  }
  return null;
}

export class JobsController {
  constructor(context) {
    this.context = Object.freeze({ ...context });
  }

  bind(root = this.context.app) {
    root.querySelector('[data-action="clear-finished-jobs"]')
      ?.addEventListener('click', this.clearFinished);
    root.querySelectorAll('[data-dismiss-job]').forEach((button) => {
      button.addEventListener('click', () => this.dismiss(button.dataset.dismissJob));
    });
    root.querySelectorAll('[data-cancel-job]').forEach((button) => {
      button.addEventListener('click', () => this.cancel(button.dataset.cancelJob));
    });
    root.querySelectorAll('[data-view-job-rom]').forEach((button) => {
      button.addEventListener('click', () => void this.viewResult(button.dataset.viewJobRom));
    });
  }

  clearFinished = () => {
    clearFinishedJobs();
    this.context.render();
  };

  cancel(jobId) {
    const job = findJob(jobId);
    if (!job || !isJobActive(job)) return;
    if (!globalThis.confirm(`Cancel “${job.title}”?`)) return;
    Promise.resolve(this.context.cancelJob?.(jobId)).catch((error) => {
      this.context.setError(error);
      this.context.render();
    });
  }

  async cancelAll() {
    const jobs = state.jobs.filter(isJobActive);
    const results = await Promise.allSettled(
      jobs.map(({ id }) => Promise.resolve(this.context.cancelJob?.(id))),
    );
    const failure = results.find((result, index) => result.status === 'rejected'
      && isJobActive(findJob(jobs[index].id)));
    if (failure) throw failure.reason;
    const remaining = jobs.filter(({ id }) => isJobActive(findJob(id)));
    if (remaining.length) throw new Error('One or more active jobs could not be cancelled.');
  }

  dismiss(jobId) {
    const job = findJob(jobId);
    if (!job || isJobActive(job)) return;
    removeJob(jobId);
    this.context.render();
  }

  async viewResult(jobId) {
    const job = findJob(jobId);
    const rom = job?.type === 'gog-import'
      ? job.result?.rom
      : job?.result?.roms?.at?.(-1);
    if (!rom?.id) return;
    try {
      const files = await api(`/api/admin/roms/${encodeURIComponent(rom.id)}/files`).catch(() => null);
      selectRom(rom, files);
      this.context.render();
    } catch (error) {
      this.context.setError(error);
      this.context.render();
    }
  }

  refreshJob(jobId) {
    if (state.screen !== 'jobs') return;
    const job = findJob(jobId);
    if (!job) {
      this.context.render();
      return;
    }
    const shell = [...this.context.app.querySelectorAll('[data-job-shell]')]
      .find((element) => element.dataset.jobShell === jobId);
    if (!shell || Number(shell.dataset.jobPriority) !== jobListPriority(job)) {
      this.context.render();
      return;
    }

    const detailsState = new Map(
      [...shell.querySelectorAll('details[data-job-details]')]
        .map((details) => [details.dataset.jobDetails, Boolean(details.open)]),
    );
    const oldTranscript = shell.querySelector('[data-job-transcript]');
    const oldScrollTop = Number(oldTranscript?.scrollTop || 0);
    const nearBottom = !oldTranscript
      || oldTranscript.scrollHeight - oldTranscript.scrollTop - oldTranscript.clientHeight <= 32;

    shell.innerHTML = renderJobCardBody(job);

    shell.querySelectorAll('details[data-job-details]').forEach((details) => {
      if (detailsState.has(details.dataset.jobDetails)) {
        details.open = detailsState.get(details.dataset.jobDetails);
      }
    });
    const newTranscript = shell.querySelector('[data-job-transcript]');
    if (newTranscript) {
      if (nearBottom) newTranscript.scrollTop = newTranscript.scrollHeight;
      else newTranscript.scrollTop = Math.min(oldScrollTop, newTranscript.scrollHeight);
    }
    this.bind(shell);
  }
}
