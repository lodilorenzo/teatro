use std::path::Path;

use crate::{
    domain::workflow::{FileOperationKind, FileOperationState},
    repositories::{file_operations as file_operation_repository, library_roots, roms},
    services::{
        file_operations::{self, UploadOperationPayload},
        ingest::filename,
    },
    state::AppState,
    storage::{file_store::FileStoreError, paths},
};

use super::{
    edit::MAX_MANIFEST_BYTES,
    ingest::{
        apply_planned_titles, build_ingest_plan, prepare_batch_finalization,
        summarize_ingest_errors,
    },
    normalization::{
        available_file_name, is_manifest_file_name, join_relative_path, normalized_title,
        resolve_platform, sanitize_upload_file_name, slugify, unique_slug,
        upload_filename_metadata,
    },
    types::{
        BatchFileOperation, IngestPlan, LibraryServiceError, PlanInputFile, PlannedRomTitle,
        UploadBatchDraft, UploadBatchFileDraft, UploadDraft, UploadPreviewDraft, UploadedBatch,
        UploadedRom,
    },
};

pub async fn finalize_upload(
    state: &AppState,
    draft: UploadDraft,
) -> Result<UploadedRom, LibraryServiceError> {
    let platform =
        resolve_platform(state, draft.platform_id, draft.platform_slug.as_deref()).await?;
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;

    if !root.writable {
        return Err(LibraryServiceError::ReadOnlyRoot);
    }

    let file_size_bytes =
        i64::try_from(draft.file_size_bytes).map_err(|_| LibraryServiceError::UploadTooLarge)?;
    let original_file_name = sanitize_upload_file_name(&draft.original_file_name)?;
    let parsed_filename = filename::parse(&original_file_name);
    if parsed_filename.split_archive.is_some() {
        return Err(LibraryServiceError::UnsupportedSplitArchive);
    }
    let title = normalized_title(draft.title.as_deref(), &original_file_name)?;
    let regions_json = serde_json::to_string(&parsed_filename.regions)?;
    let metadata_json = upload_filename_metadata(&title, &parsed_filename);

    let _root_lock = state.file_store().lock_root(&root_path).await?;
    let slug = unique_slug(state, platform.id, &slugify(&title)).await?;
    let platform_relative = paths::clean_relative_path(&platform.fs_slug)?;
    state
        .file_store()
        .create_dir_all(&root_path, &platform_relative)
        .await?;

    let file_name = available_file_name(
        state,
        root.id,
        &root_path,
        &platform_relative,
        &original_file_name,
    )
    .await?;
    let relative_path = join_relative_path(&platform_relative, &file_name);
    let operation_id = file_operations::new_operation_id();
    let payload = UploadOperationPayload::new(root.id, vec![relative_path.clone()]);
    let payload_json = serde_json::to_string(&payload)?;
    file_operation_repository::create(
        state.db(),
        &operation_id,
        FileOperationKind::Upload,
        &payload_json,
    )
    .await?;
    if let Err(error) = state
        .file_store()
        .move_new(&root_path, &relative_path, &draft.staged_path)
        .await
    {
        file_operations::fail(state, &operation_id, &error.to_string()).await;
        return Err(match error {
            FileStoreError::AlreadyExists => LibraryServiceError::NoAvailableFileName,
            error => error.into(),
        });
    }

    let persisted = roms::create_with_file(
        state.db(),
        roms::CreateRomWithFileParams {
            platform_id: platform.id,
            name: &title,
            slug: &slug,
            root_id: root.id,
            relative_path: &relative_path,
            file_name: &file_name,
            original_file_name: &original_file_name,
            file_size_bytes,
            regions_json: &regions_json,
            metadata_json: metadata_json.as_deref(),
            file_operation_id: &operation_id,
        },
    )
    .await;

    let (rom_id, file_id) = match persisted {
        Ok(ids) => ids,
        Err(error) => {
            recover_upload_persistence_error(
                state,
                &operation_id,
                &root_path,
                std::slice::from_ref(&relative_path),
                &error,
            )
            .await;
            return Err(error.into());
        }
    };
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)?;
    file_operations::complete(state, &operation_id).await?;

    Ok(UploadedRom {
        rom,
        file_id,
        file_name,
        relative_path,
        file_size_bytes,
    })
}

pub async fn preview_upload_batch(
    state: &AppState,
    mut draft: UploadPreviewDraft,
) -> Result<IngestPlan, LibraryServiceError> {
    let platform =
        resolve_platform(state, draft.platform_id, draft.platform_slug.as_deref()).await?;
    if draft.files.is_empty() {
        return Err(LibraryServiceError::EmptyBatch);
    }
    if draft.files.len() > state.config().uploads.max_batch_files {
        return Err(LibraryServiceError::UploadTooLarge);
    }

    let file_names = draft
        .files
        .iter()
        .map(|file| file.original_file_name.clone())
        .collect::<Vec<_>>();
    let existing_file_names =
        roms::existing_file_names(state.db(), platform.id, &file_names).await?;
    let mut skipped_file_names = Vec::new();
    draft.files.retain(|file| {
        if existing_file_names.contains(&file.original_file_name) {
            skipped_file_names.push(file.original_file_name.clone());
            false
        } else {
            true
        }
    });

    let total_bytes = draft
        .files
        .iter()
        .filter_map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    if draft.files.iter().any(|file| {
        file.file_size_bytes
            .is_some_and(|size| size > state.config().max_upload_bytes)
    }) || total_bytes > state.config().uploads.max_batch_bytes
    {
        return Err(LibraryServiceError::UploadTooLarge);
    }

    let inputs = draft
        .files
        .into_iter()
        .enumerate()
        .map(|(index, file)| PlanInputFile {
            index,
            original_file_name: file.original_file_name,
            file_size_bytes: file.file_size_bytes,
            staged_path: None,
            manifest_contents: file.manifest_contents,
        })
        .collect();

    let mut plan = build_ingest_plan(platform.id, &platform.slug, inputs, draft.title);
    plan.warnings.splice(
        0..0,
        skipped_file_names
            .into_iter()
            .map(|file_name| super::types::IngestPlanWarning {
                code: "file_already_exists".to_string(),
                message: "file already exists for this platform and was skipped".to_string(),
                file_name: Some(file_name),
            }),
    );
    Ok(plan)
}

pub async fn finalize_upload_batch(
    state: &AppState,
    draft: UploadBatchDraft,
) -> Result<UploadedBatch, LibraryServiceError> {
    let prepared = prepare_upload_batch(state, draft).await?;
    let _root_lock = state.file_store().lock_root(&prepared.root_path).await?;
    let finalized = prepare_batch_finalization(
        state,
        &prepared.root_path,
        prepared.root.id,
        &prepared.platform,
        &prepared.plan,
        &prepared.inputs,
    )
    .await?;
    let operation_id =
        journal_batch_operations(state, prepared.root.id, &finalized.operations).await?;
    let moved_paths = execute_batch_operations(
        state,
        &prepared.root_path,
        &operation_id,
        &finalized.operations,
    )
    .await?;
    let rom_ids = persist_batch_roms(
        state,
        &prepared.root_path,
        &operation_id,
        &moved_paths,
        finalized.roms,
    )
    .await?;
    let roms = load_uploaded_roms(state, rom_ids).await?;
    file_operations::complete(state, &operation_id).await?;

    Ok(UploadedBatch {
        roms,
        warnings: prepared.plan.warnings,
    })
}

struct PreparedUploadBatch {
    platform: crate::domain::platform::Platform,
    root: crate::domain::library::LibraryRoot,
    root_path: std::path::PathBuf,
    plan: IngestPlan,
    inputs: Vec<PlanInputFile>,
}

async fn prepare_upload_batch(
    state: &AppState,
    draft: UploadBatchDraft,
) -> Result<PreparedUploadBatch, LibraryServiceError> {
    let UploadBatchDraft {
        platform_id,
        platform_slug,
        title,
        planned_titles,
        files,
    } = draft;
    validate_batch_request(&title, &planned_titles, &files)?;
    let platform = resolve_platform(state, platform_id, platform_slug.as_deref()).await?;
    let root_path = state.config().default_library_root.canonicalize()?;
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await?
        .ok_or(LibraryServiceError::DefaultRootMissing)?;
    if !root.writable {
        return Err(LibraryServiceError::ReadOnlyRoot);
    }

    let inputs = build_batch_inputs(state, &root_path, files).await?;
    let mut plan = build_ingest_plan(platform.id, &platform.slug, inputs.clone(), title);
    if !plan.errors.is_empty() {
        return Err(LibraryServiceError::InvalidIngestPlan(
            summarize_ingest_errors(&plan.errors),
        ));
    }
    apply_planned_titles(&mut plan, &planned_titles)?;
    Ok(PreparedUploadBatch {
        platform,
        root,
        root_path,
        plan,
        inputs,
    })
}

fn validate_batch_request(
    title: &Option<String>,
    planned_titles: &[PlannedRomTitle],
    files: &[UploadBatchFileDraft],
) -> Result<(), LibraryServiceError> {
    if files.is_empty() {
        return Err(LibraryServiceError::EmptyBatch);
    }
    if !planned_titles.is_empty()
        && title
            .as_deref()
            .is_some_and(|title| !title.trim().is_empty())
    {
        return Err(LibraryServiceError::InvalidIngestPlan(
            "title and planned_title fields cannot be combined".to_string(),
        ));
    }
    Ok(())
}

async fn build_batch_inputs(
    state: &AppState,
    root_path: &Path,
    files: Vec<UploadBatchFileDraft>,
) -> Result<Vec<PlanInputFile>, LibraryServiceError> {
    let mut inputs = Vec::with_capacity(files.len());
    for (index, file) in files.into_iter().enumerate() {
        let manifest_contents = read_manifest_contents_if_needed(state, root_path, &file).await?;
        inputs.push(PlanInputFile {
            index,
            original_file_name: file.original_file_name,
            file_size_bytes: Some(file.file_size_bytes),
            staged_path: Some(file.staged_path),
            manifest_contents,
        });
    }
    Ok(inputs)
}

async fn journal_batch_operations(
    state: &AppState,
    root_id: i64,
    operations: &[BatchFileOperation],
) -> Result<String, LibraryServiceError> {
    let relative_paths = operations
        .iter()
        .map(batch_operation_path)
        .map(str::to_string)
        .collect();
    let operation_id = file_operations::new_operation_id();
    let payload_json =
        serde_json::to_string(&UploadOperationPayload::new(root_id, relative_paths))?;
    file_operation_repository::create(
        state.db(),
        &operation_id,
        FileOperationKind::Upload,
        &payload_json,
    )
    .await?;
    Ok(operation_id)
}

async fn execute_batch_operations(
    state: &AppState,
    root_path: &Path,
    operation_id: &str,
    operations: &[BatchFileOperation],
) -> Result<Vec<String>, LibraryServiceError> {
    let mut completed_paths = Vec::with_capacity(operations.len());
    for operation in operations {
        if let Err(error) = execute_batch_operation(state, root_path, operation).await {
            cleanup_batch_paths(state, root_path, &completed_paths).await;
            file_operations::fail(state, operation_id, &error.to_string()).await;
            return Err(error.into());
        }
        completed_paths.push(batch_operation_path(operation).to_string());
    }
    Ok(completed_paths)
}

async fn execute_batch_operation(
    state: &AppState,
    root_path: &Path,
    operation: &BatchFileOperation,
) -> Result<(), FileStoreError> {
    match operation {
        BatchFileOperation::Move {
            staged_path,
            relative_path,
        } => state
            .file_store()
            .move_new(root_path, relative_path, staged_path)
            .await
            .map(|_| ()),
        BatchFileOperation::WriteGenerated {
            relative_path,
            contents,
        } => state
            .file_store()
            .write_new_atomic(root_path, relative_path, contents.as_bytes())
            .await
            .map(|_| ()),
    }
}

fn batch_operation_path(operation: &BatchFileOperation) -> &str {
    match operation {
        BatchFileOperation::Move { relative_path, .. }
        | BatchFileOperation::WriteGenerated { relative_path, .. } => relative_path,
    }
}

async fn cleanup_batch_paths(state: &AppState, root_path: &Path, relative_paths: &[String]) {
    for relative_path in relative_paths {
        if let Err(error) = state.file_store().remove(root_path, relative_path).await {
            tracing::warn!(
                ?error,
                ?relative_path,
                "failed to clean up batch-uploaded file after file operation error"
            );
        }
    }
}

async fn persist_batch_roms(
    state: &AppState,
    root_path: &Path,
    operation_id: &str,
    moved_paths: &[String],
    roms_to_create: Vec<roms::CreateGroupedRomParams>,
) -> Result<Vec<i64>, LibraryServiceError> {
    match roms::create_grouped_roms(state.db(), roms_to_create, Some(operation_id)).await {
        Ok(rom_ids) => Ok(rom_ids),
        Err(error) => {
            recover_upload_persistence_error(state, operation_id, root_path, moved_paths, &error)
                .await;
            Err(error.into())
        }
    }
}

async fn load_uploaded_roms(
    state: &AppState,
    rom_ids: Vec<i64>,
) -> Result<Vec<crate::domain::rom::Rom>, LibraryServiceError> {
    let mut uploaded = Vec::with_capacity(rom_ids.len());
    for rom_id in rom_ids {
        uploaded.push(
            roms::find_by_id(state.db(), rom_id)
                .await?
                .ok_or(sqlx::Error::RowNotFound)?,
        );
    }
    Ok(uploaded)
}

async fn recover_upload_persistence_error(
    state: &AppState,
    operation_id: &str,
    root_path: &Path,
    relative_paths: &[String],
    persistence_error: &sqlx::Error,
) {
    match file_operation_repository::state_by_id(state.db(), operation_id).await {
        Ok(Some(FileOperationState::Prepared)) => {
            for relative_path in relative_paths {
                if let Err(remove_error) = state.file_store().remove(root_path, relative_path).await
                {
                    tracing::warn!(
                        ?remove_error,
                        ?relative_path,
                        operation_id,
                        "failed to clean up an uncommitted uploaded file"
                    );
                }
            }
            file_operations::fail(state, operation_id, &persistence_error.to_string()).await;
        }
        Ok(Some(operation_state)) => {
            tracing::error!(
                ?persistence_error,
                operation_id,
                %operation_state,
                "upload persistence returned an error after its journal state advanced; managed files were retained for reconciliation"
            );
        }
        Ok(None) => {
            tracing::error!(
                ?persistence_error,
                operation_id,
                "upload journal disappeared after a persistence error; managed files were retained"
            );
        }
        Err(journal_error) => {
            tracing::error!(
                ?persistence_error,
                ?journal_error,
                operation_id,
                "could not determine upload commit state; managed files were retained for safe recovery"
            );
        }
    }
}

async fn read_manifest_contents_if_needed(
    state: &AppState,
    root_path: &Path,
    file: &UploadBatchFileDraft,
) -> Result<Option<String>, LibraryServiceError> {
    if !is_manifest_file_name(&file.original_file_name) {
        return Ok(None);
    }
    if file.file_size_bytes > MAX_MANIFEST_BYTES {
        return Err(LibraryServiceError::InvalidIngestPlan(format!(
            "manifest {} is too large",
            file.original_file_name
        )));
    }

    let bytes = state
        .file_store()
        .read_managed_path(root_path, &file.staged_path, MAX_MANIFEST_BYTES)
        .await?;
    Ok(Some(String::from_utf8_lossy(&bytes).into_owned()))
}
