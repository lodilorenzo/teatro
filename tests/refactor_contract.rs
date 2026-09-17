mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use tower::ServiceExt;

#[tokio::test]
async fn malformed_json_uses_teatro_error_envelope() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/users")
                .header(
                    header::AUTHORIZATION,
                    support::basic_auth(&admin.username, &admin.password),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{not-json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = support::response_json(response).await;
    assert_eq!(body["error"]["code"], "bad_request");
    assert!(body["error"]["message"].is_string());
}

#[tokio::test]
async fn oversized_json_uses_teatro_error_envelope() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/users")
                .header(
                    header::AUTHORIZATION,
                    support::basic_auth(&admin.username, &admin.password),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(format!(
                    "{{\"username\":\"{}\",\"password\":\"password\"}}",
                    "x".repeat(3 * 1024 * 1024)
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = support::response_json(response).await;
    assert_eq!(body["error"]["code"], "payload_too_large");
}

#[tokio::test]
async fn multipart_extractor_rejections_use_teatro_error_envelope() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/uploads")
                .header(
                    header::AUTHORIZATION,
                    support::basic_auth(&admin.username, &admin.password),
                )
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = support::response_json(response).await;
    assert_eq!(body["error"]["code"], "bad_request");
    assert_eq!(body["error"]["message"], "invalid multipart upload request");
}

#[tokio::test]
async fn upload_rejects_unknown_multipart_fields_without_draining_them() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let boundary = "unknown-field-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"unexpected\"\r\n\r\nignored\r\n--{boundary}--\r\n"
    );
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/uploads")
                .header(
                    header::AUTHORIZATION,
                    support::basic_auth(&admin.username, &admin.password),
                )
                .header(
                    header::CONTENT_TYPE,
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = support::response_json(response).await;
    assert_eq!(body["error"]["code"], "bad_request");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unexpected multipart field")
    );
}

#[tokio::test]
async fn multipart_file_count_aggregate_and_text_limits_clean_staging_files() {
    let file_count_app = support::TestApp::with_config(|config| {
        config.uploads.max_batch_files = 1;
    })
    .await;
    let admin = file_count_app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let response = file_count_app
        .router
        .clone()
        .oneshot(support::multipart_request(
            "POST",
            "/api/admin/upload-batches",
            auth,
            &[("platform_slug", "genesis")],
            &[("file", "one.bin", b"1"), ("file", "two.bin", b"2")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_no_staging_files(&file_count_app);

    let aggregate_app = support::TestApp::with_config(|config| {
        config.uploads.max_batch_files = 3;
        config.uploads.max_batch_bytes = 4;
    })
    .await;
    let admin = aggregate_app.seed_admin("admin", "password").await;
    let response = aggregate_app
        .router
        .clone()
        .oneshot(support::multipart_request(
            "POST",
            "/api/admin/upload-batches",
            Some((admin.username.as_str(), admin.password.as_str())),
            &[("platform_slug", "genesis")],
            &[("file", "one.bin", b"123"), ("file", "two.bin", b"456")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_no_staging_files(&aggregate_app);

    let text_app = support::TestApp::with_config(|config| {
        config.uploads.max_text_field_bytes = 4;
    })
    .await;
    let admin = text_app.seed_admin("admin", "password").await;
    let response = text_app
        .router
        .clone()
        .oneshot(support::multipart_request(
            "POST",
            "/api/admin/uploads",
            Some((admin.username.as_str(), admin.password.as_str())),
            &[("platform_slug", "genesis"), ("title", "too long")],
            &[("file", "game.bin", b"rom")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_no_staging_files(&text_app);
}

fn assert_no_staging_files(app: &support::TestApp) {
    let staging = app.temp_dir.path().join("roms/.uploads");
    if !staging.exists() {
        return;
    }
    assert_eq!(std::fs::read_dir(staging).unwrap().count(), 0);
}

#[tokio::test]
async fn malformed_path_and_query_inputs_use_teatro_error_envelope() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let authorization = support::basic_auth(&admin.username, &admin.password);

    for uri in ["/api/roms/not-an-id", "/api/roms?limit=not-an-integer"] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::AUTHORIZATION, &authorization)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        let body = support::response_json(response).await;
        assert_eq!(body["error"]["code"], "bad_request", "{uri}");
    }
}

#[tokio::test]
async fn concurrent_same_name_uploads_never_overwrite_a_completed_file() {
    let app = support::TestApp::new().await;
    let admin = app.seed_admin("admin", "password").await;
    let auth = Some((admin.username.as_str(), admin.password.as_str()));
    let first_request = support::multipart_request(
        "POST",
        "/api/admin/uploads",
        auth,
        &[("platform_slug", "genesis")],
        &[("file", "Same Game.bin", b"first-completed-rom")],
    );
    let second_request = support::multipart_request(
        "POST",
        "/api/admin/uploads",
        auth,
        &[("platform_slug", "genesis")],
        &[("file", "Same Game.bin", b"second-completed-rom")],
    );

    let (first, second) = tokio::join!(
        app.router.clone().oneshot(first_request),
        app.router.clone().oneshot(second_request),
    );
    let first = first.unwrap();
    let second = second.unwrap();
    let first_status = first.status();
    let second_status = second.status();
    let first = support::response_json(first).await;
    let second = support::response_json(second).await;
    assert_eq!(first_status, StatusCode::CREATED, "{first}");
    assert_eq!(second_status, StatusCode::CREATED, "{second}");

    let first_path = first["relative_path"].as_str().unwrap();
    let second_path = second["relative_path"].as_str().unwrap();
    assert_ne!(first_path, second_path);
    let root = app.temp_dir.path().join("roms");
    let mut contents = [
        std::fs::read(root.join(first_path)).unwrap(),
        std::fs::read(root.join(second_path)).unwrap(),
    ];
    contents.sort();
    assert_eq!(contents[0], b"first-completed-rom");
    assert_eq!(contents[1], b"second-completed-rom");
}
