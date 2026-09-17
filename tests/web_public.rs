mod support;

use axum::{
    body::to_bytes,
    http::{StatusCode, header},
};
use support::response_text;
use tower::ServiceExt;

#[tokio::test]
async fn public_library_is_served_at_the_server_root_without_exposing_the_catalog() {
    let test_app = support::TestApp::new().await;
    test_app.seed_admin("admin", "secret").await;
    let app = test_app.router;

    let response = app.clone().oneshot(empty_request("/")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    assert!(
        response.headers()[header::CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("connect-src 'self'")
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    let body = response_text(response).await;
    assert!(body.contains("Teatro — Game Library"));
    assert!(body.contains("/public/app.css"));
    assert!(body.contains("/public/main.js"));
    assert!(body.contains("/public/favicon.svg?icon=teatro-controller"));

    let response = app
        .clone()
        .oneshot(empty_request("/api/platforms"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn public_library_assets_are_allowlisted_and_cover_the_browse_flow() {
    let test_app = support::TestApp::new().await;
    let app = test_app.router;

    let response = app
        .clone()
        .oneshot(empty_request("/public/app.css"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/css; charset=utf-8"
    );
    let css = response_text(response).await;
    for marker in [
        ".login-page",
        ".platform-grid",
        ".platform-icon",
        ".recent-games-grid",
        ".game-grid",
        ".game-detail",
        ".download-panel",
        ".toast-region",
        "font-family: \"Atkinson Hyperlegible Next\"",
        "--font-display: \"Atkinson Hyperlegible Next\", system-ui, sans-serif",
        "--palette-ink: #111018",
        "--palette-wine: #633436",
        "--palette-red: #bd4444",
        "--palette-paper: #f3f1f4",
        "--brand: #d65353",
        "--radius-l: 14px",
        "filter: blur(90px) saturate(110%)",
        "@keyframes loading-spin",
        "@media (max-width: 580px)",
        "prefers-reduced-motion",
    ] {
        assert!(css.contains(marker), "missing public CSS marker {marker}");
    }

    for font in [
        "AtkinsonHyperlegibleNext-Latin.woff2",
        "AtkinsonHyperlegibleNext-LatinExt.woff2",
    ] {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/public/{font}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{font}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "font/woff2");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.starts_with(b"wOF2"), "invalid WOFF2 file {font}");
    }

    assert!(css.contains("@import url(\"/public/pagination.css\")"));
    let response = app
        .clone()
        .oneshot(empty_request("/public/pagination.css"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/css; charset=utf-8"
    );
    let pagination_css = response_text(response).await;
    assert!(pagination_css.contains(".pagination-pages"));
    assert!(pagination_css.contains("margin-top: 1.5rem"));

    let mut javascript = String::new();
    for module in [
        "main",
        "api",
        "auth",
        "catalog",
        "pagination",
        "icons",
        "shared",
        "version",
        "downloads",
        "scroll",
        "views",
    ] {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/public/{module}.js")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{module}.js");
        assert_eq!(
            response.headers()[header::CONTENT_TYPE],
            "application/javascript; charset=utf-8"
        );
        javascript.push_str(&response_text(response).await);
    }
    for marker in [
        "/api/users/me",
        "/api/platforms",
        "/api/roms?",
        "populatedPlatforms",
        "renderPlatformPage",
        "renderGameDetail",
        "data-recent-carousel",
        "downloadArchive",
        "archive-ticket",
        "/api/downloads/archive",
        "submitDownloadTicket",
        "/api/downloads/file",
        "/download-ticket",
        "restoreScroll",
        "captureScroll(scrollPageKey(state.route)",
        "sessionStorage",
        "Sign in to Teatro",
        "658573b0171e693bc965c167592cc0b92d002a3e",
    ] {
        assert!(
            javascript.contains(marker),
            "missing public JavaScript marker {marker}"
        );
    }
    assert!(javascript.contains(&format!("v{} beta", env!("CARGO_PKG_VERSION"))));

    let response = app
        .clone()
        .oneshot(empty_request("/public/platform-icons/genesis.png"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));

    let response = app
        .clone()
        .oneshot(empty_request("/public/favicon.svg"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.starts_with(b"<svg"));

    let response = app
        .clone()
        .oneshot(empty_request("/public/game-cover-placeholder.jpg"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.starts_with(b"\xff\xd8\xff"));

    let response = app
        .oneshot(empty_request("/public/not-allowlisted.txt"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = support::response_json(response).await;
    assert_eq!(body["error"]["message"], "public asset not found");
}

fn empty_request(uri: &str) -> axum::http::Request<axum::body::Body> {
    support::empty_request("GET", uri, None)
}
