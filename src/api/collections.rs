use std::collections::{BTreeSet, HashMap};

use axum::{
    Extension, Json,
    extract::{FromRequestParts, RawQuery, Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};

use super::{
    assets,
    auth::{AdminUser, AuthenticatedUser},
    extractors::{ApiJson, ApiPath},
    romm,
};
use crate::{
    error::ApiError,
    repositories::collections::{self, Collection},
    state::AppState,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SaveCollection {
    name: String,
    slug: String,
    #[serde(default)]
    lan_party_zone: bool,
    rom_ids: Vec<i64>,
}

impl SaveCollection {
    fn validate(&mut self) -> Result<(), ApiError> {
        self.name = self.name.trim().to_string();
        self.slug = self.slug.trim().to_ascii_lowercase();
        if self.name.is_empty() || self.name.chars().count() > 100 {
            return Err(ApiError::bad_request(
                "collection name must be between 1 and 100 characters",
            ));
        }
        if self.slug.is_empty()
            || self.slug.len() > 80
            || !self
                .slug
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || self.slug.starts_with('-')
            || self.slug.ends_with('-')
        {
            return Err(ApiError::bad_request(
                "URL name must use 1 to 80 lowercase letters, digits or hyphens, starting and ending with a letter or digit",
            ));
        }
        if ["api", "admin", "assets", "public", "setup", "healthz"].contains(&self.slug.as_str()) {
            return Err(ApiError::bad_request("that URL name is reserved by Teatro"));
        }
        if self.rom_ids.len() > 10_000 || self.rom_ids.iter().any(|id| *id <= 0) {
            return Err(ApiError::bad_request("select at most 10000 valid game IDs"));
        }
        self.rom_ids = self
            .rom_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        Ok(())
    }
}

#[derive(Serialize)]
pub(super) struct CollectionDetail {
    #[serde(flatten)]
    collection: Collection,
    rom_ids: Vec<i64>,
}

pub(super) async fn list(State(state): State<AppState>) -> Result<Json<Vec<Collection>>, ApiError> {
    Ok(Json(
        collections::list(state.db())
            .await
            .map_err(database_error)?,
    ))
}

pub(super) async fn create(
    _admin: AdminUser,
    State(state): State<AppState>,
    ApiJson(mut input): ApiJson<SaveCollection>,
) -> Result<(StatusCode, Json<Collection>), ApiError> {
    input.validate()?;
    collections::save(
        state.db(),
        None,
        &input.name,
        &input.slug,
        input.lan_party_zone,
        &input.rom_ids,
    )
    .await
    .map_err(database_error)?;
    let collection = find(&state, &input.slug).await?;
    Ok((StatusCode::CREATED, Json(collection)))
}

pub(super) async fn update(
    _admin: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(mut input): ApiJson<SaveCollection>,
) -> Result<Json<Collection>, ApiError> {
    input.validate()?;
    collections::save(
        state.db(),
        Some(id),
        &input.name,
        &input.slug,
        input.lan_party_zone,
        &input.rom_ids,
    )
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::not_found("collection not found"))?;
    Ok(Json(find(&state, &input.slug).await?))
}

pub(super) async fn delete(
    _admin: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<StatusCode, ApiError> {
    if !collections::delete(state.db(), id)
        .await
        .map_err(database_error)?
    {
        return Err(ApiError::not_found("collection not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn find(state: &AppState, slug: &str) -> Result<Collection, ApiError> {
    collections::find_by_slug(state.db(), slug)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("collection not found"))
}

// This layer is installed only on the explicit collection read/download routes.
// Ordinary API routes never accept anonymous collection access.
pub(super) async fn require_access(
    State(state): State<AppState>,
    ApiPath(params): ApiPath<HashMap<String, String>>,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let collection = find(&state, &params["slug"]).await?;
    let (mut parts, body) = request.into_parts();
    if !collection.lan_party_zone {
        AuthenticatedUser::from_request_parts(&mut parts, &state).await?;
    }
    if let Some(id) = params.get("id") {
        let id = id
            .parse::<i64>()
            .map_err(|_| ApiError::bad_request("invalid game ID"))?;
        if !collections::contains(state.db(), collection.id, id)
            .await
            .map_err(database_error)?
        {
            return Err(ApiError::not_found("game not found in collection"));
        }
    }
    parts.extensions.insert(collection);
    let mut response = next.run(Request::from_parts(parts, body)).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

pub(super) async fn detail(
    State(state): State<AppState>,
    Extension(collection): Extension<Collection>,
) -> Result<Json<CollectionDetail>, ApiError> {
    let rom_ids = collections::rom_ids(state.db(), collection.id)
        .await
        .map_err(database_error)?;
    Ok(Json(CollectionDetail {
        collection,
        rom_ids,
    }))
}

pub(super) async fn roms(
    State(state): State<AppState>,
    Extension(collection): Extension<Collection>,
    RawQuery(query): RawQuery,
) -> Result<Json<romm::RomListResponse>, ApiError> {
    romm::list_roms_scoped(&state, query.as_deref(), Some(collection.id)).await
}

pub(super) async fn rom_detail(
    state: State<AppState>,
    ApiPath((_, id)): ApiPath<(String, i64)>,
) -> Result<Json<romm::RomResponse>, ApiError> {
    romm::rom_detail(state, ApiPath(id)).await
}

pub(super) async fn file_ticket(
    state: State<AppState>,
    ApiPath((_, id, file_id)): ApiPath<(String, i64, i64)>,
) -> Result<Response, ApiError> {
    romm::create_rom_file_ticket(state, ApiPath((id, file_id))).await
}

pub(super) async fn archive_ticket(
    state: State<AppState>,
    ApiPath((_, id)): ApiPath<(String, i64)>,
) -> Result<Response, ApiError> {
    romm::create_rom_archive_ticket(state, ApiPath(id)).await
}

pub(super) async fn cover(
    State(state): State<AppState>,
    Extension(collection): Extension<Collection>,
    ApiPath((_, path)): ApiPath<(String, String)>,
) -> Result<Response, ApiError> {
    if !collections::contains_cover(state.db(), collection.id, &path)
        .await
        .map_err(database_error)?
    {
        return Err(ApiError::not_found("cover not found in collection"));
    }
    assets::serve_asset(State(state), ApiPath(path)).await
}

fn database_error(error: sqlx::Error) -> ApiError {
    if let sqlx::Error::Database(ref db) = error {
        if db.is_unique_violation() {
            return ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "a collection already uses that URL name",
            );
        }
        if db.is_foreign_key_violation() {
            return ApiError::bad_request(
                "one or more selected games no longer exist; refresh the library",
            );
        }
    }
    tracing::error!(?error, "collection repository operation failed");
    ApiError::internal("collection operation failed")
}
