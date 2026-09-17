mod support;

use teatro::state::AppState;
use tempfile::TempDir;

#[tokio::test]
async fn fresh_database_uses_the_public_baseline_and_records_the_default_library_root() {
    let temp_dir = TempDir::new().expect("temp dir should be created");
    let state = AppState::initialize(support::test_config(&temp_dir))
        .await
        .unwrap();
    let expected_root = temp_dir.path().join("roms").canonicalize().unwrap();

    let migration_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(migration_count, 1);

    let platform_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM platforms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(platform_count, 62);

    let stored_root: String = sqlx::query_scalar(
        r#"
        SELECT root_path
        FROM library_roots
        WHERE name = 'Default library'
        "#,
    )
    .fetch_one(state.db())
    .await
    .unwrap();

    assert_eq!(stored_root, expected_root.to_string_lossy());
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(state.db())
            .await
            .unwrap()
            .is_empty()
    );
}
