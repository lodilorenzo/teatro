use std::{
    io,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{
    config::IgdbConfig,
    domain::rom::Rom,
    services::file_operations::CoverOperationPayload,
    storage::{
        file_store::{FileStoreError, RootMutationGuard},
        paths::PathSafetyError,
    },
};

use super::{
    client::image_url,
    covers::validate_cover_image_id,
    normalization::{
        company_names, entity_names, normalize_igdb_url, normalize_string_vec,
        release_year_from_timestamp, trimmed_optional,
    },
};

pub const DEFAULT_SEARCH_LIMIT: u32 = 10;
pub const MAX_SEARCH_LIMIT: u32 = 25;
pub(super) const TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(60);
pub(super) const MAX_TOKEN_RESPONSE_BYTES: u64 = 64 * 1024;
pub(super) const MAX_SEARCH_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_ERROR_RESPONSE_BYTES: u64 = 16 * 1024;
pub(crate) const MAX_COVER_BYTES: u64 = 10 * 1024 * 1024;
pub(super) const METADATA_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum IgdbServiceError {
    #[error("IGDB credentials are not configured")]
    NotConfigured,

    #[error("IGDB search query cannot be empty")]
    EmptyQuery,

    #[error("IGDB search limit must be between 1 and {max}")]
    InvalidLimit { max: u32 },

    #[error("selected IGDB match is invalid: {0}")]
    InvalidSelectedMatch(&'static str),

    #[error("IGDB cover image id is invalid")]
    InvalidCoverImageId,

    #[error("cover must be a JPEG, PNG, or WebP image")]
    InvalidCoverImage,

    #[error("ROM not found")]
    RomNotFound,

    #[error("{service} URL is invalid: {source}")]
    Url {
        service: &'static str,
        #[source]
        source: url::ParseError,
    },

    #[error("{service} request failed: {source}")]
    Request {
        service: &'static str,
        #[source]
        source: reqwest::Error,
    },

    #[error("{service} returned HTTP {status}: {body}")]
    UpstreamStatus {
        service: &'static str,
        status: u16,
        body: String,
    },

    #[error("failed to parse {service} response: {source}")]
    Json {
        service: &'static str,
        #[source]
        source: serde_json::Error,
    },

    #[error("{service} response exceeded {max_bytes} bytes")]
    ResponseTooLarge {
        service: &'static str,
        max_bytes: u64,
    },

    #[error("IGDB token response did not include an access token")]
    MissingAccessToken,

    #[error("cover file recovery failed: {0}")]
    CoverRecovery(String),

    #[error("cover image is too large")]
    CoverTooLarge,

    #[error(transparent)]
    PathSafety(#[from] PathSafetyError),

    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Serialize)]
pub struct IgdbStatus {
    pub configured: bool,
    pub client_id_configured: bool,
    pub client_secret_configured: bool,
    pub token_cached: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IgdbGameCandidate {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub storyline: Option<String>,
    #[serde(default)]
    pub first_release_date: Option<i64>,
    #[serde(default)]
    pub release_year: Option<i32>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub aggregated_rating: Option<f64>,
    #[serde(default)]
    pub total_rating: Option<f64>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub developers: Vec<String>,
    #[serde(default)]
    pub publishers: Vec<String>,
    #[serde(default)]
    pub cover_image_id: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(skip)]
    pub(super) platform_ids: Vec<i64>,
    #[serde(skip)]
    pub(super) platform_keys: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CachedCoverAsset {
    pub kind: String,
    pub resource_path: String,
    pub media_type: String,
    pub file_size_bytes: i64,
}

pub(super) struct DownloadedCover {
    pub(super) asset: CachedCoverAsset,
    pub(super) bytes: Vec<u8>,
}

#[derive(Debug)]
pub(super) struct PreparedCoverAssets {
    pub(super) assets: Vec<CachedCoverAsset>,
    pub(super) operation_id: Option<String>,
    pub(super) payload: Option<CoverOperationPayload>,
    // The asset-root lock remains held through metadata commit and journal cleanup.
    pub(super) _root_lock: Option<RootMutationGuard>,
}

#[derive(Debug, Clone)]
pub struct AppliedIgdbMetadata {
    pub rom: Rom,
    pub cached_covers: Vec<CachedCoverAsset>,
}

pub(crate) enum AutomaticMetadataOutcome {
    Applied(Box<AppliedIgdbMetadata>),
    NoMatch,
}

#[derive(Debug)]
pub(super) struct CachedAccessToken {
    pub(super) access_token: String,
    pub(super) expires_at: Instant,
}

#[derive(Debug, Deserialize)]
pub(super) struct TwitchTokenResponse {
    pub(super) access_token: String,
    pub(super) expires_in: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct IgdbGame {
    id: i64,
    name: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    storyline: Option<String>,
    #[serde(default)]
    first_release_date: Option<i64>,
    #[serde(default)]
    rating: Option<f64>,
    #[serde(default)]
    aggregated_rating: Option<f64>,
    #[serde(default)]
    total_rating: Option<f64>,
    #[serde(default)]
    genres: Vec<IgdbNamedEntity>,
    #[serde(default)]
    involved_companies: Vec<IgdbInvolvedCompany>,
    #[serde(default)]
    platforms: Vec<IgdbPlatform>,
    #[serde(default)]
    cover: Option<IgdbCover>,
}

impl IgdbGame {
    pub(super) fn into_candidate(
        self,
        config: &IgdbConfig,
    ) -> Result<IgdbGameCandidate, IgdbServiceError> {
        let genres = entity_names(self.genres);
        let developers = company_names(&self.involved_companies, CompanyRole::Developer);
        let publishers = company_names(&self.involved_companies, CompanyRole::Publisher);
        let mut platform_ids = self
            .platforms
            .iter()
            .map(|platform| platform.id)
            .filter(|id| *id > 0)
            .collect::<Vec<_>>();
        platform_ids.sort_unstable();
        platform_ids.dedup();
        let platform_keys = normalize_string_vec(
            self.platforms
                .into_iter()
                .flat_map(|platform| {
                    [
                        platform.name,
                        platform.abbreviation,
                        platform.alternative_name,
                        platform.slug,
                    ]
                    .into_iter()
                    .flatten()
                })
                .collect(),
        );
        let release_year = self
            .first_release_date
            .and_then(release_year_from_timestamp);
        let cover_image_id = self
            .cover
            .as_ref()
            .and_then(|cover| trimmed_optional(cover.image_id.as_deref()));
        let cover_url = match cover_image_id.as_deref() {
            Some(image_id) => Some(image_url(config, "t_cover_big", image_id)?.to_string()),
            None => self
                .cover
                .as_ref()
                .and_then(|cover| cover.url.as_deref())
                .and_then(normalize_igdb_url),
        };

        Ok(IgdbGameCandidate {
            id: self.id,
            name: self.name,
            summary: self
                .summary
                .and_then(|value| trimmed_optional(Some(&value))),
            storyline: self
                .storyline
                .and_then(|value| trimmed_optional(Some(&value))),
            first_release_date: self.first_release_date,
            release_year,
            rating: self.rating,
            aggregated_rating: self.aggregated_rating,
            total_rating: self.total_rating,
            genres,
            developers,
            publishers,
            cover_image_id,
            cover_url,
            platform_ids,
            platform_keys,
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct IgdbNamedEntity {
    #[serde(default)]
    pub(super) name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IgdbInvolvedCompany {
    #[serde(default)]
    pub(super) developer: Option<bool>,
    #[serde(default)]
    pub(super) publisher: Option<bool>,
    #[serde(default)]
    pub(super) company: Option<IgdbNamedEntity>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IgdbPlatform {
    #[serde(default)]
    id: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    abbreviation: Option<String>,
    #[serde(default)]
    alternative_name: Option<String>,
    #[serde(default)]
    slug: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct IgdbCover {
    #[serde(default)]
    image_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompanyRole {
    Developer,
    Publisher,
}

impl IgdbGameCandidate {
    pub(super) fn normalized_for_save(mut self) -> Result<Self, IgdbServiceError> {
        if self.id <= 0 {
            return Err(IgdbServiceError::InvalidSelectedMatch(
                "id must be a positive integer",
            ));
        }

        self.name = self.name.trim().to_string();
        if self.name.is_empty() {
            return Err(IgdbServiceError::InvalidSelectedMatch(
                "name cannot be empty",
            ));
        }

        self.summary = trimmed_optional(self.summary.as_deref());
        self.storyline = trimmed_optional(self.storyline.as_deref());
        self.genres = normalize_string_vec(self.genres);
        self.developers = normalize_string_vec(self.developers);
        self.publishers = normalize_string_vec(self.publishers);
        self.cover_image_id = trimmed_optional(self.cover_image_id.as_deref());
        self.cover_url = trimmed_optional(self.cover_url.as_deref());

        if let Some(image_id) = self.cover_image_id.as_deref() {
            validate_cover_image_id(image_id)?;
        }

        if self.release_year.is_none() {
            self.release_year = self
                .first_release_date
                .and_then(release_year_from_timestamp);
        }

        Ok(self)
    }

    pub(super) fn metadata_json(
        &self,
        path_cover_large: Option<&str>,
        path_cover_small: Option<&str>,
    ) -> Value {
        json!({
            "schema_version": METADATA_SCHEMA_VERSION,
            "source": "igdb",
            "igdb_id": self.id,
            "name": self.name,
            "summary": self.summary,
            "storyline": self.storyline,
            "first_release_date": self.first_release_date,
            "release_year": self.release_year,
            "rating": self.rating,
            "aggregated_rating": self.aggregated_rating,
            "total_rating": self.total_rating,
            "genres": self.genres,
            "developers": self.developers,
            "publishers": self.publishers,
            "cover_image_id": self.cover_image_id,
            "cover_url": self.cover_url,
            "path_cover_large": path_cover_large,
            "path_cover_small": path_cover_small,
        })
    }
}
