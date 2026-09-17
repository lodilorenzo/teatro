#[tokio::test]
async fn upload_preview_skips_matching_names_only_for_the_selected_platform() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "Already There.bin",
            b"existing",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/uploads/preview",
            Some(("admin", "admin-password")),
            json!({
                "platform_slug": "genesis",
                "files": [
                    {"file_name": "Already There.bin", "file_size_bytes": 8},
                    {"file_name": "Fresh Game.bin", "file_size_bytes": 5}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = response_json(response).await;
    assert_eq!(preview["warnings"][0]["code"], "file_already_exists");
    assert_eq!(preview["warnings"][0]["file_name"], "Already There.bin");
    let planned_file_names = preview["roms"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|rom| rom["files"].as_array().unwrap())
        .map(|file| file["original_file_name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(planned_file_names, vec!["Fresh Game.bin"]);

    let response = app
        .oneshot(json_request(
            "POST",
            "/api/admin/uploads/preview",
            Some(("admin", "admin-password")),
            json!({
                "platform_slug": "ps2",
                "files": [
                    {"file_name": "Already There.bin", "file_size_bytes": 8}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = response_json(response).await;
    assert!(preview["warnings"].as_array().unwrap().iter().all(|warning| {
        warning["code"] != "file_already_exists"
    }));
    assert_eq!(
        preview["roms"][0]["files"][0]["original_file_name"],
        "Already There.bin"
    );
}

#[tokio::test]
async fn upload_preview_rejects_an_oversized_batch_before_receiving_file_bytes() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 10;
        config.uploads.max_batch_bytes = 4;
    })
    .await;
    seed_user(
        &test_app.state,
        "admin",
        "admin-password",
        UserRole::Admin,
    )
    .await;

    let response = test_app
        .router
        .oneshot(json_request(
            "POST",
            "/api/admin/uploads/preview",
            Some(("admin", "admin-password")),
            json!({
                "platform_slug": "genesis",
                "files": [{"file_name": "Too Large.bin", "file_size_bytes": 5}]
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response_json(response).await["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn batch_upload_replays_a_completed_background_transfer_without_duplicate_ingest() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let request = || {
        let mut request = batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            &[("Idempotent Game.bin", b"game")],
        );
        request.headers_mut().insert(
            "x-teatro-transfer-id",
            "upload_navigation_1".parse().unwrap(),
        );
        request
    };

    let first = app.clone().oneshot(request()).await.unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = response_json(first).await;
    let second = app.oneshot(request()).await.unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
    assert_eq!(response_json(second).await, first);

    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 1);
}

#[tokio::test]
async fn batch_upload_rejects_title_edits_that_do_not_match_the_regenerated_plan() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[
                ("platform_slug", "genesis"),
                (
                    "planned_title",
                    r#"{"plan_id":"rom-1","title":"Only One Override"}"#,
                ),
            ],
            &[("First Game.bin", b"first"), ("Second Game.bin", b"second")],
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("planned titles do not match")
    );
    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 0);
    assert_eq!(
        fs::read_dir(temp_dir.path().join("roms/.uploads"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn batch_upload_rejects_missing_manifest_dependencies_and_split_archives() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let cue = br#"FILE "Missing Track.bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
"#;
    let response = app
        .clone()
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[("platform_slug", "ps2")],
            &[("Missing Dependency.cue", &cue[..])],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 0);

    let response = app
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[("platform_slug", "ps2")],
            &[
                ("Large Game.7z.001", b"part-one"),
                ("Large Game.7z.002", b"part-two"),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 0);
}
