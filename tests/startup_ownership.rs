mod support;

use std::{process::Command, str::FromStr};

use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use teatro::{
    config::AppConfig,
    domain::workflow::FileOperationKind,
    repositories::{file_operations, library_roots},
    services::file_operations::UploadOperationPayload,
    state::AppState,
};
use tempfile::TempDir;

fn teatro(config: &AppConfig) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_teatro"));
    command
        .env_clear()
        .env("TEATRO_BIND_ADDR", config.bind_addr.to_string())
        .env("TEATRO_DATABASE_URL", &config.database_url)
        .env("TEATRO_DATA_DIR", &config.data_dir)
        .env("TEATRO_DEFAULT_LIBRARY_ROOT", &config.default_library_root)
        .env("TEATRO_ASSET_ROOT", &config.asset_root);
    command
}

#[tokio::test]
async fn concurrent_cli_preserves_live_work_and_only_the_next_server_recovers_it() {
    let temp = TempDir::new().unwrap();
    let mut config = support::test_config(&temp);
    config.bind_addr = "127.0.0.1:41001".parse().unwrap();
    config.uploads.stale_part_age_seconds = 0;
    let state = AppState::initialize(config.clone()).await.unwrap();
    let root_path = config.default_library_root.canonicalize().unwrap();
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await
        .unwrap()
        .unwrap();

    let workspace_marker = config
        .data_dir
        .join("gog-import-staging/active-workspace/marker.bin");
    std::fs::create_dir_all(workspace_marker.parent().unwrap()).unwrap();
    std::fs::write(&workspace_marker, b"active workspace").unwrap();
    std::fs::create_dir_all(root_path.join(".uploads")).unwrap();
    std::fs::write(root_path.join(".uploads/active.part"), b"active upload").unwrap();
    std::fs::write(root_path.join(".uploads/stale.part"), b"stale upload").unwrap();

    let operation = UploadOperationPayload::new(root.id, Vec::new())
        .with_staged_paths(vec![".uploads/active.part".to_string()]);
    file_operations::create(
        state.db(),
        "active-upload",
        FileOperationKind::Upload,
        &serde_json::to_string(&operation).unwrap(),
    )
    .await
    .unwrap();
    let integrity_job_id = sqlx::query("INSERT INTO integrity_jobs (status) VALUES ('running')")
        .execute(state.db())
        .await
        .unwrap()
        .last_insert_rowid();

    for arguments in [["users", "list"].as_slice(), ["report"].as_slice()] {
        let output = teatro(&config).args(arguments).output().unwrap();
        assert!(
            output.status.success(),
            "CLI failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    assert_eq!(
        std::fs::read(&workspace_marker).unwrap(),
        b"active workspace"
    );
    assert_eq!(
        std::fs::read(root_path.join(".uploads/active.part")).unwrap(),
        b"active upload"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM integrity_jobs WHERE id = ?")
            .bind(integrity_job_id)
            .fetch_one(state.db())
            .await
            .unwrap(),
        "running"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM file_operations WHERE id = 'active-upload'"
        )
        .fetch_one(state.db())
        .await
        .unwrap(),
        "prepared"
    );

    let second_server = teatro(&config)
        .env("TEATRO_BIND_ADDR", "127.0.0.1:41002")
        .arg("serve")
        .output()
        .unwrap();
    assert!(!second_server.status.success());
    assert!(
        String::from_utf8_lossy(&second_server.stderr)
            .contains("another Teatro process owns this instance")
    );
    assert_eq!(
        std::fs::read(&workspace_marker).unwrap(),
        b"active workspace"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM integrity_jobs WHERE id = ?")
            .bind(integrity_job_id)
            .fetch_one(state.db())
            .await
            .unwrap(),
        "running"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM file_operations WHERE id = 'active-upload'"
        )
        .fetch_one(state.db())
        .await
        .unwrap(),
        "prepared"
    );

    state.db().close().await;
    drop(state);

    let recovered = AppState::initialize(config).await.unwrap();
    assert!(!workspace_marker.exists());
    assert!(!root_path.join(".uploads/active.part").exists());
    assert!(!root_path.join(".uploads/stale.part").exists());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM integrity_jobs WHERE id = ?")
            .bind(integrity_job_id)
            .fetch_one(recovered.db())
            .await
            .unwrap(),
        "failed"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM file_operations WHERE id = 'active-upload'"
        )
        .fetch_one(recovered.db())
        .await
        .unwrap(),
        "completed"
    );
}

#[tokio::test]
async fn concurrent_cli_refuses_to_change_a_mismatched_migration_ledger() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let state = AppState::initialize(config.clone()).await.unwrap();
    let latest: i64 = sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations")
        .fetch_one(state.db())
        .await
        .unwrap();
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = ?")
        .bind(latest)
        .execute(state.db())
        .await
        .unwrap();

    let output = teatro(&config).args(["users", "list"]).output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("database migrations do not match this Teatro binary")
    );
    let latest_still_missing: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE version = ?")
            .bind(latest)
            .fetch_one(state.db())
            .await
            .unwrap();
    assert_eq!(latest_still_missing, 0);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_startup_releases_ownership_without_following_a_workspace_symlink() {
    let temp = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    let config = support::test_config(&temp);
    let workspace = config.data_dir.join("gog-import-staging");
    std::fs::create_dir_all(&config.data_dir).unwrap();
    std::fs::write(outside.path().join("marker.bin"), b"preserve me").unwrap();
    std::os::unix::fs::symlink(outside.path(), &workspace).unwrap();

    let error = AppState::initialize(config.clone()).await.err().unwrap();
    assert!(
        error
            .to_string()
            .contains("file operation reconciliation failed")
    );
    assert_eq!(
        std::fs::read(outside.path().join("marker.bin")).unwrap(),
        b"preserve me"
    );

    std::fs::remove_file(workspace).unwrap();
    AppState::initialize(config).await.unwrap();
}

#[tokio::test]
async fn offline_cli_initializes_the_database_and_releases_ownership() {
    let temp = TempDir::new().unwrap();
    let config = support::test_config(&temp);

    let output = teatro(&config).args(["users", "list"]).output().unwrap();
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let options = SqliteConnectOptions::from_str(&config.database_url)
        .unwrap()
        .create_if_missing(false);
    let db = SqlitePool::connect_with(options).await.unwrap();
    let applied: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 1")
            .fetch_one(&db)
            .await
            .unwrap();
    assert_eq!(
        applied as usize,
        sqlx::migrate!("./migrations").iter().count()
    );
    db.close().await;

    AppState::initialize(config).await.unwrap();
}
