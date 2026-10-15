mod support;

use axum::http::StatusCode;
use serde_json::{Value, json};
use support::{TestApp, empty_request, response_json};
use tower::ServiceExt;

async fn page(app: &TestApp, query: &str) -> Value {
    let response = app
        .router
        .clone()
        .oneshot(empty_request(
            "GET",
            &format!("/api/roms?{query}"),
            Some(("reader", "password")),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{query}");
    response_json(response).await
}

#[tokio::test]
async fn metadata_filters_and_sorting_apply_to_the_full_catalog_before_pagination() {
    let app = TestApp::new().await;
    app.seed_readonly("reader", "password").await;
    let alpha = app.seed_rom("genesis", "Alpha", "alpha").await;
    let beta = app.seed_rom("genesis", "Beta", "beta").await;
    let gamma = app.seed_rom("nes", "Gamma", "gamma").await;
    let unknown = app.seed_rom("genesis", "Unknown", "unknown").await;
    for (id, metadata) in [
        (
            alpha,
            json!({"release_year": 1991, "total_rating": 90.5, "rating": 10,
            "game_modes": ["Co-operative"],
            "genres": ["Platform"], "developers": ["Studio & Co"], "publishers": ["Publisher"], "languages": ["English"]}),
        ),
        (
            beta,
            json!({"release_year": 1992, "rating": 75, "game_modes": ["Multiplayer"]}),
        ),
        (
            gamma,
            json!({"release_year": 1991, "aggregated_rating": 90.5,
            "romm": {"tags": ["co-op", "Family"], "genres": ["Platform"]}}),
        ),
        (
            unknown,
            json!({"release_year": "unknown", "rating": "unknown", "game_modes": "not an array"}),
        ),
    ] {
        sqlx::query(
            "INSERT INTO rom_metadata (rom_id, source, metadata_json) VALUES (?, 'test', ?)",
        )
        .bind(id)
        .bind(metadata.to_string())
        .execute(app.state.db())
        .await
        .unwrap();
    }
    sqlx::query("UPDATE roms SET regions_json = '[\"USA\"]' WHERE id = ?")
        .bind(alpha)
        .execute(app.state.db())
        .await
        .unwrap();
    let root: i64 = sqlx::query_scalar("SELECT id FROM library_roots LIMIT 1")
        .fetch_one(app.state.db())
        .await
        .unwrap();
    for (id, sizes) in [(alpha, vec![20, 80]), (beta, vec![60]), (gamma, vec![200])] {
        for (index, size) in sizes.into_iter().enumerate() {
            let name = format!("{id}-{index}.bin");
            sqlx::query("INSERT INTO rom_files (rom_id, root_id, relative_path, file_name, file_size_bytes, is_primary) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(id).bind(root).bind(&name).bind(&name).bind(size).bind(index == 0)
                .execute(app.state.db()).await.unwrap();
        }
    }
    let platform: i64 = sqlx::query_scalar("SELECT platform_id FROM roms WHERE id = ?")
        .bind(alpha)
        .fetch_one(app.state.db())
        .await
        .unwrap();

    for (query, expected) in [
        ("game_mode=cooperative", vec![alpha, gamma]),
        ("game_mode=multiplayer", vec![alpha, beta, gamma]),
        ("year=1991", vec![alpha, gamma]),
        ("year_min=1992&year_max=1992", vec![beta]),
        ("rating=90.5", vec![alpha, gamma]),
        ("rating_min=75&rating_max=90.5", vec![alpha, beta, gamma]),
        ("genre=platform", vec![alpha, gamma]),
        (
            "developer=Studio+%26+Co&publisher=Publisher&language=English&region=usa",
            vec![alpha],
        ),
        ("tag=family", vec![gamma]),
        ("size_min=80&size_max=150", vec![alpha]),
        ("size_max=0", vec![]),
        ("search=Beta&game_mode=multiplayer&year=1992", vec![beta]),
        ("genre=%25", vec![]),
        ("genre=%27+OR+1%3D1--", vec![]),
        ("sort=rating&order=desc", vec![alpha, gamma, beta, unknown]),
        ("sort=rating&order=asc", vec![beta, alpha, gamma, unknown]),
        ("sort=year&order=desc", vec![beta, alpha, gamma, unknown]),
        ("sort=size&order=asc", vec![beta, alpha, gamma, unknown]),
        ("sort=size&order=desc", vec![gamma, alpha, beta, unknown]),
        ("sort=name&order=desc", vec![unknown, gamma, beta, alpha]),
    ] {
        let result = page(&app, query).await;
        let ids: Vec<i64> = result["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rom| rom["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, expected, "{query}");
        assert_eq!(result["total"], expected.len(), "{query}");
    }
    let result = page(
        &app,
        &format!(
            "platform_ids={platform}&game_mode=multiplayer&sort=rating&order=desc&limit=1&offset=1"
        ),
    )
    .await;
    assert_eq!(result["total"], 2);
    assert_eq!(result["items"][0]["id"], beta);

    assert!(page(&app, "limit=1").await.get("ranges").is_none());
    for (query, expected) in [
        (
            "limit=1&offset=2".to_string(),
            json!({"year": [1991.0, 1992.0], "rating": [75.0, 90.5], "size": [60.0, 200.0]}),
        ),
        (
            "year_min=1992".to_string(),
            json!({"year": [1991.0, 1992.0], "rating": [75.0, 75.0], "size": [60.0, 60.0]}),
        ),
        (
            "rating_min=90.5".to_string(),
            json!({"year": [1991.0, 1991.0], "rating": [75.0, 90.5], "size": [100.0, 200.0]}),
        ),
        (
            "size_max=100".to_string(),
            json!({"year": [1991.0, 1992.0], "rating": [75.0, 90.5], "size": [60.0, 200.0]}),
        ),
        (
            format!("platform_ids={platform}"),
            json!({"year": [1991.0, 1992.0], "rating": [75.0, 90.5], "size": [60.0, 100.0]}),
        ),
        (
            "game_mode=cooperative&genre=platform".to_string(),
            json!({"year": [1991.0, 1991.0], "rating": [90.5, 90.5], "size": [100.0, 200.0]}),
        ),
        (
            "search=Beta".to_string(),
            json!({"year": [1992.0, 1992.0], "rating": [75.0, 75.0], "size": [60.0, 60.0]}),
        ),
        (
            "genre=missing".to_string(),
            json!({"year": null, "rating": null, "size": null}),
        ),
    ] {
        assert_eq!(
            page(&app, &format!("include_ranges=true&{query}")).await["ranges"],
            expected,
            "{query}"
        );
    }

    // Absent metadata rows also sort last and never match numeric filters.
    sqlx::query("DELETE FROM rom_metadata WHERE rom_id = ?")
        .bind(unknown)
        .execute(app.state.db())
        .await
        .unwrap();
    assert_eq!(
        page(&app, "sort=year&order=desc").await["items"][3]["id"],
        unknown
    );
    assert_eq!(page(&app, "rating_min=0").await["total"], 3);
}

#[tokio::test]
async fn catalog_rejects_invalid_filters_and_keeps_search_authenticated() {
    let app = TestApp::new().await;
    app.seed_readonly("reader", "password").await;
    for query in [
        "include_ranges=yes",
        "sort=bogus",
        "sort=age",
        "order=sideways",
        "year=0",
        "year=10000",
        "year=NaN",
        "year_min=2000&year_max=1990",
        "rating_min=NaN",
        "rating_min=inf",
        "rating_max=101",
        "rating_min=-1",
        "rating_min=90&rating_max=80",
        "size_min=-1",
        "size_min=1.5",
        "size_max=9223372036854775808",
        "size_min=100&size_max=10",
    ] {
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/roms?{query}"),
                Some(("reader", "password")),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(
            response_json(response).await["error"]["code"],
            "bad_request"
        );
    }
    for query in [
        format!("genre={}", "a".repeat(201)),
        "genre=Action&".repeat(17),
    ] {
        let response = app
            .router
            .clone()
            .oneshot(empty_request(
                "GET",
                &format!("/api/roms?{query}"),
                Some(("reader", "password")),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let response = app
        .router
        .oneshot(empty_request(
            "GET",
            "/api/roms?game_mode=cooperative",
            None,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}
