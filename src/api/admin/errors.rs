use std::io;

use axum::http::StatusCode;

use crate::{
    error::ApiError,
    repositories::{api_tokens::ApiTokenRepositoryError, users::UserRepositoryError},
    services::{
        api_tokens::ApiTokenServiceError,
        gog_import::{GogImportError, GogImportJobErrorSummary, GogImportJobRegistryError},
        igdb::IgdbServiceError,
        integrity::IntegrityServiceError,
        library::{LibraryScanJobRegistryError, LibraryServiceError},
        romm_source::{RommImportJobRegistryError, RommSourceError},
    },
    storage::{file_store::FileStoreError, paths::PathSafetyError},
};

pub(in crate::api) fn map_user_error(error: UserRepositoryError) -> ApiError {
    let message = error.to_string();

    match error {
        UserRepositoryError::InvalidUsername { .. } | UserRepositoryError::InvalidRole(_) => {
            ApiError::bad_request(message)
        }
        UserRepositoryError::AlreadyExists(_) => ApiError::conflict(message),
        UserRepositoryError::NotFound => ApiError::not_found("user not found"),
        UserRepositoryError::LastAdmin => ApiError::forbidden(message),
        UserRepositoryError::Database(error) => {
            tracing::error!(?error, "user repository operation failed");
            ApiError::internal("user management failed")
        }
    }
}

pub(super) fn map_api_token_service_error(error: ApiTokenServiceError) -> ApiError {
    match error {
        ApiTokenServiceError::InvalidScope(_)
        | ApiTokenServiceError::EmptyName
        | ApiTokenServiceError::NameTooLong => ApiError::bad_request(error.to_string()),
        ApiTokenServiceError::AdminScopeRequiresAdminUser => ApiError::forbidden(error.to_string()),
    }
}

pub(super) fn map_api_token_repository_error(error: ApiTokenRepositoryError) -> ApiError {
    match error {
        ApiTokenRepositoryError::NotFound => ApiError::not_found("API token not found"),
        ApiTokenRepositoryError::InvalidRole(error) => {
            tracing::error!(?error, "invalid user role on API token row");
            ApiError::internal("API token operation failed")
        }
        ApiTokenRepositoryError::Json(error) => {
            tracing::error!(?error, "failed to serialize API token scopes");
            ApiError::internal("API token operation failed")
        }
        ApiTokenRepositoryError::InvalidScopes(error) => {
            tracing::error!(?error, "invalid persisted API token scopes");
            ApiError::internal("API token operation failed")
        }
        ApiTokenRepositoryError::Database(error) => {
            tracing::error!(?error, "API token repository operation failed");
            ApiError::internal("API token operation failed")
        }
    }
}

pub(super) fn map_database_error(error: sqlx::Error) -> ApiError {
    tracing::error!(?error, "admin repository operation failed");
    ApiError::internal("admin operation failed")
}

pub(super) fn map_library_error(error: LibraryServiceError) -> ApiError {
    match error {
        LibraryServiceError::MissingPlatform
        | LibraryServiceError::InvalidFileName
        | LibraryServiceError::InvalidTitle
        | LibraryServiceError::InvalidMetadataField
        | LibraryServiceError::PlatformMismatch
        | LibraryServiceError::UnsupportedSplitArchive
        | LibraryServiceError::EmptyBatch
        | LibraryServiceError::EmptySidecarCleanup
        | LibraryServiceError::SidecarCleanupTooLarge
        | LibraryServiceError::InvalidIngestPlan(_) => ApiError::bad_request(error.to_string()),
        LibraryServiceError::PlatformNotFound | LibraryServiceError::RomNotFound => {
            ApiError::not_found(error.to_string())
        }
        LibraryServiceError::ReadOnlyRoot
        | LibraryServiceError::RootDeleteRejected
        | LibraryServiceError::UnsafeDeleteTarget => ApiError::forbidden(error.to_string()),
        LibraryServiceError::UploadTooLarge => ApiError::payload_too_large(error.to_string()),
        LibraryServiceError::NoAvailableFileName
        | LibraryServiceError::SidecarCleanupPreviewChanged => {
            ApiError::conflict(error.to_string())
        }
        LibraryServiceError::PathSafety(error) => map_path_error(error),
        LibraryServiceError::FileStore(error) => map_file_store_error(error),
        LibraryServiceError::Io(error) => {
            tracing::error!(?error, "library filesystem operation failed");
            ApiError::internal("library operation failed")
        }
        LibraryServiceError::Database(error) => {
            tracing::error!(?error, "library repository operation failed");
            ApiError::internal("library operation failed")
        }
        LibraryServiceError::Json(error) => {
            tracing::error!(?error, "library persistence payload serialization failed");
            ApiError::internal("library operation failed")
        }
        LibraryServiceError::FileRecovery(error) => {
            tracing::error!(?error, "library file recovery failed");
            ApiError::internal("library operation failed")
        }
        LibraryServiceError::DefaultRootMissing => {
            tracing::error!("default library root is not registered");
            ApiError::internal("library root is not configured")
        }
    }
}

pub(super) fn map_gog_import_error(error: GogImportError) -> ApiError {
    match error {
        GogImportError::Disabled | GogImportError::NotConfigured => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            error.to_string(),
        ),
        GogImportError::ExtractorUnavailable | GogImportError::ExtractorHashMismatch => {
            tracing::error!(error = %error, "GOG import extractor configuration failed");
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                error.to_string(),
            )
        }
        GogImportError::WindowsPlatformUnavailable => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            error.to_string(),
        ),
        GogImportError::InvalidTitle
        | GogImportError::InvalidSetupFileName
        | GogImportError::InvalidSetupExecutableCount
        | GogImportError::DuplicateSetupFileName
        | GogImportError::EmptyInput => ApiError::bad_request(error.to_string()),
        GogImportError::InputTooLarge
        | GogImportError::ExtractedOutputTooLarge
        | GogImportError::TooManyExtractedEntries => ApiError::payload_too_large(error.to_string()),
        GogImportError::ReadOnlyRoot => ApiError::forbidden(error.to_string()),
        GogImportError::ExtractorTimedOut { .. } => ApiError::new(
            StatusCode::GATEWAY_TIMEOUT,
            "gateway_timeout",
            error.to_string(),
        ),
        GogImportError::ExtractorRejected { .. }
        | GogImportError::ExtractorDiagnostics { .. }
        | GogImportError::MissingDataVersion
        | GogImportError::EmptyExtractedOutput
        | GogImportError::MissingLaunchCandidate
        | GogImportError::UnsafeExtractedOutput
        | GogImportError::CaseCollidingExtractedPaths
        | GogImportError::ExtractedOutputChanged
        | GogImportError::SiparioArchiveIncompatible => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unprocessable_entity",
            error.to_string(),
        ),
        GogImportError::InsufficientStorage => ApiError::insufficient_storage(error.to_string()),
        GogImportError::Library(error) => map_library_error(error),
        GogImportError::ArchiveStaging
        | GogImportError::WorkerFailed
        | GogImportError::Database(_)
        | GogImportError::Io(_)
        | GogImportError::Zip(_) => {
            tracing::error!(?error, "GOG import workflow failed");
            ApiError::internal("GOG setup import failed")
        }
    }
}

pub(super) fn summarize_gog_import_error(error: GogImportError) -> GogImportJobErrorSummary {
    let error = map_gog_import_error(error);
    GogImportJobErrorSummary::new(error.code(), error.message())
}

pub(super) fn generic_gog_import_worker_error() -> GogImportJobErrorSummary {
    GogImportJobErrorSummary::new("internal_server_error", "GOG setup import failed")
}

pub(super) fn map_gog_import_job_registry_error(error: GogImportJobRegistryError) -> ApiError {
    match error {
        GogImportJobRegistryError::NotFound => ApiError::not_found("GOG import job not found"),
        GogImportJobRegistryError::FutureCursor { .. } => {
            ApiError::bad_request("after cursor cannot be newer than the job event sequence")
        }
        GogImportJobRegistryError::TooLateToCancel => {
            ApiError::conflict("GOG import is already publishing and can no longer be cancelled")
        }
        error => {
            tracing::error!(error = %error, "GOG import job registry operation failed");
            ApiError::internal("GOG import job operation failed")
        }
    }
}

pub(super) fn map_library_scan_job_registry_error(error: LibraryScanJobRegistryError) -> ApiError {
    match error {
        LibraryScanJobRegistryError::NotFound => ApiError::not_found("library scan job not found"),
        error => {
            tracing::error!(error = %error, "library scan job registry operation failed");
            ApiError::internal("library scan job operation failed")
        }
    }
}

pub(super) fn map_romm_source_error(error: RommSourceError) -> ApiError {
    let message = error.to_string();
    match error {
        // A disabled feature must look like a route that does not exist.
        RommSourceError::Disabled => ApiError::not_found("route not found"),
        RommSourceError::NotConfigured => ApiError::conflict(message),
        RommSourceError::InvalidBaseUrl
        | RommSourceError::NoFilesSelected
        | RommSourceError::UnknownRemoteFile
        | RommSourceError::TooManyFiles
        | RommSourceError::ImportTooLarge => ApiError::bad_request(message),
        RommSourceError::Unauthorized => ApiError::bad_gateway(message),
        RommSourceError::RemoteRomNotFound => ApiError::not_found(message),
        RommSourceError::PlatformNotFound => ApiError::not_found(message),
        RommSourceError::InsufficientStorage => ApiError::insufficient_storage(message),
        RommSourceError::Unreachable
        | RommSourceError::UpstreamStatus { .. }
        | RommSourceError::MalformedResponse
        | RommSourceError::ResponseTooLarge
        | RommSourceError::RedirectRefused
        | RommSourceError::SizeMismatch
        | RommSourceError::TimedOut => ApiError::bad_gateway(message),
        RommSourceError::Library(error) => map_library_error(error),
        RommSourceError::Staging => ApiError::internal("RomM import staging failed"),
        RommSourceError::Database(error) => {
            tracing::error!(?error, "RomM source repository operation failed");
            ApiError::internal("RomM source operation failed")
        }
        RommSourceError::Io(error) => {
            tracing::error!(?error, "RomM source filesystem operation failed");
            ApiError::internal("RomM source operation failed")
        }
    }
}

pub(super) fn map_romm_import_job_registry_error(error: RommImportJobRegistryError) -> ApiError {
    match error {
        RommImportJobRegistryError::NotFound => ApiError::not_found("RomM import job not found"),
        error => {
            tracing::error!(error = %error, "RomM import job registry operation failed");
            ApiError::internal("RomM import job operation failed")
        }
    }
}

pub(super) fn map_integrity_error(error: IntegrityServiceError) -> ApiError {
    match error {
        IntegrityServiceError::RomNotFound | IntegrityServiceError::JobNotFound => {
            ApiError::not_found(error.to_string())
        }
        IntegrityServiceError::InvalidDatFileName
        | IntegrityServiceError::InvalidDatEncoding
        | IntegrityServiceError::DatParse(_) => ApiError::bad_request(error.to_string()),
        IntegrityServiceError::DatTooLarge => ApiError::payload_too_large(error.to_string()),
        IntegrityServiceError::FileStore(error) => map_file_store_error(error),
        IntegrityServiceError::Io(error) => {
            tracing::error!(?error, "integrity filesystem operation failed");
            ApiError::internal("integrity operation failed")
        }
        IntegrityServiceError::Database(error) => {
            tracing::error!(?error, "integrity repository operation failed");
            ApiError::internal("integrity operation failed")
        }
        IntegrityServiceError::Json(error) => {
            tracing::error!(?error, "integrity metadata serialization failed");
            ApiError::internal("integrity operation failed")
        }
    }
}

pub(super) fn map_igdb_error(error: IgdbServiceError) -> ApiError {
    match error {
        IgdbServiceError::NotConfigured => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "service_unavailable",
            "IGDB credentials are not configured",
        ),
        IgdbServiceError::EmptyQuery
        | IgdbServiceError::InvalidLimit { .. }
        | IgdbServiceError::InvalidSelectedMatch(_)
        | IgdbServiceError::InvalidCoverImageId
        | IgdbServiceError::InvalidCoverImage => ApiError::bad_request(error.to_string()),
        IgdbServiceError::RomNotFound => ApiError::not_found("ROM not found"),
        IgdbServiceError::UpstreamStatus { .. } => {
            tracing::warn!(?error, "IGDB upstream returned an error");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "bad_gateway",
                "IGDB upstream request failed",
            )
        }
        IgdbServiceError::Url { .. } => {
            tracing::error!(?error, "IGDB URL configuration is invalid");
            ApiError::internal("IGDB configuration is invalid")
        }
        IgdbServiceError::Request { .. }
        | IgdbServiceError::Json { .. }
        | IgdbServiceError::ResponseTooLarge { .. }
        | IgdbServiceError::MissingAccessToken => {
            tracing::error!(?error, "IGDB request failed");
            ApiError::new(
                StatusCode::BAD_GATEWAY,
                "bad_gateway",
                "IGDB upstream request failed",
            )
        }
        IgdbServiceError::CoverTooLarge => ApiError::payload_too_large(error.to_string()),
        IgdbServiceError::CoverRecovery(error) => {
            tracing::error!(?error, "IGDB cover recovery failed");
            ApiError::internal("IGDB metadata operation failed")
        }
        IgdbServiceError::PathSafety(error) => map_path_error(error),
        IgdbServiceError::FileStore(error) => map_file_store_error(error),
        IgdbServiceError::Io(error) => {
            tracing::error!(?error, "IGDB asset filesystem operation failed");
            ApiError::internal("IGDB metadata operation failed")
        }
        IgdbServiceError::Database(error) => {
            tracing::error!(?error, "IGDB metadata repository operation failed");
            ApiError::internal("IGDB metadata operation failed")
        }
    }
}

pub(super) fn map_file_store_error(error: FileStoreError) -> ApiError {
    match error {
        FileStoreError::Path(error) => map_path_error(error),
        FileStoreError::AlreadyExists => ApiError::conflict(error.to_string()),
        FileStoreError::NotRegularFile | FileStoreError::NotDirectory => {
            ApiError::forbidden(error.to_string())
        }
        FileStoreError::Io(error) => {
            tracing::error!(?error, "managed filesystem operation failed");
            ApiError::internal("managed filesystem operation failed")
        }
    }
}

pub(super) fn map_path_error(error: PathSafetyError) -> ApiError {
    match error {
        PathSafetyError::Io(error) if error.kind() == io::ErrorKind::NotFound => {
            ApiError::not_found("file not found")
        }
        PathSafetyError::Io(error) => {
            tracing::error!(?error, "failed to resolve library path");
            ApiError::internal("failed to resolve library path")
        }
        PathSafetyError::Empty
        | PathSafetyError::Absolute
        | PathSafetyError::ParentTraversal
        | PathSafetyError::EscapesRoot
        | PathSafetyError::Symlink => ApiError::forbidden("path is outside configured root"),
    }
}

pub(super) fn bytes_error(error: axum::extract::rejection::BytesRejection) -> ApiError {
    tracing::debug!(?error, "invalid request body");
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        ApiError::payload_too_large("cover image is too large")
    } else {
        ApiError::bad_request("invalid request body")
    }
}

pub(super) fn multipart_error(error: axum::extract::multipart::MultipartError) -> ApiError {
    tracing::debug!(?error, "invalid multipart upload request");
    ApiError::bad_request("invalid multipart upload request")
}

#[cfg(test)]
mod tests {
    use crate::api::admin::query::{DeleteRomQuery, GogImportJobQuery, IgdbSearchQuery};

    #[test]
    fn parses_gog_import_job_cursor_strictly() {
        assert_eq!(GogImportJobQuery::parse(None).unwrap().after, 0);
        assert_eq!(
            GogImportJobQuery::parse(Some("after=42")).unwrap().after,
            42
        );
        assert!(GogImportJobQuery::parse(Some("after=")).is_err());
        assert!(GogImportJobQuery::parse(Some("after=-1")).is_err());
        assert!(GogImportJobQuery::parse(Some("after=+1")).is_err());
        assert!(GogImportJobQuery::parse(Some("after=1&after=2")).is_err());
        assert!(GogImportJobQuery::parse(Some("after=18446744073709551616")).is_err());
    }

    #[test]
    fn parses_delete_files_query() {
        assert!(DeleteRomQuery::parse(None).unwrap().delete_files);
        assert!(
            DeleteRomQuery::parse(Some("delete_files=true"))
                .unwrap()
                .delete_files
        );
        assert!(DeleteRomQuery::parse(Some("delete_files=false")).is_err());
        assert!(DeleteRomQuery::parse(Some("delete_files=wat")).is_err());
    }

    #[test]
    fn parses_igdb_search_query() {
        let query = IgdbSearchQuery::parse(Some(
            "q=Sonic&limit=5&platform=genesis&require_platform_match=true",
        ))
        .unwrap();

        assert_eq!(query.q, "Sonic");
        assert_eq!(query.limit, 5);
        assert_eq!(query.platform_slug.as_deref(), Some("genesis"));
        assert!(query.require_platform_match);
        assert!(IgdbSearchQuery::parse(Some("limit=5")).is_err());
        assert!(IgdbSearchQuery::parse(Some("q=Sonic&limit=0")).is_err());
        assert!(IgdbSearchQuery::parse(Some("q=Sonic&require_platform_match=maybe")).is_err());
    }
}
