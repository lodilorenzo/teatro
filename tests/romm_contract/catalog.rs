#[tokio::test]
async fn platforms_are_seeded_and_require_auth() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let unauthenticated = app
        .clone()
        .oneshot(empty_request("/api/platforms", None))
        .await
        .unwrap();
    assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/platforms",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let platforms = response_json(response).await;
    let platforms = platforms.as_array().unwrap();

    assert_eq!(platforms.len(), 62);
    let stored_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM platforms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(stored_count, 62);
    let mut icon_slugs = std::fs::read_dir("web/public/platform-icons")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "png"))
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_string())
        .collect::<Vec<_>>();
    icon_slugs.sort();
    let mut catalog_slugs = platforms
        .iter()
        .map(|platform| platform["slug"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    catalog_slugs.sort();
    assert_eq!(icon_slugs, catalog_slugs);
    for slug in &catalog_slugs {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/public/platform-icons/{slug}.png"), None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{slug}");
    }
    assert!(platforms.iter().all(|platform| {
        platform["slug"].as_str().is_some()
            && platform["slug"].as_str() == platform["fs_slug"].as_str()
    }));
    for (slug, display_name) in [
        ("3do", "3DO"),
        ("amigacd32", "Commodore Amiga CD32"),
        ("atarijaguar", "Atari Jaguar"),
        ("atarijaguarcd", "Atari Jaguar CD"),
        ("atarilynx", "Atari Lynx"),
        ("genesis", "Sega Genesis"),
        ("n3ds", "Nintendo 3DS"),
        ("psx", "Sony PlayStation"),
        ("switch", "Nintendo Switch"),
        ("wiiu", "Nintendo Wii U"),
        ("win", "Windows"),
        ("xbox360", "Microsoft Xbox 360"),
    ] {
        assert!(platforms.iter().any(|platform| {
            platform["slug"] == slug && platform["display_name"] == display_name
        }));
    }
    for &removed_slug in support::REMOVED_PLATFORM_SLUGS {
        assert!(
            platforms
                .iter()
                .all(|platform| platform["slug"] != removed_slug),
            "retired platform is still supported: {removed_slug}"
        );
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM platforms WHERE slug = ?)")
            .bind(removed_slug)
            .fetch_one(state.db())
            .await
            .unwrap();
        assert!(!exists, "retired platform is still stored: {removed_slug}");
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/public/platform-icons/{removed_slug}.png"), None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{removed_slug}");
    }
    let windows = platforms
        .iter()
        .find(|platform| platform["slug"] == "win")
        .unwrap();
    assert_eq!(windows["name"], "Windows");
    assert_eq!(windows["fs_slug"], "win");
}

#[tokio::test]
async fn windows_archive_upload_exposes_sipario_install_contract() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();
    let archive_bytes = b"test Windows archive bytes";

    let response = app
        .clone()
        .oneshot(support::multipart_request(
            "POST",
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[
                ("platform_slug", "win"),
                ("title", "Example Windows Game"),
            ],
            &[("file", "Example Windows Game.zip", archive_bytes)],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let uploaded = response_json(response).await;
    let rom_id = uploaded["rom"]["id"].as_i64().unwrap();
    assert_eq!(uploaded["rom"]["platform_slug"], "win");
    assert_eq!(uploaded["rom"]["platform_display_name"], "Windows");
    assert_eq!(uploaded["relative_path"], "win/Example Windows Game.zip");

    let response = app
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    assert_eq!(detail["platform_slug"], "win");
    assert_eq!(detail["platform_display_name"], "Windows");
    assert_eq!(detail["files"][0]["file_name"], "Example Windows Game.zip");
    assert_eq!(detail["files"][0]["role"], "content");
    assert_eq!(detail["files"][0]["launchable"], true);
    assert_eq!(detail["files"][0]["file_size_bytes"], archive_bytes.len());
}

#[tokio::test]
async fn rom_list_can_sort_and_filter_missing_covers() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let older = seed_rom(
        &state,
        &test_app.temp_dir,
        "genesis",
        "Alpha Older",
        "alpha-older",
    )
    .await;
    let newer = seed_rom(
        &state,
        &test_app.temp_dir,
        "genesis",
        "Zulu Newer",
        "zulu-newer",
    )
    .await;
    sqlx::query("UPDATE roms SET created_at = ? WHERE id = ?")
        .bind("2026-01-01T00:00:00.000Z")
        .bind(older.rom_id)
        .execute(state.db())
        .await
        .unwrap();
    sqlx::query("UPDATE roms SET created_at = ? WHERE id = ?")
        .bind("2026-01-02T00:00:00.000Z")
        .bind(newer.rom_id)
        .execute(state.db())
        .await
        .unwrap();

    let app = test_app.router.clone();
    let response = app
        .clone()
        .oneshot(empty_request(
            "/api/roms?limit=10&offset=0&sort=recent",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await;
    assert_eq!(page["items"][0]["name"], "Zulu Newer");
    assert_eq!(page["items"][1]["name"], "Alpha Older");

    sqlx::query("UPDATE roms SET path_cover_large = NULL WHERE id = ?")
        .bind(older.rom_id)
        .execute(state.db())
        .await
        .unwrap();
    let response = app
        .oneshot(empty_request(
            "/api/roms?limit=10&offset=0&missing_cover=true",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["id"], older.rom_id);
}

#[tokio::test]
async fn seeded_rom_can_be_listed_detailed_and_downloaded() {
    let test_app = support::TestApp::new().await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let seeded = seed_rom(&state, temp_dir, "genesis", "Sonic The Hedgehog", "sonic").await;
    let admin = users::find_by_username(state.db(), "admin")
        .await
        .unwrap()
        .unwrap();
    let raw_read_token = api_token_service::generate_token();
    let token_hash = api_token_service::hash_token(&raw_read_token);
    let token_prefix = api_token_service::token_prefix(&raw_read_token);
    api_tokens::create(
        state.db(),
        api_tokens::CreateApiTokenParams {
            user_id: admin.id,
            name: "Sipario contract reader",
            token_hash: &token_hash,
            token_prefix: &token_prefix,
            scopes: &[ApiTokenScope::Read],
            expires_at: None,
        },
    )
    .await
    .unwrap();
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!(
                "/api/roms?limit=10&offset=0&platform_ids={}",
                seeded.platform_id
            ),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["limit"], 10);
    assert_eq!(page["offset"], 0);
    assert_eq!(page["items"][0]["name"], "Sonic The Hedgehog");
    assert_eq!(page["items"][0]["platform_slug"], "genesis");
    assert_eq!(page["items"][0]["files"][0]["id"], seeded.file_id);
    assert_eq!(page["items"][0]["metadatum"]["release_year"], 1991);
    assert!(
        page["items"][0].get("metadata").is_none(),
        "Teatro must not emit both metadatum and metadata because Sipario aliases metadata to metadatum"
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{}", seeded.rom_id),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let detail = response_json(response).await;
    assert_eq!(detail["id"], seeded.rom_id);
    assert_eq!(detail["metadatum"]["release_year"], 1991);
    assert!(
        detail.get("metadata").is_none(),
        "Teatro must not emit both metadatum and metadata because Sipario aliases metadata to metadatum"
    );
    assert_eq!(detail["files"].as_array().unwrap().len(), 1);

    let response = app
        .clone()
        .oneshot(support::empty_request(
            "POST",
            &format!("/api/roms/{}/archive-ticket", seeded.rom_id),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = response_json(response).await;
    assert_eq!(
        error["error"]["message"],
        "archive downloads require a ROM with more than one file"
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{}/download-plan", seeded.rom_id),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let plan = response_json(response).await;
    assert_eq!(plan["rom_id"], seeded.rom_id);
    assert_eq!(plan["preferred_file_id"], seeded.file_id);
    assert_eq!(plan["files"].as_array().unwrap().len(), 1);
    assert_eq!(plan["files"][0]["file_name"], "sonic.bin");
    assert!(plan["dependencies"].as_array().unwrap().is_empty());
    assert!(
        plan["files"][0].get("relative_path").is_none(),
        "download plans must not leak managed relative paths"
    );
    assert!(
        plan["files"][0].get("root_path").is_none(),
        "download plans must not leak library roots"
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/roms/{}/download-plan", seeded.rom_id))
                .header(header::AUTHORIZATION, format!("Bearer {raw_read_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(empty_request(
            &format!(
                "/api/roms/{}/content/Sonic%20Download.bin?file_ids={}",
                seeded.rom_id, seeded.file_id
            ),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH], "13");
    assert_eq!(
        response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"Sonic Download.bin\""
    );
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], b"hello genesis");
}
