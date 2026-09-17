mod support;

use axum::http::{StatusCode, header};
use serde_json::json;
use support::{empty_request, json_request, response_json, response_text};
use teatro::repositories::users;
use tower::ServiceExt;

#[tokio::test]
async fn first_run_page_creates_the_initial_admin_and_then_closes() {
    let test_app = support::TestApp::new().await;
    let app = test_app.router;

    for path in ["/", "/admin"] {
        let response = app
            .clone()
            .oneshot(empty_request("GET", path, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], "/setup");
    }

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/setup", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_text(response).await;
    assert!(body.contains("Create admin account"));
    assert!(body.contains("id=\"setup-form\""));
    assert!(body.contains("/setup/main.js"));

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/setup/main.js", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response_text(response).await.contains("fetch('/api/setup'"));

    let status = response_json(
        app.clone()
            .oneshot(empty_request("GET", "/api/setup", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, json!({ "required": true }));

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/setup",
            None,
            json!({ "username": "captain", "password": "harbor-secret" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response_json(response).await;
    assert_eq!(created["username"], "captain");
    assert_eq!(created["role"], "admin");

    let status = response_json(
        app.clone()
            .oneshot(empty_request("GET", "/api/setup", None))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, json!({ "required": false }));

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/setup",
            None,
            json!({ "username": "second", "password": "other-secret" }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);

    let response = app
        .clone()
        .oneshot(empty_request("GET", "/setup", None))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers()[header::LOCATION], "/admin");

    for path in ["/", "/admin"] {
        let response = app
            .clone()
            .oneshot(empty_request("GET", path, None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("captain", "harbor-secret")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(users::has_admin(test_app.state.db()).await.unwrap());
}

#[tokio::test]
async fn concurrent_first_admin_creation_has_one_winner() {
    let test_app = support::TestApp::new().await;
    let password_hash = teatro::services::auth::hash_password("secret").unwrap();
    let (first, second) = tokio::join!(
        users::create_initial_admin(test_app.state.db(), "first", &password_hash),
        users::create_initial_admin(test_app.state.db(), "second", &password_hash),
    );

    let outcomes = [first.unwrap(), second.unwrap()];
    assert_eq!(outcomes.iter().filter(|user| user.is_some()).count(), 1);
    assert_eq!(users::list(test_app.state.db()).await.unwrap().len(), 1);
}
