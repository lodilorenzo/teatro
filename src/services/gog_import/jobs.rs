//! DTO-independent, bounded in-memory state for asynchronous GOG imports.
//!
//! The asynchronous admin HTTP lifecycle creates and polls these jobs. The registry keeps
//! critical sections synchronous and short so reporters can be called from Tokio tasks and
//! blocking archive workers without awaiting a consumer.

use std::{
    collections::{HashMap, VecDeque},
    fmt,
    str::FromStr,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use thiserror::Error;

use super::super::bounded_text;
use super::GogImportOutcome;

pub(crate) const TERMINAL_JOB_CAPACITY: usize = 64;
pub(crate) const TERMINAL_JOB_RETENTION: Duration = Duration::from_secs(60 * 60);
pub(crate) const MAX_RETAINED_EVENT_TEXT_BYTES: usize = 256 * 1024;
pub(crate) const MAX_RETAINED_EVENTS: usize = 512;
pub(crate) const MAX_DISPLAY_ENTRY_BYTES: usize = 4 * 1024;
pub(crate) const MAX_PROGRESS_UPDATES_PER_SECOND: u32 = 10;

const JOB_ID_PREFIX: &str = "gog_";
const JOB_ID_RANDOM_BYTES: usize = 32;
const MAX_JOB_ID_BYTES: usize = 128;
const MAX_ERROR_MESSAGE_BYTES: usize = 1024;
const PROGRESS_UPDATE_INTERVAL: Duration =
    Duration::from_millis(1000 / MAX_PROGRESS_UPDATES_PER_SECOND as u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct GogImportJobId(String);

impl GogImportJobId {
    fn generate() -> Self {
        // Match the API-token generator's OS CSPRNG and URL-safe encoding. A dedicated
        // 256-bit value keeps public job IDs independent from sequential operation IDs.
        let mut bytes = [0_u8; JOB_ID_RANDOM_BYTES];
        OsRng.fill_bytes(&mut bytes);
        Self(format!("{JOB_ID_PREFIX}{}", URL_SAFE_NO_PAD.encode(bytes)))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GogImportJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for GogImportJobId {
    type Err = InvalidGogImportJobValue;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let suffix = value.strip_prefix(JOB_ID_PREFIX).unwrap_or_default();
        if value.len() > MAX_JOB_ID_BYTES
            || suffix.is_empty()
            || !suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(InvalidGogImportJobValue::new("job id", value));
        }
        Ok(Self(value.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GogImportJobState {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl GogImportJobState {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

impl fmt::Display for GogImportJobState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum GogImportPhase {
    Queued,
    ValidateRequest,
    ResolveWindowsTarget,
    VerifyInnoextractHash,
    PreparePrivateWorkspace,
    StageSetupFiles,
    ProbeInnoextractVersion,
    ProbeInstallerDataVersion,
    TestInstallerIntegrity,
    ExtractInstallerPayload,
    ValidateExtractedTree,
    CreateWindowsZip,
    FinalizeWindowsZip,
    ValidateSiparioZip,
    StageGeneratedArchive,
    PublishWindowsArchive,
    CleanupUploadedSetupStaging,
    Complete,
}

impl GogImportPhase {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::ValidateRequest => "validate_request",
            Self::ResolveWindowsTarget => "resolve_windows_target",
            Self::VerifyInnoextractHash => "verify_innoextract_hash",
            Self::PreparePrivateWorkspace => "prepare_private_workspace",
            Self::StageSetupFiles => "stage_setup_files",
            Self::ProbeInnoextractVersion => "probe_innoextract_version",
            Self::ProbeInstallerDataVersion => "probe_installer_data_version",
            Self::TestInstallerIntegrity => "test_installer_integrity",
            Self::ExtractInstallerPayload => "extract_installer_payload",
            Self::ValidateExtractedTree => "validate_extracted_tree",
            Self::CreateWindowsZip => "create_windows_zip",
            Self::FinalizeWindowsZip => "finalize_windows_zip",
            Self::ValidateSiparioZip => "validate_sipario_zip",
            Self::StageGeneratedArchive => "stage_generated_archive",
            Self::PublishWindowsArchive => "publish_windows_archive",
            Self::CleanupUploadedSetupStaging => "cleanup_uploaded_setup_staging",
            Self::Complete => "complete",
        }
    }

    pub(crate) const fn display_label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::ValidateRequest => "Validating request",
            Self::ResolveWindowsTarget => "Resolving Windows target",
            Self::VerifyInnoextractHash => "Verifying innoextract",
            Self::PreparePrivateWorkspace => "Preparing private workspace",
            Self::StageSetupFiles => "Staging setup files",
            Self::ProbeInnoextractVersion => "Checking innoextract version",
            Self::ProbeInstallerDataVersion => "Checking installer data version",
            Self::TestInstallerIntegrity => "Testing installer integrity",
            Self::ExtractInstallerPayload => "Extracting installer payload",
            Self::ValidateExtractedTree => "Validating extracted tree",
            Self::CreateWindowsZip => "Creating Windows ZIP",
            Self::FinalizeWindowsZip => "Finalizing Windows ZIP",
            Self::ValidateSiparioZip => "Validating Windows ZIP",
            Self::StageGeneratedArchive => "Staging generated archive",
            Self::PublishWindowsArchive => "Publishing Windows archive",
            Self::CleanupUploadedSetupStaging => "Cleaning uploaded setup staging",
            Self::Complete => "GOG setup import complete",
        }
    }
}

impl fmt::Display for GogImportPhase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GogImportProgressKind {
    Percent,
    Bytes,
}

impl GogImportProgressKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Percent => "percent",
            Self::Bytes => "bytes",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GogImportProgress {
    pub(crate) kind: GogImportProgressKind,
    pub(crate) current: Option<u64>,
    pub(crate) total: Option<u64>,
    pub(crate) percent: f64,
}

impl GogImportProgress {
    pub(crate) fn percent(percent: f64) -> Result<Self, InvalidGogImportProgress> {
        if !percent.is_finite() || !(0.0..=100.0).contains(&percent) {
            return Err(InvalidGogImportProgress::Percent);
        }
        Ok(Self {
            kind: GogImportProgressKind::Percent,
            current: None,
            total: None,
            percent: if percent == 0.0 { 0.0 } else { percent },
        })
    }

    pub(crate) fn bytes(current: u64, total: u64) -> Result<Self, InvalidGogImportProgress> {
        if current > total {
            return Err(InvalidGogImportProgress::ByteRange);
        }
        let percent = if total == 0 {
            100.0
        } else {
            ((current as f64 / total as f64) * 100.0).clamp(0.0, 100.0)
        };
        Ok(Self {
            kind: GogImportProgressKind::Bytes,
            current: Some(current),
            total: Some(total),
            percent,
        })
    }

    fn follows(&self, previous: &Self) -> Result<(), GogImportJobRegistryError> {
        if self.kind != previous.kind {
            return Err(GogImportJobRegistryError::ProgressKindChanged);
        }
        match self.kind {
            GogImportProgressKind::Percent if self.percent < previous.percent => {
                Err(GogImportJobRegistryError::ProgressRegressed)
            }
            GogImportProgressKind::Bytes
                if self.total != previous.total || self.current < previous.current =>
            {
                Err(GogImportJobRegistryError::ProgressRegressed)
            }
            _ => Ok(()),
        }
    }
}

/// Locally throttles byte counters before they reach the registry's phase-level coalescer.
///
/// The zero and completed boundaries are always offered to the callback. Intermediate updates are
/// offered at most at the frozen publication rate, and invalid/regressing counters are ignored so
/// observability can never make file processing fail.
pub(crate) struct GogImportByteProgress<F> {
    total_bytes: u64,
    last_observed_bytes: Option<u64>,
    last_published_bytes: Option<u64>,
    last_published_at: Option<Instant>,
    finished: bool,
    publish: F,
}

impl<F> GogImportByteProgress<F>
where
    F: FnMut(u64, u64),
{
    pub(crate) fn new(total_bytes: u64, publish: F) -> Self {
        Self {
            total_bytes,
            last_observed_bytes: None,
            last_published_bytes: None,
            last_published_at: None,
            finished: false,
            publish,
        }
    }

    pub(crate) fn start(&mut self) {
        self.start_at(Instant::now());
    }

    pub(crate) fn update(&mut self, current_bytes: u64) {
        self.update_at(current_bytes, Instant::now());
    }

    pub(crate) fn finish(&mut self, current_bytes: u64) {
        self.finish_at(current_bytes, Instant::now());
    }

    fn start_at(&mut self, now: Instant) {
        if self.finished || self.last_observed_bytes.is_some() {
            return;
        }
        self.last_observed_bytes = Some(0);
        self.publish_at(0, now);
    }

    fn update_at(&mut self, current_bytes: u64, now: Instant) {
        if self.finished || current_bytes > self.total_bytes {
            return;
        }
        if self.last_observed_bytes.is_none() {
            self.start_at(now);
        }
        if self
            .last_observed_bytes
            .is_some_and(|previous| current_bytes < previous)
        {
            return;
        }
        self.last_observed_bytes = Some(current_bytes);
        if self.last_published_bytes == Some(current_bytes) {
            return;
        }
        let ready = self
            .last_published_at
            .is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_UPDATE_INTERVAL);
        if ready {
            self.publish_at(current_bytes, now);
        }
    }

    fn finish_at(&mut self, current_bytes: u64, now: Instant) {
        if self.finished || current_bytes != self.total_bytes {
            return;
        }
        if self.last_observed_bytes.is_none() {
            self.start_at(now);
        }
        if self
            .last_observed_bytes
            .is_some_and(|previous| current_bytes < previous)
        {
            return;
        }
        self.last_observed_bytes = Some(current_bytes);
        if self.last_published_bytes != Some(current_bytes) {
            self.publish_at(current_bytes, now);
        }
        self.finished = true;
    }

    fn publish_at(&mut self, current_bytes: u64, now: Instant) {
        self.last_published_bytes = Some(current_bytes);
        self.last_published_at = Some(now);
        (self.publish)(current_bytes, self.total_bytes);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GogImportJobEventKind {
    Phase,
    Output,
}

impl GogImportJobEventKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GogImportOutputStream {
    Stdout,
    Stderr,
}

impl GogImportOutputStream {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GogImportJobEvent {
    pub(crate) seq: u64,
    pub(crate) kind: GogImportJobEventKind,
    pub(crate) phase: GogImportPhase,
    pub(crate) stream: Option<GogImportOutputStream>,
    pub(crate) text: String,
}

#[derive(Debug, Clone)]
pub(crate) struct GogImportJobResult {
    pub(crate) outcome: GogImportOutcome,
}

impl From<GogImportOutcome> for GogImportJobResult {
    fn from(outcome: GogImportOutcome) -> Self {
        Self { outcome }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GogImportJobErrorSummary {
    pub(crate) code: String,
    pub(crate) message: String,
}

impl GogImportJobErrorSummary {
    pub(crate) fn new(code: impl AsRef<str>, message: impl AsRef<str>) -> Self {
        let (code, _) = bounded_text(code.as_ref(), MAX_ERROR_MESSAGE_BYTES);
        let (message, _) = bounded_text(message.as_ref(), MAX_ERROR_MESSAGE_BYTES);
        Self { code, message }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GogImportJobSnapshot {
    pub(crate) id: GogImportJobId,
    pub(crate) state: GogImportJobState,
    pub(crate) phase: GogImportPhase,
    pub(crate) progress: Option<GogImportProgress>,
    pub(crate) events: Vec<GogImportJobEvent>,
    pub(crate) next_event_seq: u64,
    pub(crate) output_truncated: bool,
    pub(crate) result: Option<GogImportJobResult>,
    pub(crate) error: Option<GogImportJobErrorSummary>,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

pub(crate) struct GogImportJobReservation {
    pub(crate) snapshot: GogImportJobSnapshot,
    pub(crate) reporter: GogImportJobReporter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GogImportJobUpdate {
    Applied,
    Coalesced,
    Unchanged,
    IgnoredTerminal,
    IgnoredStalePhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid {kind} value {value:?}")]
pub(crate) struct InvalidGogImportJobValue {
    kind: &'static str,
    value: String,
}

impl InvalidGogImportJobValue {
    fn new(kind: &'static str, value: &str) -> Self {
        Self {
            kind,
            value: value.to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub(crate) enum InvalidGogImportProgress {
    #[error("progress percent must be finite and between zero and 100")]
    Percent,
    #[error("progress current bytes cannot exceed total bytes")]
    ByteRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub(crate) enum GogImportJobRegistryError {
    #[error("GOG import job was not found")]
    NotFound,
    #[error("event cursor {after} is newer than the job sequence {latest}")]
    FutureCursor { after: u64, latest: u64 },
    #[error("cannot {operation} a GOG import job in state {state}")]
    IllegalState {
        operation: &'static str,
        state: GogImportJobState,
    },
    #[error("cannot move GOG import phase from {from} to {to}")]
    InvalidPhaseTransition {
        from: GogImportPhase,
        to: GogImportPhase,
    },
    #[error("progress kind cannot change within one phase")]
    ProgressKindChanged,
    #[error("progress cannot decrease within one phase")]
    ProgressRegressed,
    #[error("GOG import has reached its non-cancellable publication phase")]
    TooLateToCancel,
}

pub(crate) struct GogImportJobRegistry {
    inner: Mutex<RegistryInner>,
}

impl Default for GogImportJobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl GogImportJobRegistry {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
        }
    }

    pub(crate) fn reserve(self: &Arc<Self>) -> GogImportJobReservation {
        self.reserve_at(Utc::now())
    }

    pub(crate) fn snapshot(
        &self,
        id: &GogImportJobId,
        after: u64,
    ) -> Result<GogImportJobSnapshot, GogImportJobRegistryError> {
        self.snapshot_at(id, after, Utc::now(), Instant::now())
    }

    fn reserve_at(self: &Arc<Self>, now: DateTime<Utc>) -> GogImportJobReservation {
        loop {
            // Do not call the OS random source while holding the registry mutex.
            let id = GogImportJobId::generate();
            let mut inner = self.lock();
            self.prune_locked(&mut inner, now);
            if inner.jobs.contains_key(&id) {
                continue;
            }

            let job = JobRecord::new(id.clone(), now);
            let snapshot = job.snapshot_after(0);
            inner.jobs.insert(id.clone(), job);
            drop(inner);
            return GogImportJobReservation {
                snapshot,
                reporter: GogImportJobReporter {
                    registry: Arc::clone(self),
                    id,
                },
            };
        }
    }

    fn snapshot_at(
        &self,
        id: &GogImportJobId,
        after: u64,
        wall_now: DateTime<Utc>,
        monotonic_now: Instant,
    ) -> Result<GogImportJobSnapshot, GogImportJobRegistryError> {
        let mut inner = self.lock();
        self.prune_locked(&mut inner, wall_now);
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if after > job.next_event_seq {
            return Err(GogImportJobRegistryError::FutureCursor {
                after,
                latest: job.next_event_seq,
            });
        }
        job.flush_pending_progress(wall_now, monotonic_now);
        Ok(job.snapshot_after(after))
    }

    fn transition_to_running(
        &self,
        id: &GogImportJobId,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(GogImportJobUpdate::IgnoredTerminal);
        }
        if job.state != GogImportJobState::Queued {
            return Err(GogImportJobRegistryError::IllegalState {
                operation: "start",
                state: job.state,
            });
        }
        job.state = GogImportJobState::Running;
        job.touch(now);
        Ok(GogImportJobUpdate::Applied)
    }

    fn set_phase(
        &self,
        id: &GogImportJobId,
        phase: GogImportPhase,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(GogImportJobUpdate::IgnoredTerminal);
        }
        if job.state != GogImportJobState::Running {
            return Err(GogImportJobRegistryError::IllegalState {
                operation: "set phase on",
                state: job.state,
            });
        }
        if phase == job.phase {
            return Ok(GogImportJobUpdate::Unchanged);
        }
        if matches!(phase, GogImportPhase::Queued | GogImportPhase::Complete) || phase < job.phase {
            return Err(GogImportJobRegistryError::InvalidPhaseTransition {
                from: job.phase,
                to: phase,
            });
        }

        job.phase = phase;
        job.clear_progress();
        job.append_event(
            GogImportJobEventKind::Phase,
            phase,
            None,
            phase.display_label(),
        );
        job.touch(now);
        Ok(GogImportJobUpdate::Applied)
    }

    fn set_progress(
        &self,
        id: &GogImportJobId,
        phase: GogImportPhase,
        progress: GogImportProgress,
        wall_now: DateTime<Utc>,
        monotonic_now: Instant,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(GogImportJobUpdate::IgnoredTerminal);
        }
        if job.state != GogImportJobState::Running {
            return Err(GogImportJobRegistryError::IllegalState {
                operation: "report progress for",
                state: job.state,
            });
        }
        if phase != job.phase {
            return Ok(GogImportJobUpdate::IgnoredStalePhase);
        }
        if let Some(previous) = job.latest_progress.as_ref() {
            progress.follows(previous)?;
        }
        job.latest_progress = Some(progress.clone());

        let publish_now = job.last_progress_published_at.is_none_or(|last| {
            monotonic_now.saturating_duration_since(last) >= PROGRESS_UPDATE_INTERVAL
        });
        if publish_now {
            job.progress = Some(progress);
            job.pending_progress = None;
            job.last_progress_published_at = Some(monotonic_now);
            job.touch(wall_now);
            Ok(GogImportJobUpdate::Applied)
        } else {
            job.pending_progress = Some(progress);
            Ok(GogImportJobUpdate::Coalesced)
        }
    }

    fn append_output(
        &self,
        id: &GogImportJobId,
        phase: GogImportPhase,
        stream: GogImportOutputStream,
        text: &str,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(GogImportJobUpdate::IgnoredTerminal);
        }
        if job.state != GogImportJobState::Running {
            return Err(GogImportJobRegistryError::IllegalState {
                operation: "append output to",
                state: job.state,
            });
        }
        if phase != job.phase {
            return Ok(GogImportJobUpdate::IgnoredStalePhase);
        }
        job.append_event(GogImportJobEventKind::Output, phase, Some(stream), text);
        job.touch(now);
        Ok(GogImportJobUpdate::Applied)
    }

    fn mark_output_truncated(
        &self,
        id: &GogImportJobId,
        phase: GogImportPhase,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(GogImportJobUpdate::IgnoredTerminal);
        }
        if job.state != GogImportJobState::Running {
            return Err(GogImportJobRegistryError::IllegalState {
                operation: "mark output truncated on",
                state: job.state,
            });
        }
        if phase != job.phase {
            return Ok(GogImportJobUpdate::IgnoredStalePhase);
        }
        if job.output_truncated {
            return Ok(GogImportJobUpdate::Unchanged);
        }
        job.output_truncated = true;
        job.touch(now);
        Ok(GogImportJobUpdate::Applied)
    }

    fn succeed(
        &self,
        id: &GogImportJobId,
        outcome: GogImportOutcome,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        {
            let job = inner
                .jobs
                .get_mut(id)
                .ok_or(GogImportJobRegistryError::NotFound)?;
            if job.state.is_terminal() {
                return Ok(GogImportJobUpdate::IgnoredTerminal);
            }
            if job.state != GogImportJobState::Running {
                return Err(GogImportJobRegistryError::IllegalState {
                    operation: "succeed",
                    state: job.state,
                });
            }
            job.state = GogImportJobState::Succeeded;
            job.abort_handle = None;
            job.phase = GogImportPhase::Complete;
            job.clear_progress();
            job.append_event(
                GogImportJobEventKind::Phase,
                GogImportPhase::Complete,
                None,
                GogImportPhase::Complete.display_label(),
            );
            job.result = Some(GogImportJobResult::from(outcome));
            job.error = None;
            job.touch(now);
            job.terminal_at = Some(job.updated_at);
        }
        self.prune_locked(&mut inner, now);
        Ok(GogImportJobUpdate::Applied)
    }

    fn fail(
        &self,
        id: &GogImportJobId,
        error: GogImportJobErrorSummary,
        now: DateTime<Utc>,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        let mut inner = self.lock();
        {
            let job = inner
                .jobs
                .get_mut(id)
                .ok_or(GogImportJobRegistryError::NotFound)?;
            if job.state.is_terminal() {
                return Ok(GogImportJobUpdate::IgnoredTerminal);
            }
            if job.state != GogImportJobState::Running {
                return Err(GogImportJobRegistryError::IllegalState {
                    operation: "fail",
                    state: job.state,
                });
            }
            job.state = GogImportJobState::Failed;
            job.abort_handle = None;
            job.clear_progress();
            job.result = None;
            job.error = Some(error);
            job.touch(now);
            job.terminal_at = Some(job.updated_at);
        }
        self.prune_locked(&mut inner, now);
        Ok(GogImportJobUpdate::Applied)
    }

    pub(crate) fn cancel(
        &self,
        id: &GogImportJobId,
    ) -> Result<GogImportJobSnapshot, GogImportJobRegistryError> {
        let now = Utc::now();
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            return Ok(job.snapshot_after(0));
        }
        if job.phase >= GogImportPhase::PublishWindowsArchive {
            return Err(GogImportJobRegistryError::TooLateToCancel);
        }
        job.state = GogImportJobState::Cancelled;
        job.clear_progress();
        job.result = None;
        job.error = None;
        job.touch(now);
        job.terminal_at = Some(job.updated_at);
        let abort_handle = job.abort_handle.take();
        let snapshot = job.snapshot_after(0);
        drop(inner);
        if let Some(abort_handle) = abort_handle {
            abort_handle.abort();
        }
        Ok(snapshot)
    }

    fn attach_abort_handle(
        &self,
        id: &GogImportJobId,
        abort_handle: tokio::task::AbortHandle,
    ) -> Result<(), GogImportJobRegistryError> {
        let mut inner = self.lock();
        let job = inner
            .jobs
            .get_mut(id)
            .ok_or(GogImportJobRegistryError::NotFound)?;
        if job.state.is_terminal() {
            drop(inner);
            abort_handle.abort();
        } else {
            job.abort_handle = Some(abort_handle);
        }
        Ok(())
    }

    fn prune_locked(&self, inner: &mut RegistryInner, now: DateTime<Utc>) -> usize {
        let before = inner.jobs.len();
        let retention = chrono::Duration::from_std(TERMINAL_JOB_RETENTION)
            .expect("terminal retention fits chrono duration");
        inner.jobs.retain(|_, job| {
            job.terminal_at
                .is_none_or(|terminal_at| now.signed_duration_since(terminal_at) < retention)
        });

        let mut terminal_jobs = inner
            .jobs
            .iter()
            .filter_map(|(id, job)| job.terminal_at.map(|terminal_at| (terminal_at, id.clone())))
            .collect::<Vec<_>>();
        if terminal_jobs.len() > TERMINAL_JOB_CAPACITY {
            terminal_jobs
                .sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
            let remove_count = terminal_jobs.len() - TERMINAL_JOB_CAPACITY;
            for (_, id) in terminal_jobs.into_iter().take(remove_count) {
                inner.jobs.remove(&id);
            }
        }
        before - inner.jobs.len()
    }

    fn lock(&self) -> MutexGuard<'_, RegistryInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

#[derive(Clone)]
pub(crate) struct GogImportJobReporter {
    registry: Arc<GogImportJobRegistry>,
    id: GogImportJobId,
}

/// Phase-scoped workflow reporting that can be disabled for focused service tests.
///
/// Job lifecycle transitions remain on `GogImportJobReporter`; this narrower wrapper lets the
/// service and blocking archive worker publish observability without making correctness depend on
/// a polling consumer or registry update.
#[derive(Clone, Default)]
pub(crate) struct GogImportWorkflowReporter {
    job: Option<GogImportJobReporter>,
}

impl GogImportWorkflowReporter {
    pub(crate) fn new(job: GogImportJobReporter) -> Self {
        Self { job: Some(job) }
    }

    pub(crate) fn noop() -> Self {
        Self::default()
    }

    pub(crate) fn set_phase(
        &self,
        phase: GogImportPhase,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        match &self.job {
            Some(job) => job.set_phase(phase),
            None => Ok(GogImportJobUpdate::Unchanged),
        }
    }

    pub(crate) fn set_progress(
        &self,
        phase: GogImportPhase,
        progress: GogImportProgress,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        match &self.job {
            Some(job) => job.set_progress(phase, progress),
            None => Ok(GogImportJobUpdate::Unchanged),
        }
    }

    pub(crate) fn report_bytes(&self, phase: GogImportPhase, current: u64, total: u64) {
        if let Ok(progress) = GogImportProgress::bytes(current, total) {
            let _ = self.set_progress(phase, progress);
        }
    }

    pub(crate) fn append_output(
        &self,
        phase: GogImportPhase,
        stream: GogImportOutputStream,
        text: &str,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        match &self.job {
            Some(job) => job.append_output(phase, stream, text),
            None => Ok(GogImportJobUpdate::Unchanged),
        }
    }

    pub(crate) fn mark_output_truncated(
        &self,
        phase: GogImportPhase,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        match &self.job {
            Some(job) => job.mark_output_truncated(phase),
            None => Ok(GogImportJobUpdate::Unchanged),
        }
    }
}

impl From<GogImportJobReporter> for GogImportWorkflowReporter {
    fn from(job: GogImportJobReporter) -> Self {
        Self::new(job)
    }
}

impl GogImportJobReporter {
    pub(crate) fn start(&self) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry.transition_to_running(&self.id, Utc::now())
    }

    pub(crate) fn set_phase(
        &self,
        phase: GogImportPhase,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry.set_phase(&self.id, phase, Utc::now())
    }

    pub(crate) fn set_progress(
        &self,
        phase: GogImportPhase,
        progress: GogImportProgress,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry
            .set_progress(&self.id, phase, progress, Utc::now(), Instant::now())
    }

    pub(crate) fn append_output(
        &self,
        phase: GogImportPhase,
        stream: GogImportOutputStream,
        text: &str,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry
            .append_output(&self.id, phase, stream, text, Utc::now())
    }

    pub(crate) fn mark_output_truncated(
        &self,
        phase: GogImportPhase,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry
            .mark_output_truncated(&self.id, phase, Utc::now())
    }

    pub(crate) fn succeed(
        &self,
        outcome: GogImportOutcome,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry.succeed(&self.id, outcome, Utc::now())
    }

    pub(crate) fn fail(
        &self,
        error: GogImportJobErrorSummary,
    ) -> Result<GogImportJobUpdate, GogImportJobRegistryError> {
        self.registry.fail(&self.id, error, Utc::now())
    }

    pub(crate) fn attach_abort_handle(
        &self,
        abort_handle: tokio::task::AbortHandle,
    ) -> Result<(), GogImportJobRegistryError> {
        self.registry.attach_abort_handle(&self.id, abort_handle)
    }
}

#[derive(Default)]
struct RegistryInner {
    jobs: HashMap<GogImportJobId, JobRecord>,
}

struct JobRecord {
    id: GogImportJobId,
    state: GogImportJobState,
    phase: GogImportPhase,
    progress: Option<GogImportProgress>,
    latest_progress: Option<GogImportProgress>,
    pending_progress: Option<GogImportProgress>,
    last_progress_published_at: Option<Instant>,
    events: VecDeque<GogImportJobEvent>,
    retained_event_text_bytes: usize,
    next_event_seq: u64,
    output_truncated: bool,
    result: Option<GogImportJobResult>,
    error: Option<GogImportJobErrorSummary>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    terminal_at: Option<DateTime<Utc>>,
    abort_handle: Option<tokio::task::AbortHandle>,
}

impl JobRecord {
    fn new(id: GogImportJobId, now: DateTime<Utc>) -> Self {
        Self {
            id,
            state: GogImportJobState::Queued,
            phase: GogImportPhase::Queued,
            progress: None,
            latest_progress: None,
            pending_progress: None,
            last_progress_published_at: None,
            events: VecDeque::new(),
            retained_event_text_bytes: 0,
            next_event_seq: 0,
            output_truncated: false,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            terminal_at: None,
            abort_handle: None,
        }
    }

    fn touch(&mut self, now: DateTime<Utc>) {
        if now > self.updated_at {
            self.updated_at = now;
        }
    }

    fn clear_progress(&mut self) {
        self.progress = None;
        self.latest_progress = None;
        self.pending_progress = None;
        self.last_progress_published_at = None;
    }

    fn flush_pending_progress(&mut self, wall_now: DateTime<Utc>, monotonic_now: Instant) {
        let can_publish = self.pending_progress.is_some()
            && self.last_progress_published_at.is_none_or(|last| {
                monotonic_now.saturating_duration_since(last) >= PROGRESS_UPDATE_INTERVAL
            });
        if can_publish {
            self.progress = self.pending_progress.take();
            self.last_progress_published_at = Some(monotonic_now);
            self.touch(wall_now);
        }
    }

    fn append_event(
        &mut self,
        kind: GogImportJobEventKind,
        phase: GogImportPhase,
        stream: Option<GogImportOutputStream>,
        text: &str,
    ) {
        let Some(seq) = self.next_event_seq.checked_add(1) else {
            self.output_truncated = true;
            return;
        };
        self.next_event_seq = seq;
        let (text, shortened) = bounded_text(text, MAX_DISPLAY_ENTRY_BYTES);
        self.output_truncated |= shortened;
        self.retained_event_text_bytes = self.retained_event_text_bytes.saturating_add(text.len());
        self.events.push_back(GogImportJobEvent {
            seq,
            kind,
            phase,
            stream,
            text,
        });

        while self.events.len() > MAX_RETAINED_EVENTS
            || self.retained_event_text_bytes > MAX_RETAINED_EVENT_TEXT_BYTES
        {
            let Some(position) = self
                .events
                .iter()
                .position(|event| event.kind == GogImportJobEventKind::Output)
            else {
                // The closed phase set cannot exceed the production bounds. Preserve phase
                // history rather than violating the retention policy if that ever changes.
                debug_assert!(
                    self.events.len() <= MAX_RETAINED_EVENTS
                        && self.retained_event_text_bytes <= MAX_RETAINED_EVENT_TEXT_BYTES,
                    "phase events exceeded GOG import retention limits"
                );
                break;
            };
            if let Some(removed) = self.events.remove(position) {
                self.retained_event_text_bytes = self
                    .retained_event_text_bytes
                    .saturating_sub(removed.text.len());
                self.output_truncated = true;
            }
        }
    }

    fn snapshot_after(&self, after: u64) -> GogImportJobSnapshot {
        GogImportJobSnapshot {
            id: self.id.clone(),
            state: self.state,
            phase: self.phase,
            progress: self.progress.clone(),
            events: self
                .events
                .iter()
                .filter(|event| event.seq > after)
                .cloned()
                .collect(),
            next_event_seq: self.next_event_seq,
            output_truncated: self.output_truncated,
            result: self.result.clone(),
            error: self.error.clone(),
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        sync::{Arc, Barrier},
        thread,
    };

    use chrono::{Duration as ChronoDuration, TimeZone};
    use serde_json::json;

    use super::*;
    use crate::{
        domain::rom::Rom,
        services::{gog_import::GogImportSummary, library::UploadedRom},
    };

    fn base_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 11, 20, 0, 0)
            .single()
            .unwrap()
    }

    fn failed(code: &'static str) -> GogImportJobErrorSummary {
        GogImportJobErrorSummary::new(code, "GOG setup import failed")
    }

    fn outcome() -> GogImportOutcome {
        GogImportOutcome {
            uploaded: UploadedRom {
                rom: Rom {
                    id: 42,
                    name: "Example Game".to_string(),
                    slug: "example-game".to_string(),
                    platform_id: 157,
                    platform_slug: "win".to_string(),
                    platform_display_name: "Windows".to_string(),
                    regions: Vec::new(),
                    metadata: json!({}),
                    summary: None,
                    fs_name: Some("Example Game.zip".to_string()),
                    fs_size_bytes: Some(123),
                    path_cover_large: None,
                    path_cover_small: None,
                    url_cover: None,
                    files: Vec::new(),
                },
                file_id: 52,
                file_name: "Example Game.zip".to_string(),
                relative_path: "win/Example Game.zip".to_string(),
                file_size_bytes: 123,
            },
            summary: GogImportSummary::default(),
        }
    }

    fn start_at(registry: &GogImportJobRegistry, id: &GogImportJobId, now: DateTime<Utc>) {
        assert_eq!(
            registry.transition_to_running(id, now).unwrap(),
            GogImportJobUpdate::Applied
        );
    }

    #[test]
    fn generated_ids_are_valid_and_reject_invalid_input() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let ids = (0..128)
            .map(|_| registry.reserve().snapshot.id)
            .collect::<BTreeSet<_>>();
        assert_eq!(ids.len(), 128);
        for id in ids {
            assert!(id.as_str().starts_with(JOB_ID_PREFIX));
            assert_eq!(id.as_str().len(), JOB_ID_PREFIX.len() + 43);
            assert_eq!(id.as_str().parse::<GogImportJobId>().unwrap(), id);
        }
        assert!("gog_/bad".parse::<GogImportJobId>().is_err());
    }

    #[tokio::test]
    async fn cancellation_is_terminal_and_aborts_attached_work() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id.clone();
        reservation.reporter.start().unwrap();
        let worker = tokio::spawn(std::future::pending::<()>());
        reservation
            .reporter
            .attach_abort_handle(worker.abort_handle())
            .unwrap();

        let snapshot = registry.cancel(&id).unwrap();
        assert_eq!(snapshot.state, GogImportJobState::Cancelled);
        assert!(worker.await.unwrap_err().is_cancelled());
        assert_eq!(
            reservation.reporter.fail(failed("late")).unwrap(),
            GogImportJobUpdate::IgnoredTerminal
        );
    }

    #[test]
    fn progress_values_are_finite_bounded_and_structurally_closed() {
        assert_eq!(
            GogImportProgress::percent(42.5).unwrap(),
            GogImportProgress {
                kind: GogImportProgressKind::Percent,
                current: None,
                total: None,
                percent: 42.5,
            }
        );
        assert_eq!(GogImportProgress::bytes(5, 10).unwrap().percent, 50.0);
        assert_eq!(GogImportProgress::bytes(0, 0).unwrap().percent, 100.0);
        assert!(GogImportProgress::percent(f64::NAN).is_err());
        assert!(GogImportProgress::percent(100.1).is_err());
        assert!(GogImportProgress::bytes(11, 10).is_err());
    }

    #[test]
    fn byte_progress_throttles_intermediate_updates_and_preserves_boundaries() {
        let started = Instant::now();
        let mut published = Vec::new();
        {
            let mut progress =
                GogImportByteProgress::new(10, |current, total| published.push((current, total)));
            progress.start_at(started);
            progress.update_at(2, started + Duration::from_millis(50));
            progress.update_at(1, started + Duration::from_millis(60));
            progress.update_at(4, started + PROGRESS_UPDATE_INTERVAL);
            progress.update_at(
                10,
                started + PROGRESS_UPDATE_INTERVAL + Duration::from_millis(1),
            );
            progress.finish_at(
                10,
                started + PROGRESS_UPDATE_INTERVAL + Duration::from_millis(2),
            );
            progress.update_at(9, started + Duration::from_secs(1));
        }
        assert_eq!(published, vec![(0, 10), (4, 10), (10, 10)]);

        let mut zero_byte = Vec::new();
        {
            let mut progress = GogImportByteProgress::new(0, |current, total| {
                zero_byte.push((current, total));
            });
            progress.start_at(started);
            progress.finish_at(0, started);
        }
        assert_eq!(zero_byte, vec![(0, 0)]);
    }

    #[test]
    fn transitions_are_closed_and_terminal_state_is_immutable() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        assert_eq!(reservation.snapshot.state, GogImportJobState::Queued);
        assert_eq!(reservation.snapshot.phase, GogImportPhase::Queued);
        assert_eq!(reservation.snapshot.next_event_seq, 0);

        assert!(matches!(
            registry.set_phase(&id, GogImportPhase::ValidateRequest, now),
            Err(GogImportJobRegistryError::IllegalState { .. })
        ));
        start_at(&registry, &id, now + ChronoDuration::seconds(1));
        assert!(matches!(
            registry.transition_to_running(&id, now + ChronoDuration::seconds(2)),
            Err(GogImportJobRegistryError::IllegalState { .. })
        ));
        assert_eq!(
            registry
                .set_phase(
                    &id,
                    GogImportPhase::ValidateRequest,
                    now + ChronoDuration::seconds(2),
                )
                .unwrap(),
            GogImportJobUpdate::Applied
        );
        assert!(matches!(
            registry.set_phase(
                &id,
                GogImportPhase::Queued,
                now + ChronoDuration::seconds(3)
            ),
            Err(GogImportJobRegistryError::InvalidPhaseTransition { .. })
        ));
        registry
            .fail(
                &id,
                failed("unprocessable_entity"),
                now + ChronoDuration::seconds(4),
            )
            .unwrap();

        assert_eq!(
            registry
                .append_output(
                    &id,
                    GogImportPhase::ValidateRequest,
                    GogImportOutputStream::Stdout,
                    "late output",
                    now + ChronoDuration::seconds(5),
                )
                .unwrap(),
            GogImportJobUpdate::IgnoredTerminal
        );
        assert_eq!(
            registry
                .mark_output_truncated(
                    &id,
                    GogImportPhase::ValidateRequest,
                    now + ChronoDuration::seconds(6),
                )
                .unwrap(),
            GogImportJobUpdate::IgnoredTerminal
        );
        assert_eq!(
            registry
                .succeed(&id, outcome(), now + ChronoDuration::seconds(7))
                .unwrap(),
            GogImportJobUpdate::IgnoredTerminal
        );
        let snapshot = registry
            .snapshot_at(&id, 0, now + ChronoDuration::seconds(8), Instant::now())
            .unwrap();
        assert_eq!(snapshot.state, GogImportJobState::Failed);
        assert_eq!(snapshot.phase, GogImportPhase::ValidateRequest);
        assert!(snapshot.result.is_none());
        assert_eq!(snapshot.error.unwrap().code, "unprocessable_entity");
        assert_eq!(snapshot.next_event_seq, 1);
    }

    #[test]
    fn success_sets_complete_once_and_preserves_the_service_result() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, now + ChronoDuration::seconds(1));
        registry
            .set_phase(
                &id,
                GogImportPhase::CleanupUploadedSetupStaging,
                now + ChronoDuration::seconds(2),
            )
            .unwrap();
        registry
            .succeed(&id, outcome(), now + ChronoDuration::seconds(3))
            .unwrap();

        let snapshot = registry
            .snapshot_at(&id, 0, now + ChronoDuration::seconds(4), Instant::now())
            .unwrap();
        assert_eq!(snapshot.state, GogImportJobState::Succeeded);
        assert_eq!(snapshot.phase, GogImportPhase::Complete);
        assert!(snapshot.progress.is_none());
        assert!(snapshot.error.is_none());
        assert_eq!(snapshot.result.unwrap().outcome.uploaded.rom.id, 42);
        assert_eq!(
            snapshot.events.last().unwrap().phase,
            GogImportPhase::Complete
        );
    }

    #[test]
    fn cursors_are_strict_ordered_and_reject_the_future() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, now);
        registry
            .set_phase(&id, GogImportPhase::ValidateRequest, now)
            .unwrap();
        registry
            .append_output(
                &id,
                GogImportPhase::ValidateRequest,
                GogImportOutputStream::Stdout,
                "one",
                now,
            )
            .unwrap();
        registry
            .append_output(
                &id,
                GogImportPhase::ValidateRequest,
                GogImportOutputStream::Stderr,
                "two",
                now,
            )
            .unwrap();

        let snapshot = registry.snapshot_at(&id, 1, now, Instant::now()).unwrap();
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(snapshot.next_event_seq, 3);
        assert!(matches!(
            registry.snapshot_at(&id, 4, now, Instant::now()),
            Err(GogImportJobRegistryError::FutureCursor {
                after: 4,
                latest: 3
            })
        ));
    }

    #[test]
    fn progress_is_monotonic_coalesced_and_reset_per_phase() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let wall = base_time();
        let monotonic = Instant::now();
        let reservation = registry.reserve_at(wall);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, wall);
        registry
            .set_phase(&id, GogImportPhase::ExtractInstallerPayload, wall)
            .unwrap();

        assert_eq!(
            registry
                .set_progress(
                    &id,
                    GogImportPhase::ExtractInstallerPayload,
                    GogImportProgress::percent(10.0).unwrap(),
                    wall,
                    monotonic,
                )
                .unwrap(),
            GogImportJobUpdate::Applied
        );
        assert_eq!(
            registry
                .set_progress(
                    &id,
                    GogImportPhase::ExtractInstallerPayload,
                    GogImportProgress::percent(40.0).unwrap(),
                    wall + ChronoDuration::milliseconds(50),
                    monotonic + Duration::from_millis(50),
                )
                .unwrap(),
            GogImportJobUpdate::Coalesced
        );
        assert!(matches!(
            registry.set_progress(
                &id,
                GogImportPhase::ExtractInstallerPayload,
                GogImportProgress::percent(39.0).unwrap(),
                wall + ChronoDuration::milliseconds(60),
                monotonic + Duration::from_millis(60),
            ),
            Err(GogImportJobRegistryError::ProgressRegressed)
        ));

        let early = registry
            .snapshot_at(
                &id,
                0,
                wall + ChronoDuration::milliseconds(75),
                monotonic + Duration::from_millis(75),
            )
            .unwrap();
        assert_eq!(early.progress.unwrap().percent, 10.0);
        let flushed = registry
            .snapshot_at(
                &id,
                0,
                wall + ChronoDuration::milliseconds(100),
                monotonic + Duration::from_millis(100),
            )
            .unwrap();
        assert_eq!(flushed.progress.unwrap().percent, 40.0);

        registry
            .set_phase(
                &id,
                GogImportPhase::CreateWindowsZip,
                wall + ChronoDuration::milliseconds(110),
            )
            .unwrap();
        assert_eq!(
            registry
                .set_progress(
                    &id,
                    GogImportPhase::ExtractInstallerPayload,
                    GogImportProgress::percent(90.0).unwrap(),
                    wall + ChronoDuration::milliseconds(120),
                    monotonic + Duration::from_millis(120),
                )
                .unwrap(),
            GogImportJobUpdate::IgnoredStalePhase
        );
        assert!(
            registry
                .set_progress(
                    &id,
                    GogImportPhase::CreateWindowsZip,
                    GogImportProgress::bytes(1, 10).unwrap(),
                    wall + ChronoDuration::milliseconds(120),
                    monotonic + Duration::from_millis(120),
                )
                .is_ok()
        );
    }

    #[test]
    fn output_keeps_a_bounded_sanitized_tail_and_preserves_phase_events() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, now);
        registry
            .set_phase(&id, GogImportPhase::ExtractInstallerPayload, now)
            .unwrap();

        let overlong = format!("{}\n\u{1b}", "é".repeat(MAX_DISPLAY_ENTRY_BYTES));
        for index in 0..600 {
            let text = if index == 0 {
                overlong.as_str()
            } else {
                "ordinary output"
            };
            registry
                .append_output(
                    &id,
                    GogImportPhase::ExtractInstallerPayload,
                    GogImportOutputStream::Stdout,
                    text,
                    now,
                )
                .unwrap();
        }

        let snapshot = registry.snapshot_at(&id, 0, now, Instant::now()).unwrap();
        assert!(snapshot.output_truncated);
        assert_eq!(snapshot.next_event_seq, 601);
        assert!(snapshot.events.len() <= MAX_RETAINED_EVENTS);
        assert!(
            snapshot
                .events
                .iter()
                .map(|event| event.text.len())
                .sum::<usize>()
                <= MAX_RETAINED_EVENT_TEXT_BYTES
        );
        assert_eq!(
            snapshot.events.first().unwrap().kind,
            GogImportJobEventKind::Phase
        );
        assert_eq!(snapshot.events.first().unwrap().seq, 1);
        assert!(
            snapshot
                .events
                .iter()
                .all(|event| !event.text.chars().any(char::is_control))
        );
        let sequences = snapshot
            .events
            .iter()
            .map(|event| event.seq)
            .collect::<Vec<_>>();
        assert!(sequences.windows(2).all(|window| window[0] < window[1]));
        assert!(
            sequences[1] > 2,
            "eviction should leave an explicit sequence gap"
        );
    }

    #[test]
    fn individual_output_entries_end_on_utf8_boundaries_and_make_truncation_sticky() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, now);
        registry
            .set_phase(&id, GogImportPhase::ExtractInstallerPayload, now)
            .unwrap();
        registry
            .append_output(
                &id,
                GogImportPhase::ExtractInstallerPayload,
                GogImportOutputStream::Stdout,
                &format!("line\n{}", "é".repeat(MAX_DISPLAY_ENTRY_BYTES)),
                now,
            )
            .unwrap();

        let snapshot = registry.snapshot_at(&id, 0, now, Instant::now()).unwrap();
        let output = snapshot.events.last().unwrap();
        assert!(snapshot.output_truncated);
        assert!(output.text.len() <= MAX_DISPLAY_ENTRY_BYTES);
        assert!(!output.text.chars().any(char::is_control));
        assert!(output.text.starts_with("line�"));
    }

    #[test]
    fn byte_retention_evicts_old_output_even_below_the_entry_limit() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let reservation = registry.reserve_at(now);
        let id = reservation.snapshot.id;
        start_at(&registry, &id, now);
        registry
            .set_phase(&id, GogImportPhase::ExtractInstallerPayload, now)
            .unwrap();
        let full_entry = "x".repeat(MAX_DISPLAY_ENTRY_BYTES);
        for _ in 0..70 {
            registry
                .append_output(
                    &id,
                    GogImportPhase::ExtractInstallerPayload,
                    GogImportOutputStream::Stderr,
                    &full_entry,
                    now,
                )
                .unwrap();
        }
        let snapshot = registry.snapshot_at(&id, 0, now, Instant::now()).unwrap();
        assert!(snapshot.output_truncated);
        assert!(snapshot.events.len() < 71);
        assert!(
            snapshot
                .events
                .iter()
                .map(|event| event.text.len())
                .sum::<usize>()
                <= MAX_RETAINED_EVENT_TEXT_BYTES
        );
        assert_eq!(
            snapshot.events.first().unwrap().kind,
            GogImportJobEventKind::Phase
        );
    }

    #[test]
    fn retention_expires_terminal_jobs_but_never_active_jobs() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let terminal = registry.reserve_at(now);
        let terminal_id = terminal.snapshot.id;
        start_at(&registry, &terminal_id, now + ChronoDuration::seconds(1));
        registry
            .fail(
                &terminal_id,
                failed("internal_server_error"),
                now + ChronoDuration::seconds(2),
            )
            .unwrap();
        let active_id = registry
            .reserve_at(now + ChronoDuration::seconds(3))
            .snapshot
            .id;

        assert!(
            registry
                .snapshot_at(
                    &terminal_id,
                    0,
                    now + ChronoDuration::seconds(3601),
                    Instant::now(),
                )
                .is_ok()
        );
        assert!(matches!(
            registry.snapshot_at(
                &terminal_id,
                0,
                now + ChronoDuration::seconds(3602),
                Instant::now(),
            ),
            Err(GogImportJobRegistryError::NotFound)
        ));
        assert!(
            registry
                .snapshot_at(&active_id, 0, now + ChronoDuration::days(1), Instant::now(),)
                .is_ok()
        );
    }

    #[test]
    fn terminal_capacity_prunes_oldest_transitions_without_evicting_active_jobs() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let now = base_time();
        let active_ids = (0..70)
            .map(|_| registry.reserve_at(now).snapshot.id)
            .collect::<Vec<_>>();
        let mut terminal_ids = Vec::new();
        for index in 0..=TERMINAL_JOB_CAPACITY {
            let at = now + ChronoDuration::seconds(index as i64 + 1);
            let id = registry.reserve_at(at).snapshot.id;
            start_at(&registry, &id, at);
            registry.fail(&id, failed("failed"), at).unwrap();
            terminal_ids.push(id);
        }

        assert!(matches!(
            registry.snapshot_at(
                &terminal_ids[0],
                0,
                now + ChronoDuration::minutes(2),
                Instant::now(),
            ),
            Err(GogImportJobRegistryError::NotFound)
        ));
        for id in terminal_ids.iter().skip(1) {
            assert!(
                registry
                    .snapshot_at(id, 0, now + ChronoDuration::minutes(2), Instant::now(),)
                    .is_ok()
            );
        }
        for id in active_ids {
            assert!(
                registry
                    .snapshot_at(&id, 0, now + ChronoDuration::days(1), Instant::now(),)
                    .is_ok(),
                "active jobs must not be expired or capacity-pruned"
            );
        }
    }

    #[test]
    fn reporter_updates_are_thread_safe_and_terminal_races_complete_once() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id;
        let reporter = reservation.reporter;
        reporter.start().unwrap();
        reporter
            .set_phase(GogImportPhase::ExtractInstallerPayload)
            .unwrap();

        let barrier = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for worker in 0..8 {
            let reporter = reporter.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                for record in 0..100 {
                    reporter
                        .append_output(
                            GogImportPhase::ExtractInstallerPayload,
                            GogImportOutputStream::Stdout,
                            &format!("worker {worker} record {record}"),
                        )
                        .unwrap();
                }
            }));
        }
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.next_event_seq, 801);
        assert!(snapshot.events.len() <= MAX_RETAINED_EVENTS);
        assert!(snapshot.output_truncated);
        assert!(
            snapshot
                .events
                .windows(2)
                .all(|window| window[0].seq < window[1].seq)
        );

        let barrier = Arc::new(Barrier::new(9));
        let mut finishers = Vec::new();
        for _ in 0..8 {
            let reporter = reporter.clone();
            let barrier = Arc::clone(&barrier);
            finishers.push(thread::spawn(move || {
                barrier.wait();
                reporter.fail(failed("internal_server_error")).unwrap()
            }));
        }
        barrier.wait();
        let outcomes = finishers
            .into_iter()
            .map(|finisher| finisher.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == GogImportJobUpdate::Applied)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == GogImportJobUpdate::IgnoredTerminal)
                .count(),
            7
        );
        assert_eq!(
            registry.snapshot(&id, 0).unwrap().state,
            GogImportJobState::Failed
        );
    }
}
