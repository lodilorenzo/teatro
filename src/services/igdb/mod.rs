//! IGDB client, response normalization, and recoverable cover caching.

mod client;
mod covers;
mod models;
mod normalization;
mod platform_ids;

pub use client::IgdbClient;
pub(crate) use covers::{replace_cover_bytes, replace_uploaded_cover};
pub use models::{
    AppliedIgdbMetadata, CachedCoverAsset, DEFAULT_SEARCH_LIMIT, IgdbGameCandidate,
    IgdbServiceError, IgdbStatus, MAX_SEARCH_LIMIT,
};
pub(crate) use models::{AutomaticMetadataOutcome, MAX_COVER_BYTES};
