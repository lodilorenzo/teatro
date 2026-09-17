mod support;

use teatro::{
    repositories::{file_operations, integrity, roms},
    state::AppState,
};
use tempfile::TempDir;

#[tokio::test]
async fn corrupt_persisted_json_and_group_kinds_fail_decoding() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'nes'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let rom_id = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'Corrupt', 'corrupt', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO rom_file_groups (rom_id, kind, display_name, metadata_json) VALUES (?, 'single', 'Corrupt', '{}')",
    )
    .bind(rom_id)
    .execute(state.db())
    .await
    .unwrap();

    sqlx::query("DROP TRIGGER validate_roms_json_update")
        .execute(state.db())
        .await
        .unwrap();
    sqlx::query("UPDATE roms SET regions_json = 'not-json' WHERE id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();
    let error = roms::find_by_id(state.db(), rom_id).await.unwrap_err();
    assert!(error.to_string().contains("invalid JSON persisted"));

    sqlx::query("DROP TRIGGER validate_rom_file_group_update")
        .execute(state.db())
        .await
        .unwrap();
    sqlx::query("UPDATE rom_file_groups SET kind = 'mystery' WHERE rom_id = ?")
        .bind(rom_id)
        .execute(state.db())
        .await
        .unwrap();
    let error = roms::file_groups_for_rom(state.db(), rom_id)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid persisted FileGroupKind")
    );
}

#[tokio::test]
async fn corrupt_dat_match_metadata_fails_decoding() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'nes'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots ORDER BY id LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    let rom_id = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'DAT Corrupt', 'dat-corrupt', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let file_id = sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name) VALUES (?, ?, 'nes/dat.bin', 'dat.bin')",
    )
    .bind(rom_id)
    .bind(root_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let source_id = sqlx::query(
        "INSERT INTO dat_sources (name, imported_file_name, file_sha256, entry_count) VALUES ('DAT', 'dat.xml', 'hash', 1)",
    )
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let entry_id = sqlx::query(
        "INSERT INTO dat_entries (source_id, game_name, rom_name, regions_json, languages_json) VALUES (?, 'Game', 'dat.bin', '[\"USA\"]', '[\"En\"]')",
    )
    .bind(source_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let invalid_method = sqlx::query(
        "INSERT INTO rom_file_dat_matches (file_id, dat_entry_id, matched_by) VALUES (?, ?, 'unknown')",
    )
    .bind(file_id)
    .bind(entry_id)
    .execute(state.db())
    .await
    .unwrap_err();
    assert!(
        invalid_method
            .to_string()
            .contains("invalid rom_file_dat_matches.matched_by")
    );

    sqlx::query(
        "INSERT INTO rom_file_dat_matches (file_id, dat_entry_id, matched_by) VALUES (?, ?, 'sha256')",
    )
    .bind(file_id)
    .bind(entry_id)
    .execute(state.db())
    .await
    .unwrap();

    sqlx::query("DROP TRIGGER validate_dat_entry_json_update")
        .execute(state.db())
        .await
        .unwrap();
    sqlx::query("UPDATE dat_entries SET languages_json = 'not-json' WHERE id = ?")
        .bind(entry_id)
        .execute(state.db())
        .await
        .unwrap();

    let error = integrity::matches_for_rom(state.db(), rom_id)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid JSON persisted in dat_entries.languages_json")
    );
}

#[tokio::test]
async fn corrupt_workflow_states_fail_typed_decoding() {
    let app = support::TestApp::new().await;
    let state = app.state;
    let job_id = sqlx::query("INSERT INTO integrity_jobs (total_files) VALUES (0)")
        .execute(state.db())
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query(
        "INSERT INTO file_operations (id, kind, state, payload_json) VALUES ('bad-state', 'upload', 'prepared', '{}')",
    )
    .execute(state.db())
    .await
    .unwrap();
    let rom_id = support::seed_rom(&state, "nes", "Workflow Corrupt", "workflow-corrupt").await;
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots ORDER BY id LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name) VALUES (?, ?, 'nes/workflow.bin', 'workflow.bin')",
    )
    .bind(rom_id)
    .bind(root_id)
    .execute(state.db())
    .await
    .unwrap();

    let mut connection = state.db().acquire().await.unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("UPDATE integrity_jobs SET status = 'mystery' WHERE id = ?")
        .bind(job_id)
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("UPDATE file_operations SET state = 'mystery' WHERE id = 'bad-state'")
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::query("UPDATE rom_files SET hash_status = 'mystery' WHERE rom_id = ?")
        .bind(rom_id)
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);

    assert!(
        integrity::find_job(state.db(), job_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid persisted IntegrityJobStatus")
    );
    assert!(
        file_operations::pending(state.db())
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid persisted FileOperationState")
    );
    assert!(
        roms::find_by_id(state.db(), rom_id)
            .await
            .unwrap_err()
            .to_string()
            .contains("invalid persisted FileHashStatus")
    );
}

#[tokio::test]
async fn relationship_owners_cannot_move_between_roms() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'nes'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots ORDER BY id LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();
    let first_rom = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'First', 'first', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let second_rom = sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, 'Second', 'second', '[]')",
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let group_id = sqlx::query(
        "INSERT INTO rom_file_groups (rom_id, kind, display_name) VALUES (?, 'single', 'First')",
    )
    .bind(first_rom)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let parent_file = sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, group_id) VALUES (?, ?, 'nes/first.cue', 'first.cue', ?)",
    )
    .bind(first_rom)
    .bind(root_id)
    .bind(group_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    let child_file = sqlx::query(
        "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, group_id) VALUES (?, ?, 'nes/first.bin', 'first.bin', ?)",
    )
    .bind(first_rom)
    .bind(root_id)
    .bind(group_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO rom_file_dependencies (parent_file_id, child_file_id, dependency_kind) VALUES (?, ?, 'cue_file')",
    )
    .bind(parent_file)
    .bind(child_file)
    .execute(state.db())
    .await
    .unwrap();

    let file_error = sqlx::query("UPDATE rom_files SET rom_id = ? WHERE id = ?")
        .bind(second_rom)
        .bind(child_file)
        .execute(state.db())
        .await
        .unwrap_err();
    assert!(
        file_error
            .to_string()
            .contains("rom_files.rom_id is immutable")
    );

    let group_error = sqlx::query("UPDATE rom_file_groups SET rom_id = ? WHERE id = ?")
        .bind(second_rom)
        .bind(group_id)
        .execute(state.db())
        .await
        .unwrap_err();
    assert!(
        group_error
            .to_string()
            .contains("rom_file_groups.rom_id is immutable")
    );
}
