//! Bounded multipart parsing and recoverable upload staging.
//!
//! Partial request contexts own every staged file so any field, size, disk-space, or
//! framing error can discard earlier artifacts before returning an API error.

use std::path::{Path, PathBuf};

use axum::extract::{Multipart, multipart::Field};
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::{
    domain::workflow::FileOperationKind,
    error::ApiError,
    repositories::library_roots,
    services::{
        file_operations::{self as file_operation_service, UploadOperationPayload},
        gog_import::{self, GogImportDraft, GogImportFileDraft},
        integrity as integrity_service,
        library::{PlannedRomTitle, UploadBatchDraft, UploadBatchFileDraft, UploadDraft},
    },
    state::AppState,
    storage::file_store::FileStore,
};

use super::errors::multipart_error;

const UPLOAD_WRITE_BUFFER_BYTES: usize = 2 * 1024 * 1024;

struct PartialUpload {
    state: AppState,
    platform_id: Option<i64>,
    platform_slug: Option<String>,
    title: Option<String>,
    staged_upload: Option<StagedUpload>,
}

impl PartialUpload {
    fn new(state: &AppState) -> Self {
        Self {
            state: state.clone(),
            platform_id: None,
            platform_slug: None,
            title: None,
            staged_upload: None,
        }
    }
}

struct PartialBatchUpload {
    state: AppState,
    platform_id: Option<i64>,
    platform_slug: Option<String>,
    title: Option<String>,
    planned_titles: Vec<PlannedRomTitle>,
    staged_uploads: Vec<StagedUpload>,
}

impl PartialBatchUpload {
    fn new(state: &AppState) -> Self {
        Self {
            state: state.clone(),
            platform_id: None,
            platform_slug: None,
            title: None,
            planned_titles: Vec::new(),
            staged_uploads: Vec::new(),
        }
    }

    async fn read_field(&mut self, name: &str, field: Field<'_>) -> Result<(), ApiError> {
        match name {
            "platform_id" => self.read_platform_id(field).await,
            "platform_slug" => self.read_platform_slug(field).await,
            "title" | "name" => self.read_title(name, field).await,
            "planned_title" => self.read_planned_title(field).await,
            "file" | "files" => self.read_file(field).await,
            _ => Err(ApiError::bad_request(format!(
                "unexpected multipart field {name:?}"
            ))),
        }
    }

    async fn read_platform_id(&mut self, field: Field<'_>) -> Result<(), ApiError> {
        if self.platform_id.is_some() {
            return Err(ApiError::bad_request(
                "platform_id may only be supplied once",
            ));
        }
        let value = read_text_field(&self.state, field, "platform_id").await?;
        self.platform_id = Some(
            value
                .trim()
                .parse()
                .map_err(|_| ApiError::bad_request("platform_id must be an integer"))?,
        );
        Ok(())
    }

    async fn read_platform_slug(&mut self, field: Field<'_>) -> Result<(), ApiError> {
        if self.platform_slug.is_some() {
            return Err(ApiError::bad_request(
                "platform_slug may only be supplied once",
            ));
        }
        let value = read_text_field(&self.state, field, "platform_slug").await?;
        self.platform_slug = Some(value.trim().to_string());
        Ok(())
    }

    async fn read_title(&mut self, name: &str, field: Field<'_>) -> Result<(), ApiError> {
        if self.title.is_some() {
            return Err(ApiError::bad_request("title may only be supplied once"));
        }
        self.title = Some(read_text_field(&self.state, field, name).await?);
        Ok(())
    }

    async fn read_planned_title(&mut self, field: Field<'_>) -> Result<(), ApiError> {
        if self.planned_titles.len() >= self.state.config().uploads.max_batch_files {
            return Err(ApiError::bad_request("too many planned_title fields"));
        }
        let value = read_text_field(&self.state, field, "planned_title").await?;
        let planned_title = serde_json::from_str(&value).map_err(|_| {
            ApiError::bad_request("planned_title must contain a plan_id and title JSON object")
        })?;
        self.planned_titles.push(planned_title);
        Ok(())
    }

    async fn read_file(&mut self, field: Field<'_>) -> Result<(), ApiError> {
        if self.staged_uploads.len() >= self.state.config().uploads.max_batch_files {
            return Err(ApiError::payload_too_large(
                "upload batch contains too many files",
            ));
        }
        let remaining_bytes = self.remaining_batch_bytes();
        if remaining_bytes == 0 {
            return Err(ApiError::payload_too_large("upload batch is too large"));
        }
        let original_file_name = field
            .file_name()
            .map(str::to_owned)
            .ok_or_else(|| ApiError::bad_request("file field must include a filename"))?;
        let staged = stage_file_field(
            &self.state,
            field,
            original_file_name,
            Some(remaining_bytes),
        )
        .await?;
        self.staged_uploads.push(staged);
        Ok(())
    }

    fn remaining_batch_bytes(&self) -> u64 {
        let staged_bytes = self
            .staged_uploads
            .iter()
            .map(|upload| upload.file_size_bytes)
            .fold(0_u64, u64::saturating_add);
        self.state
            .config()
            .uploads
            .max_batch_bytes
            .saturating_sub(staged_bytes)
    }

    fn into_draft(self) -> Result<UploadBatchDraft, ApiError> {
        if self.staged_uploads.is_empty() {
            return Err(ApiError::bad_request("at least one file field is required"));
        }
        Ok(UploadBatchDraft {
            platform_id: self.platform_id,
            platform_slug: self.platform_slug,
            title: self.title,
            planned_titles: self.planned_titles,
            files: self
                .staged_uploads
                .into_iter()
                .map(|staged| UploadBatchFileDraft {
                    original_file_name: staged.original_file_name,
                    staged_path: staged.path,
                    staging_operation_id: staged.operation_id,
                    file_size_bytes: staged.file_size_bytes,
                })
                .collect(),
        })
    }
}

struct PartialGogImport {
    state: AppState,
    title: Option<String>,
    staged_uploads: Vec<StagedUpload>,
}

impl PartialGogImport {
    fn new(state: &AppState) -> Self {
        Self {
            state: state.clone(),
            title: None,
            staged_uploads: Vec::new(),
        }
    }

    async fn read_field(&mut self, name: &str, field: Field<'_>) -> Result<(), ApiError> {
        match name {
            "title" | "name" => {
                if self.title.is_some() {
                    return Err(ApiError::bad_request("title may only be supplied once"));
                }
                self.title = Some(read_text_field(&self.state, field, name).await?);
                Ok(())
            }
            "file" | "files" => self.read_file(field).await,
            _ => Err(ApiError::bad_request(format!(
                "unexpected multipart field {name:?}"
            ))),
        }
    }

    async fn read_file(&mut self, field: Field<'_>) -> Result<(), ApiError> {
        if self.staged_uploads.len() >= self.state.config().uploads.max_batch_files {
            return Err(ApiError::payload_too_large(
                "GOG setup upload contains too many files",
            ));
        }
        let remaining_bytes = self.remaining_bytes();
        if remaining_bytes == 0 {
            return Err(ApiError::payload_too_large("GOG setup upload is too large"));
        }
        let original_file_name = field
            .file_name()
            .map(str::to_owned)
            .ok_or_else(|| ApiError::bad_request("file field must include a filename"))?;
        gog_import::validate_setup_file_name(&original_file_name)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        let staged = stage_file_field(
            &self.state,
            field,
            original_file_name,
            Some(remaining_bytes),
        )
        .await?;
        self.staged_uploads.push(staged);
        Ok(())
    }

    fn remaining_bytes(&self) -> u64 {
        let staged_bytes = self
            .staged_uploads
            .iter()
            .map(|upload| upload.file_size_bytes)
            .fold(0_u64, u64::saturating_add);
        self.state
            .config()
            .uploads
            .max_batch_bytes
            .saturating_sub(staged_bytes)
    }

    fn into_draft(self) -> Result<GogImportDraft, ApiError> {
        let title = self
            .title
            .ok_or_else(|| ApiError::bad_request("title field is required"))?;
        if self.staged_uploads.is_empty() {
            return Err(ApiError::bad_request(
                "at least one setup file field is required",
            ));
        }
        Ok(GogImportDraft {
            title,
            files: self
                .staged_uploads
                .into_iter()
                .map(|staged| GogImportFileDraft {
                    original_file_name: staged.original_file_name,
                    staged_path: staged.path,
                    staging_operation_id: staged.operation_id,
                    file_size_bytes: staged.file_size_bytes,
                })
                .collect(),
        })
    }
}

struct StagedUpload {
    original_file_name: String,
    path: PathBuf,
    operation_id: String,
    file_size_bytes: u64,
}

pub(super) struct DatUpload {
    pub(super) file_name: String,
    pub(super) bytes: Vec<u8>,
}

pub(super) async fn read_dat_multipart(mut multipart: Multipart) -> Result<DatUpload, ApiError> {
    let mut upload = None;

    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        if field.name() != Some("file") {
            return Err(ApiError::bad_request("unexpected multipart field"));
        }
        if upload.is_some() {
            return Err(ApiError::bad_request("only one DAT file is supported"));
        }
        let file_name = field
            .file_name()
            .map(str::to_owned)
            .ok_or_else(|| ApiError::bad_request("DAT file must include a filename"))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
            if bytes.len().saturating_add(chunk.len()) > integrity_service::MAX_DAT_BYTES {
                return Err(ApiError::payload_too_large("DAT upload is too large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        upload = Some(DatUpload { file_name, bytes });
    }

    upload.ok_or_else(|| ApiError::bad_request("DAT file field is required"))
}

pub(super) async fn read_upload_multipart(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<UploadDraft, ApiError> {
    let mut upload = PartialUpload::new(state);

    loop {
        let next_field = match multipart.next_field().await {
            Ok(next_field) => next_field,
            Err(error) => return fail_upload(upload, multipart_error(error)).await,
        };
        let Some(field) = next_field else {
            break;
        };
        let Some(name) = field.name().map(str::to_owned) else {
            return fail_upload(
                upload,
                ApiError::bad_request("multipart fields must include a name"),
            )
            .await;
        };

        match name.as_str() {
            "platform_id" => {
                if upload.platform_id.is_some() {
                    return fail_upload(
                        upload,
                        ApiError::bad_request("platform_id may only be supplied once"),
                    )
                    .await;
                }
                let value = match read_text_field(state, field, "platform_id").await {
                    Ok(value) => value,
                    Err(error) => return fail_upload(upload, error).await,
                };
                let platform_id = match value.trim().parse::<i64>() {
                    Ok(platform_id) => platform_id,
                    Err(_) => {
                        return fail_upload(
                            upload,
                            ApiError::bad_request("platform_id must be an integer"),
                        )
                        .await;
                    }
                };
                upload.platform_id = Some(platform_id);
            }
            "platform_slug" => {
                if upload.platform_slug.is_some() {
                    return fail_upload(
                        upload,
                        ApiError::bad_request("platform_slug may only be supplied once"),
                    )
                    .await;
                }
                let value = match read_text_field(state, field, "platform_slug").await {
                    Ok(value) => value,
                    Err(error) => return fail_upload(upload, error).await,
                };
                upload.platform_slug = Some(value.trim().to_string());
            }
            "title" | "name" => {
                if upload.title.is_some() {
                    return fail_upload(
                        upload,
                        ApiError::bad_request("title may only be supplied once"),
                    )
                    .await;
                }
                let value = match read_text_field(state, field, name.as_str()).await {
                    Ok(value) => value,
                    Err(error) => return fail_upload(upload, error).await,
                };
                upload.title = Some(value);
            }
            "file" => {
                if upload.staged_upload.is_some() {
                    return fail_upload(
                        upload,
                        ApiError::bad_request("only one file field is supported"),
                    )
                    .await;
                }

                let original_file_name = field
                    .file_name()
                    .map(str::to_owned)
                    .ok_or_else(|| ApiError::bad_request("file field must include a filename"))?;
                let staged_upload =
                    stage_file_field(state, field, original_file_name, None).await?;
                upload.staged_upload = Some(staged_upload);
            }
            _ => {
                return fail_upload(
                    upload,
                    ApiError::bad_request(format!("unexpected multipart field {name:?}")),
                )
                .await;
            }
        }
    }

    let Some(staged_upload) = upload.staged_upload else {
        return Err(ApiError::bad_request("file field is required"));
    };

    Ok(UploadDraft {
        platform_id: upload.platform_id,
        platform_slug: upload.platform_slug,
        title: upload.title,
        original_file_name: staged_upload.original_file_name,
        staged_path: staged_upload.path,
        staging_operation_id: staged_upload.operation_id,
        file_size_bytes: staged_upload.file_size_bytes,
    })
}

pub(super) async fn read_upload_batch_multipart(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<UploadBatchDraft, ApiError> {
    let mut upload = PartialBatchUpload::new(state);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return fail_batch_upload(upload, multipart_error(error)).await,
        };
        let Some(name) = field.name().map(str::to_owned) else {
            return fail_batch_upload(
                upload,
                ApiError::bad_request("multipart fields must include a name"),
            )
            .await;
        };
        if let Err(error) = upload.read_field(&name, field).await {
            return fail_batch_upload(upload, error).await;
        }
    }
    upload.into_draft()
}

pub(super) async fn read_gog_import_multipart(
    state: &AppState,
    mut multipart: Multipart,
) -> Result<GogImportDraft, ApiError> {
    let mut upload = PartialGogImport::new(state);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => return fail_gog_import(upload, multipart_error(error)).await,
        };
        let Some(name) = field.name().map(str::to_owned) else {
            return fail_gog_import(
                upload,
                ApiError::bad_request("multipart fields must include a name"),
            )
            .await;
        };
        if let Err(error) = upload.read_field(&name, field).await {
            return fail_gog_import(upload, error).await;
        }
    }
    if upload.title.is_none() {
        return fail_gog_import(upload, ApiError::bad_request("title field is required")).await;
    }
    if upload.staged_uploads.is_empty() {
        return fail_gog_import(
            upload,
            ApiError::bad_request("at least one setup file field is required"),
        )
        .await;
    }
    upload.into_draft()
}

async fn read_text_field(
    state: &AppState,
    mut field: Field<'_>,
    name: &str,
) -> Result<String, ApiError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        if bytes.len().saturating_add(chunk.len()) > state.config().uploads.max_text_field_bytes {
            return Err(ApiError::bad_request(format!("{name} is too long")));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ApiError::bad_request(format!("{name} must contain UTF-8 text")))
}

async fn fail_upload<T>(upload: PartialUpload, error: ApiError) -> Result<T, ApiError> {
    if let Some(staged_upload) = upload.staged_upload {
        discard_staged_upload(
            &upload.state,
            &staged_upload.path,
            &staged_upload.operation_id,
        )
        .await;
    }

    Err(error)
}

async fn fail_batch_upload<T>(upload: PartialBatchUpload, error: ApiError) -> Result<T, ApiError> {
    for staged_upload in upload.staged_uploads {
        discard_staged_upload(
            &upload.state,
            &staged_upload.path,
            &staged_upload.operation_id,
        )
        .await;
    }

    Err(error)
}

async fn fail_gog_import<T>(upload: PartialGogImport, error: ApiError) -> Result<T, ApiError> {
    for staged_upload in upload.staged_uploads {
        discard_staged_upload(
            &upload.state,
            &staged_upload.path,
            &staged_upload.operation_id,
        )
        .await;
    }
    Err(error)
}

async fn stage_file_field(
    state: &AppState,
    mut field: Field<'_>,
    original_file_name: String,
    aggregate_remaining: Option<u64>,
) -> Result<StagedUpload, ApiError> {
    if original_file_name.trim().is_empty() {
        return Err(ApiError::bad_request("file field must include a filename"));
    }

    let root = state
        .config()
        .default_library_root
        .canonicalize()
        .map_err(|error| {
            tracing::error!(
                ?error,
                "failed to canonicalize library root for upload staging"
            );
            ApiError::internal("failed to stage upload")
        })?;
    let root_record = library_roots::find_by_path(state.db(), &root)
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to resolve upload staging root");
            ApiError::internal("failed to stage upload")
        })?
        .ok_or_else(|| ApiError::internal("failed to stage upload"))?;
    let operation_id = file_operation_service::new_operation_id();
    let staging_relative_path = format!(".uploads/upload-{operation_id}.part");
    let payload = UploadOperationPayload::new(root_record.id, Vec::new())
        .with_staged_paths(vec![staging_relative_path.clone()]);
    if let Err(error) =
        file_operation_service::prepare(state, &operation_id, FileOperationKind::Upload, &payload)
            .await
    {
        tracing::error!(?error, "failed to journal upload staging file");
        return Err(ApiError::internal("failed to stage upload"));
    }
    let (path, file) = match state
        .file_store()
        .create_staging_file(&root, &staging_relative_path)
        .await
    {
        Ok(staging_file) => staging_file,
        Err(error) => {
            complete_staging_operation(state, &operation_id).await;
            tracing::error!(?error, "failed to create upload staging file");
            return Err(ApiError::internal("failed to stage upload"));
        }
    };
    // Multipart body frames are typically much smaller than an efficient disk write.
    // Coalescing them also avoids dispatching one blocking filesystem write per frame.
    let mut file = BufWriter::with_capacity(UPLOAD_WRITE_BUFFER_BYTES, file);
    let staging_dir = path
        .parent()
        .expect("journaled upload staging paths always have a parent");

    let max_file_bytes = aggregate_remaining
        .map(|remaining| remaining.min(state.config().max_upload_bytes))
        .unwrap_or(state.config().max_upload_bytes);
    let disk_check_interval = state.config().uploads.disk_check_interval_bytes.max(1);
    let mut file_size_bytes = 0_u64;
    let mut reserved_write_bytes = 0_u64;
    loop {
        let chunk = match field.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                drop(file);
                discard_staged_upload(state, &path, &operation_id).await;
                return Err(multipart_error(error));
            }
        };
        file_size_bytes = file_size_bytes.saturating_add(chunk.len() as u64);
        if file_size_bytes > max_file_bytes {
            drop(file);
            discard_staged_upload(state, &path, &operation_id).await;
            let message = if aggregate_remaining.is_some() {
                "upload exceeds the per-file or aggregate batch limit"
            } else {
                "upload exceeds TEATRO_MAX_UPLOAD_BYTES"
            };
            return Err(ApiError::payload_too_large(message));
        }

        let chunk_bytes = chunk.len() as u64;
        if reserved_write_bytes < chunk_bytes {
            match reserve_disk_space_for_writes(
                staging_dir,
                disk_check_interval,
                chunk_bytes,
                state.config().uploads.free_space_margin_bytes,
            )
            .await
            {
                Ok(reservation) => reserved_write_bytes = reservation,
                Err(error) => {
                    drop(file);
                    discard_staged_upload(state, &path, &operation_id).await;
                    return Err(error);
                }
            }
        }

        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            discard_staged_upload(state, &path, &operation_id).await;
            tracing::error!(?error, ?path, "failed to write upload chunk");
            return Err(ApiError::internal("failed to stage upload"));
        }
        reserved_write_bytes = reserved_write_bytes.saturating_sub(chunk_bytes);
    }

    if let Err(error) = file.flush().await {
        drop(file);
        discard_staged_upload(state, &path, &operation_id).await;
        tracing::error!(?error, ?path, "failed to flush upload staging file");
        return Err(ApiError::internal("failed to stage upload"));
    }
    drop(file);

    Ok(StagedUpload {
        original_file_name,
        path,
        operation_id,
        file_size_bytes,
    })
}

fn bounded_disk_reservation(
    available_space: u64,
    safety_margin_bytes: u64,
    check_interval_bytes: u64,
    next_chunk_bytes: u64,
) -> Option<u64> {
    let writable_space = available_space.saturating_sub(safety_margin_bytes);
    (writable_space >= next_chunk_bytes)
        .then(|| writable_space.min(check_interval_bytes.max(next_chunk_bytes)))
}

async fn reserve_disk_space_for_writes(
    staging_dir: &Path,
    check_interval_bytes: u64,
    next_chunk_bytes: u64,
    safety_margin_bytes: u64,
) -> Result<u64, ApiError> {
    let staging_dir = staging_dir.to_path_buf();
    let available_space = tokio::task::spawn_blocking(move || fs2::available_space(staging_dir))
        .await
        .map_err(|error| {
            tracing::error!(?error, "failed to join disk-space check task");
            ApiError::internal("failed to check available disk space")
        })?
        .map_err(|error| {
            tracing::error!(?error, "failed to check available disk space");
            ApiError::internal("failed to check available disk space")
        })?;

    bounded_disk_reservation(
        available_space,
        safety_margin_bytes,
        check_interval_bytes,
        next_chunk_bytes,
    )
    .ok_or_else(|| ApiError::insufficient_storage("not enough disk space for upload"))
}

pub(super) async fn complete_staging_operation(state: &AppState, operation_id: &str) {
    if let Err(error) = file_operation_service::complete(state, operation_id).await {
        tracing::warn!(
            ?error,
            operation_id,
            "failed to complete upload staging journal"
        );
    }
}

pub(super) async fn discard_staged_upload(state: &AppState, path: &Path, operation_id: &str) {
    cleanup_file_if_exists(path).await;
    complete_staging_operation(state, operation_id).await;
}

async fn cleanup_file_if_exists(path: &Path) {
    FileStore::new().cleanup_path(path).await;
}

#[cfg(test)]
mod multipart_tests {
    use super::bounded_disk_reservation;

    #[test]
    fn disk_reservations_preserve_the_margin_without_rejecting_small_uploads() {
        let mib = 1024 * 1024;
        assert_eq!(
            bounded_disk_reservation(600 * mib, 512 * mib, 64 * mib, 16 * 1024),
            Some(64 * mib)
        );
        assert_eq!(
            bounded_disk_reservation(513 * mib, 512 * mib, 64 * mib, 16 * 1024),
            Some(mib)
        );
        assert_eq!(
            bounded_disk_reservation(512 * mib + 1023, 512 * mib, 64 * mib, 1024),
            None
        );
    }
}
