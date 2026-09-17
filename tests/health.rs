mod support;

use axum::http::StatusCode;
use support::{empty_request, response_json};
use teatro::{api, state::AppState};
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn healthz_returns_service_status() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    let app = api::router(state);

    let response = app
        .oneshot(empty_request("GET", "/healthz", None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response).await;

    assert_eq!(json["status"], "ok");
    assert_eq!(json["service"], "teatro");
    assert!(json["version"].is_string());
    assert!(json["started_at"].is_string());
}

#[tokio::test]
async fn missing_routes_return_json_error() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    let app = api::router(state);

    let response = app
        .oneshot(empty_request("GET", "/does-not-exist", None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let json = response_json(response).await;

    assert_eq!(json["error"]["code"], "not_found");
    assert_eq!(json["error"]["message"], "route not found");
}
