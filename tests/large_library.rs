mod support;

use std::time::Instant;

use teatro::{
    domain::rom::RomListParams,
    repositories::{library_roots, platforms, roms},
    state::AppState,
};
use tempfile::TempDir;

/// Development benchmark for the constant-query page loader. The repository
/// executes three SQL statements per page: count, ROM page, and page files.
#[tokio::test]
#[ignore = "10,000-row development benchmark"]
async fn ten_thousand_rom_page_uses_bounded_work() {
    let temp = TempDir::new().unwrap();
    let state = AppState::initialize(support::test_config(&temp))
        .await
        .unwrap();
    let platform = platforms::find_by_slug(state.db(), "nes")
        .await
        .unwrap()
        .unwrap();
    let root_path = state.config().default_library_root.canonicalize().unwrap();
    let root = library_roots::find_by_path(state.db(), &root_path)
        .await
        .unwrap()
        .unwrap();

    let mut tx = state.db().begin().await.unwrap();
    for index in 0..10_000_i64 {
        let rom_id = sqlx::query(
            "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, ?, ?, '[]')",
        )
        .bind(platform.id)
        .bind(format!("Benchmark ROM {index:05}"))
        .bind(format!("benchmark-rom-{index}"))
        .execute(&mut *tx)
        .await
        .unwrap()
        .last_insert_rowid();
        if index % 10 == 0 {
            let group_id = sqlx::query(
                "INSERT INTO rom_file_groups (rom_id, kind, display_name, launchable) VALUES (?, 'playlist', ?, 1)",
            )
            .bind(rom_id)
            .bind(format!("Benchmark ROM {index:05}"))
            .execute(&mut *tx)
            .await
            .unwrap()
            .last_insert_rowid();
            for file_index in 0..4_i64 {
                let role = if file_index == 0 {
                    "launch_manifest"
                } else {
                    "disc_image"
                };
                sqlx::query(
                    "INSERT INTO rom_files (rom_id, root_id, group_id, relative_path, file_name, file_size_bytes, is_primary, role, sort_index, launchable) VALUES (?, ?, ?, ?, ?, 1024, ?, ?, ?, 1)",
                )
                .bind(rom_id)
                .bind(root.id)
                .bind(group_id)
                .bind(format!("nes/{index}-{file_index}.bin"))
                .bind(format!("{index}-{file_index}.bin"))
                .bind(if file_index == 0 { 1_i64 } else { 0_i64 })
                .bind(role)
                .bind(file_index)
                .execute(&mut *tx)
                .await
                .unwrap();
            }
        } else {
            sqlx::query(
                "INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary) VALUES (?, ?, ?, ?, 1024, 1)",
            )
            .bind(rom_id)
            .bind(root.id)
            .bind(format!("nes/{index}.nes"))
            .bind(format!("{index}.nes"))
            .execute(&mut *tx)
            .await
            .unwrap();
        }
    }
    tx.commit().await.unwrap();

    let started = Instant::now();
    let page = roms::list(
        state.db(),
        RomListParams {
            limit: 75,
            offset: 0,
            search: Some("Benchmark".to_string()),
            sort: "name".to_string(),
            descending: false,
            missing_cover: false,
            filters: Default::default(),
        },
        &[platform.id],
    )
    .await
    .unwrap();

    assert_eq!(page.total, 10_000);
    assert_eq!(page.items.len(), 75);
    assert!(page.items.iter().any(|rom| rom.files.len() == 1));
    assert!(page.items.iter().any(|rom| rom.files.len() == 4));
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "10k-ROM page exceeded the development performance budget: {elapsed:?}"
    );
    eprintln!("Teatro 10k benchmark: 3 SQL queries, 75-ROM response page, {elapsed:?}");

    sqlx::query("INSERT INTO rom_metadata (rom_id, source, metadata_json) SELECT id, 'test', json_object('total_rating', id % 100, 'game_modes', json('[\"Co-operative\"]')) FROM roms")
        .execute(state.db()).await.unwrap();
    let started = Instant::now();
    let page = roms::list(
        state.db(),
        RomListParams {
            limit: 75,
            offset: 0,
            search: None,
            sort: "rating".to_string(),
            descending: true,
            missing_cover: false,
            filters: teatro::domain::rom::RomFilters {
                rating_min: Some(90.0),
                game_mode: Some("cooperative".to_string()),
                ..Default::default()
            },
        },
        &[platform.id],
    )
    .await
    .unwrap();
    assert_eq!(page.total, 1_000);
    assert_eq!(page.items.len(), 75);
    assert!(
        page.items
            .iter()
            .all(|rom| rom.metadata["total_rating"] == 99)
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "10k-ROM metadata page exceeded budget: {elapsed:?}"
    );
    eprintln!(
        "Teatro 10k metadata benchmark: co-op filter, minimum score, rating sort, {elapsed:?}"
    );

    use tower::ServiceExt;
    support::seed_user_record(
        &state,
        "benchmark",
        "password",
        teatro::domain::user::UserRole::ReadOnly,
    )
    .await;
    let started = Instant::now();
    let response = teatro::api::router(state.clone()).oneshot(support::empty_request(
        "GET", "/api/roms?include_ranges=true&game_mode=cooperative&rating_min=90&limit=75&sort=rating&order=desc", Some(("benchmark", "password")),
    )).await.unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let result = support::response_json(response).await;
    assert_eq!(result["total"], 1000);
    assert_eq!(result["ranges"]["rating"], serde_json::json!([0.0, 99.0]));
    assert_eq!(
        result["ranges"]["size"],
        serde_json::json!([1024.0, 4096.0])
    );
    assert!(result["ranges"]["year"].is_null());
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "10k-ROM facets exceeded budget: {elapsed:?}"
    );
    eprintln!("Teatro 10k faceted API benchmark, including Basic Auth: {elapsed:?}");
}
