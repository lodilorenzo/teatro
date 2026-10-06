mod support;
use axum::http::StatusCode;
use serde_json::Value;
use sevenz_rust2::{ArchiveReader, Password};
use support::{TestApp, empty_request, multipart_request, response_json};
use tower::ServiceExt;

async fn wait(app: &TestApp, url: &str) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(empty_request("GET", url, Some(("admin", "pass"))))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let job = response_json(response).await;
            if matches!(
                job["state"].as_str(),
                Some("succeeded" | "failed" | "cancelled")
            ) {
                return job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn seven_zip_import_is_gated_verified_and_published_as_one_launchable_archive() {
    let disabled = TestApp::new().await;
    disabled.seed_admin("admin", "pass").await;
    let response = disabled
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Game"), ("platform_slug", "nes")],
            &[("files", "Game.nes", b"ROM")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/conversion/status",
            Some(("admin", "pass")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let status = response_json(response).await;
    let formats = status["formats"].as_array().unwrap();
    assert!(formats.iter().any(|format| {
        format["platform_slug"] == "nes"
            && format["output"] == "7z"
            && format["input_extensions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|ext| ext == "nes")
    }));
    for platform in [
        "psx", "ps2", "psp", "gc", "wii", "wiiu", "fds", "win", "neogeo",
    ] {
        assert!(
            !formats
                .iter()
                .any(|format| format["platform_slug"] == platform && format["output"] == "7z")
        );
    }
    let bytes = vec![42; 128 * 1024];
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Edited game title"), ("platform_slug", "nes")],
            &[("files", "Original.NES", &bytes)],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let created = response_json(response).await;
    let job = wait(&app, created["status_url"].as_str().unwrap()).await;
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["output_format"], "7z");
    assert_eq!(job["result"]["file_name"], "Edited game title.7z");
    let output = app
        .state
        .config()
        .default_library_root
        .join("nes/Edited game title.7z");
    let mut decoded = Vec::new();
    ArchiveReader::open(output, Password::empty())
        .unwrap()
        .for_each_entries(|entry, reader| {
            assert_eq!(entry.name(), "Original.NES");
            reader.read_to_end(&mut decoded)?;
            Ok(true)
        })
        .unwrap();
    assert_eq!(decoded, bytes);
    let id = job["result"]["rom_id"].as_i64().unwrap();
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{id}/download-plan"),
            Some(("admin", "pass")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let plan = response_json(response).await;
    assert_eq!(plan["files"].as_array().unwrap().len(), 1);
    assert_eq!(plan["files"][0]["launchable"], true);

    for (name, data) in [
        ("Game.zip", b"PK\x03\x04".as_slice()),
        ("Fake.nes", b"7z\xbc\xaf\x27\x1c"),
        ("Game.iso", b"not a disc"),
        ("Empty.nes", b""),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(multipart_request(
                "POST",
                "/api/admin/conversion-imports",
                Some(("admin", "pass")),
                &[("title", "Rejected"), ("platform_slug", "nes")],
                &[("files", name, data)],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let created = response_json(response).await;
        let job = wait(&app, created["status_url"].as_str().unwrap()).await;
        assert_eq!(job["state"], "failed", "{job}");
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count(),
        0
    );
}
