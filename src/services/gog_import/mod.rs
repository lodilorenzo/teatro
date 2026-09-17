//! Disabled-by-default manual GOG setup extraction and Windows archive import.
//!
//! This workflow runs in an in-process background worker after the HTTP handler safely stages
//! operator-supplied setup files. It invokes one hash-pinned
//! `innoextract` executable without a shell, validates the extracted tree, creates a ZIP in
//! private staging, and publishes only the completed ZIP through the existing recoverable
//! library upload path.

mod archive;
// The job module owns bounded phase, output, percent, and byte-progress publication for the
// asynchronous importer. The browser begins consuming the complete resource in Phase 6.
mod jobs;
mod process;

pub(crate) use jobs::{
    GogImportByteProgress, GogImportJobErrorSummary, GogImportJobEvent, GogImportJobId,
    GogImportJobRegistry, GogImportJobRegistryError, GogImportJobReporter, GogImportJobSnapshot,
    GogImportJobUpdate, GogImportPhase, GogImportProgress, GogImportWorkflowReporter,
};

use std::{
    collections::BTreeSet,
    ffi::OsString,
    io,
    path::{Component, Path, PathBuf},
    time::{Duration, Instant},
};

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tracing::Instrument;

use crate::{
    config::{AppConfig, GogImportConfig},
    domain::workflow::FileOperationKind,
    repositories::{library_roots, platforms},
    services::{
        file_operations::{self, UploadOperationPayload},
        library::{self, LibraryServiceError, UploadDraft, UploadedRom},
    },
    state::AppState,
};

use self::{
    archive::package_extracted_tree,
    process::{ProcessOutput, ProcessReport, run_innoextract, verify_extractor_hash},
};

const WINDOWS_PLATFORM_SLUG: &str = "win";
const MAX_GAME_TITLE_BYTES: usize = 512;
const MAX_SETUP_FILE_NAME_BYTES: usize = 255;
const QUICK_PROBE_TIMEOUT_SECONDS: u64 = 30;

#[must_use = "phase completion or abort should remain visible in tracing"]
pub(crate) struct GogImportPhaseLog {
    phase: GogImportPhase,
    started_at: Instant,
    completed: bool,
}

impl GogImportPhaseLog {
    pub(crate) fn start(reporter: &GogImportWorkflowReporter, phase: GogImportPhase) -> Self {
        if let Err(error) = reporter.set_phase(phase) {
            tracing::error!(
                phase = phase.as_str(),
                error = %error,
                "failed to update GOG import job phase"
            );
        }
        tracing::info!(
            phase = phase.as_str(),
            status = "started",
            "GOG import phase started"
        );
        Self {
            phase,
            started_at: Instant::now(),
            completed: false,
        }
    }

    pub(crate) fn complete(mut self) {
        self.completed = true;
        tracing::info!(
            phase = self.phase.as_str(),
            status = "completed",
            elapsed_ms = elapsed_millis(self.started_at),
            "GOG import phase completed"
        );
    }
}

impl Drop for GogImportPhaseLog {
    fn drop(&mut self) {
        if !self.completed {
            // Drop reports tracing only. It deliberately cannot advance or rewind the job, so an
            // older guard dropped after a newer phase cannot overwrite the current phase.
            tracing::warn!(
                phase = self.phase.as_str(),
                status = "aborted",
                elapsed_ms = elapsed_millis(self.started_at),
                "GOG import phase did not complete"
            );
        }
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone)]
pub(crate) struct GogImportDraft {
    pub title: String,
    pub files: Vec<GogImportFileDraft>,
}

#[derive(Debug, Clone)]
pub(crate) struct GogImportFileDraft {
    pub original_file_name: String,
    pub staged_path: PathBuf,
    pub staging_operation_id: String,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct GogImportOutcome {
    pub uploaded: UploadedRom,
    pub summary: GogImportSummary,
}

#[derive(Debug, Clone, Default, Serialize)]
pub(crate) struct GogImportSummary {
    pub input_file_count: usize,
    pub input_bytes: u64,
    pub extracted_file_count: usize,
    pub extracted_bytes: u64,
    pub launcher_count: usize,
    pub archive_bytes: u64,
    pub extractor_version: String,
    pub data_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct GogImportStatus {
    pub enabled: bool,
    pub configured: bool,
    pub bundled_extractor: bool,
    pub timeout_seconds: u64,
    pub max_extracted_bytes: u64,
    pub max_extracted_files: usize,
}

impl GogImportStatus {
    pub(crate) fn from_config(config: &GogImportConfig) -> Self {
        Self {
            enabled: config.enabled,
            configured: config.is_configured(),
            bundled_extractor: config.bundled_extractor,
            timeout_seconds: config.timeout_seconds,
            max_extracted_bytes: config.max_extracted_bytes,
            max_extracted_files: config.max_extracted_files,
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum GogImportError {
    #[error("GOG setup import is disabled")]
    Disabled,

    #[error("GOG setup import is not configured")]
    NotConfigured,

    #[error("the game title is invalid")]
    InvalidTitle,

    #[error("setup filenames must be flat .exe or .bin names")]
    InvalidSetupFileName,

    #[error("exactly one setup .exe file is required")]
    InvalidSetupExecutableCount,

    #[error("setup filenames must be unique when compared case-insensitively")]
    DuplicateSetupFileName,

    #[error("the setup file set is empty")]
    EmptyInput,

    #[error("the setup file set exceeds the configured aggregate limit")]
    InputTooLarge,

    #[error("the Windows platform is unavailable")]
    WindowsPlatformUnavailable,

    #[error("the default library root is read-only")]
    ReadOnlyRoot,

    #[error("the configured innoextract executable is unavailable")]
    ExtractorUnavailable,

    #[error("the configured innoextract executable does not match its pinned SHA-256")]
    ExtractorHashMismatch,

    #[error("innoextract timed out during {phase}")]
    ExtractorTimedOut { phase: &'static str },

    #[error("innoextract rejected the setup during {phase}")]
    ExtractorRejected { phase: &'static str },

    #[error("innoextract produced warnings or excessive diagnostics during {phase}")]
    ExtractorDiagnostics { phase: &'static str },

    #[error("innoextract did not report a data version")]
    MissingDataVersion,

    #[error("the extracted output is empty")]
    EmptyExtractedOutput,

    #[error("the extracted output contains no .exe or .bat launch candidate")]
    MissingLaunchCandidate,

    #[error("the extracted output contains an unsafe path or file type")]
    UnsafeExtractedOutput,

    #[error("the extracted output contains case-colliding paths")]
    CaseCollidingExtractedPaths,

    #[error("the extracted output contains too many entries")]
    TooManyExtractedEntries,

    #[error("the extracted output exceeds the configured byte limit")]
    ExtractedOutputTooLarge,

    #[error("the extracted output changed while it was being packaged")]
    ExtractedOutputChanged,

    #[error("the generated ZIP does not satisfy the archive safety limits")]
    SiparioArchiveIncompatible,

    #[error("not enough disk space for GOG setup import staging")]
    InsufficientStorage,

    #[error("failed to initialize recoverable archive staging")]
    ArchiveStaging,

    #[error("the GOG import worker failed")]
    WorkerFailed,

    #[error(transparent)]
    Library(#[from] LibraryServiceError),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

impl GogImportError {
    fn log_code(&self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::NotConfigured => "not_configured",
            Self::InvalidTitle => "invalid_title",
            Self::InvalidSetupFileName => "invalid_setup_file_name",
            Self::InvalidSetupExecutableCount => "invalid_setup_executable_count",
            Self::DuplicateSetupFileName => "duplicate_setup_file_name",
            Self::EmptyInput => "empty_input",
            Self::InputTooLarge => "input_too_large",
            Self::WindowsPlatformUnavailable => "windows_platform_unavailable",
            Self::ReadOnlyRoot => "read_only_root",
            Self::ExtractorUnavailable => "extractor_unavailable",
            Self::ExtractorHashMismatch => "extractor_hash_mismatch",
            Self::ExtractorTimedOut { .. } => "extractor_timed_out",
            Self::ExtractorRejected { .. } => "extractor_rejected",
            Self::ExtractorDiagnostics { .. } => "extractor_diagnostics",
            Self::MissingDataVersion => "missing_data_version",
            Self::EmptyExtractedOutput => "empty_extracted_output",
            Self::MissingLaunchCandidate => "missing_launch_candidate",
            Self::UnsafeExtractedOutput => "unsafe_extracted_output",
            Self::CaseCollidingExtractedPaths => "case_colliding_extracted_paths",
            Self::TooManyExtractedEntries => "too_many_extracted_entries",
            Self::ExtractedOutputTooLarge => "extracted_output_too_large",
            Self::ExtractedOutputChanged => "extracted_output_changed",
            Self::SiparioArchiveIncompatible => "sipario_archive_incompatible",
            Self::InsufficientStorage => "insufficient_storage",
            Self::ArchiveStaging => "archive_staging",
            Self::WorkerFailed => "worker_failed",
            Self::Library(_) => "library",
            Self::Database(_) => "database",
            Self::Io(_) => "io",
            Self::Zip(_) => "zip",
        }
    }
}

pub(crate) fn validate_setup_file_name(file_name: &str) -> Result<(), GogImportError> {
    let trimmed = file_name.trim();
    if trimmed.is_empty()
        || trimmed != file_name
        || trimmed.len() > MAX_SETUP_FILE_NAME_BYTES
        || trimmed.contains(['/', '\\', '\0'])
        || trimmed.chars().any(char::is_control)
        || Path::new(trimmed).components().count() != 1
        || !matches!(
            Path::new(trimmed).components().next(),
            Some(Component::Normal(_))
        )
    {
        return Err(GogImportError::InvalidSetupFileName);
    }
    let extension = Path::new(trimmed)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if !extension.eq_ignore_ascii_case("exe") && !extension.eq_ignore_ascii_case("bin") {
        return Err(GogImportError::InvalidSetupFileName);
    }
    Ok(())
}

pub(crate) fn validate_import_admission(
    config: &AppConfig,
    draft: &GogImportDraft,
) -> Result<(), GogImportError> {
    if !config.gog_import.enabled {
        return Err(GogImportError::Disabled);
    }
    if !config.gog_import.is_configured() {
        return Err(GogImportError::NotConfigured);
    }
    normalize_title(&draft.title)?;
    validate_draft(config, draft)?;
    Ok(())
}

pub(crate) async fn import_setup(
    state: &AppState,
    draft: GogImportDraft,
    reporter: GogImportWorkflowReporter,
) -> Result<GogImportOutcome, GogImportError> {
    let import_id = draft
        .files
        .first()
        .map(|file| file.staging_operation_id.as_str())
        .unwrap_or("unassigned")
        .to_string();
    let input_file_count = draft.files.len();
    let input_bytes = draft
        .files
        .iter()
        .map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    let span = tracing::info_span!(
        "gog_import",
        import_id = %import_id,
        input_file_count,
        input_bytes,
        target_platform = WINDOWS_PLATFORM_SLUG,
    );

    async move {
        let started_at = Instant::now();
        tracing::info!(
            phase = "workflow",
            status = "started",
            "GOG setup import started"
        );
        let result = import_setup_inner(state, draft, &reporter).await;
        match &result {
            Ok(outcome) => tracing::info!(
                phase = "workflow",
                status = "completed",
                elapsed_ms = elapsed_millis(started_at),
                rom_id = outcome.uploaded.rom.id,
                extracted_file_count = outcome.summary.extracted_file_count,
                extracted_bytes = outcome.summary.extracted_bytes,
                archive_bytes = outcome.summary.archive_bytes,
                "GOG setup import completed"
            ),
            Err(error) => tracing::warn!(
                phase = "workflow",
                status = "failed",
                elapsed_ms = elapsed_millis(started_at),
                error_code = error.log_code(),
                "GOG setup import failed"
            ),
        }
        result
    }
    .instrument(span)
    .await
}

async fn import_setup_inner(
    state: &AppState,
    draft: GogImportDraft,
    reporter: &GogImportWorkflowReporter,
) -> Result<GogImportOutcome, GogImportError> {
    let validation_phase = GogImportPhaseLog::start(reporter, GogImportPhase::ValidateRequest);
    let config = &state.config().gog_import;
    if !config.enabled {
        return Err(GogImportError::Disabled);
    }
    if !config.is_configured() {
        return Err(GogImportError::NotConfigured);
    }

    let title = normalize_title(&draft.title)?;
    let validated = validate_draft(state.config(), &draft)?;
    validation_phase.complete();

    let target_phase = GogImportPhaseLog::start(reporter, GogImportPhase::ResolveWindowsTarget);
    ensure_windows_target(state).await?;
    target_phase.complete();

    let hash_phase = GogImportPhaseLog::start(reporter, GogImportPhase::VerifyInnoextractHash);
    verify_extractor_hash(config).await?;
    hash_phase.complete();

    let workspace_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::PreparePrivateWorkspace);
    let staging_root = config.staging_root(&state.config().data_dir);
    let workspace = tempfile::Builder::new()
        .prefix("import-")
        .tempdir_in(&staging_root)?;
    let input_directory = workspace.path().join("input");
    let output_directory = workspace.path().join("output");
    let probe_directory = workspace.path().join("probe");
    tokio::fs::create_dir(&input_directory).await?;
    tokio::fs::create_dir(&output_directory).await?;
    tokio::fs::create_dir(&probe_directory).await?;

    ensure_available_space(
        &staging_root,
        validated.input_bytes.saturating_mul(3),
        state.config().uploads.free_space_margin_bytes,
    )
    .await?;
    workspace_phase.complete();

    let input_phase = GogImportPhaseLog::start(reporter, GogImportPhase::StageSetupFiles);
    let setup_path = copy_input_files(
        &draft.files,
        &input_directory,
        &state.config().default_library_root,
    )
    .await?;
    input_phase.complete();

    let setup_directory = setup_path.parent().unwrap_or(setup_path.as_path());
    let private_paths = [workspace.path(), setup_directory, setup_path.as_path()];
    let quick_timeout =
        Duration::from_secs(config.timeout_seconds.clamp(1, QUICK_PROBE_TIMEOUT_SECONDS));

    let version_phase = GogImportPhaseLog::start(reporter, GogImportPhase::ProbeInnoextractVersion);
    let extractor_version = extractor_version(
        config,
        &probe_directory,
        quick_timeout,
        reporter,
        &private_paths,
    )
    .await?;
    version_phase.complete();
    tracing::info!(
        phase = "probe_innoextract_version",
        extractor_version = %extractor_version,
        "GOG import extractor capability recorded"
    );

    let data_version_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::ProbeInstallerDataVersion);
    let data_version = data_version(
        config,
        &probe_directory,
        &setup_path,
        quick_timeout,
        reporter,
        &private_paths,
    )
    .await?;
    data_version_phase.complete();
    tracing::info!(
        phase = "probe_installer_data_version",
        data_version = %data_version,
        "GOG installer data version recorded"
    );

    let test_phase = GogImportPhaseLog::start(reporter, GogImportPhase::TestInstallerIntegrity);
    test_setup(
        config,
        &probe_directory,
        &setup_path,
        reporter,
        &private_paths,
    )
    .await?;
    test_phase.complete();

    let extraction_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::ExtractInstallerPayload);
    extract_setup(
        config,
        workspace.path(),
        &setup_path,
        &output_directory,
        reporter,
        &private_paths,
    )
    .await?;
    extraction_phase.complete();

    let archive_name = archive_file_name(&title);
    let archive_path = workspace.path().join(&archive_name);
    let output = output_directory.clone();
    let archive = archive_path.clone();
    let max_entries = config.max_extracted_files;
    let max_bytes = config.max_extracted_bytes;
    let free_space_margin_bytes = state.config().uploads.free_space_margin_bytes;
    let archive_span = tracing::Span::current();
    let archive_dispatch = tracing::dispatcher::get_default(Clone::clone);
    let archive_reporter = reporter.clone();
    let mut summary = tokio::task::spawn_blocking(move || {
        tracing::dispatcher::with_default(&archive_dispatch, || {
            archive_span.in_scope(|| {
                package_extracted_tree(
                    &output,
                    &archive,
                    max_entries,
                    max_bytes,
                    free_space_margin_bytes,
                    &archive_reporter,
                )
            })
        })
    })
    .await
    .map_err(|_| GogImportError::WorkerFailed)??;
    tracing::info!(
        phase = GogImportPhase::ValidateSiparioZip.as_str(),
        archive_bytes = summary.archive_bytes,
        browser_upload_limit_bytes = state.config().max_upload_bytes,
        browser_upload_limit_applies = false,
        "Validated generated GOG ZIP for managed import"
    );

    summary.input_file_count = draft.files.len();
    summary.input_bytes = validated.input_bytes;
    summary.extractor_version = extractor_version;
    summary.data_version = data_version;

    let uploaded = publish_archive(
        state,
        &title,
        &archive_name,
        &archive_path,
        summary.archive_bytes,
        reporter,
    )
    .await?;
    tracing::info!(
        phase = GogImportPhase::PublishWindowsArchive.as_str(),
        rom_id = uploaded.rom.id,
        target_platform = WINDOWS_PLATFORM_SLUG,
        "Imported GOG ZIP as Windows"
    );

    Ok(GogImportOutcome { uploaded, summary })
}

#[derive(Debug)]
struct ValidatedDraft {
    input_bytes: u64,
}

fn validate_draft(
    config: &AppConfig,
    draft: &GogImportDraft,
) -> Result<ValidatedDraft, GogImportError> {
    if draft.files.is_empty() {
        return Err(GogImportError::EmptyInput);
    }
    if draft.files.len() > config.uploads.max_batch_files {
        return Err(GogImportError::InputTooLarge);
    }

    let mut names = BTreeSet::new();
    let mut executable_count = 0_usize;
    let mut input_bytes = 0_u64;
    for file in &draft.files {
        validate_setup_file_name(&file.original_file_name)?;
        if !names.insert(file.original_file_name.to_lowercase()) {
            return Err(GogImportError::DuplicateSetupFileName);
        }
        if Path::new(&file.original_file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            executable_count += 1;
        }
        if file.file_size_bytes > config.max_upload_bytes {
            return Err(GogImportError::InputTooLarge);
        }
        input_bytes = input_bytes
            .checked_add(file.file_size_bytes)
            .ok_or(GogImportError::InputTooLarge)?;
    }
    if executable_count != 1 {
        return Err(GogImportError::InvalidSetupExecutableCount);
    }
    if input_bytes > config.uploads.max_batch_bytes {
        return Err(GogImportError::InputTooLarge);
    }
    Ok(ValidatedDraft { input_bytes })
}

async fn ensure_windows_target(state: &AppState) -> Result<(), GogImportError> {
    let platform = platforms::find_by_slug(state.db(), WINDOWS_PLATFORM_SLUG)
        .await?
        .ok_or(GogImportError::WindowsPlatformUnavailable)?;
    if platform.slug != WINDOWS_PLATFORM_SLUG || platform.fs_slug != WINDOWS_PLATFORM_SLUG {
        return Err(GogImportError::WindowsPlatformUnavailable);
    }
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;
    if !root.writable {
        return Err(GogImportError::ReadOnlyRoot);
    }
    Ok(())
}

async fn copy_input_files(
    files: &[GogImportFileDraft],
    input_directory: &Path,
    library_root: &Path,
) -> Result<PathBuf, GogImportError> {
    let root = library_root.canonicalize()?;
    let uploads_directory = root.join(".uploads");
    let uploads_metadata = tokio::fs::symlink_metadata(&uploads_directory).await?;
    if uploads_metadata.file_type().is_symlink() || !uploads_metadata.is_dir() {
        return Err(GogImportError::UnsafeExtractedOutput);
    }

    let mut setup_path = None;
    for file in files {
        if file.staged_path.parent() != Some(uploads_directory.as_path()) {
            return Err(GogImportError::UnsafeExtractedOutput);
        }
        let metadata = tokio::fs::symlink_metadata(&file.staged_path).await?;
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != file.file_size_bytes
        {
            return Err(GogImportError::UnsafeExtractedOutput);
        }

        let target = input_directory.join(&file.original_file_name);
        let mut source = tokio::fs::File::open(&file.staged_path).await?;
        let mut destination = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .await?;
        let copied = tokio::io::copy(&mut source, &mut destination).await?;
        if copied != file.file_size_bytes {
            return Err(GogImportError::ExtractedOutputChanged);
        }
        destination.flush().await?;
        destination.sync_all().await?;

        if Path::new(&file.original_file_name)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        {
            setup_path = Some(target);
        }
    }

    setup_path.ok_or(GogImportError::InvalidSetupExecutableCount)
}

async fn extractor_version(
    config: &GogImportConfig,
    working_directory: &Path,
    phase_timeout: Duration,
    reporter: &GogImportWorkflowReporter,
    private_paths: &[&Path],
) -> Result<String, GogImportError> {
    let arguments = vec![OsString::from("--color=false"), OsString::from("--version")];
    let output = run_innoextract(
        config,
        working_directory,
        &arguments,
        "version probe",
        phase_timeout,
        ProcessReport::new(
            reporter,
            GogImportPhase::ProbeInnoextractVersion,
            private_paths,
        ),
    )
    .await?;
    assess_process(&output, "version probe", false)?;
    first_clean_line(&output.stdout).ok_or(GogImportError::ExtractorUnavailable)
}

async fn data_version(
    config: &GogImportConfig,
    working_directory: &Path,
    setup_path: &Path,
    phase_timeout: Duration,
    reporter: &GogImportWorkflowReporter,
    private_paths: &[&Path],
) -> Result<String, GogImportError> {
    let arguments = vec![
        OsString::from("--silent"),
        OsString::from("--color=false"),
        OsString::from("--data-version"),
        OsString::from("--"),
        setup_path.as_os_str().to_owned(),
    ];
    let output = run_innoextract(
        config,
        working_directory,
        &arguments,
        "data-version probe",
        phase_timeout,
        ProcessReport::new(
            reporter,
            GogImportPhase::ProbeInstallerDataVersion,
            private_paths,
        ),
    )
    .await?;
    assess_process(&output, "data-version probe", true)?;
    first_clean_line(&output.stdout).ok_or(GogImportError::MissingDataVersion)
}

async fn test_setup(
    config: &GogImportConfig,
    working_directory: &Path,
    setup_path: &Path,
    reporter: &GogImportWorkflowReporter,
    private_paths: &[&Path],
) -> Result<(), GogImportError> {
    let arguments = vec![
        OsString::from("--silent"),
        OsString::from("--color=false"),
        OsString::from("--progress=true"),
        OsString::from("--test"),
        OsString::from("--no-extract-unknown"),
        OsString::from("--collisions=error"),
        OsString::from("--gog"),
        OsString::from("--"),
        setup_path.as_os_str().to_owned(),
    ];
    let output = run_innoextract(
        config,
        working_directory,
        &arguments,
        "integrity test",
        Duration::from_secs(config.timeout_seconds),
        ProcessReport::with_progress(
            reporter,
            GogImportPhase::TestInstallerIntegrity,
            private_paths,
        ),
    )
    .await?;
    assess_process(&output, "integrity test", true)
}

async fn extract_setup(
    config: &GogImportConfig,
    working_directory: &Path,
    setup_path: &Path,
    output_directory: &Path,
    reporter: &GogImportWorkflowReporter,
    private_paths: &[&Path],
) -> Result<(), GogImportError> {
    let arguments = vec![
        OsString::from("--silent"),
        OsString::from("--color=false"),
        OsString::from("--progress=true"),
        OsString::from("--extract"),
        OsString::from("--no-extract-unknown"),
        OsString::from("--collisions=error"),
        OsString::from("--timestamps=none"),
        OsString::from("--gog"),
        OsString::from("--output-dir"),
        output_directory.as_os_str().to_owned(),
        OsString::from("--"),
        setup_path.as_os_str().to_owned(),
    ];
    let output = run_innoextract(
        config,
        working_directory,
        &arguments,
        "extraction",
        Duration::from_secs(config.timeout_seconds),
        ProcessReport::with_progress(
            reporter,
            GogImportPhase::ExtractInstallerPayload,
            private_paths,
        ),
    )
    .await?;
    assess_process(&output, "extraction", true)
}

fn assess_process(
    output: &ProcessOutput,
    phase: &'static str,
    reject_diagnostics: bool,
) -> Result<(), GogImportError> {
    if !output.status.success() {
        return Err(GogImportError::ExtractorRejected { phase });
    }
    if output.truncated || (reject_diagnostics && output.suspicious_diagnostics) {
        return Err(GogImportError::ExtractorDiagnostics { phase });
    }
    Ok(())
}

fn first_clean_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| {
            line.chars()
                .filter(|character| !character.is_control())
                .take(256)
                .collect::<String>()
        })
        .filter(|line| !line.is_empty())
}

async fn copy_generated_archive_with_progress<F, W>(
    archive_path: &Path,
    staged_file: &mut W,
    expected_bytes: u64,
    report: &mut F,
) -> Result<(), GogImportError>
where
    F: FnMut(u64),
    W: AsyncWrite + Unpin,
{
    let mut archive = tokio::fs::File::open(archive_path).await?;
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut copied = 0_u64;
    loop {
        let read = archive.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        staged_file.write_all(&buffer[..read]).await?;
        copied = copied
            .checked_add(u64::try_from(read).map_err(|_| GogImportError::ExtractedOutputChanged)?)
            .ok_or(GogImportError::ExtractedOutputChanged)?;
        report(copied);
    }
    if copied != expected_bytes {
        return Err(GogImportError::ExtractedOutputChanged);
    }
    Ok(())
}

async fn publish_archive(
    state: &AppState,
    title: &str,
    archive_name: &str,
    archive_path: &Path,
    archive_bytes: u64,
    reporter: &GogImportWorkflowReporter,
) -> Result<UploadedRom, GogImportError> {
    let staging_phase = GogImportPhaseLog::start(reporter, GogImportPhase::StageGeneratedArchive);
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;
    ensure_available_space(
        &root_path,
        archive_bytes,
        state.config().uploads.free_space_margin_bytes,
    )
    .await?;

    let operation_id = file_operations::new_operation_id();
    let staging_relative_path = format!(".uploads/upload-{operation_id}.part");
    let payload = UploadOperationPayload::new(root.id, Vec::new())
        .with_staged_paths(vec![staging_relative_path.clone()]);
    file_operations::prepare(state, &operation_id, FileOperationKind::Upload, &payload)
        .await
        .map_err(|_| GogImportError::ArchiveStaging)?;

    let staged = state
        .file_store()
        .create_staging_file(&root_path, &staging_relative_path)
        .await;
    let (staged_path, mut staged_file) = match staged {
        Ok(staged) => staged,
        Err(error) => {
            complete_archive_staging(state, &operation_id).await;
            return Err(LibraryServiceError::from(error).into());
        }
    };

    let copy_result = async {
        let mut progress = GogImportByteProgress::new(archive_bytes, |current, total| {
            reporter.report_bytes(GogImportPhase::StageGeneratedArchive, current, total);
        });
        progress.start();
        {
            let mut report_copied_bytes = |current| progress.update(current);
            copy_generated_archive_with_progress(
                archive_path,
                &mut staged_file,
                archive_bytes,
                &mut report_copied_bytes,
            )
            .await?;
        }
        progress.finish(archive_bytes);
        staged_file.flush().await?;
        staged_file.sync_all().await?;
        Ok::<_, GogImportError>(())
    }
    .await;
    drop(staged_file);
    if let Err(error) = copy_result {
        state.file_store().cleanup_path(&staged_path).await;
        complete_archive_staging(state, &operation_id).await;
        return Err(error);
    }
    staging_phase.complete();

    let publish_phase = GogImportPhaseLog::start(reporter, GogImportPhase::PublishWindowsArchive);
    let draft = UploadDraft {
        platform_id: None,
        platform_slug: Some(WINDOWS_PLATFORM_SLUG.to_string()),
        title: Some(title.to_string()),
        original_file_name: archive_name.to_string(),
        staged_path: staged_path.clone(),
        staging_operation_id: operation_id.clone(),
        file_size_bytes: archive_bytes,
    };
    let result = library::finalize_upload(state, draft).await;
    if result.is_err() {
        state.file_store().cleanup_path(&staged_path).await;
    }
    complete_archive_staging(state, &operation_id).await;
    let uploaded = result?;
    publish_phase.complete();
    Ok(uploaded)
}

async fn complete_archive_staging(state: &AppState, operation_id: &str) {
    if let Err(error) = file_operations::complete(state, operation_id).await {
        tracing::warn!(
            ?error,
            operation_id,
            "failed to complete generated GOG archive staging journal"
        );
    }
}

async fn ensure_available_space(
    path: &Path,
    required_bytes: u64,
    margin_bytes: u64,
) -> Result<(), GogImportError> {
    let path = path.to_path_buf();
    let available = tokio::task::spawn_blocking(move || fs2::available_space(path))
        .await
        .map_err(|_| GogImportError::WorkerFailed)??;
    if available.saturating_sub(margin_bytes) < required_bytes {
        return Err(GogImportError::InsufficientStorage);
    }
    Ok(())
}

fn normalize_title(title: &str) -> Result<String, GogImportError> {
    let title = title.trim();
    if title.is_empty()
        || title.len() > MAX_GAME_TITLE_BYTES
        || title
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(GogImportError::InvalidTitle);
    }
    Ok(title.to_string())
}

fn archive_file_name(title: &str) -> String {
    let mut base = String::new();
    let mut previous_separator = false;
    for character in title.trim().chars() {
        let unsafe_character = character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            );
        let character = if unsafe_character { '_' } else { character };
        let separator = character == '_' || character.is_whitespace();
        if separator {
            if !previous_separator {
                base.push(' ');
            }
        } else {
            base.push(character);
        }
        previous_separator = separator;
        if base.len() >= 180 {
            break;
        }
    }
    let mut base = base
        .trim_matches(|character| matches!(character, ' ' | '.'))
        .to_string();
    if base.is_empty() {
        base = "Game".to_string();
    }
    let uppercase = base.to_ascii_uppercase();
    let reserved = matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || uppercase
            .strip_prefix("COM")
            .or_else(|| uppercase.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
    if reserved {
        base.push_str(" Game");
    }
    format!("{base}.zip")
}

pub(crate) async fn cleanup_abandoned_workspaces(config: &AppConfig) -> Result<(), GogImportError> {
    let root = config.gog_import.staging_root(&config.data_dir);
    let metadata = match tokio::fs::symlink_metadata(&root).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(GogImportError::UnsafeExtractedOutput);
    }
    let mut entries = tokio::fs::read_dir(&root).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if metadata.file_type().is_symlink() || metadata.is_file() {
            tokio::fs::remove_file(path).await?;
        } else if metadata.is_dir() {
            tokio::fs::remove_dir_all(path).await?;
        } else {
            return Err(GogImportError::UnsafeExtractedOutput);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
    };

    use tokio::io::{AsyncWrite, AsyncWriteExt};

    use super::{
        GogImportError, GogImportJobRegistry, GogImportPhase, GogImportPhaseLog,
        GogImportWorkflowReporter, archive_file_name, copy_generated_archive_with_progress,
        normalize_title, validate_setup_file_name,
    };

    #[test]
    fn setup_names_and_titles_are_strictly_validated() {
        assert!(validate_setup_file_name("setup_game.exe").is_ok());
        assert!(validate_setup_file_name("setup_game-1.bin").is_ok());
        assert!(matches!(
            validate_setup_file_name("../setup.exe"),
            Err(GogImportError::InvalidSetupFileName)
        ));
        assert!(validate_setup_file_name("readme.txt").is_err());
        assert_eq!(normalize_title("  My Game  ").unwrap(), "My Game");
        assert!(normalize_title("\n").is_err());
    }

    #[test]
    fn archive_names_preserve_the_title_without_windows_unsafe_characters() {
        assert_eq!(archive_file_name("My Game"), "My Game.zip");
        assert_eq!(
            archive_file_name("Game: Deluxe / Edition"),
            "Game Deluxe Edition.zip"
        );
        assert_eq!(archive_file_name("CON"), "CON Game.zip");
    }

    struct DiskFullWriter;

    impl AsyncWrite for DiskFullWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            _buffer: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "synthetic disk full",
            )))
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn generated_archive_copy_reports_bytes_and_rejects_size_or_disk_failures() {
        let temp = tempfile::TempDir::new().unwrap();
        let source = temp.path().join("source.zip");
        let destination = temp.path().join("destination.part");
        let payload = vec![0x5a; 192 * 1024];
        tokio::fs::write(&source, &payload).await.unwrap();
        let mut destination_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .await
            .unwrap();
        let mut updates = Vec::new();
        copy_generated_archive_with_progress(
            &source,
            &mut destination_file,
            payload.len() as u64,
            &mut |current| updates.push(current),
        )
        .await
        .unwrap();
        destination_file.flush().await.unwrap();
        drop(destination_file);

        assert_eq!(tokio::fs::read(destination).await.unwrap(), payload);
        assert_eq!(updates.last().copied(), Some(payload.len() as u64));
        assert!(updates.windows(2).all(|window| window[0] < window[1]));

        let short_destination = temp.path().join("short.part");
        let mut short_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(short_destination)
            .await
            .unwrap();
        let error = copy_generated_archive_with_progress(
            &source,
            &mut short_file,
            payload.len() as u64 + 1,
            &mut |_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(error, GogImportError::ExtractedOutputChanged));

        let error = copy_generated_archive_with_progress(
            &source,
            &mut DiskFullWriter,
            payload.len() as u64,
            &mut |_| {},
        )
        .await
        .unwrap_err();
        assert!(matches!(error, GogImportError::Io(_)));
    }

    #[test]
    fn dropping_an_older_phase_guard_cannot_overwrite_a_newer_phase() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id;
        reservation.reporter.start().unwrap();
        let reporter = GogImportWorkflowReporter::new(reservation.reporter);

        let older = GogImportPhaseLog::start(&reporter, GogImportPhase::ValidateRequest);
        let newer = GogImportPhaseLog::start(&reporter, GogImportPhase::ResolveWindowsTarget);
        newer.complete();
        drop(older);

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.phase, GogImportPhase::ResolveWindowsTarget);
        assert_eq!(
            snapshot
                .events
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                GogImportPhase::ValidateRequest,
                GogImportPhase::ResolveWindowsTarget
            ]
        );
    }
}
