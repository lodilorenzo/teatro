mod support;

use std::{borrow::Cow, str::FromStr};

use sqlx::{
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use teatro::state::AppState;
use tempfile::TempDir;

#[tokio::test]
async fn public_baseline_upgrades_preserve_games_and_remove_age_classifications() {
    let all = sqlx::migrate!("./migrations");
    let migrations: Vec<_> = all.iter().cloned().collect();
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str(&config.database_url)
                .unwrap()
                .create_if_missing(true)
                .foreign_keys(true),
        )
        .await
        .unwrap();
    Migrator {
        migrations: Cow::Owned(migrations[..1].to_vec()),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    }
    .run(&pool)
    .await
    .unwrap();
    let id = sqlx::query("INSERT INTO roms (platform_id, name, slug) SELECT id, 'Preserved', 'preserved' FROM platforms WHERE slug = 'nes'")
        .execute(&pool).await.unwrap().last_insert_rowid();
    sqlx::query("INSERT INTO rom_metadata (rom_id, source, metadata_json) VALUES (?, 'romm', ?)")
        .bind(id)
        .bind(serde_json::json!({"age_ratings":["PEGI 12"],"game_modes":["Co-operative"],"romm":{"age_ratings":[],"remote_rom_id":7}}).to_string())
        .execute(&pool).await.unwrap();
    pool.close().await;
    let state = AppState::initialize(config).await.unwrap();
    let metadata: String =
        sqlx::query_scalar("SELECT metadata_json FROM rom_metadata WHERE rom_id = ?")
            .bind(id)
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&metadata).unwrap(),
        serde_json::json!({"game_modes":["Co-operative"],"romm":{"remote_rom_id":7}})
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let collections: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM collections")
        .fetch_one(state.db())
        .await
        .unwrap();
    assert_eq!(collections, 0);
}

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
    assert_eq!(
        migration_count as usize,
        sqlx::migrate!("./migrations").iter().count()
    );

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
