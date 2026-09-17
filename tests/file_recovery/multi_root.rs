#[tokio::test]
async fn prepared_multi_root_delete_rechecks_rows_after_acquiring_every_root() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let first_root = state.config().default_library_root.canonicalize().unwrap();
    let second_root = temp.path().join("zz-second-root");
    std::fs::create_dir_all(&second_root).unwrap();
    let second_root = second_root.canonicalize().unwrap();
    let second_root_id = sqlx::query(
        "INSERT INTO library_roots (name, root_path, writable) VALUES ('second', ?, 1)",
    )
    .bind(second_root.to_string_lossy().as_ref())
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let (rom_id, first_root_id) = seed_rom_with_file(&state, "nes/first.rom", None).await;
    sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes) VALUES (?, ?, 'nes/second.rom', 'second.rom', 6)",
    )
    .bind(rom_id)
    .bind(second_root_id)
    .execute(state.db())
    .await
    .unwrap();

    for (root, trash, bytes) in [
        (&first_root, ".trash/multi-root/0", b"first".as_slice()),
        (&second_root, ".trash/multi-root/1", b"second".as_slice()),
    ] {
        std::fs::create_dir_all(root.join(trash).parent().unwrap()).unwrap();
        std::fs::write(root.join(trash), bytes).unwrap();
    }
    let payload = DeleteOperationPayload::new(
        vec![rom_id],
        vec![
            DeleteOperationEntry {
                root_id: first_root_id,
                original_relative_path: "nes/first.rom".to_string(),
                trash_relative_path: ".trash/multi-root/0".to_string(),
                directory: false,
            },
            DeleteOperationEntry {
                root_id: second_root_id,
                original_relative_path: "nes/second.rom".to_string(),
                trash_relative_path: ".trash/multi-root/1".to_string(),
                directory: false,
            },
        ],
    );
    file_operation_service::prepare(
        &state,
        "multi-root",
        FileOperationKind::Delete,
        &payload,
    )
        .await
        .unwrap();

    let second_root_blocker = state.file_store().lock_root(&second_root).await.unwrap();
    let reconcile_state = state.clone();
    let reconciliation =
        tokio::spawn(async move { file_operation_service::reconcile(&reconcile_state).await });
    sleep(Duration::from_millis(100)).await;
    assert!(
        !first_root.join("nes/first.rom").exists(),
        "no artifact may be restored before every root lock is held"
    );

    sqlx::query("DELETE FROM roms WHERE id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();
    drop(second_root_blocker);
    timeout(Duration::from_secs(3), reconciliation)
        .await
        .expect("reconciliation should resume")
        .unwrap()
        .unwrap();

    assert!(!first_root.join("nes/first.rom").exists());
    assert!(!second_root.join("nes/second.rom").exists());
    assert!(!first_root.join(".trash/multi-root/0").exists());
    assert!(!second_root.join(".trash/multi-root/1").exists());
}
