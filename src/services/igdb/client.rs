use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

use reqwest::header::{ACCEPT, CONTENT_TYPE};
use url::Url;

use crate::{
    config::IgdbConfig,
    domain::rom::Rom,
    repositories::roms::{self, CoverAssetUpsert},
    state::AppState,
};

use super::{
    covers::{
        commit_prepared_covers, cover_resource_path, prepare_cover_assets,
        rollback_prepared_covers, validate_cover_image_id,
    },
    models::{
        AppliedIgdbMetadata, AutomaticMetadataOutcome, CachedAccessToken, CachedCoverAsset,
        DEFAULT_SEARCH_LIMIT, DownloadedCover, IgdbGame, IgdbGameCandidate, IgdbServiceError,
        IgdbStatus, MAX_COVER_BYTES, MAX_ERROR_RESPONSE_BYTES, MAX_SEARCH_LIMIT,
        MAX_SEARCH_RESPONSE_BYTES, MAX_TOKEN_RESPONSE_BYTES, METADATA_SCHEMA_VERSION,
        PreparedCoverAssets, TOKEN_REFRESH_SKEW, TwitchTokenResponse,
    },
    normalization::rank_candidates,
    platform_ids,
};

#[derive(Debug)]
pub struct IgdbClient {
    http: reqwest::Client,
    token_cache: Mutex<Option<CachedAccessToken>>,
}

impl IgdbClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            token_cache: Mutex::new(None),
        }
    }

    pub fn status(&self, config: &IgdbConfig) -> IgdbStatus {
        IgdbStatus {
            configured: config.is_configured(),
            client_id_configured: config.client_id.is_some(),
            client_secret_configured: config.client_secret.is_some(),
            token_cached: self.token_is_cached(),
        }
    }

    pub fn clear_token_cache(&self) {
        let mut cache = self
            .token_cache
            .lock()
            .expect("IGDB token cache mutex should not be poisoned");
        *cache = None;
    }

    async fn search_once(
        &self,
        config: &IgdbConfig,
        query: &str,
        limit: u32,
        platform: Option<(&str, &str)>,
        require_platform_match: bool,
    ) -> Result<Vec<IgdbGameCandidate>, IgdbServiceError> {
        ensure_configured(config)?;

        let query = query.trim();
        if query.is_empty() {
            return Err(IgdbServiceError::EmptyQuery);
        }
        if limit == 0 || limit > MAX_SEARCH_LIMIT {
            return Err(IgdbServiceError::InvalidLimit {
                max: MAX_SEARCH_LIMIT,
            });
        }

        let approved_platform_ids = platform.and_then(|(slug, _)| platform_ids::for_slug(slug));
        if require_platform_match && approved_platform_ids.is_none() {
            if let Some((slug, _)) = platform {
                debug_assert!(
                    platform_ids::is_deliberately_unmapped(slug),
                    "Teatro platform slug is not classified for IGDB matching: {slug}"
                );
            }
            return Ok(Vec::new());
        }

        let access_token = self.access_token(config).await?;
        let url = endpoint_url(&config.api_url, "games", "IGDB API")?;
        let body = build_search_body(query, limit, approved_platform_ids);
        let client_id = config
            .client_id
            .as_deref()
            .ok_or(IgdbServiceError::NotConfigured)?;

        let response = self
            .http
            .post(url)
            .header("Client-ID", client_id)
            .bearer_auth(access_token)
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "text/plain")
            .body(body)
            .send()
            .await
            .map_err(|source| IgdbServiceError::Request {
                service: "IGDB API",
                source,
            })?;

        let status = response.status();
        let max_bytes = if status.is_success() {
            MAX_SEARCH_RESPONSE_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let bytes = read_response_limited(response, "IGDB API", max_bytes).await?;

        if !status.is_success() {
            return Err(IgdbServiceError::UpstreamStatus {
                service: "IGDB API",
                status: status.as_u16(),
                body: truncate_body(&String::from_utf8_lossy(&bytes)),
            });
        }

        let games: Vec<IgdbGame> =
            serde_json::from_slice(&bytes).map_err(|source| IgdbServiceError::Json {
                service: "IGDB API",
                source,
            })?;

        let mut candidates = games
            .into_iter()
            .map(|game| game.into_candidate(config))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(ids) = approved_platform_ids {
            candidates.retain(|candidate| candidate_matches_platform(candidate, ids));
        }
        let fallback_platform = approved_platform_ids
            .is_none()
            .then_some(platform)
            .flatten();
        rank_candidates(query, fallback_platform, &mut candidates);
        Ok(candidates)
    }

    pub async fn search(
        &self,
        config: &IgdbConfig,
        query: &str,
        limit: u32,
        platform: Option<(&str, &str)>,
        require_platform_match: bool,
    ) -> Result<Vec<IgdbGameCandidate>, IgdbServiceError> {
        let should_fallback = platform.is_some_and(|(slug, _)| {
            require_platform_match || platform_ids::for_slug(slug).is_some()
        });
        let results = self
            .search_once(config, query, limit, platform, require_platform_match)
            .await?;
        if results.is_empty() && should_fallback {
            return self.search_once(config, query, limit, None, false).await;
        }
        Ok(results)
    }

    pub(crate) async fn auto_apply_metadata(
        &self,
        state: &AppState,
        rom_id: i64,
        game_name: &str,
    ) -> Result<AutomaticMetadataOutcome, IgdbServiceError> {
        let config = state.igdb_config().await?;
        let rom = roms::find_by_id(state.db(), rom_id)
            .await?
            .ok_or(IgdbServiceError::RomNotFound)?;
        let candidates = self
            .search(
                &config,
                game_name,
                DEFAULT_SEARCH_LIMIT,
                Some((&rom.platform_slug, &rom.platform_display_name)),
                true,
            )
            .await?;
        let Some(selected) = candidates.into_iter().next() else {
            return Ok(AutomaticMetadataOutcome::NoMatch);
        };
        self.apply_metadata(state, rom_id, selected, true)
            .await
            .map(Box::new)
            .map(AutomaticMetadataOutcome::Applied)
    }

    pub async fn apply_metadata(
        &self,
        state: &AppState,
        rom_id: i64,
        selected_match: IgdbGameCandidate,
        cache_cover: bool,
    ) -> Result<AppliedIgdbMetadata, IgdbServiceError> {
        let config = state.igdb_config().await?;
        ensure_configured(&config)?;
        let selected_match = selected_match.normalized_for_save()?;
        let rom = roms::find_by_id(state.db(), rom_id)
            .await?
            .ok_or(IgdbServiceError::RomNotFound)?;

        let prepared_covers = if cache_cover {
            self.cache_cover_assets(state, &config, &rom, &selected_match)
                .await?
        } else {
            PreparedCoverAssets {
                assets: Vec::new(),
                operation_id: None,
                payload: None,
                _root_lock: None,
            }
        };
        let cached_covers = &prepared_covers.assets;

        let path_cover_large = cached_covers
            .iter()
            .find(|asset| asset.kind == "large")
            .map(|asset| asset.resource_path.as_str());
        let path_cover_small = cached_covers
            .iter()
            .find(|asset| asset.kind == "small")
            .map(|asset| asset.resource_path.as_str());
        let summary = selected_match
            .summary
            .as_deref()
            .or(selected_match.storyline.as_deref());
        let mut metadata_json = selected_match.metadata_json(path_cover_large, path_cover_small);
        if let (Some(target), Some(existing)) =
            (metadata_json.as_object_mut(), rom.metadata.as_object())
        {
            for key in [
                "integrity",
                "filename",
                "serials",
                "languages",
                "regions",
                "romm",
            ] {
                if let Some(value) = existing.get(key) {
                    target.insert(key.to_string(), value.clone());
                }
            }
        }
        let metadata_json = metadata_json.to_string();
        let cover_assets = cached_covers
            .iter()
            .map(|asset| CoverAssetUpsert {
                resource_path: asset.resource_path.as_str(),
                media_type: asset.media_type.as_str(),
                file_size_bytes: asset.file_size_bytes,
            })
            .collect();

        let save_result = roms::save_igdb_metadata(
            state.db(),
            roms::SaveIgdbMetadataParams {
                rom_id,
                metadata_json: &metadata_json,
                schema_version: METADATA_SCHEMA_VERSION,
                summary,
                url_cover: selected_match.cover_url.as_deref(),
                path_cover_large,
                path_cover_small,
                cover_assets,
                file_operation_id: prepared_covers.operation_id.as_deref(),
            },
        )
        .await;

        match save_result {
            Ok(()) => {
                commit_prepared_covers(state, &prepared_covers).await?;
                let rom = roms::find_by_id(state.db(), rom_id)
                    .await?
                    .ok_or(IgdbServiceError::RomNotFound)?;
                Ok(AppliedIgdbMetadata {
                    rom,
                    cached_covers: prepared_covers.assets,
                })
            }
            Err(error) => {
                rollback_prepared_covers(state, &prepared_covers).await;
                Err(error.into())
            }
        }
    }

    async fn access_token(&self, config: &IgdbConfig) -> Result<String, IgdbServiceError> {
        if let Some(access_token) = self.cached_access_token() {
            return Ok(access_token);
        }

        let token = self.fetch_access_token(config).await?;
        let access_token = token.access_token.clone();
        let mut cache = self
            .token_cache
            .lock()
            .expect("IGDB token cache mutex should not be poisoned");
        *cache = Some(token);

        Ok(access_token)
    }

    fn cached_access_token(&self) -> Option<String> {
        let cache = self
            .token_cache
            .lock()
            .expect("IGDB token cache mutex should not be poisoned");
        let cached = cache.as_ref()?;
        if cached.expires_at > Instant::now() {
            Some(cached.access_token.clone())
        } else {
            None
        }
    }

    fn token_is_cached(&self) -> bool {
        self.cached_access_token().is_some()
    }

    async fn fetch_access_token(
        &self,
        config: &IgdbConfig,
    ) -> Result<CachedAccessToken, IgdbServiceError> {
        let client_id = config
            .client_id
            .as_deref()
            .ok_or(IgdbServiceError::NotConfigured)?;
        let client_secret = config
            .client_secret
            .as_deref()
            .ok_or(IgdbServiceError::NotConfigured)?;
        let url = parse_url(&config.token_url, "Twitch OAuth")?;

        let response = self
            .http
            .post(url)
            .form(&[
                ("client_id", client_id),
                ("client_secret", client_secret),
                ("grant_type", "client_credentials"),
            ])
            .send()
            .await
            .map_err(|source| IgdbServiceError::Request {
                service: "Twitch OAuth",
                source,
            })?;

        let status = response.status();
        let max_bytes = if status.is_success() {
            MAX_TOKEN_RESPONSE_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let bytes = read_response_limited(response, "Twitch OAuth", max_bytes).await?;

        if !status.is_success() {
            return Err(IgdbServiceError::UpstreamStatus {
                service: "Twitch OAuth",
                status: status.as_u16(),
                body: truncate_body(&String::from_utf8_lossy(&bytes)),
            });
        }

        let token: TwitchTokenResponse =
            serde_json::from_slice(&bytes).map_err(|source| IgdbServiceError::Json {
                service: "Twitch OAuth",
                source,
            })?;

        if token.access_token.trim().is_empty() {
            return Err(IgdbServiceError::MissingAccessToken);
        }

        let expires_in = token.expires_in.max(60) as u64;
        let cache_ttl = Duration::from_secs(expires_in).saturating_sub(TOKEN_REFRESH_SKEW);

        Ok(CachedAccessToken {
            access_token: token.access_token,
            expires_at: Instant::now() + cache_ttl,
        })
    }

    async fn cache_cover_assets(
        &self,
        state: &AppState,
        config: &IgdbConfig,
        rom: &Rom,
        selected_match: &IgdbGameCandidate,
    ) -> Result<PreparedCoverAssets, IgdbServiceError> {
        let Some(image_id) = selected_match.cover_image_id.as_deref() else {
            return Ok(PreparedCoverAssets {
                assets: Vec::new(),
                operation_id: None,
                payload: None,
                _root_lock: None,
            });
        };
        validate_cover_image_id(image_id)?;

        let large_resource_path = cover_resource_path(rom, "cover-large.jpg");
        let small_resource_path = cover_resource_path(rom, "cover-small.jpg");
        let large_url = image_url(config, "t_cover_big", image_id)?;
        let small_url = image_url(config, "t_cover_small", image_id)?;

        let large = self
            .download_cover("large", &large_url, &large_resource_path)
            .await?;
        let small = self
            .download_cover("small", &small_url, &small_resource_path)
            .await?;

        prepare_cover_assets(state, rom, vec![large, small]).await
    }

    async fn download_cover(
        &self,
        kind: &str,
        url: &Url,
        resource_path: &str,
    ) -> Result<DownloadedCover, IgdbServiceError> {
        let response = self.http.get(url.clone()).send().await.map_err(|source| {
            IgdbServiceError::Request {
                service: "IGDB image CDN",
                source,
            }
        })?;

        let status = response.status();
        let media_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("image/jpeg")
            .to_string();
        let max_bytes = if status.is_success() {
            MAX_COVER_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let bytes = match read_response_limited(response, "IGDB image CDN", max_bytes).await {
            Err(IgdbServiceError::ResponseTooLarge { .. }) if status.is_success() => {
                return Err(IgdbServiceError::CoverTooLarge);
            }
            result => result?,
        };

        if !status.is_success() {
            return Err(IgdbServiceError::UpstreamStatus {
                service: "IGDB image CDN",
                status: status.as_u16(),
                body: truncate_body(&String::from_utf8_lossy(&bytes)),
            });
        }

        Ok(DownloadedCover {
            asset: CachedCoverAsset {
                kind: kind.to_string(),
                resource_path: resource_path.to_string(),
                media_type,
                file_size_bytes: bytes.len() as i64,
            },
            bytes,
        })
    }
}

impl Default for IgdbClient {
    fn default() -> Self {
        Self::new()
    }
}

fn ensure_configured(config: &IgdbConfig) -> Result<(), IgdbServiceError> {
    if config.is_configured() {
        Ok(())
    } else {
        Err(IgdbServiceError::NotConfigured)
    }
}

fn build_search_body(query: &str, limit: u32, platform_ids: Option<&[i64]>) -> String {
    let platform_filter = platform_ids
        .map(|ids| {
            let ids = ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
            format!("\nwhere platforms = ({ids});")
        })
        .unwrap_or_default();
    format!(
        "search \"{}\";\nfields name,summary,storyline,first_release_date,rating,aggregated_rating,total_rating,genres.name,involved_companies.company.name,involved_companies.developer,involved_companies.publisher,platforms.id,platforms.name,platforms.abbreviation,platforms.alternative_name,platforms.slug,cover.image_id,cover.url;{platform_filter}\nlimit {limit};",
        escape_apicalypse_string(query)
    )
}

fn candidate_matches_platform(candidate: &IgdbGameCandidate, platform_ids: &[i64]) -> bool {
    candidate
        .platform_ids
        .iter()
        .any(|id| platform_ids.contains(id))
}

fn escape_apicalypse_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn endpoint_url(
    base_url: &str,
    endpoint: &str,
    service: &'static str,
) -> Result<Url, IgdbServiceError> {
    let base_url = format!("{}/", base_url.trim_end_matches('/'));
    parse_url(&base_url, service)?
        .join(endpoint)
        .map_err(|source| IgdbServiceError::Url { service, source })
}

fn parse_url(value: &str, service: &'static str) -> Result<Url, IgdbServiceError> {
    Url::parse(value).map_err(|source| IgdbServiceError::Url { service, source })
}

pub(super) fn image_url(
    config: &IgdbConfig,
    size: &str,
    image_id: &str,
) -> Result<Url, IgdbServiceError> {
    validate_cover_image_id(image_id)?;
    parse_url(
        &format!(
            "{}/{size}/{image_id}.jpg",
            config.image_base_url.trim_end_matches('/')
        ),
        "IGDB image CDN",
    )
}

async fn read_response_limited(
    mut response: reqwest::Response,
    service: &'static str,
    max_bytes: u64,
) -> Result<Vec<u8>, IgdbServiceError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes)
    {
        return Err(IgdbServiceError::ResponseTooLarge { service, max_bytes });
    }

    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|source| IgdbServiceError::Request { service, source })?
    {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > max_bytes {
            return Err(IgdbServiceError::ResponseTooLarge { service, max_bytes });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn truncate_body(body: &str) -> String {
    const MAX_BODY_CHARS: usize = 512;
    let truncated: String = body.chars().take(MAX_BODY_CHARS).collect();
    if body.chars().count() > MAX_BODY_CHARS {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_search_body, candidate_matches_platform};
    use crate::services::igdb::{covers::safe_resource_segment, models::IgdbGameCandidate};

    #[test]
    fn search_body_escapes_quotes_and_filters_platform_ids() {
        let body = build_search_body("Sonic \\\"Test\\\"", 5, Some(&[37, 137]));

        assert!(body.contains("search \"Sonic \\\\\\\"Test\\\\\\\"\";"));
        assert!(body.contains("platforms.id"));
        assert_eq!(body.matches("where platforms = (37,137);").count(), 1);
        assert!(body.contains("limit 5;"));
        assert!(!build_search_body("Sonic", 5, None).contains("where platforms"));
    }

    #[test]
    fn candidate_platform_ids_must_intersect_the_mapping() {
        let mut candidate: IgdbGameCandidate = serde_json::from_value(json!({
            "id": 1,
            "name": "Sonic"
        }))
        .unwrap();
        candidate.platform_ids = vec![19, 29];

        assert!(candidate_matches_platform(&candidate, &[29]));
        assert!(!candidate_matches_platform(&candidate, &[7]));
    }

    #[test]
    fn resource_segments_are_path_safe() {
        assert_eq!(safe_resource_segment("../Sonic CD!"), "sonic-cd");
        assert_eq!(safe_resource_segment(""), "resource");
    }
}
