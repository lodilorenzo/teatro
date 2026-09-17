//! Outbound RomM source: bounded browse proxying and explicit per-game import.
//!
//! Teatro is an authenticated HTTP *client* here. This is the opposite direction from
//! [`crate::api::romm`], which serves ROMM-compatible clients.
//!
//! The remote server is an untrusted network peer: its catalog is untrusted input, its bytes are
//! untrusted content, and its credentials are Teatro-held secrets. Every response is bounded and
//! normalized into Teatro-shaped values at this boundary, so a RomM upgrade can never change
//! Teatro's admin contract and a hostile server can never exhaust memory, disk, or the registry.

mod catalog;
mod client;
mod duplicates;
mod import;
mod jobs;

use std::io;

use serde_json::Value;
use thiserror::Error;

pub(crate) use catalog::{cached_platforms, cached_roms, refresh_catalog};
pub(crate) use client::{RommClient, RommConnection, is_plaintext_base_url, normalized_base_url};
pub(crate) use import::{RommImportOutcome, RommImportRequest, import_failure_summary, import_rom};
pub(crate) use jobs::{
    RommImportJobProgress, RommImportJobRegistry, RommImportJobRegistryError,
    RommImportJobReporter, RommImportJobSnapshot,
};

use crate::{repositories::romm_source::RommAuthMode, services::library::LibraryServiceError};

/// Largest JSON body accepted from the remote server for browse and metadata calls.
pub(crate) const MAX_JSON_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Largest body retained from a failed upstream request, for the normalized error message.
pub(crate) const MAX_ERROR_RESPONSE_BYTES: usize = 4 * 1024;
/// Largest proxied cover image. Covers are never written to `data/assets`.
pub(crate) const MAX_COVER_BYTES: usize = 4 * 1024 * 1024;
/// Largest browse page Teatro will request or return.
pub(crate) const MAX_BROWSE_LIMIT: u32 = 48;
pub(crate) const DEFAULT_BROWSE_LIMIT: u32 = 24;
/// Largest remote platform list Teatro will retain.
pub(crate) const MAX_REMOTE_PLATFORMS: usize = 512;
/// Bound applied to every remote string Teatro renders or stores.
pub(crate) const MAX_REMOTE_TEXT_BYTES: usize = 512;

/// Administrative view of the configured source. Never carries the secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RommSourceStatus {
    pub enabled: bool,
    pub configured: bool,
    pub base_url: Option<String>,
    pub username: Option<String>,
    pub auth_mode: &'static str,
    pub secret_configured: bool,
    pub plaintext_http: bool,
    pub updated_at: Option<String>,
    pub index_refreshed_at: Option<String>,
    pub index_game_count: i64,
    pub index_platform_count: i64,
}

impl RommSourceStatus {
    pub(crate) fn disabled() -> Self {
        Self {
            enabled: false,
            configured: false,
            base_url: None,
            username: None,
            auth_mode: RommAuthMode::Token.as_str(),
            secret_configured: false,
            plaintext_http: false,
            updated_at: None,
            index_refreshed_at: None,
            index_game_count: 0,
            index_platform_count: 0,
        }
    }
}

/// One populated platform on the remote server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemotePlatform {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub rom_count: i64,
}

/// One remote game as shown in the browse grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRom {
    pub id: i64,
    pub name: String,
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub platform_name: Option<String>,
    pub fs_name: Option<String>,
    pub file_size_bytes: Option<u64>,
    pub has_cover: bool,
    pub file_count: usize,
}

/// One physical remote file inside a remote game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRomFile {
    pub id: i64,
    pub file_name: String,
    pub file_size_bytes: Option<u64>,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

impl RemoteRomFile {
    /// Which hash algorithms this remote entry actually populated.
    pub(crate) fn hash_signals(&self) -> Vec<&'static str> {
        let mut signals = Vec::new();
        if self.sha256.is_some() {
            signals.push("sha256");
        }
        if self.sha1.is_some() {
            signals.push("sha1");
        }
        if self.md5.is_some() {
            signals.push("md5");
        }
        if self.crc32.is_some() {
            signals.push("crc32");
        }
        signals
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteRomDetail {
    pub rom: RemoteRom,
    pub files: Vec<RemoteRomFile>,
    pub metadata: RemoteRomMetadata,
}

/// Bounded RomM metadata normalized at the untrusted-server boundary.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RemoteRomMetadata {
    pub summary: Option<String>,
    pub regions: Vec<String>,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteRomPage {
    pub items: Vec<RemoteRom>,
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
    /// Number of upstream entries in this page, before malformed entries are discarded.
    pub(crate) returned: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteCover {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

#[derive(Debug, Error)]
pub(crate) enum RommSourceError {
    #[error("the RomM source feature is disabled")]
    Disabled,
    #[error("no RomM source is configured")]
    NotConfigured,
    #[error("the RomM base URL must be an absolute http or https URL")]
    InvalidBaseUrl,
    #[error("the RomM server rejected the stored credentials")]
    Unauthorized,
    #[error("the RomM server could not be reached")]
    Unreachable,
    #[error("the RomM server returned an unexpected status {status}")]
    UpstreamStatus { status: u16 },
    #[error("the RomM server returned a response Teatro could not understand")]
    MalformedResponse,
    #[error("the RomM server returned more data than Teatro accepts")]
    ResponseTooLarge,
    #[error("the RomM server followed a redirect, which Teatro does not allow")]
    RedirectRefused,
    #[error("the remote game was not found on the RomM server")]
    RemoteRomNotFound,
    #[error("the requested remote file is not part of the remote game")]
    UnknownRemoteFile,
    #[error("no files were selected for import")]
    NoFilesSelected,
    #[error("the remote game declares more files than Teatro imports at once")]
    TooManyFiles,
    #[error("the remote game declares more bytes than Teatro imports at once")]
    ImportTooLarge,
    #[error("the target Teatro platform could not be resolved")]
    PlatformNotFound,
    #[error("a downloaded file did not match the size the RomM server declared")]
    SizeMismatch,
    #[error("not enough disk space for the RomM import")]
    InsufficientStorage,
    #[error("failed to prepare recoverable import staging")]
    Staging,
    #[error("the import timed out")]
    TimedOut,
    #[error(transparent)]
    Library(#[from] LibraryServiceError),
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("filesystem operation failed")]
    Io(#[from] io::Error),
}

impl RommSourceError {
    /// Stable, actionable code surfaced in job snapshots and HTTP errors.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "romm_source_disabled",
            Self::NotConfigured => "romm_source_not_configured",
            Self::InvalidBaseUrl => "romm_invalid_base_url",
            Self::Unauthorized => "romm_unauthorized",
            Self::Unreachable => "romm_unreachable",
            Self::UpstreamStatus { .. } => "romm_upstream_status",
            Self::MalformedResponse => "romm_malformed_response",
            Self::ResponseTooLarge => "romm_response_too_large",
            Self::RedirectRefused => "romm_redirect_refused",
            Self::RemoteRomNotFound => "romm_rom_not_found",
            Self::UnknownRemoteFile => "romm_unknown_file",
            Self::NoFilesSelected => "romm_no_files_selected",
            Self::TooManyFiles => "romm_too_many_files",
            Self::ImportTooLarge => "romm_import_too_large",
            Self::PlatformNotFound => "romm_platform_not_found",
            Self::SizeMismatch => "romm_size_mismatch",
            Self::InsufficientStorage => "insufficient_storage",
            Self::Staging => "romm_staging_failed",
            Self::TimedOut => "romm_import_timed_out",
            Self::Library(_) => "romm_ingest_failed",
            Self::Database(_) => "internal_server_error",
            Self::Io(_) => "internal_server_error",
        }
    }
}

/// Trims and bounds one untrusted remote string, replacing control characters.
pub(crate) fn bounded_remote_text(value: &str) -> String {
    super::bounded_text(value.trim(), MAX_REMOTE_TEXT_BYTES).0
}

/// Normalizes an untrusted remote hash to lowercase hex, rejecting anything else.
pub(crate) fn normalized_hash(value: Option<&str>, expected_len: usize) -> Option<String> {
    let value = value?.trim().to_ascii_lowercase();
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_hashes_are_only_accepted_as_exact_lowercase_hex() {
        assert_eq!(
            normalized_hash(Some("  DEADBEEF "), 8),
            Some("deadbeef".to_string())
        );
        assert_eq!(normalized_hash(Some("deadbeeg"), 8), None);
        assert_eq!(normalized_hash(Some("deadbee"), 8), None);
        assert_eq!(normalized_hash(None, 8), None);
    }

    #[test]
    fn remote_text_is_bounded_and_stripped_of_control_characters() {
        let long = "a".repeat(MAX_REMOTE_TEXT_BYTES + 32);
        assert_eq!(bounded_remote_text(&long).len(), MAX_REMOTE_TEXT_BYTES);
        assert_eq!(bounded_remote_text(" Game\u{7}Title "), "Game\u{fffd}Title");
    }
}
