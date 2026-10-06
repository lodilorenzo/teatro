use super::{errors::multipart_error, multipart::read_text_field};
use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiMultipart, ApiPath},
    },
    error::ApiError,
    services::conversion::{self, ConversionError, InputFile},
    state::AppState,
};
use axum::{
    Json,
    extract::State,
    http::{HeaderValue, StatusCode, header::LOCATION},
};
use std::collections::HashMap;
use tokio::io::AsyncWriteExt;

fn map_error(error: ConversionError) -> ApiError {
    match error {
        ConversionError::Invalid(message) => ApiError::bad_request(message),
        ConversionError::Busy => ApiError::too_many_requests(
            "Conversion capacity is full; retry after a job finishes",
            10,
        ),
        ConversionError::NotFound => ApiError::not_found("Conversion import not found"),
        ConversionError::Conflict => {
            ApiError::conflict("Conversion cancelled or publication has started")
        }
        ConversionError::Io(_) => ApiError::internal("Conversion staging failed"),
    }
}
fn available(state: &AppState) -> Result<(), ApiError> {
    if !state.config().conversion_enabled {
        return Err(ApiError::conflict(
            "Experimental conversion imports are disabled",
        ));
    }
    Ok(())
}
pub async fn status(_actor: AdminUser, State(state): State<AppState>) -> Json<serde_json::Value> {
    let mut formats = vec![
        serde_json::json!({"platform_slug":"wiiu","input":"code/content/meta folders","output":"wua","emulator":"Cemu","qualified":false}),
        serde_json::json!({"platform_slug":"gc","input":"one plain ISO/GCM disc image","output":"rvz","emulator":"Dolphin","qualified":false}),
        serde_json::json!({"platform_slug":"wii","input":"one ISO or single-file WBFS disc image","output":"rvz","emulator":"Dolphin","qualified":false}),
    ];
    formats.extend(conversion::CHD_PLATFORMS.iter().map(|&(platform, emulator)| serde_json::json!({
        "platform_slug":platform, "input":if platform == "psp" { "plain ISO or 2048-byte IMG" } else if matches!(platform, "psx" | "ps2") { "identified ISO, single-data-track IMG or raw 2352-byte BINARY CUE/BIN" } else { "single-data-track raw IMG or raw 2352-byte BINARY CUE/BIN" },
        "output":"chd", "emulator":emulator, "qualified":false,
    })));
    formats.extend(conversion::SEVEN_ZIP_PLATFORMS.iter().map(|&(platform, extensions)| serde_json::json!({
        "platform_slug":platform, "input":"one uncompressed single-file ROM", "input_extensions":extensions,
        "output":"7z", "qualified":false,
    })));
    Json(
        serde_json::json!({"enabled":state.config().conversion_enabled, "experimental":true, "formats":formats,"max_files":conversion::MAX_FILES}),
    )
}

pub async fn create(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiMultipart(mut multipart): ApiMultipart,
) -> Result<
    (
        StatusCode,
        [(axum::http::HeaderName, HeaderValue); 1],
        Json<serde_json::Value>,
    ),
    ApiError,
> {
    available(&state)?;
    let reservation = state
        .conversions()
        .reserve(actor.public_user().id)
        .map_err(map_error)?;
    let permit = state.acquire_upload_permit().await;
    let workspace = conversion::workspace(&state.config().data_dir)
        .await
        .map_err(|_| ApiError::internal("Could not create private staging"))?;
    let input = workspace.path().join("input");
    let mut paths = HashMap::new();
    let mut inputs = Vec::new();
    let mut title = None;
    let mut platform = None;
    let mut total = 0_u64;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
        if reservation
            .cancel
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(ApiError::conflict("Conversion import cancelled"));
        }
        match field.name().unwrap_or("") {
            "title" if title.is_none() => {
                title = Some(read_text_field(&state, field, "title").await?);
            }
            "platform_slug" if platform.is_none() => {
                let value = read_text_field(&state, field, "platform_slug").await?;
                platform = Some(match value.as_str() {
                    "gc" => "gc",
                    "wii" => "wii",
                    "wiiu" => "wiiu",
                    _ => conversion::CHD_PLATFORMS
                        .iter()
                        .find(|&&(slug, _)| slug == value)
                        .map(|&(slug, _)| slug)
                        .or_else(|| {
                            conversion::SEVEN_ZIP_PLATFORMS
                                .iter()
                                .find(|&&(slug, _)| slug == value)
                                .map(|&(slug, _)| slug)
                        })
                        .ok_or_else(|| ApiError::bad_request("Unsupported conversion platform"))?,
                });
            }
            "files" => {
                if inputs.len() >= conversion::MAX_FILES {
                    return Err(ApiError::payload_too_large(
                        "Too many conversion input files",
                    ));
                }
                let name = field
                    .file_name()
                    .ok_or_else(|| {
                        ApiError::bad_request("Each file requires its relative folder path")
                    })?
                    .to_string();
                conversion::validate_path(&name).map_err(map_error)?;
                let parts: Vec<_> = name.split('/').collect();
                for at in 1..=parts.len() {
                    let original = parts[..at].join("/");
                    let key = original.to_ascii_lowercase();
                    let is_file = at == parts.len();
                    if let Some((old, old_file)) = paths.get(&key) {
                        if old != &original || *old_file || is_file {
                            return Err(ApiError::bad_request(
                                "Duplicate, case-colliding or file/directory-conflicting paths",
                            ));
                        }
                    } else {
                        paths.insert(key, (original, is_file));
                    }
                }
                let path = input.join(&name);
                tokio::fs::create_dir_all(path.parent().unwrap())
                    .await
                    .map_err(|_| ApiError::bad_request("Conflicting input paths"))?;
                let mut target = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .await
                    .map_err(|_| ApiError::bad_request("Conflicting input paths"))?;
                let mut bytes = 0_u64;
                while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                    bytes = bytes.saturating_add(chunk.len() as u64);
                    total = total.saturating_add(chunk.len() as u64);
                    if bytes > state.config().max_upload_bytes
                        || total > state.config().uploads.max_batch_bytes
                    {
                        return Err(ApiError::payload_too_large(
                            "Conversion input exceeds upload byte limits",
                        ));
                    }
                    if fs2::available_space(workspace.path())
                        .map_err(|_| ApiError::internal("Could not check staging space"))?
                        < (chunk.len() as u64)
                            .saturating_add(state.config().uploads.free_space_margin_bytes)
                    {
                        return Err(ApiError::insufficient_storage(
                            "Not enough conversion staging space",
                        ));
                    }
                    target
                        .write_all(&chunk)
                        .await
                        .map_err(|_| ApiError::internal("Could not stage conversion input"))?;
                }
                target
                    .flush()
                    .await
                    .map_err(|_| ApiError::internal("Could not flush conversion input"))?;
                drop(target);
                inputs.push(InputFile { path: name, bytes });
            }
            _ => {
                return Err(ApiError::bad_request(
                    "Expected one title, optional platform_slug and files with relative paths",
                ));
            }
        }
    }
    drop(permit);
    let title = title
        .filter(|title| {
            !title.trim().is_empty() && title.len() <= 512 && !title.chars().any(char::is_control)
        })
        .ok_or_else(|| ApiError::bad_request("Provide a game title of at most 512 bytes"))?;
    if inputs.is_empty() {
        return Err(ApiError::bad_request("Select conversion input files"));
    }
    if platform.is_some_and(|slug| slug != "wiiu")
        && inputs.len() > state.config().uploads.max_batch_files
    {
        return Err(ApiError::payload_too_large(
            "Too many conversion input files",
        ));
    }
    let id = reservation.id.clone();
    let url = format!("/api/admin/conversion-imports/{id}");
    conversion::launch(
        state,
        reservation,
        workspace,
        inputs,
        title.trim().into(),
        platform.unwrap_or("wiiu"),
    );
    Ok((
        StatusCode::ACCEPTED,
        [(LOCATION, HeaderValue::from_str(&url).unwrap())],
        Json(serde_json::json!({"id":id,"status_url":url})),
    ))
}
pub async fn job(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(
        state
            .conversions()
            .snapshot(actor.public_user().id, &id)
            .map_err(map_error)?,
    ))
}
pub async fn cancel(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<StatusCode, ApiError> {
    state
        .conversions()
        .cancel(actor.public_user().id, &id)
        .map_err(map_error)?;
    Ok(StatusCode::NO_CONTENT)
}
