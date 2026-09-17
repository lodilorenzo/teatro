//! HTTP routing and translation between wire contracts and application workflows.

mod admin;
mod assets;
pub(crate) mod auth;
mod extractors;
mod health;
mod platform_icons;
mod romm;
mod sessions;
mod setup;
mod stats;
mod users;
mod web;

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post, put},
};
use tower_http::trace::TraceLayer;

use crate::{
    error::ApiError,
    services::igdb::MAX_COVER_BYTES,
    state::AppState,
    storage::{file_store::FileStoreError, paths::PathSafetyError, streaming::StreamFileError},
};

fn map_file_store_read_error(
    error: FileStoreError,
    noun: &'static str,
    path: &'static str,
    outside_root_path: &'static str,
) -> ApiError {
    match error {
        FileStoreError::Path(error) => map_path_read_error(error, noun, path, outside_root_path),
        FileStoreError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            read_not_found(noun)
        }
        FileStoreError::Io(error) => {
            tracing::error!(?error, resource = noun, "failed to open API resource");
            ApiError::internal(format!("failed to open {noun}"))
        }
        FileStoreError::AlreadyExists
        | FileStoreError::NotRegularFile
        | FileStoreError::NotDirectory => read_not_found(noun),
    }
}

fn map_path_read_error(
    error: PathSafetyError,
    noun: &'static str,
    path: &'static str,
    outside_root_path: &'static str,
) -> ApiError {
    match error {
        PathSafetyError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            read_not_found(noun)
        }
        PathSafetyError::Io(error) => {
            tracing::error!(?error, path, "failed to resolve API resource path");
            ApiError::internal(format!("failed to resolve {path}"))
        }
        PathSafetyError::Empty
        | PathSafetyError::Absolute
        | PathSafetyError::ParentTraversal
        | PathSafetyError::EscapesRoot
        | PathSafetyError::Symlink => {
            ApiError::forbidden(format!("{outside_root_path} is outside configured root"))
        }
    }
}

fn map_stream_read_error(error: StreamFileError, noun: &'static str) -> ApiError {
    match error {
        StreamFileError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
            read_not_found(noun)
        }
        error => {
            tracing::error!(?error, resource = noun, "failed to stream API resource");
            ApiError::internal(format!("failed to stream {noun}"))
        }
    }
}

fn read_not_found(noun: &'static str) -> ApiError {
    ApiError::not_found(format!("{noun} not found"))
}

fn validate_user_request_username(state: &AppState, username: &str) -> Result<(), ApiError> {
    if username.len() > state.config().auth.max_username_bytes {
        return Err(ApiError::bad_request("username is too long"));
    }
    Ok(())
}

async fn hash_user_password(state: &AppState, password: &str) -> Result<String, ApiError> {
    if password.is_empty() {
        return Err(ApiError::bad_request("password cannot be empty"));
    }
    if password.len() > state.config().auth.max_password_bytes {
        return Err(ApiError::bad_request("password is too long"));
    }

    state
        .password_service()
        .hash(password)
        .await
        .map_err(|error| {
            if error.is_capacity_error() {
                return ApiError::too_many_requests("password hashing is busy; retry shortly", 1);
            }
            tracing::error!(?error, "failed to hash user password");
            ApiError::internal("failed to hash password")
        })
}

pub fn router(state: AppState) -> Router {
    let admin_routes = Router::new()
        .route("/users", get(admin::list_users))
        .route("/users", post(admin::create_user))
        .route("/users/{id}", delete(admin::delete_user))
        .route("/users/{id}/password", patch(admin::reset_password))
        .route("/users/{id}/role", patch(admin::set_role))
        .route("/tokens", get(admin::list_api_tokens))
        .route("/tokens", post(admin::create_api_token))
        .route("/tokens/{id}", delete(admin::revoke_api_token))
        .route(
            "/uploads",
            post(admin::upload_rom).layer(DefaultBodyLimit::disable()),
        )
        .route("/uploads/preview", post(admin::preview_upload_batch))
        .route(
            "/upload-batches",
            post(admin::upload_batch)
                .layer(DefaultBodyLimit::disable())
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    admin::transfers::make_transfer_idempotent,
                )),
        )
        .route(
            "/background-transfers/{id}",
            delete(admin::transfers::cancel_background_transfer),
        )
        .route("/gog-import/status", get(admin::gog_import_status))
        .route(
            "/gog-imports",
            post(admin::import_gog_setup)
                .layer(DefaultBodyLimit::disable())
                .layer(middleware::from_fn_with_state(
                    state.clone(),
                    admin::transfers::make_transfer_idempotent,
                )),
        )
        .route(
            "/gog-imports/{id}",
            get(admin::gog_import_job).delete(admin::cancel_gog_import_job),
        )
        .route("/library/scans", post(admin::create_library_scan))
        .route(
            "/library/scans/{id}",
            get(admin::library_scan_job).delete(admin::cancel_library_scan_job),
        )
        .route(
            "/library/sidecars",
            get(admin::preview_sidecar_cleanup).delete(admin::cleanup_sidecars),
        )
        .route("/dats", get(admin::list_dats))
        .route(
            "/dats",
            post(admin::import_dat).layer(DefaultBodyLimit::disable()),
        )
        .route("/integrity/jobs", get(admin::list_integrity_jobs))
        .route("/integrity/jobs", post(admin::start_integrity_job))
        .route("/integrity/jobs/{id}", get(admin::integrity_job))
        .route("/roms", delete(admin::delete_all_roms))
        .route("/roms/{id}", patch(admin::update_rom))
        .route("/roms/{id}", delete(admin::delete_rom))
        .route(
            "/roms/{id}/cover",
            put(admin::update_cover).layer(DefaultBodyLimit::max(MAX_COVER_BYTES as usize)),
        )
        .route("/roms/{id}/files", get(admin::rom_files))
        .route("/roms/{id}/integrity", get(admin::rom_integrity))
        .route("/platforms/{id}/roms", delete(admin::delete_platform_roms))
        .route("/roms/{id}/metadata/igdb", post(admin::apply_igdb_metadata))
        .route("/igdb/status", get(admin::igdb_status))
        .route("/igdb/settings", get(admin::get_igdb_settings))
        .route("/igdb/settings", patch(admin::save_igdb_settings))
        .route("/igdb/settings", delete(admin::clear_igdb_settings))
        .route("/igdb/search", get(admin::igdb_search))
        .route("/sources/romm/status", get(admin::romm_source_status))
        .route(
            "/sources/romm/settings",
            patch(admin::save_romm_source_settings).delete(admin::clear_romm_source_settings),
        )
        .route("/sources/romm/test", post(admin::test_romm_source))
        .route(
            "/sources/romm/refresh",
            post(admin::refresh_romm_source_index),
        )
        .route("/sources/romm/platforms", get(admin::romm_source_platforms))
        .route("/sources/romm/roms", get(admin::romm_source_roms))
        .route(
            "/sources/romm/roms/{remote_id}",
            get(admin::romm_source_rom_detail),
        )
        .route(
            "/sources/romm/roms/{remote_id}/cover",
            get(admin::romm_source_rom_cover),
        )
        .route("/sources/romm/imports", post(admin::create_romm_import))
        .route(
            "/sources/romm/imports/{id}",
            get(admin::romm_import_job).delete(admin::cancel_romm_import_job),
        )
        .route("/stats", get(admin::stats));

    let authenticated_layer = middleware::from_fn_with_state(state.clone(), auth::require_auth);

    let api_routes = Router::new()
        .route(
            "/auth/session",
            post(sessions::login)
                .delete(sessions::logout)
                .layer(DefaultBodyLimit::max(256)),
        )
        .route("/users/me", get(users::me))
        .route("/stats", get(stats::public_stats))
        .route("/platforms", get(romm::list_platforms))
        .route("/roms", get(romm::list_roms))
        .route("/roms/{id}", get(romm::rom_detail))
        .route("/roms/{id}/download-plan", get(romm::rom_download_plan))
        .route(
            "/roms/{id}/archive-ticket",
            post(romm::create_rom_archive_ticket),
        )
        .route(
            "/roms/{id}/files/{file_id}/download-ticket",
            post(romm::create_rom_file_ticket),
        )
        .route(
            "/roms/{id}/content/{file_name}",
            get(romm::download_rom_file),
        )
        .nest("/admin", admin_routes)
        .fallback(not_found)
        .layer(authenticated_layer.clone());
    let api_routes = Router::new()
        .route(
            "/setup",
            get(setup::status)
                .post(setup::create_initial_admin)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/downloads/file",
            post(romm::download_rom_file_with_ticket).layer(DefaultBodyLimit::max(256)),
        )
        .route(
            "/downloads/archive",
            post(romm::download_rom_archive_with_ticket).layer(DefaultBodyLimit::max(256)),
        )
        .merge(api_routes);

    let asset_routes = Router::new()
        .route("/romm/resources/{*path}", get(assets::serve_asset))
        .fallback(not_found)
        .layer(authenticated_layer);

    Router::new()
        .route("/", get(web::public_index))
        .route("/public/{*path}", get(web::public_asset))
        .route("/healthz", get(health::healthz))
        .route("/setup", get(web::setup_index))
        .route("/setup/", get(web::setup_index))
        .route("/setup/{*path}", get(web::setup_asset))
        .route("/admin", get(web::admin_index))
        .route("/admin/", get(web::admin_index))
        .route("/admin/{*path}", get(web::admin_asset))
        .nest("/api", api_routes)
        .nest("/assets", asset_routes)
        .fallback(not_found)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn not_found() -> ApiError {
    ApiError::not_found("route not found")
}
