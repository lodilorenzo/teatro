use axum::{
    body::{Body, to_bytes},
    extract::{MatchedPath, Query, Request, State, rejection::QueryRejection},
    http::{HeaderValue, Method, StatusCode, header::RETRY_AFTER},
    middleware::Next,
    response::{IntoResponse, Response},
};

use serde::Deserialize;

use crate::{
    api::{auth::AdminUser, extractors::ApiPath},
    error::ApiError,
    services::background_transfers::{
        CachedTransferResponse, CancelTransferResult, TransferClaim, TransferKey,
    },
    state::AppState,
};

pub(crate) async fn make_transfer_idempotent(
    actor: AdminUser,
    State(state): State<AppState>,
    endpoint: MatchedPath,
    request: Request,
    next: Next,
) -> Response {
    if endpoint.as_str() == "/api/admin/gog-imports"
        && let Err(error) = super::gog_import::ensure_gog_import_available(&state)
    {
        return error.into_response();
    }
    let Some(raw_id) = request.headers().get("x-teatro-transfer-id") else {
        return next.run(request).await;
    };
    let Ok(id) = raw_id.to_str().map(str::to_owned) else {
        return ApiError::bad_request("background transfer id must be valid ASCII").into_response();
    };
    let key = TransferKey::new(
        actor.public_user().id,
        request.method().clone(),
        endpoint.as_str(),
        &id,
    );
    let Some(claim) = state.background_transfers().claim(&key) else {
        return ApiError::bad_request("invalid background transfer id").into_response();
    };

    match claim {
        TransferClaim::Running => ApiError::new(
            StatusCode::CONFLICT,
            "transfer_in_progress",
            "background transfer is already running",
        )
        .with_header(RETRY_AFTER, HeaderValue::from_static("1"))
        .into_response(),
        TransferClaim::Complete(cached) => cached_response(cached),
        TransferClaim::Cancelled => ApiError::new(
            StatusCode::GONE,
            "transfer_cancelled",
            "background transfer was cancelled",
        )
        .into_response(),
        TransferClaim::Owner(owner) => {
            let response = tokio::select! {
                response = next.run(request) => response,
                () = owner.cancelled() => {
                    return ApiError::conflict("background transfer was cancelled").into_response();
                }
            };
            if !response.status().is_success() {
                return response;
            }
            let (parts, body) = response.into_parts();
            let bytes = match to_bytes(body, usize::MAX).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::error!(
                        ?error,
                        transfer_id = id,
                        "failed to cache transfer response"
                    );
                    return ApiError::internal("failed to retain background transfer result")
                        .into_response();
                }
            };
            owner.complete(CachedTransferResponse {
                status: parts.status,
                headers: parts.headers.clone(),
                body: bytes.clone(),
            });
            Response::from_parts(parts, Body::from(bytes))
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum TransferOperation {
    Upload,
    Gog,
}

#[derive(Deserialize)]
pub(crate) struct CancelTransferQuery {
    operation: TransferOperation,
}

pub(crate) async fn cancel_background_transfer(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    query: Result<Query<CancelTransferQuery>, QueryRejection>,
) -> Result<StatusCode, ApiError> {
    let Query(query) =
        query.map_err(|_| ApiError::bad_request("operation must be upload or gog"))?;
    let endpoint = match query.operation {
        TransferOperation::Upload => "/api/admin/upload-batches",
        TransferOperation::Gog => "/api/admin/gog-imports",
    };
    let key = TransferKey::new(actor.public_user().id, Method::POST, endpoint, &id);
    match state.background_transfers().cancel(&key) {
        CancelTransferResult::Cancelled => Ok(StatusCode::NO_CONTENT),
        CancelTransferResult::Complete => {
            Err(ApiError::conflict("background transfer already completed"))
        }
        CancelTransferResult::NotFound => Err(ApiError::not_found("background transfer not found")),
    }
}

fn cached_response(cached: CachedTransferResponse) -> Response {
    let mut response = Response::new(Body::from(cached.body));
    *response.status_mut() = cached.status;
    *response.headers_mut() = cached.headers;
    response
}
