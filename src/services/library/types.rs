use std::{io, path::PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{
    domain::{
        ingest::{ParsedFilename, PlanFileKey},
        platform::Platform,
        rom::{DependencyKind, FileGroupKind, FileRole, Rom},
    },
    repositories::roms,
    services::ingest::manifests,
    storage::{file_store::FileStoreError, paths::PathSafetyError},
};

#[derive(Debug, Clone)]
pub struct UploadDraft {
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub title: Option<String>,
    pub original_file_name: String,
    pub staged_path: PathBuf,
    pub staging_operation_id: String,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct UploadPreviewDraft {
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub title: Option<String>,
    pub files: Vec<UploadPreviewFileDraft>,
}

#[derive(Debug, Clone)]
pub struct UploadPreviewFileDraft {
    pub original_file_name: String,
    pub file_size_bytes: Option<u64>,
    pub manifest_contents: Option<String>,
}

#[derive(Debug, Clone)]
pub struct UploadBatchDraft {
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub title: Option<String>,
    pub planned_titles: Vec<PlannedRomTitle>,
    pub files: Vec<UploadBatchFileDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedRomTitle {
    pub plan_id: String,
    pub title: String,
}

#[derive(Debug, Clone)]
pub struct UploadBatchFileDraft {
    pub original_file_name: String,
    pub staged_path: PathBuf,
    pub staging_operation_id: String,
    pub file_size_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct UploadedRom {
    pub rom: Rom,
    pub file_id: i64,
    pub file_name: String,
    pub relative_path: String,
    pub file_size_bytes: i64,
}

#[derive(Debug, Clone)]
pub struct UploadedBatch {
    pub roms: Vec<Rom>,
    pub warnings: Vec<IngestPlanWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryScanResult {
    pub scanned_file_count: usize,
    pub already_indexed_file_count: usize,
    pub imported_rom_count: usize,
    pub imported_file_count: usize,
    pub generated_manifest_count: usize,
    pub cover_downloaded_count: usize,
    pub cover_not_downloaded_count: usize,
    pub cover_warnings: Vec<LibraryScanCoverWarning>,
    pub not_imported_file_count: usize,
    pub unimported_file_count: usize,
    pub imported_roms: Vec<ImportedScannedRom>,
    pub imported_roms_truncated: bool,
    pub unimported_files: Vec<UnimportedScanFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedScannedRom {
    pub id: i64,
    pub name: String,
    pub platform_slug: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryScanCoverWarning {
    pub rom_id: i64,
    pub rom_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UnimportedScanFile {
    pub relative_path: String,
    pub disposition: String,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct SidecarCleanupFile {
    pub relative_path: String,
    pub file_size_bytes: u64,
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SidecarCleanupPreview {
    pub file_count: usize,
    pub total_file_bytes: u64,
    pub truncated: bool,
    pub files: Vec<SidecarCleanupFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SidecarCleanupResult {
    pub deleted_file_count: usize,
    pub deleted_file_bytes: u64,
    pub deleted_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct DeleteRomOutcome {
    pub rom: Rom,
    pub delete_files: bool,
    pub deleted_files: Vec<String>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct BulkDeleteRomsOutcome {
    pub scope: BulkDeleteScope,
    pub deleted_roms: Vec<Rom>,
    pub delete_files: bool,
    pub deleted_files: Vec<String>,
    pub missing_files: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum BulkDeleteScope {
    Platform(Platform),
    All,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateRomDraft {
    pub name: Option<String>,
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub summary: Option<Option<String>>,
    pub regions: Option<Vec<String>>,
    pub genres: Option<Vec<String>>,
    pub developers: Option<Vec<String>>,
    pub publishers: Option<Vec<String>>,
    pub release_year: Option<Option<i32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlan {
    pub platform_id: i64,
    pub platform_slug: String,
    pub roms: Vec<IngestPlanRom>,
    pub warnings: Vec<IngestPlanWarning>,
    pub errors: Vec<IngestPlanError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanRom {
    pub plan_id: String,
    pub title: String,
    pub slug: String,
    pub regions: Vec<String>,
    pub groups: Vec<IngestPlanGroup>,
    pub files: Vec<IngestPlanFile>,
    pub dependencies: Vec<IngestPlanDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanGroup {
    pub key: String,
    pub kind: FileGroupKind,
    pub display_name: String,
    pub group_key: Option<String>,
    pub disc_index: Option<i64>,
    pub disc_count: Option<i64>,
    pub launchable: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanFile {
    pub key: PlanFileKey,
    pub group_key: Option<String>,
    pub original_file_name: String,
    pub file_size_bytes: Option<u64>,
    pub role: FileRole,
    pub sort_index: i64,
    pub disc_index: Option<i64>,
    pub track_index: Option<i64>,
    pub launchable: bool,
    pub metadata: Value,
    pub parsed_filename: ParsedFilename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanDependency {
    pub parent_file_key: PlanFileKey,
    pub child_file_key: PlanFileKey,
    pub dependency_kind: DependencyKind,
    pub sort_index: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanWarning {
    pub code: String,
    pub message: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestPlanError {
    pub code: String,
    pub message: String,
    pub file_name: Option<String>,
}

#[derive(Debug, Error)]
pub enum LibraryServiceError {
    #[error("platform_id or platform_slug is required")]
    MissingPlatform,

    #[error("platform not found")]
    PlatformNotFound,

    #[error("platform_id and platform_slug refer to different platforms")]
    PlatformMismatch,

    #[error("default library root is not registered")]
    DefaultRootMissing,

    #[error("default library root is read-only")]
    ReadOnlyRoot,

    #[error("upload filename is invalid")]
    InvalidFileName,

    #[error("upload title is invalid")]
    InvalidTitle,

    #[error("metadata field is invalid")]
    InvalidMetadataField,

    #[error("upload is too large")]
    UploadTooLarge,

    #[error("split archive uploads are not supported yet")]
    UnsupportedSplitArchive,

    #[error("batch upload contains no files")]
    EmptyBatch,

    #[error("batch upload plan has validation errors: {0}")]
    InvalidIngestPlan(String),

    #[error("could not find an available filename")]
    NoAvailableFileName,

    #[error("sidecar cleanup requires at least one previewed file")]
    EmptySidecarCleanup,

    #[error("sidecar cleanup preview exceeds the file limit")]
    SidecarCleanupTooLarge,

    #[error("sidecar cleanup preview is stale; preview again before deleting")]
    SidecarCleanupPreviewChanged,

    #[error("refusing to delete a library root")]
    RootDeleteRejected,

    #[error("delete target is not a regular file")]
    UnsafeDeleteTarget,

    #[error("ROM not found")]
    RomNotFound,

    #[error("managed file recovery failed: {0}")]
    FileRecovery(String),

    #[error(transparent)]
    PathSafety(#[from] PathSafetyError),

    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub(super) struct PlanInputFile {
    pub(super) index: usize,
    pub(super) original_file_name: String,
    pub(super) file_size_bytes: Option<u64>,
    pub(super) staged_path: Option<PathBuf>,
    pub(super) manifest_contents: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct PreparedPlanFile {
    pub(super) input: PlanInputFile,
    pub(super) parsed: ParsedFilename,
    pub(super) extension: String,
    pub(super) manifest: ManifestReferences,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ManifestReferences {
    pub(super) m3u: Option<Vec<String>>,
    pub(super) cue: Option<Vec<String>>,
    pub(super) gdi: Option<Vec<manifests::GdiTrackReference>>,
}

#[derive(Debug, Clone)]
pub(super) struct InPlaceSourceFile {
    pub(super) index: usize,
    pub(super) relative_path: String,
    pub(super) file_name: String,
    pub(super) file_size_bytes: u64,
}

#[derive(Debug)]
pub(super) struct BatchFinalization {
    pub(super) roms: Vec<roms::CreateGroupedRomParams>,
    pub(super) operations: Vec<BatchFileOperation>,
}

#[derive(Debug)]
pub(super) enum BatchFileOperation {
    Move {
        staged_path: PathBuf,
        relative_path: String,
    },
    WriteGenerated {
        relative_path: String,
        contents: String,
    },
}
