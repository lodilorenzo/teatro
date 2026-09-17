#[tokio::test]
async fn upload_uses_parsed_filename_metadata_when_title_is_omitted() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "Zoop (U) [!].gen",
            b"rom",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let upload = response_json(response).await;
    assert_eq!(upload["rom"]["name"], "Zoop");
    assert_eq!(upload["rom"]["slug"], "zoop");
    assert_eq!(upload["rom"]["regions"], json!(["USA"]));
    assert_eq!(upload["relative_path"], "genesis/Zoop (U) [!].gen");
    assert_eq!(upload["rom"]["metadatum"]["source"], "filename");
    assert_eq!(
        upload["rom"]["metadatum"]["filename"]["clean_title"],
        "Zoop"
    );
    assert_eq!(
        upload["rom"]["metadatum"]["filename"]["dump_flags"]["verified_good_dump"],
        true
    );

    let response = app
        .clone()
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "Alien 3 (UE) (REV03) [h1C][o1].gen",
            b"rom",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let upload = response_json(response).await;
    assert_eq!(upload["rom"]["name"], "Alien 3");
    assert_eq!(upload["rom"]["regions"], json!(["USA", "Europe"]));
    assert_eq!(upload["rom"]["metadatum"]["revision"], 3);
    assert_eq!(
        upload["rom"]["metadatum"]["filename"]["dump_flags"]["hack"],
        "h1C"
    );
    assert_eq!(
        upload["rom"]["metadatum"]["filename"]["dump_flags"]["overdump"],
        "o1"
    );

    let response = app
        .oneshot(upload_request(
            "/api/admin/uploads",
            Some(("admin", "admin-password")),
            &[("platform_slug", "genesis")],
            "Large Game.7z.001",
            b"rom",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn batch_upload_imports_cue_with_tracks_as_one_grouped_rom() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let cue = br#"FILE "Ridge Racer (USA) (Track 01).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "Ridge Racer (USA) (Track 02).bin" BINARY
  TRACK 02 AUDIO
    INDEX 01 00:00:00
"#;

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/uploads/preview",
            Some(("admin", "admin-password")),
            json!({
                "platform_slug": "ps2",
                "files": [
                    {"file_name": "Ridge Racer (USA).cue", "file_size_bytes": cue.len(), "manifest_contents": String::from_utf8_lossy(cue)},
                    {"file_name": "Ridge Racer (USA) (Track 01).bin", "file_size_bytes": 9},
                    {"file_name": "Ridge Racer (USA) (Track 02).bin", "file_size_bytes": 9}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = response_json(response).await;
    assert_eq!(preview["errors"].as_array().unwrap().len(), 0);
    assert_eq!(preview["roms"].as_array().unwrap().len(), 1);
    assert_eq!(preview["roms"][0]["groups"][0]["kind"], "track_set");

    let response = app
        .clone()
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[("platform_slug", "ps2")],
            &[
                ("Ridge Racer (USA).cue", &cue[..]),
                ("Ridge Racer (USA) (Track 01).bin", b"track-one"),
                ("Ridge Racer (USA) (Track 02).bin", b"track-two"),
            ],
        ))
        .await
        .unwrap();

    let status = response.status();
    let upload = response_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{upload}");
    assert_eq!(upload["roms"].as_array().unwrap().len(), 1);
    let rom_id = upload["roms"][0]["id"].as_i64().unwrap();
    assert_eq!(upload["roms"][0]["name"], "Ridge Racer");
    assert_eq!(upload["roms"][0]["regions"], json!(["USA"]));
    assert_eq!(upload["roms"][0]["files"].as_array().unwrap().len(), 3);
    let cue_file_id = upload["roms"][0]["files"][0]["id"].as_i64().unwrap();

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
    let files = response_json(response).await;
    assert_eq!(files["groups"].as_array().unwrap().len(), 1);
    assert_eq!(files["groups"][0]["kind"], "track_set");
    assert_eq!(files["groups"][0]["files"].as_array().unwrap().len(), 3);
    assert_eq!(files["dependencies"].as_array().unwrap().len(), 2);
    assert_eq!(files["groups"][0]["files"][0]["role"], "descriptor");
    assert_eq!(files["groups"][0]["files"][0]["launchable"], true);
    assert!(
        files["groups"][0]["files"].as_array().unwrap()[1..]
            .iter()
            .all(|file| file["role"] == "track" && file["launchable"] == false)
    );

    assert_eq!(
        fs::read(
            temp_dir
                .path()
                .join("roms/ps2/ridge-racer/Ridge Racer (USA).cue")
        )
        .unwrap(),
        cue
    );
    assert_eq!(
        fs::read(
            temp_dir
                .path()
                .join("roms/ps2/ridge-racer/Ridge Racer (USA) (Track 02).bin")
        )
        .unwrap(),
        b"track-two"
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
    let detail = response_json(response).await;
    assert_eq!(detail["files"][0]["file_name"], "Ridge Racer (USA).cue");
    assert_eq!(detail["files"][0]["role"], "descriptor");
    assert_eq!(detail["files"][0]["launchable"], true);
    assert!(
        detail["files"].as_array().unwrap()[1..]
            .iter()
            .all(|file| file["role"] == "track" && file["launchable"] == false)
    );

    let response = app
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{rom_id}/content/Ridge%20Racer.cue?file_ids={cue_file_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], cue);
}
