mod support;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use chrono::Utc;
use serde_json::{Value, json};
use support::{basic_auth, response_json};
use teatro::{api, domain::user::UserRole, repositories::users, services::auth, state::AppState};
use tempfile::TempDir;
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    authorization: &str,
    body: Value,
) -> Response {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(header::AUTHORIZATION, authorization)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn login(app: &Router, username: &str, remember: bool) -> Value {
    let response = request(
        app,
        "POST",
        "/api/auth/session",
        &basic_auth(username, "password"),
        json!({"remember_me": remember}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let session = response_json(response).await;
    let remaining = session["expires_at"].as_i64().unwrap() - Utc::now().timestamp();
    let expected = if remember { 30 * 86400 } else { 86400 };
    assert!((expected - 5..=expected).contains(&remaining));
    assert!(
        session["token"]
            .as_str()
            .unwrap()
            .starts_with("teatro_session_")
    );
    assert!(session["user"].get("password_hash").is_none());
    session
}

fn bearer(session: &Value) -> String {
    format!("Bearer {}", session["token"].as_str().unwrap())
}

#[tokio::test]
async fn sessions_share_read_and_admin_access_preserve_roles_and_revoke_on_logout() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let hash = auth::hash_password("password").unwrap();
    users::create(state.db(), "admin", &hash, UserRole::Admin)
        .await
        .unwrap();
    users::create(state.db(), "player", &hash, UserRole::ReadOnly)
        .await
        .unwrap();
    let app = api::router(state.clone());

    for remember in [false, true] {
        for username in ["admin", "player"] {
            let session = login(&app, username, remember).await;
            let authorization = bearer(&session);
            for path in ["/api/users/me", "/api/platforms", "/api/roms"] {
                assert_eq!(
                    request(&app, "GET", path, &authorization, Value::Null)
                        .await
                        .status(),
                    StatusCode::OK
                );
            }
            let expected = if username == "admin" {
                StatusCode::OK
            } else {
                StatusCode::FORBIDDEN
            };
            assert_eq!(
                request(&app, "GET", "/api/admin/users", &authorization, Value::Null)
                    .await
                    .status(),
                expected
            );
            assert_eq!(
                request(
                    &app,
                    "POST",
                    "/api/auth/session",
                    &authorization,
                    json!({"remember_me":true})
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(
                request(
                    &app,
                    "DELETE",
                    "/api/auth/session",
                    &authorization,
                    Value::Null
                )
                .await
                .status(),
                StatusCode::NO_CONTENT
            );
            for path in ["/api/users/me", "/api/admin/users"] {
                assert_eq!(
                    request(&app, "GET", path, &authorization, Value::Null)
                        .await
                        .status(),
                    StatusCode::UNAUTHORIZED
                );
            }
        }
    }
    let session = login(&app, "admin", true).await;
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM browser_sessions")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(stored.len(), 64);
    assert_ne!(stored, session["token"].as_str().unwrap());
    drop(app);
    drop(state);
    let restarted = AppState::initialize(config).await.unwrap();
    assert_eq!(
        request(
            &api::router(restarted),
            "GET",
            "/api/admin/users",
            &bearer(&session),
            Value::Null
        )
        .await
        .status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn sessions_expire_follow_current_roles_and_fail_after_password_reset_or_user_deletion() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let hash = auth::hash_password("password").unwrap();
    users::create(state.db(), "owner", &hash, UserRole::Admin)
        .await
        .unwrap();
    let user = users::create(state.db(), "admin", &hash, UserRole::Admin)
        .await
        .unwrap();
    let app = api::router(state.clone());
    let session = login(&app, "admin", true).await;
    users::set_role(state.db(), user.id, UserRole::ReadOnly)
        .await
        .unwrap();
    let me =
        response_json(request(&app, "GET", "/api/users/me", &bearer(&session), Value::Null).await)
            .await;
    assert_eq!(me["role"], "readonly");
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/admin/users",
            &bearer(&session),
            Value::Null
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    users::update_password(
        state.db(),
        user.id,
        &auth::hash_password("new-password").unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        request(&app, "GET", "/api/users/me", &bearer(&session), Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );

    let session = login(&app, "owner", true).await;
    sqlx::query("UPDATE browser_sessions SET expires_at = unixepoch() - 1")
        .execute(state.db())
        .await
        .unwrap();
    assert_eq!(
        request(&app, "GET", "/api/users/me", &bearer(&session), Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let player = users::create(state.db(), "player", &hash, UserRole::ReadOnly)
        .await
        .unwrap();
    let session = login(&app, "player", false).await;
    users::delete(state.db(), player.id).await.unwrap();
    assert_eq!(
        request(&app, "GET", "/api/users/me", &bearer(&session), Value::Null)
            .await
            .status(),
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn session_issuance_requires_valid_password_and_bounds_input_and_session_count() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let hash = auth::hash_password("password").unwrap();
    let user = users::create(state.db(), "admin", &hash, UserRole::Admin)
        .await
        .unwrap();
    let app = api::router(state.clone());
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/auth/session",
            &basic_auth("admin", "wrong"),
            json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/auth/session",
            "Bearer teatro_session_fake",
            json!({})
        )
        .await
        .status(),
        StatusCode::UNAUTHORIZED
    );
    for body in [json!({"remember_me":"true"}), json!({"scopes":["admin"]})] {
        assert_eq!(
            request(
                &app,
                "POST",
                "/api/auth/session",
                &basic_auth("admin", "password"),
                body
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/auth/session",
            &basic_auth("admin", "password"),
            json!({"extra":"x".repeat(300)})
        )
        .await
        .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    // Populate the limit without spending 64 Argon2 verifications.
    for index in 0..64 {
        sqlx::query("INSERT INTO browser_sessions VALUES (?, ?, ?, unixepoch() + 3600)")
            .bind(format!("hash{index}"))
            .bind(user.id)
            .bind(&hash)
            .execute(state.db())
            .await
            .unwrap();
    }
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/auth/session",
            &basic_auth("admin", "password"),
            json!({})
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    sqlx::query("UPDATE browser_sessions SET expires_at = 0")
        .execute(state.db())
        .await
        .unwrap();
    login(&app, "admin", false).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM browser_sessions")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(count, 1);
}
