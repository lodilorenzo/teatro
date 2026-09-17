use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{
    api::{auth::AuthenticatedUser, extractors::ApiJson},
    domain::user::PublicUser,
    error::ApiError,
    repositories::browser_sessions,
    services::api_tokens,
    state::AppState,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    #[serde(default)]
    remember_me: bool,
}

#[derive(Serialize)]
pub struct LoginResponse {
    user: PublicUser,
    token: String,
    expires_at: i64,
}

pub async fn login(
    actor: AuthenticatedUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<LoginRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    // A stolen session or scoped API token must not mint fresh sessions.
    let password_hash = actor
        .verified_password_hash
        .as_deref()
        .ok_or_else(|| ApiError::forbidden("sign in with a username and password"))?;
    let token = api_tokens::generate_token().replacen("teatro_pat_", "teatro_session_", 1);
    let expires_at = Utc::now().timestamp()
        + if request.remember_me {
            30 * 86400
        } else {
            86400
        };
    let created = browser_sessions::create(
        state.db(),
        &api_tokens::hash_token(&token),
        actor.public_user().id,
        password_hash,
        expires_at,
    )
    .await
    .map_err(session_error)?;
    if !created {
        return Err(ApiError::too_many_requests(
            "too many browser sessions; sign out on another device",
            60,
        ));
    }
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(LoginResponse {
            user: actor.public_user().clone(),
            token,
            expires_at,
        }),
    ))
}

pub async fn logout(
    _actor: AuthenticatedUser,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split_once(' '))
        .filter(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("Bearer") && token.trim().starts_with("teatro_session_")
        })
        .map(|(_, token)| token.trim())
        .ok_or_else(|| ApiError::bad_request("browser session required"))?;
    browser_sessions::revoke(state.db(), &api_tokens::hash_token(token))
        .await
        .map_err(session_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn session_error(error: sqlx::Error) -> ApiError {
    tracing::error!(?error, "browser session storage failed");
    ApiError::internal("browser session storage failed")
}
