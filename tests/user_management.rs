mod support;

use axum::http::StatusCode;
use serde_json::json;
use support::{empty_request, json_request, response_json, seed_user_record as seed_user};
use teatro::{domain::user::UserRole, repositories::users};
use tower::ServiceExt;

#[tokio::test]
async fn admin_can_manage_users_through_api() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/users",
            Some(("admin", "admin-password")),
            json!({
                "username": "player",
                "password": "player-password",
                "role": "readonly"
            }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let created = response_json(response).await;
    let player_id = created["id"].as_i64().unwrap();
    assert_eq!(created["username"], "player");
    assert_eq!(created["role"], "readonly");
    assert!(created.get("password_hash").is_none());

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/admin/users",
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let users = response_json(response).await;
    assert_eq!(users.as_array().unwrap().len(), 2);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("player", "player-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let me = response_json(response).await;
    assert_eq!(me["username"], "player");
    assert_eq!(me["role"], "readonly");

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/users/{player_id}/password"),
            Some(("admin", "admin-password")),
            json!({ "password": "new-player-password" }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/users/me",
            Some(("player", "new-player-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/users/{player_id}/role"),
            Some(("admin", "admin-password")),
            json!({ "role": "admin" }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["role"], "admin");

    let response = app
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/users/{player_id}"),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["username"], "player");
}

#[tokio::test]
async fn concurrent_admin_demotions_keep_one_admin() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    let first = seed_user(&state, "first", "password", UserRole::Admin).await;
    let second = seed_user(&state, "second", "password", UserRole::Admin).await;

    let (left, right) = tokio::join!(
        users::set_role(state.db(), first.id, UserRole::ReadOnly),
        users::set_role(state.db(), second.id, UserRole::ReadOnly),
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let admin_count = users::list(state.db())
        .await
        .unwrap()
        .into_iter()
        .filter(|user| user.role.is_admin())
        .count();
    assert_eq!(admin_count, 1);
}

#[tokio::test]
async fn readonly_user_cannot_manage_users_through_api() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    seed_user(&state, "reader", "reader-password", UserRole::ReadOnly).await;
    let app = test_app.router.clone();

    let response = app
        .oneshot(empty_request(
            "GET",
            "/api/admin/users",
            Some(("reader", "reader-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn cannot_delete_or_demote_last_admin() {
    let test_app = support::TestApp::new().await;
    let state = test_app.state.clone();
    let admin = seed_user(&state, "admin", "admin-password", UserRole::Admin).await;
    let app = test_app.router.clone();

    let response = app
        .clone()
        .oneshot(empty_request(
            "DELETE",
            &format!("/api/admin/users/{}", admin.id),
            Some(("admin", "admin-password")),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let response = app
        .oneshot(json_request(
            "PATCH",
            &format!("/api/admin/users/{}/role", admin.id),
            Some(("admin", "admin-password")),
            json!({ "role": "readonly" }),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}
