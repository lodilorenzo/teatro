use crate::{
    domain::{api_token::ApiToken, integrity::IntegrityJob, user::User},
    repositories::audit as audit_repository,
    services::{
        gog_import::GogImportOutcome,
        igdb::AppliedIgdbMetadata,
        integrity::DatImportOutcome,
        library::{LibraryScanResult, UploadedBatch, UploadedRom},
        romm_source::RommSourceStatus,
    },
    state::AppState,
};

use super::dto::IgdbSettingsResponse;

pub(super) async fn record_user_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    action: &'static str,
    user: &User,
) {
    let metadata_json = serde_json::json!({
        "username": user.username,
        "role": user.role.as_str(),
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        action,
        Some("user"),
        Some(user.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_api_token_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    action: &'static str,
    token: &ApiToken,
) {
    let metadata_json = serde_json::json!({
        "token_id": token.id,
        "token_name": token.name,
        "token_prefix": token.token_prefix,
        "username": token.user.username,
        "user_id": token.user.id,
        "scopes": token.scopes,
        "expires_at": token.expires_at,
        "revoked_at": token.revoked_at,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        action,
        Some("api_token"),
        Some(token.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_upload_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    uploaded: &UploadedRom,
) {
    let metadata_json = serde_json::json!({
        "rom_name": uploaded.rom.name,
        "platform_slug": uploaded.rom.platform_slug,
        "file_id": uploaded.file_id,
        "file_name": uploaded.file_name,
        "relative_path": uploaded.relative_path,
        "file_size_bytes": uploaded.file_size_bytes,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.uploaded",
        Some("rom"),
        Some(uploaded.rom.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_gog_import_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    outcome: &GogImportOutcome,
) {
    // GOG setup titles and generated filenames can disclose a private purchase. Keep this
    // success event useful for operations without copying either name or any tool transcript,
    // setup path, or staging identifier into durable audit metadata.
    let metadata_json = serde_json::json!({
        "platform_slug": outcome.uploaded.rom.platform_slug,
        "file_id": outcome.uploaded.file_id,
        "input_file_count": outcome.summary.input_file_count,
        "input_bytes": outcome.summary.input_bytes,
        "extracted_file_count": outcome.summary.extracted_file_count,
        "extracted_bytes": outcome.summary.extracted_bytes,
        "launcher_count": outcome.summary.launcher_count,
        "archive_bytes": outcome.summary.archive_bytes,
        "extractor_version": outcome.summary.extractor_version,
        "data_version": outcome.summary.data_version,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.gog_setup_imported",
        Some("rom"),
        Some(outcome.uploaded.rom.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_library_scan_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    result: &LibraryScanResult,
) {
    let metadata_json = serde_json::json!({
        "scanned_file_count": result.scanned_file_count,
        "already_indexed_file_count": result.already_indexed_file_count,
        "imported_rom_count": result.imported_rom_count,
        "imported_file_count": result.imported_file_count,
        "generated_manifest_count": result.generated_manifest_count,
        "cover_downloaded_count": result.cover_downloaded_count,
        "cover_not_downloaded_count": result.cover_not_downloaded_count,
        "not_imported_file_count": result.not_imported_file_count,
        "reason_codes": crate::services::library::scan_reason_codes(result),
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.library_scanned",
        Some("library_root"),
        None,
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_batch_upload_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    uploaded: &UploadedBatch,
) {
    let metadata_json = serde_json::json!({
        "rom_count": uploaded.roms.len(),
        "roms": uploaded.roms.iter().map(|rom| serde_json::json!({
            "rom_id": rom.id,
            "rom_name": rom.name,
            "platform_slug": rom.platform_slug,
            "file_count": rom.files.len(),
        })).collect::<Vec<_>>(),
        "warnings": uploaded.warnings,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.batch_uploaded",
        Some("rom"),
        None,
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_update_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    rom: &crate::domain::rom::Rom,
) {
    let metadata_json = serde_json::json!({
        "rom_name": rom.name,
        "platform_slug": rom.platform_slug,
        "regions": rom.regions,
        "metadata_source": rom.metadata.get("source"),
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.updated",
        Some("rom"),
        Some(rom.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_igdb_settings_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    response: &IgdbSettingsResponse,
    event_name: &'static str,
) {
    let metadata_json = serde_json::json!({
        "configured": response.configured,
        "client_id_configured": response.client_id_configured,
        "client_secret_configured": response.client_secret_configured,
        "client_id_source": response.client_id_source,
        "client_secret_source": response.client_secret_source,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        event_name,
        Some("igdb_settings"),
        None,
        Some(metadata_json),
    )
    .await;
}

/// Records a RomM source lifecycle event. The base URL, username, and secret are deliberately
/// absent: an audit row must never carry a credential or a credential-bearing endpoint.
pub(super) async fn record_romm_source_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    status: &RommSourceStatus,
    event_name: &'static str,
) {
    let metadata_json = serde_json::json!({
        "configured": status.configured,
        "secret_configured": status.secret_configured,
        "auth_mode": status.auth_mode,
        "plaintext_http": status.plaintext_http,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        event_name,
        Some("romm_source"),
        None,
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_igdb_metadata_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    outcome: &AppliedIgdbMetadata,
) {
    let metadata_json = serde_json::json!({
        "rom_name": outcome.rom.name,
        "platform_slug": outcome.rom.platform_slug,
        "igdb_id": outcome.rom.metadata.get("igdb_id"),
        "cached_covers": outcome.cached_covers,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "roms.metadata_igdb_applied",
        Some("rom"),
        Some(outcome.rom.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_dat_import_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    outcome: &DatImportOutcome,
) {
    let metadata_json = serde_json::json!({
        "source_name": outcome.source.name,
        "source_version": outcome.source.version,
        "imported_file_name": outcome.source.imported_file_name,
        "imported_entries": outcome.imported_entries,
        "skipped_entries": outcome.skipped_entries,
        "already_imported": outcome.already_imported,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "integrity.dat_imported",
        Some("dat_source"),
        Some(outcome.source.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_integrity_job_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    job: &IntegrityJob,
) {
    let metadata_json = serde_json::json!({
        "rom_id": job.rom_id,
        "force": job.force,
        "total_files": job.total_files,
    })
    .to_string();

    record_audit_event(
        state,
        actor_user_id,
        "integrity.job_started",
        Some("integrity_job"),
        Some(job.id),
        Some(metadata_json),
    )
    .await;
}

pub(super) async fn record_audit_event(
    state: &AppState,
    actor_user_id: Option<i64>,
    action: &'static str,
    entity_type: Option<&'static str>,
    entity_id: Option<i64>,
    metadata_json: Option<String>,
) {
    if let Err(error) = audit_repository::record(
        state.db(),
        audit_repository::AuditEvent {
            actor_user_id,
            action,
            entity_type,
            entity_id,
            metadata_json: metadata_json.as_deref(),
        },
    )
    .await
    {
        tracing::warn!(?error, "failed to record admin audit event");
    }
}
