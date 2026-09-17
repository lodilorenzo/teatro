mod support;

use std::fs;
use support::{response_json, seed_user_record as seed_user};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use base64::{Engine, engine::general_purpose};
use serde::Deserialize;
use teatro::{
    domain::{api_token::ApiTokenScope, user::UserRole},
    repositories::{api_tokens, users},
    services::api_tokens as api_token_service,
    state::AppState,
};
use tempfile::TempDir;
use tower::ServiceExt;

#[derive(Debug, Deserialize)]
struct SiparioRomDetailContract {
    files: Vec<SiparioRomFileContract>,
}

#[derive(Debug, Deserialize)]
struct SiparioRomFileContract {
    id: i64,
    file_name: String,
    file_size_bytes: i64,
}

// Scenario fragments intentionally share the contract fixtures above.
include!("romm_contract/catalog.rs");
include!("romm_contract/grouped_downloads.rs");
include!("romm_contract/file_tickets.rs");
include!("romm_contract/failure_paths.rs");
include!("romm_contract/fixtures.rs");

async fn seed_rom(
    state: &AppState,
    temp_dir: &TempDir,
    platform_slug: &str,
    name: &str,
    slug: &str,
) -> SeededRom {
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = ?")
        .bind(platform_slug)
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();

    let rom_dir = temp_dir.path().join("roms").join(platform_slug);
    fs::create_dir_all(&rom_dir).unwrap();
    let file_name = format!("{slug}.bin");
    fs::write(rom_dir.join(&file_name), b"hello genesis").unwrap();

    let rom_id = sqlx::query(
        r#"
        INSERT INTO roms (
            platform_id,
            name,
            slug,
            summary,
            regions_json,
            path_cover_large
        )
        VALUES (?, ?, ?, 'A fast platform game', '["World"]', 'covers/sonic.webp')
        "#,
    )
    .bind(platform_id)
    .bind(name)
    .bind(slug)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, fetched_at)
        VALUES (?, 'test', '{"genres":["Platform"],"release_year":1991}', '2026-07-07T00:00:00Z')
        "#,
    )
    .bind(rom_id)
    .execute(state.db())
    .await
    .unwrap();

    let group_id = sqlx::query(
        r#"
        INSERT INTO rom_file_groups (rom_id, kind, display_name, group_key, launchable)
        VALUES (?, 'single', ?, ?, 1)
        "#,
    )
    .bind(rom_id)
    .bind(name)
    .bind(slug)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    let relative_path = format!("{platform_slug}/{file_name}");
    let file_id = sqlx::query(
        r#"
        INSERT INTO rom_files (
            rom_id,
            root_id,
            relative_path,
            file_name,
            file_size_bytes,
            is_primary,
            group_id,
            original_file_name,
            role,
            launchable
        )
        VALUES (?, ?, ?, ?, 13, 1, ?, ?, 'content', 1)
        "#,
    )
    .bind(rom_id)
    .bind(root_id)
    .bind(relative_path)
    .bind(&file_name)
    .bind(group_id)
    .bind(&file_name)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    SeededRom {
        platform_id,
        rom_id,
        file_id,
    }
}

struct SeededRom {
    platform_id: i64,
    rom_id: i64,
    file_id: i64,
}

fn empty_request(uri: &str, auth: Option<(&str, &str)>) -> Request<Body> {
    support::empty_request("GET", uri, auth)
}

fn archive_ticket_request(ticket: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/downloads/archive")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("ticket={ticket}")))
        .unwrap()
}
