mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::json;
use support::{response_json, seed_user_record as seed_user};
use teatro::{config::AuthConfig, domain::user::UserRole};
use tower::ServiceExt;

#[tokio::test]
async fn rate_limiter_blocks_repeated_failed_auth_and_audits_metadata() {
    let test_app = support::TestApp::with_config(|config| {
        config.auth = AuthConfig {
            rate_limit_max_failures: 2,
            rate_limit_window_seconds: 300,
            rate_limit_lockout_seconds: 60,
            ..AuthConfig::default()
        };
    })
    .await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("admin", "wrong-password")),
            Some("10.0.0.1"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("admin", "wrong-password")),
            Some("10.0.0.1"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[header::RETRY_AFTER], "60");

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("admin", "admin-password")),
            Some("10.0.0.1"),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);

    let rows = sqlx::query_as::<_, (String,)>(
        r#"
        SELECT metadata_json
        FROM audit_log
        WHERE action = 'auth.failed'
        ORDER BY id
        "#,
    )
    .fetch_all(state.db())
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);

    let second: serde_json::Value = serde_json::from_str(&rows[1].0).unwrap();
    assert_eq!(second["auth_method"], "basic");
    assert_eq!(second["username"], "admin");
    assert_eq!(second["reason"], "invalid_password");
    assert_eq!(second["client_id"], "unknown");
    assert_eq!(second["failure_count"], 2);
    assert!(second["locked_until"].is_string());

    let third: serde_json::Value = serde_json::from_str(&rows[2].0).unwrap();
    assert_eq!(third["reason"], "rate_limited");
    let retry_after = third["retry_after_seconds"].as_u64().unwrap();
    assert!((1..=60).contains(&retry_after));
}

#[tokio::test]
async fn bounded_password_queue_handles_admin_refresh_fan_out() {
    let test_app = support::TestApp::with_config(|config| {
        config.auth = AuthConfig {
            password_concurrency: 1,
            password_queue_depth: 4,
            password_queue_timeout_seconds: 10,
            ..AuthConfig::default()
        };
    })
    .await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let (platforms, stats, igdb_status, igdb_settings, roms) = tokio::join!(
        app.clone().oneshot(empty_request(
            "GET",
            "/api/platforms",
            Some(("admin", "admin-password")),
            None,
        )),
        app.clone().oneshot(empty_request(
            "GET",
            "/api/admin/stats",
            Some(("admin", "admin-password")),
            None,
        )),
        app.clone().oneshot(empty_request(
            "GET",
            "/api/admin/igdb/status",
            Some(("admin", "admin-password")),
            None,
        )),
        app.clone().oneshot(empty_request(
            "GET",
            "/api/admin/igdb/settings",
            Some(("admin", "admin-password")),
            None,
        )),
        app.oneshot(empty_request(
            "GET",
            "/api/roms",
            Some(("admin", "admin-password")),
            None,
        )),
    );

    for response in [platforms, stats, igdb_status, igdb_settings, roms] {
        assert_eq!(response.unwrap().status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn admin_can_create_use_list_and_revoke_bearer_api_tokens() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/tokens",
            Some(("admin", "admin-password")),
            None,
            json!({
                "name": "Read-only integration token",
                "scopes": ["read"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response_json(response).await;
    let read_token_id = created["id"].as_i64().unwrap();
    let read_token = created["token"].as_str().unwrap().to_string();
    assert!(read_token.starts_with("teatro_pat_"));
    assert_eq!(created["scopes"], json!(["read"]));

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/users/me", None, None).with_bearer(&read_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let me = response_json(response).await;
    assert_eq!(me["username"], "admin");
    assert_eq!(me["role"], "readonly");
    let first_last_used: Option<String> =
        sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id = ?")
            .bind(read_token_id)
            .fetch_one(state.db())
            .await
            .unwrap();
    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/users/me", None, None).with_bearer(&read_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second_last_used: Option<String> =
        sqlx::query_scalar("SELECT last_used_at FROM api_tokens WHERE id = ?")
            .bind(read_token_id)
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(first_last_used, second_last_used);

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/users", None, None).with_bearer(&read_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/tokens",
            Some(("admin", "admin-password")),
            None,
            json!({
                "name": "Admin integration token",
                "scopes": ["admin"]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response_json(response).await;
    let admin_token = created["token"].as_str().unwrap().to_string();
    assert_eq!(created["scopes"], json!(["admin", "read"]));

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/api/admin/users", None, None).with_bearer(&admin_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/tokens",
            Some(("admin", "admin-password")),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let tokens = response_json(response).await;
    let token_rows = tokens.as_array().unwrap();
    assert_eq!(token_rows.len(), 2);
    assert!(token_rows.iter().all(|token| token.get("token").is_none()));
    assert!(
        token_rows
            .iter()
            .any(|token| { token["id"] == read_token_id && token["last_used_at"].is_string() })
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/tokens/{read_token_id}"),
            Some(("admin", "admin-password")),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let revoked = response_json(response).await;
    assert_eq!(revoked["id"], read_token_id);
    assert!(revoked["revoked_at"].is_string());

    let response = app
        .oneshot(empty_request("GET", "/api/users/me", None, None).with_bearer(&read_token))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

trait RequestBearerExt {
    fn with_bearer(self, token: &str) -> Self;
}

impl RequestBearerExt for Request<Body> {
    fn with_bearer(mut self, token: &str) -> Self {
        self.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        self
    }
}

fn empty_request(
    method: &str,
    uri: &str,
    auth: Option<(&str, &str)>,
    forwarded_for: Option<&str>,
) -> Request<Body> {
    let mut request = support::empty_request(method, uri, auth);
    if let Some(forwarded_for) = forwarded_for {
        request.headers_mut().insert(
            "x-forwarded-for",
            forwarded_for
                .parse()
                .expect("forwarded test header should be valid"),
        );
    }
    request
}

fn json_request(
    method: &str,
    uri: &str,
    auth: Option<(&str, &str)>,
    forwarded_for: Option<&str>,
    body: serde_json::Value,
) -> Request<Body> {
    let mut request = support::json_request(method, uri, auth, body);
    if let Some(forwarded_for) = forwarded_for {
        request.headers_mut().insert(
            "x-forwarded-for",
            forwarded_for
                .parse()
                .expect("forwarded test header should be valid"),
        );
    }
    request
}
