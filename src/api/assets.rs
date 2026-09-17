use axum::{extract::State, response::Response};

use super::{extractors::ApiPath, map_file_store_read_error, map_stream_read_error};
use crate::{error::ApiError, state::AppState, storage::streaming};

pub async fn serve_asset(
    State(state): State<AppState>,
    ApiPath(resource_path): ApiPath<String>,
) -> Result<Response, ApiError> {
    let file = state
        .file_store()
        .open_existing(&state.config().asset_root, &resource_path)
        .await
        .map_err(|error| map_file_store_read_error(error, "asset", "asset path", "asset path"))?;
    let content_type = content_type_for_path(&resource_path);

    streaming::stream_file(file, content_type, None)
        .await
        .map_err(|error| map_stream_read_error(error, "asset"))
}

fn content_type_for_path(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "avif" => "image/avif",
        "gif" => "image/gif",
        "jpg" | "jpeg" => "image/jpeg",
        "json" => "application/json",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}
