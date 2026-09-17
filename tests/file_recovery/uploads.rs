#[tokio::test]
async fn batch_upload_failure_after_one_move_removes_completed_targets() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let root = state.config().default_library_root.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".uploads")).unwrap();
    let first_staged = root.join(".uploads/first.part");
    let missing_second = root.join(".uploads/missing.part");
    std::fs::write(&first_staged, b"first").unwrap();

    let error = library::finalize_upload_batch(
        &state,
        library::UploadBatchDraft {
            platform_id: None,
            platform_slug: Some("genesis".to_string()),
            title: None,
            planned_titles: Vec::new(),
            files: vec![
                library::UploadBatchFileDraft {
                    original_file_name: "First.bin".to_string(),
                    staged_path: first_staged,
                    staging_operation_id: "first".to_string(),
                    file_size_bytes: 5,
                },
                library::UploadBatchFileDraft {
                    original_file_name: "Second.bin".to_string(),
                    staged_path: missing_second,
                    staging_operation_id: "second".to_string(),
                    file_size_bytes: 6,
                },
            ],
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("No such file") || error.to_string().contains("not found"));
    assert!(!root.join("genesis/First.bin").exists());
    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 0);
}

#[tokio::test]
async fn upload_database_failure_removes_the_moved_target() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let root = state.config().default_library_root.canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".uploads")).unwrap();
    let staged = root.join(".uploads/database-failure.part");
    std::fs::write(&staged, b"database failure").unwrap();
    sqlx::query(
        "CREATE TRIGGER fail_test_rom_insert BEFORE INSERT ON roms BEGIN SELECT RAISE(ABORT, 'injected ROM insert failure'); END",
    )
    .execute(state.db())
    .await
    .unwrap();

    let error = library::finalize_upload(
        &state,
        library::UploadDraft {
            platform_id: None,
            platform_slug: Some("genesis".to_string()),
            title: None,
            original_file_name: "Database Failure.bin".to_string(),
            staged_path: staged,
            staging_operation_id: "database-failure".to_string(),
            file_size_bytes: 16,
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("injected ROM insert failure"));
    assert!(!root.join("genesis/Database Failure.bin").exists());
    let rom_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(rom_count, 0);
}

#[tokio::test]
async fn startup_reconciliation_removes_uncommitted_upload_artifacts_idempotently() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(root_path.join("nes")).unwrap();
    std::fs::write(root_path.join("nes/interrupted.rom"), b"partial").unwrap();
    let payload = serde_json::to_string(&UploadOperationPayload::new(
        root.id,
        vec!["nes/interrupted.rom".to_string()],
    ))
    .unwrap();
    file_operations::create(
        state.db(),
        "interrupted-upload",
        FileOperationKind::Upload,
        &payload,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config.clone()).await.unwrap();
    assert!(!root_path.join("nes/interrupted.rom").exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'interrupted-upload'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
    reconciled.db().close().await;
    drop(reconciled);

    let second_restart = AppState::initialize(config).await.unwrap();
    assert!(!root_path.join("nes/interrupted.rom").exists());
    second_restart.db().close().await;
}

#[tokio::test]
async fn startup_reconciliation_removes_journaled_staging_files_immediately() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await
        .unwrap()
        .unwrap();
    std::fs::create_dir_all(root_path.join(".uploads")).unwrap();
    std::fs::write(root_path.join(".uploads/recover-now.part"), b"partial").unwrap();
    let payload = UploadOperationPayload::new(root.id, Vec::new())
        .with_staged_paths(vec![".uploads/recover-now.part".to_string()]);
    file_operation_service::prepare(
        &state,
        "staged-upload",
        FileOperationKind::Upload,
        &payload,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();

    assert!(!root_path.join(".uploads/recover-now.part").exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'staged-upload'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_completes_committed_uploads_without_removing_files() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (_, root_id) = seed_rom_with_file(&state, "nes/committed.rom", None).await;
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    std::fs::create_dir_all(root_path.join("nes")).unwrap();
    std::fs::write(root_path.join("nes/committed.rom"), b"committed").unwrap();
    let payload = serde_json::to_string(&UploadOperationPayload::new(
        root_id,
        vec!["nes/committed.rom".to_string()],
    ))
    .unwrap();
    file_operations::create(
        state.db(),
        "committed-upload",
        FileOperationKind::Upload,
        &payload,
    )
        .await
        .unwrap();
    file_operations::set_state(
        state.db(),
        "committed-upload",
        FileOperationState::DbCommitted,
        None,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert_eq!(
        std::fs::read(root_path.join("nes/committed.rom")).unwrap(),
        b"committed"
    );
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'committed-upload'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}
