use std::collections::BTreeSet;

use axum::{
    Form, Json,
    extract::{RawQuery, State},
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use url::form_urlencoded;

use super::{
    extractors::ApiPath, map_file_store_read_error, map_path_read_error, map_stream_read_error,
};
use crate::{
    domain::{
        platform::Platform,
        rom::{PaginatedRoms, Rom, RomFile, RomFileDependency, RomListParams},
    },
    error::ApiError,
    repositories::{platforms, roms},
    services::library::{
        DOWNLOAD_TICKET_TTL_SECONDS, DownloadArchiveError, DownloadTicketError,
        PreparedDownloadArchive, prepare_download_archive,
    },
    state::AppState,
    storage::streaming,
};

const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 10_000;

pub async fn list_platforms(
    State(state): State<AppState>,
) -> Result<Json<Vec<Platform>>, ApiError> {
    let platforms = platforms::list_with_rom_counts(state.db())
        .await
        .map_err(database_error)?;

    Ok(Json(platforms))
}

pub async fn list_roms(
    State(state): State<AppState>,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<RomListResponse>, ApiError> {
    let query = RomQuery::parse(raw_query.as_deref())?;
    let page = roms::list(
        state.db(),
        RomListParams {
            limit: query.limit,
            offset: query.offset,
            search: query.search.clone(),
            newest_first: query.newest_first,
            missing_cover: query.missing_cover,
        },
        &query.platform_ids,
    )
    .await
    .map_err(database_error)?;

    Ok(Json(RomListResponse::from(page)))
}

pub async fn rom_detail(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<RomResponse>, ApiError> {
    let rom = roms::find_by_id(state.db(), id)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("ROM not found"))?;

    Ok(Json(RomResponse::from(rom)))
}

pub async fn rom_download_plan(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<i64>,
) -> Result<Json<RomDownloadPlanResponse>, ApiError> {
    let rom = roms::find_by_id(state.db(), id)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("ROM not found"))?;
    let preferred_file_id = rom
        .files
        .iter()
        .find(|file| file.launchable)
        .map(|file| file.id)
        .ok_or_else(|| {
            tracing::error!(rom_id = id, "ROM has no launchable download-plan file");
            ApiError::internal("ROM download plan is invalid")
        })?;
    let dependencies = roms::file_dependencies_for_rom(state.db(), id)
        .await
        .map_err(database_error)?;

    Ok(Json(RomDownloadPlanResponse::new(
        rom.id,
        preferred_file_id,
        rom.files,
        dependencies,
    )))
}

pub async fn create_rom_archive_ticket(
    State(state): State<AppState>,
    ApiPath(rom_id): ApiPath<i64>,
) -> Result<Response, ApiError> {
    let archive = prepare_download_archive(&state, rom_id)
        .await
        .map_err(download_archive_error)?;
    let registry = state.download_archive_tickets().clone();
    let ticket = registry.issue(archive).map_err(download_ticket_error)?;
    Ok(download_ticket_response(ticket))
}

pub async fn create_rom_file_ticket(
    State(state): State<AppState>,
    ApiPath((rom_id, file_id)): ApiPath<(i64, i64)>,
) -> Result<Response, ApiError> {
    let file = roms::find_file_by_id(state.db(), rom_id, file_id)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("ROM file not found"))?;
    let mut response = file_download_response(&state, &file, &file.file_name).await?;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    let ticket = state
        .download_file_tickets()
        .issue(response)
        .map_err(download_ticket_error)?;
    Ok(download_ticket_response(ticket))
}

pub async fn download_rom_file_with_ticket(
    State(state): State<AppState>,
    Form(request): Form<DownloadTicketRequest>,
) -> Result<Response, ApiError> {
    state
        .download_file_tickets()
        .consume(&request.ticket)
        .ok_or_else(|| ApiError::not_found("download ticket is invalid or expired"))
}

fn download_ticket_response(ticket: String) -> Response {
    let mut response = Json(DownloadTicketResponse {
        ticket,
        expires_in_seconds: DOWNLOAD_TICKET_TTL_SECONDS,
    })
    .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    response
}

pub async fn download_rom_archive_with_ticket(
    State(state): State<AppState>,
    Form(request): Form<DownloadTicketRequest>,
) -> Result<Response, ApiError> {
    let archive = state
        .download_archive_tickets()
        .consume(&request.ticket)
        .ok_or_else(|| ApiError::not_found("download ticket is invalid or expired"))?;
    download_archive_response(archive)
}

fn download_archive_response(
    PreparedDownloadArchive {
        reader,
        file_name,
        file_size_bytes,
    }: PreparedDownloadArchive,
) -> Result<Response, ApiError> {
    let mut response =
        streaming::stream_reader(reader, file_size_bytes, "application/zip", Some(&file_name))
            .map_err(|error| map_stream_read_error(error, "file"))?;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    Ok(response)
}

#[derive(Debug, Serialize)]
pub struct DownloadTicketResponse {
    ticket: String,
    expires_in_seconds: u64,
}

#[derive(Debug, Deserialize)]
pub struct DownloadTicketRequest {
    ticket: String,
}

pub async fn download_rom_file(
    State(state): State<AppState>,
    ApiPath((rom_id, output_file_name)): ApiPath<(i64, String)>,
    RawQuery(raw_query): RawQuery,
) -> Result<Response, ApiError> {
    let query = RomContentQuery::parse(raw_query.as_deref())?;
    let rom = roms::find_by_id(state.db(), rom_id)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::not_found("ROM not found"))?;

    let file = select_file(&state, &rom, query.file_id).await?;
    let download_name = if output_file_name.is_empty() {
        file.file_name.as_str()
    } else {
        output_file_name.as_str()
    };

    file_download_response(&state, &file, download_name).await
}

async fn file_download_response(
    state: &AppState,
    file: &RomFile,
    download_name: &str,
) -> Result<Response, ApiError> {
    let opened_file = state
        .file_store()
        .open_existing(&file.root_path, &file.relative_path)
        .await
        .map_err(|error| map_file_store_read_error(error, "file", "file path", "path"))?;
    streaming::stream_file(opened_file, "application/octet-stream", Some(download_name))
        .await
        .map_err(|error| map_stream_read_error(error, "file"))
}

async fn select_file(
    state: &AppState,
    rom: &Rom,
    file_id: Option<i64>,
) -> Result<RomFile, ApiError> {
    if let Some(file_id) = file_id {
        return roms::find_file_by_id(state.db(), rom.id, file_id)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::not_found("ROM file not found"));
    }

    match rom.files.as_slice() {
        [] => Err(ApiError::not_found("ROM file not found")),
        [file] => Ok(file.clone()),
        _ => Err(ApiError::bad_request(
            "file_ids is required when a ROM has multiple files",
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RomQuery {
    limit: i64,
    offset: i64,
    platform_ids: Vec<i64>,
    search: Option<String>,
    newest_first: bool,
    missing_cover: bool,
}

impl RomQuery {
    fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let mut limit = DEFAULT_LIMIT;
        let mut offset = 0;
        let mut platform_ids = BTreeSet::new();
        let mut search = None;
        let mut newest_first = false;
        let mut missing_cover = false;

        for (key, value) in query_pairs(raw_query) {
            match key.as_str() {
                "limit" => limit = parse_non_negative_i64("limit", &value)?.min(MAX_LIMIT),
                "offset" => offset = parse_non_negative_i64("offset", &value)?,
                "platform_ids" => {
                    for platform_id in parse_i64_list("platform_ids", &value)? {
                        platform_ids.insert(platform_id);
                    }
                }
                "search" | "q" => {
                    let value = value.trim();
                    search = (!value.is_empty()).then(|| value.to_string());
                }
                "sort" if value == "recent" => newest_first = true,
                "sort" => return Err(ApiError::bad_request("sort must be recent")),
                "missing_cover" if value == "true" => missing_cover = true,
                "missing_cover" if value == "false" => missing_cover = false,
                "missing_cover" => {
                    return Err(ApiError::bad_request("missing_cover must be true or false"));
                }
                _ => {}
            }
        }

        Ok(Self {
            limit,
            offset,
            platform_ids: platform_ids.into_iter().collect(),
            search,
            newest_first,
            missing_cover,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RomContentQuery {
    file_id: Option<i64>,
}

impl RomContentQuery {
    fn parse(raw_query: Option<&str>) -> Result<Self, ApiError> {
        let mut file_id = None;

        for (key, value) in query_pairs(raw_query) {
            if key.as_str() != "file_ids" {
                continue;
            }

            let file_ids = parse_i64_list("file_ids", &value)?;
            match file_ids.as_slice() {
                [] => {}
                [selected] => file_id = Some(*selected),
                _ => {
                    return Err(ApiError::bad_request(
                        "only one file_ids value is supported for downloads",
                    ));
                }
            }
        }

        Ok(Self { file_id })
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

fn parse_non_negative_i64(name: &'static str, value: &str) -> Result<i64, ApiError> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| ApiError::bad_request(format!("{name} must be an integer")))?;

    if parsed < 0 {
        return Err(ApiError::bad_request(format!(
            "{name} must be greater than or equal to zero"
        )));
    }

    Ok(parsed)
}

fn parse_i64_list(name: &'static str, value: &str) -> Result<Vec<i64>, ApiError> {
    value
        .split(',')
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse::<i64>()
                .map_err(|_| ApiError::bad_request(format!("{name} must contain integers")))
        })
        .collect()
}

#[derive(Debug, Serialize)]
pub struct RomListResponse {
    items: Vec<RomResponse>,
    total: i64,
    limit: i64,
    offset: i64,
}

impl From<PaginatedRoms> for RomListResponse {
    fn from(page: PaginatedRoms) -> Self {
        Self {
            items: page.items.into_iter().map(RomResponse::from).collect(),
            total: page.total,
            limit: page.limit,
            offset: page.offset,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RomResponse {
    id: i64,
    name: String,
    slug: String,
    platform_id: i64,
    platform_slug: String,
    platform_display_name: String,
    regions: Vec<String>,
    metadatum: serde_json::Value,
    summary: Option<String>,
    fs_name: Option<String>,
    fs_size_bytes: Option<i64>,
    path_cover_large: Option<String>,
    path_cover_small: Option<String>,
    url_cover: Option<String>,
    files: Vec<RomFileResponse>,
}

impl From<Rom> for RomResponse {
    fn from(rom: Rom) -> Self {
        let metadata = rom.metadata;
        Self {
            id: rom.id,
            name: rom.name,
            slug: rom.slug,
            platform_id: rom.platform_id,
            platform_slug: rom.platform_slug,
            platform_display_name: rom.platform_display_name,
            regions: rom.regions,
            metadatum: metadata,
            summary: rom.summary,
            fs_name: rom.fs_name,
            fs_size_bytes: rom.fs_size_bytes,
            path_cover_large: rom.path_cover_large,
            path_cover_small: rom.path_cover_small,
            url_cover: rom.url_cover,
            files: rom.files.into_iter().map(RomFileResponse::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RomFileResponse {
    id: i64,
    file_name: String,
    file_size_bytes: i64,
    role: String,
    launchable: bool,
    disc_index: Option<i64>,
    group_id: Option<i64>,
}

impl From<RomFile> for RomFileResponse {
    fn from(file: RomFile) -> Self {
        Self {
            id: file.id,
            file_name: file.file_name,
            file_size_bytes: file.file_size_bytes,
            role: file.role.to_string(),
            launchable: file.launchable,
            disc_index: file.disc_index,
            group_id: file.group_id,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RomDownloadPlanResponse {
    rom_id: i64,
    preferred_file_id: i64,
    files: Vec<RomFileResponse>,
    dependencies: Vec<RomFileDependencyResponse>,
}

impl RomDownloadPlanResponse {
    fn new(
        rom_id: i64,
        preferred_file_id: i64,
        files: Vec<RomFile>,
        dependencies: Vec<RomFileDependency>,
    ) -> Self {
        Self {
            rom_id,
            preferred_file_id,
            files: files.into_iter().map(RomFileResponse::from).collect(),
            dependencies: dependencies
                .into_iter()
                .map(RomFileDependencyResponse::from)
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RomFileDependencyResponse {
    parent_file_id: i64,
    child_file_id: i64,
    dependency_kind: String,
    sort_index: i64,
}

impl From<RomFileDependency> for RomFileDependencyResponse {
    fn from(dependency: RomFileDependency) -> Self {
        Self {
            parent_file_id: dependency.parent_file_id,
            child_file_id: dependency.child_file_id,
            dependency_kind: dependency.dependency_kind.to_string(),
            sort_index: dependency.sort_index,
        }
    }
}

fn download_ticket_error(error: DownloadTicketError) -> ApiError {
    match error {
        DownloadTicketError::Busy => ApiError::too_many_requests(error.to_string(), 1),
    }
}

fn download_archive_error(error: DownloadArchiveError) -> ApiError {
    match error {
        DownloadArchiveError::RomNotFound => ApiError::not_found(error.to_string()),
        DownloadArchiveError::RequiresMultipleFiles => ApiError::bad_request(error.to_string()),
        DownloadArchiveError::TooManyFiles { .. } | DownloadArchiveError::TooLarge { .. } => {
            ApiError::payload_too_large(error.to_string())
        }
        DownloadArchiveError::Busy => ApiError::too_many_requests(error.to_string(), 1),
        DownloadArchiveError::InsufficientStorage => {
            ApiError::insufficient_storage(error.to_string())
        }
        DownloadArchiveError::InvalidFileName | DownloadArchiveError::SourceChanged => {
            ApiError::new(
                axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable_entity",
                error.to_string(),
            )
        }
        DownloadArchiveError::PathSafety(error) => {
            map_path_read_error(error, "file", "file path", "path")
        }
        DownloadArchiveError::FileStore(error) => {
            map_file_store_read_error(error, "file", "file path", "path")
        }
        DownloadArchiveError::Database(error) => database_error(error),
        DownloadArchiveError::Io(_)
        | DownloadArchiveError::Zip(_)
        | DownloadArchiveError::WorkerFailed => {
            tracing::error!(?error, "failed to prepare ROM download archive");
            ApiError::internal("failed to prepare game archive")
        }
    }
}

fn database_error(error: sqlx::Error) -> ApiError {
    tracing::error!(?error, "ROMM repository operation failed");
    ApiError::internal("ROMM API operation failed")
}

#[cfg(test)]
mod tests {
    use super::{RomContentQuery, RomQuery};

    #[test]
    fn parses_rom_query_with_repeated_and_comma_platform_ids() {
        let query = RomQuery::parse(Some(
            "limit=20000&offset=10&platform_ids=1,2&platform_ids=3&missing_cover=true",
        ))
        .unwrap();

        assert_eq!(query.limit, 10_000);
        assert_eq!(query.offset, 10);
        assert_eq!(query.platform_ids, vec![1, 2, 3]);
        assert!(!query.newest_first);
        assert!(query.missing_cover);
        assert!(RomQuery::parse(Some("missing_cover=yes")).is_err());

        let recent = RomQuery::parse(Some("limit=10&sort=recent")).unwrap();
        assert!(recent.newest_first);
    }

    #[test]
    fn parses_file_ids_query() {
        let query = RomContentQuery::parse(Some("file_ids=42")).unwrap();

        assert_eq!(query.file_id, Some(42));
    }
}
