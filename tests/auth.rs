mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use support::{basic_auth, response_json};
use teatro::{api, repositories::users, services::auth, state::AppState};
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn users_me_requires_basic_auth() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    let app = api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response.headers()[header::WWW_AUTHENTICATE],
        "Basic realm=\"Teatro\", charset=\"UTF-8\""
    );

    let json = response_json(response).await;
    assert_eq!(json["error"]["code"], "unauthorized");
}

#[tokio::test]
async fn api_routes_require_basic_auth_before_not_found() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    let app = api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn authenticated_unknown_api_routes_return_not_found() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    seed_admin(&state, "admin", "correct-password").await;
    let app = api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/does-not-exist")
                .header(
                    header::AUTHORIZATION,
                    basic_auth("admin", "correct-password"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn users_me_rejects_invalid_password() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    seed_admin(&state, "admin", "correct-password").await;
    let app = api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users/me")
                .header(header::AUTHORIZATION, basic_auth("admin", "wrong-password"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn users_me_returns_authenticated_user() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    seed_admin(&state, "admin", "correct-password").await;
    let app = api::router(state);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/users/me")
                .header(
                    header::AUTHORIZATION,
                    basic_auth("admin", "correct-password"),
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response).await;
    assert_eq!(json["username"], "admin");
    assert_eq!(json["role"], "admin");
    assert!(json["id"].is_number());
    assert!(json.get("password_hash").is_none());
}

async fn seed_admin(state: &AppState, username: &str, password: &str) {
    let password_hash = auth::hash_password(password).unwrap();
    users::create_admin(state.db(), username, &password_hash, false)
        .await
        .unwrap();
}
