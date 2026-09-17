use axum::{
    Json,
    extract::State,
    http::{HeaderName, HeaderValue, StatusCode, header::LOCATION},
};
use tracing::{Instrument, instrument::WithSubscriber};

use crate::{
    api::{
        auth::AdminUser,
        extractors::{ApiJson, ApiPath},
    },
    error::ApiError,
    services::library::{
        self, LibraryScanJobError, LibraryScanJobReporter, SidecarCleanupPreview,
        SidecarCleanupResult,
    },
    state::AppState,
};

use super::{
    audit::record_library_scan_event,
    dto::{LibraryScanJobCreateResponse, LibraryScanJobStatusResponse, SidecarCleanupRequest},
    errors::{map_library_error, map_library_scan_job_registry_error},
};

pub async fn preview_sidecar_cleanup(
    _actor: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<SidecarCleanupPreview>, ApiError> {
    Ok(Json(
        library::preview_sidecar_cleanup(&state)
            .await
            .map_err(map_library_error)?,
    ))
}

pub async fn cleanup_sidecars(
    actor: AdminUser,
    State(state): State<AppState>,
    ApiJson(request): ApiJson<SidecarCleanupRequest>,
) -> Result<Json<SidecarCleanupResult>, ApiError> {
    if request.confirm != "DELETE SIDECARS" {
        return Err(ApiError::bad_request(
            "confirm must exactly match DELETE SIDECARS",
        ));
    }
    Ok(Json(
        library::cleanup_sidecars(&state, request.files, Some(actor.public_user().id))
            .await
            .map_err(map_library_error)?,
    ))
}

pub async fn create_library_scan(
    actor: AdminUser,
    State(state): State<AppState>,
) -> Result<
    (
        StatusCode,
        [(HeaderName, HeaderValue); 1],
        Json<LibraryScanJobCreateResponse>,
    ),
    ApiError,
> {
    let (snapshot, reporter) = state.library_scan_jobs().reserve();
    let status_url = format!("/api/admin/library/scans/{}", snapshot.id);
    let location = HeaderValue::from_str(&status_url)
        .expect("generated scan status URLs contain only valid header characters");
    let response = LibraryScanJobCreateResponse::new(snapshot, status_url);
    spawn_library_scan_worker(state, actor.public_user().id, reporter);

    Ok((StatusCode::ACCEPTED, [(LOCATION, location)], Json(response)))
}

pub async fn cancel_library_scan_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<LibraryScanJobStatusResponse>, ApiError> {
    let snapshot = state
        .library_scan_jobs()
        .cancel(&id)
        .map_err(map_library_scan_job_registry_error)?;
    Ok(Json(LibraryScanJobStatusResponse::from(snapshot)))
}

pub async fn library_scan_job(
    _actor: AdminUser,
    State(state): State<AppState>,
    ApiPath(id): ApiPath<String>,
) -> Result<Json<LibraryScanJobStatusResponse>, ApiError> {
    let snapshot = state
        .library_scan_jobs()
        .snapshot(&id)
        .map_err(map_library_scan_job_registry_error)?;
    Ok(Json(LibraryScanJobStatusResponse::from(snapshot)))
}

fn spawn_library_scan_worker(state: AppState, actor_id: i64, reporter: LibraryScanJobReporter) {
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    tokio::spawn(
        supervise_library_scan_worker(state, actor_id, reporter)
            .instrument(tracing::info_span!("library_scan", actor_id))
            .with_subscriber(dispatch),
    );
}

async fn supervise_library_scan_worker(
    state: AppState,
    actor_id: i64,
    reporter: LibraryScanJobReporter,
) {
    match reporter.start() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::error!(?error, "failed to start admitted library scan job");
            let _ = reporter.fail(LibraryScanJobError::new(
                "internal_server_error",
                "The managed library scan could not be started.",
            ));
            return;
        }
    }

    let child_state = state.clone();
    let child_reporter = reporter.clone();
    let span = tracing::Span::current();
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    let workflow = tokio::spawn(
        async move { library::scan_library(&child_state, &child_reporter).await }
            .instrument(span)
            .with_subscriber(dispatch),
    );
    if let Err(error) = reporter.attach_abort_handle(workflow.abort_handle()) {
        tracing::error!(?error, "failed to attach library scan cancellation handle");
    }
    let completion = workflow.await;

    let terminal = match completion {
        Ok(Ok(result)) => {
            record_library_scan_event(&state, Some(actor_id), &result).await;
            reporter.succeed(result)
        }
        Ok(Err(error)) => reporter.fail(library::scan_failure_summary(&error)),
        Err(error) => {
            tracing::error!(
                worker_panicked = error.is_panic(),
                worker_cancelled = error.is_cancelled(),
                "supervised library scan task ended without a result"
            );
            reporter.fail(LibraryScanJobError::new(
                "internal_server_error",
                "The managed library scan failed.",
            ))
        }
    };
    if let Err(error) = terminal {
        tracing::error!(?error, "failed to record terminal library scan state");
    }
}
