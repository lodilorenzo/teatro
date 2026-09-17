//! Bounded, redirect-refusing HTTP client for one configured RomM server.
//!
//! Every method normalizes the upstream envelope into Teatro-shaped values. Nothing here parses
//! or forwards RomM JSON to the browser, and nothing here logs, returns, or stores the secret.

use std::{
    collections::HashSet,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use chrono::{DateTime, Datelike, Utc};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderValue};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use url::Url;

use crate::{
    config::RommSourceConfig,
    repositories::romm_source::{RommAuthMode, RommSourceSettings},
};

use super::{
    MAX_BROWSE_LIMIT, MAX_COVER_BYTES, MAX_ERROR_RESPONSE_BYTES, MAX_JSON_RESPONSE_BYTES,
    MAX_REMOTE_PLATFORMS, RemoteCover, RemotePlatform, RemoteRom, RemoteRomDetail, RemoteRomFile,
    RemoteRomMetadata, RemoteRomPage, RommSourceError, bounded_remote_text, normalized_hash,
};

/// Refresh a cached access token slightly before the server expires it.
const TOKEN_REFRESH_SKEW: Duration = Duration::from_secs(60);
const MAX_TOKEN_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_REMOTE_METADATA_ITEMS: usize = 128;
/// A complete snapshot is still bounded because the remote peer is untrusted.
pub(super) const MAX_REMOTE_GAMES: usize = 100_000;

/// Normalized result of one connection probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RommConnection {
    pub reachable: bool,
    pub authenticated: bool,
    pub version: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug)]
struct CachedToken {
    cache_key: String,
    header: String,
    expires_at: Instant,
}

#[derive(Debug)]
pub(crate) struct RommClient {
    browse: reqwest::Client,
    download: reqwest::Client,
    token_cache: Mutex<Option<CachedToken>>,
}

impl RommClient {
    pub(crate) fn new(config: &RommSourceConfig) -> Self {
        let timeout = Duration::from_secs(config.timeout_seconds.max(1));
        let build = |client: reqwest::ClientBuilder| {
            client
                // A remote redirect is a silent re-target of an admin-configured, credentialed
                // request; refuse it rather than following it anywhere.
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(timeout)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        };
        Self {
            browse: build(reqwest::Client::builder().timeout(timeout)),
            // Downloads are bounded by the whole-import wall clock and by an idle read timeout
            // rather than by a total request timeout, because a large ROM legitimately takes
            // longer than one browse call.
            download: build(reqwest::Client::builder().read_timeout(timeout)),
            token_cache: Mutex::new(None),
        }
    }

    pub(crate) fn clear_token_cache(&self) {
        *self.lock_token_cache() = None;
    }

    /// One bounded authenticated probe reporting reachable/unauthorized/unreachable.
    pub(crate) async fn probe(
        &self,
        settings: &RommSourceSettings,
    ) -> Result<RommConnection, RommSourceError> {
        let heartbeat = self
            .get_json(settings, "api/heartbeat", &[], false)
            .await
            .map(|value| extract_version(&value));
        let version = match heartbeat {
            Ok(version) => version,
            Err(RommSourceError::Unauthorized) => None,
            Err(error) => return Err(error),
        };

        match self.get_json(settings, "api/users/me", &[], true).await {
            Ok(value) => Ok(RommConnection {
                reachable: true,
                authenticated: true,
                version,
                username: value
                    .get("username")
                    .and_then(Value::as_str)
                    .map(bounded_remote_text),
            }),
            Err(RommSourceError::Unauthorized) => Ok(RommConnection {
                reachable: true,
                authenticated: false,
                version,
                username: None,
            }),
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn platforms(
        &self,
        settings: &RommSourceSettings,
    ) -> Result<Vec<RemotePlatform>, RommSourceError> {
        let value = self.get_json(settings, "api/platforms", &[], true).await?;
        let items = as_items(&value).ok_or(RommSourceError::MalformedResponse)?;
        Ok(items
            .iter()
            .filter_map(parse_platform)
            .filter(|platform| platform.rom_count > 0)
            .take(MAX_REMOTE_PLATFORMS)
            .collect())
    }

    pub(crate) async fn roms(
        &self,
        settings: &RommSourceSettings,
        platform_id: Option<i64>,
        search: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<RemoteRomPage, RommSourceError> {
        let limit = limit.clamp(1, MAX_BROWSE_LIMIT);
        let query = browse_query(platform_id, search, limit, offset);

        let value = self.get_json(settings, "api/roms", &query, true).await?;
        let items = as_items(&value).ok_or(RommSourceError::MalformedResponse)?;
        if items.len() > limit as usize {
            return Err(RommSourceError::MalformedResponse);
        }
        let returned = items.len();
        let parsed = items.iter().filter_map(parse_rom).collect::<Vec<_>>();
        if parsed.len() != returned {
            return Err(RommSourceError::MalformedResponse);
        }
        let total = value
            .get("total")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| i64::try_from(parsed.len()).unwrap_or(i64::MAX))
            .max(0);

        Ok(RemoteRomPage {
            items: parsed,
            total,
            limit,
            offset,
            returned,
        })
    }

    /// Downloads every page of the remote game catalog for an atomic local snapshot.
    pub(crate) async fn all_roms(
        &self,
        settings: &RommSourceSettings,
    ) -> Result<Vec<RemoteRom>, RommSourceError> {
        let mut roms = Vec::new();
        let mut seen = HashSet::new();
        let mut offset = 0_u32;

        loop {
            let page = self
                .roms(settings, None, None, MAX_BROWSE_LIMIT, offset)
                .await?;
            if page.total > MAX_REMOTE_GAMES as i64 {
                return Err(RommSourceError::ResponseTooLarge);
            }
            for rom in page.items {
                if !seen.insert(rom.id) {
                    return Err(RommSourceError::MalformedResponse);
                }
                roms.push(rom);
            }
            if roms.len() > MAX_REMOTE_GAMES {
                return Err(RommSourceError::ResponseTooLarge);
            }
            if page.returned < page.limit as usize {
                break;
            }
            offset = offset
                .checked_add(page.limit)
                .ok_or(RommSourceError::ResponseTooLarge)?;
        }
        Ok(roms)
    }

    pub(crate) async fn rom_detail(
        &self,
        settings: &RommSourceSettings,
        remote_rom_id: i64,
    ) -> Result<RemoteRomDetail, RommSourceError> {
        let value = self
            .get_json(settings, &format!("api/roms/{remote_rom_id}"), &[], true)
            .await
            .map_err(|error| match error {
                RommSourceError::UpstreamStatus { status: 404 } => {
                    RommSourceError::RemoteRomNotFound
                }
                error => error,
            })?;

        let rom = parse_rom(&value).ok_or(RommSourceError::MalformedResponse)?;
        let metadata = parse_rom_metadata(&value, remote_rom_id);
        let mut files = parse_files(&value);
        if files.is_empty() {
            // A single-file remote game may only describe its filesystem name.
            if let Some(file_name) = rom.fs_name.clone() {
                files.push(RemoteRomFile {
                    id: rom.id,
                    file_name,
                    file_size_bytes: rom.file_size_bytes,
                    crc32: None,
                    md5: None,
                    sha1: None,
                    sha256: None,
                });
            }
        }
        Ok(RemoteRomDetail {
            rom,
            files,
            metadata,
        })
    }

    /// Fetches one bounded cover image. Never written to `data/assets`.
    pub(crate) async fn cover(
        &self,
        settings: &RommSourceSettings,
        remote_rom_id: i64,
    ) -> Result<RemoteCover, RommSourceError> {
        let detail = self
            .get_json(settings, &format!("api/roms/{remote_rom_id}"), &[], true)
            .await?;
        let path = ["path_cover_small", "path_cover_large"]
            .iter()
            .find_map(|key| detail.get(*key).and_then(Value::as_str))
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .ok_or(RommSourceError::RemoteRomNotFound)?;

        let url = self.endpoint(settings, path.trim_start_matches('/'), &[])?;
        let response = self.send(&self.browse, settings, url).await?;
        let status = response.status();
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .filter(|value| value.starts_with("image/"))
            .unwrap_or("application/octet-stream")
            .to_string();
        let bytes = read_bounded(response, MAX_COVER_BYTES).await?;
        if !status.is_success() {
            return Err(status_error(status.as_u16()));
        }
        Ok(RemoteCover {
            bytes,
            content_type,
        })
    }

    /// Opens a streaming download for one remote physical file.
    pub(crate) async fn download_file(
        &self,
        settings: &RommSourceSettings,
        remote_rom_id: i64,
        file: &RemoteRomFile,
    ) -> Result<reqwest::Response, RommSourceError> {
        let url = self.endpoint(
            settings,
            &format!(
                "api/roms/{remote_rom_id}/content/{}",
                utf8_percent_segment(&file.file_name)
            ),
            &[("file_ids".to_string(), file.id.to_string())],
        )?;
        let response = self.send(&self.download, settings, url).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status.as_u16()));
        }
        Ok(response)
    }

    async fn get_json(
        &self,
        settings: &RommSourceSettings,
        path: &str,
        query: &[(String, String)],
        authenticate: bool,
    ) -> Result<Value, RommSourceError> {
        let url = self.endpoint(settings, path, query)?;
        let request = self.browse.get(url).header(ACCEPT, "application/json");
        let request = if authenticate {
            request.header(AUTHORIZATION, self.authorization(settings).await?)
        } else {
            request
        };

        let response = request.send().await.map_err(request_error)?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_JSON_RESPONSE_BYTES
        } else {
            MAX_ERROR_RESPONSE_BYTES
        };
        let bytes = read_bounded(response, limit).await?;
        if !status.is_success() {
            if status.as_u16() == 401 || status.as_u16() == 403 {
                self.clear_token_cache();
            }
            return Err(status_error(status.as_u16()));
        }
        serde_json::from_slice(&bytes).map_err(|_| RommSourceError::MalformedResponse)
    }

    async fn send(
        &self,
        http: &reqwest::Client,
        settings: &RommSourceSettings,
        url: Url,
    ) -> Result<reqwest::Response, RommSourceError> {
        http.get(url)
            .header(AUTHORIZATION, self.authorization(settings).await?)
            .send()
            .await
            .map_err(request_error)
    }

    async fn authorization(
        &self,
        settings: &RommSourceSettings,
    ) -> Result<HeaderValue, RommSourceError> {
        let username = settings
            .username
            .as_deref()
            .ok_or(RommSourceError::NotConfigured)?;
        let secret = settings
            .secret
            .as_deref()
            .ok_or(RommSourceError::NotConfigured)?;

        let header = match settings.auth_mode {
            RommAuthMode::Basic => basic_header(username, secret),
            RommAuthMode::Token => {
                let cache_key = format!("{}\u{0}{username}", settings.base_url);
                if let Some(cached) = self.cached_token(&cache_key) {
                    cached
                } else {
                    let token = self.request_token(settings, username, secret).await?;
                    let header = format!("Bearer {}", token.access_token);
                    *self.lock_token_cache() = Some(CachedToken {
                        cache_key,
                        header: header.clone(),
                        expires_at: Instant::now()
                            + Duration::from_secs(token.expires_in.max(1))
                                .saturating_sub(TOKEN_REFRESH_SKEW),
                    });
                    header
                }
            }
        };

        // Invalid header bytes can only come from a credential the operator typed; report it as
        // an authentication problem rather than leaking the value into an error.
        HeaderValue::from_str(&header).map_err(|_| RommSourceError::Unauthorized)
    }

    async fn request_token(
        &self,
        settings: &RommSourceSettings,
        username: &str,
        secret: &str,
    ) -> Result<TokenResponse, RommSourceError> {
        let url = self.endpoint(settings, "api/token", &[])?;
        let response = self
            .browse
            .post(url)
            .header(ACCEPT, "application/json")
            .form(&[
                ("grant_type", "password"),
                ("username", username),
                ("password", secret),
            ])
            .send()
            .await
            .map_err(request_error)?;

        let status = response.status();
        let bytes = read_bounded(
            response,
            if status.is_success() {
                MAX_TOKEN_RESPONSE_BYTES
            } else {
                MAX_ERROR_RESPONSE_BYTES
            },
        )
        .await?;
        if !status.is_success() {
            return Err(match status.as_u16() {
                400 | 401 | 403 | 422 => RommSourceError::Unauthorized,
                status => status_error(status),
            });
        }

        let parsed: RawTokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| RommSourceError::MalformedResponse)?;
        let access_token = parsed.access_token.ok_or(RommSourceError::Unauthorized)?;
        if access_token.trim().is_empty() {
            return Err(RommSourceError::Unauthorized);
        }
        Ok(TokenResponse {
            access_token,
            expires_in: parsed.expires_in.or(parsed.expires).unwrap_or(900),
        })
    }

    fn cached_token(&self, cache_key: &str) -> Option<String> {
        let cache = self.lock_token_cache();
        cache
            .as_ref()
            .filter(|token| token.cache_key == cache_key && token.expires_at > Instant::now())
            .map(|token| token.header.clone())
    }

    fn lock_token_cache(&self) -> std::sync::MutexGuard<'_, Option<CachedToken>> {
        self.token_cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn endpoint(
        &self,
        settings: &RommSourceSettings,
        path: &str,
        query: &[(String, String)],
    ) -> Result<Url, RommSourceError> {
        let mut url = normalized_base_url(&settings.base_url)?
            .join(path)
            .map_err(|_| RommSourceError::InvalidBaseUrl)?;
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (key, value) in query {
                pairs.append_pair(key, value);
            }
        }
        Ok(url)
    }
}

/// Builds the remote browse query.
///
/// The platform filter is sent under both `platform_ids` (current RomM builds, where the parameter
/// is a repeatable list) and `platform_id` (older builds). RomM is a FastAPI service and ignores
/// query parameters its handler does not declare, so sending both filters correctly on either
/// generation instead of silently returning the whole remote library.
fn browse_query(
    platform_id: Option<i64>,
    search: Option<&str>,
    limit: u32,
    offset: u32,
) -> Vec<(String, String)> {
    let mut query: Vec<(String, String)> = vec![
        ("limit".to_string(), limit.to_string()),
        ("offset".to_string(), offset.to_string()),
        ("order_by".to_string(), "name".to_string()),
        ("order_dir".to_string(), "asc".to_string()),
    ];
    if let Some(platform_id) = platform_id {
        query.push(("platform_ids".to_string(), platform_id.to_string()));
        query.push(("platform_id".to_string(), platform_id.to_string()));
    }
    if let Some(search) = search.filter(|search| !search.trim().is_empty()) {
        query.push(("search_term".to_string(), search.trim().to_string()));
    }
    query
}

/// Validates and normalizes an admin-supplied base URL.
///
/// Only `http` and `https` are accepted, and embedded credentials are refused so a secret can
/// never arrive through the URL field and end up in a log or an error message.
pub(crate) fn normalized_base_url(base_url: &str) -> Result<Url, RommSourceError> {
    let mut url = Url::parse(base_url.trim()).map_err(|_| RommSourceError::InvalidBaseUrl)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.cannot_be_a_base()
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(RommSourceError::InvalidBaseUrl);
    }
    url.set_query(None);
    url.set_fragment(None);
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

pub(crate) fn is_plaintext_base_url(base_url: &str) -> bool {
    normalized_base_url(base_url).is_ok_and(|url| url.scheme() == "http")
}

fn basic_header(username: &str, secret: &str) -> String {
    format!("Basic {}", BASE64.encode(format!("{username}:{secret}")))
}

/// Percent-encodes one path segment without pulling in a new dependency.
fn utf8_percent_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            byte => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn request_error(error: reqwest::Error) -> RommSourceError {
    if error.is_redirect() {
        return RommSourceError::RedirectRefused;
    }
    if error.is_timeout() || error.is_connect() || error.is_request() {
        return RommSourceError::Unreachable;
    }
    if error.is_decode() {
        return RommSourceError::MalformedResponse;
    }
    RommSourceError::Unreachable
}

fn status_error(status: u16) -> RommSourceError {
    match status {
        401 | 403 => RommSourceError::Unauthorized,
        404 => RommSourceError::UpstreamStatus { status },
        301..=308 => RommSourceError::RedirectRefused,
        status => RommSourceError::UpstreamStatus { status },
    }
}

/// Reads a response body, refusing anything beyond the caller's ceiling.
pub(crate) async fn read_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, RommSourceError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(RommSourceError::ResponseTooLarge);
    }
    let mut buffer = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_error)? {
        if buffer.len().saturating_add(chunk.len()) > max_bytes {
            return Err(RommSourceError::ResponseTooLarge);
        }
        buffer.extend_from_slice(&chunk);
    }
    Ok(buffer)
}

/// Maps a transport failure from a streaming download onto Teatro's normalized errors.
pub(crate) fn transport_error(error: reqwest::Error) -> RommSourceError {
    request_error(error)
}

#[derive(Debug)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

#[derive(Debug, Deserialize)]
struct RawTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    expires: Option<u64>,
}

/// Accepts both a bare array and a paginated `{items: [...]}` envelope.
fn as_items(value: &Value) -> Option<&Vec<Value>> {
    value
        .as_array()
        .or_else(|| value.get("items").and_then(Value::as_array))
}

fn extract_version(value: &Value) -> Option<String> {
    for path in [
        &["SYSTEM", "VERSION"][..],
        &["system", "version"][..],
        &["VERSION"][..],
        &["version"][..],
    ] {
        let mut current = value;
        let mut found = true;
        for key in path {
            match current.get(*key) {
                Some(next) => current = next,
                None => {
                    found = false;
                    break;
                }
            }
        }
        if found && let Some(version) = current.as_str() {
            return Some(bounded_remote_text(version));
        }
    }
    None
}

fn parse_platform(value: &Value) -> Option<RemotePlatform> {
    let id = value.get("id").and_then(Value::as_i64)?;
    let slug = value
        .get("slug")
        .and_then(Value::as_str)
        .map(bounded_remote_text)
        .unwrap_or_default();
    let name = value
        .get("name")
        .or_else(|| value.get("display_name"))
        .and_then(Value::as_str)
        .map(bounded_remote_text)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| slug.clone());
    let rom_count = ["rom_count", "roms_count", "rom_count_total"]
        .iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
        .unwrap_or(0)
        .max(0);

    Some(RemotePlatform {
        id,
        slug,
        name,
        rom_count,
    })
}

fn parse_rom(value: &Value) -> Option<RemoteRom> {
    let id = value.get("id").and_then(Value::as_i64)?;
    let name = value
        .get("name")
        .and_then(Value::as_str)
        .map(bounded_remote_text)
        .filter(|name| !name.is_empty())
        .or_else(|| {
            value
                .get("fs_name")
                .and_then(Value::as_str)
                .map(bounded_remote_text)
        })
        .unwrap_or_else(|| format!("Remote game {id}"));
    let files = value.get("files").and_then(Value::as_array);

    Some(RemoteRom {
        id,
        name,
        platform_id: value.get("platform_id").and_then(Value::as_i64),
        platform_slug: value
            .get("platform_slug")
            .and_then(Value::as_str)
            .map(bounded_remote_text),
        platform_name: ["platform_name", "platform_display_name"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_str))
            .map(bounded_remote_text),
        fs_name: value
            .get("fs_name")
            .and_then(Value::as_str)
            .map(bounded_remote_text)
            .filter(|name| !name.is_empty()),
        file_size_bytes: ["fs_size_bytes", "file_size_bytes", "size_bytes"]
            .iter()
            .find_map(|key| value.get(*key).and_then(Value::as_u64)),
        has_cover: ["path_cover_small", "path_cover_large", "url_cover"]
            .iter()
            .any(|key| {
                value
                    .get(*key)
                    .and_then(Value::as_str)
                    .is_some_and(|path| !path.trim().is_empty())
            }),
        file_count: files.map(Vec::len).unwrap_or(1),
    })
}

fn parse_rom_metadata(value: &Value, remote_rom_id: i64) -> RemoteRomMetadata {
    let summary = metadata_text(value, "summary");
    let first_release_date = metadata_i64(value, "first_release_date");
    let release_year = first_release_date
        .and_then(|timestamp| DateTime::<Utc>::from_timestamp(timestamp, 0))
        .map(|date| date.year());
    let regions = metadata_text_list(value, "regions");
    let languages = metadata_text_list(value, "languages");
    let genres = metadata_text_list(value, "genres");
    let developers = metadata_text_list(value, "developers");
    let publishers = metadata_text_list(value, "publishers");
    let provider_ids = remote_provider_ids(value);
    let provenance = json!({
        "remote_rom_id": remote_rom_id,
        "provider_ids": provider_ids,
        "original_name": metadata_text(value, "name"),
        "summary": summary.clone(),
        "first_release_date": first_release_date,
        "release_year": release_year,
        "average_rating": metadata_f64(value, "average_rating"),
        "genres": genres.clone(),
        "developers": developers.clone(),
        "publishers": publishers.clone(),
        "regions": regions.clone(),
        "languages": languages.clone(),
        "platform_id": metadata_i64(value, "platform_id"),
        "platform_slug": metadata_text(value, "platform_slug"),
        "platform_name": metadata_text(value, "platform_name")
            .or_else(|| metadata_text(value, "platform_display_name")),
        "franchises": metadata_text_list(value, "franchises"),
        "collections": metadata_text_list(value, "collections"),
        "companies": metadata_text_list(value, "companies"),
        "game_modes": metadata_text_list(value, "game_modes"),
        "age_ratings": metadata_text_list(value, "age_ratings"),
        "player_count": metadata_text(value, "player_count"),
        "alternative_names": metadata_text_list(value, "alternative_names"),
        "revision": metadata_text(value, "revision"),
        "version": metadata_text(value, "version"),
        "tags": metadata_text_list(value, "tags"),
    });
    let normalized = json!({
        "schema_version": 1,
        "source": "romm",
        "summary": summary,
        "first_release_date": first_release_date,
        "release_year": release_year,
        "rating": metadata_f64(value, "rating")
            .or_else(|| metadata_f64(value, "average_rating")),
        "aggregated_rating": metadata_f64(value, "aggregated_rating"),
        "total_rating": metadata_f64(value, "total_rating")
            .or_else(|| metadata_f64(value, "average_rating")),
        "genres": genres,
        "developers": developers,
        "publishers": publishers,
        "regions": regions,
        "languages": languages,
        "romm": provenance,
    });

    RemoteRomMetadata {
        summary: metadata_text(value, "summary"),
        regions: metadata_text_list(value, "regions"),
        value: normalized,
    }
}

fn metadata_field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key).or_else(|| {
        ["metadatum", "manual_metadata", "igdb_metadata"]
            .iter()
            .find_map(|container| value.get(*container).and_then(|item| item.get(key)))
    })
}

fn metadata_text(value: &Value, key: &str) -> Option<String> {
    metadata_field(value, key)
        .and_then(Value::as_str)
        .map(bounded_remote_text)
        .filter(|text| !text.is_empty())
}

fn metadata_i64(value: &Value, key: &str) -> Option<i64> {
    metadata_field(value, key).and_then(|item| {
        item.as_i64()
            .or_else(|| item.as_str().and_then(|text| text.parse().ok()))
    })
}

fn metadata_f64(value: &Value, key: &str) -> Option<f64> {
    metadata_field(value, key)
        .and_then(|item| {
            item.as_f64()
                .or_else(|| item.as_str().and_then(|text| text.parse().ok()))
        })
        .filter(|number| number.is_finite())
}

fn metadata_text_list(value: &Value, key: &str) -> Vec<String> {
    let mut items = metadata_field(value, key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            item.as_str().or_else(|| {
                ["name", "rating", "category"]
                    .iter()
                    .find_map(|key| item.get(*key).and_then(Value::as_str))
            })
        })
        .map(bounded_remote_text)
        .filter(|text| !text.is_empty())
        .take(MAX_REMOTE_METADATA_ITEMS)
        .collect::<Vec<_>>();
    items.sort();
    items.dedup();
    items
}

fn remote_provider_ids(value: &Value) -> Value {
    let mut ids = Map::new();
    for key in [
        "igdb_id",
        "sgdb_id",
        "moby_id",
        "ss_id",
        "ra_id",
        "launchbox_id",
        "hasheous_id",
        "tgdb_id",
        "hltb_id",
    ] {
        if let Some(id) = value.get(key).and_then(Value::as_i64).filter(|id| *id > 0) {
            ids.insert(key.to_string(), json!(id));
        }
    }
    for key in ["flashpoint_id", "gamelist_id", "libretro_id"] {
        if let Some(id) = value
            .get(key)
            .and_then(Value::as_str)
            .map(bounded_remote_text)
            .filter(|id| !id.is_empty())
        {
            ids.insert(key.to_string(), json!(id));
        }
    }
    Value::Object(ids)
}

fn parse_files(value: &Value) -> Vec<RemoteRomFile> {
    let Some(files) = value.get("files").and_then(Value::as_array) else {
        return Vec::new();
    };
    files
        .iter()
        .filter_map(|file| {
            let id = file.get("id").and_then(Value::as_i64)?;
            let file_name = ["file_name", "fs_name", "name"]
                .iter()
                .find_map(|key| file.get(*key).and_then(Value::as_str))
                .map(bounded_remote_text)
                .filter(|name| !name.is_empty())?;
            Some(RemoteRomFile {
                id,
                file_name,
                file_size_bytes: ["file_size_bytes", "size_bytes", "fs_size_bytes"]
                    .iter()
                    .find_map(|key| file.get(*key).and_then(Value::as_u64)),
                crc32: normalized_hash(hash_field(file, "crc"), 8),
                md5: normalized_hash(hash_field(file, "md5"), 32),
                sha1: normalized_hash(hash_field(file, "sha1"), 40),
                sha256: normalized_hash(hash_field(file, "sha256"), 64),
            })
        })
        .collect()
}

fn hash_field<'a>(file: &'a Value, algorithm: &str) -> Option<&'a str> {
    [
        format!("{algorithm}_hash"),
        algorithm.to_string(),
        format!("{algorithm}sum"),
    ]
    .iter()
    .find_map(|key| file.get(key).and_then(Value::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browse_queries_carry_the_platform_filter_under_both_parameter_names() {
        let query = browse_query(Some(7), None, 24, 0);
        assert!(query.contains(&("platform_ids".to_string(), "7".to_string())));
        assert!(query.contains(&("platform_id".to_string(), "7".to_string())));
        assert!(query.iter().all(|(key, _)| key != "search_term"));

        let query = browse_query(Some(7), Some("  sonic  "), 24, 0);
        assert!(query.contains(&("search_term".to_string(), "sonic".to_string())));
        assert!(query.contains(&("platform_ids".to_string(), "7".to_string())));

        let query = browse_query(None, Some("   "), 24, 48);
        assert!(query.iter().all(|(key, _)| !key.starts_with("platform_id")));
        assert!(query.iter().all(|(key, _)| key != "search_term"));
        assert!(query.contains(&("offset".to_string(), "48".to_string())));
    }

    #[test]
    fn base_urls_reject_non_http_schemes_and_embedded_credentials() {
        assert!(normalized_base_url("file:///etc/passwd").is_err());
        assert!(normalized_base_url("ftp://host/romm").is_err());
        assert!(normalized_base_url("http://user:secret@host/").is_err());
        assert!(normalized_base_url("not a url").is_err());
        let url = normalized_base_url("  http://192.168.1.9:8080/romm?x=1#f  ").unwrap();
        assert_eq!(url.as_str(), "http://192.168.1.9:8080/romm/");
    }

    #[test]
    fn endpoints_stay_under_the_configured_base_path() {
        let client = RommClient::new(&RommSourceConfig::default());
        let settings = RommSourceSettings {
            base_url: "http://host:8080/romm".to_string(),
            username: Some("teatro".to_string()),
            secret: Some("secret".to_string()),
            auth_mode: RommAuthMode::Token,
            updated_at: String::new(),
        };
        let url = client
            .endpoint(
                &settings,
                "api/roms",
                &[("limit".to_string(), "24".to_string())],
            )
            .unwrap();
        assert_eq!(url.as_str(), "http://host:8080/romm/api/roms?limit=24");
    }

    #[test]
    fn remote_file_names_are_percent_encoded_in_download_paths() {
        assert_eq!(
            utf8_percent_segment("Sonic (USA) [!].md"),
            "Sonic%20%28USA%29%20%5B%21%5D.md"
        );
        assert_eq!(
            utf8_percent_segment("../../etc/passwd"),
            "..%2F..%2Fetc%2Fpasswd"
        );
    }

    #[test]
    fn paginated_and_bare_list_envelopes_are_both_accepted() {
        let bare = serde_json::json!([{ "id": 1, "name": "A" }]);
        let paged = serde_json::json!({ "items": [{ "id": 1, "name": "A" }], "total": 9 });
        assert_eq!(as_items(&bare).unwrap().len(), 1);
        assert_eq!(as_items(&paged).unwrap().len(), 1);
        assert!(as_items(&serde_json::json!({ "nope": 1 })).is_none());
    }

    #[test]
    fn remote_rom_parsing_bounds_untrusted_fields() {
        let value = serde_json::json!({
            "id": 7,
            "name": "Game\u{7}Name",
            "platform_id": 3,
            "platform_slug": "snes",
            "fs_name": "game.sfc",
            "fs_size_bytes": 1024,
            "path_cover_small": "assets/romm/resources/cover.png",
            "files": [
                { "id": 11, "file_name": "game.sfc", "file_size_bytes": 1024,
                  "crc_hash": "DEADBEEF", "md5_hash": "zz", "sha1_hash": null }
            ]
        });
        let rom = parse_rom(&value).unwrap();
        assert_eq!(rom.name, "Game\u{fffd}Name");
        assert!(rom.has_cover);
        assert_eq!(rom.file_count, 1);

        let files = parse_files(&value);
        assert_eq!(files[0].crc32.as_deref(), Some("deadbeef"));
        assert_eq!(files[0].md5, None);
        assert_eq!(files[0].hash_signals(), vec!["crc32"]);
    }

    #[test]
    fn romm_metadata_is_normalized_and_bounded() {
        let value = serde_json::json!({
            "id": 7,
            "name": "Chrono Trigger",
            "summary": "A time-travel RPG",
            "platform_id": 3,
            "platform_slug": "snes",
            "platform_display_name": "Super Nintendo",
            "igdb_id": 123,
            "moby_id": 456,
            "regions": ["USA", "Japan", "USA"],
            "languages": ["English", "Japanese"],
            "tags": (0..200).map(|index| format!("tag-{index}")).collect::<Vec<_>>(),
            "metadatum": {
                "genres": ["RPG", "Adventure", "RPG"],
                "companies": ["Square"],
                "game_modes": ["Single player"],
                "age_ratings": [{ "rating": "E" }],
                "first_release_date": 946684800,
                "average_rating": "87.5"
            },
            "moby_metadata": { "unbounded_blob": "not retained" }
        });

        let metadata = parse_rom_metadata(&value, 7);
        assert_eq!(metadata.summary.as_deref(), Some("A time-travel RPG"));
        assert_eq!(metadata.regions, vec!["Japan", "USA"]);
        assert_eq!(metadata.value["release_year"], 2000);
        assert_eq!(metadata.value["total_rating"], 87.5);
        assert_eq!(
            metadata.value["genres"],
            serde_json::json!(["Adventure", "RPG"])
        );
        assert_eq!(metadata.value["romm"]["provider_ids"]["igdb_id"], 123);
        assert_eq!(
            metadata.value["romm"]["companies"],
            serde_json::json!(["Square"])
        );
        assert_eq!(
            metadata.value["romm"]["tags"].as_array().unwrap().len(),
            MAX_REMOTE_METADATA_ITEMS
        );
        assert!(metadata.value["romm"].get("moby_metadata").is_none());
    }

    #[test]
    fn heartbeat_versions_are_read_from_known_shapes() {
        assert_eq!(
            extract_version(&serde_json::json!({ "SYSTEM": { "VERSION": "3.5.0" } })).as_deref(),
            Some("3.5.0")
        );
        assert_eq!(
            extract_version(&serde_json::json!({ "version": "4.0.1" })).as_deref(),
            Some("4.0.1")
        );
        assert_eq!(extract_version(&serde_json::json!({})), None);
    }
}
