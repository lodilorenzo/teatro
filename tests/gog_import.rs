mod support;

#[test]
fn asynchronous_job_contract_fixture_is_frozen() {
    use std::collections::BTreeSet;

    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/gog-import-job-contract.json")).unwrap();
    let contract = &fixture["contract"];

    assert_eq!(contract["create_http_status"], 202);
    assert_eq!(
        contract["location"],
        fixture["create_response"]["status_url"]
    );
    assert_eq!(
        contract["job_states"],
        serde_json::json!(["queued", "running", "succeeded", "failed", "cancelled"])
    );
    assert_eq!(
        contract["phases"],
        serde_json::json!([
            "queued",
            "validate_request",
            "resolve_windows_target",
            "verify_innoextract_hash",
            "prepare_private_workspace",
            "stage_setup_files",
            "probe_innoextract_version",
            "probe_installer_data_version",
            "test_installer_integrity",
            "extract_installer_payload",
            "validate_extracted_tree",
            "create_windows_zip",
            "finalize_windows_zip",
            "validate_sipario_zip",
            "stage_generated_archive",
            "publish_windows_archive",
            "cleanup_uploaded_setup_staging",
            "complete"
        ])
    );
    assert_eq!(
        contract["progress_kinds"],
        serde_json::json!(["percent", "bytes"])
    );
    assert_eq!(
        contract["event_kinds"],
        serde_json::json!(["phase", "output"])
    );
    assert_eq!(
        contract["output_streams"],
        serde_json::json!(["stdout", "stderr"])
    );
    assert_eq!(
        contract["limits"],
        serde_json::json!({
            "terminal_job_capacity": 64,
            "terminal_retention_seconds": 3600,
            "event_text_bytes": 262144,
            "event_entries": 512,
            "display_entry_bytes": 4096,
            "progress_updates_per_second": 10,
            "browser_poll_interval_ms": 750,
            "browser_transcript_entries": 512
        })
    );
    assert_eq!(
        contract["transcript_overflow"],
        "evict_oldest_output_preserve_phase_events"
    );
    assert_eq!(contract["browser_job_id_storage"], "session_storage");

    assert_object_keys(&fixture["create_response"], &["job", "status_url"]);
    assert_object_keys(
        &fixture["create_response"]["job"],
        &[
            "id",
            "state",
            "phase",
            "progress",
            "output_truncated",
            "last_event_seq",
            "created_at",
            "updated_at",
        ],
    );
    assert_eq!(fixture["create_response"]["job"]["state"], "queued");
    assert_eq!(fixture["create_response"]["job"]["phase"], "queued");
    assert_eq!(
        fixture["create_response"]["job"]["progress"],
        serde_json::Value::Null
    );

    let status_keys = [
        "id",
        "state",
        "phase",
        "progress",
        "events",
        "next_event_seq",
        "output_truncated",
        "result",
        "error",
        "created_at",
        "updated_at",
    ];
    for name in [
        "running_percent_response",
        "running_bytes_response",
        "succeeded_response",
        "failed_response",
    ] {
        let response = &fixture[name];
        assert_object_keys(response, &status_keys);
        for event in response["events"].as_array().unwrap() {
            assert_object_keys(event, &["seq", "kind", "phase", "stream", "text"]);
            assert!(
                contract["event_kinds"]
                    .as_array()
                    .unwrap()
                    .contains(&event["kind"])
            );
            assert!(
                contract["phases"]
                    .as_array()
                    .unwrap()
                    .contains(&event["phase"])
            );
            if !event["stream"].is_null() {
                assert!(
                    contract["output_streams"]
                        .as_array()
                        .unwrap()
                        .contains(&event["stream"])
                );
            }
        }
    }

    let percent = &fixture["running_percent_response"]["progress"];
    assert_object_keys(percent, &["kind", "current", "total", "percent"]);
    assert_eq!(percent["kind"], "percent");
    assert!(percent["current"].is_null());
    assert!(percent["total"].is_null());
    assert_eq!(percent["percent"], 42.5);

    let bytes = &fixture["running_bytes_response"]["progress"];
    assert_object_keys(bytes, &["kind", "current", "total", "percent"]);
    assert_eq!(bytes["kind"], "bytes");
    assert_eq!(bytes["current"], 2_147_483_648_u64);
    assert_eq!(bytes["total"], 4_294_967_296_u64);
    assert_eq!(bytes["percent"], 50.0);

    let succeeded = &fixture["succeeded_response"];
    assert_eq!(succeeded["state"], "succeeded");
    assert_eq!(succeeded["phase"], "complete");
    assert!(succeeded["progress"].is_null());
    assert!(succeeded["error"].is_null());
    assert_object_keys(
        &succeeded["result"],
        &[
            "rom",
            "file_id",
            "file_name",
            "relative_path",
            "file_size_bytes",
            "import",
        ],
    );
    assert_object_keys(
        &succeeded["result"]["rom"],
        &[
            "id",
            "name",
            "slug",
            "platform_id",
            "platform_slug",
            "platform_display_name",
            "regions",
            "metadatum",
            "summary",
            "fs_name",
            "fs_size_bytes",
            "path_cover_large",
            "path_cover_small",
            "url_cover",
            "files",
        ],
    );
    assert_object_keys(
        &succeeded["result"]["rom"]["files"][0],
        &[
            "id",
            "file_name",
            "file_size_bytes",
            "role",
            "launchable",
            "disc_index",
            "group_id",
        ],
    );
    assert_object_keys(
        &succeeded["result"]["import"],
        &[
            "input_file_count",
            "input_bytes",
            "extracted_file_count",
            "extracted_bytes",
            "launcher_count",
            "archive_bytes",
            "extractor_version",
            "data_version",
        ],
    );

    let failed = &fixture["failed_response"];
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["phase"], "extract_installer_payload");
    assert!(failed["progress"].is_null());
    assert!(failed["result"].is_null());
    assert_object_keys(&failed["error"], &["code", "message"]);
    assert_eq!(failed["error"]["code"], "unprocessable_entity");

    fn assert_object_keys(value: &serde_json::Value, expected: &[&str]) {
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let expected = expected.iter().copied().collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}

#[cfg(unix)]
mod unix_tests {
    use std::{
        fs::File,
        io::{Read, Write},
        os::unix::fs::PermissionsExt,
        sync::{Arc, Mutex},
        time::Duration,
    };

    use axum::http::{StatusCode, header};
    use sha2::{Digest, Sha256};
    use tower::ServiceExt;
    use tracing::instrument::WithSubscriber;
    use zip::ZipArchive;

    use super::support;

    // Keep the scoped log-capture subscriber isolated from the other importer requests.
    static GOG_IMPORT_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    const SUCCESS_PHASE_EVENTS: &[(&str, &str)] = &[
        ("validate_request", "Validating request"),
        ("resolve_windows_target", "Resolving Windows target"),
        ("verify_innoextract_hash", "Verifying innoextract"),
        ("prepare_private_workspace", "Preparing private workspace"),
        ("stage_setup_files", "Staging setup files"),
        ("probe_innoextract_version", "Checking innoextract version"),
        (
            "probe_installer_data_version",
            "Checking installer data version",
        ),
        ("test_installer_integrity", "Testing installer integrity"),
        ("extract_installer_payload", "Extracting installer payload"),
        ("validate_extracted_tree", "Validating extracted tree"),
        ("create_windows_zip", "Creating Windows ZIP"),
        ("finalize_windows_zip", "Finalizing Windows ZIP"),
        ("validate_sipario_zip", "Validating Windows ZIP"),
        ("stage_generated_archive", "Staging generated archive"),
        ("publish_windows_archive", "Publishing Windows archive"),
        (
            "cleanup_uploaded_setup_staging",
            "Cleaning uploaded setup staging",
        ),
        ("complete", "GOG setup import complete"),
    ];

    #[tokio::test]
    async fn admin_can_create_poll_and_complete_a_gog_import_job() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let status = app
            .router
            .clone()
            .oneshot(support::empty_request(
                "GET",
                "/api/admin/gog-import/status",
                auth,
            ))
            .await
            .unwrap();
        assert_eq!(status.status(), StatusCode::OK);
        let status = support::response_json(status).await;
        assert_eq!(status["enabled"], true);
        assert_eq!(status["configured"], true);
        assert_eq!(status["bundled_extractor"], false);
        assert!(status.get("experimental").is_none());

        let captured_logs = Arc::new(Mutex::new(Vec::new()));
        let log_writer = {
            let captured_logs = Arc::clone(&captured_logs);
            move || CapturedLogWriter(Arc::clone(&captured_logs))
        };
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_writer(log_writer)
            .finish();
        let (location, create, body) = async {
            let mut request = support::multipart_request(
                "POST",
                "/api/admin/gog-imports",
                auth,
                &[("title", "Test Game")],
                &[
                    ("file", "setup_test_game.exe", b"synthetic setup"),
                    ("file", "setup_test_game-1.bin", b"synthetic sidecar"),
                ],
            );
            request
                .headers_mut()
                .insert("x-teatro-transfer-id", "gog_replay".parse().unwrap());
            let response = app.router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::ACCEPTED);
            let location = response
                .headers()
                .get(header::LOCATION)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            let create = support::response_json(response).await;
            let mut retry = support::empty_request("POST", "/api/admin/gog-imports", auth);
            retry
                .headers_mut()
                .insert("x-teatro-transfer-id", "gog_replay".parse().unwrap());
            let replay = app.router.clone().oneshot(retry).await.unwrap();
            assert_eq!(replay.status(), StatusCode::ACCEPTED);
            assert_eq!(replay.headers()[header::LOCATION], location);
            assert_eq!(support::response_json(replay).await, create);
            let body = wait_for_terminal_job(&app, &location, auth).await;
            assert_eq!(rom_count(&app).await, 1);
            (location, create, body)
        }
        .with_subscriber(subscriber)
        .await;

        assert_object_keys(&create, &["job", "status_url"]);
        assert_object_keys(
            &create["job"],
            &[
                "id",
                "state",
                "phase",
                "progress",
                "output_truncated",
                "last_event_seq",
                "created_at",
                "updated_at",
            ],
        );
        assert_eq!(create["status_url"], location);
        assert_eq!(create["job"]["state"], "queued");
        assert_eq!(create["job"]["phase"], "queued");
        assert!(create["job"]["progress"].is_null());
        assert_eq!(create["job"]["output_truncated"], false);
        assert_eq!(create["job"]["last_event_seq"], 0);
        assert!(create["job"]["id"].as_str().unwrap().starts_with("gog_"));

        assert_object_keys(
            &body,
            &[
                "id",
                "state",
                "phase",
                "progress",
                "events",
                "next_event_seq",
                "output_truncated",
                "result",
                "error",
                "created_at",
                "updated_at",
            ],
        );
        assert_eq!(body["state"], "succeeded");
        assert_eq!(body["phase"], "complete");
        assert!(body["progress"].is_null());
        assert!(body["error"].is_null());
        assert_eq!(body["output_truncated"], false);
        assert_phase_events(&body, SUCCESS_PHASE_EVENTS);
        let output = output_events(&body);
        assert!(output.iter().any(|event| {
            event["stream"] == "stdout"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text == "<script>private transcript at [redacted]</script>")
        }));

        let result = &body["result"];
        assert_eq!(result["rom"]["name"], "Test Game");
        assert_eq!(result["rom"]["platform_slug"], "win");
        assert_eq!(result["file_name"], "Test Game.zip");
        assert_eq!(result["relative_path"], "win/Test Game.zip");
        assert_eq!(result["import"]["input_file_count"], 2);
        assert_eq!(result["import"]["extracted_file_count"], 2);
        assert_eq!(result["import"]["launcher_count"], 1);
        assert!(
            result["import"]["archive_bytes"].as_u64().unwrap()
                > app.state.config().max_upload_bytes,
            "server-generated ZIPs must not inherit the browser upload size limit"
        );
        assert_eq!(
            result["import"]["extractor_version"],
            "innoextract 1.10-dev fake"
        );
        assert_eq!(result["import"]["data_version"], "6.3.3");

        let last_sequence = body["next_event_seq"].as_u64().unwrap();
        let after = get_job(&app, &format!("{location}?after={last_sequence}"), auth).await;
        assert_eq!(after.status(), StatusCode::OK);
        let after = support::response_json(after).await;
        assert_eq!(after["events"], serde_json::json!([]));
        assert_eq!(after["next_event_seq"], last_sequence);
        for query in [
            "after=wat".to_string(),
            format!("after={last_sequence}&after={last_sequence}"),
            format!("after={}", last_sequence + 1),
        ] {
            let response = get_job(&app, &format!("{location}?{query}"), auth).await;
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
            let error = support::response_json(response).await;
            assert_eq!(error["error"]["code"], "bad_request");
        }

        let archive_path = app.temp_dir.path().join("roms/win/Test Game.zip");
        let mut archive = ZipArchive::new(File::open(archive_path).unwrap()).unwrap();
        let mut payload = String::new();
        archive
            .by_name("app/data/game.dat")
            .unwrap()
            .read_to_string(&mut payload)
            .unwrap();
        assert_eq!(payload, "game payload");
        assert!(archive.by_name("app/Test Game.exe").is_ok());

        let audit_rows: Vec<String> = sqlx::query_scalar(
            "SELECT metadata_json FROM audit_log WHERE action = 'roms.gog_setup_imported'",
        )
        .fetch_all(app.state.db())
        .await
        .unwrap();
        assert_eq!(audit_rows.len(), 1);
        let audit: serde_json::Value = serde_json::from_str(&audit_rows[0]).unwrap();
        assert_object_keys(
            &audit,
            &[
                "platform_slug",
                "file_id",
                "input_file_count",
                "input_bytes",
                "extracted_file_count",
                "extracted_bytes",
                "launcher_count",
                "archive_bytes",
                "extractor_version",
                "data_version",
            ],
        );
        assert!(audit.get("rom_name").is_none());
        assert!(audit.get("file_name").is_none());
        assert_staging_is_empty(&app);

        let logs = String::from_utf8(captured_logs.lock().unwrap().clone()).unwrap();
        for phase in [
            "upload_setup_files",
            "validate_request",
            "resolve_windows_target",
            "verify_innoextract_hash",
            "prepare_private_workspace",
            "stage_setup_files",
            "probe_innoextract_version",
            "probe_installer_data_version",
            "test_installer_integrity",
            "extract_installer_payload",
            "validate_extracted_tree",
            "create_windows_zip",
            "finalize_windows_zip",
            "validate_sipario_zip",
            "stage_generated_archive",
            "publish_windows_archive",
            "cleanup_uploaded_setup_staging",
        ] {
            assert!(
                logs.contains(phase),
                "missing GOG import phase {phase}; captured logs:\n{logs}"
            );
        }
        assert!(logs.contains("background job admission begins"));
        assert!(logs.contains("Imported GOG ZIP as Windows"));
        assert!(!logs.contains("Windows (x86)"));
        assert!(
            logs.lines().count() < 100,
            "progress output must not flood tracing"
        );
        assert!(!logs.contains("private transcript"));
        assert!(!logs.contains("setup_test_game"));
        assert!(!logs.contains("Test Game"));
        assert!(!logs.contains(&app.temp_dir.path().display().to_string()));
    }

    struct CapturedLogWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for CapturedLogWriter {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn failed_workflows_become_terminal_job_resources() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        for (mode, wrong_hash, expected_code, expected_phase, expected_truncation) in [
            (
                "success",
                true,
                "service_unavailable",
                "verify_innoextract_hash",
                false,
            ),
            (
                "warning",
                false,
                "unprocessable_entity",
                "test_installer_integrity",
                false,
            ),
            (
                "nonzero-exit",
                false,
                "unprocessable_entity",
                "test_installer_integrity",
                false,
            ),
            (
                "timeout",
                false,
                "gateway_timeout",
                "test_installer_integrity",
                false,
            ),
            (
                "no-launcher",
                false,
                "unprocessable_entity",
                "validate_extracted_tree",
                false,
            ),
            (
                "high-output-warning",
                false,
                "unprocessable_entity",
                "test_installer_integrity",
                true,
            ),
        ] {
            let app = test_app_with_extractor(mode, wrong_hash).await;
            let admin = app.seed_admin("admin", "password").await;
            let auth = Some((admin.username.as_str(), admin.password.as_str()));
            let (location, _) = create_job(&app, import_request(auth)).await;
            let body = wait_for_terminal_job(&app, &location, auth).await;

            assert_eq!(body["state"], "failed", "mode {mode}");
            assert_eq!(body["phase"], expected_phase, "mode {mode}");
            assert!(body["progress"].is_null(), "mode {mode}");
            assert!(body["result"].is_null(), "mode {mode}");
            assert_eq!(body["error"]["code"], expected_code, "mode {mode}");
            assert_eq!(body["output_truncated"], expected_truncation, "mode {mode}");
            let events = body["events"].as_array().unwrap();
            assert!(events.len() <= 512, "mode {mode}");
            assert!(
                events
                    .iter()
                    .map(|event| event["text"].as_str().unwrap().len())
                    .sum::<usize>()
                    <= 256 * 1024,
                "mode {mode}"
            );
            let last_phase = SUCCESS_PHASE_EVENTS
                .iter()
                .position(|(phase, _)| *phase == expected_phase)
                .unwrap();
            assert_phase_events(&body, &SUCCESS_PHASE_EVENTS[..=last_phase]);
            if mode == "warning" {
                assert!(
                    body["error"]["message"]
                        .as_str()
                        .unwrap()
                        .contains("warnings")
                );
                let output = output_events(&body);
                assert!(output.iter().any(|event| {
                    event["stream"] == "stderr"
                        && event["text"].as_str().is_some_and(|text| {
                            text.contains("Warning") && text.contains("[redacted]")
                        })
                }));
                assert!(!output.iter().any(|event| {
                    event["text"].as_str().is_some_and(|text| {
                        text.contains(&app.temp_dir.path().display().to_string())
                    })
                }));
            }
            if mode == "timeout" {
                tokio::time::sleep(Duration::from_millis(1_500)).await;
                assert!(
                    !app.temp_dir
                        .path()
                        .join("gog-timeout-descendant-survived")
                        .exists(),
                    "timeout must terminate the extractor's whole process group"
                );
            }
            assert_eq!(rom_count(&app).await, 0, "mode {mode}");
            assert_staging_is_empty(&app);
        }
    }

    #[tokio::test]
    async fn job_status_requires_admin_for_basic_and_bearer_without_leaking_job_data() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let reader = app.seed_readonly("reader", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));
        let (location, create) = create_job(&app, import_request(auth)).await;
        let terminal = wait_for_terminal_job(&app, &location, auth).await;
        assert_eq!(terminal["state"], "succeeded");
        let job_id = create["job"]["id"].as_str().unwrap();

        let read_token = create_api_token(&app, auth, "Read GOG status", "read").await;
        let admin_token = create_api_token(&app, auth, "Admin GOG status", "admin").await;

        let missing = get_job(&app, &location, None).await;
        assert_job_data_is_hidden(missing, StatusCode::UNAUTHORIZED, job_id).await;
        let invalid = get_job(&app, &location, Some(("admin", "wrong"))).await;
        assert_job_data_is_hidden(invalid, StatusCode::UNAUTHORIZED, job_id).await;
        let invalid_bearer = get_job_with_bearer(&app, &location, "teatro_pat_invalid").await;
        assert_job_data_is_hidden(invalid_bearer, StatusCode::UNAUTHORIZED, job_id).await;
        let readonly = get_job(
            &app,
            &location,
            Some((reader.username.as_str(), reader.password.as_str())),
        )
        .await;
        assert_job_data_is_hidden(readonly, StatusCode::FORBIDDEN, job_id).await;
        let read_bearer = get_job_with_bearer(&app, &location, &read_token).await;
        assert_job_data_is_hidden(read_bearer, StatusCode::FORBIDDEN, job_id).await;

        let basic_admin = get_job(&app, &location, auth).await;
        assert_eq!(basic_admin.status(), StatusCode::OK);
        assert_eq!(support::response_json(basic_admin).await["id"], job_id);
        let bearer_admin = get_job_with_bearer(&app, &location, &admin_token).await;
        assert_eq!(bearer_admin.status(), StatusCode::OK);
        assert_eq!(support::response_json(bearer_admin).await["id"], job_id);

        let unknown = get_job(&app, "/api/admin/gog-imports/gog_unknown", auth).await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let malformed = get_job(&app, "/api/admin/gog-imports/not-a-job-id", auth).await;
        assert_eq!(malformed.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn detached_jobs_keep_the_full_import_concurrency_permit() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("gated", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let first = app
            .router
            .clone()
            .oneshot(import_request(auth))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        let first_location = first
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        drop(first);

        let gate_started = app.temp_dir.path().join("gog-extractor-gate.started");
        wait_for_path(&gate_started).await;

        let running = wait_for_live_job(
            &app,
            &first_location,
            auth,
            "test_installer_integrity",
            Some(42.5),
            "42.5%",
            Some("synthetic stderr note"),
        )
        .await;
        assert_eq!(running["state"], "running");
        assert_eq!(running["phase"], "test_installer_integrity");
        assert_percent_progress(&running, 42.5);
        assert_live_extractor_output(
            &running,
            "test_installer_integrity",
            "42.5%",
            "synthetic stderr note",
        );
        let integrity_phase = SUCCESS_PHASE_EVENTS
            .iter()
            .position(|(phase, _)| *phase == "test_installer_integrity")
            .unwrap();
        assert_phase_events(&running, &SUCCESS_PHASE_EVENTS[..=integrity_phase]);

        let second_request = import_request(auth);
        let second_router = app.router.clone();
        let second =
            tokio::spawn(async move { second_router.oneshot(second_request).await.unwrap() });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !second.is_finished(),
            "a second upload must wait while the first background job owns the permit"
        );

        std::fs::write(
            app.temp_dir.path().join("gog-extractor-gate.release"),
            b"release",
        )
        .unwrap();

        let extract_gate_started = app
            .temp_dir
            .path()
            .join("gog-extractor-extract-gate.started");
        wait_for_path(&extract_gate_started).await;
        let extracting = wait_for_live_job(
            &app,
            &first_location,
            auth,
            "extract_installer_payload",
            Some(67.5),
            "67.5%",
            Some("synthetic extraction stderr note"),
        )
        .await;
        assert_eq!(extracting["state"], "running");
        assert_eq!(extracting["phase"], "extract_installer_payload");
        assert_percent_progress(&extracting, 67.5);
        assert_live_extractor_output(
            &extracting,
            "extract_installer_payload",
            "67.5%",
            "synthetic extraction stderr note",
        );
        assert!(
            !second.is_finished(),
            "the first job must retain the permit during extraction"
        );

        std::fs::write(
            app.temp_dir
                .path()
                .join("gog-extractor-extract-gate.release"),
            b"release",
        )
        .unwrap();
        let first_terminal = wait_for_terminal_job(&app, &first_location, auth).await;
        assert_eq!(first_terminal["state"], "succeeded");

        let second = second.await.unwrap();
        assert_eq!(second.status(), StatusCode::ACCEPTED);
        let second_location = second
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let second_terminal = wait_for_terminal_job(&app, &second_location, auth).await;
        assert_eq!(second_terminal["state"], "succeeded");
        assert_eq!(rom_count(&app).await, 2);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn configured_concurrency_allows_multiple_active_jobs() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("multi-gated", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let (first_location, _) = create_job(&app, import_request(auth)).await;
        let (second_location, _) = create_job(&app, import_request(auth)).await;
        let gate_started = app.temp_dir.path().join("gog-extractor-gate.started");
        wait_for_line_count(&gate_started, 2).await;

        for location in [&first_location, &second_location] {
            let running = get_job(&app, location, auth).await;
            assert_eq!(running.status(), StatusCode::OK);
            let running = support::response_json(running).await;
            assert_eq!(running["state"], "running");
            assert_eq!(running["phase"], "test_installer_integrity");
        }

        std::fs::write(
            app.temp_dir.path().join("gog-extractor-gate.release"),
            b"release",
        )
        .unwrap();
        let extraction_started = app
            .temp_dir
            .path()
            .join("gog-extractor-extract-gate.started");
        wait_for_line_count(&extraction_started, 2).await;
        std::fs::write(
            app.temp_dir
                .path()
                .join("gog-extractor-extract-gate.release"),
            b"release",
        )
        .unwrap();

        let first = wait_for_terminal_job(&app, &first_location, auth).await;
        let second = wait_for_terminal_job(&app, &second_location, auth).await;
        assert_eq!(first["state"], "succeeded");
        assert_eq!(second["state"], "succeeded");
        assert_eq!(rom_count(&app).await, 2);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn malformed_progress_stays_indeterminate_without_failing_the_import() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("malformed-gated", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));
        let (location, _) = create_job(&app, import_request(auth)).await;

        let gate_started = app.temp_dir.path().join("gog-extractor-gate.started");
        wait_for_path(&gate_started).await;
        let running = wait_for_live_job(
            &app,
            &location,
            auth,
            "test_installer_integrity",
            None,
            "NaN% malformed progress",
            None,
        )
        .await;
        assert_eq!(running["state"], "running");
        assert_eq!(running["phase"], "test_installer_integrity");
        assert!(running["progress"].is_null());
        assert!(output_events(&running).iter().any(|event| {
            event["stream"] == "stdout"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("NaN% malformed progress"))
        }));

        std::fs::write(
            app.temp_dir.path().join("gog-extractor-gate.release"),
            b"release",
        )
        .unwrap();
        let terminal = wait_for_terminal_job(&app, &location, auth).await;
        assert_eq!(terminal["state"], "succeeded");
        assert!(terminal["progress"].is_null());
        assert_eq!(terminal["output_truncated"], false);
        assert_eq!(rom_count(&app).await, 1);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn zip_creation_exposes_source_byte_progress() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("archive-progress", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));
        let (location, _) = create_job(&app, import_request(auth)).await;

        let running = wait_for_byte_progress(&app, &location, auth, "create_windows_zip").await;
        assert_eq!(running["state"], "running");
        assert_eq!(running["progress"]["kind"], "bytes");
        let current = running["progress"]["current"].as_u64().unwrap();
        let total = running["progress"]["total"].as_u64().unwrap();
        let percent = running["progress"]["percent"].as_f64().unwrap();
        assert!(current > 0 && current <= total);
        assert!(percent.is_finite() && percent > 0.0 && percent <= 100.0);

        let health = app
            .router
            .clone()
            .oneshot(support::empty_request("GET", "/healthz", None))
            .await
            .unwrap();
        assert_eq!(
            health.status(),
            StatusCode::OK,
            "Tokio must remain responsive while ZIP work runs in spawn_blocking"
        );

        let terminal = wait_for_terminal_job(&app, &location, auth).await;
        assert_eq!(terminal["state"], "succeeded");
        assert_eq!(terminal["result"]["import"]["extracted_bytes"], total);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn importer_rejects_incomplete_or_ambiguous_setup_selections() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let response = app
            .router
            .clone()
            .oneshot(support::multipart_request(
                "POST",
                "/api/admin/gog-imports",
                auth,
                &[("title", "Ambiguous Game")],
                &[("file", "setup.exe", b"one"), ("file", "patch.exe", b"two")],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(rom_count(&app).await, 0);
        assert_staging_is_empty(&app);

        let response = app
            .router
            .clone()
            .oneshot(support::multipart_request(
                "POST",
                "/api/admin/gog-imports",
                auth,
                &[],
                &[("file", "setup.exe", b"one")],
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn truncated_multipart_upload_is_rejected_and_cleans_staging_before_admission() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let response = app
            .router
            .clone()
            .oneshot(truncated_import_request(auth))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error = support::response_json(response).await;
        assert_eq!(error["error"]["code"], "bad_request");
        assert_eq!(rom_count(&app).await, 0);
        assert_staging_is_empty(&app);
    }

    #[tokio::test]
    async fn publication_database_failure_leaves_no_rom_archive_or_pending_staging() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));
        sqlx::query(
            "CREATE TRIGGER fail_gog_rom_insert BEFORE INSERT ON roms BEGIN SELECT RAISE(ABORT, 'synthetic GOG finalizer failure'); END",
        )
        .execute(app.state.db())
        .await
        .unwrap();

        let (location, _) = create_job(&app, import_request(auth)).await;
        let terminal = wait_for_terminal_job(&app, &location, auth).await;
        assert_eq!(terminal["state"], "failed");
        assert_eq!(terminal["phase"], "publish_windows_archive");
        assert_eq!(terminal["error"]["code"], "internal_server_error");
        assert_eq!(rom_count(&app).await, 0);
        assert!(!app.temp_dir.path().join("roms/win/Test Game.zip").exists());
        let successful_audits: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'roms.gog_setup_imported'",
        )
        .fetch_one(app.state.db())
        .await
        .unwrap();
        assert_eq!(successful_audits, 0);
        let unfinished_operations: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM file_operations WHERE state != 'completed'")
                .fetch_one(app.state.db())
                .await
                .unwrap();
        assert_eq!(
            unfinished_operations, 1,
            "the failed finalizer journal remains explicit until startup recovery"
        );

        let config = app.state.config().clone();
        let support::TestApp {
            temp_dir,
            state,
            router,
        } = app;
        drop(router);
        state.db().close().await;
        drop(state);
        let temp_path = temp_dir.path().to_path_buf();

        let recovered_state = teatro::state::AppState::initialize(config).await.unwrap();
        let unfinished_after_recovery: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM file_operations WHERE state != 'completed'")
                .fetch_one(recovered_state.db())
                .await
                .unwrap();
        assert_eq!(unfinished_after_recovery, 0);
        let roms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
            .fetch_one(recovered_state.db())
            .await
            .unwrap();
        assert_eq!(roms, 0);
        assert!(!temp_path.join("roms/win/Test Game.zip").exists());
        assert_staging_paths_are_empty(&temp_path);
    }

    #[tokio::test]
    async fn a_fresh_process_state_loses_terminal_jobs_but_preserves_published_roms() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = test_app_with_extractor("success", false).await;
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));
        let (location, _) = create_job(&app, import_request(auth)).await;
        let terminal = wait_for_terminal_job(&app, &location, auth).await;
        assert_eq!(terminal["state"], "succeeded");

        let config = app.state.config().clone();
        let support::TestApp {
            temp_dir,
            state,
            router,
        } = app;
        drop(router);
        state.db().close().await;
        drop(state);
        let temp_path = temp_dir.path().to_path_buf();

        let restarted_state = teatro::state::AppState::initialize(config).await.unwrap();
        let restarted_router = teatro::api::router(restarted_state.clone());
        let response = restarted_router
            .oneshot(support::empty_request("GET", &location, auth))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let persisted_roms: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
            .fetch_one(restarted_state.db())
            .await
            .unwrap();
        assert_eq!(persisted_roms, 1);
        assert!(temp_path.join("roms/win/Test Game.zip").exists());
        assert_staging_paths_are_empty(&temp_path);
    }

    #[tokio::test]
    async fn importer_is_disabled_by_default() {
        let _test_guard = GOG_IMPORT_TEST_LOCK.lock().await;
        let app = support::TestApp::with_config(|config| {
            let abandoned = config
                .data_dir
                .join("gog-import-staging/import-abandoned/private.exe");
            std::fs::create_dir_all(abandoned.parent().unwrap()).unwrap();
            std::fs::write(abandoned, b"private staging data").unwrap();
        })
        .await;
        assert_staging_is_empty(&app);
        let admin = app.seed_admin("admin", "password").await;
        let auth = Some((admin.username.as_str(), admin.password.as_str()));

        let status = app
            .router
            .clone()
            .oneshot(support::empty_request(
                "GET",
                "/api/admin/gog-import/status",
                auth,
            ))
            .await
            .unwrap();
        let status = support::response_json(status).await;
        assert_eq!(status["enabled"], false);
        assert_eq!(status["configured"], false);
        assert_eq!(status["bundled_extractor"], false);

        let response = app
            .router
            .clone()
            .oneshot(import_request(auth))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(rom_count(&app).await, 0);
    }

    fn import_request(auth: Option<(&str, &str)>) -> axum::http::Request<axum::body::Body> {
        support::multipart_request(
            "POST",
            "/api/admin/gog-imports",
            auth,
            &[("title", "Test Game")],
            &[("file", "setup_test_game.exe", b"synthetic setup")],
        )
    }

    fn truncated_import_request(
        auth: Option<(&str, &str)>,
    ) -> axum::http::Request<axum::body::Body> {
        let boundary = "teatro-truncated-gog-boundary";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"title\"\r\n\r\nDisconnected Game\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"setup.exe\"\r\nContent-Type: application/octet-stream\r\n\r\ncomplete first part\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"setup-1.bin\"\r\nContent-Type: application/octet-stream\r\n\r\ntruncated body"
        );
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/admin/gog-imports")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            );
        if let Some((username, password)) = auth {
            request = request.header(
                header::AUTHORIZATION,
                support::basic_auth(username, password),
            );
        }
        request.body(axum::body::Body::from(body)).unwrap()
    }

    async fn test_app_with_extractor(mode: &str, wrong_hash: bool) -> support::TestApp {
        let mode = mode.to_string();
        support::TestApp::with_config(move |config| {
            let root = config.data_dir.parent().unwrap();
            let extractor = root.join(format!("fake-innoextract-{mode}.sh"));
            let gate_started = root.join("gog-extractor-gate.started");
            let gate_release = root.join("gog-extractor-gate.release");
            let extract_gate_started = root.join("gog-extractor-extract-gate.started");
            let extract_gate_release = root.join("gog-extractor-extract-gate.release");
            let timeout_descendant_marker = root.join("gog-timeout-descendant-survived");
            let script = fake_extractor_script(
                &mode,
                &gate_started,
                &gate_release,
                &extract_gate_started,
                &extract_gate_release,
                &timeout_descendant_marker,
            );
            std::fs::write(&extractor, script.as_bytes()).unwrap();
            let mut permissions = std::fs::metadata(&extractor).unwrap().permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&extractor, permissions).unwrap();
            let digest = format!("{:x}", Sha256::digest(script.as_bytes()));

            // Each uploaded setup part fits, while the generated ZIP is larger. This
            // proves the browser upload limit is not reused for managed publication.
            config.max_upload_bytes = 64;
            config.gog_import.enabled = true;
            config.gog_import.innoextract_path = Some(extractor);
            config.gog_import.innoextract_sha256 =
                Some(if wrong_hash { "0".repeat(64) } else { digest });
            config.gog_import.timeout_seconds = if mode == "timeout" { 1 } else { 10 };
            config.gog_import.max_extracted_bytes = if mode == "archive-progress" {
                8 * 1024 * 1024
            } else {
                1024 * 1024
            };
            config.gog_import.max_extracted_files = 100;
            config.uploads.free_space_margin_bytes = 0;
            if mode == "gated" {
                config.uploads.max_concurrent_uploads = 1;
            } else if mode == "multi-gated" {
                config.uploads.max_concurrent_uploads = 2;
            }
        })
        .await
    }

    fn fake_extractor_script(
        mode: &str,
        gate_started: &std::path::Path,
        gate_release: &std::path::Path,
        extract_gate_started: &std::path::Path,
        extract_gate_release: &std::path::Path,
        timeout_descendant_marker: &std::path::Path,
    ) -> String {
        format!(
            r#"#!/bin/sh
set -eu
mode='{mode}'
gate_started='{gate_started}'
gate_release='{gate_release}'
extract_gate_started='{extract_gate_started}'
extract_gate_release='{extract_gate_release}'
timeout_descendant_marker='{timeout_descendant_marker}'
phase=''
output=''
progress=''
silent=''
color=''
setup=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      printf '%s\n' 'innoextract 1.10-dev fake'
      exit 0
      ;;
    --data-version)
      printf '%s\n' '6.3.3'
      exit 0
      ;;
    --test)
      phase='test'
      ;;
    --extract)
      phase='extract'
      ;;
    --progress=true)
      progress='true'
      ;;
    --progress=false)
      exit 2
      ;;
    --silent)
      silent='true'
      ;;
    --color=false)
      color='false'
      ;;
    --output-dir)
      shift
      output="$1"
      ;;
    --)
      ;;
    *)
      setup="$1"
      ;;
  esac
  shift
done
if [ "$phase" = 'test' ]; then
  [ "$progress" = 'true' ] || exit 2
  [ "$silent" = 'true' ] || exit 2
  [ "$color" = 'false' ] || exit 2
  if [ "$mode" = 'warning' ]; then
    printf 'War' >&2
    printf 'ning: synthetic compatibility warning at %s/setup-1.bin\n' "${{setup%/*}}" >&2
  fi
  if [ "$mode" = 'high-output-warning' ]; then
    i=0
    while [ "$i" -lt 300 ]; do
      printf '%01024d\r' "$i"
      i=$((i + 1))
    done
    printf 'War' >&2
    printf '%s\n' 'ning: diagnostic after excessive output' >&2
  fi
  if [ "$mode" = 'success' ]; then
    printf '<script>private transcript at %s</script>\n' "$setup"
  fi
  if [ "$mode" = 'nonzero-exit' ]; then
    printf '%s\n' 'synthetic extractor rejection' >&2
    exit 2
  fi
  if [ "$mode" = 'timeout' ]; then
    ( /bin/sleep 2; printf '%s' 'survived' > "$timeout_descendant_marker" ) &
    wait
  fi
  if [ "$mode" = 'gated' ] || [ "$mode" = 'multi-gated' ]; then
    printf '\r\033['
    printf 'K[====>] 42.5%% 38.1 MiB/s\r'
    printf '%s\n' 'synthetic stderr note 99.9%' >&2
    printf '%s\n' "$$" >> "$gate_started"
    while [ ! -f "$gate_release" ]; do
      /bin/sleep 0.02
    done
    printf '\r\033[K100.0%% 42.3 MiB/s\r'
  fi
  if [ "$mode" = 'malformed-gated' ]; then
    printf '\r\033[K NaN%% malformed progress\r'
    : > "$gate_started"
    while [ ! -f "$gate_release" ]; do
      /bin/sleep 0.02
    done
  fi
  exit 0
fi
if [ "$phase" = 'extract' ]; then
  [ "$progress" = 'true' ] || exit 2
  [ "$silent" = 'true' ] || exit 2
  [ "$color" = 'false' ] || exit 2
  if [ "$mode" = 'gated' ] || [ "$mode" = 'multi-gated' ]; then
    printf '\r\033[K 67.5%% 40.0 MiB/s\r'
    printf '%s\n' 'synthetic extraction stderr note 99.9%' >&2
    printf '%s\n' "$$" >> "$extract_gate_started"
    while [ ! -f "$extract_gate_release" ]; do
      /bin/sleep 0.02
    done
    printf '\r\033[K100.0%% 41.0 MiB/s\r'
  fi
  if [ "$mode" = 'malformed-gated' ]; then
    printf '\r\033[K invalid%% progress\r'
  fi
  /bin/mkdir -p "$output/app/data"
  if [ "$mode" != 'no-launcher' ]; then
    printf '%s' 'MZ synthetic launcher' > "$output/app/Test Game.exe"
  fi
  printf '%s' 'game payload' > "$output/app/data/game.dat"
  if [ "$mode" = 'archive-progress' ]; then
    /bin/dd if=/dev/urandom of="$output/app/data/progress.dat" bs=1048576 count=4 2>/dev/null
  fi
  exit 0
fi
exit 2
"#,
            gate_started = gate_started.display(),
            gate_release = gate_release.display(),
            extract_gate_started = extract_gate_started.display(),
            extract_gate_release = extract_gate_release.display(),
            timeout_descendant_marker = timeout_descendant_marker.display(),
        )
    }

    async fn create_job(
        app: &support::TestApp,
        request: axum::http::Request<axum::body::Body>,
    ) -> (String, serde_json::Value) {
        let response = app.router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let body = support::response_json(response).await;
        assert_eq!(body["status_url"], location);
        (location, body)
    }

    async fn get_job(
        app: &support::TestApp,
        uri: &str,
        auth: Option<(&str, &str)>,
    ) -> axum::response::Response {
        app.router
            .clone()
            .oneshot(support::empty_request("GET", uri, auth))
            .await
            .unwrap()
    }

    async fn get_job_with_bearer(
        app: &support::TestApp,
        uri: &str,
        token: &str,
    ) -> axum::response::Response {
        let mut request = support::empty_request("GET", uri, None);
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        app.router.clone().oneshot(request).await.unwrap()
    }

    async fn create_api_token(
        app: &support::TestApp,
        auth: Option<(&str, &str)>,
        name: &str,
        scope: &str,
    ) -> String {
        let response = app
            .router
            .clone()
            .oneshot(support::json_request(
                "POST",
                "/api/admin/tokens",
                auth,
                serde_json::json!({ "name": name, "scopes": [scope] }),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        support::response_json(response).await["token"]
            .as_str()
            .unwrap()
            .to_string()
    }

    async fn assert_job_data_is_hidden(
        response: axum::response::Response,
        expected_status: StatusCode,
        job_id: &str,
    ) {
        assert_eq!(response.status(), expected_status);
        let body = support::response_text(response).await;
        assert!(!body.contains(job_id));
        assert!(!body.contains("Test Game"));
        assert!(!body.contains("private transcript"));
    }

    async fn wait_for_terminal_job(
        app: &support::TestApp,
        location: &str,
        auth: Option<(&str, &str)>,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let response = get_job(app, location, auth).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = support::response_json(response).await;
            if matches!(body["state"].as_str(), Some("succeeded" | "failed")) {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "GOG import job did not become terminal: {body}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn wait_for_live_job(
        app: &support::TestApp,
        location: &str,
        auth: Option<(&str, &str)>,
        phase: &str,
        percent: Option<f64>,
        stdout_fragment: &str,
        stderr_fragment: Option<&str>,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let response = get_job(app, location, auth).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = support::response_json(response).await;
            let progress_matches = percent.map_or_else(
                || body["progress"].is_null(),
                |expected| body["progress"]["percent"].as_f64() == Some(expected),
            );
            let output = output_events(&body);
            let stdout_matches = output.iter().any(|event| {
                event["stream"] == "stdout"
                    && event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(stdout_fragment))
            });
            let stderr_matches = stderr_fragment.is_none_or(|fragment| {
                output.iter().any(|event| {
                    event["stream"] == "stderr"
                        && event["text"]
                            .as_str()
                            .is_some_and(|text| text.contains(fragment))
                })
            });
            if body["state"] == "running"
                && body["phase"] == phase
                && progress_matches
                && stdout_matches
                && stderr_matches
            {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "GOG import job did not expose live {phase} output: {body}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_byte_progress(
        app: &support::TestApp,
        location: &str,
        auth: Option<(&str, &str)>,
        phase: &str,
    ) -> serde_json::Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let response = get_job(app, location, auth).await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = support::response_json(response).await;
            if body["state"] == "running"
                && body["phase"] == phase
                && body["progress"]["kind"] == "bytes"
                && body["progress"]["current"]
                    .as_u64()
                    .is_some_and(|value| value > 0)
            {
                return body;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "GOG import job did not expose live {phase} byte progress: {body}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_path(path: &std::path::Path) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_for_line_count(path: &std::path::Path, expected: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let lines = std::fs::read_to_string(path)
                .map(|contents| contents.lines().count())
                .unwrap_or_default();
            if lines >= expected {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {expected} records in {}",
                path.display()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn rom_count(app: &support::TestApp) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM roms")
            .fetch_one(app.state.db())
            .await
            .unwrap()
    }

    fn assert_phase_events(body: &serde_json::Value, expected: &[(&str, &str)]) {
        let events = body["events"].as_array().unwrap();
        for event in events {
            assert_object_keys(event, &["seq", "kind", "phase", "stream", "text"]);
        }
        assert!(events.windows(2).all(|events| {
            events[0]["seq"].as_u64().unwrap() < events[1]["seq"].as_u64().unwrap()
        }));
        if let Some(last) = events.last() {
            assert_eq!(body["next_event_seq"], last["seq"]);
        }

        let phases = events
            .iter()
            .filter(|event| event["kind"] == "phase")
            .collect::<Vec<_>>();
        assert_eq!(phases.len(), expected.len());
        for (event, (phase, text)) in phases.into_iter().zip(expected) {
            assert_eq!(event["phase"], *phase);
            assert_eq!(event["stream"], serde_json::Value::Null);
            assert_eq!(event["text"], *text);
        }
    }

    fn output_events(body: &serde_json::Value) -> Vec<&serde_json::Value> {
        body["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "output")
            .collect()
    }

    fn assert_percent_progress(body: &serde_json::Value, expected: f64) {
        assert_object_keys(&body["progress"], &["kind", "current", "total", "percent"]);
        assert_eq!(body["progress"]["kind"], "percent");
        assert!(body["progress"]["current"].is_null());
        assert!(body["progress"]["total"].is_null());
        assert_eq!(body["progress"]["percent"], expected);
    }

    fn assert_live_extractor_output(
        body: &serde_json::Value,
        phase: &str,
        stdout_fragment: &str,
        stderr_fragment: &str,
    ) {
        let output = output_events(body);
        assert!(output.iter().any(|event| {
            event["phase"] == phase
                && event["stream"] == "stdout"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(stdout_fragment))
        }));
        assert!(output.iter().any(|event| {
            event["phase"] == phase
                && event["stream"] == "stderr"
                && event["text"]
                    .as_str()
                    .is_some_and(|text| text.contains(stderr_fragment))
        }));
        assert!(output.iter().all(|event| {
            event["text"].as_str().is_some_and(|text| {
                text.len() <= 4096
                    && !text.contains('\u{1b}')
                    && !text.chars().any(char::is_control)
            })
        }));
    }

    fn assert_object_keys(value: &serde_json::Value, expected: &[&str]) {
        let actual = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        let expected = expected
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }

    fn assert_staging_is_empty(app: &support::TestApp) {
        assert_staging_paths_are_empty(app.temp_dir.path());
    }

    fn assert_staging_paths_are_empty(root: &std::path::Path) {
        for relative in ["roms/.uploads", "data/gog-import-staging"] {
            let path = root.join(relative);
            if path.exists() {
                assert_eq!(
                    std::fs::read_dir(&path).unwrap().count(),
                    0,
                    "staging directory was not empty: {}",
                    path.display()
                );
            }
        }
    }
}
