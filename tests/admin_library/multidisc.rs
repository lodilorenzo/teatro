#[tokio::test]
async fn batch_upload_generates_m3u_for_multidisc_chd_set() {
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
        .oneshot(json_request(
            "POST",
            "/api/admin/uploads/preview",
            Some(("admin", "admin-password")),
            json!({
                "platform_slug": "ps2",
                "files": [
                    {"file_name": "Final Fantasy VII (USA) (Disc 1).chd", "file_size_bytes": 8},
                    {"file_name": "Final Fantasy VII (USA) (Disc 2).chd", "file_size_bytes": 8},
                    {"file_name": "Final Fantasy VII (USA) (Disc 3).chd", "file_size_bytes": 10}
                ]
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = response_json(response).await;
    assert_eq!(preview["errors"].as_array().unwrap().len(), 0);
    assert_eq!(preview["roms"].as_array().unwrap().len(), 1);
    assert_eq!(preview["roms"][0]["title"], "Final Fantasy VII");
    assert_eq!(preview["roms"][0]["groups"].as_array().unwrap().len(), 4);
    assert_eq!(preview["roms"][0]["groups"][0]["kind"], "playlist");
    assert_eq!(preview["roms"][0]["groups"][0]["disc_count"], 3);
    assert!(
        preview["roms"][0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|file| file["role"] == "launch_manifest"
                && file["original_file_name"] == "Final Fantasy VII (USA).m3u"
                && file["metadata"]["source"] == "generated")
    );

    let response = app
        .clone()
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[
                ("platform_slug", "ps2"),
                (
                    "planned_title",
                    r#"{"plan_id":"rom-1","title":"Final Fantasy VII Remastered"}"#,
                ),
            ],
            &[
                ("Final Fantasy VII (USA) (Disc 1).chd", b"disc-one"),
                ("Final Fantasy VII (USA) (Disc 2).chd", b"disc-two"),
                ("Final Fantasy VII (USA) (Disc 3).chd", b"disc-three"),
            ],
        ))
        .await
        .unwrap();

    let status = response.status();
    let upload = response_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{upload}");
    assert_eq!(upload["roms"].as_array().unwrap().len(), 1);
    let rom = &upload["roms"][0];
    let rom_id = rom["id"].as_i64().unwrap();
    assert_eq!(rom["name"], "Final Fantasy VII Remastered");
    assert_eq!(rom["slug"], "final-fantasy-vii-remastered");
    assert_eq!(rom["regions"], json!(["USA"]));
    let rom_files = rom["files"].as_array().unwrap();
    assert_eq!(rom_files.len(), 4);
    let grouped_size: i64 = rom_files
        .iter()
        .map(|file| file["file_size_bytes"].as_i64().unwrap())
        .sum();
    assert_eq!(rom["fs_size_bytes"], grouped_size);
    assert_eq!(rom_files[0]["file_name"], "Final Fantasy VII (USA).m3u");
    let m3u_file_id = rom_files[0]["id"].as_i64().unwrap();

    let m3u_contents = "Final Fantasy VII (USA) (Disc 1).chd\nFinal Fantasy VII (USA) (Disc 2).chd\nFinal Fantasy VII (USA) (Disc 3).chd\n";
    assert_eq!(
        fs::read_to_string(
            temp_dir
                .path()
                .join("roms/ps2/Final Fantasy VII (USA).m3u/Final Fantasy VII (USA).m3u")
        )
        .unwrap(),
        m3u_contents
    );
    assert_eq!(
        fs::read(
            temp_dir
                .path()
                .join("roms/ps2/Final Fantasy VII (USA).m3u/Final Fantasy VII (USA) (Disc 3).chd")
        )
        .unwrap(),
        b"disc-three"
    );

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
    assert_eq!(files["groups"].as_array().unwrap().len(), 4);
    assert_eq!(files["groups"][0]["kind"], "playlist");
    assert_eq!(
        files["groups"][0]["display_name"],
        "Final Fantasy VII Remastered"
    );
    assert_eq!(
        files["groups"][0]["group_key"],
        "final-fantasy-vii-remastered:playlist"
    );
    assert_eq!(files["groups"][0]["disc_count"], 3);
    assert_eq!(files["groups"][0]["files"][0]["role"], "launch_manifest");
    assert_eq!(files["groups"][1]["kind"], "disc");
    assert_eq!(files["groups"][1]["disc_index"], 1);
    assert_eq!(files["groups"][3]["disc_index"], 3);
    assert_eq!(files["dependencies"].as_array().unwrap().len(), 3);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!(
                "/api/roms/{rom_id}/content/Final%20Fantasy%20VII%20%28USA%29.m3u?file_ids={m3u_file_id}"
            ),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&bytes[..], m3u_contents.as_bytes());

    let response = app
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = response_json(response).await;
    assert_eq!(deleted["deleted_files"].as_array().unwrap().len(), 4);
    let rom_dir = temp_dir
        .path()
        .join("roms/ps2/Final Fantasy VII (USA).m3u");
    for file_name in [
        "Final Fantasy VII (USA).m3u",
        "Final Fantasy VII (USA) (Disc 1).chd",
        "Final Fantasy VII (USA) (Disc 2).chd",
        "Final Fantasy VII (USA) (Disc 3).chd",
    ] {
        assert!(!rom_dir.join(file_name).exists());
    }
}

#[tokio::test]
async fn batch_upload_preserves_user_supplied_m3u_img_order_and_roles() {
    let test_app = support::TestApp::with_config(|config| {
        config.max_upload_bytes = 1024 * 1024;
    })
    .await;
    let temp_dir = &test_app.temp_dir;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let m3u = b"Swap Game (USA) (Disc 2).img\nSwap Game (USA) (Disc 1).img\n";
    let response = app
        .clone()
        .oneshot(batch_upload_request(
            "/api/admin/upload-batches",
            Some(("admin", "admin-password")),
            &[("platform_slug", "ps2")],
            &[
                ("Swap Game (USA).m3u", &m3u[..]),
                ("Swap Game (USA) (Disc 1).img", b"disc-one"),
                ("Swap Game (USA) (Disc 2).img", b"disc-two"),
            ],
        ))
        .await
        .unwrap();

    let status = response.status();
    let upload = response_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{upload}");
    let files = upload["roms"][0]["files"].as_array().unwrap();
    assert_eq!(files.len(), 3);
    assert_eq!(files[0]["file_name"], "Swap Game (USA).m3u");
    assert_eq!(files[0]["role"], "launch_manifest");
    assert_eq!(files[1]["file_name"], "Swap Game (USA) (Disc 2).img");
    assert_eq!(files[1]["role"], "disc_image");
    assert_eq!(files[2]["file_name"], "Swap Game (USA) (Disc 1).img");
    assert_eq!(files[2]["role"], "disc_image");
    assert_eq!(
        fs::read(
            temp_dir
                .path()
                .join("roms/ps2/Swap Game (USA).m3u/Swap Game (USA).m3u")
        )
        .unwrap(),
        m3u
    );
}
