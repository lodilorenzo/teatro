//! Durable recovery state machines for managed uploads, deletes, and cover replacement.
//!
//! The service interprets typed journal rows, acquires every affected root lock, and
//! reconciles filesystem artifacts against committed repository state. Keep transition
//! dispatch centralized here so feature services do not invent recovery behavior.

use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::{
    domain::workflow::{FileOperationKind, FileOperationState},
    error::AppError,
    repositories::{file_operations, library_roots, roms},
    state::AppState,
    storage::file_store::{FileStoreError, RootMutationGuard},
};

const PAYLOAD_VERSION: u32 = 1;
const COMPLETED_HISTORY_TO_RETAIN: i64 = 1_000;
static OPERATION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadOperationPayload {
    pub version: u32,
    pub root_id: i64,
    pub relative_paths: Vec<String>,
    #[serde(default)]
    pub staged_relative_paths: Vec<String>,
}

impl UploadOperationPayload {
    pub fn new(root_id: i64, relative_paths: Vec<String>) -> Self {
        Self {
            version: PAYLOAD_VERSION,
            root_id,
            relative_paths,
            staged_relative_paths: Vec::new(),
        }
    }

    pub fn with_staged_paths(mut self, staged_relative_paths: Vec<String>) -> Self {
        self.staged_relative_paths = staged_relative_paths;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteOperationPayload {
    pub version: u32,
    pub rom_ids: Vec<i64>,
    pub entries: Vec<DeleteOperationEntry>,
    /// Every root the original delete locked, including roots whose files were
    /// already missing and therefore have no artifact entry. Root 0 is assets.
    #[serde(default)]
    pub root_ids: Vec<i64>,
}

impl DeleteOperationPayload {
    pub fn new(rom_ids: Vec<i64>, entries: Vec<DeleteOperationEntry>) -> Self {
        let root_ids = entries
            .iter()
            .map(|entry| entry.root_id)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Self {
            version: PAYLOAD_VERSION,
            rom_ids,
            entries,
            root_ids,
        }
    }

    pub fn with_root_ids(mut self, root_ids: Vec<i64>) -> Self {
        self.root_ids = root_ids
            .into_iter()
            .chain(self.entries.iter().map(|entry| entry.root_id))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteOperationEntry {
    pub root_id: i64,
    pub original_relative_path: String,
    pub trash_relative_path: String,
    #[serde(default)]
    pub directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverOperationPayload {
    pub version: u32,
    pub rom_id: i64,
    pub previous_paths: Vec<String>,
    pub entries: Vec<CoverOperationEntry>,
    #[serde(default)]
    pub obsolete_entries: Vec<CoverOperationEntry>,
    /// New cover saves transition the journal state in the metadata transaction.
    #[serde(default)]
    pub transactional_state: bool,
}

impl CoverOperationPayload {
    pub fn new(
        rom_id: i64,
        previous_paths: Vec<String>,
        entries: Vec<CoverOperationEntry>,
    ) -> Self {
        Self {
            version: PAYLOAD_VERSION,
            rom_id,
            previous_paths,
            entries,
            obsolete_entries: Vec::new(),
            transactional_state: true,
        }
    }

    pub fn with_obsolete_entries(mut self, entries: Vec<CoverOperationEntry>) -> Self {
        self.obsolete_entries = entries;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverOperationEntry {
    pub resource_path: String,
    pub backup_relative_path: Option<String>,
}

pub fn new_operation_id() -> String {
    let sequence = OPERATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("op-{timestamp:x}-{}-{sequence:x}", std::process::id())
}

pub async fn prepare<T: Serialize>(
    state: &AppState,
    id: &str,
    kind: FileOperationKind,
    payload: &T,
) -> Result<(), AppError> {
    let payload_json = serde_json::to_string(payload)?;
    file_operations::create(state.db(), id, kind, &payload_json).await?;
    Ok(())
}

pub async fn complete(state: &AppState, id: &str) -> Result<(), sqlx::Error> {
    file_operations::set_state(state.db(), id, FileOperationState::Completed, None).await
}

pub async fn fail(state: &AppState, id: &str, error: &str) {
    if let Err(journal_error) =
        file_operations::set_state(state.db(), id, FileOperationState::Failed, Some(error)).await
    {
        tracing::error!(
            ?journal_error,
            operation_id = id,
            "failed to mark file operation failed"
        );
    }
}

pub async fn reconcile(state: &AppState) -> Result<(), AppError> {
    for operation in file_operations::pending(state.db()).await? {
        let result = match (operation.kind, operation.state) {
            (
                FileOperationKind::Upload,
                FileOperationState::Prepared | FileOperationState::Failed,
            ) => reconcile_prepared_upload(state, &operation.payload_json)
                .await
                .map(|()| false),
            (
                FileOperationKind::Upload,
                FileOperationState::DbCommitted | FileOperationState::CleanupPending,
            ) => reconcile_committed_upload(state, &operation.payload_json)
                .await
                .map(|()| false),
            (
                FileOperationKind::Delete,
                FileOperationState::Prepared | FileOperationState::Failed,
            ) => reconcile_prepared_delete(state, &operation.payload_json)
                .await
                .map(|()| false),
            (
                FileOperationKind::Delete,
                FileOperationState::DbCommitted | FileOperationState::CleanupPending,
            ) => reconcile_committed_delete(state, &operation.payload_json)
                .await
                .map(|()| false),
            (FileOperationKind::CoverReplace, _) => {
                reconcile_cover_operation(state, &operation.id, &operation.payload_json)
                    .await
                    .map(|()| true)
            }
            _ => {
                tracing::error!(
                    operation_id = %operation.id,
                    kind = %operation.kind,
                    state = %operation.state,
                    "unknown file operation state requires manual repair"
                );
                continue;
            }
        };

        match result {
            Ok(completed_while_locked) => {
                if !completed_while_locked {
                    complete(state, &operation.id).await?;
                }
            }
            Err(error) => {
                let message = error.to_string();
                fail(state, &operation.id, &message).await;
                tracing::error!(
                    operation_id = %operation.id,
                    ?error,
                    "file operation reconciliation failed"
                );
            }
        }
    }

    file_operations::prune_completed(state.db(), COMPLETED_HISTORY_TO_RETAIN).await?;
    Ok(())
}

async fn reconcile_prepared_upload(state: &AppState, payload: &str) -> Result<(), AppError> {
    let payload: UploadOperationPayload = serde_json::from_str(payload)?;
    ensure_version(payload.version)?;
    let root = library_roots::find_by_id(state.db(), payload.root_id)
        .await?
        .ok_or_else(|| {
            AppError::Reconciliation("journal references a missing library root".into())
        })?;
    let _lock = state.file_store().lock_root(&root.root_path).await?;
    for staged_relative_path in &payload.staged_relative_paths {
        state
            .file_store()
            .remove(&root.root_path, staged_relative_path)
            .await?;
    }
    let mut persisted = Vec::with_capacity(payload.relative_paths.len());
    for relative_path in &payload.relative_paths {
        persisted
            .push(roms::relative_path_exists(state.db(), payload.root_id, relative_path).await?);
    }

    if persisted.iter().all(|persisted| !persisted) {
        for relative_path in payload.relative_paths {
            state
                .file_store()
                .remove(&root.root_path, &relative_path)
                .await?;
        }
        return Ok(());
    }
    if persisted.iter().all(|persisted| *persisted) {
        for relative_path in payload.relative_paths {
            if !state
                .file_store()
                .exists(&root.root_path, &relative_path)
                .await?
            {
                return Err(AppError::Reconciliation(format!(
                    "committed upload is missing managed file {relative_path:?}"
                )));
            }
        }
        return Ok(());
    }

    Err(AppError::Reconciliation(
        "prepared upload has a partial set of committed rows".into(),
    ))
}

async fn reconcile_committed_upload(state: &AppState, payload: &str) -> Result<(), AppError> {
    let payload: UploadOperationPayload = serde_json::from_str(payload)?;
    ensure_version(payload.version)?;
    let root = library_roots::find_by_id(state.db(), payload.root_id)
        .await?
        .ok_or_else(|| {
            AppError::Reconciliation("journal references a missing library root".into())
        })?;
    let _lock = state.file_store().lock_root(&root.root_path).await?;
    for staged_relative_path in payload.staged_relative_paths {
        state
            .file_store()
            .remove(&root.root_path, &staged_relative_path)
            .await?;
    }
    for relative_path in payload.relative_paths {
        if !state
            .file_store()
            .exists(&root.root_path, &relative_path)
            .await?
        {
            return Err(AppError::Reconciliation(format!(
                "committed upload is missing managed file {relative_path:?}"
            )));
        }
    }
    Ok(())
}

async fn reconcile_prepared_delete(state: &AppState, payload: &str) -> Result<(), AppError> {
    let payload: DeleteOperationPayload = serde_json::from_str(payload)?;
    ensure_version(payload.version)?;
    let entries = resolve_delete_entries(state, payload.entries).await?;
    let roots = resolve_delete_roots(state, &entries, &payload.root_ids).await?;

    // Every Teatro delete acquires the same canonical root set before committing its
    // database transaction. Keep all of those locks until this operation has either
    // been fully restored or fully discarded, and only then decide which side won.
    let _locks = lock_delete_roots(state, roots).await?;
    if !payload.rom_ids.is_empty() {
        let mut present = Vec::with_capacity(payload.rom_ids.len());
        for rom_id in &payload.rom_ids {
            present.push(roms::find_by_id(state.db(), *rom_id).await?.is_some());
        }
        if present.iter().all(|present| !present) {
            return cleanup_resolved_delete_entries(state, &entries).await;
        }
        if !present.iter().all(|present| *present) {
            return Err(AppError::Reconciliation(
                "prepared bulk delete has a partial set of committed rows".into(),
            ));
        }
    }

    for resolved in &entries {
        let entry = &resolved.entry;
        let root_path = &resolved.root_path;
        let trash_exists = state
            .file_store()
            .exists(root_path, &entry.trash_relative_path)
            .await?;
        let original_exists = state
            .file_store()
            .exists(root_path, &entry.original_relative_path)
            .await?;
        match (trash_exists, original_exists) {
            (true, false) => {
                if entry.directory {
                    state
                        .file_store()
                        .move_managed_directory(
                            root_path,
                            &entry.trash_relative_path,
                            &entry.original_relative_path,
                        )
                        .await?;
                } else {
                    state
                        .file_store()
                        .move_managed(
                            root_path,
                            &entry.trash_relative_path,
                            &entry.original_relative_path,
                        )
                        .await?;
                }
            }
            (false, true) => {}
            (false, false) => {
                return Err(AppError::Reconciliation(format!(
                    "delete artifact is missing from both its original and trash paths: {:?}",
                    entry.original_relative_path
                )));
            }
            (true, true) => {
                return Err(AppError::Reconciliation(
                    "both original and trash delete artifacts exist".into(),
                ));
            }
        }
    }
    Ok(())
}

async fn reconcile_committed_delete(state: &AppState, payload: &str) -> Result<(), AppError> {
    let payload: DeleteOperationPayload = serde_json::from_str(payload)?;
    ensure_version(payload.version)?;
    cleanup_delete_entries(state, payload.entries, &payload.root_ids).await
}

async fn cleanup_delete_entries(
    state: &AppState,
    entries: Vec<DeleteOperationEntry>,
    root_ids: &[i64],
) -> Result<(), AppError> {
    let entries = resolve_delete_entries(state, entries).await?;
    let roots = resolve_delete_roots(state, &entries, root_ids).await?;
    let _locks = lock_delete_roots(state, roots).await?;
    cleanup_resolved_delete_entries(state, &entries).await
}

struct ResolvedDeleteEntry {
    entry: DeleteOperationEntry,
    root_path: PathBuf,
}

async fn resolve_delete_entries(
    state: &AppState,
    entries: Vec<DeleteOperationEntry>,
) -> Result<Vec<ResolvedDeleteEntry>, AppError> {
    let mut resolved = Vec::with_capacity(entries.len());
    for entry in entries {
        let root_path = operation_root_path(state, entry.root_id)
            .await?
            .canonicalize()?;
        resolved.push(ResolvedDeleteEntry { entry, root_path });
    }
    Ok(resolved)
}

async fn resolve_delete_roots(
    state: &AppState,
    entries: &[ResolvedDeleteEntry],
    root_ids: &[i64],
) -> Result<BTreeSet<PathBuf>, AppError> {
    let mut roots: BTreeSet<_> = entries
        .iter()
        .map(|entry| entry.root_path.clone())
        .collect();
    for root_id in root_ids {
        roots.insert(operation_root_path(state, *root_id).await?.canonicalize()?);
    }
    Ok(roots)
}

async fn lock_delete_roots(
    state: &AppState,
    roots: BTreeSet<PathBuf>,
) -> Result<Vec<RootMutationGuard>, AppError> {
    let mut locks = Vec::with_capacity(roots.len());
    for root in roots {
        locks.push(state.file_store().lock_root(&root).await?);
    }
    Ok(locks)
}

async fn cleanup_resolved_delete_entries(
    state: &AppState,
    entries: &[ResolvedDeleteEntry],
) -> Result<(), AppError> {
    for resolved in entries {
        if resolved.entry.directory {
            state
                .file_store()
                .remove_directory_all(&resolved.root_path, &resolved.entry.trash_relative_path)
                .await?;
        } else {
            state
                .file_store()
                .remove(&resolved.root_path, &resolved.entry.trash_relative_path)
                .await?;
        }
    }
    Ok(())
}

async fn operation_root_path(
    state: &AppState,
    root_id: i64,
) -> Result<std::path::PathBuf, AppError> {
    if root_id == 0 {
        return Ok(state.config().asset_root.canonicalize()?);
    }
    library_roots::find_by_id(state.db(), root_id)
        .await?
        .map(|root| root.root_path)
        .ok_or_else(|| AppError::Reconciliation("journal references a missing library root".into()))
}

/// Reconcile every unfinished cover operation while the caller owns the asset lock.
///
/// New cover replacements and deletes call this before inspecting asset paths, so a
/// second Teatro process cannot build a mutation on top of files left by a crashed one.
pub async fn reconcile_pending_covers_locked(
    state: &AppState,
    asset_guard: &RootMutationGuard,
) -> Result<(), AppError> {
    let asset_root = state.config().asset_root.canonicalize()?;
    if asset_guard.root() != asset_root {
        return Err(AppError::Reconciliation(
            "pending covers must be reconciled while holding the asset-root lock".into(),
        ));
    }

    for operation in file_operations::pending(state.db())
        .await?
        .into_iter()
        .filter(|operation| operation.kind == FileOperationKind::CoverReplace)
    {
        if let Err(error) = reconcile_cover_operation_locked(
            state,
            &asset_root,
            &operation.id,
            &operation.payload_json,
        )
        .await
        {
            fail(state, &operation.id, &error.to_string()).await;
            return Err(error);
        }
    }

    Ok(())
}

async fn reconcile_cover_operation(
    state: &AppState,
    operation_id: &str,
    payload_json: &str,
) -> Result<(), AppError> {
    let root = state.config().asset_root.canonicalize()?;
    let lock = state.file_store().lock_root(&root).await?;
    reconcile_cover_operation_locked(state, lock.root(), operation_id, payload_json).await
}

async fn reconcile_cover_operation_locked(
    state: &AppState,
    root: &Path,
    operation_id: &str,
    payload_json: &str,
) -> Result<(), AppError> {
    let payload: CoverOperationPayload = serde_json::from_str(payload_json)?;
    ensure_version(payload.version)?;

    // Another Teatro process may have reconciled this operation before this caller
    // acquired the cross-process asset lock. Always dispatch from the current state.
    let operation_state = file_operations::state_by_id(state.db(), operation_id)
        .await?
        .ok_or_else(|| AppError::Reconciliation("cover journal row disappeared".into()))?;
    if operation_state == FileOperationState::Completed {
        return Ok(());
    }

    let rom_exists = roms::exists(state.db(), payload.rom_id).await?;

    match operation_state {
        FileOperationState::Prepared | FileOperationState::Failed
            if payload.transactional_state && rom_exists =>
        {
            rollback_cover_files(state, root, &payload).await?;
        }
        FileOperationState::Prepared | FileOperationState::Failed
            if payload.transactional_state =>
        {
            discard_cover_files(state, root, &payload).await?;
        }
        FileOperationState::Prepared | FileOperationState::Failed => {
            reconcile_legacy_prepared_cover(state, root, &payload).await?;
        }
        FileOperationState::DbCommitted | FileOperationState::CleanupPending if rom_exists => {
            cleanup_cover_files(state, root, &payload, true).await?;
        }
        FileOperationState::DbCommitted | FileOperationState::CleanupPending => {
            discard_cover_files(state, root, &payload).await?;
        }
        FileOperationState::Completed => return Ok(()),
    }

    // Complete before releasing the asset lock so a waiting process cannot replay
    // a successful rollback from a stale pending-operation snapshot.
    complete(state, operation_id).await?;
    Ok(())
}

async fn reconcile_legacy_prepared_cover(
    state: &AppState,
    root: &Path,
    payload: &CoverOperationPayload,
) -> Result<(), AppError> {
    let current = roms::cover_paths_for_rom(state.db(), payload.rom_id).await?;
    let rom_exists = current.is_some();
    let current_paths: Vec<String> = current
        .into_iter()
        .flat_map(|(large, small)| [large, small])
        .flatten()
        .collect();
    let new_paths: Vec<String> = payload
        .entries
        .iter()
        .map(|entry| entry.resource_path.clone())
        .collect();

    if current_paths == payload.previous_paths && current_paths != new_paths {
        rollback_cover_files(state, root, payload).await?;
        Ok(())
    } else if current_paths == new_paths && current_paths != payload.previous_paths {
        cleanup_cover_files(state, root, payload, rom_exists).await?;
        Ok(())
    } else {
        Err(AppError::Reconciliation(
            "cover operation database state is ambiguous; backups were retained".into(),
        ))
    }
}

/// Remove replacement files and restore every retained cover backup.
///
/// The checks make replay safe when a previous process already completed some or all
/// of the rollback before losing its journal-state update.
pub async fn rollback_cover_files(
    state: &AppState,
    root: &Path,
    payload: &CoverOperationPayload,
) -> Result<(), FileStoreError> {
    for entry in payload.entries.iter().rev() {
        match entry.backup_relative_path.as_deref() {
            Some(backup) => {
                let backup_exists = state.file_store().exists(root, backup).await?;
                let resource_exists = state
                    .file_store()
                    .exists(root, &entry.resource_path)
                    .await?;
                match (backup_exists, resource_exists) {
                    (true, true) => {
                        state
                            .file_store()
                            .remove(root, &entry.resource_path)
                            .await?;
                        state
                            .file_store()
                            .move_managed(root, backup, &entry.resource_path)
                            .await?;
                    }
                    (true, false) => {
                        state
                            .file_store()
                            .move_managed(root, backup, &entry.resource_path)
                            .await?;
                    }
                    // Either this entry had not yet been moved or another process
                    // already restored it while holding the same root lock.
                    (false, true) => {}
                    (false, false) => return Err(missing_cover_artifact(&entry.resource_path)),
                }
            }
            None => {
                state
                    .file_store()
                    .remove(root, &entry.resource_path)
                    .await?;
            }
        }
    }

    for entry in payload.obsolete_entries.iter().rev() {
        let Some(backup) = entry.backup_relative_path.as_deref() else {
            continue;
        };
        let backup_exists = state.file_store().exists(root, backup).await?;
        let resource_exists = state
            .file_store()
            .exists(root, &entry.resource_path)
            .await?;
        match (backup_exists, resource_exists) {
            (true, false) => {
                state
                    .file_store()
                    .move_managed(root, backup, &entry.resource_path)
                    .await?;
            }
            (false, true) => {}
            (true, true) => return Err(ambiguous_cover_artifact(&entry.resource_path)),
            (false, false) => return Err(missing_cover_artifact(&entry.resource_path)),
        }
    }

    Ok(())
}

/// Verify committed replacement files before discarding their recoverable backups.
pub async fn cleanup_cover_files(
    state: &AppState,
    root: &Path,
    payload: &CoverOperationPayload,
    verify_replacements: bool,
) -> Result<(), FileStoreError> {
    if verify_replacements {
        for entry in &payload.entries {
            if !state
                .file_store()
                .exists(root, &entry.resource_path)
                .await?
            {
                return Err(missing_cover_artifact(&entry.resource_path));
            }
        }
    }

    for entry in payload.entries.iter().chain(&payload.obsolete_entries) {
        if let Some(backup) = entry.backup_relative_path.as_deref() {
            state.file_store().remove(root, backup).await?;
        }
    }
    Ok(())
}

async fn discard_cover_files(
    state: &AppState,
    root: &Path,
    payload: &CoverOperationPayload,
) -> Result<(), FileStoreError> {
    for entry in payload.entries.iter().chain(&payload.obsolete_entries) {
        state
            .file_store()
            .remove(root, &entry.resource_path)
            .await?;
        if let Some(backup) = entry.backup_relative_path.as_deref() {
            state.file_store().remove(root, backup).await?;
        }
    }
    Ok(())
}

fn missing_cover_artifact(path: &str) -> FileStoreError {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("cover artifact is missing from both managed paths: {path:?}"),
    )
    .into()
}

fn ambiguous_cover_artifact(path: &str) -> FileStoreError {
    io::Error::other(format!(
        "cover artifact exists at both its original and backup paths: {path:?}"
    ))
    .into()
}

fn ensure_version(version: u32) -> Result<(), AppError> {
    if version == PAYLOAD_VERSION {
        Ok(())
    } else {
        Err(AppError::Reconciliation(format!(
            "unsupported file operation payload version {version}"
        )))
    }
}
