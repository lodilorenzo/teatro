#[tokio::test]
async fn upload_rejects_oversized_files_traversal_filenames_and_readonly_users() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 4;
    })
    .await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    seed_user(&state, "reader", "reader-password", UserRole::ReadOnly).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("reader", "reader-password")),
            &[("platform_slug", "genesis")],
            "game.bin",
            b"rom",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "big.bin",
            b"12345",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "../secret.bin",
            b"rom",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(empty_request(
            "GET",
            "/api/admin/stats",
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn package_delete_preserves_a_folder_shared_with_another_indexed_rom() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let root_path = state.config().default_library_root.join("psx/shared");
    fs::create_dir_all(&root_path).unwrap();
    for (name, bytes) in [
        ("first-a.bin", b"a".as_slice()),
        ("first-b.bin", b"b".as_slice()),
        ("second.bin", b"second".as_slice()),
        (".Identifier", b"sidecar".as_slice()),
    ] {
        fs::write(root_path.join(name), bytes).unwrap();
    }
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'psx'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    let first_rom = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'First', 'first', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let second_rom = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'Second', 'second', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    for (rom_id, relative_path, file_name) in [
        (first_rom, "psx/shared/first-a.bin", "first-a.bin"),
        (first_rom, "psx/shared/first-b.bin", "first-b.bin"),
        (second_rom, "psx/shared/second.bin", "second.bin"),
    ] {
        sqlx::query(
            "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary) VALUES (?, ?, ?, ?, 1, 0)",
        )
        .bind(rom_id)
        .bind(root_id)
        .bind(relative_path)
        .bind(file_name)
        .execute(state.db())
        .await
        .unwrap();
    }

    let response = test_app
        .router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{first_rom}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["deleted_files"].as_array().unwrap().len(), 2);
    assert!(!root_path.join("first-a.bin").exists());
    assert!(!root_path.join("first-b.bin").exists());
    assert_eq!(fs::read(root_path.join("second.bin")).unwrap(), b"second");
    assert_eq!(fs::read(root_path.join(".Identifier")).unwrap(), b"sidecar");
}

#[tokio::test]
async fn delete_files_rejects_stored_paths_that_escape_library_root() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let rom_id = seed_malicious_rom(&state).await;
    fs::write(temp_dir.path().join("secret.bin"), b"secret").unwrap();
    let app = test_app.router.clone();

    let response = app
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}?delete_files=true"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        fs::read(temp_dir.path().join("secret.bin")).unwrap(),
        b"secret"
    );

    let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms WHERE id = ?")
        .bind(rom_id)
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(row_count, 1, "unsafe delete should leave the DB row intact");
}
