use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
    pub started_at: DateTime<Utc>,
}

pub async fn healthz(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "teatro",
        version: env!("CARGO_PKG_VERSION"),
        started_at: state.started_at(),
    })
}
