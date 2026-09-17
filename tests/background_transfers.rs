mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::json;
use support::{TestApp, basic_auth, response_json};
use teatro::domain::user::UserRole;
use tower::ServiceExt;

const UPLOAD: &str = "/api/admin/upload-batches";
const GOG: &str = "/api/admin/gog-imports";

fn transfer_request(endpoint: &str, authorization: &str, id: &str) -> Request<Body> {
    let mut request = support::multipart_request(
        "POST",
        endpoint,
        None,
        &[("platform_slug", "genesis")],
        &[("file", "Synthetic.bin", b"synthetic transfer bytes")],
    );
    request
        .headers_mut()
        .insert(header::AUTHORIZATION, authorization.parse().unwrap());
    request
        .headers_mut()
        .insert("x-teatro-transfer-id", id.parse().unwrap());
    request
}

fn cancel_request(authorization: &str, id: &str, operation: &str) -> Request<Body> {
    Request::builder()
        .method("DELETE")
        .uri(format!(
            "/api/admin/background-transfers/{id}?operation={operation}"
        ))
        .header(header::AUTHORIZATION, authorization)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn cross_endpoint_replay_keeps_disabled_feature_error() {
    let test = TestApp::new().await;
    test.seed_admin("owner", "password").await;
    let admin = basic_auth("owner", "password");
    let id = "upload_endpoint_regression";
    let response = test
        .router
        .clone()
        .oneshot(transfer_request(UPLOAD, &admin, id))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let response = test
        .router
        .oneshot(transfer_request(GOG, &admin, id))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "service_unavailable"
    );
}

#[tokio::test]
async fn running_and_cancelled_transfers_require_the_current_owner_role() {
    use std::time::Duration;
    use tokio_util::io::ReaderStream;

    let test = TestApp::new().await;
    let owner = support::seed_user_record(&test.state, "owner", "password", UserRole::Admin).await;
    test.seed_admin("other", "password").await;
    test.seed_readonly("reader", "password").await;
    let admin = basic_auth("owner", "password");
    let other = basic_auth("other", "password");
    let reader = basic_auth("reader", "password");
    let id = "upload_running";
    let app = test.router.clone();
    let token_response = app
        .clone()
        .oneshot(support::json_request(
            "POST",
            "/api/admin/tokens",
            Some(("owner", "password")),
            json!({"name": "Read scoped", "scopes": ["read"]}),
        ))
        .await
        .unwrap();
    let token = response_json(token_response).await;
    let read_token = format!("Bearer {}", token["token"].as_str().unwrap());

    // Hold the multipart body open so the request stays in the running cache state.
    let (writer, body) = tokio::io::duplex(64);
    let mut request = transfer_request(UPLOAD, &admin, id);
    *request.body_mut() = Body::from_stream(ReaderStream::new(body));
    let active = tokio::spawn(app.clone().oneshot(request));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let mut retry = transfer_request(UPLOAD, &admin, id);
            *retry.body_mut() = Body::empty();
            let response = app.clone().oneshot(retry).await.unwrap();
            if response.status() == StatusCode::CONFLICT {
                assert_eq!(response.headers()[header::RETRY_AFTER], "1");
                assert_eq!(
                    response_json(response).await["error"]["code"],
                    "transfer_in_progress"
                );
                break;
            }
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    for expected in [StatusCode::CONFLICT, StatusCode::GONE] {
        for authorization in [&reader, &read_token] {
            let response = app
                .clone()
                .oneshot(transfer_request(UPLOAD, authorization, id))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            let response = app
                .clone()
                .oneshot(cancel_request(authorization, id, "upload"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        for (authorization, operation) in [(&other, "upload"), (&admin, "gog")] {
            let response = app
                .clone()
                .oneshot(cancel_request(authorization, id, operation))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
        }
        sqlx::query("UPDATE users SET role = 'readonly' WHERE id = ?")
            .bind(owner.id)
            .execute(test.state.db())
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(transfer_request(UPLOAD, &admin, id))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let response = app
            .clone()
            .oneshot(cancel_request(&admin, id, "upload"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        sqlx::query("UPDATE users SET role = 'admin' WHERE id = ?")
            .bind(owner.id)
            .execute(test.state.db())
            .await
            .unwrap();
        let response = app
            .clone()
            .oneshot(transfer_request(UPLOAD, &admin, id))
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::GONE {
            assert_eq!(
                response_json(response).await["error"]["code"],
                "transfer_cancelled"
            );
        }
        let response = app
            .clone()
            .oneshot(cancel_request(&admin, id, "upload"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    let response = tokio::time::timeout(Duration::from_secs(5), active)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    drop(writer);

    for authorization in [&admin, &reader] {
        for invalid in [
            "invalid",
            "upload_",
            "upload_bad-id",
            &format!("upload_{}", "x".repeat(128)),
        ] {
            let response = app
                .clone()
                .oneshot(transfer_request(UPLOAD, authorization, invalid))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if authorization == &admin {
                    StatusCode::BAD_REQUEST
                } else {
                    StatusCode::FORBIDDEN
                }
            );
        }
    }
    for operation in ["", "invalid", "upload&operation=gog"] {
        let response = app
            .clone()
            .oneshot(cancel_request(&admin, id, operation))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response_json(response).await["error"]["code"],
            "bad_request"
        );
    }
    let response = app
        .oneshot(support::empty_request(
            "DELETE",
            &format!("/api/admin/background-transfers/{id}"),
            Some(("owner", "password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn completed_transfer_revalidates_roles_and_isolates_users_and_endpoints() {
    let test = TestApp::new().await;
    let owner = support::seed_user_record(&test.state, "owner", "password", UserRole::Admin).await;
    test.seed_admin("other", "password").await;
    test.seed_readonly("reader", "password").await;
    let admin = basic_auth("owner", "password");
    let other = basic_auth("other", "password");
    let reader = basic_auth("reader", "password");
    let app = test.router.clone();
    let token_response = app
        .clone()
        .oneshot(support::json_request(
            "POST",
            "/api/admin/tokens",
            Some(("owner", "password")),
            json!({"name": "Read scoped", "scopes": ["read"]}),
        ))
        .await
        .unwrap();
    assert_eq!(token_response.status(), StatusCode::CREATED);
    let token = response_json(token_response).await;
    let read_token = format!("Bearer {}", token["token"].as_str().unwrap());

    let id = "upload_auth_regression";
    let response = app
        .clone()
        .oneshot(transfer_request(UPLOAD, &admin, id))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let original = response_json(response).await;
    let mut anonymous = transfer_request(UPLOAD, &admin, id);
    anonymous.headers_mut().remove(header::AUTHORIZATION);
    let response = app.clone().oneshot(anonymous).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    for authorization in [&reader, &read_token] {
        for endpoint in [UPLOAD, GOG] {
            let response = app
                .clone()
                .oneshot(transfer_request(endpoint, authorization, id))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{endpoint}");
        }
        let response = app
            .clone()
            .oneshot(cancel_request(authorization, id, "upload"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    // The same user and operation replay even with a different payload, without ingesting it.
    let mut retry = transfer_request(UPLOAD, &admin, id);
    *retry.body_mut() = Body::from("different payload");
    let response = app.clone().oneshot(retry).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response_json(response).await, original);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roms")
        .fetch_one(test.state.db())
        .await
        .unwrap();
    assert_eq!(count, 1);
    let paths: Vec<String> = sqlx::query_scalar("SELECT relative_path FROM rom_files")
        .fetch_all(test.state.db())
        .await
        .unwrap();
    assert_eq!(paths.len(), 1);
    assert_eq!(
        std::fs::read(test.state.config().default_library_root.join(&paths[0])).unwrap(),
        b"synthetic transfer bytes"
    );

    let mut other_request = transfer_request(UPLOAD, &other, id);
    *other_request.body_mut() = Body::empty();
    let response = app.clone().oneshot(other_request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = app
        .clone()
        .oneshot(cancel_request(&other, id, "upload"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .clone()
        .oneshot(cancel_request(&admin, id, "gog"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = app
        .clone()
        .oneshot(cancel_request(&admin, id, "upload"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    sqlx::query("UPDATE users SET role = 'readonly' WHERE id = ?")
        .bind(owner.id)
        .execute(test.state.db())
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(transfer_request(UPLOAD, &admin, id))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .oneshot(cancel_request(&admin, id, "upload"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
