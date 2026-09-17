use axum::{
    Json,
    extract::{FromRequest, Request},
};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    S: Send + Sync,
    T: DeserializeOwned,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|rejection| {
                let status = rejection.status();
                if status == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::payload_too_large("request body is too large")
                } else {
                    ApiError::bad_request("request body must contain valid JSON")
                }
            })
    }
}
