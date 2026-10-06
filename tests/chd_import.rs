mod support;
use axum::http::StatusCode;
use serde_json::Value;
use support::{TestApp, empty_request, json_request, multipart_request, response_json};
use tower::ServiceExt;

fn iso(platform: &str) -> Vec<u8> {
    let mut bytes = vec![42; 64 * 2048];
    let pvd = &mut bytes[16 * 2048..17 * 2048];
    pvd.fill(0);
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    pvd[80..84].copy_from_slice(&64_u32.to_le_bytes());
    pvd[84..88].copy_from_slice(&64_u32.to_be_bytes());
    pvd[128..132].copy_from_slice(&[0, 8, 8, 0]);
    if platform == "psp" {
        pvd[8..16].copy_from_slice(b"PSP GAME");
    }
    pvd[156] = 34;
    pvd[158..162].copy_from_slice(&20_u32.to_le_bytes());
    pvd[166..170].copy_from_slice(&2048_u32.to_le_bytes());
    let root = &mut bytes[20 * 2048..21 * 2048];
    root.fill(0);
    let name = b"SYSTEM.CNF;1";
    root[0] = (33 + name.len()) as u8;
    root[2..6].copy_from_slice(&21_u32.to_le_bytes());
    root[10..14].copy_from_slice(&32_u32.to_le_bytes());
    root[32] = name.len() as u8;
    root[33..33 + name.len()].copy_from_slice(name);
    bytes[21 * 2048..21 * 2048 + 32].fill(0);
    let boot: &[u8] = if platform == "ps2" { b"BOOT2" } else { b"BOOT" };
    bytes[21 * 2048..21 * 2048 + boot.len()].copy_from_slice(boot);
    bytes
}
fn cd(platform: &str) -> Vec<u8> {
    let iso = iso(if platform == "ps2" { "ps2" } else { "psx" });
    let mut raw = vec![0; 64 * 2352];
    for (sector, payload) in raw.chunks_exact_mut(2352).zip(iso.chunks_exact(2048)) {
        sector[..12].copy_from_slice(&[0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]);
        sector[15] = 1;
        sector[16..2064].copy_from_slice(payload);
    }
    if platform == "saturn" {
        raw[16..32].copy_from_slice(b"SEGA SEGASATURN ");
    }
    if platform == "segacd" {
        raw[16..30].copy_from_slice(b"SEGADISCSYSTEM");
    }
    if matches!(platform, "pcenginecd" | "neogeocd") {
        raw[16 * 2352 + 16..16 * 2352 + 22].fill(0);
    }
    raw
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
async fn stage(app: &TestApp, platform: &str, files: &[(String, Vec<u8>)]) -> String {
    let files: Vec<_> = files
        .iter()
        .map(|(name, data)| ("files", name.as_str(), data.as_slice()))
        .collect();
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Edited title"), ("platform_slug", platform)],
            &files,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    response_json(response).await["status_url"]
        .as_str()
        .unwrap()
        .into()
}
async fn wait(app: &TestApp, url: &str) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let response = app
                .router
                .clone()
                .oneshot(empty_request("GET", url, Some(("admin", "pass"))))
                .await
                .unwrap();
            let job = response_json(response).await;
            assert!(job["proposal"].is_null());
            assert_ne!(job["phase"], "awaiting_review");
            if matches!(
                job["state"].as_str(),
                Some("failed" | "succeeded" | "cancelled")
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
async fn multidisc_multitrack_chds_preserve_bytes_sbi_and_generated_or_supplied_playlist_order() {
    use rom_converto_lib::{
        disc::chd::extract_from_chd,
        util::{CancelToken, NoProgress},
    };
    for supplied in [false, true] {
        let app = enabled().await;
        let data = cd("psx");
        let audio: Vec<u8> = (0..2352 * 7).map(|i| (i % 251) as u8).collect();
        let mut files = Vec::new();
        for disc in [2, 1] {
            let name = format!("Game (Disc {disc})");
            let ext = if supplied { "img" } else { "bin" };
            files.push((format!("{name}.cue"), format!("FILE \"{name} data.{ext}\" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\nFILE \"{name} audio.bin\" BINARY\n TRACK 02 AUDIO\n INDEX 01 00:00:00\n").into_bytes()));
            let mut disc_data = data.clone();
            disc_data[60 * 2352 + 24] = disc;
            files.push((format!("{name} data.{ext}"), disc_data));
            files.push((format!("{name} audio.bin"), audio.clone()));
            files.push((
                format!("{name}.sbi"),
                b"SBI\0synthetic corrections".to_vec(),
            ));
        }
        if supplied {
            files.push((
                "Custom order.m3u".into(),
                b"# keep requested order\nGame (Disc 2).cue\nGame (Disc 1).cue\n".to_vec(),
            ));
        }
        let url = stage(&app, "psx", &files).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "succeeded", "{job}");
        assert_eq!(job["output_format"], "chd");
        assert_eq!(job["result"]["output_format"], "chd");
        assert_eq!(job["result"]["title"], "Edited title");
        let id = job["result"]["rom_id"].as_i64().unwrap();
        let rows: Vec<(String, String, String)> = sqlx::query_as(
            "SELECT file_name,relative_path,role FROM rom_files WHERE rom_id=? ORDER BY sort_index",
        )
        .bind(id)
        .fetch_all(app.state.db())
        .await
        .unwrap();
        assert_eq!(rows.len(), 5, "{rows:?}");
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM roms")
                .fetch_one(app.state.db())
                .await
                .unwrap(),
            1
        );
        let playlist = rows.iter().find(|row| row.0.ends_with(".m3u")).unwrap();
        let text =
            std::fs::read_to_string(app.state.config().default_library_root.join(&playlist.1))
                .unwrap();
        assert_eq!(
            text,
            if supplied {
                "Game (Disc 2).chd\nGame (Disc 1).chd\n"
            } else {
                "Game (Disc 1).chd\nGame (Disc 2).chd\n"
            }
        );
        assert!(rows.iter().all(|row| !row.0.ends_with(".bin")
            && !row.0.ends_with(".cue")
            && !row.0.ends_with(".img")));
        for row in &rows {
            let path = app.state.config().default_library_root.join(&row.1);
            if row.0.ends_with(".chd") {
                let out = app.temp_dir.path().join("restored.cue");
                extract_from_chd(&NoProgress, path, out.clone(), None, CancelToken::new())
                    .await
                    .unwrap();
                let mut expected = data.clone();
                expected[60 * 2352 + 24] = if row.0.contains("Disc 1") { 1 } else { 2 };
                assert_eq!(
                    std::fs::read(out.with_extension("bin")).unwrap(),
                    [expected, audio.clone()].concat()
                );
            } else if row.0.ends_with(".sbi") {
                assert_eq!(row.2, "metadata_sidecar");
                assert_eq!(std::fs::read(path).unwrap(), b"SBI\0synthetic corrections");
            }
        }
        let plan = app
            .router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/roms/{id}/download-plan"),
                Some(("admin", "pass")),
            ))
            .await
            .unwrap();
        assert_eq!(plan.status(), StatusCode::OK);
        assert_eq!(
            response_json(plan).await["files"].as_array().unwrap().len(),
            5
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
async fn plain_psp_iso_and_allowlisted_cd_platforms_round_trip_without_confirmation() {
    let app = enabled().await;
    for platform in [
        "psp",
        "ps2",
        "saturn",
        "saturnjp",
        "segacd",
        "megacd",
        "megacdjp",
        "pcenginecd",
        "tg-cd",
        "neogeocd",
    ] {
        let family = match platform {
            "saturnjp" => "saturn",
            "megacd" | "megacdjp" => "segacd",
            "tg-cd" => "pcenginecd",
            other => other,
        };
        let name = format!("Synthetic {platform}");
        let files = if platform == "psp" {
            vec![(format!("{name}.iso"), iso("psp"))]
        } else {
            vec![
                (
                    format!("{name}.cue"),
                    format!(
                        "FILE \"{name}.bin\" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n"
                    )
                    .into_bytes(),
                ),
                (format!("{name}.bin"), cd(family)),
            ]
        };
        let url = stage(&app, platform, &files).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "succeeded", "{platform}: {job}");
        assert_eq!(job["result"]["file_name"], format!("{name}.chd"));
    }
}

#[tokio::test]
async fn psx_iso_disc_sets_and_standalone_images_keep_grouping_and_decoded_bytes() {
    use rom_converto_lib::{
        disc::chd::{extract_from_chd, is_dvd_mode_chd},
        util::{CancelToken, NoProgress},
    };
    let app = enabled().await;
    let cooked = iso("psx");
    let raw = cd("psx");
    let mut mode2 = vec![0; 64 * 2352];
    for (sector, payload) in mode2.chunks_exact_mut(2352).zip(cooked.chunks_exact(2048)) {
        sector[..12].copy_from_slice(&raw[..12]);
        sector[15] = 2;
        sector[24..2072].copy_from_slice(payload);
    }
    for supplied in [false, true] {
        let mut files = vec![
            ("Game (Disc 2).img".into(), raw.clone()),
            ("Game (Disc 1).iso".into(), cooked.clone()),
        ];
        if supplied {
            files.push((
                "Custom.m3u".into(),
                b"Game (Disc 2).img\nGame (Disc 1).iso\n".to_vec(),
            ));
        }
        let url = stage(&app, "psx", &files).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "succeeded", "{job}");
        let id = job["result"]["rom_id"].as_i64().unwrap();
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT file_name,relative_path FROM rom_files WHERE rom_id=?")
                .bind(id)
                .fetch_all(app.state.db())
                .await
                .unwrap();
        assert_eq!(rows.len(), 3);
        for (name, path) in rows {
            let path = app.state.config().default_library_root.join(path);
            if name.ends_with(".m3u") {
                assert_eq!(
                    std::fs::read_to_string(path).unwrap(),
                    if supplied {
                        "Game (Disc 2).chd\nGame (Disc 1).chd\n"
                    } else {
                        "Game (Disc 1).chd\nGame (Disc 2).chd\n"
                    }
                );
            } else {
                assert!(
                    !is_dvd_mode_chd(path.clone()).await.unwrap(),
                    "PSX must use CD metadata"
                );
                let out = app.temp_dir.path().join("iso-restored.cue");
                extract_from_chd(&NoProgress, path, out.clone(), None, CancelToken::new())
                    .await
                    .unwrap();
                assert_eq!(
                    std::fs::read(out.with_extension("bin")).unwrap(),
                    if name.contains("Disc 1") {
                        cooked.clone()
                    } else {
                        raw.clone()
                    }
                );
            }
        }
    }
    for (platform, name, bytes, dvd) in [
        ("psx", "Cooked.img", cooked, false),
        ("psx", "Raw.iso", raw, false),
        ("psx", "Mode2.IMG", mode2, false),
        ("ps2", "CD.iso", iso("ps2"), false),
        ("psp", "UMD.img", iso("psp"), true),
        ("saturn", "Saturn.img", cd("saturn"), false),
    ] {
        let url = stage(&app, platform, &[(name.into(), bytes.clone())]).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "succeeded", "{name}: {job}");
        let path: String = sqlx::query_scalar("SELECT relative_path FROM rom_files WHERE rom_id=?")
            .bind(job["result"]["rom_id"].as_i64().unwrap())
            .fetch_one(app.state.db())
            .await
            .unwrap();
        let path = app.state.config().default_library_root.join(path);
        assert_eq!(is_dvd_mode_chd(path.clone()).await.unwrap(), dvd);
        let out = app.temp_dir.path().join(if dvd {
            "image-restored.iso"
        } else {
            "image-restored.cue"
        });
        extract_from_chd(&NoProgress, path, out.clone(), None, CancelToken::new())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(if dvd { out } else { out.with_extension("bin") }).unwrap(),
            bytes
        );
    }
}

#[tokio::test]
async fn review_detects_a_disc_set_and_separate_images_before_independent_conversion_jobs() {
    let app = enabled().await;
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug='psx'")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    let inputs = [
        "Other.iso",
        "Game (Disc 2).iso",
        "Standalone.img",
        "Game (Disc 1).iso",
    ];
    let response = app.router.clone().oneshot(json_request("POST", "/api/admin/uploads/preview", Some(("admin", "pass")), serde_json::json!({
        "platform_id":platform_id,
        "files":inputs.iter().map(|name| serde_json::json!({"file_name":name,"file_size_bytes":64*2048})).collect::<Vec<_>>()
    }))).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let plan = response_json(response).await;
    assert_eq!(plan["errors"], serde_json::json!([]));
    let roms = plan["roms"].as_array().unwrap();
    assert_eq!(roms.len(), 3, "{plan}");
    for rom in roms {
        let files: Vec<_> = rom["files"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|file| file["metadata"]["source"] != "generated")
            .map(|file| {
                (
                    file["original_file_name"].as_str().unwrap().to_string(),
                    iso("psx"),
                )
            })
            .collect();
        let expected_files = files.len() + usize::from(files.len() > 1);
        let url = stage(&app, "psx", &files).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "succeeded", "{job}");
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rom_files WHERE rom_id=?")
            .bind(job["result"]["rom_id"].as_i64().unwrap())
            .fetch_one(app.state.db())
            .await
            .unwrap();
        assert_eq!(count as usize, expected_files);
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM roms")
            .fetch_one(app.state.db())
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn disc_input_count_limit_cleans_private_staging_before_job_launch() {
    let app = TestApp::with_config(|config| {
        config.conversion_enabled = true;
        config.uploads.free_space_margin_bytes = 0;
        config.uploads.max_batch_files = 1;
    })
    .await;
    app.seed_admin("admin", "pass").await;
    let data = cd("psx");
    let response = app
        .router
        .clone()
        .oneshot(multipart_request(
            "POST",
            "/api/admin/conversion-imports",
            Some(("admin", "pass")),
            &[("title", "Game"), ("platform_slug", "psx")],
            &[
                (
                    "files",
                    "Game.cue",
                    b"FILE \"Game.bin\" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n",
                ),
                ("files", "Game.bin", &data),
            ],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        std::fs::read_dir(app.state.config().data_dir.join(".conversion-staging"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM rom_files")
            .fetch_one(app.state.db())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn incomplete_unsafe_wrong_platform_and_lossy_track_layouts_never_publish() {
    let app = enabled().await;
    let cue = b"FILE \"disc.bin\" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n".to_vec();
    let mut mixed = cd("psx");
    mixed[60 * 2352..61 * 2352].fill(0);
    let mut changing_mode = cd("psx");
    changing_mode[60 * 2352 + 15] = 2;
    let cases = vec![
        ("psx", vec![("disc.img".into(), mixed)]),
        ("psx", vec![("disc.img".into(), changing_mode)]),
        ("psx", vec![("disc.img".into(), vec![42; 64 * 2048])]),
        (
            "psx",
            vec![("disc.img".into(), cd("psx")[..64 * 2352 - 1].to_vec())],
        ),
        ("psp", vec![("disc.img".into(), cd("psx"))]),
        ("psx", vec![("disc.iso".into(), iso("psp"))]),
        (
            "psx",
            vec![("disc.iso".into(), iso("psx")[..64 * 2048 - 1].to_vec())],
        ),
        (
            "psx",
            vec![
                ("one.iso".into(), iso("psx")),
                ("two.iso".into(), iso("psx")),
            ],
        ),
        (
            "psx",
            vec![
                ("disc.img".into(), cd("psx")),
                ("disc.sub".into(), vec![0; 96 * 64]),
            ],
        ),
        ("psx", vec![("disc.cue".into(), cue.clone())]),
        (
            "psx",
            vec![
                (
                    "disc.cue".into(),
                    b"FILE \"../outside.bin\" BINARY\n TRACK 01 MODE1/2352\n INDEX 01 00:00:00\n"
                        .to_vec(),
                ),
                ("disc.bin".into(), cd("psx")),
            ],
        ),
        (
            "psx",
            vec![
                ("disc.cue".into(), cue.clone()),
                ("disc.bin".into(), vec![0; 2352 + 1]),
            ],
        ),
        (
            "psx",
            vec![
                (
                    "disc.cue".into(),
                    [cue.clone(), b" INDEX 00 00:00:00\n".to_vec()].concat(),
                ),
                ("disc.bin".into(), cd("psx")),
            ],
        ),
        (
            "psx",
            vec![
                (
                    "disc.cue".into(),
                    [cue.clone(), b" POSTGAP 00:00:01\n".to_vec()].concat(),
                ),
                ("disc.bin".into(), cd("psx")),
            ],
        ),
        (
            "psx",
            vec![
                ("disc.cue".into(), cue.clone()),
                ("disc.bin".into(), cd("saturn")),
            ],
        ),
        ("ps2", vec![("disc.iso".into(), iso("psp"))]),
        ("psp", vec![("disc.cso".into(), iso("psp"))]),
        ("psx", vec![("disc.chd".into(), iso("psp"))]),
        (
            "psx",
            vec![
                ("disc.cue".into(), cue),
                ("disc.bin".into(), cd("psx")),
                ("extra.bin".into(), cd("psx")),
            ],
        ),
    ];
    for (platform, files) in cases {
        let url = stage(&app, platform, &files).await;
        let job = wait(&app, &url).await;
        assert_eq!(job["state"], "failed", "{job}");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM rom_files")
            .fetch_one(app.state.db())
            .await
            .unwrap(),
        0
    );
}
