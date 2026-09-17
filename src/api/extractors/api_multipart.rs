use axum::extract::{FromRequest, Multipart, Request};

use crate::error::ApiError;

pub struct ApiMultipart(pub Multipart);

impl<S> FromRequest<S> for ApiMultipart
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        Multipart::from_request(request, state)
            .await
            .map(Self)
            .map_err(|rejection| {
                tracing::debug!(?rejection, "invalid multipart upload request");
                ApiError::bad_request("invalid multipart upload request")
            })
    }
}
