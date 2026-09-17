//! Recoverable ROM deletion orchestration.
//!
//! Deletion resolves and locks all roots, reconciles covers, journals trash moves,
//! commits rows and audit metadata atomically, then removes recoverable trash.

use std::{
    collections::{BTreeSet, HashMap},
    path::{Component, Path, PathBuf},
};

use serde_json::Value;

use crate::{
    domain::{
        rom::Rom,
        workflow::{FileOperationKind, FileOperationState},
    },
    repositories::{audit, file_operations as file_operation_repository, platforms, roms},
    services::file_operations::{self, DeleteOperationEntry, DeleteOperationPayload},
    state::AppState,
    storage::file_store::RootMutationGuard,
};

use super::types::{
    BulkDeleteRomsOutcome, BulkDeleteScope, DeleteRomOutcome, LibraryServiceError,
    SidecarCleanupFile,
};

struct ManagedDeleteEntry {
    root_id: i64,
    root_path: PathBuf,
    original_relative_path: String,
    trash_relative_path: String,
    directory: bool,
    reported_rom_files: Vec<String>,
}

pub async fn delete_rom(
    state: &AppState,
    rom_id: i64,
    delete_files: bool,
    actor_user_id: Option<i64>,
) -> Result<DeleteRomOutcome, LibraryServiceError> {
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(LibraryServiceError::RomNotFound)?;
    let (deleted_files, missing_files) = delete_rom_records(
        state,
        std::slice::from_ref(&rom),
        delete_files,
        actor_user_id,
        "roms.deleted",
        Some(rom.id),
        None,
    )
    .await?;

    Ok(DeleteRomOutcome {
        rom,
        delete_files,
        deleted_files,
        missing_files,
    })
}

pub async fn delete_roms_for_platform(
    state: &AppState,
    platform_id: i64,
    actor_user_id: Option<i64>,
) -> Result<BulkDeleteRomsOutcome, LibraryServiceError> {
    let platform = platforms::find_by_id(state.db(), platform_id)
        .await?
        .ok_or(LibraryServiceError::PlatformNotFound)?;
    let rom_ids = roms::ids_by_platform(state.db(), platform.id).await?;
    bulk_delete_roms(
        state,
        BulkDeleteScope::Platform(platform),
        rom_ids,
        actor_user_id,
    )
    .await
}

pub async fn delete_all_roms(
    state: &AppState,
    actor_user_id: Option<i64>,
) -> Result<BulkDeleteRomsOutcome, LibraryServiceError> {
    let rom_ids = roms::all_ids(state.db()).await?;
    bulk_delete_roms(state, BulkDeleteScope::All, rom_ids, actor_user_id).await
}

async fn bulk_delete_roms(
    state: &AppState,
    scope: BulkDeleteScope,
    rom_ids: Vec<i64>,
    actor_user_id: Option<i64>,
) -> Result<BulkDeleteRomsOutcome, LibraryServiceError> {
    let mut roms_to_delete = Vec::with_capacity(rom_ids.len());
    for rom_id in rom_ids {
        if let Some(rom) = roms::find_by_id(state.db(), rom_id).await? {
            roms_to_delete.push(rom);
        }
    }

    let scope_json = match &scope {
        BulkDeleteScope::Platform(platform) => serde_json::json!({
            "kind": "platform",
            "platform_id": platform.id,
            "platform_slug": platform.slug,
            "platform_display_name": platform.display_name,
        }),
        BulkDeleteScope::All => serde_json::json!({"kind": "all"}),
    };
    let (deleted_files, missing_files) = delete_rom_records(
        state,
        &roms_to_delete,
        true,
        actor_user_id,
        "roms.bulk_deleted",
        None,
        Some(scope_json),
    )
    .await?;

    Ok(BulkDeleteRomsOutcome {
        scope,
        deleted_roms: roms_to_delete,
        delete_files: true,
        deleted_files,
        missing_files,
    })
}

pub(super) async fn delete_unindexed_files(
    state: &AppState,
    root_id: i64,
    root_lock: &RootMutationGuard,
    files: &[SidecarCleanupFile],
    actor_user_id: Option<i64>,
) -> Result<(), LibraryServiceError> {
    let operation_id = file_operations::new_operation_id();
    let entries = files
        .iter()
        .enumerate()
        .map(|(index, file)| ManagedDeleteEntry {
            root_id,
            root_path: root_lock.root().to_path_buf(),
            original_relative_path: file.relative_path.clone(),
            trash_relative_path: format!(".trash/{operation_id}/{index}"),
            directory: false,
            reported_rom_files: Vec::new(),
        })
        .collect::<Vec<_>>();
    let payload = DeleteOperationPayload::new(
        Vec::new(),
        entries
            .iter()
            .map(|entry| DeleteOperationEntry {
                root_id: entry.root_id,
                original_relative_path: entry.original_relative_path.clone(),
                trash_relative_path: entry.trash_relative_path.clone(),
                directory: false,
            })
            .collect(),
    )
    .with_root_ids(vec![root_id]);
    file_operation_repository::create(
        state.db(),
        &operation_id,
        FileOperationKind::Delete,
        &serde_json::to_string(&payload)?,
    )
    .await?;
    move_delete_entries(state, &operation_id, &entries).await?;

    let metadata_json = serde_json::json!({
        "deleted_file_count": files.len(),
        "deleted_file_bytes": files.iter().fold(0_u64, |total, file| total.saturating_add(file.file_size_bytes)),
    })
    .to_string();
    let committed = async {
        let mut tx = state.db().begin().await?;
        audit::record_tx(
            &mut tx,
            audit::AuditEvent {
                actor_user_id,
                action: "library.sidecars_deleted",
                entity_type: Some("library_root"),
                entity_id: Some(root_id),
                metadata_json: Some(&metadata_json),
            },
        )
        .await?;
        file_operation_repository::set_state_tx(
            &mut tx,
            &operation_id,
            FileOperationState::DbCommitted,
        )
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(error) = committed {
        restore_delete_entries(state, &entries).await;
        file_operations::fail(state, &operation_id, &error.to_string()).await;
        return Err(error.into());
    }
    finish_delete_cleanup(state, &operation_id, &entries).await
}

async fn delete_rom_records(
    state: &AppState,
    roms_to_delete: &[Rom],
    delete_files: bool,
    actor_user_id: Option<i64>,
    audit_action: &'static str,
    audit_entity_id: Option<i64>,
    audit_scope: Option<Value>,
) -> Result<(Vec<String>, Vec<String>), LibraryServiceError> {
    let rom_ids: Vec<i64> = roms_to_delete.iter().map(|rom| rom.id).collect();
    if rom_ids.is_empty() || !delete_files {
        if !rom_ids.is_empty() || audit_scope.is_some() {
            let metadata_json =
                delete_audit_metadata(roms_to_delete, delete_files, &[], &[], audit_scope.as_ref());
            commit_delete_rows(
                state,
                &rom_ids,
                actor_user_id,
                audit_action,
                audit_entity_id,
                &metadata_json,
                None,
            )
            .await?;
        }
        return Ok((Vec::new(), Vec::new()));
    }

    let operation_id = file_operations::new_operation_id();
    let preparation =
        prepare_managed_delete(state, roms_to_delete, &rom_ids, &operation_id).await?;
    journal_delete_operation(state, &operation_id, &rom_ids, &preparation).await?;
    move_delete_entries(state, &operation_id, &preparation.entries).await?;

    let deleted_files = preparation
        .entries
        .iter()
        .flat_map(|entry| entry.reported_rom_files.iter().cloned())
        .collect::<Vec<_>>();
    let metadata_json = delete_audit_metadata(
        roms_to_delete,
        true,
        &deleted_files,
        &preparation.missing_files,
        audit_scope.as_ref(),
    );
    if let Err(error) = commit_delete_rows(
        state,
        &rom_ids,
        actor_user_id,
        audit_action,
        audit_entity_id,
        &metadata_json,
        Some(&operation_id),
    )
    .await
    {
        restore_delete_entries(state, &preparation.entries).await;
        file_operations::fail(state, &operation_id, &error.to_string()).await;
        return Err(error);
    }
    finish_delete_cleanup(state, &operation_id, &preparation.entries).await?;
    Ok((deleted_files, preparation.missing_files.clone()))
}

struct DeleteCandidate {
    root_id: i64,
    root_path: PathBuf,
    relative_path: String,
    directory: bool,
    reported_rom_files: Vec<String>,
}

struct DeleteCandidates {
    roots: BTreeSet<PathBuf>,
    root_ids: BTreeSet<i64>,
    entries: Vec<DeleteCandidate>,
}

struct ManagedDeletePreparation {
    entries: Vec<ManagedDeleteEntry>,
    missing_files: Vec<String>,
    journal_root_ids: Vec<i64>,
    _locks: Vec<RootMutationGuard>,
}

async fn prepare_managed_delete(
    state: &AppState,
    roms_to_delete: &[Rom],
    rom_ids: &[i64],
    operation_id: &str,
) -> Result<ManagedDeletePreparation, LibraryServiceError> {
    let asset_root = state.config().asset_root.canonicalize()?;
    let mut candidates = collect_delete_candidates(roms_to_delete, &asset_root)?;
    let locks = lock_delete_roots(state, &candidates.roots).await?;
    let asset_guard = locks
        .iter()
        .find(|guard| guard.root() == asset_root)
        .expect("the asset-root lock is always acquired for file deletion");
    file_operations::reconcile_pending_covers_locked(state, asset_guard)
        .await
        .map_err(|error| LibraryServiceError::FileRecovery(error.to_string()))?;
    candidates.entries = protect_shared_package_folders(state, candidates.entries, rom_ids).await?;

    // Refresh covers only after acquiring the asset lock so a concurrent cover
    // transaction cannot commit an asset between discovery and deletion.
    for (_, cover_paths) in roms::cover_asset_paths_for_rom_ids(state.db(), rom_ids).await? {
        candidates.entries.extend(
            cover_paths
                .into_iter()
                .map(|relative_path| DeleteCandidate {
                    root_id: 0,
                    root_path: asset_root.clone(),
                    relative_path,
                    directory: false,
                    reported_rom_files: Vec::new(),
                }),
        );
    }
    let (entries, missing_files) =
        resolve_delete_entries(state, operation_id, candidates.entries).await?;
    Ok(ManagedDeletePreparation {
        entries,
        missing_files,
        journal_root_ids: candidates.root_ids.into_iter().collect(),
        _locks: locks,
    })
}

fn collect_delete_candidates(
    roms_to_delete: &[Rom],
    asset_root: &Path,
) -> Result<DeleteCandidates, LibraryServiceError> {
    let mut roots = BTreeSet::from([asset_root.to_path_buf()]);
    let mut root_ids = BTreeSet::from([0_i64]);
    let mut candidates = Vec::new();
    for rom in roms_to_delete {
        if let Some(package) = package_folder_candidate(rom)? {
            roots.insert(package.root_path.clone());
            root_ids.insert(package.root_id);
            candidates.push(package);
        } else {
            for file in &rom.files {
                let root_path = file.root_path.canonicalize()?;
                roots.insert(root_path.clone());
                root_ids.insert(file.root_id);
                candidates.push(DeleteCandidate {
                    root_id: file.root_id,
                    root_path,
                    relative_path: file.relative_path.clone(),
                    directory: false,
                    reported_rom_files: vec![file.relative_path.clone()],
                });
            }
        }
        candidates.extend(
            [rom.path_cover_large.clone(), rom.path_cover_small.clone()]
                .into_iter()
                .flatten()
                .map(|relative_path| DeleteCandidate {
                    root_id: 0,
                    root_path: asset_root.to_path_buf(),
                    relative_path,
                    directory: false,
                    reported_rom_files: Vec::new(),
                }),
        );
    }
    Ok(DeleteCandidates {
        roots,
        root_ids,
        entries: candidates,
    })
}

fn package_folder_candidate(rom: &Rom) -> Result<Option<DeleteCandidate>, LibraryServiceError> {
    if rom.files.len() < 2 {
        return Ok(None);
    }
    let first = &rom.files[0];
    let root_path = first.root_path.canonicalize()?;
    let Some(folder) = direct_game_folder(&first.relative_path) else {
        return Ok(None);
    };
    if rom.files.iter().any(|file| {
        file.root_id != first.root_id
            || file.root_path != first.root_path
            || direct_game_folder(&file.relative_path).as_deref() != Some(folder.as_str())
    }) {
        return Ok(None);
    }
    Ok(Some(DeleteCandidate {
        root_id: first.root_id,
        root_path,
        relative_path: folder,
        directory: true,
        reported_rom_files: rom
            .files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect(),
    }))
}

fn direct_game_folder(relative_path: &str) -> Option<String> {
    let path = Path::new(relative_path);
    let components = path.components().collect::<Vec<_>>();
    if components.len() != 3
        || components
            .iter()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    let folder = components[..2]
        .iter()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()?;
    if folder.iter().any(|component| component.starts_with('.')) {
        return None;
    }
    Some(folder.join("/"))
}

async fn protect_shared_package_folders(
    state: &AppState,
    candidates: Vec<DeleteCandidate>,
    rom_ids: &[i64],
) -> Result<Vec<DeleteCandidate>, LibraryServiceError> {
    let mut protected = Vec::new();
    for candidate in candidates {
        if candidate.directory
            && roms::folder_has_files_outside_roms(
                state.db(),
                candidate.root_id,
                &candidate.relative_path,
                rom_ids,
            )
            .await?
        {
            protected.extend(
                candidate
                    .reported_rom_files
                    .into_iter()
                    .map(|relative_path| DeleteCandidate {
                        root_id: candidate.root_id,
                        root_path: candidate.root_path.clone(),
                        relative_path: relative_path.clone(),
                        directory: false,
                        reported_rom_files: vec![relative_path],
                    }),
            );
        } else {
            protected.push(candidate);
        }
    }
    Ok(protected)
}

async fn lock_delete_roots(
    state: &AppState,
    roots: &BTreeSet<PathBuf>,
) -> Result<Vec<RootMutationGuard>, LibraryServiceError> {
    let mut locks = Vec::with_capacity(roots.len());
    for root_path in roots {
        locks.push(state.file_store().lock_root(root_path).await?);
    }
    Ok(locks)
}

async fn resolve_delete_entries(
    state: &AppState,
    operation_id: &str,
    candidates: Vec<DeleteCandidate>,
) -> Result<(Vec<ManagedDeleteEntry>, Vec<String>), LibraryServiceError> {
    let mut merged = Vec::<DeleteCandidate>::new();
    let mut positions: HashMap<(i64, String, bool), usize> = HashMap::new();
    for mut candidate in candidates {
        let key = (
            candidate.root_id,
            candidate.relative_path.clone(),
            candidate.directory,
        );
        if let Some(index) = positions.get(&key).copied() {
            for relative_path in candidate.reported_rom_files.drain(..) {
                if !merged[index].reported_rom_files.contains(&relative_path) {
                    merged[index].reported_rom_files.push(relative_path);
                }
            }
        } else {
            positions.insert(key, merged.len());
            merged.push(candidate);
        }
    }

    let mut entries = Vec::new();
    let mut missing_files = Vec::new();
    for mut candidate in merged {
        if !state
            .file_store()
            .exists(&candidate.root_path, &candidate.relative_path)
            .await?
        {
            missing_files.extend(candidate.reported_rom_files);
            continue;
        }
        if candidate.directory {
            let mut present_files = Vec::new();
            for relative_path in candidate.reported_rom_files {
                if state
                    .file_store()
                    .exists(&candidate.root_path, &relative_path)
                    .await?
                {
                    present_files.push(relative_path);
                } else {
                    missing_files.push(relative_path);
                }
            }
            candidate.reported_rom_files = present_files;
        }
        entries.push(ManagedDeleteEntry {
            root_id: candidate.root_id,
            root_path: candidate.root_path,
            original_relative_path: candidate.relative_path,
            trash_relative_path: format!(".trash/{operation_id}/{}", entries.len()),
            directory: candidate.directory,
            reported_rom_files: candidate.reported_rom_files,
        });
    }
    Ok((entries, missing_files))
}

async fn journal_delete_operation(
    state: &AppState,
    operation_id: &str,
    rom_ids: &[i64],
    preparation: &ManagedDeletePreparation,
) -> Result<(), LibraryServiceError> {
    let entries = preparation
        .entries
        .iter()
        .map(|entry| DeleteOperationEntry {
            root_id: entry.root_id,
            original_relative_path: entry.original_relative_path.clone(),
            trash_relative_path: entry.trash_relative_path.clone(),
            directory: entry.directory,
        })
        .collect();
    let payload = DeleteOperationPayload::new(rom_ids.to_vec(), entries)
        .with_root_ids(preparation.journal_root_ids.clone());
    let payload_json = serde_json::to_string(&payload)?;
    file_operation_repository::create(
        state.db(),
        operation_id,
        FileOperationKind::Delete,
        &payload_json,
    )
    .await?;
    Ok(())
}

async fn move_delete_entries(
    state: &AppState,
    operation_id: &str,
    entries: &[ManagedDeleteEntry],
) -> Result<(), LibraryServiceError> {
    for (moved_count, entry) in entries.iter().enumerate() {
        let moved = if entry.directory {
            state
                .file_store()
                .move_managed_directory(
                    &entry.root_path,
                    &entry.original_relative_path,
                    &entry.trash_relative_path,
                )
                .await
        } else {
            state
                .file_store()
                .move_managed(
                    &entry.root_path,
                    &entry.original_relative_path,
                    &entry.trash_relative_path,
                )
                .await
        };
        if let Err(error) = moved {
            restore_delete_entries(state, &entries[..moved_count]).await;
            file_operations::fail(state, operation_id, &error.to_string()).await;
            return Err(error.into());
        }
    }
    Ok(())
}

async fn finish_delete_cleanup(
    state: &AppState,
    operation_id: &str,
    entries: &[ManagedDeleteEntry],
) -> Result<(), LibraryServiceError> {
    let mut cleanup_error = None;
    for entry in entries {
        let removed = if entry.directory {
            state
                .file_store()
                .remove_directory_all(&entry.root_path, &entry.trash_relative_path)
                .await
        } else {
            state
                .file_store()
                .remove(&entry.root_path, &entry.trash_relative_path)
                .await
        };
        if let Err(error) = removed {
            tracing::warn!(?error, operation_id, "delete trash cleanup remains pending");
            cleanup_error = Some(error.to_string());
        }
    }
    if let Some(error) = cleanup_error {
        file_operation_repository::set_state(
            state.db(),
            operation_id,
            FileOperationState::CleanupPending,
            Some(&error),
        )
        .await?;
    } else {
        file_operations::complete(state, operation_id).await?;
    }
    Ok(())
}

async fn commit_delete_rows(
    state: &AppState,
    rom_ids: &[i64],
    actor_user_id: Option<i64>,
    audit_action: &'static str,
    audit_entity_id: Option<i64>,
    metadata_json: &str,
    operation_id: Option<&str>,
) -> Result<(), LibraryServiceError> {
    let mut tx = state.db().begin().await?;
    let deleted = roms::delete_ids_tx(&mut tx, rom_ids).await?;
    if deleted != rom_ids.len() as u64 {
        return Err(LibraryServiceError::RomNotFound);
    }
    audit::record_tx(
        &mut tx,
        audit::AuditEvent {
            actor_user_id,
            action: audit_action,
            entity_type: Some("rom"),
            entity_id: audit_entity_id,
            metadata_json: Some(metadata_json),
        },
    )
    .await?;
    if let Some(operation_id) = operation_id {
        file_operation_repository::set_state_tx(
            &mut tx,
            operation_id,
            FileOperationState::DbCommitted,
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn delete_audit_metadata(
    roms_to_delete: &[Rom],
    delete_files: bool,
    deleted_files: &[String],
    missing_files: &[String],
    scope: Option<&Value>,
) -> String {
    match scope {
        Some(scope) => serde_json::json!({
            "scope": scope,
            "deleted_rom_count": roms_to_delete.len(),
            "deleted_files": deleted_files,
            "missing_files": missing_files,
        }),
        None => {
            let rom = &roms_to_delete[0];
            serde_json::json!({
                "rom_name": rom.name,
                "platform_slug": rom.platform_slug,
                "delete_files": delete_files,
                "deleted_files": deleted_files,
                "missing_files": missing_files,
            })
        }
    }
    .to_string()
}

async fn restore_delete_entries(state: &AppState, entries: &[ManagedDeleteEntry]) {
    for entry in entries.iter().rev() {
        let restored = if entry.directory {
            state
                .file_store()
                .move_managed_directory(
                    &entry.root_path,
                    &entry.trash_relative_path,
                    &entry.original_relative_path,
                )
                .await
        } else {
            state
                .file_store()
                .move_managed(
                    &entry.root_path,
                    &entry.trash_relative_path,
                    &entry.original_relative_path,
                )
                .await
        };
        if let Err(error) = restored {
            tracing::error!(
                ?error,
                original = %entry.original_relative_path,
                trash = %entry.trash_relative_path,
                "failed to restore delete artifact; startup reconciliation required"
            );
        }
    }
}
