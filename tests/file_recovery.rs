mod support;

use teatro::{
    domain::workflow::{FileOperationKind, FileOperationState},
    repositories::{file_operations, library_roots},
    services::{
        file_operations::{
            self as file_operation_service, CoverOperationEntry, CoverOperationPayload,
            DeleteOperationEntry, DeleteOperationPayload, UploadOperationPayload,
        },
        library,
    },
    state::AppState,
};
use tempfile::TempDir;
use tokio::time::{Duration, sleep, timeout};

// Scenario fragments intentionally share the recovery fixture helpers above.
include!("file_recovery/uploads.rs");
include!("file_recovery/deletes.rs");
include!("file_recovery/covers.rs");
include!("file_recovery/multi_root.rs");

async fn seed_rom_with_file(
    state: &AppState,
    relative_path: &str,
    cover_path: Option<&str>,
) -> (i64, i64) {
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'nes'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await
        .unwrap()
        .unwrap();
    let rom_id = sqlx::query(
        r#"
        INSERT INTO roms (
            platform_id, name, slug, regions_json, path_cover_large, path_cover_small
        )
        VALUES (?, 'Recovery Test', ?, '[]', ?, ?)
        "#,
    )
    .bind(platform_id)
    .bind(format!("recovery-test-{}", root.id))
    .bind(cover_path)
    .bind(cover_path)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();
    sqlx::query(
        r#"
        INSERT INTO rom_files (
            rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary
        )
        VALUES (?, ?, ?, 'game.rom', 3, 1)
        "#,
    )
    .bind(rom_id)
    .bind(root.id)
    .bind(relative_path)
    .execute(state.db())
    .await
    .unwrap();
    (rom_id, root.id)
}
