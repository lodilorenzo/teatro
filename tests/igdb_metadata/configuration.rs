#[tokio::test]
async fn igdb_status_and_search_use_mocked_upstream() {
    let mock_igdb = MockIgdb::spawn().await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/status",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = response_json(response).await;
    assert_eq!(status["configured"], true);
    assert_eq!(status["client_id_configured"], true);
    assert_eq!(status["client_secret_configured"], true);
    assert_eq!(status["token_cached"], false);

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&limit=5&platform=genesis",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let results = response_json(response).await;
    let first = &results.as_array().unwrap()[0];
    assert_eq!(first["id"], 123);
    assert_eq!(first["name"], "Sonic The Hedgehog");
    assert_eq!(first["release_year"], 1991);
    assert_eq!(first["genres"], json!(["Platform"]));
    assert_eq!(first["developers"], json!(["Sega"]));
    assert_eq!(first["publishers"], json!(["Sega"]));
    assert_eq!(
        first["cover_url"],
        format!("{}/images/t_cover_big/co123.jpg", mock_igdb.base_url)
    );

    let requests = mock_igdb.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].contains("search \"Sonic\";"));
    assert!(requests[0].contains("platforms.id"));
    assert!(requests[0].contains("where platforms = (29);"));
    assert!(requests[0].contains("limit 5;"));
    assert_eq!(first.get("platform_ids"), None);
    assert_eq!(first.get("platform_keys"), None);
}

#[tokio::test]
async fn igdb_search_retries_all_platforms_when_scoped_search_is_empty() {
    let mock_igdb = MockIgdb::spawn_with_scoped_miss().await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;

    let response = test_app
        .router
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&limit=5&platform=genesis&require_platform_match=true",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await[0]["id"], 123);

    let requests = mock_igdb.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].contains("where platforms = (29);"));
    assert!(!requests[1].contains("where platforms"));
}

#[tokio::test]
async fn igdb_search_retries_all_platforms_after_mapped_platform_misses() {
    let mock_igdb = MockIgdb::spawn_with_wrong_platform().await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&limit=5&platform=genesis&require_platform_match=true",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await[0]["id"], 123);

    let response = app
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Game&limit=5&platform=n3ds",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await[0]["id"], 123);

    let requests = mock_igdb.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].contains("where platforms = (29);"));
    assert!(!requests[1].contains("where platforms"));
    assert!(requests[2].contains("where platforms = (37,137);"));
    assert!(!requests[3].contains("where platforms"));
}

#[tokio::test]
async fn unmapped_platform_searches_fall_back_to_all_platforms() {
    let mock_igdb = MockIgdb::spawn().await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&platform=sgb&require_platform_match=true",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await[0]["id"], 123);
    assert_eq!(mock_igdb.requests.lock().unwrap().len(), 1);

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&platform=sgb",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await[0]["id"], 123);

    let response = app
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&require_platform_match=true",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let requests = mock_igdb.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|request| !request.contains("where platforms")));
}

#[tokio::test]
async fn igdb_search_rejects_oversized_upstream_responses() {
    let mock_igdb = MockIgdb::spawn_with_oversized_games(true).await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&limit=5",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "bad_gateway");
}

#[tokio::test]
async fn admin_can_save_write_only_igdb_credentials() {
    let mock_igdb = MockIgdb::spawn().await;
    let mut igdb = mock_igdb.config();
    igdb.client_id = None;
    igdb.client_secret = None;
    let test_app = support::TestApp::with_config(|config| config.igdb = igdb).await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/settings",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings = response_json(response).await;
    assert_eq!(settings["configured"], false);
    assert_eq!(settings["client_id_source"], "none");
    assert_eq!(settings["client_secret_source"], "none");
    assert_eq!(settings.get("client_secret"), None);

    let response = app
        .clone()
        .oneshot(patch_json_request(
            "/api/admin/igdb/settings",
            Some(("admin", "admin-password")),
            json!({
                "client_id": "test-client",
                "client_secret": "test-secret"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings = response_json(response).await;
    assert_eq!(settings["configured"], true);
    assert_eq!(settings.get("client_id"), None);
    assert_eq!(settings["client_id_source"], "database");
    assert_eq!(settings["client_secret_source"], "database");
    assert_eq!(settings.get("client_secret"), None);

    let stored_secret: String =
        sqlx::query_scalar("SELECT client_secret FROM igdb_settings WHERE id = 1")
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(stored_secret, "test-secret");

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/status",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = response_json(response).await;
    assert_eq!(status["configured"], true);

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/admin/igdb/search?q=Sonic&limit=5",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let results = response_json(response).await;
    assert_eq!(results[0]["name"], "Sonic The Hedgehog");

    let response = app
        .clone()
        .oneshot(delete_request(
            "/api/admin/igdb/settings",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings = response_json(response).await;
    assert_eq!(settings["configured"], false);
    assert_eq!(settings["client_id_source"], "none");
    assert_eq!(settings["client_secret_source"], "none");

    let stored_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM igdb_settings WHERE id = 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(stored_rows, 0);
}
