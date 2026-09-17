#[tokio::test]
async fn romm_file_order_prefers_launchable_manifests_over_dependencies() {
    let test_app = support::TestApp::new().await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    seed_user(&state, "readonly", "readonly-password", UserRole::ReadOnly).await;

    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'ps2'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();

    let rom_id = sqlx::query(
        r#"
        INSERT INTO roms (platform_id, name, slug, regions_json)
        VALUES (?, 'Multi Disc Game', 'multi-disc-game', '[]')
        "#,
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    let group_id = sqlx::query(
        r#"
        INSERT INTO rom_file_groups (rom_id, kind, display_name, group_key, launchable)
        VALUES (?, 'playlist', 'Multi Disc Game', 'multi-disc-game', 1)
        "#,
    )
    .bind(rom_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    let grouped_rom_dir = temp_dir.path().join("roms/ps2");
    fs::create_dir_all(&grouped_rom_dir).unwrap();
    for (file_name, role, launchable, sort_index) in [
        ("Track 01.bin", "track", 0_i64, 30_i64),
        ("Disc 1.chd", "disc_image", 1, 20),
        ("Disc 1.cue", "descriptor", 1, 15),
        ("Multi Disc Game.m3u", "launch_manifest", 1, 10),
    ] {
        fs::write(grouped_rom_dir.join(file_name), [sort_index as u8]).unwrap();
        sqlx::query(
            r#"
            INSERT INTO rom_files (
                rom_id,
                root_id,
                relative_path,
                file_name,
                file_size_bytes,
                is_primary,
                group_id,
                original_file_name,
                role,
                sort_index,
                launchable
            )
            VALUES (?, ?, ?, ?, 1, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(rom_id)
        .bind(root_id)
        .bind(format!("ps2/{file_name}"))
        .bind(file_name)
        .bind(if role == "launch_manifest" {
            1_i64
        } else {
            0_i64
        })
        .bind(group_id)
        .bind(file_name)
        .bind(role)
        .bind(sort_index)
        .bind(launchable)
        .execute(state.db())
        .await
        .unwrap();
    }
    let manifest_id: i64 = sqlx::query_scalar(
        "SELECT id FROM rom_files WHERE rom_id = ? AND role = 'launch_manifest'",
    )
    .bind(rom_id)
    .fetch_one(state.db())
    .await
    .unwrap();
    let track_id: i64 =
        sqlx::query_scalar("SELECT id FROM rom_files WHERE rom_id = ? AND role = 'track'")
            .bind(rom_id)
            .fetch_one(state.db())
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO rom_file_dependencies (parent_file_id, child_file_id, dependency_kind) VALUES (?, ?, 'playlist_entry')",
    )
    .bind(manifest_id)
    .bind(track_id)
    .execute(state.db())
    .await
    .unwrap();

    let app = test_app.router.clone();
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
    let files = detail["files"].as_array().unwrap();
    assert_eq!(detail["fs_size_bytes"], 4);
    assert_eq!(files[0]["file_name"], "Multi Disc Game.m3u");
    assert_eq!(files[0]["role"], "launch_manifest");
    assert_eq!(files[0]["launchable"], true);
    assert_eq!(files[1]["file_name"], "Disc 1.cue");
    assert_eq!(files[1]["role"], "descriptor");
    assert_eq!(files[2]["file_name"], "Disc 1.chd");
    assert_eq!(files[2]["role"], "disc_image");
    assert_eq!(files[3]["file_name"], "Track 01.bin");
    assert_eq!(files[3]["role"], "track");
    assert_eq!(files[3]["launchable"], false);

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}/download-plan"),
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let plan = response_json(response).await;
    assert_eq!(plan["preferred_file_id"], manifest_id);
    assert_eq!(plan["files"][0]["file_name"], "Multi Disc Game.m3u");
    assert_eq!(plan["files"][3]["file_name"], "Track 01.bin");
    assert_eq!(plan["dependencies"][0]["parent_file_id"], manifest_id);
    assert_eq!(plan["dependencies"][0]["child_file_id"], track_id);
    assert_eq!(plan["dependencies"][0]["dependency_kind"], "playlist_entry");
    assert_eq!(plan["dependencies"][0]["sort_index"], 0);

    // This mirrors the fields decoded by current Sipario. Additive role/group
    // fields must remain safely ignorable, and its first-file choice must never
    // resolve to a dependency-only track.
    let decoded: SiparioRomDetailContract = serde_json::from_value(detail).unwrap();
    assert_eq!(decoded.files[0].file_name, "Multi Disc Game.m3u");
    assert!(decoded.files[0].id > 0);
    assert_eq!(decoded.files[0].file_size_bytes, 1);

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}/content/Multi%20Disc%20Game.m3u?file_ids={manifest_id}"),
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap()[..],
        &[10]
    );

    let response = app
        .clone()
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}/content/Track%2001.bin?file_ids={track_id}"),
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), usize::MAX).await.unwrap()[..],
        &[30]
    );

    let ticket_path = format!("/api/roms/{rom_id}/archive-ticket");
    let unauthenticated_ticket = app
        .clone()
        .oneshot(support::empty_request("POST", &ticket_path, None))
        .await
        .unwrap();
    assert_eq!(unauthenticated_ticket.status(), StatusCode::UNAUTHORIZED);

    let ticket_response = app
        .clone()
        .oneshot(support::empty_request(
            "POST",
            &ticket_path,
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(ticket_response.status(), StatusCode::OK);
    assert_eq!(
        ticket_response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let ticket_body = response_json(ticket_response).await;
    let ticket = ticket_body["ticket"].as_str().unwrap();
    assert!(ticket.starts_with("teatro_dl_"));
    assert_eq!(ticket_body["expires_in_seconds"], 60);

    let archive_response = app
        .clone()
        .oneshot(archive_ticket_request(ticket))
        .await
        .unwrap();
    assert_eq!(archive_response.status(), StatusCode::OK);
    assert_eq!(archive_response.headers()[header::CONTENT_TYPE], "application/zip");
    assert_eq!(
        archive_response.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"Multi Disc Game.zip\""
    );
    assert_eq!(
        archive_response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(!archive_response
        .headers()
        .contains_key(header::WWW_AUTHENTICATE));
    let archive_content_length = archive_response.headers()[header::CONTENT_LENGTH]
        .to_str()
        .unwrap()
        .parse::<usize>()
        .unwrap();

    // The archive permit remains attached to the consumed ticket response body,
    // bounding temporary archive retention until the client finishes.
    let busy = app
        .clone()
        .oneshot(support::empty_request(
            "POST",
            &ticket_path,
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(busy.headers()[header::RETRY_AFTER], "1");

    let archive_bytes = to_bytes(archive_response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(archive_bytes.len(), archive_content_length);
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).unwrap();
    assert_eq!(archive.len(), 4);
    for (index, (name, contents)) in [
        ("Multi Disc Game.m3u", vec![10]),
        ("Disc 1.cue", vec![15]),
        ("Disc 1.chd", vec![20]),
        ("Track 01.bin", vec![30]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = archive.by_index(index).unwrap();
        assert_eq!(entry.name(), name);
        let mut actual = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut actual).unwrap();
        assert_eq!(actual, contents);
    }

    let replay = app
        .clone()
        .oneshot(archive_ticket_request(ticket))
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::NOT_FOUND);
    assert!(!replay.headers().contains_key(header::WWW_AUTHENTICATE));
    assert_eq!(
        response_json(replay).await["error"]["message"],
        "download ticket is invalid or expired"
    );

    let retry_ticket_response = app
        .clone()
        .oneshot(support::empty_request(
            "POST",
            &ticket_path,
            Some(("readonly", "readonly-password")),
        ))
        .await
        .unwrap();
    assert_eq!(retry_ticket_response.status(), StatusCode::OK);
    let retry_ticket_body = response_json(retry_ticket_response).await;
    let retry_ticket = retry_ticket_body["ticket"].as_str().unwrap();
    let retry_response = app
        .clone()
        .oneshot(archive_ticket_request(retry_ticket))
        .await
        .unwrap();
    assert_eq!(retry_response.status(), StatusCode::OK);
    drop(retry_response);
    let remaining_data_files: Vec<_> = fs::read_dir(temp_dir.path().join("data"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        remaining_data_files,
        [".teatro.instance.lock"],
        "temporary archive files must be removed when response bodies are dropped"
    );

    let response = app
        .oneshot(empty_request(
            &format!("/api/roms/{rom_id}/content/Multi%20Disc%20Game.m3u"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let error = response_json(response).await;
    assert_eq!(
        error["error"]["message"],
        "file_ids is required when a ROM has multiple files"
    );
}
