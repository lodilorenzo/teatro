use std::path::PathBuf;

use axum::{Json, extract::State, http::StatusCode};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiMultipart},
    },
    error::ApiError,
    services::library::{self, IngestPlan},
    state::AppState,
};

use super::{
    audit::{record_batch_upload_event, record_upload_event},
    dto::{UploadBatchResponse, UploadPreviewRequest, UploadRomResponse},
    errors::map_library_error,
    multipart::{
        complete_staging_operation, discard_staged_upload, read_upload_batch_multipart,
        read_upload_multipart,
    },
};

pub async fn upload_rom(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiMultipart(multipart): ApiMultipart,
) -> Result<(StatusCode, Json<UploadRomResponse>), ApiError> {
    let _upload_permit = state.acquire_upload_permit().await;
    let draft = read_upload_multipart(&state, multipart).await?;
    let staged_path = draft.staged_path.clone();
    let staging_operation_id = draft.staging_operation_id.clone();

    match library::finalize_upload(&state, draft).await {
        Ok(uploaded) => {
            complete_staging_operation(&state, &staging_operation_id).await;
            record_upload_event(&state, Some(_actor.public_user().id), &uploaded).await;
            Ok((StatusCode::CREATED, Json(UploadRomResponse::from(uploaded))))
        }
        Err(error) => {
            discard_staged_upload(&state, &staged_path, &staging_operation_id).await;
            Err(map_library_error(error))
        }
    }
}

pub async fn preview_upload_batch(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<UploadPreviewRequest>,
) -> Result<Json<IngestPlan>, ApiError> {
    let plan = library::preview_upload_batch(&state, request.into())
        .await
        .map_err(map_library_error)?;

    Ok(Json(plan))
}

pub async fn upload_batch(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiMultipart(multipart): ApiMultipart,
) -> Result<(StatusCode, Json<UploadBatchResponse>), ApiError> {
    let _upload_permit = state.acquire_upload_permit().await;
    let draft = read_upload_batch_multipart(&state, multipart).await?;
    let staged_operations: Vec<(PathBuf, String)> = draft
        .files
        .iter()
        .map(|file| (file.staged_path.clone(), file.staging_operation_id.clone()))
        .collect();

    match library::finalize_upload_batch(&state, draft).await {
        Ok(uploaded) => {
            for (_, operation_id) in &staged_operations {
                complete_staging_operation(&state, operation_id).await;
            }
            record_batch_upload_event(&state, Some(_actor.public_user().id), &uploaded).await;
            Ok((
                StatusCode::CREATED,
                Json(UploadBatchResponse::from(uploaded)),
            ))
        }
        Err(error) => {
            for (staged_path, operation_id) in staged_operations {
                discard_staged_upload(&state, &staged_path, &operation_id).await;
            }
            Err(map_library_error(error))
        }
    }
}
