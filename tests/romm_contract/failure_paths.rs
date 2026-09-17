#[tokio::test]
async fn persisted_rom_without_launchable_file_fails_download_plan_closed() {
    let test_app = support::TestApp::new().await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "reader", "reader-password", UserRole::ReadOnly).await;
    let seeded = seed_rom(&state, temp_dir, "genesis", "Malformed", "malformed").await;
    sqlx::query("UPDATE rom_files SET group_id = NULL, launchable = 0 WHERE id = ?")
        .bind(seeded.file_id)
        .execute(state.db())
        .await
        .unwrap();
    let app = test_app.router.clone();

    let response = app
        .oneshot(empty_request(
            &format!("/api/roms/{}/download-plan", seeded.rom_id),
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let error = response_json(response).await;
    assert_eq!(error["error"]["code"], "internal_server_error");
}

#[tokio::test]
async fn download_resources_return_normal_not_found_envelopes() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "reader", "reader-password", UserRole::ReadOnly).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/roms/999999/download-plan",
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error = response_json(response).await;
    assert_eq!(error["error"]["code"], "not_found");
    assert_eq!(error["error"]["message"], "ROM not found");

    let response = app
        .oneshot(empty_request(
            "/api/roms/999999/archive",
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let error = response_json(response).await;
    assert_eq!(error["error"]["code"], "not_found");
    assert_eq!(error["error"]["message"], "route not found");
}

#[tokio::test]
async fn archive_download_enforces_the_configured_source_byte_limit() {
    let test_app = support::TestApp::with_config(|config| {
        config.download_archives.max_source_bytes = 1;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "reader", "reader-password", UserRole::ReadOnly).await;
    let seeded = seed_rom(&state, temp_dir, "genesis", "Archive Limit", "archive-limit").await;
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    fs::write(temp_dir.path().join("roms/genesis/sidecar.txt"), b"x").unwrap();
    sqlx::query(
        r#"
        INSERT INTO rom_files (
            rom_id, root_id, relative_path, file_name, file_size_bytes,
            original_file_name, role, launchable
        )
        VALUES (?, ?, 'genesis/sidecar.txt', 'sidecar.txt', 1, 'sidecar.txt', 'manual', 0)
        "#,
    )
    .bind(seeded.rom_id)
    .bind(root_id)
    .execute(state.db())
    .await
    .unwrap();

    let response = test_app
        .router
        .oneshot(support::empty_request(
            "POST",
            &format!("/api/roms/{}/archive-ticket", seeded.rom_id),
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let error = response_json(response).await;
    assert_eq!(error["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn asset_route_serves_files_and_rejects_traversal() {
    let test_app = support::TestApp::new().await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;

    let asset_dir = temp_dir.path().join("assets/covers");
    fs::create_dir_all(&asset_dir).unwrap();
    fs::write(asset_dir.join("sonic.webp"), b"webp").unwrap();
    fs::write(temp_dir.path().join("secret.txt"), b"secret").unwrap();

    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/assets/romm/resources/covers/sonic.webp",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/webp");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"webp");

    let response = app
        .oneshot(empty_request(
            "/assets/romm/resources/%2E%2E/secret.txt",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn download_rejects_relative_paths_that_escape_library_root() {
    let test_app = support::TestApp::new().await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let seeded = seed_rom(&state, temp_dir, "genesis", "Bad Path", "bad-path").await;

    fs::write(temp_dir.path().join("secret.bin"), b"secret").unwrap();
    sqlx::query(
        r#"
        UPDATE rom_files
        SET relative_path = '../secret.bin'
        WHERE id = ?
        "#,
    )
    .bind(seeded.file_id)
    .execute(state.db())
    .await
    .unwrap();

    let app = test_app.router.clone();
    let response = app
        .oneshot(empty_request(
            &format!(
                "/api/roms/{}/content/secret.bin?file_ids={}",
                seeded.rom_id, seeded.file_id
            ),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
