use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    api::romm::RomResponse,
    domain::{
        api_token::{ApiToken, ApiTokenScope, CreatedApiToken},
        library::{LibraryRootStats, LibraryStats, PlatformLibraryStats},
        rom::{RomFile, RomFileDependency, RomFileGroup},
        user::{PublicUser, UserRole},
        workflow::FileHashStatus,
    },
    services::{
        gog_import::{
            GogImportJobEvent, GogImportJobSnapshot, GogImportOutcome, GogImportProgress,
            GogImportSummary,
        },
        igdb::{self as igdb_service, CachedCoverAsset, IgdbGameCandidate},
        library::{
            self, BulkDeleteRomsOutcome, BulkDeleteScope, DeleteRomOutcome, IngestPlanWarning,
            LibraryScanJobProgress, LibraryScanJobSnapshot, LibraryScanResult, SidecarCleanupFile,
            UploadPreviewDraft, UploadPreviewFileDraft, UploadedBatch, UploadedRom,
        },
        romm_source::{
            RemotePlatform, RemoteRom, RemoteRomFile, RommConnection, RommImportJobProgress,
            RommImportJobSnapshot, RommImportOutcome, RommSourceStatus,
        },
    },
};

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub(super) username: String,
    pub(super) password: String,
    #[serde(default)]
    pub(super) role: Option<UserRole>,
}

#[derive(Debug, Deserialize)]
pub struct ResetPasswordRequest {
    pub(super) password: String,
}

#[derive(Debug, Deserialize)]
pub struct SetRoleRequest {
    pub(super) role: UserRole,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiTokenRequest {
    #[serde(default)]
    pub(super) user_id: Option<i64>,
    #[serde(default)]
    pub(super) username: Option<String>,
    pub(super) name: String,
    #[serde(default)]
    pub(super) scopes: Option<Vec<String>>,
    #[serde(default)]
    pub(super) expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApiTokenResponse {
    id: i64,
    user: PublicUser,
    name: String,
    token_prefix: String,
    scopes: Vec<ApiTokenScope>,
    expires_at: Option<String>,
    revoked_at: Option<String>,
    last_used_at: Option<String>,
    created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CreatedApiTokenResponse {
    #[serde(flatten)]
    token_record: ApiTokenResponse,
    token: String,
}

#[derive(Debug, Deserialize)]
pub struct UploadPreviewRequest {
    #[serde(default)]
    platform_id: Option<i64>,
    #[serde(default)]
    platform_slug: Option<String>,
    #[serde(default, alias = "name")]
    title: Option<String>,
    files: Vec<UploadPreviewFileRequest>,
}

#[derive(Debug, Deserialize)]
pub struct UploadPreviewFileRequest {
    #[serde(alias = "name", alias = "filename")]
    file_name: String,
    #[serde(default, alias = "size")]
    file_size_bytes: Option<u64>,
    #[serde(default)]
    manifest_contents: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateIntegrityJobRequest {
    #[serde(default)]
    pub(super) rom_id: Option<i64>,
    #[serde(default)]
    pub(super) force: bool,
}

#[derive(Debug, Serialize)]
pub struct UploadRomResponse {
    rom: RomResponse,
    file_id: i64,
    file_name: String,
    relative_path: String,
    file_size_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct UploadBatchResponse {
    roms: Vec<RomResponse>,
    warnings: Vec<IngestPlanWarning>,
}

#[derive(Debug, Serialize)]
pub struct GogImportResponse {
    #[serde(flatten)]
    upload: UploadRomResponse,
    #[serde(rename = "import")]
    import_summary: GogImportSummary,
}

#[derive(Debug, Deserialize)]
pub struct SidecarCleanupRequest {
    pub(super) confirm: String,
    pub(super) files: Vec<SidecarCleanupFile>,
}

#[derive(Debug, Serialize)]
pub struct LibraryScanJobCreateResponse {
    job: LibraryScanJobSummaryResponse,
    status_url: String,
}

#[derive(Debug, Serialize)]
struct LibraryScanJobSummaryResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<LibraryScanProgressResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct LibraryScanJobStatusResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<LibraryScanProgressResponse>,
    result: Option<LibraryScanResult>,
    error: Option<LibraryScanJobErrorResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct LibraryScanProgressResponse {
    kind: String,
    current: u64,
    total: u64,
    percent: f64,
}

#[derive(Debug, Serialize)]
struct LibraryScanJobErrorResponse {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
pub struct GogImportJobCreateResponse {
    job: GogImportJobSummaryResponse,
    status_url: String,
}

#[derive(Debug, Serialize)]
struct GogImportJobSummaryResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<GogImportProgressResponse>,
    output_truncated: bool,
    last_event_seq: u64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct GogImportJobStatusResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<GogImportProgressResponse>,
    events: Vec<GogImportJobEventResponse>,
    next_event_seq: u64,
    output_truncated: bool,
    result: Option<GogImportResponse>,
    error: Option<GogImportJobErrorResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct GogImportProgressResponse {
    kind: String,
    current: Option<u64>,
    total: Option<u64>,
    percent: f64,
}

#[derive(Debug, Serialize)]
struct GogImportJobEventResponse {
    seq: u64,
    kind: String,
    phase: String,
    stream: Option<String>,
    text: String,
}

#[derive(Debug, Serialize)]
struct GogImportJobErrorResponse {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
pub struct DeleteRomResponse {
    rom: RomResponse,
    delete_files: bool,
    deleted_files: Vec<String>,
    missing_files: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BulkDeleteRomsResponse {
    scope: BulkDeleteScopeResponse,
    deleted_roms: Vec<BulkDeletedRomResponse>,
    deleted_rom_count: usize,
    delete_files: bool,
    deleted_files: Vec<String>,
    missing_files: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BulkDeleteScopeResponse {
    Platform {
        id: i64,
        slug: String,
        display_name: String,
    },
    All,
}

#[derive(Debug, Serialize)]
pub struct BulkDeletedRomResponse {
    id: i64,
    name: String,
    platform_id: i64,
    platform_slug: String,
}

#[derive(Debug, Serialize)]
pub struct RomFilesResponse {
    rom_id: i64,
    groups: Vec<RomFileGroupResponse>,
    ungrouped_files: Vec<AdminRomFileResponse>,
    dependencies: Vec<RomFileDependencyResponse>,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct RomFileGroupResponse {
    id: i64,
    rom_id: i64,
    kind: String,
    display_name: String,
    group_key: Option<String>,
    disc_index: Option<i64>,
    disc_count: Option<i64>,
    launchable: bool,
    metadata: serde_json::Value,
    files: Vec<AdminRomFileResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminRomFileResponse {
    id: i64,
    rom_id: i64,
    root_id: i64,
    group_id: Option<i64>,
    relative_path: String,
    file_name: String,
    original_file_name: Option<String>,
    file_size_bytes: i64,
    crc32: Option<String>,
    md5: Option<String>,
    sha1: Option<String>,
    sha256: Option<String>,
    hash_status: FileHashStatus,
    hashed_at: Option<String>,
    hash_error: Option<String>,
    role: String,
    sort_index: i64,
    disc_index: Option<i64>,
    track_index: Option<i64>,
    launchable: bool,
    is_primary: bool,
    metadata: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct RomFileDependencyResponse {
    parent_file_id: i64,
    child_file_id: i64,
    dependency_kind: String,
    sort_index: i64,
}

#[derive(Debug, Serialize)]
pub struct LibraryStatsResponse {
    total_roms: i64,
    total_files: i64,
    total_file_bytes: i64,
    platforms_with_roms: i64,
    library_roots: Vec<LibraryRootStatsResponse>,
    platforms: Vec<PlatformLibraryStatsResponse>,
}

#[derive(Debug, Serialize)]
pub struct LibraryRootStatsResponse {
    id: i64,
    name: String,
    root_path: String,
    writable: bool,
    file_count: i64,
    total_file_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct PlatformLibraryStatsResponse {
    id: i64,
    slug: String,
    display_name: String,
    rom_count: i64,
    file_count: i64,
    total_file_bytes: i64,
}

#[derive(Debug, Serialize)]
pub struct IgdbStatusResponse {
    configured: bool,
    client_id_configured: bool,
    client_secret_configured: bool,
    token_cached: bool,
}

#[derive(Debug, Serialize)]
pub struct IgdbSettingsResponse {
    pub(super) configured: bool,
    pub(super) client_id_configured: bool,
    pub(super) client_secret_configured: bool,
    pub(super) token_cached: bool,
    pub(super) client_id_source: &'static str,
    pub(super) client_secret_source: &'static str,
    pub(super) updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SaveIgdbSettingsRequest {
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ApplyIgdbMetadataRequest {
    #[serde(rename = "match", alias = "candidate")]
    pub(super) selected_match: IgdbGameCandidate,
    #[serde(default = "default_true")]
    pub(super) cache_cover: bool,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRomRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    platform_id: Option<i64>,
    #[serde(default)]
    platform_slug: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    summary: Option<Option<String>>,
    #[serde(default)]
    regions: Option<Vec<String>>,
    #[serde(default)]
    genres: Option<Vec<String>>,
    #[serde(default)]
    developers: Option<Vec<String>>,
    #[serde(default)]
    publishers: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_optional_field")]
    release_year: Option<Option<i32>>,
}

#[derive(Debug, Serialize)]
pub struct ApplyIgdbMetadataResponse {
    pub(super) rom: RomResponse,
    pub(super) cached_covers: Vec<CachedCoverAsset>,
}

fn default_true() -> bool {
    true
}

fn deserialize_optional_field<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

impl From<ApiToken> for ApiTokenResponse {
    fn from(token: ApiToken) -> Self {
        Self {
            id: token.id,
            user: token.user,
            name: token.name,
            token_prefix: token.token_prefix,
            scopes: token.scopes,
            expires_at: token.expires_at,
            revoked_at: token.revoked_at,
            last_used_at: token.last_used_at,
            created_at: token.created_at,
        }
    }
}

impl From<CreatedApiToken> for CreatedApiTokenResponse {
    fn from(created: CreatedApiToken) -> Self {
        Self {
            token_record: ApiTokenResponse::from(created.record),
            token: created.token,
        }
    }
}

impl From<UploadPreviewRequest> for UploadPreviewDraft {
    fn from(request: UploadPreviewRequest) -> Self {
        Self {
            platform_id: request.platform_id,
            platform_slug: request.platform_slug,
            title: request.title,
            files: request
                .files
                .into_iter()
                .map(|file| UploadPreviewFileDraft {
                    original_file_name: file.file_name,
                    file_size_bytes: file.file_size_bytes,
                    manifest_contents: file.manifest_contents,
                })
                .collect(),
        }
    }
}

impl From<UploadedRom> for UploadRomResponse {
    fn from(uploaded: UploadedRom) -> Self {
        Self {
            rom: RomResponse::from(uploaded.rom),
            file_id: uploaded.file_id,
            file_name: uploaded.file_name,
            relative_path: uploaded.relative_path,
            file_size_bytes: uploaded.file_size_bytes,
        }
    }
}

impl From<UploadedBatch> for UploadBatchResponse {
    fn from(uploaded: UploadedBatch) -> Self {
        Self {
            roms: uploaded.roms.into_iter().map(RomResponse::from).collect(),
            warnings: uploaded.warnings,
        }
    }
}

impl From<GogImportOutcome> for GogImportResponse {
    fn from(outcome: GogImportOutcome) -> Self {
        Self {
            upload: UploadRomResponse::from(outcome.uploaded),
            import_summary: outcome.summary,
        }
    }
}

impl LibraryScanJobCreateResponse {
    pub(super) fn new(snapshot: LibraryScanJobSnapshot, status_url: String) -> Self {
        Self {
            job: LibraryScanJobSummaryResponse {
                id: snapshot.id,
                state: snapshot.state.to_string(),
                phase: snapshot.phase.to_string(),
                progress: snapshot.progress.map(LibraryScanProgressResponse::from),
                created_at: snapshot.created_at,
                updated_at: snapshot.updated_at,
            },
            status_url,
        }
    }
}

impl From<LibraryScanJobSnapshot> for LibraryScanJobStatusResponse {
    fn from(snapshot: LibraryScanJobSnapshot) -> Self {
        Self {
            id: snapshot.id,
            state: snapshot.state.to_string(),
            phase: snapshot.phase.to_string(),
            progress: snapshot.progress.map(LibraryScanProgressResponse::from),
            result: snapshot.result,
            error: snapshot.error.map(|error| LibraryScanJobErrorResponse {
                code: error.code,
                message: error.message,
            }),
            created_at: snapshot.created_at,
            updated_at: snapshot.updated_at,
        }
    }
}

impl From<LibraryScanJobProgress> for LibraryScanProgressResponse {
    fn from(progress: LibraryScanJobProgress) -> Self {
        Self {
            kind: "batches".to_string(),
            current: progress.current,
            total: progress.total,
            percent: progress.percent,
        }
    }
}

impl GogImportJobCreateResponse {
    pub(super) fn new(snapshot: GogImportJobSnapshot, status_url: String) -> Self {
        Self {
            job: GogImportJobSummaryResponse {
                id: snapshot.id.to_string(),
                state: snapshot.state.as_str().to_string(),
                phase: snapshot.phase.as_str().to_string(),
                progress: snapshot.progress.map(GogImportProgressResponse::from),
                output_truncated: snapshot.output_truncated,
                last_event_seq: snapshot.next_event_seq,
                created_at: snapshot.created_at,
                updated_at: snapshot.updated_at,
            },
            status_url,
        }
    }
}

impl From<GogImportJobSnapshot> for GogImportJobStatusResponse {
    fn from(snapshot: GogImportJobSnapshot) -> Self {
        Self {
            id: snapshot.id.to_string(),
            state: snapshot.state.as_str().to_string(),
            phase: snapshot.phase.as_str().to_string(),
            progress: snapshot.progress.map(GogImportProgressResponse::from),
            events: snapshot
                .events
                .into_iter()
                .map(GogImportJobEventResponse::from)
                .collect(),
            next_event_seq: snapshot.next_event_seq,
            output_truncated: snapshot.output_truncated,
            result: snapshot
                .result
                .map(|result| GogImportResponse::from(result.outcome)),
            error: snapshot.error.map(|error| GogImportJobErrorResponse {
                code: error.code,
                message: error.message,
            }),
            created_at: snapshot.created_at,
            updated_at: snapshot.updated_at,
        }
    }
}

impl From<GogImportProgress> for GogImportProgressResponse {
    fn from(progress: GogImportProgress) -> Self {
        Self {
            kind: progress.kind.as_str().to_string(),
            current: progress.current,
            total: progress.total,
            percent: progress.percent,
        }
    }
}

impl From<GogImportJobEvent> for GogImportJobEventResponse {
    fn from(event: GogImportJobEvent) -> Self {
        Self {
            seq: event.seq,
            kind: event.kind.as_str().to_string(),
            phase: event.phase.as_str().to_string(),
            stream: event.stream.map(|stream| stream.as_str().to_string()),
            text: event.text,
        }
    }
}

impl From<DeleteRomOutcome> for DeleteRomResponse {
    fn from(outcome: DeleteRomOutcome) -> Self {
        Self {
            rom: RomResponse::from(outcome.rom),
            delete_files: outcome.delete_files,
            deleted_files: outcome.deleted_files,
            missing_files: outcome.missing_files,
        }
    }
}

impl From<BulkDeleteRomsOutcome> for BulkDeleteRomsResponse {
    fn from(outcome: BulkDeleteRomsOutcome) -> Self {
        let scope = match outcome.scope {
            BulkDeleteScope::Platform(platform) => BulkDeleteScopeResponse::Platform {
                id: platform.id,
                slug: platform.slug,
                display_name: platform.display_name,
            },
            BulkDeleteScope::All => BulkDeleteScopeResponse::All,
        };
        let deleted_rom_count = outcome.deleted_roms.len();

        Self {
            scope,
            deleted_roms: outcome
                .deleted_roms
                .into_iter()
                .map(|rom| BulkDeletedRomResponse {
                    id: rom.id,
                    name: rom.name,
                    platform_id: rom.platform_id,
                    platform_slug: rom.platform_slug,
                })
                .collect(),
            deleted_rom_count,
            delete_files: outcome.delete_files,
            deleted_files: outcome.deleted_files,
            missing_files: outcome.missing_files,
        }
    }
}

impl RomFilesResponse {
    pub(super) fn new(
        rom_id: i64,
        groups: Vec<RomFileGroup>,
        files: Vec<RomFile>,
        dependencies: Vec<RomFileDependency>,
    ) -> Self {
        let mut files_by_group: BTreeMap<i64, Vec<AdminRomFileResponse>> = BTreeMap::new();
        let mut ungrouped_files = Vec::new();

        for file in files {
            match file.group_id {
                Some(group_id) => files_by_group
                    .entry(group_id)
                    .or_default()
                    .push(AdminRomFileResponse::from(file)),
                None => ungrouped_files.push(AdminRomFileResponse::from(file)),
            }
        }

        let groups = groups
            .into_iter()
            .map(|group| {
                let files = files_by_group.remove(&group.id).unwrap_or_default();
                RomFileGroupResponse::from_group_and_files(group, files)
            })
            .collect();

        let mut orphaned_group_files: Vec<AdminRomFileResponse> =
            files_by_group.into_values().flatten().collect();
        ungrouped_files.append(&mut orphaned_group_files);

        Self {
            rom_id,
            groups,
            ungrouped_files,
            dependencies: dependencies
                .into_iter()
                .map(RomFileDependencyResponse::from)
                .collect(),
            warnings: Vec::new(),
        }
    }
}

impl RomFileGroupResponse {
    fn from_group_and_files(group: RomFileGroup, files: Vec<AdminRomFileResponse>) -> Self {
        Self {
            id: group.id,
            rom_id: group.rom_id,
            kind: group.kind.to_string(),
            display_name: group.display_name,
            group_key: group.group_key,
            disc_index: group.disc_index,
            disc_count: group.disc_count,
            launchable: group.launchable,
            metadata: group.metadata,
            files,
        }
    }
}

impl From<RomFile> for AdminRomFileResponse {
    fn from(file: RomFile) -> Self {
        Self {
            id: file.id,
            rom_id: file.rom_id,
            root_id: file.root_id,
            group_id: file.group_id,
            relative_path: file.relative_path,
            file_name: file.file_name,
            original_file_name: file.original_file_name,
            file_size_bytes: file.file_size_bytes,
            crc32: file.crc32,
            md5: file.md5,
            sha1: file.sha1,
            sha256: file.sha256,
            hash_status: file.hash_status,
            hashed_at: file.hashed_at,
            hash_error: file.hash_error,
            role: file.role.to_string(),
            sort_index: file.sort_index,
            disc_index: file.disc_index,
            track_index: file.track_index,
            launchable: file.launchable,
            is_primary: file.is_primary,
            metadata: file.metadata,
        }
    }
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

impl From<LibraryStats> for LibraryStatsResponse {
    fn from(stats: LibraryStats) -> Self {
        Self {
            total_roms: stats.total_roms,
            total_files: stats.total_files,
            total_file_bytes: stats.total_file_bytes,
            platforms_with_roms: stats.platforms_with_roms,
            library_roots: stats
                .library_roots
                .into_iter()
                .map(LibraryRootStatsResponse::from)
                .collect(),
            platforms: stats
                .platforms
                .into_iter()
                .map(PlatformLibraryStatsResponse::from)
                .collect(),
        }
    }
}

impl From<LibraryRootStats> for LibraryRootStatsResponse {
    fn from(stats: LibraryRootStats) -> Self {
        Self {
            id: stats.id,
            name: stats.name,
            root_path: stats.root_path.display().to_string(),
            writable: stats.writable,
            file_count: stats.file_count,
            total_file_bytes: stats.total_file_bytes,
        }
    }
}

impl From<PlatformLibraryStats> for PlatformLibraryStatsResponse {
    fn from(stats: PlatformLibraryStats) -> Self {
        Self {
            id: stats.id,
            slug: stats.slug,
            display_name: stats.display_name,
            rom_count: stats.rom_count,
            file_count: stats.file_count,
            total_file_bytes: stats.total_file_bytes,
        }
    }
}

impl From<igdb_service::IgdbStatus> for IgdbStatusResponse {
    fn from(status: igdb_service::IgdbStatus) -> Self {
        Self {
            configured: status.configured,
            client_id_configured: status.client_id_configured,
            client_secret_configured: status.client_secret_configured,
            token_cached: status.token_cached,
        }
    }
}

impl From<UpdateRomRequest> for library::UpdateRomDraft {
    fn from(request: UpdateRomRequest) -> Self {
        Self {
            name: request.name,
            platform_id: request.platform_id,
            platform_slug: request.platform_slug,
            summary: request.summary,
            regions: request.regions,
            genres: request.genres,
            developers: request.developers,
            publishers: request.publishers,
            release_year: request.release_year,
        }
    }
}

// ---------------------------------------------------------------------------
// RomM source (Teatro as an authenticated client of a remote RomM server)
//
// These are Teatro-shaped DTOs, never passthrough RomM JSON: the browser must not parse upstream
// envelopes, and a RomM upgrade must not be able to change Teatro's admin contract. The stored
// secret is never present in any response below.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct RommSourceStatusResponse {
    pub(super) enabled: bool,
    pub(super) configured: bool,
    pub(super) base_url: Option<String>,
    pub(super) username: Option<String>,
    pub(super) auth_mode: &'static str,
    pub(super) secret_configured: bool,
    pub(super) plaintext_http: bool,
    pub(super) updated_at: Option<String>,
    pub(super) index_refreshed_at: Option<String>,
    pub(super) index_game_count: i64,
    pub(super) index_platform_count: i64,
}

impl From<RommSourceStatus> for RommSourceStatusResponse {
    fn from(status: RommSourceStatus) -> Self {
        Self {
            enabled: status.enabled,
            configured: status.configured,
            base_url: status.base_url,
            username: status.username,
            auth_mode: status.auth_mode,
            secret_configured: status.secret_configured,
            plaintext_http: status.plaintext_http,
            updated_at: status.updated_at,
            index_refreshed_at: status.index_refreshed_at,
            index_game_count: status.index_game_count,
            index_platform_count: status.index_platform_count,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SaveRommSourceRequest {
    pub(super) base_url: String,
    #[serde(default)]
    pub(super) username: Option<String>,
    /// Omit to keep the stored secret unchanged; the secret is never read back.
    #[serde(default)]
    pub(super) secret: Option<String>,
    #[serde(default)]
    pub(super) auth_mode: Option<String>,
    /// Required when the base URL is plaintext `http://`.
    #[serde(default)]
    pub(super) acknowledge_plaintext_http: bool,
}

#[derive(Debug, Serialize)]
pub struct RommSourceTestResponse {
    /// `reachable`, `unauthorized`, or `unreachable`.
    pub(super) result: &'static str,
    pub(super) version: Option<String>,
    pub(super) username: Option<String>,
    pub(super) plaintext_http: bool,
    pub(super) message: String,
}

impl RommSourceTestResponse {
    pub(super) fn new(connection: RommConnection, plaintext_http: bool) -> Self {
        let (result, message) = if !connection.reachable {
            (
                "unreachable",
                "The RomM server could not be reached.".to_string(),
            )
        } else if !connection.authenticated {
            (
                "unauthorized",
                "The RomM server was reached but rejected the stored credentials.".to_string(),
            )
        } else {
            (
                "reachable",
                format!(
                    "Connected to RomM{}{}.",
                    connection
                        .version
                        .as_deref()
                        .map(|version| format!(" {version}"))
                        .unwrap_or_default(),
                    connection
                        .username
                        .as_deref()
                        .map(|username| format!(" as {username}"))
                        .unwrap_or_default()
                ),
            )
        };
        Self {
            result,
            version: connection.version,
            username: connection.username,
            plaintext_http,
            message,
        }
    }

    pub(super) fn unreachable(plaintext_http: bool, message: String) -> Self {
        Self {
            result: "unreachable",
            version: None,
            username: None,
            plaintext_http,
            message,
        }
    }

    pub(super) fn unauthorized(plaintext_http: bool) -> Self {
        Self {
            result: "unauthorized",
            version: None,
            username: None,
            plaintext_http,
            message: "The RomM server rejected the stored credentials.".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RommRemotePlatformResponse {
    pub(super) id: i64,
    pub(super) slug: String,
    pub(super) name: String,
    pub(super) rom_count: i64,
    /// Local platform matched by slug, for target prefill. `null` means "choose a platform".
    pub(super) target_platform_id: Option<i64>,
    pub(super) target_platform_name: Option<String>,
}

impl RommRemotePlatformResponse {
    pub(super) fn new(platform: RemotePlatform, local_platform: Option<(i64, String)>) -> Self {
        let (target_platform_id, target_platform_name) = match local_platform {
            Some((id, name)) => (Some(id), Some(name)),
            None => (None, None),
        };
        Self {
            id: platform.id,
            slug: platform.slug,
            name: platform.name,
            rom_count: platform.rom_count,
            target_platform_id,
            target_platform_name,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RommRemoteRomResponse {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) platform_slug: Option<String>,
    pub(super) platform_name: Option<String>,
    pub(super) fs_name: Option<String>,
    pub(super) file_size_bytes: Option<u64>,
    pub(super) file_count: usize,
    pub(super) has_cover: bool,
    pub(super) cover_url: Option<String>,
    /// Advisory only. Enforcement always happens server-side during the import job.
    pub(super) already_present: bool,
    pub(super) already_present_reason: Option<String>,
    pub(super) target_platform_id: Option<i64>,
    pub(super) target_platform_name: Option<String>,
}

impl RommRemoteRomResponse {
    pub(super) fn new(
        rom: RemoteRom,
        local_platform: Option<(i64, String)>,
        already_present: Option<String>,
    ) -> Self {
        let cover_url = rom
            .has_cover
            .then(|| format!("/api/admin/sources/romm/roms/{}/cover", rom.id));
        let (target_platform_id, target_platform_name) = match local_platform {
            Some((id, name)) => (Some(id), Some(name)),
            None => (None, None),
        };
        Self {
            id: rom.id,
            name: rom.name,
            platform_slug: rom.platform_slug,
            platform_name: rom.platform_name,
            fs_name: rom.fs_name,
            file_size_bytes: rom.file_size_bytes,
            file_count: rom.file_count,
            has_cover: rom.has_cover,
            cover_url,
            already_present: already_present.is_some(),
            already_present_reason: already_present,
            target_platform_id,
            target_platform_name,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RommRemoteRomListResponse {
    pub(super) items: Vec<RommRemoteRomResponse>,
    pub(super) total: i64,
    pub(super) limit: u32,
    pub(super) offset: u32,
}

#[derive(Debug, Serialize)]
pub struct RommRemoteFileResponse {
    pub(super) id: i64,
    pub(super) file_name: String,
    pub(super) file_size_bytes: Option<u64>,
    /// Which hash algorithms the remote server actually populated for this file.
    pub(super) hash_signals: Vec<&'static str>,
}

impl From<RemoteRomFile> for RommRemoteFileResponse {
    fn from(file: RemoteRomFile) -> Self {
        Self {
            hash_signals: file.hash_signals(),
            id: file.id,
            file_name: file.file_name,
            file_size_bytes: file.file_size_bytes,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct RommRemoteRomDetailResponse {
    pub(super) rom: RommRemoteRomResponse,
    pub(super) files: Vec<RommRemoteFileResponse>,
    pub(super) total_size_bytes: u64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRommImportRequest {
    pub(super) remote_rom_id: i64,
    #[serde(default)]
    pub(super) remote_file_ids: Vec<i64>,
    pub(super) platform_id: i64,
    #[serde(default)]
    pub(super) title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RommImportJobCreateResponse {
    job: RommImportJobSummaryResponse,
    status_url: String,
}

#[derive(Debug, Serialize)]
struct RommImportJobSummaryResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<RommImportProgressResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct RommImportJobStatusResponse {
    id: String,
    state: String,
    phase: String,
    progress: Option<RommImportProgressResponse>,
    result: Option<RommImportOutcome>,
    error: Option<RommImportJobErrorResponse>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct RommImportProgressResponse {
    kind: String,
    current: u64,
    total: u64,
    percent: f64,
}

#[derive(Debug, Serialize)]
struct RommImportJobErrorResponse {
    code: String,
    message: String,
}

impl RommImportJobCreateResponse {
    pub(super) fn new(snapshot: RommImportJobSnapshot, status_url: String) -> Self {
        Self {
            job: RommImportJobSummaryResponse {
                id: snapshot.id,
                state: snapshot.state.to_string(),
                phase: snapshot.phase.to_string(),
                progress: snapshot.progress.map(RommImportProgressResponse::from),
                created_at: snapshot.created_at,
                updated_at: snapshot.updated_at,
            },
            status_url,
        }
    }
}

impl From<RommImportJobSnapshot> for RommImportJobStatusResponse {
    fn from(snapshot: RommImportJobSnapshot) -> Self {
        Self {
            id: snapshot.id,
            state: snapshot.state.to_string(),
            phase: snapshot.phase.to_string(),
            progress: snapshot.progress.map(RommImportProgressResponse::from),
            result: snapshot.result,
            error: snapshot.error.map(|error| RommImportJobErrorResponse {
                code: error.code,
                message: error.message,
            }),
            created_at: snapshot.created_at,
            updated_at: snapshot.updated_at,
        }
    }
}

impl From<RommImportJobProgress> for RommImportProgressResponse {
    fn from(progress: RommImportJobProgress) -> Self {
        Self {
            kind: "bytes".to_string(),
            current: progress.current,
            total: progress.total,
            percent: progress.percent,
        }
    }
}
