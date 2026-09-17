mod support;

use axum::http::StatusCode;
use support::{TestApp, empty_request, response_json};
use tower::ServiceExt;

#[tokio::test]
async fn public_stats_returns_storage_and_uptime_for_authenticated_users() {
    let app = TestApp::new().await;
    app.seed_readonly("viewer", "password123").await;
    let rom_id = app.seed_rom("nes", "Storage Test", "storage-test").await;
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots ORDER BY id LIMIT 1")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes) VALUES (?, ?, 'nes/storage-test.nes', 'storage-test.nes', 123)",
    )
    .bind(rom_id)
    .bind(root_id)
    .execute(app.state.db())
    .await
    .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/stats",
            Some(("viewer", "password123")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let json = response_json(response).await;

    let total = json["storage_total_bytes"]
        .as_u64()
        .expect("total is a u64");
    let used = json["storage_used_bytes"].as_u64().expect("used is a u64");
    let library = json["storage_library_bytes"]
        .as_u64()
        .expect("library is a u64");
    let other = json["storage_other_bytes"]
        .as_u64()
        .expect("other is a u64");
    let free = json["storage_free_bytes"].as_u64().expect("free is a u64");

    assert!(total > 0, "library-root filesystem reports a total size");
    assert_eq!(library, 123, "indexed file bytes are library storage");
    assert_eq!(other + library, used, "other and library partition used");
    assert_eq!(other + library + free, total, "segments partition total");
    assert!(json["uptime_seconds"].as_i64().expect("uptime is an i64") >= 0);
}

#[tokio::test]
async fn public_stats_requires_authentication() {
    let app = TestApp::new().await;

    let response = app
        .router
        .clone()
        .oneshot(empty_request("GET", "/api/stats", None))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
