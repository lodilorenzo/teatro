//! Shared process-local latest-snapshot job lifecycle used by managed-library scans
//! and RomM imports: bounded, in-memory, non-durable, admin-only.
//!
//! Snapshots never carry credentials; each feature picks its own result payload.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use thiserror::Error;

use super::bounded_text;

const RANDOM_BYTES: usize = 32;
const MAX_ID_BYTES: usize = 128;
const MAX_ERROR_BYTES: usize = 1024;
const TERMINAL_CAPACITY: usize = 64;
const TERMINAL_RETENTION: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct JobProgress {
    pub(crate) current: u64,
    pub(crate) total: u64,
    pub(crate) percent: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JobError {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl JobError {
    pub(crate) fn new(code: &str, message: &str) -> Self {
        Self {
            code: bounded_text(code, MAX_ERROR_BYTES).0,
            message: bounded_text(message, MAX_ERROR_BYTES).0,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct JobSnapshot<R> {
    pub(crate) id: String,
    pub(crate) state: &'static str,
    pub(crate) phase: &'static str,
    pub(crate) progress: Option<JobProgress>,
    pub(crate) result: Option<R>,
    pub(crate) error: Option<JobError>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub(crate) enum JobRegistryError {
    #[error("job was not found")]
    NotFound,
    #[error("job is not in the required state")]
    IllegalState,
    #[error("job phase cannot move backwards")]
    InvalidPhaseTransition,
    #[error("job progress cannot decrease")]
    ProgressRegressed,
}

pub(crate) struct JobRegistry<R> {
    id_prefix: &'static str,
    jobs: Mutex<HashMap<String, JobRecord<R>>>,
}

impl<R: Clone> JobRegistry<R> {
    pub(crate) fn new(id_prefix: &'static str) -> Self {
        Self {
            id_prefix,
            jobs: Mutex::new(HashMap::new()),
        }
    }

    fn valid_id(&self, value: &str) -> bool {
        let suffix = value.strip_prefix(self.id_prefix).unwrap_or_default();
        value.len() <= MAX_ID_BYTES
            && !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    }

    pub(crate) fn reserve(self: &Arc<Self>) -> (JobSnapshot<R>, JobReporter<R>) {
        let now = Utc::now();
        loop {
            let id = generate_id(self.id_prefix);
            let mut jobs = self.lock();
            prune(&mut jobs, now);
            if jobs.contains_key(&id) {
                continue;
            }
            let record = JobRecord::new(id.clone(), now);
            let snapshot = record.snapshot();
            jobs.insert(id.clone(), record);
            return (
                snapshot,
                JobReporter {
                    registry: Arc::clone(self),
                    id,
                },
            );
        }
    }

    pub(crate) fn snapshot(&self, id: &str) -> Result<JobSnapshot<R>, JobRegistryError> {
        if !self.valid_id(id) {
            return Err(JobRegistryError::NotFound);
        }
        let mut jobs = self.lock();
        prune(&mut jobs, Utc::now());
        jobs.get(id)
            .map(JobRecord::snapshot)
            .ok_or(JobRegistryError::NotFound)
    }

    fn start(&self, id: &str) -> Result<bool, JobRegistryError> {
        let mut jobs = self.lock();
        let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
        if terminal(job.state) {
            return Ok(false);
        }
        if job.state != "queued" {
            return Err(JobRegistryError::IllegalState);
        }
        job.state = "running";
        job.touch();
        Ok(true)
    }

    fn set_phase(&self, id: &str, phase: &'static str, order: u8) -> Result<(), JobRegistryError> {
        let mut jobs = self.lock();
        let job = running_job(&mut jobs, id)?;
        if terminal(job.state) {
            return Ok(());
        }
        if order < job.phase_order || (order == job.phase_order && phase != job.phase) {
            return Err(JobRegistryError::InvalidPhaseTransition);
        }
        if order > job.phase_order {
            job.phase = phase;
            job.phase_order = order;
            job.progress = None;
            job.touch();
        }
        Ok(())
    }

    fn set_progress(&self, id: &str, current: u64, total: u64) -> Result<(), JobRegistryError> {
        if current > total {
            return Err(JobRegistryError::ProgressRegressed);
        }
        let progress = JobProgress {
            current,
            total,
            percent: if total == 0 {
                100.0
            } else {
                (current as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
            },
        };
        let mut jobs = self.lock();
        let job = running_job(&mut jobs, id)?;
        if terminal(job.state) {
            return Ok(());
        }
        if job.progress.as_ref().is_some_and(|previous| {
            progress.total != previous.total || progress.current < previous.current
        }) {
            return Err(JobRegistryError::ProgressRegressed);
        }
        if job.progress.as_ref() != Some(&progress) {
            job.progress = Some(progress);
            job.touch();
        }
        Ok(())
    }

    fn succeed(&self, id: &str, result: R) -> Result<(), JobRegistryError> {
        let now = Utc::now();
        let mut jobs = self.lock();
        let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
        if terminal(job.state) {
            return Ok(());
        }
        if job.state != "running" {
            return Err(JobRegistryError::IllegalState);
        }
        job.state = "succeeded";
        job.abort_handle = None;
        job.phase = "complete";
        job.phase_order = COMPLETE_PHASE_ORDER;
        job.progress = None;
        job.result = Some(result);
        job.error = None;
        job.touch();
        job.terminal_at = Some(job.updated_at);
        prune(&mut jobs, now);
        Ok(())
    }

    fn fail(&self, id: &str, error: JobError) -> Result<(), JobRegistryError> {
        let now = Utc::now();
        let mut jobs = self.lock();
        let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
        if terminal(job.state) {
            return Ok(());
        }
        job.state = "failed";
        job.abort_handle = None;
        job.progress = None;
        job.result = None;
        job.error = Some(error);
        job.touch();
        job.terminal_at = Some(job.updated_at);
        prune(&mut jobs, now);
        Ok(())
    }

    pub(crate) fn cancel(&self, id: &str) -> Result<JobSnapshot<R>, JobRegistryError> {
        if !self.valid_id(id) {
            return Err(JobRegistryError::NotFound);
        }
        let now = Utc::now();
        let mut jobs = self.lock();
        let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
        if terminal(job.state) {
            return Ok(job.snapshot());
        }
        job.state = "cancelled";
        job.progress = None;
        job.result = None;
        job.error = None;
        job.touch();
        job.terminal_at = Some(job.updated_at);
        let abort_handle = job.abort_handle.take();
        let snapshot = job.snapshot();
        drop(jobs);
        if let Some(abort_handle) = abort_handle {
            abort_handle.abort();
        }
        let mut jobs = self.lock();
        prune(&mut jobs, now);
        Ok(snapshot)
    }

    fn attach_abort_handle(
        &self,
        id: &str,
        abort_handle: tokio::task::AbortHandle,
    ) -> Result<(), JobRegistryError> {
        let mut jobs = self.lock();
        let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
        if terminal(job.state) {
            drop(jobs);
            abort_handle.abort();
        } else {
            job.abort_handle = Some(abort_handle);
        }
        Ok(())
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, JobRecord<R>>> {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Clone)]
pub(crate) struct JobReporter<R> {
    registry: Arc<JobRegistry<R>>,
    id: String,
}

impl<R: Clone> JobReporter<R> {
    pub(crate) fn start(&self) -> Result<bool, JobRegistryError> {
        self.registry.start(&self.id)
    }

    pub(crate) fn set_phase(&self, phase: &'static str, order: u8) -> Result<(), JobRegistryError> {
        self.registry.set_phase(&self.id, phase, order)
    }

    pub(crate) fn set_progress(&self, current: u64, total: u64) -> Result<(), JobRegistryError> {
        self.registry.set_progress(&self.id, current, total)
    }

    pub(crate) fn succeed(&self, result: R) -> Result<(), JobRegistryError> {
        self.registry.succeed(&self.id, result)
    }

    pub(crate) fn fail(&self, error: JobError) -> Result<(), JobRegistryError> {
        self.registry.fail(&self.id, error)
    }

    pub(crate) fn attach_abort_handle(
        &self,
        abort_handle: tokio::task::AbortHandle,
    ) -> Result<(), JobRegistryError> {
        self.registry.attach_abort_handle(&self.id, abort_handle)
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }
}

const COMPLETE_PHASE_ORDER: u8 = u8::MAX;

struct JobRecord<R> {
    id: String,
    state: &'static str,
    phase: &'static str,
    phase_order: u8,
    progress: Option<JobProgress>,
    result: Option<R>,
    error: Option<JobError>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    terminal_at: Option<DateTime<Utc>>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl<R: Clone> JobRecord<R> {
    fn new(id: String, now: DateTime<Utc>) -> Self {
        Self {
            id,
            state: "queued",
            phase: "queued",
            phase_order: 0,
            progress: None,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            terminal_at: None,
            abort_handle: None,
        }
    }

    fn snapshot(&self) -> JobSnapshot<R> {
        JobSnapshot {
            id: self.id.clone(),
            state: self.state,
            phase: self.phase,
            progress: self.progress.clone(),
            result: self.result.clone(),
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    fn touch(&mut self) {
        self.updated_at = self.updated_at.max(Utc::now());
    }
}

fn generate_id(id_prefix: &str) -> String {
    let mut bytes = [0_u8; RANDOM_BYTES];
    OsRng.fill_bytes(&mut bytes);
    format!("{id_prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

fn running_job<'a, R>(
    jobs: &'a mut HashMap<String, JobRecord<R>>,
    id: &str,
) -> Result<&'a mut JobRecord<R>, JobRegistryError> {
    let job = jobs.get_mut(id).ok_or(JobRegistryError::NotFound)?;
    if job.state != "running" && !terminal(job.state) {
        return Err(JobRegistryError::IllegalState);
    }
    Ok(job)
}

fn terminal(state: &str) -> bool {
    matches!(state, "succeeded" | "failed" | "cancelled")
}

fn prune<R>(jobs: &mut HashMap<String, JobRecord<R>>, now: DateTime<Utc>) {
    let retention = chrono::Duration::from_std(TERMINAL_RETENTION)
        .expect("terminal retention fits chrono duration");
    jobs.retain(|_, job| {
        job.terminal_at
            .is_none_or(|at| now.signed_duration_since(at) < retention)
    });

    let mut terminal = jobs
        .iter()
        .filter_map(|(id, job)| job.terminal_at.map(|at| (at, id.clone())))
        .collect::<Vec<_>>();
    if terminal.len() > TERMINAL_CAPACITY {
        terminal.sort();
        let remove_count = terminal.len() - TERMINAL_CAPACITY;
        for (_, id) in terminal.into_iter().take(remove_count) {
            jobs.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_advance_only_forwards_and_terminal_jobs_are_bounded() {
        let registry = Arc::new(JobRegistry::<u32>::new("job_"));
        let (active, _) = registry.reserve();
        let (snapshot, reporter) = registry.reserve();
        reporter.start().unwrap();
        reporter.set_phase("downloading", 3).unwrap();
        reporter.set_progress(5, 10).unwrap();
        assert!(reporter.set_phase("resolving", 1).is_err());
        assert!(reporter.set_progress(1, 10).is_err());
        reporter.set_phase("downloading", 3).unwrap();
        let observed = registry.snapshot(&snapshot.id).unwrap();
        assert_eq!(observed.phase, "downloading");
        assert_eq!(observed.progress.unwrap().current, 5);

        reporter.fail(JobError::new("first", "first")).unwrap();
        reporter.fail(JobError::new("late", "late")).unwrap();
        let failed = registry.snapshot(&snapshot.id).unwrap();
        assert_eq!(failed.state, "failed");
        assert_eq!(failed.error.unwrap().code, "first");

        for _ in 0..TERMINAL_CAPACITY {
            let (_, reporter) = registry.reserve();
            reporter.start().unwrap();
            reporter.fail(JobError::new("failed", "failed")).unwrap();
        }
        assert!(registry.snapshot(&active.id).is_ok());
        assert!(
            registry
                .lock()
                .values()
                .filter(|job| terminal(job.state))
                .count()
                <= TERMINAL_CAPACITY
        );
    }
}
