mod support;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use support::{TestApp, empty_request, json_request, response_bytes, response_json};
use tower::ServiceExt;

const ADMIN: Option<(&str, &str)> = Some(("admin", "secret"));
const PLAYER: Option<(&str, &str)> = Some(("player", "secret"));

async fn save(app: &TestApp, method: &str, path: &str, body: Value) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(json_request(method, path, ADMIN, body))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        if method == "POST" {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        }
    );
    response_json(response).await
}

#[tokio::test]
async fn collection_crud_validates_membership_urls_and_permissions_atomically() {
    let app = TestApp::new().await;
    app.seed_admin("admin", "secret").await;
    app.seed_readonly("player", "secret").await;
    let game = app.seed_rom("nes", "Selected", "selected").await;
    let input = json!({"name":" Friday games ","slug":"Friday-games","rom_ids":[game,game]});
    for auth in [None, PLAYER] {
        let response = app
            .router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/admin/collections",
                auth,
                input.clone(),
            ))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if auth.is_none() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            }
        );
    }
    let collection = save(&app, "POST", "/api/admin/collections", input).await;
    let id = collection["id"].as_i64().unwrap();
    assert_eq!(collection["name"], "Friday games");
    assert_eq!(collection["slug"], "friday-games");
    assert_eq!(collection["rom_count"], 1);
    assert_eq!(collection["lan_party_zone"], false);
    for slug in [
        "api", "admin", "public", "setup", "assets", "healthz", "../games", "bad/name", "-games",
        "games-", "", "games?x", "é",
    ] {
        let response = app
            .router
            .clone()
            .oneshot(json_request(
                "POST",
                "/api/admin/collections",
                ADMIN,
                json!({"name":"Invalid", "slug":slug, "rom_ids":[]}),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{slug}");
    }
    let response = app
        .router
        .clone()
        .oneshot(json_request(
            "POST",
            "/api/admin/collections",
            ADMIN,
            json!({"name":"Duplicate", "slug":"FRIDAY-GAMES", "rom_ids":[]}),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = app.router.clone().oneshot(json_request("PUT", &format!("/api/admin/collections/{id}"), ADMIN, json!({"name":"Broken", "slug":"broken", "lan_party_zone":true,"rom_ids":[game,999999]}))).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let detail = response_json(
        app.router
            .clone()
            .oneshot(empty_request(
                "GET",
                "/api/collections/friday-games",
                PLAYER,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["rom_ids"], json!([game]));
    assert_eq!(detail["lan_party_zone"], false);
    let list = response_json(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/api/collections", PLAYER))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    let updated = save(
        &app,
        "PUT",
        &format!("/api/admin/collections/{id}"),
        json!({"name":"Renamed", "slug":"renamed", "lan_party_zone":true,"rom_ids":[]}),
    )
    .await;
    assert_eq!(updated["rom_count"], 0);
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/friday-games", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/renamed/", None))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for auth in [None, PLAYER] {
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                "DELETE",
                &format!("/api/admin/collections/{id}"),
                auth,
            ))
            .await
            .unwrap();
        assert!(matches!(
            response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ));
    }
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request(
                "DELETE",
                &format!("/api/admin/collections/{id}"),
                ADMIN
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/api/collections/renamed", None))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        teatro::repositories::roms::find_by_id(app.state.db(), game)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn lan_access_is_scoped_before_filters_pagination_covers_and_downloads() {
    let app = TestApp::with_config(|config| config.auth.rate_limit_max_failures = 100).await;
    app.seed_admin("admin", "secret").await;
    app.seed_readonly("player", "secret").await;
    let selected = app.seed_rom("nes", "Selected", "selected").await;
    let second = app.seed_rom("nes", "Second", "second").await;
    let private = app.seed_rom("nes", "Private", "private").await;
    for (id, year) in [(selected, 1990), (second, 1991), (private, 2026)] {
        sqlx::query(
            "INSERT INTO rom_metadata (rom_id, source, metadata_json) VALUES (?, 'test', ?)",
        )
        .bind(id)
        .bind(json!({"release_year":year}).to_string())
        .execute(app.state.db())
        .await
        .unwrap();
    }
    let zone = save(
        &app,
        "POST",
        "/api/admin/collections",
        json!({"name":"LAN", "slug":"lan", "lan_party_zone":true,"rom_ids":[selected,second]}),
    )
    .await;
    save(
        &app,
        "POST",
        "/api/admin/collections",
        json!({"name":"Private", "slug":"private", "rom_ids":[private]}),
    )
    .await;
    for path in [
        "/api/collections",
        "/api/roms",
        "/api/platforms",
        "/api/users/me",
        "/api/admin/stats",
        "/api/collections/private",
        "/api/collections/private/roms",
        &format!("/api/collections/private/roms/{private}"),
        &format!("/api/collections/private/roms/{private}/archive-ticket"),
    ] {
        let method = if path.ends_with("ticket") {
            "POST"
        } else {
            "GET"
        };
        assert_eq!(
            app.router
                .clone()
                .oneshot(empty_request(method, path, None))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    let response = app.router.clone().oneshot(empty_request("GET", "/api/collections/lan/roms?limit=1&sort=year&order=desc&include_ranges=true&collection_id=999999", None)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let page = response_json(response).await;
    assert_eq!(page["total"], 2);
    assert_eq!(page["items"][0]["id"], second);
    assert_eq!(page["ranges"]["year"], json!([1990.0, 1991.0]));
    let response = response_json(
        app.router
            .clone()
            .oneshot(empty_request(
                "GET",
                "/api/collections/lan/roms?search=Private",
                None,
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(response["total"], 0);
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/collections/lan/roms/{selected}"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    for auth in [None, ADMIN] {
        for (method, path) in [
            ("GET", format!("/api/collections/lan/roms/{private}")),
            (
                "POST",
                format!("/api/collections/lan/roms/{private}/archive-ticket"),
            ),
            (
                "POST",
                format!("/api/collections/lan/roms/{private}/files/1/download-ticket"),
            ),
            (
                "GET",
                "/api/collections/lan/covers/covers/private.png".to_string(),
            ),
        ] {
            assert_eq!(
                app.router
                    .clone()
                    .oneshot(empty_request(method, &path, auth))
                    .await
                    .unwrap()
                    .status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }
    tokio::fs::create_dir_all(app.state.config().asset_root.join("covers"))
        .await
        .unwrap();
    tokio::fs::write(
        app.state.config().asset_root.join("covers/selected.png"),
        b"synthetic cover",
    )
    .await
    .unwrap();
    sqlx::query("UPDATE roms SET path_cover_large = 'covers/selected.png' WHERE id = ?")
        .bind(selected)
        .execute(app.state.db())
        .await
        .unwrap();
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            "/api/collections/lan/covers/covers/selected.png",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_bytes(response).await, "synthetic cover");
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request(
                "GET",
                "/assets/romm/resources/covers/selected.png",
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    // Turning off the zone takes effect on the next request, including metadata, covers and ticket issuance.
    save(
        &app,
        "PUT",
        &format!("/api/admin/collections/{}", zone["id"]),
        json!({"name":"LAN", "slug":"lan", "lan_party_zone":false,"rom_ids":[selected,second]}),
    )
    .await;
    for (method, path) in [
        ("GET", "/api/collections/lan".to_string()),
        ("GET", "/api/collections/lan/roms".to_string()),
        (
            "GET",
            "/api/collections/lan/covers/covers/selected.png".to_string(),
        ),
        (
            "POST",
            format!("/api/collections/lan/roms/{selected}/archive-ticket"),
        ),
    ] {
        assert_eq!(
            app.router
                .clone()
                .oneshot(empty_request(method, &path, None))
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/api/collections/lan/roms", PLAYER))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    sqlx::query("DELETE FROM roms WHERE id = ?")
        .bind(selected)
        .execute(app.state.db())
        .await
        .unwrap();
    let detail = response_json(
        app.router
            .clone()
            .oneshot(empty_request("GET", "/api/collections/lan", ADMIN))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(detail["rom_ids"], json!([second]));
}

#[tokio::test]
async fn lan_guests_download_only_selected_files_and_archives_using_existing_tickets() {
    let app = TestApp::new().await;
    app.seed_admin("admin", "secret").await;
    let response = app
        .router
        .clone()
        .oneshot(support::multipart_request(
            "POST",
            "/api/admin/uploads",
            ADMIN,
            &[("platform_slug", "nes"), ("title", "Synthetic game")],
            &[("file", "synthetic.nes", b"synthetic game bytes")],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let game = response_json(response).await["rom"].clone();
    let id = game["id"].as_i64().unwrap();
    let file_id = game["files"][0]["id"].as_i64().unwrap();
    save(
        &app,
        "POST",
        "/api/admin/collections",
        json!({"name":"Downloads", "slug":"downloads", "lan_party_zone":true,"rom_ids":[id]}),
    )
    .await;
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/api/collections/downloads/roms/{id}/files/{file_id}/download-ticket"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ticket = response_json(response).await["ticket"]
        .as_str()
        .unwrap()
        .to_string();
    let download = || {
        Request::builder()
            .method("POST")
            .uri("/api/downloads/file")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(format!("ticket={ticket}")))
            .unwrap()
    };
    let response = app.router.clone().oneshot(download()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_bytes(response).await, "synthetic game bytes");
    assert_eq!(
        app.router
            .clone()
            .oneshot(download())
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.router
            .clone()
            .oneshot(empty_request(
                "POST",
                &format!("/api/collections/downloads/roms/{id}/files/999999/download-ticket"),
                None
            ))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    // A second registered physical file exercises the same grouped ZIP preparation used by signed-in players.
    let root_id: i64 = sqlx::query_scalar("SELECT root_id FROM rom_files WHERE id = ?")
        .bind(file_id)
        .fetch_one(app.state.db())
        .await
        .unwrap();
    tokio::fs::write(
        app.state.config().default_library_root.join("extra.nes"),
        b"extra bytes",
    )
    .await
    .unwrap();
    sqlx::query("INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, role, launchable) VALUES (?, ?, 'extra.nes', 'extra.nes', 11, 'content', 1)")
        .bind(id).bind(root_id).execute(app.state.db()).await.unwrap();
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "POST",
            &format!("/api/collections/downloads/roms/{id}/archive-ticket"),
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let ticket = response_json(response).await["ticket"]
        .as_str()
        .unwrap()
        .to_string();
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/downloads/archive")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("ticket={ticket}")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/zip");
    let bytes = response_bytes(response).await;
    let archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(archive.len(), 2);
}
