#[tokio::test]
async fn concurrent_cover_reconciliation_restores_backups_only_once() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let (rom_id, _) = seed_rom_with_file(&state, "nes/game.rom", None).await;
    let asset_root = state.config().asset_root.canonicalize().unwrap();
    let large = "library/nes/game/cover-large.jpg";
    let small = "library/nes/game/cover-small.jpg";
    sqlx::query("UPDATE roms SET path_cover_large = ?, path_cover_small = ? WHERE id = ?")
        .bind(large)
        .bind(small)
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();

    for (path, contents) in [
        (large, b"new-large".as_slice()),
        (small, b"new-small".as_slice()),
    ] {
        let full_path = asset_root.join(path);
        std::fs::create_dir_all(full_path.parent().unwrap()).unwrap();
        std::fs::write(full_path, contents).unwrap();
    }
    let backup_large = ".trash/interrupted-cover/cover-0";
    let backup_small = ".trash/interrupted-cover/cover-1";
    std::fs::create_dir_all(asset_root.join(".trash/interrupted-cover")).unwrap();
    std::fs::write(asset_root.join(backup_large), b"old-large").unwrap();
    std::fs::write(asset_root.join(backup_small), b"old-small").unwrap();

    let payload = CoverOperationPayload::new(
        rom_id,
        vec![large.to_string(), small.to_string()],
        vec![
            CoverOperationEntry {
                resource_path: large.to_string(),
                backup_relative_path: Some(backup_large.to_string()),
            },
            CoverOperationEntry {
                resource_path: small.to_string(),
                backup_relative_path: Some(backup_small.to_string()),
            },
        ],
    );
    file_operation_service::prepare(
        &state,
        "interrupted-cover",
        FileOperationKind::CoverReplace,
        &payload,
    )
        .await
        .unwrap();

    // Ensure both reconcilers can snapshot the pending row before either one mutates it.
    let blocker = state.file_store().lock_root(&asset_root).await.unwrap();
    let first_state = state.clone();
    let first = tokio::spawn(async move { file_operation_service::reconcile(&first_state).await });
    let second_state = state.clone();
    let second =
        tokio::spawn(async move { file_operation_service::reconcile(&second_state).await });
    sleep(Duration::from_millis(100)).await;
    drop(blocker);

    timeout(Duration::from_secs(3), async {
        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();
    })
    .await
    .expect("cover reconcilers should not deadlock");

    assert_eq!(std::fs::read(asset_root.join(large)).unwrap(), b"old-large");
    assert_eq!(std::fs::read(asset_root.join(small)).unwrap(), b"old-small");
    assert!(!asset_root.join(backup_large).exists());
    assert!(!asset_root.join(backup_small).exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'interrupted-cover'")
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn reconciliation_discards_cover_artifacts_for_an_already_deleted_rom() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let (rom_id, _) = seed_rom_with_file(&state, "nes/deleted.rom", None).await;
    sqlx::query("DELETE FROM roms WHERE id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();

    let asset_root = state.config().asset_root.canonicalize().unwrap();
    let cover_path = "library/nes/deleted/cover-large.jpg";
    let backup_path = ".trash/deleted-cover/cover-0";
    std::fs::create_dir_all(asset_root.join(cover_path).parent().unwrap()).unwrap();
    std::fs::create_dir_all(asset_root.join(backup_path).parent().unwrap()).unwrap();
    std::fs::write(asset_root.join(cover_path), b"replacement").unwrap();
    std::fs::write(asset_root.join(backup_path), b"original").unwrap();
    let payload = CoverOperationPayload::new(
        rom_id,
        Vec::new(),
        vec![CoverOperationEntry {
            resource_path: cover_path.to_string(),
            backup_relative_path: Some(backup_path.to_string()),
        }],
    );
    file_operation_service::prepare(
        &state,
        "deleted-rom-cover",
        FileOperationKind::CoverReplace,
        &payload,
    )
        .await
        .unwrap();

    file_operation_service::reconcile(&state).await.unwrap();

    assert!(!asset_root.join(cover_path).exists());
    assert!(!asset_root.join(backup_path).exists());
    let operation_state: String =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = 'deleted-rom-cover'")
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn delete_reconciles_an_interrupted_cover_before_collecting_assets() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let root = state.config().default_library_root.canonicalize().unwrap();
    let cover_path = "library/nes/interrupted/cover-large.jpg";
    let backup_path = ".trash/interrupted-cover-delete/cover-0";
    let (rom_id, _) = seed_rom_with_file(&state, "nes/game.rom", Some(cover_path)).await;
    std::fs::create_dir_all(root.join("nes")).unwrap();
    std::fs::write(root.join("nes/game.rom"), b"rom").unwrap();

    let asset_root = state.config().asset_root.canonicalize().unwrap();
    std::fs::create_dir_all(asset_root.join(cover_path).parent().unwrap()).unwrap();
    std::fs::create_dir_all(asset_root.join(backup_path).parent().unwrap()).unwrap();
    std::fs::write(asset_root.join(cover_path), b"replacement").unwrap();
    std::fs::write(asset_root.join(backup_path), b"original").unwrap();
    let payload = CoverOperationPayload::new(
        rom_id,
        vec![cover_path.to_string()],
        vec![CoverOperationEntry {
            resource_path: cover_path.to_string(),
            backup_relative_path: Some(backup_path.to_string()),
        }],
    );
    file_operation_service::prepare(
        &state,
        "interrupted-cover-delete",
        FileOperationKind::CoverReplace,
        &payload,
    )
    .await
    .unwrap();

    library::delete_rom(&state, rom_id, true, None)
        .await
        .unwrap();

    assert!(!asset_root.join(cover_path).exists());
    assert!(!asset_root.join(backup_path).exists());
    let operation_state: String = sqlx::query_scalar(
        "SELECT state FROM file_operations WHERE id = 'interrupted-cover-delete'",
    )
    .fetch_one(state.db())
    .await
    .unwrap();
    assert_eq!(operation_state, "completed");
}

#[tokio::test]
async fn delete_serializes_with_first_cover_application_even_without_existing_covers() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let root = state.config().default_library_root.canonicalize().unwrap();
    let (rom_id, _) = seed_rom_with_file(&state, "nes/game.rom", None).await;
    std::fs::create_dir_all(root.join("nes")).unwrap();
    std::fs::write(root.join("nes/game.rom"), b"rom").unwrap();

    let asset_root = state.config().asset_root.canonicalize().unwrap();
    let blocker = state.file_store().lock_root(&asset_root).await.unwrap();
    let delete_state = state.clone();
    let deletion =
        tokio::spawn(async move { library::delete_rom(&delete_state, rom_id, true, None).await });
    sleep(Duration::from_millis(100)).await;
    assert!(
        !deletion.is_finished(),
        "delete must wait for an in-flight cover mutation"
    );
    drop(blocker);

    let outcome = timeout(Duration::from_secs(3), deletion)
        .await
        .expect("delete should continue after the asset lock is released")
        .unwrap()
        .unwrap();
    assert_eq!(outcome.rom.id, rom_id);
}

#[tokio::test]
async fn delete_deduplicates_an_asset_root_that_is_also_a_library_root() {
    let temp = TempDir::new().unwrap();
    let mut config = support::test_config(&temp);
    config.asset_root = config.default_library_root.clone();
    let state = AppState::initialize(config).await.unwrap();
    let root = state.config().default_library_root.canonicalize().unwrap();
    let cover_path = "covers/game.jpg";
    let legacy_cover_path = "covers/legacy-game.jpg";
    let (rom_id, _) = seed_rom_with_file(&state, "nes/game.rom", Some(cover_path)).await;
    std::fs::create_dir_all(root.join("nes")).unwrap();
    std::fs::write(root.join("nes/game.rom"), b"rom").unwrap();
    std::fs::create_dir_all(root.join("covers")).unwrap();
    std::fs::write(root.join(cover_path), b"cover").unwrap();
    std::fs::write(root.join(legacy_cover_path), b"legacy").unwrap();
    for registered_path in [cover_path, legacy_cover_path] {
        sqlx::query(
            "INSERT INTO cover_assets (rom_id, resource_path, media_type, file_size_bytes) VALUES (?, ?, 'image/jpeg', 5)",
        )
        .bind(rom_id)
        .bind(registered_path)
        .execute(state.db())
        .await
        .unwrap();
    }

    let outcome = timeout(
        Duration::from_secs(3),
        library::delete_rom(&state, rom_id, true, None),
    )
    .await
    .expect("deletion should not deadlock on the shared physical root")
    .unwrap();

    assert_eq!(outcome.rom.id, rom_id);
    assert!(!root.join("nes/game.rom").exists());
    assert!(!root.join(cover_path).exists());
    assert!(!root.join(legacy_cover_path).exists());
}
