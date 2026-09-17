#[tokio::test]
async fn admin_can_upload_browse_download_get_stats_and_delete_roms() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis"), ("title", "Teatro Upload")],
            "Teatro Upload.bin",
            b"uploaded rom bytes",
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let upload = response_json(response).await;
    let rom_id = upload["rom"]["id"].as_i64().unwrap();
    let file_id = upload["file_id"].as_i64().unwrap();
    assert_eq!(upload["rom"]["name"], "Teatro Upload");
    assert_eq!(upload["rom"]["platform_slug"], "genesis");
    assert_eq!(upload["relative_path"], "genesis/Teatro Upload.bin");

    let first_file = temp_dir.path().join("roms/genesis/Teatro Upload.bin");
    assert_eq!(fs::read(&first_file).unwrap(), b"uploaded rom bytes");

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/admin/roms/{rom_id}/files"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let file_listing = response_json(response).await;
    assert_eq!(file_listing["rom_id"], rom_id);
    assert_eq!(file_listing["groups"].as_array().unwrap().len(), 1);
    assert_eq!(file_listing["groups"][0]["kind"], "single");
    assert_eq!(file_listing["groups"][0]["launchable"], true);
    assert_eq!(
        file_listing["groups"][0]["files"].as_array().unwrap().len(),
        1
    );
    assert_eq!(file_listing["groups"][0]["files"][0]["id"], file_id);
    assert_eq!(file_listing["groups"][0]["files"][0]["role"], "content");
    assert_eq!(file_listing["groups"][0]["files"][0]["launchable"], true);
    assert_eq!(
        file_listing["groups"][0]["files"][0]["original_file_name"],
        "Teatro Upload.bin"
    );
    assert_eq!(file_listing["dependencies"].as_array().unwrap().len(), 0);
    assert_eq!(file_listing["warnings"].as_array().unwrap().len(), 0);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
            json!({
                "name": "Teatro Edited",
                "summary": "Updated from the admin UI flow.",
                "regions": ["World"],
                "genres": ["Action", "Action", "Arcade"],
                "developers": ["Teatro Lab"],
                "release_year": 1991
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let edited = response_json(response).await;
    assert_eq!(edited["name"], "Teatro Edited");
    assert_eq!(edited["slug"], "teatro-edited");
    assert_eq!(edited["summary"], "Updated from the admin UI flow.");
    assert_eq!(edited["regions"], json!(["World"]));
    assert_eq!(edited["metadatum"]["source"], "manual");
    assert_eq!(edited["metadatum"]["genres"], json!(["Action", "Arcade"]));
    assert_eq!(edited["metadatum"]["developers"], json!(["Teatro Lab"]));
    assert_eq!(edited["metadatum"]["release_year"], 1991);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
            json!({
                "summary": null,
                "release_year": null
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let edited = response_json(response).await;
    assert_eq!(edited["summary"], serde_json::Value::Null);
    assert_eq!(edited["metadatum"]["summary"], serde_json::Value::Null);
    assert_eq!(edited["metadatum"]["release_year"], serde_json::Value::Null);

    let response = app
        .clone()
        .oneshot(cover_request(
            &format!("/api/admin/roms/{rom_id}/cover"),
            ("admin", "admin-password"),
            "image/svg+xml",
            b"<svg onload=alert(1)>",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .clone()
        .oneshot(cover_request(
            &format!("/api/admin/roms/{rom_id}/cover"),
            ("admin", "admin-password"),
            "image/png",
            &vec![0; 10 * 1024 * 1024 + 1],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response_json(response).await["error"]["code"], "payload_too_large");

    let cover_bytes = b"\x89PNG\r\n\x1a\nteatro-cover";
    let response = app
        .clone()
        .oneshot(cover_request(
            &format!("/api/admin/roms/{rom_id}/cover"),
            ("admin", "admin-password"),
            "image/png",
            cover_bytes,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let covered = response_json(response).await;
    let cover_path = covered["path_cover_large"].as_str().unwrap();
    assert_eq!(covered["path_cover_small"], cover_path);
    assert_eq!(covered["url_cover"], serde_json::Value::Null);
    assert_eq!(covered["metadatum"]["cover_source"], "manual");
    assert_eq!(covered["metadatum"]["path_cover_large"], cover_path);
    assert!(cover_path.ends_with(".png"));
    let cover_file = temp_dir.path().join("assets").join(cover_path);
    assert_eq!(fs::read(&cover_file).unwrap(), cover_bytes);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/assets/romm/resources/{cover_path}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    assert_eq!(&to_bytes(response.into_body(), usize::MAX).await.unwrap()[..], cover_bytes);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/roms?limit=10&offset=0",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let page = response_json(response).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["items"][0]["files"][0]["id"], file_id);

    let download_response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{rom_id}/content/Teatro%20Upload.bin?file_ids={file_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(download_response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/stats",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let stats = response_json(response).await;
    assert_eq!(stats["total_roms"], 1);
    assert_eq!(stats["total_files"], 1);
    assert_eq!(stats["total_file_bytes"], 18);
    assert!(
        stats["platforms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|platform| {
                platform["slug"] == "genesis"
                    && platform["rom_count"] == 1
                    && platform["file_count"] == 1
            })
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}?delete_files=false"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        first_file.exists(),
        "rejected delete_files=false must leave the ROM file in place"
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["delete_files"], true);
    assert_eq!(deleted["deleted_files"][0], "genesis/Teatro Upload.bin");
    assert!(
        !first_file.exists(),
        "default delete must remove the managed ROM file"
    );
    assert!(!cover_file.exists(), "default delete must remove its cover");
    let bytes = to_bytes(download_response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(
        &bytes[..],
        b"uploaded rom bytes",
        "an already-open download must remain readable while deletion completes"
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis"), ("title", "Teatro Upload")],
            "Teatro Upload.bin",
            b"replacement bytes",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let upload = response_json(response).await;
    let second_rom_id = upload["rom"]["id"].as_i64().unwrap();
    assert_eq!(upload["relative_path"], "genesis/Teatro Upload.bin");
    let second_file = temp_dir.path().join("roms/genesis/Teatro Upload.bin");
    assert_eq!(fs::read(&second_file).unwrap(), b"replacement bytes");

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{second_rom_id}?delete_files=true"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["delete_files"], true);
    assert_eq!(deleted["deleted_files"][0], "genesis/Teatro Upload.bin");
    assert!(
        !second_file.exists(),
        "delete must remove the managed ROM file"
    );
}

#[tokio::test]
async fn deleting_a_rom_already_missing_from_disk_removes_its_library_record() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));

    let response = app
        .router
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            auth,
            &[("platform_slug", "genesis"), ("title", "Missing Game")],
            "Missing Game.bin",
            b"missing",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let uploaded = response_json(response).await;
    let rom_id = uploaded["rom"]["id"].as_i64().unwrap();
    let relative_path = uploaded["relative_path"].as_str().unwrap();
    fs::remove_file(app.state.config().default_library_root.join(relative_path)).unwrap();

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}"),
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["deleted_files"], json!([]));
    assert_eq!(deleted["missing_files"], json!([relative_path]));

    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{rom_id}"),
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_can_bulk_delete_platform_roms_and_clear_library_contents() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let genesis = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "Bulk Genesis.bin",
            b"genesis",
        ))
        .await
        .unwrap();
    assert_eq!(genesis.status(), StatusCode::CREATED);
    let genesis = response_json(genesis).await;
    let genesis_platform_id = genesis["rom"]["platform_id"].as_i64().unwrap();
    let genesis_file = temp_dir.path().join("roms/genesis/Bulk Genesis.bin");
    assert!(genesis_file.exists());

    let ps2 = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "ps2")],
            "Bulk PS2.iso",
            b"ps2",
        ))
        .await
        .unwrap();
    assert_eq!(ps2.status(), StatusCode::CREATED);
    let ps2_file = temp_dir.path().join("roms/ps2/Bulk PS2.iso");
    assert!(ps2_file.exists());

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/platforms/{genesis_platform_id}/roms?confirm=wrong"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(genesis_file.exists());

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/platforms/{genesis_platform_id}/roms?confirm=genesis"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["scope"]["kind"], "platform");
    assert_eq!(deleted["deleted_rom_count"], 1);
    assert_eq!(deleted["deleted_files"][0], "genesis/Bulk Genesis.bin");
    assert!(!genesis_file.exists());
    assert!(ps2_file.exists());

    let response = app
        .oneshot(empty_request(
            "DELETE",
            "/api/admin/roms?confirm=DELETE%20ALL",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["scope"]["kind"], "all");
    assert_eq!(deleted["deleted_rom_count"], 1);
    assert!(!ps2_file.exists());
}
