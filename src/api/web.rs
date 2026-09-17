use axum::{
    extract::State,
    http::{HeaderValue, header},
    response::{IntoResponse, Redirect, Response},
};

use crate::{api::extractors::ApiPath, error::ApiError, repositories::users, state::AppState};

macro_rules! web_asset {
    ($content_type:literal, $path:literal) => {
        (
            $content_type,
            &include_bytes!(concat!("../../web/", $path))[..],
        )
    };
}

macro_rules! js {
    ($path:literal) => {
        web_asset!("application/javascript; charset=utf-8", $path)
    };
}

const FAVICON: &[u8] = include_bytes!("../../web/admin/favicon.svg");
const GAME_COVER_PLACEHOLDER: &[u8] = include_bytes!("../../web/admin/game-cover-placeholder.jpg");
const ATKINSON_LATIN: &[u8] =
    include_bytes!("../../web/fonts/AtkinsonHyperlegibleNext-Latin.woff2");
const ATKINSON_LATIN_EXT: &[u8] =
    include_bytes!("../../web/fonts/AtkinsonHyperlegibleNext-LatinExt.woff2");
const VERSION_JS: &str = concat!(
    "// Generated from Cargo.toml.\nexport const VERSION_LABEL = 'v",
    env!("CARGO_PKG_VERSION"),
    " beta';\n",
);

pub async fn public_index(State(state): State<AppState>) -> Result<Response, ApiError> {
    if setup_required(&state).await? {
        return Ok(Redirect::temporary("/setup").into_response());
    }

    Ok(secure_static_response(
        "text/html; charset=utf-8",
        include_bytes!("../../web/public/index.html"),
    ))
}

pub async fn public_asset(ApiPath(path): ApiPath<String>) -> Result<Response, ApiError> {
    let (content_type, body): (&str, &[u8]) = match path.as_str() {
        "app.css" => web_asset!("text/css; charset=utf-8", "public/app.css"),
        "main.js" => js!("public/main.js"),
        "api.js" => js!("public/api.js"),
        "auth.js" => js!("public/auth.js"),
        "catalog.js" => js!("public/catalog.js"),
        "pagination.js" => js!("public/pagination.js"),
        "pagination.css" => web_asset!("text/css; charset=utf-8", "public/pagination.css"),
        "icons.js" => js!("public/icons.js"),
        "downloads.js" => js!("public/downloads.js"),
        "scroll.js" => js!("public/scroll.js"),
        "shared.js" => js!("public/shared.js"),
        "version.js" => (
            "application/javascript; charset=utf-8",
            VERSION_JS.as_bytes(),
        ),
        "views.js" => js!("public/views.js"),
        "favicon.svg" => ("image/svg+xml", FAVICON),
        "game-cover-placeholder.jpg" => ("image/jpeg", GAME_COVER_PLACEHOLDER),
        "AtkinsonHyperlegibleNext-Latin.woff2" => ("font/woff2", ATKINSON_LATIN),
        "AtkinsonHyperlegibleNext-LatinExt.woff2" => ("font/woff2", ATKINSON_LATIN_EXT),
        _ => (
            "image/png",
            super::platform_icons::get(&path)
                .ok_or_else(|| ApiError::not_found("public asset not found"))?,
        ),
    };

    Ok(secure_static_response(content_type, body))
}

pub async fn setup_index(State(state): State<AppState>) -> Result<Response, ApiError> {
    if !setup_required(&state).await? {
        return Ok(Redirect::temporary("/admin").into_response());
    }

    Ok(secure_static_response(
        "text/html; charset=utf-8",
        include_bytes!("../../web/setup/index.html"),
    ))
}

pub async fn setup_asset(ApiPath(path): ApiPath<String>) -> Result<Response, ApiError> {
    match path.as_str() {
        "main.js" => Ok(secure_static_response(
            "application/javascript; charset=utf-8",
            include_bytes!("../../web/setup/main.js"),
        )),
        _ => Err(ApiError::not_found("setup asset not found")),
    }
}

pub async fn admin_index(State(state): State<AppState>) -> Result<Response, ApiError> {
    if setup_required(&state).await? {
        return Ok(Redirect::temporary("/setup").into_response());
    }

    Ok(secure_static_response(
        "text/html; charset=utf-8",
        include_bytes!("../../web/admin/index.html"),
    ))
}

pub async fn admin_asset(ApiPath(path): ApiPath<String>) -> Result<Response, ApiError> {
    let (content_type, body): (&str, &[u8]) = match path.as_str() {
        "app.css" => web_asset!("text/css; charset=utf-8", "admin/app.css"),
        "main.js" => js!("admin/main.js"),
        "api.js" => js!("admin/api.js"),
        "auth.js" => js!("admin/auth.js"),
        "state.js" => js!("admin/state.js"),
        "dom.js" => js!("admin/dom.js"),
        "views/dashboard.js" => js!("admin/views/dashboard.js"),
        "views/gog-import.js" => js!("admin/views/gog-import.js"),
        "views/jobs.js" => js!("admin/views/jobs.js"),
        "views/library.js" => js!("admin/views/library.js"),
        "views/library-renderers.js" => js!("admin/views/library-renderers.js"),
        "views/metadata.js" => js!("admin/views/metadata.js"),
        "views/romm-browse.js" => js!("admin/views/romm-browse.js"),
        "views/romm-source.js" => js!("admin/views/romm-source.js"),
        "views/server-cleanup.js" => js!("admin/views/server-cleanup.js"),
        "views/shared.js" => js!("admin/views/shared.js"),
        "views/upload.js" => js!("admin/views/upload.js"),
        "views/upload-renderers.js" => js!("admin/views/upload-renderers.js"),
        "views/settings.js" => js!("admin/views/settings.js"),
        "features/background-transfer.js" => js!("admin/features/background-transfer.js"),
        "features/gog-import.js" => js!("admin/features/gog-import.js"),
        "features/gog-title.js" => js!("admin/features/gog-title.js"),
        "features/igdb.js" => js!("admin/features/igdb.js"),
        "features/jobs.js" => js!("admin/features/jobs.js"),
        "features/library.js" => js!("admin/features/library.js"),
        "features/library-scan.js" => js!("admin/features/library-scan.js"),
        "features/romm-covers.js" => js!("admin/features/romm-covers.js"),
        "features/romm-source.js" => js!("admin/features/romm-source.js"),
        "features/server-jobs.js" => js!("admin/features/server-jobs.js"),
        "features/server-cleanup.js" => js!("admin/features/server-cleanup.js"),
        "features/upload.js" => js!("admin/features/upload.js"),
        "features/upload-transfer.js" => js!("admin/features/upload-transfer.js"),
        "favicon.svg" => ("image/svg+xml", FAVICON),
        "game-cover-placeholder.jpg" => ("image/jpeg", GAME_COVER_PLACEHOLDER),
        "AtkinsonHyperlegibleNext-Latin.woff2" => ("font/woff2", ATKINSON_LATIN),
        "AtkinsonHyperlegibleNext-LatinExt.woff2" => ("font/woff2", ATKINSON_LATIN_EXT),
        _ => return Err(ApiError::not_found("admin asset not found")),
    };

    Ok(secure_static_response(content_type, body))
}

async fn setup_required(state: &AppState) -> Result<bool, ApiError> {
    users::has_admin(state.db())
        .await
        .map(|has_admin| !has_admin)
        .map_err(|error| {
            tracing::error!(?error, "failed to read initial setup status");
            ApiError::internal("failed to read initial setup status")
        })
}

fn secure_static_response(content_type: &'static str, body: &'static [u8]) -> Response {
    let mut response = ([(header::CONTENT_TYPE, content_type)], body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' blob: data: https://images.igdb.com; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    response
}
