use axum::{Json, extract::State, http::StatusCode};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
    },
    domain::{api_token::CreatedApiToken, user::User},
    error::ApiError,
    repositories::{api_tokens, users as user_repository},
    services::api_tokens as api_token_service,
    state::AppState,
};

use super::{
    audit::record_api_token_event,
    dto::{ApiTokenResponse, CreateApiTokenRequest, CreatedApiTokenResponse},
    errors::{map_api_token_repository_error, map_api_token_service_error, map_user_error},
};

pub async fn list_api_tokens(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApiTokenResponse>>, ApiError> {
    let tokens = api_tokens::list(state.db())
        .await
        .map_err(map_api_token_repository_error)?;
    let tokens = tokens.into_iter().map(ApiTokenResponse::from).collect();

    Ok(Json(tokens))
}

pub async fn create_api_token(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<CreatedApiTokenResponse>), ApiError> {
    let target_user = resolve_token_user(&state, &request, _actor.public_user().id).await?;
    let name =
        api_token_service::validate_name(&request.name).map_err(map_api_token_service_error)?;
    let scopes = api_token_service::normalize_scopes(request.scopes, &target_user)
        .map_err(map_api_token_service_error)?;
    let expires_at = normalize_expires_at(request.expires_at.as_deref())?;
    let raw_token = api_token_service::generate_token();
    let token_hash = api_token_service::hash_token(&raw_token);
    let token_prefix = api_token_service::token_prefix(&raw_token);

    let record = api_tokens::create(
        state.db(),
        api_tokens::CreateApiTokenParams {
            user_id: target_user.id,
            name: &name,
            token_hash: &token_hash,
            token_prefix: &token_prefix,
            scopes: &scopes,
            expires_at: expires_at.as_deref(),
        },
    )
    .await
    .map_err(map_api_token_repository_error)?;

    let created = CreatedApiToken {
        token: raw_token,
        record,
    };
    record_api_token_event(
        &state,
        Some(_actor.public_user().id),
        "api_tokens.created",
        &created.record,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreatedApiTokenResponse::from(created)),
    ))
}

pub async fn revoke_api_token(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<ApiTokenResponse>, ApiError> {
    let token = api_tokens::revoke(state.db(), id)
        .await
        .map_err(map_api_token_repository_error)?;
    record_api_token_event(
        &state,
        Some(_actor.public_user().id),
        "api_tokens.revoked",
        &token,
    )
    .await;

    Ok(Json(ApiTokenResponse::from(token)))
}

async fn resolve_token_user(
    state: &AppState,
    request: &CreateApiTokenRequest,
    actor_user_id: i64,
) -> Result<User, ApiError> {
    match (request.user_id, request.username.as_deref()) {
        (Some(user_id), Some(username)) => {
            let by_id = user_repository::find_by_id(state.db(), user_id)
                .await
                .map_err(map_user_error)?
                .ok_or_else(|| ApiError::not_found("user not found"))?;
            let by_username = user_repository::find_by_username(state.db(), username)
                .await
                .map_err(map_user_error)?
                .ok_or_else(|| ApiError::not_found("user not found"))?;

            if by_id.id != by_username.id {
                return Err(ApiError::bad_request(
                    "user_id and username refer to different users",
                ));
            }

            Ok(by_id)
        }
        (Some(user_id), None) => user_repository::find_by_id(state.db(), user_id)
            .await
            .map_err(map_user_error)?
            .ok_or_else(|| ApiError::not_found("user not found")),
        (None, Some(username)) => user_repository::find_by_username(state.db(), username)
            .await
            .map_err(map_user_error)?
            .ok_or_else(|| ApiError::not_found("user not found")),
        (None, None) => user_repository::find_by_id(state.db(), actor_user_id)
            .await
            .map_err(map_user_error)?
            .ok_or_else(|| ApiError::not_found("user not found")),
    }
}

fn normalize_expires_at(expires_at: Option<&str>) -> Result<Option<String>, ApiError> {
    let Some(expires_at) = expires_at.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };

    let parsed = DateTime::parse_from_rfc3339(expires_at)
        .map_err(|_| ApiError::bad_request("expires_at must be an RFC3339 timestamp"))?
        .with_timezone(&Utc);

    if parsed <= Utc::now() {
        return Err(ApiError::bad_request("expires_at must be in the future"));
    }

    Ok(Some(parsed.to_rfc3339_opts(SecondsFormat::Millis, true)))
}
