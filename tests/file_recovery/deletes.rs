#[tokio::test]
async fn startup_reconciliation_restores_prepared_delete_artifacts() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (rom_id, root_id) = seed_rom_with_file(&state, "nes/restored.rom", None).await;
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    std::fs::create_dir_all(root_path.join(".trash/prepared-delete")).unwrap();
    std::fs::write(root_path.join(".trash/prepared-delete/0"), b"recoverable").unwrap();
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/restored.rom".to_string(),
            trash_relative_path: ".trash/prepared-delete/0".to_string(),
            directory: false,
        }],
    );
    file_operation_service::prepare(
        &state,
        "prepared-delete",
        FileOperationKind::Delete,
        &payload,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert_eq!(
        std::fs::read(root_path.join("nes/restored.rom")).unwrap(),
        b"recoverable"
    );
    assert!(!root_path.join(".trash/prepared-delete/0").exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'prepared-delete'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_restores_prepared_sidecar_cleanup() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    let trash = root_path.join(".trash/prepared-sidecars/0");
    std::fs::create_dir_all(trash.parent().unwrap()).unwrap();
    std::fs::write(&trash, b"sidecar").unwrap();
    let payload = DeleteOperationPayload::new(
        Vec::new(),
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/Notes.txt".to_string(),
            trash_relative_path: ".trash/prepared-sidecars/0".to_string(),
            directory: false,
        }],
    );
    file_operation_service::prepare(
        &state,
        "prepared-sidecars",
        FileOperationKind::Delete,
        &payload,
    )
    .await
    .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert_eq!(
        std::fs::read(root_path.join("nes/Notes.txt")).unwrap(),
        b"sidecar"
    );
    assert!(!trash.exists());
    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE id = 'prepared-sidecars'",
    )
    .fetch_one(reconciled.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_finishes_committed_sidecar_cleanup() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    let trash = root_path.join(".trash/committed-sidecars/0");
    std::fs::create_dir_all(trash.parent().unwrap()).unwrap();
    std::fs::write(&trash, b"sidecar").unwrap();
    let payload = DeleteOperationPayload::new(
        Vec::new(),
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/Notes.txt".to_string(),
            trash_relative_path: ".trash/committed-sidecars/0".to_string(),
            directory: false,
        }],
    );
    file_operation_service::prepare(
        &state,
        "committed-sidecars",
        FileOperationKind::Delete,
        &payload,
    )
    .await
    .unwrap();
    file_operations::set_state(
        state.db(),
        "committed-sidecars",
        FileOperationState::DbCommitted,
        None,
    )
    .await
    .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert!(!trash.exists());
    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE id = 'committed-sidecars'",
    )
    .fetch_one(reconciled.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_restores_a_prepared_package_directory() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (rom_id, root_id) = seed_rom_with_file(&state, "nes/package/game.rom", None).await;
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let trash = root_path.join(".trash/prepared-package/0");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("game.rom"), b"rom").unwrap();
    std::fs::write(trash.join(".Identifier"), b"sidecar").unwrap();
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/package".to_string(),
            trash_relative_path: ".trash/prepared-package/0".to_string(),
            directory: true,
        }],
    );
    file_operation_service::prepare(
        &state,
        "prepared-package",
        FileOperationKind::Delete,
        &payload,
    )
    .await
    .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert_eq!(
        std::fs::read(root_path.join("nes/package/game.rom")).unwrap(),
        b"rom"
    );
    assert_eq!(
        std::fs::read(root_path.join("nes/package/.Identifier")).unwrap(),
        b"sidecar"
    );
    assert!(!trash.exists());
    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE id = 'prepared-package'",
    )
    .fetch_one(reconciled.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_cleans_committed_delete_trash() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (rom_id, root_id) = seed_rom_with_file(&state, "nes/deleted.rom", None).await;
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    std::fs::create_dir_all(root_path.join(".trash/committed-delete")).unwrap();
    std::fs::write(root_path.join(".trash/committed-delete/0"), b"deleted").unwrap();
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/deleted.rom".to_string(),
            trash_relative_path: ".trash/committed-delete/0".to_string(),
            directory: false,
        }],
    );
    file_operation_service::prepare(
        &state,
        "committed-delete",
        FileOperationKind::Delete,
        &payload,
    )
        .await
        .unwrap();
    sqlx::query("DELETE FROM roms WHERE id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();
    file_operations::set_state(
        state.db(),
        "committed-delete",
        FileOperationState::CleanupPending,
        None,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert!(!root_path.join(".trash/committed-delete/0").exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'committed-delete'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn startup_reconciliation_cleans_a_committed_package_directory() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (rom_id, root_id) = seed_rom_with_file(&state, "nes/package/game.rom", None).await;
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let trash = root_path.join(".trash/committed-package/0");
    std::fs::create_dir_all(&trash).unwrap();
    std::fs::write(trash.join("game.rom"), b"rom").unwrap();
    std::fs::write(trash.join(".Identifier"), b"sidecar").unwrap();
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/package".to_string(),
            trash_relative_path: ".trash/committed-package/0".to_string(),
            directory: true,
        }],
    );
    file_operation_service::prepare(
        &state,
        "committed-package",
        FileOperationKind::Delete,
        &payload,
    )
    .await
    .unwrap();
    sqlx::query("DELETE FROM roms WHERE id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();
    file_operations::set_state(
        state.db(),
        "committed-package",
        FileOperationState::CleanupPending,
        None,
    )
    .await
    .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    assert!(!trash.exists());
    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE id = 'committed-package'",
    )
    .fetch_one(reconciled.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn missing_prepared_delete_artifacts_remain_failed_for_manual_repair() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let (rom_id, root_id) = seed_rom_with_file(&state, "nes/lost.rom", None).await;
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![DeleteOperationEntry {
            root_id,
            original_relative_path: "nes/lost.rom".to_string(),
            trash_relative_path: ".trash/interrupted-delete/0".to_string(),
            directory: false,
        }],
    );
    file_operation_service::prepare(
        &state,
        "interrupted-delete",
        FileOperationKind::Delete,
        &payload,
    )
        .await
        .unwrap();
    state.db().close().await;
    drop(state);

    let reconciled = AppState::initialize(config).await.unwrap();
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'interrupted-delete'")
            .fetch_one(reconciled.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "failed");
    let rom_exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roms WHERE id = ?)")
        .bind(rom_id)
        .fetch_one(reconciled.db())
        .await
        .unwrap();
    assert!(rom_exists);
}
