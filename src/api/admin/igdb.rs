use axum::{
    Json,
    extract::{RawQuery, State},
};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
        romm::RomResponse,
    },
    error::ApiError,
    repositories::{igdb_settings, platforms},
    services::igdb::{AppliedIgdbMetadata, IgdbGameCandidate},
    state::AppState,
};

use super::{
    audit::{record_igdb_metadata_event, record_igdb_settings_event},
    dto::{
        ApplyIgdbMetadataRequest, ApplyIgdbMetadataResponse, IgdbSettingsResponse,
        IgdbStatusResponse, SaveIgdbSettingsRequest,
    },
    errors::{map_database_error, map_igdb_error},
    query::{IgdbSearchQuery, normalize_igdb_setting},
};

pub async fn igdb_status(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<IgdbStatusResponse>, ApiError> {
    let config = state.igdb_config().await.map_err(map_database_error)?;

    Ok(Json(IgdbStatusResponse::from(
        state.igdb_client().status(&config),
    )))
}

pub async fn get_igdb_settings(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<IgdbSettingsResponse>, ApiError> {
    Ok(Json(igdb_settings_response(&state).await?))
}

pub async fn save_igdb_settings(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<SaveIgdbSettingsRequest>,
) -> Result<Json<IgdbSettingsResponse>, ApiError> {
    let stored = igdb_settings::load(state.db())
        .await
        .map_err(map_database_error)?;
    let client_id = normalize_igdb_setting("client_id", &request.client_id, 256)?;
    let client_secret = match request
        .client_secret
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => Some(normalize_igdb_setting("client_secret", value, 2048)?),
        None => stored
            .as_ref()
            .and_then(|settings| settings.client_secret.clone()),
    };

    igdb_settings::save(
        state.db(),
        igdb_settings::SaveIgdbSettingsParams {
            client_id: Some(&client_id),
            client_secret: client_secret.as_deref(),
        },
    )
    .await
    .map_err(map_database_error)?;

    state.igdb_client().clear_token_cache();
    let response = igdb_settings_response(&state).await?;
    record_igdb_settings_event(
        &state,
        Some(_actor.public_user().id),
        &response,
        "igdb.settings_saved",
    )
    .await;

    Ok(Json(response))
}

pub async fn clear_igdb_settings(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<IgdbSettingsResponse>, ApiError> {
    igdb_settings::clear(state.db())
        .await
        .map_err(map_database_error)?;

    state.igdb_client().clear_token_cache();
    let response = igdb_settings_response(&state).await?;
    record_igdb_settings_event(
        &state,
        Some(_actor.public_user().id),
        &response,
        "igdb.settings_cleared",
    )
    .await;

    Ok(Json(response))
}

pub async fn igdb_search(
    _actor: AdminUser,
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<Vec<IgdbGameCandidate>>, ApiError> {
    let query = IgdbSearchQuery::parse(raw_query.as_deref())?;
    let config = state.igdb_config().await.map_err(map_database_error)?;
    let platform = match query.platform_slug.as_deref() {
        Some(slug) => Some(
            platforms::find_by_slug(state.db(), slug)
                .await
                .map_err(map_database_error)?
                .ok_or_else(|| ApiError::bad_request("platform was not found"))?,
        ),
        None => None,
    };
    let platform = platform
        .as_ref()
        .map(|platform| (platform.slug.as_str(), platform.display_name.as_str()));
    if query.require_platform_match && platform.is_none() {
        return Err(ApiError::bad_request(
            "platform is required when require_platform_match is true",
        ));
    }

    let results = state
        .igdb_client()
        .search(
            &config,
            &query.q,
            query.limit,
            platform,
            query.require_platform_match,
        )
        .await
        .map_err(map_igdb_error)?;

    Ok(Json(results))
}

pub async fn apply_igdb_metadata(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(request): ApiJson<ApplyIgdbMetadataRequest>,
) -> Result<Json<ApplyIgdbMetadataResponse>, ApiError> {
    let outcome = state
        .igdb_client()
        .apply_metadata(&state, id, request.selected_match, request.cache_cover)
        .await
        .map_err(map_igdb_error)?;

    record_igdb_metadata_event(&state, Some(_actor.public_user().id), &outcome).await;

    Ok(Json(ApplyIgdbMetadataResponse::from(outcome)))
}

async fn igdb_settings_response(state: &AppState) -> Result<IgdbSettingsResponse, ApiError> {
    let stored = igdb_settings::load(state.db())
        .await
        .map_err(map_database_error)?;
    let config = state.igdb_config().await.map_err(map_database_error)?;
    let status = state.igdb_client().status(&config);
    let stored_client_secret_configured = stored
        .as_ref()
        .and_then(|settings| settings.client_secret.as_deref())
        .is_some();
    let env_client_id_configured = state.config().igdb.client_id.is_some();
    let env_client_secret_configured = state.config().igdb.client_secret.is_some();

    Ok(IgdbSettingsResponse {
        configured: status.configured,
        client_id_configured: status.client_id_configured,
        client_secret_configured: status.client_secret_configured,
        token_cached: status.token_cached,
        client_id_source: credential_source(
            stored
                .as_ref()
                .and_then(|settings| settings.client_id.as_deref())
                .is_some(),
            env_client_id_configured,
        ),
        client_secret_source: credential_source(
            stored_client_secret_configured,
            env_client_secret_configured,
        ),
        updated_at: stored.map(|settings| settings.updated_at),
    })
}

fn credential_source(stored: bool, env: bool) -> &'static str {
    match (stored, env) {
        (true, _) => "database",
        (false, true) => "environment",
        (false, false) => "none",
    }
}

impl From<AppliedIgdbMetadata> for ApplyIgdbMetadataResponse {
    fn from(outcome: AppliedIgdbMetadata) -> Self {
        Self {
            rom: RomResponse::from(outcome.rom),
            cached_covers: outcome.cached_covers,
        }
    }
}
