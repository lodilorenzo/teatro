//! Public, non-privileged library and host telemetry for the storefront hero.

use std::path::Path;

use axum::{Json, extract::State};
use serde::Serialize;

use crate::{error::ApiError, state::AppState};

/// Coarse, non-sensitive runtime figures any authenticated viewer may see.
#[derive(Debug, Serialize)]
pub struct PublicStatsResponse {
    pub storage_used_bytes: u64,
    pub storage_library_bytes: u64,
    pub storage_other_bytes: u64,
    pub storage_free_bytes: u64,
    pub storage_total_bytes: u64,
    pub uptime_seconds: i64,
}

pub async fn public_stats(
    State(state): State<AppState>,
) -> Result<Json<PublicStatsResponse>, ApiError> {
    let library_root = state.config().default_library_root.clone();
    let usage = tokio::task::spawn_blocking(move || disk_usage(&library_root))
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let indexed_library_bytes: i64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(file_size_bytes), 0) FROM rom_files")
            .fetch_one(state.db())
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;
    // ponytail: one bar assumes indexed roots share this filesystem; add per-root bars for multi-disk reporting.
    let library_bytes = u64::try_from(indexed_library_bytes)
        .unwrap_or(0)
        .min(usage.used);

    let uptime_seconds = (chrono::Utc::now() - state.started_at())
        .num_seconds()
        .max(0);

    Ok(Json(PublicStatsResponse {
        storage_used_bytes: usage.used,
        storage_library_bytes: library_bytes,
        storage_other_bytes: usage.used.saturating_sub(library_bytes),
        storage_free_bytes: usage.free,
        storage_total_bytes: usage.total,
        uptime_seconds,
    }))
}

struct DiskUsage {
    used: u64,
    free: u64,
    total: u64,
}

#[cfg(unix)]
fn disk_usage(path: &Path) -> std::io::Result<DiskUsage> {
    let stat = nix::sys::statvfs::statvfs(path).map_err(std::io::Error::other)?;
    let frsize = stat.fragment_size() as u64;
    let total = stat.blocks() as u64 * frsize;
    // Blocks available to unprivileged callers; matches what `df` reports as free.
    let free = stat.blocks_available() as u64 * frsize;
    let used = total.saturating_sub(free);
    Ok(DiskUsage { used, free, total })
}

#[cfg(not(unix))]
fn disk_usage(_path: &Path) -> std::io::Result<DiskUsage> {
    Ok(DiskUsage {
        used: 0,
        free: 0,
        total: 0,
    })
}
