use url::form_urlencoded;

use crate::{
    error::ApiError,
    services::{igdb as igdb_service, romm_source},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GogImportJobQuery {
    pub(super) after: u64,
}

impl GogImportJobQuery {
    pub(super) fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let mut after = None;
        for (key, value) in query_pairs(raw_query) {
            if key != "after" {
                continue;
            }
            if after.is_some() {
                return Err(ApiError::bad_request(
                    "after query parameter may only be supplied once",
                ));
            }
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(ApiError::bad_request(
                    "after query parameter must be an unsigned base-10 integer",
                ));
            }
            after = Some(value.parse::<u64>().map_err(|_| {
                ApiError::bad_request("after query parameter must be an unsigned base-10 integer")
            })?);
        }
        Ok(Self {
            after: after.unwrap_or(0),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct IgdbSearchQuery {
    pub(super) q: String,
    pub(super) limit: u32,
    pub(super) platform_slug: Option<String>,
    pub(super) require_platform_match: bool,
}

impl IgdbSearchQuery {
    pub(super) fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let mut q = None;
        let mut limit = igdb_service::DEFAULT_SEARCH_LIMIT;
        let mut platform_slug = None;
        let mut require_platform_match = false;

        for (key, value) in query_pairs(raw_query) {
            match key.as_str() {
                "q" | "query" => q = Some(value.trim().to_string()),
                "limit" => limit = parse_u32("limit", &value)?,
                "platform" | "platform_slug" => platform_slug = Some(value.trim().to_string()),
                "require_platform_match" => {
                    require_platform_match = parse_bool("require_platform_match", &value)?
                }
                _ => {}
            }
        }

        let q = q
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::bad_request("q query parameter is required"))?;
        let platform_slug = platform_slug.filter(|value| !value.is_empty());

        if limit == 0 || limit > igdb_service::MAX_SEARCH_LIMIT {
            return Err(ApiError::bad_request(format!(
                "limit must be between 1 and {}",
                igdb_service::MAX_SEARCH_LIMIT
            )));
        }

        Ok(Self {
            q,
            limit,
            platform_slug,
            require_platform_match,
        })
    }
}

/// Bounded browse query for the RomM source proxy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RommBrowseQuery {
    pub(super) platform_id: Option<i64>,
    pub(super) search: Option<String>,
    pub(super) limit: u32,
    pub(super) offset: u32,
}

impl RommBrowseQuery {
    pub(super) fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let mut platform_id = None;
        let mut search = None;
        let mut limit = romm_source::DEFAULT_BROWSE_LIMIT;
        let mut offset = 0_u32;

        for (key, value) in query_pairs(raw_query) {
            match key.as_str() {
                "platform_id" => {
                    platform_id =
                        Some(value.trim().parse::<i64>().map_err(|_| {
                            ApiError::bad_request("platform_id must be an integer")
                        })?);
                }
                "search" | "q" => search = Some(value.trim().to_string()),
                "limit" => limit = parse_u32("limit", &value)?,
                "offset" => offset = parse_u32("offset", &value)?,
                _ => {}
            }
        }

        if limit == 0 || limit > romm_source::MAX_BROWSE_LIMIT {
            return Err(ApiError::bad_request(format!(
                "limit must be between 1 and {}",
                romm_source::MAX_BROWSE_LIMIT
            )));
        }
        Ok(Self {
            platform_id,
            search: search.filter(|value| !value.is_empty()),
            limit,
            offset,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BulkDeleteConfirmQuery {
    pub(super) confirm: String,
}

impl BulkDeleteConfirmQuery {
    pub(super) fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let confirm = query_pairs(raw_query)
            .into_iter()
            .find_map(|(key, value)| (key == "confirm").then_some(value))
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ApiError::bad_request("confirm query parameter is required"))?;

        Ok(Self { confirm })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DeleteRomQuery {
    pub(super) delete_files: bool,
}

impl DeleteRomQuery {
    pub(super) fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        for (key, value) in query_pairs(raw_query) {
            if key != "delete_files" {
                continue;
            }

            let delete_files = parse_bool("delete_files", &value)?;
            if !delete_files {
                return Err(ApiError::bad_request(
                    "delete_files=false is not supported; deleting a ROM also removes its managed filesystem entries",
                ));
            }
        }

        Ok(Self { delete_files: true })
    }
}

fn query_pairs(raw_query: Option<&str>) -> Vec<(String, String)> {
    raw_query
        .map(|query| {
            form_urlencoded::parse(query.as_bytes())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default()
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, ApiError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(ApiError::bad_request(format!(
            "{name} must be true or false"
        ))),
    }
}

fn parse_u32(name: &'static str, value: &str) -> Result<u32, ApiError> {
    value
        .parse::<u32>()
        .map_err(|_| ApiError::bad_request(format!("{name} must be an integer")))
}

pub(super) fn normalize_igdb_setting(
    name: &'static str,
    value: &str,
    max_len: usize,
) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ApiError::bad_request(format!("{name} cannot be empty")));
    }
    if value.len() > max_len
        || value
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(ApiError::bad_request(format!("{name} is invalid")));
    }

    Ok(value.to_string())
}
