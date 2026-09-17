mod support;

use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Json, Router,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use teatro::config::IgdbConfig;
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

async fn create_scan(app: &support::TestApp, auth: Option<(&str, &str)>) -> (String, Value) {
    let response = app
        .router
        .clone()
        .oneshot(support::empty_request(
            "POST",
            "/api/admin/library/scans",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let location = response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body = support::response_json(response).await;
    assert_eq!(body["job"]["state"], "queued");
    assert_eq!(body["job"]["phase"], "queued");
    assert!(
        !body["job"]
            .as_object()
            .unwrap()
            .contains_key("last_event_seq")
    );
    assert_eq!(body["status_url"], location);
    assert!(body["job"]["id"].as_str().unwrap().starts_with("scan_"));
    (location, body)
}

async fn wait_for_scan(
    app: &support::TestApp,
    location: &str,
    auth: Option<(&str, &str)>,
) -> Value {
    for _ in 0..200 {
        let response = app
            .router
            .clone()
            .oneshot(support::empty_request("GET", location, auth))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = support::response_json(response).await;
        let resource = body.as_object().unwrap();
        assert!(!resource.contains_key("events"));
        assert!(!resource.contains_key("next_event_seq"));
        if matches!(body["state"].as_str(), Some("succeeded" | "failed")) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("library scan did not finish");
}

#[tokio::test]
async fn scan_imports_in_place_generates_playlists_reports_skips_and_is_idempotent() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = app.state.config().default_library_root.clone();

    fs::create_dir_all(root.join("genesis")).unwrap();
    fs::write(root.join("genesis/Standalone.bin"), b"standalone").unwrap();
    let source_game_folder = root.join("psx/Saga");
    let game_folder = root.join("psx/Saga.m3u");
    fs::create_dir_all(&source_game_folder).unwrap();
    fs::write(source_game_folder.join("Saga (Disc 1).chd"), b"disc one").unwrap();
    fs::write(source_game_folder.join("Saga (Disc 2).chd"), b"disc two").unwrap();
    fs::create_dir_all(root.join("not-a-platform")).unwrap();
    fs::write(root.join("not-a-platform/Private.rom"), b"private").unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["phase"], "complete");
    assert!(terminal["progress"].is_null());
    assert!(terminal["error"].is_null());
    let result = &terminal["result"];
    assert_eq!(result["scanned_file_count"], 4);
    assert_eq!(result["imported_rom_count"], 2);
    assert_eq!(result["imported_file_count"], 3);
    assert_eq!(result["generated_manifest_count"], 1);
    assert_eq!(result["cover_downloaded_count"], 0);
    assert_eq!(result["cover_not_downloaded_count"], 2);
    assert_eq!(result["cover_warnings"].as_array().unwrap().len(), 2);
    assert_eq!(result["already_indexed_file_count"], 0);
    assert_eq!(result["not_imported_file_count"], 1);
    assert_eq!(result["unimported_file_count"], 1);
    assert_eq!(
        result["unimported_files"][0]["relative_path"],
        "not-a-platform/Private.rom"
    );
    assert_eq!(
        result["unimported_files"][0]["reason_code"],
        "unknown_platform_directory"
    );

    assert_eq!(
        fs::read(root.join("genesis/Standalone.bin")).unwrap(),
        b"standalone"
    );
    assert!(!source_game_folder.exists());
    assert_eq!(
        fs::read(game_folder.join("Saga (Disc 1).chd")).unwrap(),
        b"disc one"
    );
    assert_eq!(
        fs::read(game_folder.join("Saga (Disc 2).chd")).unwrap(),
        b"disc two"
    );
    let playlist = fs::read_to_string(game_folder.join("Saga.m3u")).unwrap();
    assert_eq!(playlist, "Saga (Disc 1).chd\nSaga (Disc 2).chd\n");

    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    let file_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rom_files")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    let dependency_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rom_file_dependencies")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    assert_eq!((rom_count, file_count, dependency_count), (2, 4, 2));

    let (second_location, _) = create_scan(&app, auth).await;
    let second = wait_for_scan(&app, &second_location, auth).await;
    assert_eq!(second["state"], "succeeded");
    assert_eq!(second["result"]["imported_rom_count"], 0);
    assert_eq!(second["result"]["imported_file_count"], 0);
    assert_eq!(second["result"]["cover_downloaded_count"], 0);
    assert_eq!(second["result"]["cover_not_downloaded_count"], 0);
    assert_eq!(second["result"]["already_indexed_file_count"], 4);
    assert_eq!(second["result"]["not_imported_file_count"], 1);
    assert_eq!(second["result"]["unimported_file_count"], 5);

    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM roms), (SELECT COUNT(*) FROM rom_files), (SELECT COUNT(*) FROM rom_file_dependencies)",
    )
    .fetch_one(app.state.db())
    .await
    .unwrap();
    assert_eq!(counts, (2, 4, 2));

    let audits: Vec<String> = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_log WHERE action = 'roms.library_scanned' ORDER BY id",
    )
    .fetch_all(app.state.db())
    .await
    .unwrap();
    assert_eq!(audits.len(), 2);
    assert!(
        audits
            .iter()
            .all(|audit| !audit.contains(&root.display().to_string()))
    );
    assert!(audits.iter().all(|audit| !audit.contains("Standalone.bin")));
}

#[tokio::test]
async fn scan_renames_a_multidisc_folder_to_match_its_supplied_m3u() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let platform = app.state.config().default_library_root.join("psx");
    let source = platform.join("Wrong Folder");
    let destination = platform.join("Custom Order.m3u");
    fs::create_dir_all(&source).unwrap();
    fs::write(
        source.join("Custom Order.m3u"),
        b"Custom Order (Disc 2).chd\nCustom Order (Disc 1).chd\n",
    )
    .unwrap();
    fs::write(source.join("Custom Order (Disc 1).chd"), b"one").unwrap();
    fs::write(source.join("Custom Order (Disc 2).chd"), b"two").unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;

    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["result"]["imported_rom_count"], 1);
    assert_eq!(terminal["result"]["generated_manifest_count"], 0);
    assert!(!source.exists());
    assert_eq!(
        fs::read_to_string(destination.join("Custom Order.m3u")).unwrap(),
        "Custom Order (Disc 2).chd\nCustom Order (Disc 1).chd\n"
    );
    let paths: Vec<String> =
        sqlx::query_scalar("SELECT relative_path FROM rom_files ORDER BY sort_index, id")
            .fetch_all(app.state.db())
            .await
            .unwrap();
    assert!(
        paths
            .iter()
            .all(|path| path.starts_with("psx/Custom Order.m3u/"))
    );
}

#[tokio::test]
async fn scan_preserves_and_reports_sidecars_without_blocking_grouped_ingest() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = &app.state.config().default_library_root;
    let game = root.join("psx/Complete.m3u");
    let unknown = root.join("unsupported-folder/nested/game");
    fs::create_dir_all(game.join("artwork")).unwrap();
    fs::create_dir_all(&unknown).unwrap();
    fs::write(
        game.join("Complete.m3u"),
        b"Complete (Disc 1).chd\nComplete (Disc 2).chd\n",
    )
    .unwrap();
    fs::write(game.join("Complete (Disc 1).chd"), b"disc one").unwrap();
    fs::write(game.join("Complete (Disc 2).chd"), b"disc two").unwrap();
    let sidecars = [
        (game.join("Notes.txt"), b"operator note".as_slice()),
        (game.join("cover.jpg"), b"cover".as_slice()),
        (game.join("artwork/cover.jpg"), b"nested cover".as_slice()),
        (game.join("Game.chd:Zone.Identifier"), b"zone".as_slice()),
        (unknown.join("save.json"), b"save".as_slice()),
        (unknown.join("manual.xml"), b"manual".as_slice()),
    ];
    for (path, contents) in &sidecars {
        fs::write(path, contents).unwrap();
    }

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["result"]["imported_rom_count"], 1);
    assert_eq!(terminal["result"]["imported_file_count"], 3);
    assert_eq!(terminal["result"]["generated_manifest_count"], 0);
    let files = terminal["result"]["unimported_files"].as_array().unwrap();
    assert_eq!(
        files
            .iter()
            .filter(|file| file["reason_code"] == "ignored_sidecar")
            .count(),
        sidecars.len()
    );
    for (path, contents) in &sidecars {
        assert_eq!(fs::read(path).unwrap(), *contents);
    }

    let (location, _) = create_scan(&app, auth).await;
    let repeated = wait_for_scan(&app, &location, auth).await;
    assert_eq!(repeated["state"], "succeeded");
    assert_eq!(repeated["result"]["imported_rom_count"], 0);
    for (path, contents) in &sidecars {
        assert_eq!(fs::read(path).unwrap(), *contents);
    }
}

#[tokio::test]
async fn indexed_sidecar_prevents_scan_folder_rename() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = &app.state.config().default_library_root;
    let game = root.join("psx/Saga");
    fs::create_dir_all(&game).unwrap();
    fs::write(game.join("Saga (Disc 1).chd"), b"disc one").unwrap();
    fs::write(game.join("Saga (Disc 2).chd"), b"disc two").unwrap();
    fs::write(game.join("Notes.txt"), b"indexed note").unwrap();
    fs::write(game.join(".Identifier"), b"indexed metadata").unwrap();

    let rom_id = app.seed_rom("psx", "Indexed note", "indexed-note").await;
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary) VALUES (?, ?, 'psx/Saga/Notes.txt', 'Notes.txt', 12, 1), (?, ?, 'psx/Saga/.Identifier', '.Identifier', 16, 0)",
    )
    .bind(rom_id)
    .bind(root_id)
    .bind(rom_id)
    .bind(root_id)
    .execute(app.state.db())
    .await
    .unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["result"]["imported_rom_count"], 0);
    assert_eq!(terminal["result"]["generated_manifest_count"], 0);
    assert!(game.is_dir());
    assert!(!root.join("psx/Saga.m3u").exists());
    assert_eq!(fs::read(game.join("Notes.txt")).unwrap(), b"indexed note");
    assert_eq!(
        fs::read(game.join(".Identifier")).unwrap(),
        b"indexed metadata"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn sidecar_cleanup_preserves_asset_path_with_backslash_lookalike() {
    let app = support::TestApp::with_config(|config| {
        config.asset_root = config.default_library_root.join("assets");
    })
    .await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = &app.state.config().default_library_root;
    let asset = root.join("assets/keep.jpg");
    let lookalike = root.join(r"assets\keep.jpg");
    fs::write(&asset, b"protected asset").unwrap();
    fs::write(&lookalike, b"unindexed sidecar").unwrap();

    let response = app
        .router
        .clone()
        .oneshot(support::empty_request(
            "GET",
            "/api/admin/library/sidecars",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = support::response_json(response).await;
    assert_eq!(preview["file_count"], 1);
    assert_eq!(preview["files"][0]["relative_path"], r"assets\keep.jpg");

    let response = app
        .router
        .clone()
        .oneshot(support::json_request(
            "DELETE",
            "/api/admin/library/sidecars",
            auth,
            json!({"confirm": "DELETE SIDECARS", "files": preview["files"]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!lookalike.exists());
    assert_eq!(fs::read(asset).unwrap(), b"protected asset");
}

#[tokio::test]
async fn sidecar_cleanup_requires_a_matching_preview_and_uses_recoverable_deletion() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = &app.state.config().default_library_root;
    let candidate = root.join("unsupported/nested/save.json");
    let second_candidate = root.join("psx/Notes.txt");
    let indexed = root.join("genesis/indexed.txt");
    let asset = app.state.config().asset_root.join("keep.jpg");
    for path in [&candidate, &second_candidate, &indexed, &asset] {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
    }
    fs::write(&candidate, b"save").unwrap();
    fs::write(&second_candidate, b"notes").unwrap();
    fs::write(&indexed, b"indexed").unwrap();
    fs::write(&asset, b"asset").unwrap();

    let rom_id = app
        .seed_rom("genesis", "Indexed note", "indexed-note")
        .await;
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary) VALUES (?, ?, 'genesis/indexed.txt', 'indexed.txt', 7, 1)",
    )
    .bind(rom_id)
    .bind(root_id)
    .execute(app.state.db())
    .await
    .unwrap();

    let response = app
        .router
        .clone()
        .oneshot(support::empty_request(
            "GET",
            "/api/admin/library/sidecars",
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let preview = support::response_json(response).await;
    assert_eq!(preview["file_count"], 2);
    assert_eq!(preview["truncated"], false);
    assert_eq!(
        preview["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|file| file["relative_path"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["psx/Notes.txt", "unsupported/nested/save.json"]
    );

    fs::write(&candidate, b"changed after preview").unwrap();
    let response = app
        .router
        .clone()
        .oneshot(support::json_request(
            "DELETE",
            "/api/admin/library/sidecars",
            auth,
            json!({"confirm": "DELETE SIDECARS", "files": preview["files"]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(fs::read(&second_candidate).unwrap(), b"notes");

    let response = app
        .router
        .clone()
        .oneshot(support::empty_request(
            "GET",
            "/api/admin/library/sidecars",
            auth,
        ))
        .await
        .unwrap();
    let current = support::response_json(response).await;
    let response = app
        .router
        .clone()
        .oneshot(support::json_request(
            "DELETE",
            "/api/admin/library/sidecars",
            auth,
            json!({"confirm": "DELETE SIDECARS", "files": current["files"]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = support::response_json(response).await;
    assert_eq!(deleted["deleted_file_count"], 2);
    assert!(!candidate.exists());
    assert!(!second_candidate.exists());
    assert!(root.join("unsupported/nested").is_dir());
    assert_eq!(fs::read(indexed).unwrap(), b"indexed");
    assert_eq!(fs::read(asset).unwrap(), b"asset");

    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE kind = 'delete' ORDER BY created_at DESC, id DESC LIMIT 1",
    )
    .fetch_one(app.state.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
    let audit: String = sqlx::query_scalar(
        "SELECT metadata_json FROM audit_log WHERE action = 'library.sidecars_deleted'",
    )
    .fetch_one(app.state.db())
    .await
    .unwrap();
    assert!(audit.contains("\"deleted_file_count\":2"));
    assert!(!audit.contains("save.json"));
}

#[tokio::test]
async fn scan_admission_returns_before_the_worker_can_acquire_the_root_lock() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = app
        .state
        .config()
        .default_library_root
        .canonicalize()
        .unwrap();
    let guard = app.state.file_store().lock_root(&root).await.unwrap();

    let admitted = tokio::time::timeout(Duration::from_secs(2), create_scan(&app, auth))
        .await
        .expect("scan admission must not wait for the managed-root lock");
    let location = admitted.0;
    let live = app
        .router
        .clone()
        .oneshot(support::empty_request("GET", &location, auth))
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    let live = support::response_json(live).await;
    assert!(matches!(live["state"].as_str(), Some("queued" | "running")));

    drop(guard);
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
}

#[tokio::test]
async fn failed_scan_preserves_sidecars() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = app
        .state
        .config()
        .default_library_root
        .canonicalize()
        .unwrap();
    let sidecar = root.join("psx/failed-scan-note.txt");
    fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    fs::write(&sidecar, b"keep after failure").unwrap();
    let guard = app.state.file_store().lock_root(&root).await.unwrap();
    let (location, _) = create_scan(&app, auth).await;
    sqlx::query("DROP TABLE platforms")
        .execute(app.state.db())
        .await
        .unwrap();
    drop(guard);

    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "failed");
    assert_eq!(fs::read(sidecar).unwrap(), b"keep after failure");
}

#[tokio::test]
async fn admitted_scan_can_be_cancelled_and_remains_terminal() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let root = app
        .state
        .config()
        .default_library_root
        .canonicalize()
        .unwrap();
    let sidecar = root.join("psx/cancelled-scan-note.txt");
    fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
    fs::write(&sidecar, b"keep after cancellation").unwrap();
    let guard = app.state.file_store().lock_root(&root).await.unwrap();
    let (location, _) = create_scan(&app, auth).await;

    let response = app
        .router
        .clone()
        .oneshot(support::empty_request("DELETE", &location, auth))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cancelled = support::response_json(response).await;
    assert_eq!(cancelled["state"], "cancelled");
    assert!(cancelled["progress"].is_null());
    drop(guard);

    tokio::time::sleep(Duration::from_millis(20)).await;
    let response = app
        .router
        .clone()
        .oneshot(support::empty_request("GET", &location, auth))
        .await
        .unwrap();
    assert_eq!(support::response_json(response).await["state"], "cancelled");
    assert_eq!(fs::read(sidecar).unwrap(), b"keep after cancellation");
}

struct MockIgdb {
    base_url: String,
    games_requests: Arc<AtomicUsize>,
    handle: JoinHandle<()>,
}

impl MockIgdb {
    async fn spawn() -> Self {
        Self::spawn_with_scoped_miss(false).await
    }

    async fn spawn_with_scoped_miss(scoped_miss: bool) -> Self {
        let games_requests = Arc::new(AtomicUsize::new(0));
        let request_counter = games_requests.clone();
        let router = Router::new()
            .route(
                "/oauth2/token",
                post(|| async {
                    Json(json!({
                        "access_token": "scan-test-token",
                        "expires_in": 3600,
                        "token_type": "bearer"
                    }))
                }),
            )
            .route(
                "/v4/games",
                post(move |body: String| {
                    let request_counter = request_counter.clone();
                    async move {
                        request_counter.fetch_add(1, Ordering::Relaxed);
                        if scoped_miss && body.contains("where platforms") {
                            return Json(json!([]));
                        }
                        Json(json!([{
                            "id": 123,
                            "name": "Sonic The Hedgehog",
                            "platforms": [{ "id": 29, "name": "Sega Mega Drive/Genesis" }],
                            "cover": { "image_id": "co123" }
                        }]))
                    }
                }),
            )
            .route(
                "/images/t_cover_big/co123.jpg",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "image/jpeg")],
                        b"large-cover".to_vec(),
                    )
                        .into_response()
                }),
            )
            .route(
                "/images/t_cover_small/co123.jpg",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "image/jpeg")],
                        b"small-cover".to_vec(),
                    )
                        .into_response()
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self {
            base_url: format!("http://{address}"),
            games_requests,
            handle,
        }
    }

    fn config(&self) -> IgdbConfig {
        IgdbConfig {
            client_id: Some("test-client".into()),
            client_secret: Some("test-secret".into()),
            token_url: format!("{}/oauth2/token", self.base_url),
            api_url: format!("{}/v4", self.base_url),
            image_base_url: format!("{}/images", self.base_url),
        }
    }
}

impl Drop for MockIgdb {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

#[tokio::test]
async fn scan_downloads_igdb_covers_after_selected_platform_search_misses() {
    let igdb = MockIgdb::spawn_with_scoped_miss(true).await;
    let igdb_config = igdb.config();
    let app = support::TestApp::with_config(|config| {
        config.igdb = igdb_config;
        config.asset_root = config.default_library_root.clone();
    })
    .await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let game_path = app
        .state
        .config()
        .default_library_root
        .join("genesis/Sonic The Hedgehog.bin");
    fs::create_dir_all(game_path.parent().unwrap()).unwrap();
    fs::write(&game_path, b"sonic").unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["result"]["cover_downloaded_count"], 1);
    assert_eq!(terminal["result"]["cover_not_downloaded_count"], 0);
    assert_eq!(terminal["result"]["cover_warnings"], json!([]));
    assert_eq!(igdb.games_requests.load(Ordering::Relaxed), 2);

    let (large, small): (String, String) = sqlx::query_as(
        "SELECT path_cover_large, path_cover_small FROM roms WHERE name = 'Sonic The Hedgehog'",
    )
    .fetch_one(app.state.db())
    .await
    .unwrap();
    assert_eq!(
        fs::read(app.state.config().asset_root.join(&large)).unwrap(),
        b"large-cover"
    );
    assert_eq!(
        fs::read(app.state.config().asset_root.join(&small)).unwrap(),
        b"small-cover"
    );

    let (location, _) = create_scan(&app, auth).await;
    assert_eq!(
        wait_for_scan(&app, &location, auth).await["state"],
        "succeeded"
    );
    assert_eq!(
        fs::read(app.state.config().asset_root.join(&large)).unwrap(),
        b"large-cover"
    );
    assert_eq!(
        fs::read(app.state.config().asset_root.join(&small)).unwrap(),
        b"small-cover"
    );
    assert_eq!(igdb.games_requests.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn scan_retries_automatic_igdb_across_all_platforms_when_unmapped() {
    let igdb = MockIgdb::spawn().await;
    let igdb_config = igdb.config();
    let app = support::TestApp::with_config(|config| config.igdb = igdb_config).await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let game_path = app
        .state
        .config()
        .default_library_root
        .join("sgb/Unsupported Game.bin");
    fs::create_dir_all(game_path.parent().unwrap()).unwrap();
    fs::write(&game_path, b"unsupported").unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    assert_eq!(terminal["state"], "succeeded");
    assert_eq!(terminal["result"]["cover_downloaded_count"], 1);
    assert_eq!(terminal["result"]["cover_not_downloaded_count"], 0);
    assert_eq!(terminal["result"]["cover_warnings"], json!([]));
    assert_eq!(igdb.games_requests.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn deleting_a_scanned_package_removes_its_complete_folder_not_the_platform() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let platform = app.state.config().default_library_root.join("psx");
    let source_package = platform.join("Delete Package");
    let package = platform.join("Delete Package.m3u");
    fs::create_dir_all(&source_package).unwrap();
    fs::write(source_package.join("Delete Package (Disc 1).chd"), b"one").unwrap();
    fs::write(source_package.join("Delete Package (Disc 2).chd"), b"two").unwrap();
    fs::write(platform.join("Keep Me.bin"), b"keep").unwrap();

    let (location, _) = create_scan(&app, auth).await;
    let terminal = wait_for_scan(&app, &location, auth).await;
    let package_rom = terminal["result"]["imported_roms"]
        .as_array()
        .unwrap()
        .iter()
        .find(|rom| rom["name"] == "Delete Package")
        .unwrap();
    let rom_id = package_rom["id"].as_i64().unwrap();
    assert!(!source_package.exists());
    fs::write(package.join(".Identifier"), b"copy metadata").unwrap();

    let response = app
        .router
        .clone()
        .oneshot(support::empty_request(
            "DELETE",
            &format!("/api/admin/roms/{rom_id}"),
            auth,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let deleted = support::response_json(response).await;
    assert_eq!(deleted["deleted_files"].as_array().unwrap().len(), 3);
    assert!(
        !package.exists(),
        "the package folder and sidecars must be removed"
    );
    assert!(
        platform.exists(),
        "the platform directory must never be removed"
    );
    assert_eq!(fs::read(platform.join("Keep Me.bin")).unwrap(), b"keep");
}

#[tokio::test]
async fn scan_endpoints_require_admin_and_hide_unknown_jobs() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let readonly = app.seed_readonly("reader", "password").await;

    for auth in [
        None,
        Some((readonly.username.as_str(), readonly.password.as_str())),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(support::empty_request(
                "POST",
                "/api/admin/library/scans",
                auth,
            ))
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
        let response = app
            .router
            .clone()
            .oneshot(support::empty_request(
                "GET",
                "/api/admin/library/sidecars",
                auth,
            ))
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }

    for id in ["invalid", "scan_unknown"] {
        let response = app
            .router
            .clone()
            .oneshot(support::empty_request(
                "GET",
                &format!("/api/admin/library/scans/{id}"),
                Some((admin.username.as_str(), admin.password.as_str())),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
