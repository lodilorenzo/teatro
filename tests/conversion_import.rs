mod support;
use axum::http::StatusCode;
use serde_json::{Value, json};
use support::{TestApp, empty_request, multipart_request, response_bytes, response_json};
use tower::ServiceExt;

fn fixture(id: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        (
            "Game/code/app.xml".into(),
            format!("<app><title_id>{id}</title_id><title_version>1</title_version></app>")
                .into_bytes(),
        ),
        ("Game/code/cos.xml".into(), b"<cos/>".to_vec()),
        (
            "Game/meta/meta.xml".into(),
            format!("<menu><title_id>{id}</title_id><title_version>1</title_version></menu>")
                .into_bytes(),
        ),
        ("Game/content/data.bin".into(), vec![42; 128 * 1024]),
    ]
}
async fn stage(app: &TestApp, files: &[(String, Vec<u8>)]) -> Value {
    let files: Vec<_> = files
        .iter()
        .map(|(name, bytes)| ("files", name.as_str(), bytes.as_slice()))
        .collect();
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Synthetic title")],
            &files,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    response_json(response).await
}
async fn poll(app: &TestApp, url: &str, phase: &str) -> Value {
    let mut last = Value::Null;
    // WBFS verification streams a canonical 4.7 GiB logical disc even for tiny fixtures.
    tokio::time::timeout(std::time::Duration::from_secs(180), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(empty_request("GET", url, Some(("admin", "pass"))))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let job = response_json(response).await;
            if job["phase"] == phase || job["state"] == phase {
                return job;
            }
            assert_ne!(job["state"], "failed", "{job}");
            assert_ne!(
                job["phase"], "awaiting_review",
                "Jobs must finish without approval: {job}"
            );
            last = job;
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Timed out waiting for {phase}: {last}"))
}
async fn enabled() -> TestApp {
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    app
}
#[tokio::test]
async fn wua_with_update_and_dlc_is_automatically_verified_published_and_downloadable() {
    let app = enabled().await;
    let mut files = fixture("0005000010102000");
    for (folder, id) in [("Update", "0005000e10102000"), ("DLC", "0005000c10102000")] {
        files.extend(
            fixture(id)
                .into_iter()
                .map(|(name, mut bytes)| {
                    if folder == "Update" {
                        bytes = match name.as_str() {
                            "Game/code/app.xml" => b"\xef\xbb\xbf<app><title_id type=\"hexBinary\">0005000e10102000</title_id><title_version type=\"hexBinary\">0041</title_version></app>".to_vec(),
                            "Game/meta/meta.xml" => b"\xef\xbb\xbf<menu><title_id type=\"hexBinary\">0005000010102000</title_id><title_version type=\"unsignedInt\">65</title_version></menu>".to_vec(),
                            _ => bytes,
                        };
                    }
                    (name.replacen("Game", folder, 1), bytes)
                }),
        );
    }
    let created = stage(&app, &files).await;
    let url = created["status_url"].as_str().unwrap();
    let complete = poll(&app, url, "succeeded").await;
    assert!(complete["proposal"].is_null());
    app.seed_admin("other", "pass").await;
    for method in ["GET", "DELETE"] {
        assert_eq!(
            app.router
                .clone()
                .oneshot(empty_request(method, url, Some(("other", "pass"))))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(complete["result"]["title"], "Synthetic title");
    assert_eq!(complete["result"]["file_name"], "Synthetic title.wua");
    assert_eq!(complete["result"]["metadata_applied"], false);
    assert!(
        complete["result"]["metadata_warning"]
            .as_str()
            .unwrap()
            .contains("not configured")
    );
    assert!(
        complete["result"]["output_bytes"].as_u64().unwrap()
            < complete["result"]["input_bytes"].as_u64().unwrap()
    );
    let id = complete["result"]["rom_id"].as_i64().unwrap();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT file_name,relative_path FROM rom_files WHERE rom_id = ?")
            .bind(id)
            .fetch_all(app.state.db())
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "Synthetic title.wua");
    assert_eq!(rows[0].1, "wiiu/Synthetic title.wua");
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
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms/{id}/content/{}", rows[0].0.replace(' ', "%20")),
            Some(("admin", "pass")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let output = response_bytes(response).await;
    assert_eq!(
        output.len() as u64,
        complete["result"]["output_bytes"].as_u64().unwrap()
    );
    assert_eq!(
        std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count(),
        0
    );
}
#[tokio::test]
async fn wua_import_applies_igdb_metadata_and_covers_without_a_browser() {
    use axum::{
        Json, Router,
        http::header,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};
    use teatro::config::IgdbConfig;

    for mode in ["match", "no_match", "upstream_failure"] {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let captured = requests.clone();
        let mock = Router::new()
            .route("/token", post(|| async { Json(json!({"access_token":"test-token", "expires_in":3600})) }))
            .route("/v4/games", post(move |body: String| {
                let captured = captured.clone();
                async move {
                    captured.lock().unwrap().push(body);
                    if mode == "upstream_failure" { return StatusCode::BAD_GATEWAY.into_response(); }
                    if mode == "no_match" { return Json(json!([])).into_response(); }
                    Json(json!([{
                        "id":123, "name":"Canonical base title", "summary":"Game summary.", "storyline":"Game story.",
                        "first_release_date":1384992000, "rating":85.0, "aggregated_rating":86.0, "total_rating":85.5,
                        "genres":[{"name":"Platform"}], "platforms":[{"id":41,"name":"Wii U"}],
                        "involved_companies":[{"developer":true,"publisher":true,"company":{"name":"Nintendo"}}],
                        "cover":{"image_id":"co123"}
                    }])).into_response()
                }
            }))
            .route("/images/t_cover_big/co123.jpg", get(|| async { ([(header::CONTENT_TYPE,"image/jpeg")], b"large-cover".to_vec()) }))
            .route("/images/t_cover_small/co123.jpg", get(|| async { ([(header::CONTENT_TYPE,"image/jpeg")], b"small-cover".to_vec()) }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, mock).await.unwrap();
        });
        let app = TestApp::with_config(|config| {
            config.conversion_enabled = true;
            config.uploads.free_space_margin_bytes = 0;
            config.igdb = IgdbConfig {
                client_id: Some("test-client".into()),
                client_secret: Some("test-secret".into()),
                token_url: format!("{base}/token"),
                api_url: format!("{base}/v4"),
                image_base_url: format!("{base}/images"),
            };
        })
        .await;
        app.seed_admin("admin", "pass").await;
        let created = stage(&app, &fixture("0005000010102000")).await;
        let url = created["status_url"].as_str().unwrap();
        let complete = poll(&app, url, "succeeded").await;
        let id = complete["result"]["rom_id"].as_i64().unwrap();
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/roms/{id}"),
                Some(("admin", "pass")),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let rom = response_json(response).await;
        assert_eq!(complete["result"]["file_name"], "Synthetic title.wua");
        assert_eq!(complete["result"]["metadata_applied"], mode == "match");
        let queries = requests.lock().unwrap();
        assert!(queries[0].contains("Synthetic title"));
        assert!(queries[0].contains("where platforms = (41)"));
        if mode == "match" {
            assert_eq!(queries.len(), 1);
            assert_eq!(complete["result"]["metadata_warning"], Value::Null);
            assert_eq!(complete["result"]["title"], "Canonical base title");
            assert_eq!(rom["name"], "Canonical base title");
            assert_eq!(rom["summary"], "Game summary.");
            let metadata = &rom["metadatum"];
            assert_eq!(metadata["source"], "igdb");
            assert_eq!(metadata["igdb_id"], 123);
            assert_eq!(metadata["storyline"], "Game story.");
            assert_eq!(metadata["release_year"], 2013);
            assert_eq!(metadata["rating"], 85.0);
            assert_eq!(metadata["aggregated_rating"], 86.0);
            assert_eq!(metadata["total_rating"], 85.5);
            assert_eq!(metadata["genres"], json!(["Platform"]));
            assert_eq!(metadata["developers"], json!(["Nintendo"]));
            assert_eq!(metadata["publishers"], json!(["Nintendo"]));
            for (field, bytes) in [
                ("path_cover_large", b"large-cover".as_slice()),
                ("path_cover_small", b"small-cover".as_slice()),
            ] {
                let path = rom[field].as_str().unwrap();
                assert_eq!(
                    std::fs::read(app.state.config().asset_root.join(path)).unwrap(),
                    bytes
                );
            }
        } else {
            assert!(
                complete["result"]["metadata_warning"]
                    .as_str()
                    .unwrap()
                    .contains(if mode == "no_match" {
                        "No IGDB game"
                    } else {
                        "failed"
                    })
            );
            assert_eq!(rom["name"], "Synthetic title");
            assert!(
                app.state
                    .config()
                    .default_library_root
                    .join("wiiu/Synthetic title.wua")
                    .is_file()
            );
            if mode == "no_match" {
                assert_eq!(queries.len(), 2);
                assert!(!queries[1].contains("where platforms"));
            }
        }
        server.abort();
    }
}

#[tokio::test]
async fn invalid_paths_and_byte_limits_leave_no_private_workspace() {
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.max_upload_bytes = 64;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    for files in [
        vec![("../code/app.xml", &b"x"[..])],
        vec![("Game/code/a", &b"x"[..]), ("game/code/b", &b"x"[..])],
        vec![("Game/code/a", &b"x"[..]), ("Game/code/a", &b"x"[..])],
        vec![("Game/content/a", &[0_u8; 65][..])],
    ] {
        let files: Vec<_> = files
            .iter()
            .map(|(path, data)| ("files", *path, *data))
            .collect();
        let response = app
            .router
            .clone()
            .oneshot(multipart_request(
                "POST",
                "/api/admin/conversion-imports",
                Some(("admin", "pass")),
                &[("title", "Game")],
                &files,
            ))
            .await
            .unwrap();
        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::PAYLOAD_TOO_LARGE].contains(&response.status())
        );
        assert_eq!(
            std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
                .unwrap()
                .count(),
            0
        );
    }
}
#[tokio::test]
async fn cancellation_discards_staged_inputs_without_publishing() {
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.max_upload_bytes = 16 * 1024 * 1024;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    let mut files = fixture("0005000010102000");
    let mut seed = 1_u32;
    files.last_mut().unwrap().1 = (0..8 * 1024 * 1024)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as u8
        })
        .collect();
    let created = stage(&app, &files).await;
    let url = created["status_url"].as_str().unwrap();
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request("DELETE", url, Some(("admin", "pass"))))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    poll(&app, url, "cancelled").await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count()
            != 0
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM roms")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!(count, 0);
}
#[tokio::test]
async fn larger_verified_output_is_published_without_another_confirmation() {
    let app = enabled().await;
    let mut files = fixture("0005000010102000");
    files.last_mut().unwrap().1 = vec![42; 64];
    let created = stage(&app, &files).await;
    let complete = poll(&app, created["status_url"].as_str().unwrap(), "succeeded").await;
    assert!(
        complete["result"]["output_bytes"].as_u64().unwrap()
            > complete["result"]["input_bytes"].as_u64().unwrap()
    );
    assert!(complete["proposal"].is_null());
}

fn gamecube_fixture() -> Vec<u8> {
    let mut disc = vec![42; 512 * 1024];
    disc[..0x440].fill(0);
    disc[..6].copy_from_slice(b"GTEST1");
    disc[0x1c..0x20].copy_from_slice(&0xc2339f3du32.to_be_bytes());
    disc[0x424..0x428].copy_from_slice(&0x2500u32.to_be_bytes());
    disc[0x428..0x42c].copy_from_slice(&0x100u32.to_be_bytes());
    disc
}
async fn stage_gamecube(app: &TestApp, name: &str, disc: &[u8]) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[
                ("title", name.rsplit_once('.').unwrap().1),
                ("platform_slug", "gc"),
            ],
            &[("files", name, disc)],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    response_json(response).await
}

#[tokio::test]
async fn gamecube_iso_and_gcm_automatically_publish_byte_identical_rvz() {
    use rom_converto_lib::nintendo::disc::rvz::decompress::RvzDiscReader;
    use std::io::Read;
    let app = enabled().await;
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
    let status = response_json(response).await;
    assert!(
        status["formats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["platform_slug"] == "gc" && f["output"] == "rvz" && f["qualified"] == false)
    );
    let disc = gamecube_fixture();
    for extension in ["ISO", "gcm"] {
        let created = stage_gamecube(&app, &format!("Disc.{extension}"), &disc).await;
        let url = created["status_url"].as_str().unwrap();
        let title = extension;
        let complete = poll(&app, url, "succeeded").await;
        assert_eq!(complete["result"]["file_name"], format!("{title}.rvz"));
        let path = app
            .state
            .config()
            .default_library_root
            .join(format!("gc/{title}.rvz"));
        let mut decoded = Vec::new();
        RvzDiscReader::open(&path)
            .unwrap()
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, disc);
        let id = complete["result"]["rom_id"].as_i64().unwrap();
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
        let plan = response_json(response).await;
        assert_eq!(plan["files"].as_array().unwrap().len(), 1);
        assert_eq!(plan["files"][0]["launchable"], true);
    }
    assert_eq!(
        std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn gamecube_rejects_renamed_wii_legacy_truncated_and_bad_geometry_images() {
    let app = enabled().await;
    let original = gamecube_fixture();
    let mut wii = original.clone();
    wii[0x1c..0x20].fill(0);
    wii[0x18..0x1c].copy_from_slice(&0x5d1c9ea3u32.to_be_bytes());
    let mut legacy = original.clone();
    legacy[0x200..0x204].copy_from_slice(b"NKIT");
    let mut geometry = original.clone();
    geometry[0x424..0x428].copy_from_slice(&u32::MAX.to_be_bytes());
    for (name, bytes) in [
        ("Wii.iso", wii.as_slice()),
        ("Legacy.iso", legacy.as_slice()),
        ("Small.iso", &original[..100]),
        ("Bad.gcm", geometry.as_slice()),
        ("Disc.rvz", original.as_slice()),
        ("Disc.nkit.iso", original.as_slice()),
    ] {
        let created = stage_gamecube(&app, name, bytes).await;
        let failed = poll(&app, created["status_url"].as_str().unwrap(), "failed").await;
        assert!(failed["proposal"].is_null());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM roms")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!(count, 0);
}

fn wii_fixture() -> Vec<u8> {
    use rom_converto_lib::nintendo::rvl::{
        constants::{WII_SECTOR_PAYLOAD_SIZE, WII_SECTOR_SIZE, WII_TICKET_SIZE},
        encrypt_sector, encrypt_title_key,
        partition::recompute_hash_regions,
    };
    let mut disc = vec![0; 0x270000];
    disc[..6].copy_from_slice(b"RTEST1");
    disc[0x18..0x1c].copy_from_slice(&0x5d1c9ea3u32.to_be_bytes());
    disc[0x40000..0x40004].copy_from_slice(&1u32.to_be_bytes());
    disc[0x40004..0x40008].copy_from_slice(&(0x40100u32 / 4).to_be_bytes());
    disc[0x40100..0x40104].copy_from_slice(&(0x50000u32 / 4).to_be_bytes());
    let key = [0xa5; 16];
    let mut ticket = [0; WII_TICKET_SIZE];
    ticket[0x1dc..0x1e4].copy_from_slice(&0x0001000012345678u64.to_be_bytes());
    let encrypted = encrypt_title_key(&ticket, &key).unwrap();
    ticket[0x1bf..0x1cf].copy_from_slice(&encrypted);
    disc[0x50000..0x50000 + ticket.len()].copy_from_slice(&ticket);
    disc[0x502b8..0x502bc].copy_from_slice(&(0x20000u32 / 4).to_be_bytes());
    disc[0x502bc..0x502c0].copy_from_slice(&(0x200000u32 / 4).to_be_bytes());
    let payloads: Vec<[u8; WII_SECTOR_PAYLOAD_SIZE]> = (0..64)
        .map(|i| [i as u8; WII_SECTOR_PAYLOAD_SIZE])
        .collect();
    let hashes = recompute_hash_regions(&payloads);
    for i in 0..64 {
        let mut sector = [0; WII_SECTOR_SIZE];
        sector[..0x400].copy_from_slice(&hashes[i]);
        sector[0x400..].copy_from_slice(&payloads[i]);
        encrypt_sector(&mut sector, &key).unwrap();
        let start = 0x70000 + i * WII_SECTOR_SIZE;
        disc[start..start + WII_SECTOR_SIZE].copy_from_slice(&sector);
    }
    disc
}
fn wbfs_fixture(disc: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 2 * 1024 * 1024;
    let blocks = disc.len().div_ceil(BLOCK);
    let mut wbfs = vec![0; (blocks + 1) * BLOCK];
    wbfs[..4].copy_from_slice(b"WBFS");
    let sectors = (wbfs.len() / 512) as u32;
    wbfs[4..8].copy_from_slice(&sectors.to_be_bytes());
    wbfs[8] = 9;
    wbfs[9] = 21;
    wbfs[12] = 1;
    wbfs[512..768].copy_from_slice(&disc[..0x100]);
    for i in 0..blocks {
        wbfs[768 + i * 2..770 + i * 2].copy_from_slice(&((i + 1) as u16).to_be_bytes());
    }
    wbfs[BLOCK..BLOCK + disc.len()].copy_from_slice(disc);
    wbfs
}
async fn stage_wii(app: &TestApp, name: &str, disc: &[u8]) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[
                ("title", name.rsplit_once('.').unwrap().1),
                ("platform_slug", "wii"),
            ],
            &[("files", name, disc)],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    response_json(response).await
}
#[tokio::test]
async fn wii_iso_and_wbfs_preserve_encrypted_partition_bytes_and_publish_to_wii() {
    use rom_converto_lib::{
        nintendo::disc::{input::open_disc_input, rvz::decompress_disc},
        util::{CancelToken, NoProgress},
    };
    use std::io::Read;
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.max_upload_bytes = 8 * 1024 * 1024;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    let status = response_json(
        app.router
            .clone()
            .oneshot(empty_request(
                "GET",
                "/api/admin/conversion/status",
                Some(("admin", "pass")),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        status["formats"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["platform_slug"] == "wii"
                && f["output"] == "rvz"
                && f["qualified"] == false)
    );
    let iso = wii_fixture();
    let wbfs = wbfs_fixture(&iso);
    for (extension, bytes) in [("ISO", &iso), ("WBFS", &wbfs)] {
        let created = stage_wii(&app, &format!("Disc.{extension}"), bytes).await;
        let url = created["status_url"].as_str().unwrap();
        let title = extension;
        let output = app
            .state
            .config()
            .default_library_root
            .join(format!("wii/{title}.rvz"));
        let complete = poll(&app, url, "succeeded").await;
        let source = app.temp_dir.path().join(format!("source.{extension}"));
        std::fs::write(&source, bytes).unwrap();
        let mut original = open_disc_input(&source).unwrap();
        let decoded = app.temp_dir.path().join("decoded.iso");
        decompress_disc(&output, &decoded, &NoProgress, CancelToken::new())
            .await
            .unwrap();
        let mut restored = std::fs::File::open(&decoded).unwrap();
        assert_eq!(restored.metadata().unwrap().len(), original.logical_size());
        let mut left = vec![0; 128 * 1024];
        let mut right = left.clone();
        let mut remaining = original.logical_size();
        while remaining > 0 {
            let count = remaining.min(left.len() as u64) as usize;
            original.read_exact(&mut left[..count]).unwrap();
            restored.read_exact(&mut right[..count]).unwrap();
            assert_eq!(left[..count], right[..count]);
            remaining -= count as u64;
        }
        let id = complete["result"]["rom_id"].as_i64().unwrap();
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT relative_path FROM rom_files WHERE rom_id = ?")
                .bind(id)
                .fetch_all(app.state.db())
                .await
                .unwrap();
        assert_eq!(rows, [format!("wii/{title}.rvz")]);
        let plan = response_json(
            app.router
                .clone()
                .oneshot(empty_request(
                    "GET",
                    &format!("/api/roms/{id}/download-plan"),
                    Some(("admin", "pass")),
                ))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(plan["files"].as_array().unwrap().len(), 1);
        assert_eq!(plan["files"][0]["launchable"], true);
        drop(restored);
        std::fs::remove_file(decoded).unwrap();
    }
}
#[tokio::test]
async fn wii_rejects_wrong_platform_legacy_bad_partitions_and_unsafe_wbfs() {
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.max_upload_bytes = 8 * 1024 * 1024;
        config.uploads.free_space_margin_bytes = 0;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    let iso = wii_fixture();
    let wbfs = wbfs_fixture(&iso);
    let mut legacy = iso.clone();
    legacy[0x200..0x204].copy_from_slice(b"NKIT");
    let mut geometry = iso.clone();
    geometry[0x502bc..0x502c0].copy_from_slice(&u32::MAX.to_be_bytes());
    let mut table = iso.clone();
    table[0x40000..0x40004].copy_from_slice(&17u32.to_be_bytes());
    let mut shift = wbfs.clone();
    shift[9] = 255;
    let mut multi = wbfs.clone();
    multi[13] = 1;
    let mut split = wbfs.clone();
    split.pop();
    for (name, bytes) in [
        ("GameCube.iso", gamecube_fixture()),
        ("Legacy.iso", legacy),
        ("Game.nkit.iso", iso.clone()),
        ("Game.wia", iso.clone()),
        ("Game.gcz", iso.clone()),
        ("Game.rvz", iso.clone()),
        ("Small.iso", iso[..100].to_vec()),
        ("Bad.iso", geometry),
        ("Table.iso", table),
        ("Shift.wbfs", shift),
        ("Multi.wbfs", multi),
        ("Split.wbfs", split),
        ("Renamed.iso", wbfs),
        ("NotWBFS.wbfs", iso),
    ] {
        let created = stage_wii(&app, name, &bytes).await;
        let failed = poll(&app, created["status_url"].as_str().unwrap(), "failed").await;
        assert!(failed["proposal"].is_null());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM roms")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!(count, 0);
    assert_eq!(
        std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn conversion_is_opt_in_admin_only_and_restart_removes_abandoned_inputs() {
    let app = TestApp::with_config(|config| {
        let path = config
            .data_dir
            .join(".conversion-staging/import-interrupted");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("source"), b"private").unwrap();
    })
    .await;
    app.seed_admin("admin", "pass").await;
    app.seed_readonly("reader", "pass").await;
    assert!(
        !app.state
            .config()
            .data_dir
            .join(".conversion-staging/import-interrupted")
            .exists()
    );
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/conversion/status",
            Some(("reader", "pass")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Game")],
            &[],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}
