use std::{path::PathBuf, time::Instant};

use axum::{
    Json,
    extract::{RawQuery, State},
    http::{HeaderName, HeaderValue, StatusCode, header::LOCATION},
};
use tokio::sync::OwnedSemaphorePermit;
use tracing::{Instrument, instrument::WithSubscriber};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiMultipart, ApiPath},
    },
    error::ApiError,
    services::gog_import::{
        self, GogImportError, GogImportJobId, GogImportJobReporter, GogImportJobUpdate,
        GogImportPhase, GogImportPhaseLog, GogImportStatus, GogImportWorkflowReporter,
    },
    state::AppState,
};

use super::{
    audit::record_gog_import_event,
    dto::{GogImportJobCreateResponse, GogImportJobStatusResponse},
    errors::{
        generic_gog_import_worker_error, map_gog_import_error, map_gog_import_job_registry_error,
        summarize_gog_import_error,
    },
    multipart::{discard_staged_upload, read_gog_import_multipart},
    query::GogImportJobQuery,
};

pub async fn gog_import_status(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Json<GogImportStatus> {
    Json(GogImportStatus::from_config(&state.config().gog_import))
}

pub(super) fn ensure_gog_import_available(state: &AppState) -> Result<(), ApiError> {
    if !state.config().gog_import.enabled {
        return Err(map_gog_import_error(GogImportError::Disabled));
    }
    if !state.config().gog_import.is_configured() {
        return Err(map_gog_import_error(GogImportError::NotConfigured));
    }
    Ok(())
}

pub async fn import_gog_setup(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiMultipart(multipart): ApiMultipart,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<GogImportJobCreateResponse>,
    ),
    ApiError,
> {
    ensure_gog_import_available(&state)?;

    let actor_id = actor.public_user().id;
    let upload_permit = state.acquire_upload_permit().await;
    let upload_started_at = Instant::now();
    tracing::info!(
        actor_id,
        phase = "upload_setup_files",
        status = "started",
        "GOG setup upload started"
    );
    let draft = match read_gog_import_multipart(&state, multipart).await {
        Ok(draft) => draft,
        Err(error) => {
            tracing::warn!(
                actor_id,
                phase = "upload_setup_files",
                status = "failed",
                elapsed_ms = elapsed_millis(upload_started_at),
                "GOG setup upload failed"
            );
            return Err(error);
        }
    };
    let import_id = draft
        .files
        .first()
        .map(|file| file.staging_operation_id.as_str())
        .unwrap_or("unassigned")
        .to_string();
    let input_file_count = draft.files.len();
    let input_bytes = draft
        .files
        .iter()
        .map(|file| file.file_size_bytes)
        .fold(0_u64, u64::saturating_add);
    tracing::info!(
        actor_id,
        import_id = %import_id,
        phase = "upload_setup_files",
        status = "completed",
        elapsed_ms = elapsed_millis(upload_started_at),
        input_file_count,
        input_bytes,
        "GOG setup upload completed; background job admission begins"
    );

    let staged_operations = draft
        .files
        .iter()
        .map(|file| (file.staged_path.clone(), file.staging_operation_id.clone()))
        .collect::<Vec<_>>();
    if let Err(error) = gog_import::validate_import_admission(state.config(), &draft) {
        cleanup_staged_operations(&state, &staged_operations).await;
        return Err(map_gog_import_error(error));
    }

    let reservation = state.gog_import_jobs().reserve();
    let status_url = format!(
        "/api/admin/gog-imports/{}",
        reservation.snapshot.id.as_str()
    );
    let location = HeaderValue::from_str(&status_url)
        .expect("generated GOG import status URLs contain only valid header characters");
    let response = GogImportJobCreateResponse::new(reservation.snapshot, status_url);

    spawn_gog_import_worker(
        state,
        actor_id,
        import_id,
        upload_permit,
        draft,
        staged_operations,
        reservation.reporter,
    );

    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(response)))
}

pub async fn cancel_gog_import_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<GogImportJobStatusResponse>, ApiError> {
    let id = id
        .parse::<GogImportJobId>()
        .map_err(|_| ApiError::not_found("GOG import job not found"))?;
    let snapshot = state
        .gog_import_jobs()
        .cancel(&id)
        .map_err(map_gog_import_job_registry_error)?;
    Ok(Json(GogImportJobStatusResponse::from(snapshot)))
}

pub async fn gog_import_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<GogImportJobStatusResponse>, ApiError> {
    let id = id
        .parse::<GogImportJobId>()
        .map_err(|_| ApiError::not_found("GOG import job not found"))?;
    let query = GogImportJobQuery::parse(raw_query.as_deref())?;
    let snapshot = state
        .gog_import_jobs()
        .snapshot(&id, query.after)
        .map_err(map_gog_import_job_registry_error)?;
    Ok(Json(GogImportJobStatusResponse::from(snapshot)))
}

fn spawn_gog_import_worker(
    state: AppState,
    actor_id: i64,
    import_id: String,
    upload_permit: OwnedSemaphorePermit,
    draft: gog_import::GogImportDraft,
    staged_operations: Vec<(PathBuf, String)>,
    reporter: GogImportJobReporter,
) {
    let request_span = tracing::info_span!(
        "gog_import_request",
        actor_id,
        import_id = %import_id,
    );
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    tokio::spawn(
        supervise_gog_import_worker(
            state,
            actor_id,
            upload_permit,
            draft,
            staged_operations,
            reporter,
        )
        .instrument(request_span)
        .with_subscriber(dispatch),
    );
}

async fn supervise_gog_import_worker(
    state: AppState,
    actor_id: i64,
    upload_permit: OwnedSemaphorePermit,
    draft: gog_import::GogImportDraft,
    staged_operations: Vec<(PathBuf, String)>,
    reporter: GogImportJobReporter,
) {
    match reporter.start() {
        Ok(GogImportJobUpdate::Applied) => {}
        Ok(GogImportJobUpdate::IgnoredTerminal) => {
            cleanup_staged_operations_with_log(
                &state,
                &staged_operations,
                &GogImportWorkflowReporter::noop(),
            )
            .await;
            drop(upload_permit);
            return;
        }
        Ok(_) => {}
        Err(error) => {
            tracing::error!(
                error = %error,
                "failed to transition admitted GOG import job to running"
            );
            cleanup_staged_operations_with_log(
                &state,
                &staged_operations,
                &GogImportWorkflowReporter::noop(),
            )
            .await;
            drop(upload_permit);
            return;
        }
    }

    let workflow_reporter = GogImportWorkflowReporter::new(reporter.clone());
    let child_reporter = workflow_reporter.clone();
    let workflow_state = state.clone();
    let workflow_span = tracing::Span::current();
    let workflow_dispatch = tracing::dispatcher::get_default(Clone::clone);
    let workflow = tokio::spawn(
        async move {
            let result = gog_import::import_setup(&workflow_state, draft, child_reporter).await;
            if let Ok(outcome) = &result {
                record_gog_import_event(&workflow_state, Some(actor_id), outcome).await;
            }
            result
        }
        .instrument(workflow_span)
        .with_subscriber(workflow_dispatch),
    );

    if let Err(error) = reporter.attach_abort_handle(workflow.abort_handle()) {
        tracing::error!(%error, "failed to attach GOG import cancellation handle");
    }
    let completion = workflow.await;
    let noop_reporter = GogImportWorkflowReporter::noop();
    let cleanup_reporter = if matches!(&completion, Ok(Ok(_))) {
        &workflow_reporter
    } else {
        // Cleanup still runs and remains traced after a failure, but it must not replace the
        // operation that failed as the terminal job phase.
        &noop_reporter
    };
    cleanup_staged_operations_with_log(&state, &staged_operations, cleanup_reporter).await;
    record_terminal_completion(&reporter, completion);

    // The upload/import concurrency slot intentionally covers all extraction, packaging,
    // publication, audit, and setup-staging cleanup work rather than only the HTTP body.
    drop(upload_permit);
}

fn record_terminal_completion(
    reporter: &GogImportJobReporter,
    completion: Result<
        Result<gog_import::GogImportOutcome, GogImportError>,
        tokio::task::JoinError,
    >,
) {
    let terminal_update = match completion {
        Ok(Ok(outcome)) => reporter.succeed(outcome),
        Ok(Err(error)) => reporter.fail(summarize_gog_import_error(error)),
        Err(error) => {
            tracing::error!(
                worker_panicked = error.is_panic(),
                worker_cancelled = error.is_cancelled(),
                "supervised GOG import workflow task ended without a result"
            );
            reporter.fail(generic_gog_import_worker_error())
        }
    };
    if let Err(error) = terminal_update {
        tracing::error!(
            error = %error,
            "failed to record terminal GOG import job state"
        );
    }
}

async fn cleanup_staged_operations_with_log(
    state: &AppState,
    staged_operations: &[(PathBuf, String)],
    reporter: &GogImportWorkflowReporter,
) {
    let cleanup_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::CleanupUploadedSetupStaging);
    cleanup_staged_operations(state, staged_operations).await;
    cleanup_phase.complete();
}

async fn cleanup_staged_operations(state: &AppState, staged_operations: &[(PathBuf, String)]) {
    for (staged_path, operation_id) in staged_operations {
        discard_staged_upload(state, staged_path, operation_id).await;
    }
}

fn elapsed_millis(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::record_terminal_completion;
    use crate::services::gog_import::{GogImportError, GogImportJobRegistry, GogImportOutcome};

    #[tokio::test]
    async fn supervisor_converts_a_workflow_panic_to_a_terminal_failure() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id.clone();
        reservation.reporter.start().unwrap();

        let completion = tokio::spawn(async {
            panic!("synthetic supervised workflow panic");
            #[allow(unreachable_code)]
            Err::<GogImportOutcome, GogImportError>(GogImportError::WorkerFailed)
        })
        .await;
        record_terminal_completion(&reservation.reporter, completion);

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.state.as_str(), "failed");
        assert_eq!(snapshot.phase.as_str(), "queued");
        assert_eq!(snapshot.error.unwrap().code, "internal_server_error");
    }

    #[tokio::test]
    async fn supervisor_converts_a_cancelled_join_to_a_terminal_failure() {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id.clone();
        reservation.reporter.start().unwrap();

        let workflow = tokio::spawn(async {
            std::future::pending::<Result<GogImportOutcome, GogImportError>>().await
        });
        workflow.abort();
        let completion = workflow.await;
        assert!(completion.as_ref().unwrap_err().is_cancelled());
        record_terminal_completion(&reservation.reporter, completion);

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.state.as_str(), "failed");
        assert_eq!(snapshot.error.unwrap().code, "internal_server_error");
    }
}
