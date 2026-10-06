//! Opt-in, process-local conversion imports. Writers outlive cancellation until joined.
mod chd;
mod rvz;
mod seven_zip;
mod wua;
pub(crate) use chd::PLATFORMS as CHD_PLATFORMS;
pub(crate) use seven_zip::PLATFORMS as SEVEN_ZIP_PLATFORMS;

fn output_format(platform: &str) -> &'static str {
    if platform == "wiiu" {
        "wua"
    } else if matches!(platform, "gc" | "wii") {
        "rvz"
    } else if SEVEN_ZIP_PLATFORMS
        .iter()
        .any(|&(slug, _)| slug == platform)
    {
        "7z"
    } else {
        "chd"
    }
}

struct OutputFile {
    path: std::path::PathBuf,
    name: String,
    bytes: u64,
}

enum Inventory {
    Wua(wua::Inventory),
    Rvz(rvz::Inventory),
    Chd(chd::Inventory),
    SevenZip(seven_zip::Inventory),
}

use crate::{
    domain::workflow::FileOperationKind,
    repositories::library_roots,
    services::{
        file_operations::{self, UploadOperationPayload},
        igdb::{AutomaticMetadataOutcome, IgdbServiceError},
        job_registry::{JobError, JobRegistry, JobReporter},
        library::{self, UploadBatchDraft, UploadBatchFileDraft, UploadDraft},
    },
    state::AppState,
};
use serde::Serialize;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{Notify, OwnedSemaphorePermit, Semaphore},
};
pub(crate) use wua::{InputFile, MAX_FILES};

pub(crate) const STAGING_DIR: &str = ".conversion-staging";
#[derive(Debug, thiserror::Error)]
pub(crate) enum ConversionError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Conversion capacity is full")]
    Busy,
    #[error("Conversion import not found")]
    NotFound,
    #[error("Conversion cancelled or publication has started")]
    Conflict,
    #[error(transparent)]
    Io(#[from] io::Error),
}
#[derive(Clone, Serialize)]
pub(crate) struct ResultSummary {
    pub rom_id: i64,
    pub title: String,
    pub file_name: String,
    pub output_format: &'static str,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub metadata_applied: bool,
    pub metadata_warning: Option<&'static str>,
}
struct Entry {
    owner: i64,
    cancel: Arc<AtomicBool>,
    notify: Arc<Notify>,
    publishing: bool,
    output_format: Option<&'static str>,
}
pub(crate) struct ConversionRegistry {
    jobs: Arc<JobRegistry<ResultSummary>>,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
    capacity: Arc<Semaphore>,
    converter: Arc<Semaphore>,
}
pub(crate) struct Reservation {
    pub id: String,
    pub cancel: Arc<AtomicBool>,
    entry: Arc<Mutex<Entry>>,
    reporter: JobReporter<ResultSummary>,
    _permit: Arc<OwnedSemaphorePermit>,
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.reporter.fail(JobError::new(
            "interrupted",
            "Conversion import was interrupted",
        ));
    }
}
impl ConversionRegistry {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(JobRegistry::new("conv_")),
            entries: Mutex::new(HashMap::new()),
            capacity: Arc::new(Semaphore::new(4)),
            converter: Arc::new(Semaphore::new(1)),
        }
    }
    pub fn reserve(&self, owner: i64) -> Result<Reservation, ConversionError> {
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| ConversionError::Busy)?;
        let (snapshot, reporter) = self.jobs.reserve();
        let cancel = Arc::new(AtomicBool::new(false));
        let entry = Arc::new(Mutex::new(Entry {
            owner,
            cancel: cancel.clone(),
            notify: Arc::new(Notify::new()),
            publishing: false,
            output_format: None,
        }));
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|id, _| self.jobs.snapshot(id).is_ok());
        entries.insert(snapshot.id.clone(), entry.clone());
        let _ = reporter.start();
        let _ = reporter.set_phase("uploading", 1);
        Ok(Reservation {
            id: snapshot.id,
            cancel,
            entry,
            reporter,
            _permit: Arc::new(permit),
        })
    }
    fn entry(&self, owner: i64, id: &str) -> Result<Arc<Mutex<Entry>>, ConversionError> {
        let entry = self
            .entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .ok_or(ConversionError::NotFound)?;
        if entry.lock().unwrap_or_else(|e| e.into_inner()).owner != owner {
            return Err(ConversionError::NotFound);
        }
        Ok(entry)
    }
    pub fn snapshot(&self, owner: i64, id: &str) -> Result<serde_json::Value, ConversionError> {
        let output_format = self
            .entry(owner, id)?
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .output_format;
        let job = self
            .jobs
            .snapshot(id)
            .map_err(|_| ConversionError::NotFound)?;
        Ok(
            serde_json::json!({ "id":job.id, "state":job.state, "phase":job.phase,
            "progress":job.progress.map(|p| serde_json::json!({"current":p.current,"total":p.total,"percent":p.percent})),
            "proposal": null, "output_format":output_format,
            "result":job.result, "error":job.error.map(|e| serde_json::json!({"code":e.code,"message":e.message})),
            "created_at":job.created_at,"updated_at":job.updated_at }),
        )
    }
    pub fn cancel(&self, owner: i64, id: &str) -> Result<(), ConversionError> {
        let entry = self.entry(owner, id)?;
        let entry = entry.lock().unwrap_or_else(|e| e.into_inner());
        if entry.publishing {
            return Err(ConversionError::Conflict);
        }
        entry.cancel.store(true, Ordering::Relaxed);
        entry.notify.notify_one();
        self.jobs
            .cancel(id)
            .map_err(|_| ConversionError::NotFound)?;
        Ok(())
    }
}

pub(crate) async fn workspace(data: &Path) -> io::Result<tempfile::TempDir> {
    let root = data.join(STAGING_DIR);
    tokio::fs::create_dir_all(&root).await?;
    if tokio::fs::symlink_metadata(&root)
        .await?
        .file_type()
        .is_symlink()
    {
        return Err(io::Error::other("Unsafe conversion staging root"));
    }
    tempfile::Builder::new().prefix("import-").tempdir_in(root)
}
pub(crate) async fn cleanup_abandoned(data: &Path) -> io::Result<()> {
    let root = data.join(STAGING_DIR);
    let meta = match tokio::fs::symlink_metadata(&root).await {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(io::Error::other("Unsafe conversion staging root"));
    }
    let mut entries = tokio::fs::read_dir(root).await?;
    while let Some(entry) = entries.next_entry().await? {
        if entry.file_name().to_string_lossy().starts_with("import-") {
            let kind = entry.file_type().await?;
            if kind.is_symlink() || !kind.is_dir() {
                return Err(io::Error::other("Unsafe conversion workspace"));
            }
            tokio::fs::remove_dir_all(entry.path()).await?;
        }
    }
    Ok(())
}
pub(crate) fn validate_path(path: &str) -> Result<(), ConversionError> {
    wua::safe_path(path).map_err(|_| {
        ConversionError::Invalid("File paths must be safe relative ASCII paths, at most 240 bytes")
    })
}

pub(crate) fn launch(
    state: AppState,
    mut reservation: Reservation,
    workspace: tempfile::TempDir,
    inputs: Vec<InputFile>,
    title: String,
    platform: &'static str,
) {
    reservation
        .entry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .output_format = Some(output_format(platform));
    let workspace = Arc::new(workspace);
    tokio::spawn(async move {
        let result = run(
            &state,
            &mut reservation,
            &workspace,
            inputs,
            title,
            platform,
        )
        .await;
        // A cancelled blocking worker has already been joined before this workspace can drop.
        let _ = tokio::task::spawn_blocking(move || drop(workspace)).await;
        match result {
            Ok(summary) => {
                let owner = reservation
                    .entry
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .owner;
                let metadata = serde_json::json!({"format":summary.output_format,"input_bytes":summary.input_bytes,"output_bytes":summary.output_bytes}).to_string();
                let _ = crate::repositories::audit::record(
                    state.db(),
                    crate::repositories::audit::AuditEvent {
                        actor_user_id: Some(owner),
                        action: "rom.convert",
                        entity_type: Some("rom"),
                        entity_id: Some(summary.rom_id),
                        metadata_json: Some(&metadata),
                    },
                )
                .await;
                let _ = reservation.reporter.succeed(summary);
            }
            Err(error) => {
                let message = match error {
                    ConversionError::Invalid(message) => message,
                    _ => "Conversion import failed. Check the library before retrying",
                };
                let _ = reservation
                    .reporter
                    .fail(JobError::new("conversion_failed", message));
            }
        }
    });
}
async fn run(
    state: &AppState,
    reservation: &mut Reservation,
    workspace: &Arc<tempfile::TempDir>,
    inputs: Vec<InputFile>,
    title: String,
    platform: &'static str,
) -> Result<ResultSummary, ConversionError> {
    let root = workspace.path().join("input");
    reservation
        .reporter
        .set_phase("inspecting", 2)
        .map_err(|_| ConversionError::Conflict)?;
    let inspection_state = state.clone();
    let inspection_workspace = workspace.clone();
    let inspection_permit = reservation._permit.clone();
    let inventory = tokio::task::spawn_blocking(move || {
        let _server_instance = inspection_state;
        let _workspace = inspection_workspace;
        let _preparation_permit = inspection_permit;
        if platform == "wiiu" {
            wua::inspect(&root, &inputs).map(Inventory::Wua)
        } else if matches!(platform, "gc" | "wii") {
            rvz::inspect(&root, &inputs, platform).map(Inventory::Rvz)
        } else if output_format(platform) == "7z" {
            seven_zip::inspect(&root, &inputs, platform).map(Inventory::SevenZip)
        } else {
            chd::inspect(&root, &inputs, platform).map(Inventory::Chd)
        }
    })
    .await
    .map_err(|_| ConversionError::Invalid("Game inspection failed"))?
    .map_err(|_| {
        if output_format(platform) == "7z" {
            return ConversionError::Invalid("7z compression requires one uncompressed single-file ROM for the selected non-disc platform");
        }
        ConversionError::Invalid(match platform {
            "gc" => "Select one valid plain GameCube ISO/GCM image; Wii and legacy images are not supported",
            "wii" => "Select one valid Wii ISO or single-file WBFS image; GameCube, split WBFS and legacy images are not supported",
            "wiiu" => "Select complete matching Wii U code/content/meta trees with one base game",
            _ => "Select one complete CHD-compatible game: matching ISO, single-data-track IMG or raw 2352-byte BINARY CUE/BIN discs, optional M3U and matching SBI. Audio or mixed-mode IMG needs its CUE. Unsupported track directives and INDEX 00 are rejected",
        })
    })?;
    let (input_bytes, input_files, format) = match &inventory {
        Inventory::Wua(value) => (value.bytes, value.files.len(), "wua"),
        Inventory::Rvz(value) => (value.bytes, 1, "rvz"),
        Inventory::Chd(value) => (value.bytes, 1, "chd"),
        Inventory::SevenZip(value) => (value.bytes, 1, "7z"),
    };
    reservation
        .reporter
        .set_phase("queued_conversion", 4)
        .map_err(|_| ConversionError::Conflict)?;
    let notify = reservation
        .entry
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .notify
        .clone();
    let codec_permit = tokio::select! {
        result = state.conversions().converter.clone().acquire_owned() => result.map_err(|_| ConversionError::Busy)?,
        _ = notify.notified() => return Err(ConversionError::Conflict),
    };
    if reservation.cancel.load(Ordering::Relaxed) {
        return Err(ConversionError::Conflict);
    }
    let limit = state.config().uploads.max_batch_bytes;
    let margin = state.config().uploads.free_space_margin_bytes;
    if fs2::available_space(workspace.path())?
        < limit
            .min(
                input_bytes
                    .saturating_add(input_files as u64 * 4096)
                    .saturating_add(1024 * 1024),
            )
            .saturating_add(margin)
    {
        return Err(ConversionError::Invalid(
            "Not enough free space for conversion",
        ));
    }
    reservation
        .reporter
        .set_phase("compressing", 5)
        .map_err(|_| ConversionError::Conflict)?;
    let _ = reservation.reporter.set_progress(0, input_bytes);
    let output = workspace.path().join(format!("game.{format}"));
    let worker_output = output.clone();
    let cancel = reservation.cancel.clone();
    let reporter = reservation.reporter.clone();
    let phase_reporter = reporter.clone();
    let writer_state = state.clone();
    let writer_workspace = workspace.clone();
    let writer_permit = reservation._permit.clone();
    let runtime = tokio::runtime::Handle::current();
    let mut worker = tokio::task::spawn_blocking(move || {
        // Retain instance ownership, staging and capacity if the supervisor is dropped.
        let _server_instance = writer_state;
        let _workspace = writer_workspace;
        let _preparation_permit = writer_permit;
        let _codec_permit = codec_permit;
        let progress = |current, total| {
            let _ = reporter.set_progress(current, total);
        };
        let phase = |phase| {
            let order = if phase == "compressing" {
                5
            } else if phase == "finalizing_wua" {
                6
            } else {
                7
            };
            let _ = phase_reporter.set_phase(phase, order);
        };
        let bytes = match inventory {
            Inventory::Wua(value) => wua::pack(
                value,
                &worker_output,
                limit,
                margin,
                cancel,
                progress,
                phase,
            ),
            Inventory::Rvz(value) => runtime.block_on(rvz::pack(
                value,
                &worker_output,
                limit,
                margin,
                cancel,
                progress,
                phase,
            )),
            Inventory::SevenZip(value) => seven_zip::pack(
                value,
                &worker_output,
                limit,
                margin,
                cancel,
                progress,
                phase,
            ),
            Inventory::Chd(value) => {
                return runtime.block_on(chd::pack(
                    value,
                    &worker_output,
                    limit,
                    margin,
                    cancel,
                    progress,
                    phase,
                ));
            }
        }?;
        Ok(vec![OutputFile {
            path: worker_output,
            name: String::new(),
            bytes,
        }])
    });
    let result = tokio::select! {
        result = &mut worker => result,
        _ = tokio::time::sleep(Duration::from_secs(3600)) => {
            reservation.cancel.store(true, Ordering::Relaxed);
            let _ = worker.await;
            return Err(ConversionError::Invalid("Conversion exceeded one hour and was stopped"));
        }
    };
    let outputs = result.map_err(|_| ConversionError::Invalid("Conversion worker failed"))??;
    let bytes: u64 = outputs.iter().map(|file| file.bytes).sum();
    if bytes > limit
        || outputs
            .iter()
            .any(|file| file.bytes > state.config().max_upload_bytes)
        || (format == "chd" && outputs.len() > state.config().uploads.max_batch_files)
    {
        return Err(ConversionError::Invalid(
            "Conversion output exceeds upload limits",
        ));
    }
    if reservation.cancel.load(Ordering::Relaxed) {
        return Err(ConversionError::Conflict);
    }
    {
        let mut entry = reservation.entry.lock().unwrap_or_else(|e| e.into_inner());
        if entry.cancel.load(Ordering::Relaxed) {
            return Err(ConversionError::Conflict);
        }
        entry.publishing = true;
    }
    reservation
        .reporter
        .set_phase("publishing", 9)
        .map_err(|_| ConversionError::Conflict)?;
    let root = state.config().default_library_root.canonicalize()?;
    let dbroot = library_roots::find_by_path(state.db(), &root)
        .await
        .map_err(|_| ConversionError::Invalid("Library root unavailable"))?
        .ok_or(ConversionError::Invalid("Library root unavailable"))?;
    if !dbroot.writable {
        return Err(ConversionError::Invalid("Library root is read-only"));
    }
    if fs2::available_space(&root)? < bytes.saturating_add(margin) {
        return Err(ConversionError::Invalid("Not enough free space to publish"));
    }
    let operation = file_operations::new_operation_id();
    let relatives: Vec<_> = (0..outputs.len())
        .map(|index| format!(".uploads/upload-{operation}-{index}.part"))
        .collect();
    file_operations::prepare(
        state,
        &operation,
        FileOperationKind::Upload,
        &UploadOperationPayload::new(dbroot.id, vec![]).with_staged_paths(relatives.clone()),
    )
    .await
    .map_err(|_| ConversionError::Invalid("Could not journal conversion output"))?;
    let publication = async {
        let mut files = Vec::new();
        let mut buffer = vec![0; 64 * 1024];
        for (output, relative) in outputs.into_iter().zip(&relatives) {
            let (staged_path, mut staged) = state
                .file_store()
                .create_staging_file(&root, relative)
                .await
                .map_err(|_| ConversionError::Invalid("Could not stage conversion output"))?;
            let mut source = tokio::fs::File::open(&output.path).await?;
            let mut copied = 0_u64;
            loop {
                let n = source.read(&mut buffer).await?;
                if n == 0 {
                    break;
                }
                copied += n as u64;
                if copied > output.bytes
                    || fs2::available_space(&root)? < (n as u64).saturating_add(margin)
                {
                    return Err(ConversionError::Invalid(
                        "Publication space or size limit reached",
                    ));
                }
                staged.write_all(&buffer[..n]).await?;
            }
            if copied != output.bytes {
                return Err(ConversionError::Invalid("Conversion output changed"));
            }
            staged.flush().await?;
            staged.sync_all().await?;
            drop(staged);
            files.push(UploadBatchFileDraft {
                original_file_name: if output.name.is_empty() {
                    library::archive_file_name(&title, format)
                } else {
                    output.name
                },
                staged_path,
                staging_operation_id: operation.clone(),
                file_size_bytes: output.bytes,
            });
        }
        let (rom_id, file_name) = if format == "chd" {
            let uploaded = library::finalize_upload_batch(
                state,
                UploadBatchDraft {
                    platform_id: None,
                    platform_slug: Some(platform.into()),
                    title: Some(title.clone()),
                    planned_titles: vec![],
                    files,
                },
            )
            .await
            .map_err(|_| ConversionError::Invalid("Could not publish CHD disc set"))?;
            let [rom] = uploaded.roms.as_slice() else {
                return Err(ConversionError::Invalid("CHD output grouping changed"));
            };
            (rom.id, rom.fs_name.clone().unwrap_or_default())
        } else {
            let file = files.pop().unwrap();
            let uploaded = library::finalize_upload(
                state,
                UploadDraft {
                    platform_id: None,
                    platform_slug: Some(platform.into()),
                    title: Some(title.clone()),
                    original_file_name: file.original_file_name,
                    staged_path: file.staged_path,
                    staging_operation_id: operation.clone(),
                    file_size_bytes: file.file_size_bytes,
                },
            )
            .await
            .map_err(|_| ConversionError::Invalid("Could not publish conversion output"))?;
            (uploaded.rom.id, uploaded.file_name)
        };
        Ok(ResultSummary {
            rom_id,
            title,
            file_name,
            output_format: format,
            input_bytes,
            output_bytes: bytes,
            metadata_applied: false,
            metadata_warning: None,
        })
    }
    .await;
    for relative in &relatives {
        state.file_store().cleanup_path(&root.join(relative)).await;
    }
    let _ = file_operations::complete(state, &operation).await;
    let mut summary = publication?;
    // Publication is committed; optional metadata must not turn a saved game into a failed import.
    let _ = reservation.reporter.set_phase("matching_igdb_metadata", 10);
    let metadata = tokio::time::timeout(
        Duration::from_secs(60),
        state
            .igdb_client()
            .auto_apply_metadata(state, summary.rom_id, &summary.title),
    )
    .await;
    match metadata {
        Ok(Ok(AutomaticMetadataOutcome::Applied(applied))) => {
            summary.metadata_applied = true;
            summary.title = applied.rom.name;
        }
        Ok(Ok(AutomaticMetadataOutcome::NoMatch)) => {
            summary.metadata_warning = Some(
                "No IGDB game was found on the selected or any platform. The game was imported without IGDB metadata.",
            );
        }
        Ok(Err(IgdbServiceError::NotConfigured)) => {
            summary.metadata_warning = Some(
                "IGDB credentials are not configured. The game was imported without IGDB metadata.",
            );
        }
        Ok(Err(error)) => {
            tracing::warn!(
                ?error,
                rom_id = summary.rom_id,
                "IGDB metadata lookup failed after conversion import"
            );
            summary.metadata_warning = Some(
                "The IGDB lookup or cover download failed. The game is imported; retry metadata matching from the game editor.",
            );
        }
        Err(_) => {
            summary.metadata_warning = Some(
                "IGDB metadata matching timed out. The game is imported; retry metadata matching from the game editor.",
            );
        }
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ownership_cancellation_and_writer_capacity_are_preserved() {
        let registry = ConversionRegistry::new();
        let reservation = registry.reserve(1).unwrap();
        assert!(registry.snapshot(2, &reservation.id).is_err());
        assert!(registry.cancel(2, &reservation.id).is_err());
        assert!(registry.snapshot(1, &reservation.id).unwrap()["proposal"].is_null());
        registry.cancel(1, &reservation.id).unwrap();
        assert!(reservation.cancel.load(Ordering::Relaxed));
        let interrupted = registry.reserve(1).unwrap();
        let cancellation = interrupted.cancel.clone();
        let writer_permit = interrupted._permit.clone();
        drop(interrupted);
        assert!(cancellation.load(Ordering::Relaxed));
        assert_eq!(registry.capacity.available_permits(), 2);
        drop(writer_permit);
        assert_eq!(registry.capacity.available_permits(), 3);
    }
}
