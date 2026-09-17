use axum::{Json, extract::State, http::StatusCode};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
        hash_user_password, validate_user_request_username,
    },
    domain::user::{PublicUser, UserRole},
    error::ApiError,
    repositories::users as user_repository,
    state::AppState,
};

use super::{
    audit::record_user_event,
    dto::{CreateUserRequest, ResetPasswordRequest, SetRoleRequest},
    errors::map_user_error,
};

pub async fn list_users(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PublicUser>>, ApiError> {
    let users = user_repository::list(state.db())
        .await
        .map_err(map_user_error)?;
    let users = users.into_iter().map(|user| user.public()).collect();

    Ok(Json(users))
}

pub async fn create_user(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateUserRequest>,
) -> Result<(StatusCode, Json<PublicUser>), ApiError> {
    validate_user_request_username(&state, &request.username)?;
    let password_hash = hash_user_password(&state, &request.password).await?;
    let user = user_repository::create(
        state.db(),
        &request.username,
        &password_hash,
        request.role.unwrap_or(UserRole::ReadOnly),
    )
    .await
    .map_err(map_user_error)?;

    record_user_event(
        &state,
        Some(_actor.public_user().id),
        "users.created",
        &user,
    )
    .await;

    Ok((StatusCode::CREATED, Json(user.public())))
}

pub async fn reset_password(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(request): ApiJson<ResetPasswordRequest>,
) -> Result<Json<PublicUser>, ApiError> {
    let password_hash = hash_user_password(&state, &request.password).await?;
    let user = user_repository::update_password(state.db(), id, &password_hash)
        .await
        .map_err(map_user_error)?;

    record_user_event(
        &state,
        Some(_actor.public_user().id),
        "users.password_reset",
        &user,
    )
    .await;

    Ok(Json(user.public()))
}

pub async fn set_role(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
    ApiJson(request): ApiJson<SetRoleRequest>,
) -> Result<Json<PublicUser>, ApiError> {
    let user = user_repository::set_role(state.db(), id, request.role)
        .await
        .map_err(map_user_error)?;

    record_user_event(
        &state,
        Some(_actor.public_user().id),
        "users.role_changed",
        &user,
    )
    .await;

    Ok(Json(user.public()))
}

pub async fn delete_user(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<PublicUser>, ApiError> {
    let user = user_repository::delete(state.db(), id)
        .await
        .map_err(map_user_error)?;

    record_user_event(
        &state,
        Some(_actor.public_user().id),
        "users.deleted",
        &user,
    )
    .await;

    Ok(Json(user.public()))
}
