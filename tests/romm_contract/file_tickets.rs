fn file_ticket_request(ticket: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/downloads/file?file_id=999999")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("ticket={ticket}&rom_id=999999")))
        .unwrap()
}

#[tokio::test]
async fn selected_file_tickets_preserve_session_auth_selection_and_managed_paths() {
    let test = support::TestApp::new().await;
    seed_user(&test.state, "player", "password", UserRole::ReadOnly).await;
    let rom = seed_rom(
        &test.state,
        &test.temp_dir,
        "genesis",
        "Ticket game",
        "ticket-game",
    )
    .await;
    let other = seed_rom(
        &test.state,
        &test.temp_dir,
        "genesis",
        "Other game",
        "other-game",
    )
    .await;
    let app = test.router.clone();
    let login = app
        .clone()
        .oneshot(support::json_request(
            "POST",
            "/api/auth/session",
            Some(("player", "password")),
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::CREATED);
    let session = response_json(login).await;
    let authorization = format!("Bearer {}", session["token"].as_str().unwrap());
    let issue = |rom_id, file_id| {
        Request::builder()
            .method("POST")
            .uri(format!(
                "/api/roms/{rom_id}/files/{file_id}/download-ticket"
            ))
            .header(header::AUTHORIZATION, &authorization)
            .body(Body::empty())
            .unwrap()
    };

    let mut anonymous = issue(rom.rom_id, rom.file_id);
    anonymous.headers_mut().remove(header::AUTHORIZATION);
    assert_eq!(
        app.clone().oneshot(anonymous).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    for (rom_id, file_id) in [
        (rom.rom_id, other.file_id),
        (999999, rom.file_id),
        (rom.rom_id, 999999),
    ] {
        assert_eq!(
            app.clone()
                .oneshot(issue(rom_id, file_id))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let invalid = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/roms/{}/files/nope/download-ticket",
            rom.rom_id
        ))
        .header(header::AUTHORIZATION, &authorization)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(invalid).await.unwrap().status(),
        StatusCode::BAD_REQUEST
    );

    let created = app
        .clone()
        .oneshot(issue(rom.rom_id, rom.file_id))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::OK);
    assert_eq!(
        created.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let created = response_json(created).await;
    assert_eq!(created["expires_in_seconds"], 60);
    let ticket = created["ticket"].as_str().unwrap();
    let wrong_endpoint = app
        .clone()
        .oneshot(archive_ticket_request(ticket))
        .await
        .unwrap();
    assert_eq!(wrong_endpoint.status(), StatusCode::NOT_FOUND);
    // No request-supplied ROM, file, or filename can redirect the authorized open file.
    let download = app
        .clone()
        .oneshot(file_ticket_request(ticket))
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(
        download.headers()[header::CONTENT_TYPE],
        "application/octet-stream"
    );
    assert_eq!(download.headers()[header::CONTENT_LENGTH], "13");
    assert_eq!(
        download.headers()[header::CONTENT_DISPOSITION],
        "attachment; filename=\"ticket-game.bin\""
    );
    assert_eq!(
        download.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert!(!download.headers().contains_key(header::WWW_AUTHENTICATE));
    assert_eq!(
        support::response_bytes(download).await,
        &b"hello genesis"[..]
    );
    for invalid in [ticket, "bad", ""] {
        let response = app
            .clone()
            .oneshot(file_ticket_request(invalid))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    }

    // Ticket issuance only opens the selected physical file, even for a one-file ROM.
    assert_eq!(
        fs::read_dir(test.temp_dir.path().join("data"))
            .unwrap()
            .count(),
        1
    );
    let path = test.temp_dir.path().join("roms/genesis/ticket-game.bin");
    fs::remove_file(&path).unwrap();
    assert_eq!(
        app.clone()
            .oneshot(issue(rom.rom_id, rom.file_id))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    fs::write(&path, b"changed bytes").unwrap();
    sqlx::query("UPDATE rom_files SET relative_path = '../outside.bin' WHERE id = ?")
        .bind(rom.file_id)
        .execute(test.state.db())
        .await
        .unwrap();
    assert_eq!(
        app.clone()
            .oneshot(issue(rom.rom_id, rom.file_id))
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE rom_files SET relative_path = 'genesis/ticket-game.bin' WHERE id = ?")
        .bind(rom.file_id)
        .execute(test.state.db())
        .await
        .unwrap();
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(
            test.temp_dir.path().join("roms/genesis/other-game.bin"),
            &path,
        )
        .unwrap();
        assert_eq!(
            app.clone()
                .oneshot(issue(rom.rom_id, rom.file_id))
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"changed bytes").unwrap();
    }

    let issued = app
        .clone()
        .oneshot(issue(rom.rom_id, rom.file_id))
        .await
        .unwrap();
    let issued = response_json(issued).await;
    sqlx::query("UPDATE browser_sessions SET expires_at = 0")
        .execute(test.state.db())
        .await
        .unwrap();
    assert_eq!(
        app.clone()
            .oneshot(issue(rom.rom_id, rom.file_id))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let granted = app
        .clone()
        .oneshot(file_ticket_request(issued["ticket"].as_str().unwrap()))
        .await
        .unwrap();
    assert_eq!(
        granted.status(),
        StatusCode::OK,
        "already-issued tickets remain single-use capabilities"
    );
    let response = app
        .oneshot(support::empty_request("POST", "/api/downloads/file", None))
        .await
        .unwrap();
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
}

#[tokio::test]
async fn selected_file_tickets_are_bounded_and_dropped_responses_release_the_open_file() {
    let test = support::TestApp::new().await;
    seed_user(&test.state, "player", "password", UserRole::ReadOnly).await;
    let rom = seed_rom(
        &test.state,
        &test.temp_dir,
        "genesis",
        "Ticket game",
        "ticket-game",
    )
    .await;
    let path = format!(
        "/api/roms/{}/files/{}/download-ticket",
        rom.rom_id, rom.file_id
    );
    let app = test.router.clone();
    let login = app
        .clone()
        .oneshot(support::json_request(
            "POST",
            "/api/auth/session",
            Some(("player", "password")),
            serde_json::json!({}),
        ))
        .await
        .unwrap();
    let session = response_json(login).await;
    let authorization = format!("Bearer {}", session["token"].as_str().unwrap());
    let issue = || {
        Request::builder()
            .method("POST")
            .uri(&path)
            .header(header::AUTHORIZATION, &authorization)
            .body(Body::empty())
            .unwrap()
    };
    let mut first = String::new();
    for index in 0..64 {
        let response = app.clone().oneshot(issue()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        if index == 0 {
            first = response_json(response).await["ticket"]
                .as_str()
                .unwrap()
                .to_owned();
        }
    }
    let busy = app.clone().oneshot(issue()).await.unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(busy.headers()[header::RETRY_AFTER], "1");
    #[cfg(target_os = "linux")]
    let open_file_count = || {
        fs::read_dir("/proc/self/fd")
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .ok()
                    .and_then(|entry| fs::read_link(entry.path()).ok())
                    == Some(test.temp_dir.path().join("roms/genesis/ticket-game.bin"))
            })
            .count()
    };
    #[cfg(target_os = "linux")]
    assert_eq!(open_file_count(), 64);
    let response = app
        .clone()
        .oneshot(file_ticket_request(&first))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    drop(response);
    #[cfg(target_os = "linux")]
    assert_eq!(open_file_count(), 63);
    assert_eq!(
        app.clone().oneshot(issue()).await.unwrap().status(),
        StatusCode::OK
    );
    assert_eq!(
        app.oneshot(file_ticket_request(&first))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
}
