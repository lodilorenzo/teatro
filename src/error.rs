use std::{backtrace::Backtrace, borrow::Cow, io, path::PathBuf, sync::Arc};

use axum::{
    Json,
    http::{
        HeaderName, HeaderValue, StatusCode,
        header::{RETRY_AFTER, WWW_AUTHENTICATE},
    },
    response::IntoResponse,
};
use serde::Serialize;
use thiserror::Error;

use crate::{
    cli::CliError, repositories::users::UserRepositoryError, storage::file_store::FileStoreError,
};

#[derive(Debug, Error)]
pub enum AppError {
    #[error(transparent)]
    Config(#[from] ConfigError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Migration(#[from] sqlx::migrate::MigrateError),

    #[error(transparent)]
    PasswordHash(#[from] argon2::password_hash::Error),

    #[error(transparent)]
    UserRepository(#[from] UserRepositoryError),

    #[error(transparent)]
    Cli(#[from] CliError),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error("file operation reconciliation failed: {0}")]
    Reconciliation(String),

    #[error("another Teatro process owns this instance: {0}")]
    InstanceAlreadyRunning(PathBuf),

    #[error("instance lock path is a symlink: {0}")]
    UnsafeInstanceLock(PathBuf),

    #[error(
        "database migrations do not match this Teatro binary; stop the running server and rerun the command to migrate offline"
    )]
    OfflineMigrationRequired,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid value for {key}: {value:?} ({reason})")]
    InvalidValue {
        key: &'static str,
        value: String,
        reason: Cow<'static, str>,
    },

    #[error("failed to create directory {path}: {source}")]
    CreateDir {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl ConfigError {
    pub fn invalid_value(
        key: &'static str,
        value: impl Into<String>,
        reason: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self::InvalidValue {
            key,
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub fn create_dir(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::CreateDir {
            path: path.into(),
            source,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiError {
    status: StatusCode,
    code: Cow<'static, str>,
    message: Cow<'static, str>,
    headers: Vec<(HeaderName, HeaderValue)>,
    backtrace: Option<Arc<Backtrace>>,
}

impl ApiError {
    pub fn new(
        status: StatusCode,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        let backtrace = status
            .is_server_error()
            .then(|| Arc::new(Backtrace::force_capture()));

        Self {
            status,
            code: code.into(),
            message: message.into(),
            headers: Vec::new(),
            backtrace,
        }
    }

    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.push((name, value));
        self
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }

    pub(crate) fn message(&self) -> &str {
        &self.message
    }

    pub fn unauthorized(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message).with_header(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Teatro\", charset=\"UTF-8\""),
        )
    }

    pub fn bad_request(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    pub fn forbidden(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::FORBIDDEN, "forbidden", message)
    }

    pub fn conflict(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    pub fn payload_too_large(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large", message)
    }

    pub fn too_many_requests(
        message: impl Into<Cow<'static, str>>,
        retry_after_seconds: u64,
    ) -> Self {
        let retry_after = HeaderValue::from_str(&retry_after_seconds.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("1"));

        Self::new(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", message)
            .with_header(RETRY_AFTER, retry_after)
    }

    pub fn insufficient_storage(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(
            StatusCode::INSUFFICIENT_STORAGE,
            "insufficient_storage",
            message,
        )
    }

    /// An upstream server Teatro depends on failed, refused, or answered unusably.
    pub fn bad_gateway(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "bad_gateway", message)
    }

    pub fn not_found(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub fn internal(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            message,
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let Self {
            status,
            code,
            message,
            headers,
            backtrace,
        } = self;

        if status.is_server_error() {
            match backtrace.as_deref() {
                Some(backtrace) => tracing::error!(
                    status = status.as_u16(),
                    code = %code,
                    message = %message,
                    backtrace = %backtrace,
                    "API request failed"
                ),
                None => tracing::error!(
                    status = status.as_u16(),
                    code = %code,
                    message = %message,
                    "API request failed"
                ),
            }
        }

        let body = ErrorResponse {
            error: ErrorBody { code, message },
        };

        let mut response = (status, Json(body)).into_response();

        for (name, value) in headers {
            response.headers_mut().insert(name, value);
        }

        response
    }
}

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: ErrorBody,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    pub code: Cow<'static, str>,
    pub message: Cow<'static, str>,
}
