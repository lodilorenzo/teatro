//! The supervised RomM import workflow.
//!
//! An import is an ordinary Teatro ingest whose bytes happen to arrive over HTTP. Downloads
//! stream into the managed root's journalled `.uploads` staging area — the same recoverable
//! staging an admin upload uses — and are then handed to `finalize_upload` /
//! `finalize_upload_batch`, so grouped manifests, slugging, and the file-operation journal behave
//! identically to an upload. Nothing here writes to the remote server.

use std::{path::PathBuf, time::Duration};

use crc32fast::Hasher as Crc32Hasher;
use md5::Md5;
use serde::Serialize;
use serde_json::{Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::{
    domain::{integrity::FileHashes, workflow::FileOperationKind},
    repositories::{integrity, library_roots, platforms, romm_source, roms},
    services::{
        file_operations::{self, UploadOperationPayload},
        igdb,
        library::{
            self, LibraryServiceError, UpdateRomDraft, UploadBatchDraft, UploadBatchFileDraft,
            UploadDraft, normalized_title, sanitize_upload_file_name,
        },
    },
    state::AppState,
    storage::file_store::FileStore,
};

use super::{
    RemoteRomFile, RemoteRomMetadata, RommSourceError,
    client::transport_error,
    duplicates::{self, CandidateFile, DuplicateMatch},
    jobs::{RommImportJobError, RommImportJobReporter},
};

const DOWNLOAD_WRITE_BUFFER_BYTES: usize = 1024 * 1024;

/// One explicit admin request to import one remote game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RommImportRequest {
    pub remote_rom_id: i64,
    pub remote_file_ids: Vec<i64>,
    pub platform_id: i64,
    pub title: Option<String>,
}

/// Terminal report for one import. `already_present` is a normal outcome, not an error.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RommImportOutcome {
    /// `imported` or `already_present`.
    pub outcome: &'static str,
    pub remote_rom_id: i64,
    pub title: String,
    pub platform_id: i64,
    pub platform_slug: String,
    pub file_count: usize,
    pub downloaded_bytes: u64,
    /// Which hash algorithms the duplicate check could actually use.
    pub hash_signals: Vec<&'static str>,
    pub imported_rom_ids: Vec<i64>,
    pub imported_rom_name: Option<String>,
    pub duplicate: Option<DuplicateMatch>,
    /// Declared-versus-computed hash conflicts. Non-empty means the import completed with a
    /// warning: Teatro kept the bytes it actually received and stored the digests it computed
    /// from them, so the local library describes the local file rather than the remote claim.
    pub hash_conflicts: Vec<RommHashConflict>,
}

/// One hash the remote server declared that the downloaded bytes did not produce.
///
/// Both values are lowercase hex — the declared side is only accepted in that form by the client —
/// and the report names which algorithm disagreed so an operator can act on it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct RommHashConflict {
    pub file_name: String,
    pub algorithm: &'static str,
    pub declared: String,
    pub computed: String,
}

/// Bounds one job snapshot: four algorithms across a handful of files is already actionable, and a
/// snapshot is polled repeatedly by every watching browser.
const MAX_REPORTED_HASH_CONFLICTS: usize = 32;

/// Summarizes a workflow failure for the job snapshot, without leaking credentials.
pub(crate) fn import_failure_summary(error: &RommSourceError) -> RommImportJobError {
    RommImportJobError::new(error.code(), &error.to_string())
}

fn publish_phase(reporter: &RommImportJobReporter, phase: &'static str, order: u8) {
    if let Err(error) = reporter.set_phase(phase, order) {
        tracing::warn!(%error, phase, "ignored an invalid RomM import phase transition");
    }
}

fn publish_progress(reporter: &RommImportJobReporter, current: u64, total: u64) {
    if let Err(error) = reporter.set_progress(current, total) {
        tracing::debug!(%error, "ignored an invalid RomM import progress update");
    }
}

pub(crate) async fn import_rom(
    state: &AppState,
    request: RommImportRequest,
    reporter: &RommImportJobReporter,
) -> Result<RommImportOutcome, RommSourceError> {
    let timeout = Duration::from_secs(state.config().romm_source.import_timeout_seconds.max(1));
    match tokio::time::timeout(timeout, import_rom_inner(state, request, reporter)).await {
        Ok(result) => result,
        Err(_) => Err(RommSourceError::TimedOut),
    }
}

async fn import_rom_inner(
    state: &AppState,
    request: RommImportRequest,
    reporter: &RommImportJobReporter,
) -> Result<RommImportOutcome, RommSourceError> {
    // Phase 1: resolving. Teatro re-fetches the remote detail server-side; the browser never
    // dictates URLs, paths, or sizes.
    publish_phase(reporter, "resolving", 1);
    if !state.config().romm_source.enabled {
        return Err(RommSourceError::Disabled);
    }
    let settings = romm_source::load(state.db())
        .await?
        .filter(romm_source::RommSourceSettings::has_credentials)
        .ok_or(RommSourceError::NotConfigured)?;

    let detail = state
        .romm_client()
        .rom_detail(&settings, request.remote_rom_id)
        .await?;
    let selected = select_files(&detail.files, &request.remote_file_ids)?;
    let limits = state.config().romm_source;
    if selected.len() > limits.max_import_files {
        return Err(RommSourceError::TooManyFiles);
    }
    let declared_bytes = selected
        .iter()
        .filter_map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    if declared_bytes > limits.max_import_bytes {
        return Err(RommSourceError::ImportTooLarge);
    }

    let platform = platforms::find_by_id(state.db(), request.platform_id)
        .await?
        .ok_or(RommSourceError::PlatformNotFound)?;

    // Remote filenames are attacker-controlled: they pass Teatro's existing path-safety and
    // normalization primitives before touching the filesystem or the database.
    let mut staged_names = Vec::with_capacity(selected.len());
    for file in &selected {
        staged_names.push(sanitize_upload_file_name(&file.file_name)?);
    }
    let title = normalized_title(
        request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .or(Some(detail.rom.name.as_str())),
        &staged_names[0],
    )?;
    let igdb_search_title = igdb_title_from_file_name(&staged_names[0])?;

    let declared_candidates = staged_names
        .iter()
        .zip(&selected)
        .map(|(file_name, file)| CandidateFile {
            file_name: file_name.clone(),
            crc32: file.crc32.clone(),
            md5: file.md5.clone(),
            sha1: file.sha1.clone(),
            sha256: file.sha256.clone(),
        })
        .collect::<Vec<_>>();

    // Phase 2: admission-time duplicate refusal, before a single byte is transferred.
    publish_phase(reporter, "checking", 2);
    if let Some(duplicate) =
        duplicates::find_duplicate(state, platform.id, &title, &declared_candidates).await?
    {
        return Ok(RommImportOutcome {
            outcome: "already_present",
            remote_rom_id: request.remote_rom_id,
            title,
            platform_id: platform.id,
            platform_slug: platform.slug,
            file_count: selected.len(),
            downloaded_bytes: 0,
            hash_signals: duplicates::hash_signals(&declared_candidates),
            imported_rom_ids: Vec::new(),
            imported_rom_name: None,
            duplicate: Some(duplicate),
            // Nothing was downloaded, so there is nothing to compare a declared hash against.
            hash_conflicts: Vec::new(),
        });
    }

    // Phases 3-6 own staged files, so every exit from here cleans staging.
    let staged = match download_all(
        state,
        &settings,
        &request,
        &selected,
        &staged_names,
        reporter,
    )
    .await
    {
        Ok(staged) => staged,
        Err(error) => return Err(error),
    };

    let result = finish_import(
        state, &request, &platform, &title, &selected, &staged, reporter,
    )
    .await;
    if result.is_err() || matches!(&result, Ok(outcome) if outcome.outcome == "already_present") {
        discard_staged(state, &staged).await;
    }
    if let Ok(outcome) = &result
        && outcome.outcome == "imported"
    {
        attach_remote_metadata(state, &detail.metadata, &outcome.imported_rom_ids).await;
        let missing_cover_ids = if detail.rom.has_cover {
            attach_remote_cover(
                state,
                &settings,
                request.remote_rom_id,
                &outcome.imported_rom_ids,
            )
            .await
        } else {
            outcome.imported_rom_ids.clone()
        };
        apply_igdb_fallback(state, &missing_cover_ids, &igdb_search_title).await;
    }
    result
}

fn igdb_title_from_file_name(file_name: &str) -> Result<String, LibraryServiceError> {
    normalized_title(None, &sanitize_upload_file_name(file_name)?)
}

/// Copies the remote game's cover onto the newly imported ROMs.
///
/// This runs after the ingest succeeded, so it is deliberately best-effort: a remote server that
/// has no usable cover, refuses the request, or returns something that is not a JPEG, PNG, or WebP
/// must not turn a completed import into a failure. The bytes go through the same bounded,
/// magic-byte-checked, journalled cover writer an admin upload uses; the remote server never
/// dictates a path or a media type.
async fn attach_remote_metadata(state: &AppState, remote: &RemoteRomMetadata, rom_ids: &[i64]) {
    for rom_id in rom_ids {
        let rom = match roms::find_by_id(state.db(), *rom_id).await {
            Ok(Some(rom)) => rom,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    rom_id,
                    "could not reload a RomM import for metadata"
                );
                continue;
            }
        };
        let (metadata, regions) = merged_remote_metadata(&rom.metadata, &rom.regions, remote);
        let metadata_json = metadata.to_string();
        let regions_json = match serde_json::to_string(&regions) {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(?error, rom_id, "could not encode RomM import regions");
                continue;
            }
        };
        if let Err(error) = roms::update_admin_metadata(
            state.db(),
            roms::UpdateRomMetadataParams {
                rom_id: *rom_id,
                platform_id: rom.platform_id,
                name: &rom.name,
                slug: &rom.slug,
                summary: remote.summary.as_deref().or(rom.summary.as_deref()),
                regions_json: &regions_json,
                metadata_source: "romm",
                metadata_json: &metadata_json,
                schema_version: 1,
            },
        )
        .await
        {
            tracing::warn!(
                ?error,
                rom_id,
                "could not store RomM metadata for an import"
            );
        }
    }
}

fn merged_remote_metadata(
    existing: &Value,
    local_regions: &[String],
    remote: &RemoteRomMetadata,
) -> (Value, Vec<String>) {
    let mut metadata = remote.value.as_object().cloned().unwrap_or_default();
    if let Some(existing) = existing.as_object() {
        for key in ["integrity", "filename", "serials"] {
            if let Some(value) = existing.get(key) {
                metadata.insert(key.to_string(), value.clone());
            }
        }
        let languages = merged_text_values(
            existing.get("languages"),
            remote.value.get("languages"),
            &[],
        );
        metadata.insert("languages".to_string(), json!(languages));
    }
    let regions = merged_text_values(
        existing.get("regions"),
        remote.value.get("regions"),
        local_regions,
    );
    metadata.insert("regions".to_string(), json!(&regions));
    (Value::Object(metadata), regions)
}

fn merged_text_values(
    existing: Option<&Value>,
    incoming: Option<&Value>,
    base: &[String],
) -> Vec<String> {
    let mut values = base.to_vec();
    for value in [existing, incoming]
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(Value::as_str)
    {
        let value = super::bounded_remote_text(value);
        if !value.is_empty() {
            values.push(value);
        }
    }
    values.sort();
    values.dedup();
    values
}

/// Copies the remote game's cover onto the newly imported ROMs and returns IDs still missing one.
async fn attach_remote_cover(
    state: &AppState,
    settings: &romm_source::RommSourceSettings,
    remote_rom_id: i64,
    rom_ids: &[i64],
) -> Vec<i64> {
    if rom_ids.is_empty() {
        return Vec::new();
    }
    let cover = match state.romm_client().cover(settings, remote_rom_id).await {
        Ok(cover) => cover,
        Err(error) => {
            tracing::info!(
                code = error.code(),
                remote_rom_id,
                "no remote cover was stored for an imported game"
            );
            return rom_ids.to_vec();
        }
    };

    let mut missing = Vec::new();
    for rom_id in rom_ids {
        if let Err(error) =
            igdb::replace_cover_bytes(state, *rom_id, cover.bytes.clone(), "romm").await
        {
            tracing::warn!(
                ?error,
                rom_id,
                "could not store the remote cover for an import"
            );
            missing.push(*rom_id);
        }
    }
    missing
}

async fn apply_igdb_fallback(state: &AppState, rom_ids: &[i64], search_title: &str) {
    for rom_id in rom_ids {
        match state
            .igdb_client()
            .auto_apply_metadata(state, *rom_id, search_title)
            .await
        {
            Ok(igdb::AutomaticMetadataOutcome::Applied(applied)) => {
                let detected_name = igdb_detected_name(&applied.rom.metadata);
                let name_updated = if let Some(name) = detected_name
                    && name != applied.rom.name.as_str()
                {
                    match library::update_rom(
                        state,
                        *rom_id,
                        UpdateRomDraft {
                            name: Some(name.to_string()),
                            ..UpdateRomDraft::default()
                        },
                    )
                    .await
                    {
                        Ok(_) => true,
                        Err(error) => {
                            tracing::warn!(
                                ?error,
                                rom_id,
                                "could not apply the IGDB game name to a RomM import"
                            );
                            false
                        }
                    }
                } else {
                    false
                };
                tracing::info!(
                    rom_id,
                    cover_downloaded = !applied.cached_covers.is_empty(),
                    name_updated,
                    "applied IGDB metadata because the RomM import had no usable cover"
                );
            }
            Ok(igdb::AutomaticMetadataOutcome::NoMatch) => {
                tracing::info!(
                    rom_id,
                    "IGDB found no match for a RomM import without a cover"
                );
            }
            Err(igdb::IgdbServiceError::NotConfigured) => {
                tracing::info!(rom_id, "IGDB fallback is not configured for a RomM import");
            }
            Err(error) => {
                tracing::warn!(?error, rom_id, "IGDB fallback failed for a RomM import");
            }
        }
    }
}

fn igdb_detected_name(metadata: &Value) -> Option<&str> {
    metadata
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
}

/// One downloaded file held in the managed root's journalled staging area.
struct StagedDownload {
    file_name: String,
    staged_path: PathBuf,
    operation_id: String,
    downloaded_bytes: u64,
    hashes: ComputedHashes,
}

#[derive(Debug, Clone, Default)]
struct ComputedHashes {
    crc32: String,
    md5: String,
    sha1: String,
    sha256: String,
}

async fn download_all(
    state: &AppState,
    settings: &romm_source::RommSourceSettings,
    request: &RommImportRequest,
    selected: &[RemoteRomFile],
    staged_names: &[String],
    reporter: &RommImportJobReporter,
) -> Result<Vec<StagedDownload>, RommSourceError> {
    publish_phase(reporter, "downloading", 3);
    let declared_total = selected
        .iter()
        .filter_map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    let mut staged: Vec<StagedDownload> = Vec::with_capacity(selected.len());
    let mut completed_bytes = 0_u64;

    for (file, file_name) in selected.iter().zip(staged_names) {
        let outcome = download_one(
            state,
            settings,
            request.remote_rom_id,
            file,
            file_name,
            reporter,
            completed_bytes,
            declared_total,
        )
        .await;
        match outcome {
            Ok(download) => {
                completed_bytes = completed_bytes.saturating_add(download.downloaded_bytes);
                staged.push(download);
            }
            Err(error) => {
                discard_staged(state, &staged).await;
                return Err(error);
            }
        }
    }
    if declared_total > 0 {
        publish_progress(
            reporter,
            declared_total.min(completed_bytes),
            declared_total,
        );
    }
    Ok(staged)
}

#[allow(clippy::too_many_arguments)]
async fn download_one(
    state: &AppState,
    settings: &romm_source::RommSourceSettings,
    remote_rom_id: i64,
    file: &RemoteRomFile,
    file_name: &str,
    reporter: &RommImportJobReporter,
    completed_bytes: u64,
    declared_total: u64,
) -> Result<StagedDownload, RommSourceError> {
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;
    if !root.writable {
        return Err(LibraryServiceError::ReadOnlyRoot.into());
    }
    ensure_available_space(
        &root_path,
        file.file_size_bytes.unwrap_or(0),
        state.config().uploads.free_space_margin_bytes,
    )
    .await?;

    let operation_id = file_operations::new_operation_id();
    let staging_relative_path = format!(".uploads/upload-{operation_id}.part");
    let payload = UploadOperationPayload::new(root.id, Vec::new())
        .with_staged_paths(vec![staging_relative_path.clone()]);
    file_operations::prepare(state, &operation_id, FileOperationKind::Upload, &payload)
        .await
        .map_err(|_| RommSourceError::Staging)?;

    let (staged_path, staging_file) = match state
        .file_store()
        .create_staging_file(&root_path, &staging_relative_path)
        .await
    {
        Ok(staged) => staged,
        Err(error) => {
            complete_staging_operation(state, &operation_id).await;
            return Err(LibraryServiceError::from(error).into());
        }
    };

    let stream = stream_to_staging(
        state,
        settings,
        remote_rom_id,
        file,
        staging_file,
        &root_path,
        reporter,
        completed_bytes,
        declared_total,
    )
    .await;

    match stream {
        Ok((downloaded_bytes, hashes)) => Ok(StagedDownload {
            file_name: file_name.to_string(),
            staged_path,
            operation_id,
            downloaded_bytes,
            hashes,
        }),
        Err(error) => {
            state.file_store().cleanup_path(&staged_path).await;
            complete_staging_operation(state, &operation_id).await;
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn stream_to_staging(
    state: &AppState,
    settings: &romm_source::RommSourceSettings,
    remote_rom_id: i64,
    file: &RemoteRomFile,
    staging_file: tokio::fs::File,
    root_path: &std::path::Path,
    reporter: &RommImportJobReporter,
    completed_bytes: u64,
    declared_total: u64,
) -> Result<(u64, ComputedHashes), RommSourceError> {
    let limits = state.config().romm_source;
    let mut response = state
        .romm_client()
        .download_file(settings, remote_rom_id, file)
        .await?;
    if response
        .content_length()
        .is_some_and(|length| length > limits.max_import_bytes)
    {
        return Err(RommSourceError::ImportTooLarge);
    }

    let mut writer = BufWriter::with_capacity(DOWNLOAD_WRITE_BUFFER_BYTES, staging_file);
    let mut crc32 = Crc32Hasher::new();
    let mut md5 = Md5::new();
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut written = 0_u64;
    let mut checked_bytes = 0_u64;
    let disk_check_interval = state.config().uploads.disk_check_interval_bytes.max(1);

    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        written = written.saturating_add(chunk.len() as u64);
        if written > limits.max_import_bytes {
            return Err(RommSourceError::ImportTooLarge);
        }
        // Abort mid-download when the free-space margin would be crossed.
        if written.saturating_sub(checked_bytes) >= disk_check_interval {
            checked_bytes = written;
            ensure_available_space(
                root_path,
                disk_check_interval,
                state.config().uploads.free_space_margin_bytes,
            )
            .await?;
        }
        crc32.update(&chunk);
        md5.update(&chunk);
        sha1.update(&chunk);
        sha256.update(&chunk);
        writer.write_all(&chunk).await?;
        if declared_total > 0 {
            publish_progress(
                reporter,
                completed_bytes.saturating_add(written).min(declared_total),
                declared_total,
            );
        }
    }

    writer.flush().await?;
    let staging_file = writer.into_inner();
    staging_file.sync_all().await?;
    drop(staging_file);

    Ok((
        written,
        ComputedHashes {
            crc32: format!("{:08x}", crc32.finalize()),
            md5: format!("{:x}", md5.finalize()),
            sha1: format!("{:x}", sha1.finalize()),
            sha256: format!("{:x}", sha256.finalize()),
        },
    ))
}

/// Compares what the remote server declared against what the transfer produced.
///
/// A declared size the transfer did not produce is terminal: a short or overlong body is a
/// truncated or wrong download rather than a stale remote record. A declared *hash* that disagrees
/// is collected and returned as a warning, and the import continues — Teatro keeps the bytes it
/// actually received and stores the digests it computed from them, so the local library describes
/// the local file instead of the remote claim.
fn verify_downloads(
    selected: &[RemoteRomFile],
    staged: &[StagedDownload],
) -> Result<Vec<RommHashConflict>, RommSourceError> {
    let mut conflicts = Vec::new();
    for (file, download) in selected.iter().zip(staged) {
        if file
            .file_size_bytes
            .is_some_and(|declared| declared != download.downloaded_bytes)
        {
            return Err(RommSourceError::SizeMismatch);
        }
        let declared_hashes = [
            (
                "sha256",
                file.sha256.as_deref(),
                download.hashes.sha256.as_str(),
            ),
            ("sha1", file.sha1.as_deref(), download.hashes.sha1.as_str()),
            ("md5", file.md5.as_deref(), download.hashes.md5.as_str()),
            (
                "crc32",
                file.crc32.as_deref(),
                download.hashes.crc32.as_str(),
            ),
        ];
        for (algorithm, declared, computed) in declared_hashes {
            let Some(declared) = declared.filter(|declared| *declared != computed) else {
                continue;
            };
            if conflicts.len() >= MAX_REPORTED_HASH_CONFLICTS {
                return Ok(conflicts);
            }
            conflicts.push(RommHashConflict {
                file_name: download.file_name.clone(),
                algorithm,
                declared: declared.to_string(),
                computed: computed.to_string(),
            });
        }
    }
    Ok(conflicts)
}

async fn finish_import(
    state: &AppState,
    request: &RommImportRequest,
    platform: &crate::domain::platform::Platform,
    title: &str,
    selected: &[RemoteRomFile],
    staged: &[StagedDownload],
    reporter: &RommImportJobReporter,
) -> Result<RommImportOutcome, RommSourceError> {
    // Phase 4: verification against what the remote server declared, then the post-download
    // duplicate check using the locally computed hashes. This is what closes the race window.
    publish_phase(reporter, "verifying", 4);
    let hash_conflicts = verify_downloads(selected, staged)?;
    if !hash_conflicts.is_empty() {
        tracing::warn!(
            remote_rom_id = request.remote_rom_id,
            conflicts = hash_conflicts.len(),
            "importing a RomM game whose downloaded bytes did not match a declared hash; storing the computed digests"
        );
    }

    let computed_candidates = staged
        .iter()
        .map(|download| CandidateFile {
            file_name: download.file_name.clone(),
            crc32: Some(download.hashes.crc32.clone()),
            md5: Some(download.hashes.md5.clone()),
            sha1: Some(download.hashes.sha1.clone()),
            sha256: Some(download.hashes.sha256.clone()),
        })
        .collect::<Vec<_>>();
    let downloaded_bytes = staged
        .iter()
        .map(|download| download.downloaded_bytes)
        .fold(0_u64, u64::saturating_add);

    if let Some(duplicate) =
        duplicates::find_duplicate(state, platform.id, title, &computed_candidates).await?
    {
        return Ok(RommImportOutcome {
            outcome: "already_present",
            remote_rom_id: request.remote_rom_id,
            title: title.to_string(),
            platform_id: platform.id,
            platform_slug: platform.slug.clone(),
            file_count: staged.len(),
            downloaded_bytes,
            hash_signals: duplicates::hash_signals(&computed_candidates),
            imported_rom_ids: Vec::new(),
            imported_rom_name: None,
            duplicate: Some(duplicate),
            // The bytes did arrive here, so a declared-hash conflict is still worth reporting even
            // though the duplicate rule means nothing was kept.
            hash_conflicts,
        });
    }

    // Phase 5: hand the staged bytes to the ordinary ingest path.
    publish_phase(reporter, "ingesting", 5);
    let (imported_rom_ids, imported_rom_name) = ingest(state, platform.id, title, staged).await?;

    publish_phase(reporter, "finalizing", 6);
    persist_computed_hashes(state, &imported_rom_ids, staged).await;
    for download in staged {
        complete_staging_operation(state, &download.operation_id).await;
    }

    Ok(RommImportOutcome {
        outcome: "imported",
        remote_rom_id: request.remote_rom_id,
        title: title.to_string(),
        platform_id: platform.id,
        platform_slug: platform.slug.clone(),
        file_count: staged.len(),
        downloaded_bytes,
        hash_signals: duplicates::hash_signals(&computed_candidates),
        imported_rom_ids,
        imported_rom_name,
        duplicate: None,
        hash_conflicts,
    })
}

/// Stores the hashes the download already computed on the newly created file rows.
///
/// Without this, the content-hash duplicate rule would be inert for imported files: ingest leaves
/// `hash_status = 'pending'` until an integrity job runs, so a later import of identical bytes
/// under a different name would have nothing to match. The digests cover exactly the bytes that
/// were moved into place, so they are the same values an integrity job would compute.
///
/// A failure here is logged and not fatal: the import itself succeeded, and the integrity job
/// remains able to fill the values in later.
async fn persist_computed_hashes(state: &AppState, rom_ids: &[i64], staged: &[StagedDownload]) {
    for rom_id in rom_ids {
        let rom = match roms::find_by_id(state.db(), *rom_id).await {
            Ok(Some(rom)) => rom,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    ?error,
                    rom_id,
                    "could not reload an imported ROM to store hashes"
                );
                continue;
            }
        };
        for file in &rom.files {
            let Some(download) = staged
                .iter()
                .find(|download| download.file_name == file.file_name)
            else {
                // Generated manifests have no downloaded counterpart.
                continue;
            };
            let hashes = FileHashes {
                crc32: download.hashes.crc32.clone(),
                md5: download.hashes.md5.clone(),
                sha1: download.hashes.sha1.clone(),
                sha256: download.hashes.sha256.clone(),
            };
            if let Err(error) = integrity::save_file_hashes(state.db(), file.id, &hashes).await {
                tracing::warn!(
                    ?error,
                    file_id = file.id,
                    "could not store computed hashes for an imported file"
                );
            }
        }
    }
}

async fn ingest(
    state: &AppState,
    platform_id: i64,
    title: &str,
    staged: &[StagedDownload],
) -> Result<(Vec<i64>, Option<String>), RommSourceError> {
    if let [only] = staged {
        let uploaded = library::finalize_upload(
            state,
            UploadDraft {
                platform_id: Some(platform_id),
                platform_slug: None,
                title: Some(title.to_string()),
                original_file_name: only.file_name.clone(),
                staged_path: only.staged_path.clone(),
                staging_operation_id: only.operation_id.clone(),
                file_size_bytes: only.downloaded_bytes,
            },
        )
        .await
        .map_err(ingest_error)?;
        return Ok((vec![uploaded.rom.id], Some(uploaded.rom.name)));
    }

    let batch = library::finalize_upload_batch(
        state,
        UploadBatchDraft {
            platform_id: Some(platform_id),
            platform_slug: None,
            title: Some(title.to_string()),
            planned_titles: Vec::new(),
            files: staged
                .iter()
                .map(|download| UploadBatchFileDraft {
                    original_file_name: download.file_name.clone(),
                    staged_path: download.staged_path.clone(),
                    staging_operation_id: download.operation_id.clone(),
                    file_size_bytes: download.downloaded_bytes,
                })
                .collect(),
        },
    )
    .await
    .map_err(ingest_error)?;

    Ok((
        batch.roms.iter().map(|rom| rom.id).collect(),
        batch.roms.first().map(|rom| rom.name.clone()),
    ))
}

/// The ingest layer stays authoritative. A collision there is a bug in the checks above, so it is
/// reported as a terminal failure rather than accepted as a renamed second copy.
fn ingest_error(error: LibraryServiceError) -> RommSourceError {
    if matches!(error, LibraryServiceError::NoAvailableFileName) {
        tracing::error!(
            %error,
            "RomM import reached the ingest-layer collision backstop; the duplicate checks missed a local copy"
        );
    }
    RommSourceError::Library(error)
}

fn select_files(
    files: &[RemoteRomFile],
    requested_ids: &[i64],
) -> Result<Vec<RemoteRomFile>, RommSourceError> {
    if requested_ids.is_empty() {
        return Err(RommSourceError::NoFilesSelected);
    }
    let mut selected = Vec::with_capacity(requested_ids.len());
    for id in requested_ids {
        let file = files
            .iter()
            .find(|file| file.id == *id)
            .ok_or(RommSourceError::UnknownRemoteFile)?;
        if selected
            .iter()
            .any(|existing: &RemoteRomFile| existing.id == file.id)
        {
            continue;
        }
        selected.push(file.clone());
    }
    Ok(selected)
}

async fn discard_staged(state: &AppState, staged: &[StagedDownload]) {
    for download in staged {
        FileStore::new().cleanup_path(&download.staged_path).await;
        complete_staging_operation(state, &download.operation_id).await;
    }
}

async fn complete_staging_operation(state: &AppState, operation_id: &str) {
    if let Err(error) = file_operations::complete(state, operation_id).await {
        tracing::warn!(
            ?error,
            operation_id,
            "failed to complete RomM import staging journal"
        );
    }
}

async fn ensure_available_space(
    path: &std::path::Path,
    required_bytes: u64,
    margin_bytes: u64,
) -> Result<(), RommSourceError> {
    let path = path.to_path_buf();
    let available = tokio::task::spawn_blocking(move || fs2::available_space(path))
        .await
        .map_err(|_| RommSourceError::Staging)??;
    if available.saturating_sub(margin_bytes) < required_bytes {
        return Err(RommSourceError::InsufficientStorage);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote_file(id: i64, name: &str) -> RemoteRomFile {
        RemoteRomFile {
            id,
            file_name: name.to_string(),
            file_size_bytes: Some(16),
            crc32: None,
            md5: None,
            sha1: None,
            sha256: None,
        }
    }

    #[test]
    fn selection_validates_ids_against_the_server_side_detail() {
        let files = vec![remote_file(1, "a.bin"), remote_file(2, "b.bin")];
        assert_eq!(select_files(&files, &[2, 1, 2]).unwrap().len(), 2);
        assert!(matches!(
            select_files(&files, &[3]),
            Err(RommSourceError::UnknownRemoteFile)
        ));
        assert!(matches!(
            select_files(&files, &[]),
            Err(RommSourceError::NoFilesSelected)
        ));
    }

    fn staged_download(name: &str, bytes: u64) -> StagedDownload {
        StagedDownload {
            file_name: name.to_string(),
            staged_path: PathBuf::from(format!("/tmp/{name}.part")),
            operation_id: "op".to_string(),
            downloaded_bytes: bytes,
            hashes: ComputedHashes {
                crc32: "0000ffff".to_string(),
                md5: "b".repeat(32),
                sha1: "c".repeat(40),
                sha256: "d".repeat(64),
            },
        }
    }

    #[test]
    fn a_declared_hash_the_download_did_not_produce_is_reported_and_not_fatal() {
        let mut file = remote_file(1, "game.sfc");
        file.sha256 = Some("a".repeat(64));
        file.crc32 = Some("0000ffff".to_string());
        let staged = vec![staged_download("game.sfc", 16)];

        let conflicts = verify_downloads(&[file], &staged).expect("a hash conflict is not fatal");
        // Only the algorithm that disagreed is reported, and it carries both values.
        assert_eq!(
            conflicts,
            vec![RommHashConflict {
                file_name: "game.sfc".to_string(),
                algorithm: "sha256",
                declared: "a".repeat(64),
                computed: "d".repeat(64),
            }]
        );
    }

    #[test]
    fn matching_and_absent_declared_hashes_report_nothing() {
        let mut file = remote_file(1, "game.sfc");
        file.sha256 = Some("d".repeat(64));
        file.md5 = None;
        assert!(
            verify_downloads(&[file], &[staged_download("game.sfc", 16)])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_declared_size_the_download_did_not_produce_is_still_terminal() {
        // A short or overlong body is a truncated or wrong transfer, not a stale remote record.
        let file = remote_file(1, "game.sfc");
        assert!(matches!(
            verify_downloads(&[file], &[staged_download("game.sfc", 15)]),
            Err(RommSourceError::SizeMismatch)
        ));
    }

    #[test]
    fn the_reported_conflict_list_is_bounded() {
        let files = (0..40)
            .map(|index| {
                let mut file = remote_file(index, &format!("game-{index}.sfc"));
                file.sha256 = Some("a".repeat(64));
                file.sha1 = Some("b".repeat(40));
                file
            })
            .collect::<Vec<_>>();
        let staged = (0..40)
            .map(|index| staged_download(&format!("game-{index}.sfc"), 16))
            .collect::<Vec<_>>();
        assert_eq!(
            verify_downloads(&files, &staged).unwrap().len(),
            MAX_REPORTED_HASH_CONFLICTS
        );
    }

    #[test]
    fn romm_metadata_keeps_local_integrity_and_merges_regions_and_languages() {
        let existing = serde_json::json!({
            "integrity": { "status": "verified" },
            "filename": { "title": "Chrono Trigger" },
            "serials": ["SNS-CT-USA"],
            "regions": ["Europe"],
            "languages": ["French"],
            "discarded": true
        });
        let remote = RemoteRomMetadata {
            summary: Some("A time-travel RPG".to_string()),
            regions: vec!["Japan".to_string()],
            value: serde_json::json!({
                "source": "romm",
                "regions": ["Japan"],
                "languages": ["English"],
                "romm": { "remote_rom_id": 7 }
            }),
        };

        let (metadata, regions) = merged_remote_metadata(&existing, &["USA".to_string()], &remote);
        assert_eq!(regions, vec!["Europe", "Japan", "USA"]);
        assert_eq!(
            metadata["languages"],
            serde_json::json!(["English", "French"])
        );
        assert_eq!(metadata["integrity"]["status"], "verified");
        assert_eq!(metadata["romm"]["remote_rom_id"], 7);
        assert!(metadata.get("discarded").is_none());
    }

    #[test]
    fn remote_file_names_are_rejected_before_they_reach_the_filesystem() {
        for hostile in ["../../etc/passwd", "/abs/path.bin", "", "  "] {
            assert!(sanitize_upload_file_name(hostile).is_err(), "{hostile}");
        }
        assert_eq!(
            sanitize_upload_file_name("Sonic (USA).md").unwrap(),
            "Sonic (USA).md"
        );
    }

    #[test]
    fn igdb_fallback_uses_the_sanitized_filename_title() {
        assert_eq!(
            igdb_title_from_file_name("Great Volleyball (USA, Europe).zip").unwrap(),
            "Great Volleyball"
        );
        assert!(igdb_title_from_file_name("../wrong-game.zip").is_err());
    }

    #[test]
    fn igdb_fallback_uses_the_detected_game_name() {
        assert_eq!(
            igdb_detected_name(&json!({ "name": "Great Volleyball" })),
            Some("Great Volleyball")
        );
        assert_eq!(igdb_detected_name(&json!({ "name": "  " })), None);
    }
}
