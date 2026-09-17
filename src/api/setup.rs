use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    api::{admin::errors::map_user_error, extractors::ApiJson},
    domain::user::PublicUser,
    error::ApiError,
    repositories::{audit, users},
    state::AppState,
};

#[derive(Debug, Serialize)]
pub struct SetupStatus {
    required: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateInitialAdminRequest {
    username: String,
    password: String,
}

pub async fn status(State(state): State<AppState>) -> Result<Json<SetupStatus>, ApiError> {
    let required = !users::has_admin(state.db()).await.map_err(map_user_error)?;
    Ok(Json(SetupStatus { required }))
}

pub async fn create_initial_admin(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateInitialAdminRequest>,
) -> Result<(StatusCode, Json<PublicUser>), ApiError> {
    if users::has_admin(state.db()).await.map_err(map_user_error)? {
        return Err(ApiError::conflict("initial setup is already complete"));
    }

    super::validate_user_request_username(&state, &request.username)?;
    let password_hash = super::hash_user_password(&state, &request.password).await?;
    let Some(user) = users::create_initial_admin(state.db(), &request.username, &password_hash)
        .await
        .map_err(map_user_error)?
    else {
        return Err(ApiError::conflict("initial setup is already complete"));
    };

    let metadata = serde_json::json!({
        "username": user.username,
        "role": user.role.as_str(),
    })
    .to_string();
    if let Err(error) = audit::record(
        state.db(),
        audit::AuditEvent {
            actor_user_id: None,
            action: "users.initial_admin_created",
            entity_type: Some("user"),
            entity_id: Some(user.id),
            metadata_json: Some(&metadata),
        },
    )
    .await
    {
        tracing::warn!(?error, "failed to record initial admin creation");
    }

    Ok((StatusCode::CREATED, Json(user.public())))
}
