mod support;

use axum::{
    body::{Body, to_bytes},
    http::{StatusCode, header},
};
use support::response_text;
use tower::ServiceExt;

#[tokio::test]
async fn web_admin_assets_are_served_without_api_auth() {
    let test_app = support::TestApp::new().await;
    test_app.seed_admin("admin", "secret").await;
    let app = test_app.router;

    let response = app.clone().oneshot(empty_request("/admin")).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/html; charset=utf-8"
    );
    let content_security_policy = response.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap();
    assert!(content_security_policy.contains("img-src 'self' blob: data: https://images.igdb.com"));
    let body = response_text(response).await;
    assert!(body.contains("Teatro Admin"));
    assert!(body.contains("/admin/main.js"));
    assert!(body.contains(
        "rel=\"icon\" type=\"image/svg+xml\" href=\"/admin/favicon.svg?icon=teatro-controller\""
    ));

    let response = app
        .clone()
        .oneshot(empty_request("/admin/app.css"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/css; charset=utf-8"
    );
    let body = response_text(response).await;
    assert!(body.contains("font-family: \"Atkinson Hyperlegible Next\""));
    assert!(body.contains("--font-display: \"Atkinson Hyperlegible Next\", system-ui, sans-serif"));
    assert!(body.contains("--palette-ink: #111018"));
    assert!(body.contains("--palette-wine: #633436"));
    assert!(body.contains("--palette-red: #bd4444"));
    assert!(body.contains("--palette-paper: #f3f1f4"));
    assert!(body.contains("--brand: #d65353"));
    assert!(body.contains("filter: blur(90px) saturate(110%)"));
    assert!(body.contains(".icon {"));
    assert!(body.contains(".drop-zone"));
    assert!(body.contains(".upload-file-list"));
    assert!(body.contains(".upload-file-remove"));
    assert!(body.contains(".upload-file-actions"));
    assert!(body.contains(".upload-progress"));
    assert!(body.contains(".nav-activity"));
    assert!(body.contains("@keyframes nav-activity-ping"));
    assert!(body.contains(".gog-import-warning"));
    assert!(body.contains(".gog-import-output"));
    assert!(body.contains(".gog-import-poll-warning"));
    assert!(body.contains(".jobs-summary"));
    assert!(body.contains(".job-list"));
    assert!(body.contains(".job-card"));
    assert!(body.contains(".upload-plan"));
    assert!(body.contains(".plan-title-edit"));
    assert!(body.contains(".plan-rom-card"));
    assert!(body.contains(".file-group-card"));
    assert!(body.contains(".detail-media"));
    assert!(body.contains(".admin-file-row strong"));
    assert!(body.contains(".detail-metadata"));
    assert!(body.contains(".library-detail-drawer"));
    assert!(body.contains(".credential-status"));
    assert!(body.contains(".settings-subsection"));
    assert!(body.contains("@import url(\"/public/pagination.css\")"));
    assert!(body.contains(".cleanup-section"));

    for font in [
        "AtkinsonHyperlegibleNext-Latin.woff2",
        "AtkinsonHyperlegibleNext-LatinExt.woff2",
    ] {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/admin/{font}")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{font}");
        assert_eq!(response.headers()[header::CONTENT_TYPE], "font/woff2");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert!(bytes.starts_with(b"wOF2"), "invalid WOFF2 file {font}");
    }

    let response = app
        .clone()
        .oneshot(empty_request("/admin/main.js"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "application/javascript; charset=utf-8"
    );
    assert!(
        response
            .headers()
            .contains_key(header::CONTENT_SECURITY_POLICY)
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    let mut body = response_text(response).await;
    assert!(body.contains("sideNavButton('upload', 'upload', 'Import')"));
    assert!(body.contains("aria-label=\"Open player library\" title=\"Player library\">${icon('external-link')}</button>"));
    assert!(
        body.contains(
            "aria-label=\"Refresh data\" title=\"Refresh\">${icon('refresh-cw')}</button>"
        )
    );
    assert!(body.contains("aria-label=\"Sign out\" title=\"Sign out\">${icon('logout')}</button>"));
    assert!(!body.contains("sideNavButton('gogImport'"));
    for view in [
        "dashboard",
        "gog-import",
        "jobs",
        "library",
        "library-renderers",
        "metadata",
        "romm-browse",
        "romm-source",
        "server-cleanup",
        "settings",
        "shared",
        "upload",
        "upload-renderers",
    ] {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/admin/views/{view}.js")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        body.push_str(&response_text(response).await);
    }
    for feature in [
        "background-transfer",
        "gog-import",
        "gog-title",
        "igdb",
        "jobs",
        "library",
        "library-scan",
        "romm-source",
        "server-jobs",
        "server-cleanup",
        "upload",
        "upload-transfer",
    ] {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/admin/features/{feature}.js")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        body.push_str(&response_text(response).await);
    }
    let response = app
        .clone()
        .oneshot(empty_request("/admin/api.js"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body.push_str(&response_text(response).await);
    assert!(body.contains("../public/api.js"));
    assert!(body.contains("../../public/downloads.js"));
    assert!(body.contains("../../public/pagination.js"));
    assert!(body.contains("../public/scroll.js"));
    assert!(body.contains("data-scroll-key=\"screen\""));
    assert!(body.contains("/api/admin/uploads"));
    assert!(body.contains("/api/admin/uploads/preview"));
    assert!(body.contains("/api/admin/upload-batches"));
    assert!(body.contains("/api/admin/library/scans"));
    assert!(body.contains("/api/admin/library/sidecars"));
    assert!(body.contains("Scan library"));
    assert!(body.contains("Preview sidecar cleanup"));
    assert!(body.contains("DELETE SIDECARS"));
    assert!(body.contains("data-action=\"public-library\""));
    assert!(body.contains("window.location.assign('/')"));
    assert!(body.contains("beforeunload"));
    assert!(body.contains("pagehide"));
    assert!(body.contains("keepalive: true"));
    assert!(body.contains("jobsController.cancelAll()"));
    assert!(body.contains("Ongoing jobs will be cancelled"));
    assert!(body.contains("waitForBackgroundTransfer"));
    assert!(body.contains("calculateTransferProgress"));
    assert!(body.contains("new XMLHttpRequest()"));
    assert!(body.contains("X-Teatro-Transfer-Id"));
    assert!(body.contains("SERVER_JOB_STORAGE_KEY"));
    assert!(body.contains("/api/admin/gog-import/status"));
    assert!(body.contains("/api/admin/gog-imports"));
    assert!(body.contains("Review files"));
    assert!(body.contains("GOG setup import"));
    assert!(body.contains("job${active === 1 ? '' : 's'} in progress"));
    assert!(body.contains("Upload games"));
    assert!(body.contains("Import installer"));
    assert!(body.contains("Track it in Jobs"));
    assert!(body.contains("Follow progress in Jobs"));
    assert!(body.contains("validateGogSetupSelection"));
    assert!(body.contains("suggestGogTitleFromFileName"));
    assert!(body.contains("suggestGogTitleFromSelection"));
    assert!(body.contains("Suggested from the filename"));
    assert!(body.contains("gog-import-title-hint"));
    assert!(body.contains("next_event_seq"));
    assert!(body.contains("renderGogImportProgress"));
    assert!(body.contains("data-cancel-job"));
    assert!(body.contains("Cancel</button>"));
    assert!(body.contains("renderUploadPlanPanel"));
    assert!(body.contains("runUploadJob"));
    assert!(body.contains("renderJobs"));
    assert!(body.contains("state.jobs"));
    assert!(body.contains("planned_title"));
    assert!(body.contains("applyPlannedTitle"));
    assert!(!body.contains("uploadSeparateFiles"));
    assert!(!body.contains("upload_mode"));
    assert!(!body.contains("Separate ROMs"));
    assert!(!body.contains("Title override"));
    assert!(body.contains("uploadSelectionLabel"));
    assert!(body.contains("appendSelectedFiles"));
    assert!(body.contains("removeSelectedFile"));
    assert!(body.contains("clearSelectedFiles"));
    assert!(body.contains("data-remove-upload-file"));
    assert!(body.contains("clear-upload-list"));
    assert!(body.contains("detail-media"));
    assert!(body.contains("detail-metadata"));
    assert!(body.contains("<h3>Files</h3>"));
    assert!(body.contains("data-download-package"));
    assert!(body.contains("archive-ticket"));
    assert!(body.contains("submitDownloadTicket"));
    assert!(body.contains("renderRomFileGroups"));
    assert!(body.contains("Edit details"));
    assert!(body.contains("renderRomEditForm"));
    assert!(body.contains("open-rom-edit"));
    assert!(body.contains("missing_cover"));
    assert!(body.contains("Missing cover"));
    assert!(body.contains("/cover"));
    assert!(body.contains("image/jpeg,image/png,image/webp"));
    assert!(body.contains("Client ID is set."));
    assert!(!body.contains("Client ID <code>"));
    assert!(body.contains("Clear credentials"));
    assert!(body.contains("onSettingsClear"));
    assert!(body.contains("/api/admin/igdb/settings"));
    assert!(body.contains("<h1>Settings</h1>"));
    assert!(body.contains("Delete library content"));
    assert!(body.contains("onPlatformDelete"));
    assert!(body.contains("onClearAll"));
    assert!(body.contains("/api/admin/platforms/"));
    assert!(body.contains("/api/admin/roms?confirm="));
    assert!(!body.contains("Selected ROM"));
    assert!(body.contains("uploadWithProgress"));
    assert!(body.contains("platformQuery"));
    assert!(body.contains("/api/admin/igdb/search"));

    for uri in [
        "/admin/favicon.svg",
        "/admin/favicon.svg?icon=teatro-controller",
    ] {
        let response = app.clone().oneshot(empty_request(uri)).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), include_bytes!("../web/admin/favicon.svg"));
    }

    let response = app
        .oneshot(empty_request("/admin/game-cover-placeholder.jpg"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert!(bytes.starts_with(b"\xff\xd8\xff"));
}

/// Every shipped admin module must be reachable through the serving allowlist.
///
/// A module that exists on disk but is missing from the allowlist 404s in the browser, which
/// breaks the whole ES module graph and leaves the admin UI stuck on its loading screen. The
/// enumerated lists above cannot catch that, because a forgotten module is also forgotten there.
#[tokio::test]
async fn every_admin_javascript_module_on_disk_is_served() {
    let test_app = support::TestApp::new().await;
    let app = test_app.router;
    let admin_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/admin");

    let mut directories = vec![admin_root.clone()];
    let mut modules = Vec::new();
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                directories.push(path);
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            // Test modules and package manifests are never served to the browser.
            if !name.ends_with(".js") || name.ends_with(".test.js") {
                continue;
            }
            modules.push(
                path.strip_prefix(&admin_root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    modules.sort();
    assert!(modules.len() > 20, "expected the admin module set on disk");

    for module in modules {
        let response = app
            .clone()
            .oneshot(empty_request(&format!("/admin/{module}")))
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "/admin/{module} is on disk but is not in the serving allowlist"
        );
    }
}

fn empty_request(uri: &str) -> axum::http::Request<Body> {
    support::empty_request("GET", uri, None)
}
