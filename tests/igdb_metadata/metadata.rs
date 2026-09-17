#[tokio::test]
async fn igdb_match_saves_metadata_and_caches_cover_assets() {
    let mock_igdb = MockIgdb::spawn().await;
    let test_app = support::TestApp::with_config(|config| config.igdb = mock_igdb.config()).await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let rom_id = test_app
        .seed_rom("genesis", "Sonic Pending", "sonic")
        .await;
    let app = test_app.router.clone();

    let request_body = json!({
        "match": {
            "id": 123,
            "name": "Sonic The Hedgehog",
            "summary": "A fast platform game.",
            "first_release_date": 662688000,
            "release_year": 1991,
            "genres": ["Platform"],
            "developers": ["Sega"],
            "publishers": ["Sega"],
            "cover_image_id": "co123",
            "cover_url": format!("{}/images/t_cover_big/co123.jpg", mock_igdb.base_url)
        },
        "cache_cover": true
    });

    let response = app
        .clone()
        .oneshot(json_request(
            &format!("/api/admin/roms/{rom_id}/metadata/igdb"),
            Some(("admin", "admin-password")),
            request_body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let applied = response_json(response).await;
    assert_eq!(applied["rom"]["metadatum"]["igdb_id"], 123);
    assert_eq!(applied["rom"]["metadatum"]["release_year"], 1991);
    assert_eq!(applied["cached_covers"].as_array().unwrap().len(), 2);

    let large_path = applied["rom"]["path_cover_large"]
        .as_str()
        .unwrap()
        .to_string();
    let small_path = applied["rom"]["path_cover_small"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        large_path,
        format!("library/genesis/{rom_id}-sonic/cover-large.jpg")
    );
    assert_eq!(
        small_path,
        format!("library/genesis/{rom_id}-sonic/cover-small.jpg")
    );
    assert_eq!(
        fs::read(temp_dir.path().join("assets").join(&large_path)).unwrap(),
        b"large-cover"
    );
    assert_eq!(
        fs::read(temp_dir.path().join("assets").join(&small_path)).unwrap(),
        b"small-cover"
    );

    let cover_asset_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cover_assets WHERE rom_id = ?")
            .bind(rom_id)
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(cover_asset_rows, 2);

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    assert_eq!(detail["summary"], "A fast platform game.");
    assert_eq!(detail["metadatum"]["source"], "igdb");
    assert_eq!(detail["metadatum"].get("metadata"), None);
    assert_eq!(detail["path_cover_large"], large_path.as_str());

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/assets/romm/resources/{large_path}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    let bytes = support::response_bytes(response).await;
    assert_eq!(&bytes[..], b"large-cover");

    let response = app
        .clone()
        .oneshot(patch_json_request(
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
            json!({"name": "Sonic Renamed"}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            &format!("/api/admin/roms/{rom_id}/metadata/igdb"),
            Some(("admin", "admin-password")),
            request_body,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reapplied = response_json(response).await;
    let renamed_large_path = reapplied["rom"]["path_cover_large"].as_str().unwrap();
    let renamed_small_path = reapplied["rom"]["path_cover_small"].as_str().unwrap();
    assert!(renamed_large_path.contains("sonic-renamed"));
    assert!(!temp_dir.path().join("assets").join(&large_path).exists());
    assert!(!temp_dir.path().join("assets").join(&small_path).exists());
    assert!(
        temp_dir
            .path()
            .join("assets")
            .join(renamed_large_path)
            .exists()
    );
    let cover_asset_rows: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cover_assets WHERE rom_id = ?")
            .bind(rom_id)
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(cover_asset_rows, 2);

    let response = app
        .oneshot(delete_request(
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !temp_dir
            .path()
            .join("assets")
            .join(renamed_large_path)
            .exists()
    );
    assert!(
        !temp_dir
            .path()
            .join("assets")
            .join(renamed_small_path)
            .exists()
    );
}
