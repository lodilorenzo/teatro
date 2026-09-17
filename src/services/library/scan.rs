//! In-place discovery and ingest of files already under the managed library root.

use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use thiserror::Error;

use crate::{
    domain::{
        platform::Platform,
        workflow::{FileOperationKind, FileOperationState},
    },
    repositories::{file_operations as file_operation_repository, library_roots, platforms, roms},
    services::{
        file_operations::{self, UploadOperationPayload},
        igdb::{AutomaticMetadataOutcome, IgdbServiceError},
        ingest::filename,
    },
    state::AppState,
    storage::file_store::FileStoreError,
};

use super::super::bounded_text;
use super::{
    edit::MAX_MANIFEST_BYTES,
    ingest::{build_ingest_plan, prepare_in_place_finalization, summarize_ingest_errors},
    normalization::{is_manifest_file_name, join_relative_path, m3u_directory_name},
    scan_jobs::{LibraryScanJobError, LibraryScanJobReporter},
    types::{
        BatchFileOperation, ImportedScannedRom, InPlaceSourceFile, LibraryScanCoverWarning,
        LibraryScanResult, LibraryServiceError, PlanInputFile, SidecarCleanupFile,
        SidecarCleanupPreview, SidecarCleanupResult, UnimportedScanFile,
    },
};

const MAX_DISPLAY_PATH_BYTES: usize = 4 * 1024;
const MAX_REASON_BYTES: usize = 4 * 1024;
const IMPORTED_ROM_SUMMARY_LIMIT: usize = 100;
pub(crate) const SIDECAR_CLEANUP_FILE_LIMIT: usize = 256;

#[derive(Debug, Error)]
pub(crate) enum LibraryScanError {
    #[error(transparent)]
    Library(#[from] LibraryServiceError),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    FileStore(#[from] FileStoreError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub(crate) async fn scan_library(
    state: &AppState,
    reporter: &LibraryScanJobReporter,
) -> Result<LibraryScanResult, LibraryScanError> {
    publish_phase(reporter, "discovering", 1);

    let (root_id, root_path) = writable_default_root(state).await?;

    // The lock is intentionally acquired by the worker, never by the create request.
    let root_lock = state.file_store().lock_root(&root_path).await?;
    let indexed_paths = roms::indexed_relative_paths(state.db(), root_id).await?;
    let platform_list = platforms::list_with_rom_counts(state.db()).await?;
    let mut discovery = discover(&root_path, platform_list, &indexed_paths).await?;

    publish_phase(reporter, "planning", 2);
    publish_progress(reporter, 0, discovery.batches.len());
    let mut planned = Vec::new();
    let planning_total = discovery.batches.len();
    for (index, batch) in discovery.batches.into_iter().enumerate() {
        plan_batch(
            state,
            &root_path,
            batch,
            &indexed_paths,
            &mut discovery.unimported,
            &mut planned,
        )
        .await;
        publish_progress(reporter, index + 1, planning_total);
        tokio::task::yield_now().await;
    }

    publish_phase(reporter, "importing", 3);
    publish_progress(reporter, 0, planned.len());
    let importing_total = planned.len();
    let mut imported_roms = Vec::new();
    let mut imported_file_count = 0_usize;
    let mut generated_manifest_count = 0_usize;

    for (index, planned_batch) in planned.into_iter().enumerate() {
        if let Some((code, reason)) =
            revalidate_batch(state, &root_path, root_id, &planned_batch.batch).await?
        {
            reject_batch(
                &planned_batch.batch,
                &code,
                &reason,
                &mut discovery.unimported,
            );
        } else {
            match import_batch(state, &root_path, root_id, &planned_batch).await {
                Ok(imported) => {
                    imported_file_count += planned_batch.batch.files.len();
                    generated_manifest_count += imported.generated_manifest_count;
                    imported_roms.push(imported.summary);
                }
                Err(LibraryScanError::Library(LibraryServiceError::NoAvailableFileName)) => {
                    reject_batch(
                        &planned_batch.batch,
                        "game_folder_conflict",
                        "The required playlist or matching game-folder path is already in use.",
                        &mut discovery.unimported,
                    );
                }
                Err(error) => return Err(error),
            }
        }
        publish_progress(reporter, index + 1, importing_total);
        tokio::task::yield_now().await;
    }

    drop(root_lock);
    let (cover_downloaded_count, cover_warnings) =
        fetch_imported_rom_covers(state, reporter, &imported_roms).await;
    let cover_not_downloaded_count = cover_warnings.len();

    discovery.unimported.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.disposition.cmp(&right.disposition))
    });
    let already_indexed_file_count = discovery
        .unimported
        .iter()
        .filter(|file| file.disposition == "already_indexed")
        .count();
    let not_imported_file_count = discovery
        .unimported
        .iter()
        .filter(|file| file.disposition == "not_imported")
        .count();
    let imported_rom_count = imported_roms.len();
    debug_assert_eq!(
        imported_rom_count,
        cover_downloaded_count + cover_not_downloaded_count,
        "every imported ROM must receive one cover outcome"
    );
    let imported_roms_truncated = imported_rom_count > IMPORTED_ROM_SUMMARY_LIMIT;
    imported_roms.truncate(IMPORTED_ROM_SUMMARY_LIMIT);
    let unimported_file_count = discovery.unimported.len();

    debug_assert_eq!(
        discovery.scanned_file_count,
        imported_file_count + unimported_file_count,
        "every discovered file must receive one disposition"
    );

    Ok(LibraryScanResult {
        scanned_file_count: discovery.scanned_file_count,
        already_indexed_file_count,
        imported_rom_count,
        imported_file_count,
        generated_manifest_count,
        cover_downloaded_count,
        cover_not_downloaded_count,
        cover_warnings,
        not_imported_file_count,
        unimported_file_count,
        imported_roms,
        imported_roms_truncated,
        unimported_files: discovery.unimported,
    })
}

async fn fetch_imported_rom_covers(
    state: &AppState,
    reporter: &LibraryScanJobReporter,
    imported_roms: &[ImportedScannedRom],
) -> (usize, Vec<LibraryScanCoverWarning>) {
    if imported_roms.is_empty() {
        return (0, Vec::new());
    }

    publish_phase(reporter, "fetching_covers", 4);
    publish_progress(reporter, 0, imported_roms.len());
    let unavailable_reason = match state.igdb_config().await {
        Ok(config) if config.is_configured() => None,
        Ok(_) => Some("IGDB credentials are not configured."),
        Err(error) => {
            tracing::warn!(
                ?error,
                "IGDB configuration could not be loaded after library scan"
            );
            Some("The IGDB configuration could not be loaded.")
        }
    };
    if let Some(reason) = unavailable_reason {
        publish_progress(reporter, imported_roms.len(), imported_roms.len());
        return (
            0,
            imported_roms
                .iter()
                .map(|rom| cover_warning(rom, reason))
                .collect(),
        );
    }

    let mut downloaded = 0;
    let mut warnings = Vec::new();
    for (index, rom) in imported_roms.iter().enumerate() {
        match state
            .igdb_client()
            .auto_apply_metadata(state, rom.id, &rom.name)
            .await
        {
            Ok(AutomaticMetadataOutcome::Applied(applied)) if !applied.cached_covers.is_empty() => {
                downloaded += 1
            }
            Ok(AutomaticMetadataOutcome::Applied(_)) => warnings.push(cover_warning(
                rom,
                "The selected IGDB match has no downloadable cover.",
            )),
            Ok(AutomaticMetadataOutcome::NoMatch) => warnings.push(cover_warning(
                rom,
                "No IGDB game was found on the selected or any platform.",
            )),
            Err(IgdbServiceError::NotConfigured) => {
                warnings.push(cover_warning(rom, "IGDB credentials are not configured."))
            }
            Err(error) => {
                tracing::warn!(
                    ?error,
                    rom_id = rom.id,
                    "automatic IGDB cover download failed after library scan"
                );
                warnings.push(cover_warning(
                    rom,
                    "The IGDB lookup or cover download failed.",
                ));
            }
        }
        publish_progress(reporter, index + 1, imported_roms.len());
        tokio::task::yield_now().await;
    }
    (downloaded, warnings)
}

fn cover_warning(rom: &ImportedScannedRom, reason: &str) -> LibraryScanCoverWarning {
    LibraryScanCoverWarning {
        rom_id: rom.id,
        rom_name: bounded_text(&rom.name, MAX_DISPLAY_PATH_BYTES).0,
        reason: bounded_text(reason, MAX_REASON_BYTES).0,
    }
}

pub(crate) fn scan_failure_summary(error: &LibraryScanError) -> LibraryScanJobError {
    let code = match error {
        LibraryScanError::Library(LibraryServiceError::DefaultRootMissing) => {
            "library_root_missing"
        }
        LibraryScanError::Library(LibraryServiceError::ReadOnlyRoot) => "library_root_read_only",
        LibraryScanError::Database(_) => "database_error",
        LibraryScanError::Io(_) | LibraryScanError::FileStore(_) => "library_root_unavailable",
        LibraryScanError::Library(_) | LibraryScanError::Json(_) => "library_scan_failed",
    };
    LibraryScanJobError::new(code, "The managed library scan could not be completed.")
}

fn publish_phase(reporter: &LibraryScanJobReporter, phase: &'static str, order: u8) {
    if let Err(error) = reporter.set_phase(phase, order) {
        tracing::error!(?error, phase, "failed to publish library scan phase");
    }
}

fn publish_progress(reporter: &LibraryScanJobReporter, current: usize, total: usize) {
    if let Err(error) = reporter.set_progress(
        u64::try_from(current).unwrap_or(u64::MAX),
        u64::try_from(total).unwrap_or(u64::MAX),
    ) {
        tracing::error!(
            ?error,
            current,
            total,
            "failed to publish library scan progress"
        );
    }
}

struct Discovery<'a> {
    batches: Vec<ScanBatch>,
    unimported: Vec<UnimportedScanFile>,
    scanned_file_count: usize,
    indexed_paths: &'a HashSet<String>,
}

#[derive(Debug)]
struct ScanBatch {
    platform: Platform,
    in_game_folder: bool,
    files: Vec<DiscoveredFile>,
    blocker: Option<(&'static str, &'static str)>,
}

#[derive(Debug, Clone)]
struct DiscoveredFile {
    relative_path: String,
    display_path: String,
    file_name: String,
    file_size_bytes: u64,
    modified: SystemTime,
}

async fn writable_default_root(state: &AppState) -> Result<(i64, PathBuf), LibraryServiceError> {
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;
    if !root.writable {
        return Err(LibraryServiceError::ReadOnlyRoot);
    }
    Ok((root.id, root_path))
}

pub(crate) async fn preview_sidecar_cleanup(
    state: &AppState,
) -> Result<SidecarCleanupPreview, LibraryServiceError> {
    let (root_id, root_path) = writable_default_root(state).await?;
    let _root_lock = state.file_store().lock_root(&root_path).await?;
    let indexed_paths = roms::indexed_relative_paths(state.db(), root_id).await?;
    let asset_root = state.config().asset_root.canonicalize()?;
    Ok(collect_sidecars(
        &root_path,
        &asset_root,
        &indexed_paths,
        SIDECAR_CLEANUP_FILE_LIMIT,
    )
    .await?)
}

pub(crate) async fn cleanup_sidecars(
    state: &AppState,
    expected: Vec<SidecarCleanupFile>,
    actor_user_id: Option<i64>,
) -> Result<SidecarCleanupResult, LibraryServiceError> {
    if expected.is_empty() {
        return Err(LibraryServiceError::EmptySidecarCleanup);
    }
    if expected.len() > SIDECAR_CLEANUP_FILE_LIMIT {
        return Err(LibraryServiceError::SidecarCleanupTooLarge);
    }

    let (root_id, root_path) = writable_default_root(state).await?;
    let root_lock = state.file_store().lock_root(&root_path).await?;
    let indexed_paths = roms::indexed_relative_paths(state.db(), root_id).await?;
    let asset_root = state.config().asset_root.canonicalize()?;
    let current = collect_sidecars(
        &root_path,
        &asset_root,
        &indexed_paths,
        SIDECAR_CLEANUP_FILE_LIMIT,
    )
    .await?;
    if current.files != expected {
        return Err(LibraryServiceError::SidecarCleanupPreviewChanged);
    }

    let deleted_file_bytes = expected.iter().fold(0_u64, |total, file| {
        total.saturating_add(file.file_size_bytes)
    });
    let deleted_files = expected
        .iter()
        .map(|file| file.relative_path.clone())
        .collect::<Vec<_>>();
    super::delete::delete_unindexed_files(state, root_id, &root_lock, &expected, actor_user_id)
        .await?;
    Ok(SidecarCleanupResult {
        deleted_file_count: deleted_files.len(),
        deleted_file_bytes,
        deleted_files,
    })
}

async fn collect_sidecars(
    root: &Path,
    asset_root: &Path,
    indexed_paths: &HashSet<String>,
    limit: usize,
) -> Result<SidecarCleanupPreview, io::Error> {
    let mut pending = VecDeque::from([root.to_path_buf()]);
    let mut files = Vec::new();
    let mut total_file_bytes = 0_u64;
    while let Some(directory) = pending.pop_front() {
        for entry in sorted_entries(&directory).await? {
            let path = entry.path();
            let name = entry.file_name();
            if path == asset_root
                || (asset_root == root && directory == root && name == OsStr::new("library"))
                || (directory == root && is_application_owned_name(&name))
            {
                continue;
            }
            let metadata = tokio::fs::symlink_metadata(&path).await?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push_back(path);
                continue;
            }
            if metadata.file_type().is_symlink()
                || !metadata.is_file()
                || !is_obvious_sidecar(&name)
            {
                continue;
            }
            let Some(relative_path) = relative_path_text(root, &path) else {
                continue;
            };
            if indexed_paths.contains(&relative_path) {
                continue;
            }
            if files.len() == limit {
                return Ok(SidecarCleanupPreview {
                    file_count: files.len(),
                    total_file_bytes,
                    truncated: true,
                    files,
                });
            }
            total_file_bytes = total_file_bytes.saturating_add(metadata.len());
            files.push(SidecarCleanupFile {
                relative_path,
                file_size_bytes: metadata.len(),
                fingerprint: format!(
                    "{}:{}",
                    metadata.len(),
                    metadata
                        .modified()?
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| {
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                "sidecar modification time predates the Unix epoch",
                            )
                        })?
                        .as_nanos()
                ),
            });
        }
    }
    Ok(SidecarCleanupPreview {
        file_count: files.len(),
        total_file_bytes,
        truncated: false,
        files,
    })
}

fn is_application_owned_name(name: &OsStr) -> bool {
    matches!(name.to_str(), Some(".teatro.lock" | ".uploads" | ".trash"))
}

fn is_zone_identifier(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    let lower = lower.strip_suffix(":$data").unwrap_or(&lower);
    lower == ".identifier"
        || lower.ends_with(":zone.identifier")
        || lower.ends_with("\u{f03a}zone.identifier")
}

fn is_obvious_sidecar(name: &OsStr) -> bool {
    if is_zone_identifier(name) {
        return true;
    }
    let Some(name) = name.to_str() else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    matches!(lower.as_str(), ".ds_store" | "thumbs.db" | "desktop.ini")
        || lower.starts_with("._")
        || matches!(
            Path::new(&lower).extension().and_then(OsStr::to_str),
            Some(
                "txt"
                    | "nfo"
                    | "jpg"
                    | "jpeg"
                    | "png"
                    | "gif"
                    | "webp"
                    | "bmp"
                    | "svg"
                    | "pdf"
                    | "xml"
                    | "json"
            )
        )
}

async fn discover<'a>(
    root: &Path,
    platforms: Vec<Platform>,
    indexed_paths: &'a HashSet<String>,
) -> Result<Discovery<'a>, io::Error> {
    let platforms_by_directory: HashMap<String, Platform> = platforms
        .into_iter()
        .map(|platform| (platform.fs_slug.clone(), platform))
        .collect();
    let mut discovery = Discovery {
        batches: Vec::new(),
        unimported: Vec::new(),
        scanned_file_count: 0,
        indexed_paths,
    };

    for entry in sorted_entries(root).await? {
        let name = entry.file_name();
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if discovery.report_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if is_internal_name(&name) && !discovery.is_indexed_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            discovery.reject_path(
                root,
                &path,
                "symlink_not_supported",
                "Symlinks are not followed or imported.",
            );
            continue;
        }

        let Some(name_utf8) = name.to_str() else {
            if metadata.is_dir() {
                discovery
                    .reject_tree(
                        root,
                        path,
                        "non_utf8_path",
                        "This path cannot be represented as UTF-8 text.",
                    )
                    .await?;
            } else {
                discovery.reject_path(
                    root,
                    &path,
                    "non_utf8_path",
                    "This path cannot be represented as UTF-8 text.",
                );
            }
            continue;
        };

        let Some(platform) = platforms_by_directory.get(name_utf8).cloned() else {
            if metadata.is_dir() {
                discovery
                    .reject_tree(
                        root,
                        path,
                        "unknown_platform_directory",
                        "The top-level directory does not match a configured platform.",
                    )
                    .await?;
            } else {
                discovery.reject_path(
                    root,
                    &path,
                    "unsupported_root_file",
                    "Files must be placed inside a configured platform directory.",
                );
            }
            continue;
        };

        if !metadata.is_dir() {
            discovery.reject_path(
                root,
                &path,
                "invalid_platform_directory",
                "A configured platform path must be a directory.",
            );
            continue;
        }
        discover_platform(root, &path, platform, &mut discovery).await?;
    }

    discovery.batches.sort_by(|left, right| {
        left.platform
            .fs_slug
            .cmp(&right.platform.fs_slug)
            .then_with(|| {
                left.files
                    .first()
                    .map(|file| &file.relative_path)
                    .cmp(&right.files.first().map(|file| &file.relative_path))
            })
    });
    Ok(discovery)
}

async fn discover_platform(
    root: &Path,
    platform_path: &Path,
    platform: Platform,
    discovery: &mut Discovery<'_>,
) -> Result<(), io::Error> {
    for entry in sorted_entries(platform_path).await? {
        let name = entry.file_name();
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if discovery.report_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if is_internal_name(&name) && !discovery.is_indexed_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            discovery.reject_path(
                root,
                &path,
                "symlink_not_supported",
                "Symlinks are not followed or imported.",
            );
        } else if metadata.is_file() {
            if let Some(file) = discovered_file(root, &path, &metadata)? {
                discovery.scanned_file_count += 1;
                discovery.batches.push(ScanBatch {
                    platform: platform.clone(),
                    in_game_folder: false,
                    files: vec![file],
                    blocker: None,
                });
            } else {
                discovery.reject_path(
                    root,
                    &path,
                    "non_utf8_path",
                    "This path cannot be represented as UTF-8 text.",
                );
            }
        } else if metadata.is_dir() {
            discover_game_folder(root, &path, platform.clone(), discovery).await?;
        } else {
            discovery.reject_path(
                root,
                &path,
                "non_regular_file",
                "Only regular files can be imported.",
            );
        }
    }
    Ok(())
}

async fn discover_game_folder(
    root: &Path,
    folder: &Path,
    platform: Platform,
    discovery: &mut Discovery<'_>,
) -> Result<(), io::Error> {
    let mut files = Vec::new();
    let mut blocker = None;
    for entry in sorted_entries(folder).await? {
        let name = entry.file_name();
        let path = entry.path();
        let metadata = tokio::fs::symlink_metadata(&path).await?;
        if discovery.report_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if is_internal_name(&name) && !discovery.is_indexed_sidecar(root, &path, &name, &metadata) {
            continue;
        }
        if metadata.file_type().is_symlink() {
            blocker.get_or_insert((
                "symlink_not_supported",
                "The game folder contains a symlink, so the complete batch was left untouched.",
            ));
            discovery.reject_path(
                root,
                &path,
                "symlink_not_supported",
                "Symlinks are not followed or imported.",
            );
        } else if metadata.is_file() {
            if let Some(file) = discovered_file(root, &path, &metadata)? {
                discovery.scanned_file_count += 1;
                files.push(file);
            } else {
                blocker.get_or_insert((
                    "non_utf8_path",
                    "The game folder contains a non-UTF-8 path, so the complete batch was left untouched.",
                ));
                discovery.reject_path(
                    root,
                    &path,
                    "non_utf8_path",
                    "This path cannot be represented as UTF-8 text.",
                );
            }
        } else if metadata.is_dir() {
            if discovery
                .reject_tree(
                    root,
                    path,
                    "unsupported_nested_path",
                    "Files below the supported game-folder level are not imported.",
                )
                .await?
            {
                blocker.get_or_insert((
                    "unsupported_nested_path",
                    "The game folder contains nested files, so the complete batch was left untouched.",
                ));
            }
        } else {
            blocker.get_or_insert((
                "non_regular_file",
                "The game folder contains a non-regular entry, so the complete batch was left untouched.",
            ));
            discovery.reject_path(
                root,
                &path,
                "non_regular_file",
                "Only regular files can be imported.",
            );
        }
    }
    if !files.is_empty() {
        discovery.batches.push(ScanBatch {
            platform,
            in_game_folder: true,
            files,
            blocker,
        });
    }
    Ok(())
}

impl Discovery<'_> {
    fn is_indexed_sidecar(
        &self,
        root: &Path,
        path: &Path,
        name: &OsStr,
        metadata: &std::fs::Metadata,
    ) -> bool {
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && is_obvious_sidecar(name)
            && relative_path_text(root, path)
                .is_some_and(|relative_path| self.indexed_paths.contains(&relative_path))
    }

    fn report_sidecar(
        &mut self,
        root: &Path,
        path: &Path,
        name: &OsStr,
        metadata: &std::fs::Metadata,
    ) -> bool {
        if metadata.is_file()
            && !metadata.file_type().is_symlink()
            && is_obvious_sidecar(name)
            && !self.is_indexed_sidecar(root, path, name, metadata)
        {
            self.reject_path(
                root,
                path,
                "ignored_sidecar",
                "This known sidecar was preserved and not imported. Use sidecar cleanup to remove it explicitly.",
            );
            true
        } else {
            false
        }
    }

    fn reject_path(&mut self, root: &Path, path: &Path, code: &str, reason: &str) {
        self.scanned_file_count += 1;
        self.unimported.push(unimported(
            safe_display_path(root, path),
            "not_imported",
            code,
            reason,
        ));
    }

    async fn reject_tree(
        &mut self,
        root: &Path,
        start: PathBuf,
        code: &str,
        reason: &str,
    ) -> Result<bool, io::Error> {
        let mut pending = VecDeque::from([start]);
        let mut rejected_non_sidecar = false;
        while let Some(directory) = pending.pop_front() {
            for entry in sorted_entries(&directory).await? {
                let name = entry.file_name();
                let path = entry.path();
                let metadata = tokio::fs::symlink_metadata(&path).await?;
                if self.report_sidecar(root, &path, &name, &metadata) {
                    continue;
                }
                if is_internal_name(&name)
                    && !self.is_indexed_sidecar(root, &path, &name, &metadata)
                {
                    continue;
                }
                if metadata.is_dir() && !metadata.file_type().is_symlink() {
                    pending.push_back(path);
                } else {
                    self.reject_path(root, &path, code, reason);
                    rejected_non_sidecar = true;
                }
            }
        }
        Ok(rejected_non_sidecar)
    }
}

async fn sorted_entries(path: &Path) -> Result<Vec<tokio::fs::DirEntry>, io::Error> {
    let mut reader = tokio::fs::read_dir(path).await?;
    let mut entries = Vec::new();
    while let Some(entry) = reader.next_entry().await? {
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn discovered_file(
    root: &Path,
    path: &Path,
    metadata: &std::fs::Metadata,
) -> Result<Option<DiscoveredFile>, io::Error> {
    let Some(relative_path) = relative_path_text(root, path) else {
        return Ok(None);
    };
    let Some(file_name) = path.file_name().and_then(OsStr::to_str) else {
        return Ok(None);
    };
    Ok(Some(DiscoveredFile {
        relative_path,
        display_path: safe_display_path(root, path),
        file_name: file_name.to_string(),
        file_size_bytes: metadata.len(),
        modified: metadata.modified()?,
    }))
}

fn relative_path_text(root: &Path, path: &Path) -> Option<String> {
    path.strip_prefix(root)
        .ok()?
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|components| components.join("/"))
}

fn is_internal_name(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| name.starts_with('.'))
}

async fn plan_batch(
    state: &AppState,
    root_path: &Path,
    batch: ScanBatch,
    indexed_paths: &HashSet<String>,
    unimported_files: &mut Vec<UnimportedScanFile>,
    planned: &mut Vec<PlannedBatch>,
) {
    let indexed_count = batch
        .files
        .iter()
        .filter(|file| indexed_paths.contains(&file.relative_path))
        .count();
    if indexed_count > 0 {
        for file in &batch.files {
            if indexed_paths.contains(&file.relative_path) {
                unimported_files.push(unimported(
                    file.display_path.clone(),
                    "already_indexed",
                    "already_indexed",
                    "This exact managed-library path is already indexed.",
                ));
            } else {
                unimported_files.push(unimported(
                    file.display_path.clone(),
                    "not_imported",
                    "indexed_game_changed",
                    "This game folder also contains indexed files and is not changed by scans.",
                ));
            }
        }
        return;
    }

    if let Some((code, reason)) = batch.blocker {
        reject_batch(&batch, code, reason, unimported_files);
        return;
    }

    if !batch.in_game_folder {
        let file = &batch.files[0];
        let parsed = filename::parse(&file.file_name);
        if parsed.split_archive.is_some() {
            reject_batch(
                &batch,
                "unsupported_split_archive",
                "Split archive volumes are not imported.",
                unimported_files,
            );
            return;
        }
        if is_manifest_file_name(&file.file_name) || parsed.disc.is_some() {
            reject_batch(
                &batch,
                "grouped_game_requires_folder",
                "Descriptors, playlists, and disc-marked files must be placed in a dedicated game folder.",
                unimported_files,
            );
            return;
        }
    }

    if has_case_collisions(&batch.files) {
        reject_batch(
            &batch,
            "case_colliding_file_names",
            "The game folder contains filenames that differ only by letter case.",
            unimported_files,
        );
        return;
    }
    if batch
        .files
        .iter()
        .any(|file| filename::parse(&file.file_name).split_archive.is_some())
    {
        reject_batch(
            &batch,
            "unsupported_split_archive",
            "Split archive volumes are not imported.",
            unimported_files,
        );
        return;
    }

    let mut inputs = Vec::with_capacity(batch.files.len());
    for (index, file) in batch.files.iter().enumerate() {
        let manifest_contents = if is_manifest_file_name(&file.file_name) {
            match state
                .file_store()
                .read_existing(root_path, &file.relative_path, MAX_MANIFEST_BYTES)
                .await
            {
                Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
                Err(_) => {
                    reject_batch(
                        &batch,
                        "manifest_unreadable",
                        "A descriptor or playlist could not be read safely.",
                        unimported_files,
                    );
                    return;
                }
            }
        } else {
            None
        };
        inputs.push(PlanInputFile {
            index,
            original_file_name: file.file_name.clone(),
            file_size_bytes: Some(file.file_size_bytes),
            staged_path: None,
            manifest_contents,
        });
    }

    let plan = build_ingest_plan(batch.platform.id, &batch.platform.slug, inputs, None);
    if !plan.errors.is_empty() {
        let code = plan.errors[0].code.clone();
        reject_batch(
            &batch,
            &code,
            &summarize_ingest_errors(&plan.errors),
            unimported_files,
        );
        return;
    }
    if plan.roms.len() != 1 {
        reject_batch(
            &batch,
            "ambiguous_game_folder",
            "The game folder does not describe exactly one logical ROM.",
            unimported_files,
        );
        return;
    }

    planned.push(PlannedBatch {
        batch,
        rom: plan.roms.into_iter().next().expect("one planned ROM"),
    });
}

fn has_case_collisions(files: &[DiscoveredFile]) -> bool {
    let mut names = HashSet::new();
    files
        .iter()
        .any(|file| !names.insert(file.file_name.to_lowercase()))
}

struct PlannedBatch {
    batch: ScanBatch,
    rom: super::types::IngestPlanRom,
}

async fn revalidate_batch(
    state: &AppState,
    root_path: &Path,
    root_id: i64,
    batch: &ScanBatch,
) -> Result<Option<(String, String)>, LibraryScanError> {
    for file in &batch.files {
        let path = match state
            .file_store()
            .resolve_existing(root_path, &file.relative_path)
        {
            Ok(path) => path,
            Err(_) => {
                return Ok(Some((
                    "file_changed_during_scan".into(),
                    "The file disappeared or became unsafe during the scan.".into(),
                )));
            }
        };
        let metadata = match tokio::fs::symlink_metadata(path).await {
            Ok(metadata) => metadata,
            Err(_) => {
                return Ok(Some((
                    "file_changed_during_scan".into(),
                    "The file disappeared or became unreadable during the scan.".into(),
                )));
            }
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != file.file_size_bytes
            || metadata.modified().ok() != Some(file.modified)
        {
            return Ok(Some((
                "file_changed_during_scan".into(),
                "The file size, modification time, or type changed during the scan.".into(),
            )));
        }
        if roms::relative_path_exists(state.db(), root_id, &file.relative_path).await? {
            return Ok(Some((
                "path_became_indexed".into(),
                "The path was indexed by another operation during the scan.".into(),
            )));
        }
    }
    Ok(None)
}

struct ImportedBatch {
    summary: ImportedScannedRom,
    generated_manifest_count: usize,
}

async fn import_batch(
    state: &AppState,
    root_path: &Path,
    root_id: i64,
    planned: &PlannedBatch,
) -> Result<ImportedBatch, LibraryScanError> {
    let mut sources = planned
        .batch
        .files
        .iter()
        .enumerate()
        .map(|(index, file)| InPlaceSourceFile {
            index,
            relative_path: file.relative_path.clone(),
            file_name: file.file_name.clone(),
            file_size_bytes: file.file_size_bytes,
        })
        .collect::<Vec<_>>();
    normalize_m3u_game_folder(state, root_path, &planned.rom, &mut sources).await?;
    let finalized = prepare_in_place_finalization(
        state,
        root_path,
        root_id,
        &planned.batch.platform,
        &planned.rom,
        &sources,
    )
    .await?;
    let generated_manifest_count = finalized.operations.len();

    let operation_id = if finalized.operations.is_empty() {
        None
    } else {
        let operation_id = file_operations::new_operation_id();
        let paths = finalized
            .operations
            .iter()
            .map(operation_path)
            .map(str::to_string)
            .collect();
        let payload = serde_json::to_string(&UploadOperationPayload::new(root_id, paths))?;
        file_operation_repository::create(
            state.db(),
            &operation_id,
            FileOperationKind::Upload,
            &payload,
        )
        .await?;
        for operation in &finalized.operations {
            let BatchFileOperation::WriteGenerated {
                relative_path,
                contents,
            } = operation
            else {
                return Err(LibraryServiceError::InvalidIngestPlan(
                    "in-place ingest attempted to move a source file".into(),
                )
                .into());
            };
            if let Err(error) = state
                .file_store()
                .write_new_atomic(root_path, relative_path, contents.as_bytes())
                .await
            {
                file_operations::fail(state, &operation_id, &error.to_string()).await;
                return Err(error.into());
            }
        }
        Some(operation_id)
    };

    let persisted =
        roms::create_grouped_roms(state.db(), finalized.roms, operation_id.as_deref()).await;
    let rom_id = match persisted {
        Ok(ids) => ids.into_iter().next().ok_or(sqlx::Error::RowNotFound)?,
        Err(error) => {
            if let Some(operation_id) = operation_id.as_deref() {
                recover_generated_manifest_error(
                    state,
                    root_path,
                    operation_id,
                    &finalized.operations,
                    &error,
                )
                .await;
            }
            return Err(error.into());
        }
    };
    if let Some(operation_id) = operation_id.as_deref() {
        file_operations::complete(state, operation_id).await?;
    }
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;

    Ok(ImportedBatch {
        summary: ImportedScannedRom {
            id: rom.id,
            name: rom.name,
            platform_slug: rom.platform_slug,
            file_count: rom.files.len(),
        },
        generated_manifest_count,
    })
}

async fn normalize_m3u_game_folder(
    state: &AppState,
    root_path: &Path,
    planned_rom: &super::types::IngestPlanRom,
    sources: &mut [InPlaceSourceFile],
) -> Result<(), LibraryScanError> {
    let Some(directory_name) = m3u_directory_name(planned_rom)? else {
        return Ok(());
    };
    let current_parent = sources
        .first()
        .and_then(|source| Path::new(&source.relative_path).parent())
        .ok_or_else(|| {
            LibraryServiceError::InvalidIngestPlan("scan batch has no source parent".into())
        })?;
    if current_parent.file_name() == Some(OsStr::new(&directory_name)) {
        return Ok(());
    }
    let platform_parent = current_parent.parent().ok_or_else(|| {
        LibraryServiceError::InvalidIngestPlan("scan batch is not inside a platform folder".into())
    })?;
    let current_parent = current_parent.to_str().ok_or_else(|| {
        LibraryServiceError::InvalidIngestPlan("scan batch path is not UTF-8".into())
    })?;
    let target_parent = join_relative_path(platform_parent, &directory_name);
    if state.file_store().exists(root_path, &target_parent).await? {
        return Err(LibraryServiceError::NoAvailableFileName.into());
    }

    // ponytail: this atomic normalization is intentionally retryable instead of journaled.
    match state
        .file_store()
        .move_managed_directory(root_path, current_parent, &target_parent)
        .await
    {
        Ok(_) => {}
        Err(FileStoreError::AlreadyExists) => {
            return Err(LibraryServiceError::NoAvailableFileName.into());
        }
        Err(error) => return Err(error.into()),
    }
    for source in sources {
        source.relative_path = join_relative_path(Path::new(&target_parent), &source.file_name);
    }
    Ok(())
}

async fn recover_generated_manifest_error(
    state: &AppState,
    root_path: &Path,
    operation_id: &str,
    operations: &[BatchFileOperation],
    persistence_error: &sqlx::Error,
) {
    match file_operation_repository::state_by_id(state.db(), operation_id).await {
        Ok(Some(FileOperationState::Prepared)) => {
            for operation in operations {
                if let Err(error) = state
                    .file_store()
                    .remove(root_path, operation_path(operation))
                    .await
                {
                    tracing::warn!(
                        ?error,
                        operation_id,
                        "failed to remove an uncommitted generated playlist"
                    );
                }
            }
            file_operations::fail(state, operation_id, &persistence_error.to_string()).await;
        }
        Ok(_) | Err(_) => tracing::error!(
            ?persistence_error,
            operation_id,
            "generated playlist was retained for startup reconciliation"
        ),
    }
}

fn operation_path(operation: &BatchFileOperation) -> &str {
    match operation {
        BatchFileOperation::Move { relative_path, .. }
        | BatchFileOperation::WriteGenerated { relative_path, .. } => relative_path,
    }
}

fn reject_batch(batch: &ScanBatch, code: &str, reason: &str, output: &mut Vec<UnimportedScanFile>) {
    output.extend(
        batch
            .files
            .iter()
            .map(|file| unimported(file.display_path.clone(), "not_imported", code, reason)),
    );
}

fn unimported(
    relative_path: String,
    disposition: &str,
    reason_code: &str,
    reason: &str,
) -> UnimportedScanFile {
    UnimportedScanFile {
        relative_path: bounded_text(&relative_path, MAX_DISPLAY_PATH_BYTES).0,
        disposition: disposition.to_string(),
        reason_code: bounded_text(reason_code, 128).0,
        reason: bounded_text(reason, MAX_REASON_BYTES).0,
    }
}

fn safe_display_path(root: &Path, path: &Path) -> String {
    let Ok(relative) = path.strip_prefix(root) else {
        return "<unavailable>".to_string();
    };
    let display = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    bounded_text(&display, MAX_DISPLAY_PATH_BYTES).0
}

pub(crate) fn scan_reason_codes(result: &LibraryScanResult) -> BTreeSet<String> {
    result
        .unimported_files
        .iter()
        .map(|file| file.reason_code.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::is_obvious_sidecar;

    #[test]
    fn zone_identifiers_are_classified_as_sidecars() {
        for name in [
            ".Identifier",
            "Game.chd:Zone.Identifier",
            "Game.chd\u{f03a}Zone.Identifier",
        ] {
            assert!(is_obvious_sidecar(OsStr::new(name)));
        }
    }
}
