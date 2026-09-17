use axum::{
    Json,
    body::Bytes,
    extract::{RawQuery, State},
};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
        romm::RomResponse,
    },
    error::ApiError,
    repositories::{platforms, roms as rom_repository},
    services::{igdb, library},
    state::AppState,
};

use super::{
    audit::record_update_event,
    dto::{
        BulkDeleteRomsResponse, DeleteRomResponse, LibraryStatsResponse, RomFilesResponse,
        UpdateRomRequest,
    },
    errors::{bytes_error, map_database_error, map_igdb_error, map_library_error},
    query::{BulkDeleteConfirmQuery, DeleteRomQuery},
};

pub async fn update_rom(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(request): ApiJson<UpdateRomRequest>,
) -> Result<Json<RomResponse>, ApiError> {
    let rom = library::update_rom(&state, id, request.into())
        .await
        .map_err(map_library_error)?;

    record_update_event(&state, Some(_actor.public_user().id), &rom).await;

    Ok(Json(RomResponse::from(rom)))
}

pub async fn update_cover(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    body: Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Result<Json<RomResponse>, ApiError> {
    let body = body.map_err(bytes_error)?;
    let rom = igdb::replace_uploaded_cover(&state, id, body.to_vec())
        .await
        .map_err(map_igdb_error)?;

    record_update_event(&state, Some(actor.public_user().id), &rom).await;

    Ok(Json(RomResponse::from(rom)))
}

pub async fn rom_files(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<RomFilesResponse>, ApiError> {
    let rom = rom_repository::find_by_id(state.db(), id)
        .await
        .map_err(map_database_error)?
        .ok_or_else(|| ApiError::not_found("ROM not found"))?;
    let groups = rom_repository::file_groups_for_rom(state.db(), id)
        .await
        .map_err(map_database_error)?;
    let dependencies = rom_repository::file_dependencies_for_rom(state.db(), id)
        .await
        .map_err(map_database_error)?;

    Ok(Json(RomFilesResponse::new(
        rom.id,
        groups,
        rom.files,
        dependencies,
    )))
}

pub async fn delete_platform_roms(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<BulkDeleteRomsResponse>, ApiError> {
    let confirm = BulkDeleteConfirmQuery::parse(raw_query.as_deref())?;

    let platform = platforms::find_by_id(state.db(), id)
        .await
        .map_err(map_database_error)?
        .ok_or_else(|| ApiError::not_found("platform not found"))?;
    if confirm.confirm != platform.slug {
        return Err(ApiError::bad_request(format!(
            "confirm must exactly match platform slug '{}'",
            platform.slug
        )));
    }

    let outcome =
        library::delete_roms_for_platform(&state, platform.id, Some(_actor.public_user().id))
            .await
            .map_err(map_library_error)?;

    Ok(Json(BulkDeleteRomsResponse::from(outcome)))
}

pub async fn delete_all_roms(
    _actor: AdminUser,
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<BulkDeleteRomsResponse>, ApiError> {
    let confirm = BulkDeleteConfirmQuery::parse(raw_query.as_deref())?;
    if confirm.confirm != "DELETE ALL" {
        return Err(ApiError::bad_request(
            "confirm must exactly match DELETE ALL",
        ));
    }

    let outcome = library::delete_all_roms(&state, Some(_actor.public_user().id))
        .await
        .map_err(map_library_error)?;

    Ok(Json(BulkDeleteRomsResponse::from(outcome)))
}

pub async fn delete_rom(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<DeleteRomResponse>, ApiError> {
    let query = DeleteRomQuery::parse(raw_query.as_deref())?;

    let outcome = library::delete_rom(
        &state,
        id,
        query.delete_files,
        Some(_actor.public_user().id),
    )
    .await
    .map_err(map_library_error)?;

    Ok(Json(DeleteRomResponse::from(outcome)))
}

pub async fn stats(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<LibraryStatsResponse>, ApiError> {
    let stats = library::stats(&state).await.map_err(map_library_error)?;

    Ok(Json(LibraryStatsResponse::from(stats)))
}
