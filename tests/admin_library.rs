mod support;

use std::fs;
use support::{empty_request, json_request, response_json, seed_user_record as seed_user};

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::json;
use teatro::{domain::user::UserRole, state::AppState};
use tower::ServiceExt;

include!("admin_library/core.rs");
// Scenario fragments intentionally share the fixture helpers and imports above.
include!("admin_library/descriptors.rs");
include!("admin_library/multidisc.rs");
include!("admin_library/validation.rs");
include!("admin_library/safety.rs");

async fn seed_malicious_rom(state: &AppState) -> i64 {
    let platform_id: i64 = sqlx::query_scalar("SELECT id FROM platforms WHERE slug = 'genesis'")
        .fetch_one(state.db())
        .await
        .unwrap();
    let root_id: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(state.db())
        .await
        .unwrap();

    let rom_id = sqlx::query(
        r#"
        INSERT INTO roms (platform_id, name, slug, regions_json)
        VALUES (?, 'Bad Delete', 'bad-delete', '[]')
        "#,
    )
    .bind(platform_id)
    .execute(state.db())
    .await
    .unwrap()
    .last_insert_rowid();

    sqlx::query(
        r#"
        INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary)
        VALUES (?, ?, '../secret.bin', 'secret.bin', 6, 1)
        "#,
    )
    .bind(rom_id)
    .bind(root_id)
    .execute(state.db())
    .await
    .unwrap();

    rom_id
}

fn upload_request(
    uri: &str,
    auth: Option<(&str, &str)>,
    fields: &[(&str, &str)],
    file_name: &str,
    file_bytes: &[u8],
) -> Request<Body> {
    support::multipart_request(
        "POST",
        uri,
        auth,
        fields,
        &[("file", file_name, file_bytes)],
    )
}

fn cover_request(uri: &str, auth: (&str, &str), media_type: &str, bytes: &[u8]) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header(header::AUTHORIZATION, support::basic_auth(auth.0, auth.1))
        .header(header::CONTENT_TYPE, media_type)
        .body(Body::from(bytes.to_vec()))
        .unwrap()
}

fn batch_upload_request(
    uri: &str,
    auth: Option<(&str, &str)>,
    fields: &[(&str, &str)],
    files: &[(&str, &[u8])],
) -> Request<Body> {
    let files: Vec<_> = files
        .iter()
        .map(|(file_name, bytes)| ("file", *file_name, *bytes))
        .collect();
    support::multipart_request("POST", uri, auth, fields, &files)
}
