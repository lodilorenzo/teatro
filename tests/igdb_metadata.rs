mod support;

use std::{
    fs,
    sync::{Arc, Mutex},
};
use support::{response_json, seed_user_record as seed_user};

use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use serde_json::{Value, json};
use teatro::{config::IgdbConfig, domain::user::UserRole};
use tokio::{net::TcpListener, task::JoinHandle};
use tower::ServiceExt;

// Scenario fragments intentionally share the mock-server fixture above.
include!("igdb_metadata/configuration.rs");
include!("igdb_metadata/metadata.rs");

#[derive(Clone, Default)]
struct MockIgdbState {
    requests: Arc<Mutex<Vec<String>>>,
    oversized_games: bool,
    wrong_platform: bool,
    empty_scoped: bool,
}

struct MockIgdb {
    base_url: String,
    requests: Arc<Mutex<Vec<String>>>,
    handle: JoinHandle<()>,
}

impl MockIgdb {
    async fn spawn() -> Self {
        Self::spawn_with_options(false, false, false).await
    }

    async fn spawn_with_oversized_games(oversized_games: bool) -> Self {
        Self::spawn_with_options(oversized_games, false, false).await
    }

    async fn spawn_with_wrong_platform() -> Self {
        Self::spawn_with_options(false, true, false).await
    }

    async fn spawn_with_scoped_miss() -> Self {
        Self::spawn_with_options(false, false, true).await
    }

    async fn spawn_with_options(
        oversized_games: bool,
        wrong_platform: bool,
        empty_scoped: bool,
    ) -> Self {
        let state = MockIgdbState {
            oversized_games,
            wrong_platform,
            empty_scoped,
            ..MockIgdbState::default()
        };
        let requests = state.requests.clone();
        let app = Router::new()
            .route("/oauth2/token", post(mock_token))
            .route("/v4/games", post(mock_games))
            .route("/images/t_cover_big/co123.jpg", get(large_cover))
            .route("/images/t_cover_small/co123.jpg", get(small_cover))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            base_url: format!("http://{address}"),
            requests,
            handle,
        }
    }

    fn config(&self) -> IgdbConfig {
        IgdbConfig {
            client_id: Some("test-client".to_string()),
            client_secret: Some("test-secret".to_string()),
            token_url: format!("{}/oauth2/token", self.base_url),
            api_url: format!("{}/v4", self.base_url),
            image_base_url: format!("{}/images", self.base_url),
        }
    }
}

impl Drop for MockIgdb {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

async fn mock_token() -> Json<Value> {
    Json(json!({
        "access_token": "mock-access-token",
        "expires_in": 3600,
        "token_type": "bearer"
    }))
}

async fn mock_games(
    State(state): State<MockIgdbState>,
    headers: HeaderMap,
    body: String,
) -> axum::response::Response {
    assert_eq!(headers["client-id"], "test-client");
    assert_eq!(headers[header::AUTHORIZATION], "Bearer mock-access-token");
    let empty_scoped = state.empty_scoped && body.contains("where platforms");
    state.requests.lock().unwrap().push(body);

    if state.oversized_games {
        return (StatusCode::OK, vec![b' '; 2 * 1024 * 1024 + 1]).into_response();
    }
    if empty_scoped {
        return Json(json!([])).into_response();
    }

    let platform = if state.wrong_platform {
        json!({ "id": 7, "name": "PlayStation", "abbreviation": "PS1", "slug": "ps" })
    } else {
        json!({ "id": 29, "name": "Sega Mega Drive/Genesis", "abbreviation": "Genesis", "slug": "sega-mega-drive-genesis" })
    };
    Json(json!([
        {
            "id": 123,
            "name": "Sonic The Hedgehog",
            "summary": "A fast platform game.",
            "first_release_date": 662688000,
            "genres": [{ "name": "Platform" }],
            "platforms": [platform],
            "involved_companies": [
                { "developer": true, "publisher": false, "company": { "name": "Sega" } },
                { "developer": false, "publisher": true, "company": { "name": "Sega" } }
            ],
            "cover": { "image_id": "co123" }
        }
    ]))
    .into_response()
}

async fn large_cover() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/jpeg")],
        b"large-cover".to_vec(),
    )
}

async fn small_cover() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/jpeg")],
        b"small-cover".to_vec(),
    )
}

fn empty_request(uri: &str, auth: Option<(&str, &str)>) -> Request<Body> {
    support::empty_request("GET", uri, auth)
}

fn json_request(uri: &str, auth: Option<(&str, &str)>, body: Value) -> Request<Body> {
    support::json_request("POST", uri, auth, body)
}

fn patch_json_request(uri: &str, auth: Option<(&str, &str)>, body: Value) -> Request<Body> {
    support::json_request("PATCH", uri, auth, body)
}

fn delete_request(uri: &str, auth: Option<(&str, &str)>) -> Request<Body> {
    support::empty_request("DELETE", uri, auth)
}
