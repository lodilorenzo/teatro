//! Contract and security tests for the admin RomM source routes.
//!
//! These cover the Teatro side only. Behavior against a real RomM server depends on Phase 0
//! evidence that has not been captured yet.

mod support;

use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use support::{TestApp, empty_request, json_request, response_json, response_text};
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn enabled_app() -> TestApp {
    TestApp::with_config(|config| config.romm_source.enabled = true).await
}

#[tokio::test]
async fn every_route_is_absent_while_the_feature_is_disabled() {
    let app = TestApp::new().await;
    let admin = app.seed_admin("admin", "admin-password").await;

    for (method, path) in [
        ("GET", "/api/admin/sources/romm/status"),
        ("POST", "/api/admin/sources/romm/test"),
        ("POST", "/api/admin/sources/romm/refresh"),
        ("GET", "/api/admin/sources/romm/platforms"),
        ("GET", "/api/admin/sources/romm/roms"),
        ("GET", "/api/admin/sources/romm/roms/1"),
        ("GET", "/api/admin/sources/romm/roms/1/cover"),
        ("GET", "/api/admin/sources/romm/imports/romm_abc"),
        ("DELETE", "/api/admin/sources/romm/settings"),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                method,
                path,
                Some((&admin.username, &admin.password)),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} {path} must not exist while the feature is disabled"
        );
    }
}

#[tokio::test]
async fn read_only_users_cannot_reach_the_source() {
    let app = enabled_app().await;
    let reader = app.seed_readonly("reader", "reader-password").await;

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/sources/romm/status",
            Some((&reader.username, &reader.password)),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn settings_reject_unsafe_base_urls_and_never_return_the_secret() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    for hostile in [
        "file:///etc/passwd",
        "ftp://romm.example/",
        "http://user:secret@romm.example/",
        "not a url",
    ] {
        let response = app
            .router
            .clone()
            .oneshot(json_request(
                "PATCH",
                "/api/admin/sources/romm/settings",
                auth,
                serde_json::json!({
                    "base_url": hostile,
                    "username": "teatro",
                    "secret": "super-secret-value",
                    "acknowledge_plaintext_http": true,
                }),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{hostile} must be refused"
        );
    }

    // A plaintext origin requires an explicit acknowledgement before a credential is stored.
    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": "http://romm.example:8080",
                "username": "teatro",
                "secret": "super-secret-value",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": "http://romm.example:8080",
                "username": "teatro",
                "secret": "super-secret-value",
                "acknowledge_plaintext_http": true,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let saved = response_json(response).await;
    assert_eq!(saved["configured"], true);
    assert_eq!(saved["secret_configured"], true);
    assert_eq!(saved["plaintext_http"], true);
    assert_eq!(saved["base_url"], "http://romm.example:8080/");
    assert_eq!(saved["auth_mode"], "token");
    assert!(saved.get("secret").is_none());

    let response = app
        .router
        .clone()
        .oneshot(empty_request("GET", "/api/admin/sources/romm/status", auth))
        .await
        .unwrap();
    let body = response_text(response).await;
    assert!(
        !body.contains("super-secret-value"),
        "the stored secret must never be returned"
    );

    // The audit trail records the lifecycle without the credential or its endpoint.
    let audit: Vec<(String, Option<String>)> =
        sqlx::query_as("SELECT action, metadata_json FROM audit_log ORDER BY id")
            .fetch_all(app.state.db())
            .await
            .unwrap();
    assert!(
        audit
            .iter()
            .any(|(action, _)| action == "romm_source.settings_saved")
    );
    for (_, metadata) in &audit {
        let metadata = metadata.clone().unwrap_or_default();
        assert!(!metadata.contains("super-secret-value"));
        assert!(!metadata.contains("romm.example"));
    }
}

#[tokio::test]
async fn browse_and_import_require_a_configured_source() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/sources/romm/platforms",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/sources/romm/imports",
            auth,
            serde_json::json!({
                "remote_rom_id": 1,
                "remote_file_ids": [1],
                "platform_id": 1,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn browse_queries_and_import_bodies_are_bounded() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    app.router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": "https://romm.example",
                "username": "teatro",
                "secret": "super-secret-value",
            }),
        ))
        .await
        .unwrap();

    // A page above the frozen 48-entry ceiling is refused before any upstream call.
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/sources/romm/roms?limit=500",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // An import naming no files is refused without reserving a job.
    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/sources/romm/imports",
            auth,
            serde_json::json!({
                "remote_rom_id": 1,
                "remote_file_ids": [],
                "platform_id": 1,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // More files than TEATRO_ROMM_IMPORT_MAX_FILES is refused the same way.
    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/sources/romm/imports",
            auth,
            serde_json::json!({
                "remote_rom_id": 1,
                "remote_file_ids": (1..=200).collect::<Vec<i64>>(),
                "platform_id": 1,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn refresh_downloads_every_remote_page_and_browsing_stays_local() {
    #[derive(Clone)]
    struct MockState {
        calls: Arc<AtomicUsize>,
    }

    async fn platforms(State(state): State<MockState>) -> Json<serde_json::Value> {
        state.calls.fetch_add(1, Ordering::Relaxed);
        Json(serde_json::json!({
            "items": [{ "id": 4, "slug": "snes", "name": "Super Nintendo", "rom_count": 55 }]
        }))
    }

    async fn roms(
        State(state): State<MockState>,
        Query(query): Query<HashMap<String, String>>,
    ) -> Json<serde_json::Value> {
        state.calls.fetch_add(1, Ordering::Relaxed);
        let offset = query
            .get("offset")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        let limit = query
            .get("limit")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(48);
        let items = (offset..55)
            .take(limit)
            .map(|index| {
                serde_json::json!({
                    "id": index + 1,
                    "name": format!("Remote Game {index:02}"),
                    "platform_id": 4,
                    "platform_slug": "snes",
                    "platform_name": "Super Nintendo",
                    "fs_name": format!("game-{index:02}.sfc"),
                    "fs_size_bytes": 1024,
                })
            })
            .collect::<Vec<_>>();
        Json(serde_json::json!({ "items": items, "total": 55 }))
    }

    let calls = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let mock = Router::new()
        .route("/api/platforms", get(platforms))
        .route("/api/roms", get(roms))
        .with_state(MockState {
            calls: calls.clone(),
        });
    let server = tokio::spawn(async move {
        axum::serve(listener, mock).await.unwrap();
    });

    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    app.router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": base_url,
                "username": "teatro",
                "secret": "secret",
                "auth_mode": "basic",
                "acknowledge_plaintext_http": true,
            }),
        ))
        .await
        .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "POST",
            "/api/admin/sources/romm/refresh",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = response_json(response).await;
    assert_eq!(status["index_game_count"], 55);
    assert_eq!(status["index_platform_count"], 1);
    assert!(status["index_refreshed_at"].is_string());
    assert_eq!(calls.load(Ordering::Relaxed), 3); // platforms plus two game pages

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/sources/romm/roms?search=remote+game+54&limit=24&offset=0",
            auth,
        ))
        .await
        .unwrap();
    let page = response_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["name"], "Remote Game 54");
    assert_eq!(calls.load(Ordering::Relaxed), 3);

    server.abort();
}

#[tokio::test]
async fn browse_reads_the_complete_local_index_without_contacting_romm() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    app.router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": "http://127.0.0.1:9",
                "username": "teatro",
                "secret": "super-secret-value",
                "auth_mode": "basic",
                "acknowledge_plaintext_http": true,
            }),
        ))
        .await
        .unwrap();

    sqlx::query("INSERT INTO romm_remote_platforms (id, slug, name, rom_count) VALUES (4, 'snes', 'Super Nintendo', 2)")
        .execute(app.state.db())
        .await
        .unwrap();
    for (id, name) in [(91_i64, "Chrono Trigger"), (92, "Super Metroid")] {
        sqlx::query(
            r#"
            INSERT INTO romm_remote_roms
                (id, name, platform_id, platform_slug, platform_name, fs_name,
                 file_size_bytes, has_cover, file_count)
            VALUES (?, ?, 4, 'snes', 'Super Nintendo', ?, 1024, 0, 1)
            "#,
        )
        .bind(id)
        .bind(name)
        .bind(format!("{name}.sfc"))
        .execute(app.state.db())
        .await
        .unwrap();
    }
    sqlx::query("INSERT INTO romm_remote_index (id, refreshed_at, game_count, platform_count) VALUES (1, '2026-08-06T00:00:00Z', 2, 1)")
        .execute(app.state.db())
        .await
        .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/sources/romm/roms?search=chrono&limit=24&offset=0",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["name"], "Chrono Trigger");

    let response = app
        .router
        .clone()
        .oneshot(empty_request("GET", "/api/admin/sources/romm/status", auth))
        .await
        .unwrap();
    let status = response_json(response).await;
    assert_eq!(status["index_game_count"], 2);
    assert_eq!(status["index_platform_count"], 1);
    assert_eq!(status["index_refreshed_at"], "2026-08-06T00:00:00Z");
}

#[tokio::test]
async fn unknown_import_jobs_are_not_found() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    for id in ["romm_missing", "scan_wrongprefix", "../../etc/passwd"] {
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/admin/sources/romm/imports/{id}"),
                auth,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{id}");
    }
}

#[tokio::test]
async fn clearing_the_source_removes_its_credentials() {
    let app = enabled_app().await;
    let admin = app.seed_admin("admin", "admin-password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    app.router
        .clone()
        .oneshot(json_request(
            "PATCH",
            "/api/admin/sources/romm/settings",
            auth,
            serde_json::json!({
                "base_url": "https://romm.example",
                "username": "teatro",
                "secret": "super-secret-value",
            }),
        ))
        .await
        .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            "/api/admin/sources/romm/settings",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cleared = response_json(response).await;
    assert_eq!(cleared["configured"], false);
    assert_eq!(cleared["secret_configured"], false);
    assert_eq!(cleared["base_url"], serde_json::Value::Null);

    let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM romm_source_settings")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}
