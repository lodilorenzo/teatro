use axum::{Json, extract::State, http::StatusCode};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiMultipart, ApiPath},
    },
    domain::integrity::{DatSource, IntegrityJob, RomIntegrityReport},
    error::ApiError,
    services::integrity::{self as integrity_service, DatImportOutcome},
    state::AppState,
};

use super::{
    audit::{record_dat_import_event, record_integrity_job_event},
    dto::CreateIntegrityJobRequest,
    errors::map_integrity_error,
    multipart::read_dat_multipart,
};

pub async fn list_dats(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DatSource>>, ApiError> {
    let sources = integrity_service::list_dat_sources(&state)
        .await
        .map_err(map_integrity_error)?;
    Ok(Json(sources))
}

pub async fn import_dat(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiMultipart(multipart): ApiMultipart,
) -> Result<(StatusCode, Json<DatImportOutcome>), ApiError> {
    let _upload_permit = state.acquire_upload_permit().await;
    let upload = read_dat_multipart(multipart).await?;
    let outcome = integrity_service::import_logiqx_dat(&state, &upload.file_name, &upload.bytes)
        .await
        .map_err(map_integrity_error)?;
    record_dat_import_event(&state, Some(_actor.public_user().id), &outcome).await;

    Ok((StatusCode::CREATED, Json(outcome)))
}

pub async fn list_integrity_jobs(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<IntegrityJob>>, ApiError> {
    let jobs = integrity_service::list_jobs(&state)
        .await
        .map_err(map_integrity_error)?;
    Ok(Json(jobs))
}

pub async fn start_integrity_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateIntegrityJobRequest>,
) -> Result<(StatusCode, Json<IntegrityJob>), ApiError> {
    let job = integrity_service::start_hash_job(state.clone(), request.rom_id, request.force)
        .await
        .map_err(map_integrity_error)?;
    record_integrity_job_event(&state, Some(_actor.public_user().id), &job).await;

    Ok((StatusCode::ACCEPTED, Json(job)))
}

pub async fn integrity_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<IntegrityJob>, ApiError> {
    let job = integrity_service::find_job(&state, id)
        .await
        .map_err(map_integrity_error)?;
    Ok(Json(job))
}

pub async fn rom_integrity(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<RomIntegrityReport>, ApiError> {
    let report = integrity_service::rom_report(&state, id)
        .await
        .map_err(map_integrity_error)?;
    Ok(Json(report))
}
