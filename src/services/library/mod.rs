//! ROM library planning and recoverable mutation workflows.
//!
//! Ingest planning is pure: it does not access HTTP, SQL, or managed files. Finalization
//! validates every group/file relationship, acquires the root lock, journals relative paths,
//! performs no-overwrite file operations, and only then persists the ROM graph.

mod delete;
mod download;
mod edit;
mod ingest;
mod normalization;
mod scan;
mod scan_jobs;
mod types;
mod upload;

pub use delete::{delete_all_roms, delete_rom, delete_roms_for_platform};
pub(crate) use download::{
    DOWNLOAD_TICKET_TTL_SECONDS, DownloadArchiveError, DownloadTicketError, DownloadTicketRegistry,
    PreparedDownloadArchive, prepare_download_archive,
};
pub use edit::{stats, update_rom};
pub(crate) use normalization::{normalized_title, sanitize_upload_file_name, slugify};
pub(crate) use scan::{
    cleanup_sidecars, preview_sidecar_cleanup, scan_failure_summary, scan_library,
    scan_reason_codes,
};
pub(crate) use scan_jobs::{
    LibraryScanJobError, LibraryScanJobProgress, LibraryScanJobRegistry,
    LibraryScanJobRegistryError, LibraryScanJobReporter, LibraryScanJobSnapshot,
};
pub use types::{
    BulkDeleteRomsOutcome, BulkDeleteScope, DeleteRomOutcome, ImportedScannedRom, IngestPlan,
    IngestPlanDependency, IngestPlanError, IngestPlanFile, IngestPlanGroup, IngestPlanRom,
    IngestPlanWarning, LibraryScanCoverWarning, LibraryScanResult, LibraryServiceError,
    PlannedRomTitle, SidecarCleanupFile, SidecarCleanupPreview, SidecarCleanupResult,
    UnimportedScanFile, UpdateRomDraft, UploadBatchDraft, UploadBatchFileDraft, UploadDraft,
    UploadPreviewDraft, UploadPreviewFileDraft, UploadedBatch, UploadedRom,
};
pub use upload::{finalize_upload, finalize_upload_batch, preview_upload_batch};
