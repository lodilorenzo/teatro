mod support;

use std::time::Duration;
use support::{response_json, seed_user_record as seed_user};

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use teatro::domain::user::UserRole;
use tower::ServiceExt;

#[tokio::test]
async fn logiqx_dat_job_hashes_matches_and_upgrades_filename_metadata() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(multipart_request(
            "/api/admin/uploads",
            "upload-boundary",
            &[form_field("platform_slug", "genesis")],
            "file",
            "Verified Game (Europe).bin",
            b"test",
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let upload = response_json(response).await;
    let rom_id = upload["rom"]["id"].as_i64().unwrap();
    assert_eq!(upload["rom"]["regions"], json!(["Europe"]));

    let dat = br#"<?xml version="1.0"?>
<datafile>
  <header>
    <name>Teatro Verification DAT</name>
    <version>2026-07-09</version>
    <author>Teatro tests</author>
  </header>
  <game name="Verified Game (USA)">
    <description>Verified Game (USA)</description>
    <serial>MK-12345</serial>
    <release name="Verified Game" region="USA" language="En,Ja" />
    <rom name="Verified Game.bin" size="4"
         crc="D87F7E0C"
         md5="098F6BCD4621D373CADE4E832627B4F6"
         sha1="A94A8FE5CCB19BA61C4C0873D391E987982FBBD3"
         sha256="9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08" />
  </game>
</datafile>"#;
    let response = app
        .clone()
        .oneshot(multipart_request(
            "/api/admin/dats",
            "dat-boundary",
            &[],
            "file",
            "verification.dat",
            dat,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let imported = response_json(response).await;
    assert_eq!(imported["source"]["name"], "Teatro Verification DAT");
    assert_eq!(imported["imported_entries"], 1);
    assert_eq!(imported["already_imported"], false);

    let response = app
        .clone()
        .oneshot(multipart_request(
            "/api/admin/dats",
            "duplicate-dat-boundary",
            &[],
            "file",
            "verification-copy.dat",
            dat,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let duplicate = response_json(response).await;
    assert_eq!(duplicate["source"]["id"], imported["source"]["id"]);
    assert_eq!(duplicate["already_imported"], true);

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/integrity/jobs",
            json!({"rom_id": rom_id}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let job = response_json(response).await;
    let job_id = job["id"].as_i64().unwrap();

    let completed = wait_for_job(&app, job_id).await;
    assert_eq!(completed["status"], "completed", "{completed}");
    assert_eq!(completed["processed_files"], 1);
    assert_eq!(completed["hashed_files"], 1);
    assert_eq!(completed["matched_files"], 1);
    assert_eq!(completed["error_count"], 0);

    let response = app
        .clone()
        .oneshot(empty_request(&format!(
            "/api/admin/roms/{rom_id}/integrity"
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let report = response_json(response).await;
    assert_eq!(report["status"], "verified");
    assert_eq!(report["files"][0]["hash_status"], "complete");
    assert_eq!(report["files"][0]["hashes"]["crc32"], "d87f7e0c");
    assert_eq!(
        report["files"][0]["hashes"]["sha256"],
        "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
    );
    assert_eq!(report["files"][0]["matches"][0]["matched_by"], "sha256");
    assert_eq!(report["files"][0]["matches"][0]["serial"], "MK-12345");
    assert_eq!(report["files"][0]["matches"][0]["regions"], json!(["USA"]));

    let response = app
        .clone()
        .oneshot(empty_request(&format!("/api/roms/{rom_id}")))
        .await
        .unwrap();
    let detail = response_json(response).await;
    assert_eq!(detail["regions"], json!(["USA"]));
    assert_eq!(detail["metadatum"]["source"], "dat");
    assert_eq!(detail["metadatum"]["filename"]["confidence"], "high");
    assert_eq!(detail["metadatum"]["integrity"]["authority"], "dat");
    assert_eq!(detail["metadatum"]["integrity"]["filename_fallback"], false);
    assert_eq!(detail["metadatum"]["serials"], json!(["MK-12345"]));
    assert_eq!(detail["metadatum"]["languages"], json!(["En", "Ja"]));

    let response = app
        .oneshot(empty_request(&format!("/api/admin/roms/{rom_id}/files")))
        .await
        .unwrap();
    let files = response_json(response).await;
    assert_eq!(files["groups"][0]["files"][0]["hash_status"], "complete");
    assert_eq!(
        files["groups"][0]["files"][0]["sha1"],
        "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3"
    );
}

#[tokio::test]
async fn integrity_job_admission_returns_the_single_active_job() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state;
    let active_id = sqlx::query("INSERT INTO integrity_jobs (total_files) VALUES (0)")
        .execute(state.db())
        .await
        .unwrap()
        .last_insert_rowid();

    let claimed = teatro::services::integrity::start_hash_job(state.clone(), None, true)
        .await
        .unwrap();

    assert_eq!(claimed.id, active_id);
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM integrity_jobs WHERE status IN ('queued', 'running')",
    )
    .fetch_one(state.db())
    .await
    .unwrap();
    assert_eq!(active_count, 1);
}

async fn wait_for_job(app: &axum::Router, job_id: i64) -> Value {
    for _ in 0..100 {
        let response = app
            .clone()
            .oneshot(empty_request(&format!(
                "/api/admin/integrity/jobs/{job_id}"
            )))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let job = response_json(response).await;
        if matches!(job["status"].as_str(), Some("completed" | "failed")) {
            return job;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("integrity job did not complete");
}

fn form_field(name: &str, value: &str) -> Vec<u8> {
    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").into_bytes()
}

fn multipart_request(
    uri: &str,
    boundary: &str,
    fields: &[Vec<u8>],
    file_field: &str,
    file_name: &str,
    bytes: &[u8],
) -> Request<Body> {
    let mut body = Vec::new();
    for field in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(field);
    }
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    Request::builder()
        .method("POST")
        .uri(uri)
        .header(
            header::AUTHORIZATION,
            support::basic_auth("admin", "admin-password"),
        )
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap()
}

fn json_request(method: &str, uri: &str, body: Value) -> Request<Body> {
    support::json_request(method, uri, Some(("admin", "admin-password")), body)
}

fn empty_request(uri: &str) -> Request<Body> {
    support::empty_request("GET", uri, Some(("admin", "admin-password")))
}
