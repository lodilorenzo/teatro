//! Admin HTTP surface for the RomM source: settings, browse proxy, and import jobs.
//!
//! Every route in this module returns `404` when `TEATRO_ROMM_SOURCE_ENABLED` is false, so a
//! disabled deployment exposes no evidence that the feature exists. The stored secret is accepted
//! by `PATCH` and never returned, logged, or placed in a job snapshot or audit row.

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{
        HeaderName, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, LOCATION},
    },
    response::{IntoResponse, Response},
};
use tracing::{Instrument, instrument::WithSubscriber};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
    },
    error::ApiError,
    repositories::{
        platforms,
        romm_source::{self, RommAuthMode, RommSourceSettings},
        roms,
    },
    services::{
        library::{sanitize_upload_file_name, slugify},
        romm_source::{
            RemoteRom, RommImportJobReporter, RommImportRequest, RommSourceError, RommSourceStatus,
            cached_platforms, cached_roms, import_failure_summary, import_rom,
            is_plaintext_base_url, normalized_base_url, refresh_catalog,
        },
    },
    state::AppState,
};

use super::{
    audit::record_romm_source_event,
    dto::{
        CreateRommImportRequest, RommImportJobCreateResponse, RommImportJobStatusResponse,
        RommRemoteFileResponse, RommRemotePlatformResponse, RommRemoteRomDetailResponse,
        RommRemoteRomListResponse, RommRemoteRomResponse, RommSourceStatusResponse,
        RommSourceTestResponse, SaveRommSourceRequest,
    },
    errors::{map_database_error, map_romm_import_job_registry_error, map_romm_source_error},
    query::RommBrowseQuery,
};

const MAX_BASE_URL_BYTES: usize = 512;
const MAX_CREDENTIAL_BYTES: usize = 512;
const MAX_TITLE_BYTES: usize = 512;

pub async fn romm_source_status(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<RommSourceStatusResponse>, ApiError> {
    ensure_enabled(&state)?;
    Ok(Json(RommSourceStatusResponse::from(
        load_status(&state).await?,
    )))
}

pub async fn save_romm_source_settings(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<SaveRommSourceRequest>,
) -> Result<Json<RommSourceStatusResponse>, ApiError> {
    ensure_enabled(&state)?;

    let base_url = request.base_url.trim();
    if base_url.len() > MAX_BASE_URL_BYTES {
        return Err(ApiError::bad_request("base_url is too long"));
    }
    let parsed = normalized_base_url(base_url).map_err(map_romm_source_error)?;
    let plaintext_http = parsed.scheme() == "http";
    if plaintext_http && !request.acknowledge_plaintext_http {
        // Credentials and ROM bytes traverse a plaintext link unencrypted. Teatro does not claim
        // the connection is secure, and it does not accept the credential without the operator
        // acknowledging that.
        return Err(ApiError::bad_request(
            "acknowledge_plaintext_http is required for an http:// RomM base URL because credentials and ROM bytes traverse the link unencrypted",
        ));
    }

    let auth_mode = match request.auth_mode.as_deref().map(str::trim) {
        None | Some("") => RommAuthMode::Token,
        Some(value) => RommAuthMode::parse(value)
            .ok_or_else(|| ApiError::bad_request("auth_mode must be token or basic"))?,
    };
    let stored = romm_source::load(state.db())
        .await
        .map_err(map_database_error)?;
    let username = bounded_credential("username", request.username.as_deref())?
        .or_else(|| stored.as_ref().and_then(|stored| stored.username.clone()));
    let secret = bounded_credential("secret", request.secret.as_deref())?
        // A missing secret means "unchanged", matching the IGDB settings form.
        .or_else(|| stored.as_ref().and_then(|stored| stored.secret.clone()));

    let reset_index = stored.as_ref().is_none_or(|stored| {
        stored.base_url != parsed.as_str()
            || stored.username != username
            || stored.auth_mode != auth_mode
    });
    romm_source::save(
        state.db(),
        romm_source::SaveRommSourceParams {
            base_url: parsed.as_str(),
            username: username.as_deref(),
            secret: secret.as_deref(),
            auth_mode,
            reset_index,
        },
    )
    .await
    .map_err(map_database_error)?;
    state.romm_client().clear_token_cache();

    let status = load_status(&state).await?;
    record_romm_source_event(
        &state,
        Some(actor.public_user().id),
        &status,
        "romm_source.settings_saved",
    )
    .await;
    Ok(Json(RommSourceStatusResponse::from(status)))
}

pub async fn clear_romm_source_settings(
    actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<RommSourceStatusResponse>, ApiError> {
    ensure_enabled(&state)?;
    romm_source::clear(state.db())
        .await
        .map_err(map_database_error)?;
    state.romm_client().clear_token_cache();

    let status = load_status(&state).await?;
    record_romm_source_event(
        &state,
        Some(actor.public_user().id),
        &status,
        "romm_source.settings_cleared",
    )
    .await;
    Ok(Json(RommSourceStatusResponse::from(status)))
}

pub async fn test_romm_source(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<RommSourceTestResponse>, ApiError> {
    let settings = configured_settings(&state).await?;
    let plaintext_http = is_plaintext_base_url(&settings.base_url);

    match state.romm_client().probe(&settings).await {
        Ok(connection) => Ok(Json(RommSourceTestResponse::new(
            connection,
            plaintext_http,
        ))),
        Err(RommSourceError::Unauthorized) => {
            Ok(Json(RommSourceTestResponse::unauthorized(plaintext_http)))
        }
        Err(error @ (RommSourceError::Unreachable | RommSourceError::RedirectRefused)) => Ok(Json(
            RommSourceTestResponse::unreachable(plaintext_http, error.to_string()),
        )),
        Err(error) => Err(map_romm_source_error(error)),
    }
}

pub async fn refresh_romm_source_index(
    actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<RommSourceStatusResponse>, ApiError> {
    let settings = configured_settings(&state).await?;
    refresh_catalog(state.db(), state.romm_client(), &settings)
        .await
        .map_err(map_romm_source_error)?;
    let status = load_status(&state).await?;
    record_romm_source_event(
        &state,
        Some(actor.public_user().id),
        &status,
        "romm_source.index_refreshed",
    )
    .await;
    Ok(Json(RommSourceStatusResponse::from(status)))
}

pub async fn romm_source_platforms(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<RommRemotePlatformResponse>>, ApiError> {
    configured_settings(&state).await?;
    let remote = cached_platforms(state.db())
        .await
        .map_err(map_romm_source_error)?;

    let mut responses = Vec::with_capacity(remote.len());
    for platform in remote {
        let local_platform = local_platform(&state, Some(platform.slug.as_str())).await?;
        responses.push(RommRemotePlatformResponse::new(platform, local_platform));
    }
    Ok(Json(responses))
}

pub async fn romm_source_roms(
    _actor: AdminUser,
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<RommRemoteRomListResponse>, ApiError> {
    configured_settings(&state).await?;
    let query = RommBrowseQuery::parse(raw_query.as_deref())?;
    let page = cached_roms(
        state.db(),
        query.platform_id,
        query.search.as_deref(),
        query.limit,
        query.offset,
    )
    .await
    .map_err(map_romm_source_error)?;

    let mut items = Vec::with_capacity(page.items.len());
    for rom in page.items {
        items.push(annotate_rom(&state, rom).await?);
    }
    Ok(Json(RommRemoteRomListResponse {
        items,
        total: page.total,
        limit: page.limit,
        offset: page.offset,
    }))
}

pub async fn romm_source_rom_detail(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(remote_rom_id): ApiPath<i64>,
) -> Result<Json<RommRemoteRomDetailResponse>, ApiError> {
    let settings = configured_settings(&state).await?;
    let detail = state
        .romm_client()
        .rom_detail(&settings, remote_rom_id)
        .await
        .map_err(map_romm_source_error)?;

    let total_size_bytes = detail
        .files
        .iter()
        .filter_map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    Ok(Json(RommRemoteRomDetailResponse {
        rom: annotate_rom(&state, detail.rom).await?,
        files: detail
            .files
            .into_iter()
            .map(RommRemoteFileResponse::from)
            .collect(),
        total_size_bytes,
    }))
}

/// Proxies one bounded cover image. `no-store`, and never written to `data/assets`.
pub async fn romm_source_rom_cover(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(remote_rom_id): ApiPath<i64>,
) -> Result<Response, ApiError> {
    let settings = configured_settings(&state).await?;
    let cover = state
        .romm_client()
        .cover(&settings, remote_rom_id)
        .await
        .map_err(map_romm_source_error)?;

    let content_type = HeaderValue::from_str(&cover.content_type)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    Ok((
        [
            (CONTENT_TYPE, content_type),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        cover.bytes,
    )
        .into_response())
}

pub async fn create_romm_import(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateRommImportRequest>,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<RommImportJobCreateResponse>,
    ),
    ApiError,
> {
    configured_settings(&state).await?;
    if request.remote_file_ids.is_empty() {
        return Err(ApiError::bad_request(
            "remote_file_ids must name at least one remote file",
        ));
    }
    if request.remote_file_ids.len() > state.config().romm_source.max_import_files {
        return Err(map_romm_source_error(RommSourceError::TooManyFiles));
    }
    let title = match request.title.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(title) if title.len() > MAX_TITLE_BYTES => {
            return Err(ApiError::bad_request("title is too long"));
        }
        Some(title) => Some(title.to_string()),
    };

    let (snapshot, reporter) = state.romm_import_jobs().reserve();
    let status_url = format!("/api/admin/sources/romm/imports/{}", snapshot.id);
    let location = HeaderValue::from_str(&status_url)
        .expect("generated RomM import status URLs contain only valid header characters");
    let response = RommImportJobCreateResponse::new(snapshot, status_url);

    spawn_romm_import_worker(
        state,
        actor.public_user().id,
        RommImportRequest {
            remote_rom_id: request.remote_rom_id,
            remote_file_ids: request.remote_file_ids,
            platform_id: request.platform_id,
            title,
        },
        reporter,
    );

    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(response)))
}

pub async fn romm_import_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<RommImportJobStatusResponse>, ApiError> {
    ensure_enabled(&state)?;
    let snapshot = state
        .romm_import_jobs()
        .snapshot(&id)
        .map_err(map_romm_import_job_registry_error)?;
    Ok(Json(RommImportJobStatusResponse::from(snapshot)))
}

pub async fn cancel_romm_import_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<RommImportJobStatusResponse>, ApiError> {
    ensure_enabled(&state)?;
    let snapshot = state
        .romm_import_jobs()
        .cancel(&id)
        .map_err(map_romm_import_job_registry_error)?;
    Ok(Json(RommImportJobStatusResponse::from(snapshot)))
}

fn spawn_romm_import_worker(
    state: AppState,
    actor_id: i64,
    request: RommImportRequest,
    reporter: RommImportJobReporter,
) {
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let span = tracing::info_span!(
        "romm_import",
        actor_id,
        remote_rom_id = request.remote_rom_id,
        job_id = reporter.id(),
    );
    tokio::spawn(
        supervise_romm_import_worker(state, actor_id, request, reporter)
            .instrument(span)
            .with_subscriber(dispatch),
    );
}

async fn supervise_romm_import_worker(
    state: AppState,
    actor_id: i64,
    request: RommImportRequest,
    reporter: RommImportJobReporter,
) {
    match reporter.start() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::error!(?error, "failed to start admitted RomM import job");
            let _ = reporter.fail(import_failure_summary(&RommSourceError::Staging));
            return;
        }
    }

    // The import holds an upload slot for its whole lifetime, so remote downloads and admin
    // uploads cannot together exceed the configured concurrency.
    let permit = state.acquire_upload_permit().await;
    let child_state = state.clone();
    let child_reporter = reporter.clone();
    let span = tracing::Span::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let workflow = tokio::spawn(
        async move { import_rom(&child_state, request, &child_reporter).await }
            .instrument(span)
            .with_subscriber(dispatch),
    );
    if let Err(error) = reporter.attach_abort_handle(workflow.abort_handle()) {
        tracing::error!(?error, "failed to attach RomM import cancellation handle");
    }
    let completion = workflow.await;

    let terminal = match completion {
        Ok(Ok(outcome)) => {
            record_romm_source_event(
                &state,
                Some(actor_id),
                &load_status(&state)
                    .await
                    .unwrap_or(RommSourceStatus::disabled()),
                if outcome.outcome == "imported" {
                    "romm_source.import_completed"
                } else {
                    "romm_source.import_skipped_duplicate"
                },
            )
            .await;
            reporter.succeed(outcome)
        }
        Ok(Err(error)) => {
            tracing::warn!(code = error.code(), "RomM import failed");
            reporter.fail(import_failure_summary(&error))
        }
        Err(error) => {
            tracing::error!(
                worker_panicked = error.is_panic(),
                worker_cancelled = error.is_cancelled(),
                "supervised RomM import task ended without a result"
            );
            reporter.fail(import_failure_summary(&RommSourceError::Staging))
        }
    };
    if let Err(error) = terminal {
        tracing::error!(?error, "failed to record terminal RomM import state");
    }
    drop(permit);
}

fn ensure_enabled(state: &AppState) -> Result<(), ApiError> {
    if state.config().romm_source.enabled {
        return Ok(());
    }
    // A disabled deployment must not reveal that the feature exists.
    Err(ApiError::not_found("route not found"))
}

async fn load_status(state: &AppState) -> Result<RommSourceStatus, ApiError> {
    let settings = romm_source::load(state.db())
        .await
        .map_err(map_database_error)?;
    let index = romm_source::index_status(state.db())
        .await
        .map_err(map_database_error)?;
    Ok(match settings {
        Some(settings) => RommSourceStatus {
            enabled: true,
            configured: settings.has_credentials(),
            plaintext_http: is_plaintext_base_url(&settings.base_url),
            base_url: Some(settings.base_url),
            username: settings.username,
            auth_mode: settings.auth_mode.as_str(),
            secret_configured: settings.secret.is_some(),
            updated_at: Some(settings.updated_at),
            index_refreshed_at: index.as_ref().map(|index| index.refreshed_at.clone()),
            index_game_count: index.as_ref().map_or(0, |index| index.game_count),
            index_platform_count: index.map_or(0, |index| index.platform_count),
        },
        None => RommSourceStatus {
            enabled: true,
            ..RommSourceStatus::disabled()
        },
    })
}

async fn configured_settings(state: &AppState) -> Result<RommSourceSettings, ApiError> {
    ensure_enabled(state)?;
    romm_source::load(state.db())
        .await
        .map_err(map_database_error)?
        .filter(RommSourceSettings::has_credentials)
        .ok_or_else(|| map_romm_source_error(RommSourceError::NotConfigured))
}

/// Adds the target-platform prefill and the advisory "already present" marker.
async fn annotate_rom(state: &AppState, rom: RemoteRom) -> Result<RommRemoteRomResponse, ApiError> {
    let local_platform = local_platform(state, rom.platform_slug.as_deref()).await?;
    let already = match (&local_platform, rom.fs_name.as_deref()) {
        (Some((platform_id, _)), Some(file_name)) => {
            already_present(state, *platform_id, file_name, &rom.name).await?
        }
        _ => None,
    };
    Ok(RommRemoteRomResponse::new(rom, local_platform, already))
}

/// Bounded local lookup backing the advisory browse marker. Never the enforcement.
async fn already_present(
    state: &AppState,
    platform_id: i64,
    file_name: &str,
    title: &str,
) -> Result<Option<String>, ApiError> {
    if let Ok(file_name) = sanitize_upload_file_name(file_name) {
        let existing =
            roms::existing_file_names(state.db(), platform_id, std::slice::from_ref(&file_name))
                .await
                .map_err(map_database_error)?;
        if existing.contains(&file_name) {
            return Ok(Some(format!(
                "{file_name} already exists on the matched Teatro platform."
            )));
        }
    }

    let slug = slugify(title);
    if !slug.is_empty()
        && roms::slug_exists(state.db(), platform_id, &slug)
            .await
            .map_err(map_database_error)?
    {
        return Ok(Some(format!(
            "“{title}” already exists on the matched Teatro platform."
        )));
    }
    Ok(None)
}

async fn local_platform(
    state: &AppState,
    slug: Option<&str>,
) -> Result<Option<(i64, String)>, ApiError> {
    let Some(slug) = slug.map(str::trim).filter(|slug| !slug.is_empty()) else {
        return Ok(None);
    };
    Ok(
        platforms::find_by_slug(state.db(), local_platform_slug(slug))
            .await
            .map_err(map_database_error)?
            .map(|platform| (platform.id, platform.display_name)),
    )
}

fn local_platform_slug(slug: &str) -> &str {
    match slug {
        "dc" => "dreamcast",
        "neo-geo-cd" => "neogeocd",
        "ngc" => "gc",
        "sega32" => "sega32x",
        "turbografx-cd" => "tg-cd",
        _ => slug,
    }
}

fn bounded_credential(
    field: &'static str,
    value: Option<&str>,
) -> Result<Option<String>, ApiError> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None => Ok(None),
        Some(value) if value.len() > MAX_CREDENTIAL_BYTES => {
            Err(ApiError::bad_request(format!("{field} is too long")))
        }
        Some(value) if value.chars().any(char::is_control) => Err(ApiError::bad_request(format!(
            "{field} must not contain control characters"
        ))),
        Some(value) => Ok(Some(value.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn romm_platform_aliases_match_local_slugs() {
        for (romm, local) in [
            ("dc", "dreamcast"),
            ("neo-geo-cd", "neogeocd"),
            ("ngc", "gc"),
            ("sega32", "sega32x"),
            ("turbografx-cd", "tg-cd"),
            ("snes", "snes"),
        ] {
            assert_eq!(local_platform_slug(romm), local);
        }
    }

    #[test]
    fn credentials_are_bounded_and_reject_control_characters() {
        assert_eq!(bounded_credential("username", Some("  ")).unwrap(), None);
        assert_eq!(
            bounded_credential("username", Some(" teatro ")).unwrap(),
            Some("teatro".to_string())
        );
        assert!(bounded_credential("secret", Some("bad\nvalue")).is_err());
        assert!(bounded_credential("secret", Some(&"x".repeat(MAX_CREDENTIAL_BYTES + 1))).is_err());
    }
}
