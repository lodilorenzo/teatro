use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

use crate::{
    config::{AppConfig, LogFormat},
    domain::{
        user::PublicUser,
        workflow::{FileOperationKind, FileOperationState},
    },
    error::AppError,
    repositories::{file_operations, users},
    state::AppState,
};

#[derive(Debug, Clone, Serialize)]
pub struct ServerReport {
    pub service: ServiceReport,
    pub config: ConfigReport,
    pub database: DatabaseReport,
    pub users: UsersReport,
    pub library_roots: Vec<LibraryRootReport>,
    pub library: LibraryReport,
    pub platforms: Vec<PlatformReport>,
    pub file_operations: FileOperationsReport,
    pub recent_audit_events: Vec<AuditEventReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceReport {
    pub name: &'static str,
    pub version: &'static str,
    pub started_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConfigReport {
    pub bind_addr: String,
    pub database_url: String,
    pub data_dir: String,
    pub default_library_root: String,
    pub asset_root: String,
    pub lan_discovery_enabled: bool,
    pub discovery_name: String,
    pub max_upload_bytes: u64,
    pub max_upload_batch_files: usize,
    pub max_upload_batch_bytes: u64,
    pub max_concurrent_uploads: usize,
    pub upload_free_space_margin_bytes: u64,
    pub download_archive_max_files: usize,
    pub download_archive_max_bytes: u64,
    pub download_archive_concurrency: usize,
    pub download_archive_free_space_margin_bytes: u64,
    pub gog_import_enabled: bool,
    pub gog_import_configured: bool,
    pub gog_import_bundled_extractor: bool,
    pub gog_import_timeout_seconds: u64,
    pub gog_import_max_extracted_bytes: u64,
    pub gog_import_max_extracted_files: usize,
    pub auth_rate_limit_max_entries: usize,
    pub password_concurrency: usize,
    pub password_queue_depth: usize,
    pub password_queue_timeout_seconds: u64,
    pub trusted_proxy_ips: Vec<String>,
    pub log_format: &'static str,
    pub auth_rate_limit_max_failures: u32,
    pub auth_rate_limit_window_seconds: u64,
    pub auth_rate_limit_lockout_seconds: u64,
    pub igdb_configured: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatabaseReport {
    pub migration_count: i64,
    pub latest_migration_version: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UsersReport {
    pub total: usize,
    pub admins: usize,
    pub readonly: usize,
    pub users: Vec<PublicUser>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryRootReport {
    pub id: i64,
    pub name: String,
    pub root_path: String,
    pub writable: bool,
    pub exists_on_disk: bool,
    pub file_count: i64,
    pub total_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LibraryReport {
    pub total_roms: i64,
    pub total_files: i64,
    pub total_file_bytes: i64,
    pub metadata_rows: i64,
    pub cover_asset_rows: i64,
    pub cover_asset_bytes: i64,
    pub asset_root_exists_on_disk: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlatformReport {
    pub id: i64,
    pub slug: String,
    pub fs_slug: String,
    pub display_name: String,
    pub rom_count: i64,
    pub file_count: i64,
    pub total_file_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOperationsReport {
    pub counts_by_state: Vec<FileOperationStateCount>,
    pub pending: Vec<PendingFileOperationReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileOperationStateCount {
    pub state: FileOperationState,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingFileOperationReport {
    pub id: String,
    pub kind: FileOperationKind,
    pub state: FileOperationState,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditEventReport {
    pub id: i64,
    pub actor_user_id: Option<i64>,
    pub action: String,
    pub entity_type: Option<String>,
    pub entity_id: Option<i64>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

pub async fn build(state: &AppState) -> Result<ServerReport, AppError> {
    let config = state.config();
    let db = state.db();
    let users = users::list(db).await?;
    let public_users: Vec<_> = users.iter().map(|user| user.public()).collect();
    let admins = users.iter().filter(|user| user.role.is_admin()).count();
    let readonly = users.len().saturating_sub(admins);

    Ok(ServerReport {
        service: ServiceReport {
            name: "teatro",
            version: env!("CARGO_PKG_VERSION"),
            started_at: state.started_at().to_rfc3339(),
        },
        config: ConfigReport::from_config(config, state.igdb_config().await?.is_configured()),
        database: database_report(db).await?,
        users: UsersReport {
            total: users.len(),
            admins,
            readonly,
            users: public_users,
        },
        library_roots: library_root_reports(db).await?,
        library: library_report(db, config).await?,
        platforms: platform_reports(db).await?,
        file_operations: file_operation_report(db).await?,
        recent_audit_events: recent_audit_events(db).await?,
    })
}

impl ConfigReport {
    fn from_config(config: &AppConfig, igdb_configured: bool) -> Self {
        Self {
            bind_addr: config.bind_addr.to_string(),
            database_url: config.database_url.clone(),
            data_dir: config.data_dir.display().to_string(),
            default_library_root: config.default_library_root.display().to_string(),
            asset_root: config.asset_root.display().to_string(),
            lan_discovery_enabled: config.lan_discovery.enabled,
            discovery_name: config.lan_discovery.name.clone(),
            max_upload_bytes: config.max_upload_bytes,
            max_upload_batch_files: config.uploads.max_batch_files,
            max_upload_batch_bytes: config.uploads.max_batch_bytes,
            max_concurrent_uploads: config.uploads.max_concurrent_uploads,
            upload_free_space_margin_bytes: config.uploads.free_space_margin_bytes,
            download_archive_max_files: config.download_archives.max_files,
            download_archive_max_bytes: config.download_archives.max_source_bytes,
            download_archive_concurrency: config.download_archives.max_concurrent,
            download_archive_free_space_margin_bytes: config
                .download_archives
                .free_space_margin_bytes,
            gog_import_enabled: config.gog_import.enabled,
            gog_import_configured: config.gog_import.is_configured(),
            gog_import_bundled_extractor: config.gog_import.bundled_extractor,
            gog_import_timeout_seconds: config.gog_import.timeout_seconds,
            gog_import_max_extracted_bytes: config.gog_import.max_extracted_bytes,
            gog_import_max_extracted_files: config.gog_import.max_extracted_files,
            auth_rate_limit_max_entries: config.auth.rate_limit_max_entries,
            password_concurrency: config.auth.password_concurrency,
            password_queue_depth: config.auth.password_queue_depth,
            password_queue_timeout_seconds: config.auth.password_queue_timeout_seconds,
            trusted_proxy_ips: config
                .auth
                .trusted_proxy_ips
                .iter()
                .map(ToString::to_string)
                .collect(),
            log_format: log_format(config.log_format),
            auth_rate_limit_max_failures: config.auth.rate_limit_max_failures,
            auth_rate_limit_window_seconds: config.auth.rate_limit_window_seconds,
            auth_rate_limit_lockout_seconds: config.auth.rate_limit_lockout_seconds,
            igdb_configured,
        }
    }
}

async fn database_report(db: &SqlitePool) -> Result<DatabaseReport, sqlx::Error> {
    let row = sqlx::query_as::<_, MigrationSummaryRow>(
        r#"
        SELECT
            COUNT(*) AS migration_count,
            MAX(version) AS latest_migration_version
        FROM _sqlx_migrations
        WHERE success = 1
        "#,
    )
    .fetch_one(db)
    .await?;

    Ok(DatabaseReport {
        migration_count: row.migration_count,
        latest_migration_version: row.latest_migration_version,
    })
}

async fn library_root_reports(db: &SqlitePool) -> Result<Vec<LibraryRootReport>, sqlx::Error> {
    let rows = sqlx::query_as::<_, LibraryRootRow>(
        r#"
        SELECT
            lr.id,
            lr.name,
            lr.root_path,
            lr.writable,
            COUNT(rf.id) AS file_count,
            COALESCE(SUM(rf.file_size_bytes), 0) AS total_bytes
        FROM library_roots lr
        LEFT JOIN rom_files rf ON rf.root_id = lr.id
        GROUP BY lr.id, lr.name, lr.root_path, lr.writable
        ORDER BY lr.id
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| LibraryRootReport {
            exists_on_disk: std::path::Path::new(&row.root_path).exists(),
            id: row.id,
            name: row.name,
            root_path: row.root_path,
            writable: row.writable != 0,
            file_count: row.file_count,
            total_bytes: row.total_bytes,
        })
        .collect())
}

async fn library_report(db: &SqlitePool, config: &AppConfig) -> Result<LibraryReport, sqlx::Error> {
    let row = sqlx::query_as::<_, LibrarySummaryRow>(
        r#"
        SELECT
            (SELECT COUNT(*) FROM roms) AS total_roms,
            (SELECT COUNT(*) FROM rom_files) AS total_files,
            (SELECT COALESCE(SUM(file_size_bytes), 0) FROM rom_files) AS total_file_bytes,
            (SELECT COUNT(*) FROM rom_metadata) AS metadata_rows,
            (SELECT COUNT(*) FROM cover_assets) AS cover_asset_rows,
            (SELECT COALESCE(SUM(file_size_bytes), 0) FROM cover_assets) AS cover_asset_bytes
        "#,
    )
    .fetch_one(db)
    .await?;

    Ok(LibraryReport {
        total_roms: row.total_roms,
        total_files: row.total_files,
        total_file_bytes: row.total_file_bytes,
        metadata_rows: row.metadata_rows,
        cover_asset_rows: row.cover_asset_rows,
        cover_asset_bytes: row.cover_asset_bytes,
        asset_root_exists_on_disk: config.asset_root.exists(),
    })
}

async fn platform_reports(db: &SqlitePool) -> Result<Vec<PlatformReport>, sqlx::Error> {
    let rows = sqlx::query_as::<_, PlatformReportRow>(
        r#"
        SELECT
            p.id,
            p.slug,
            p.fs_slug,
            p.display_name,
            COUNT(DISTINCT r.id) AS rom_count,
            COUNT(rf.id) AS file_count,
            COALESCE(SUM(rf.file_size_bytes), 0) AS total_file_bytes
        FROM platforms p
        LEFT JOIN roms r ON r.platform_id = p.id
        LEFT JOIN rom_files rf ON rf.rom_id = r.id
        GROUP BY p.id, p.slug, p.fs_slug, p.display_name
        ORDER BY p.display_name COLLATE NOCASE, p.id
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| PlatformReport {
            id: row.id,
            slug: row.slug,
            fs_slug: row.fs_slug,
            display_name: row.display_name,
            rom_count: row.rom_count,
            file_count: row.file_count,
            total_file_bytes: row.total_file_bytes,
        })
        .collect())
}

async fn recent_audit_events(db: &SqlitePool) -> Result<Vec<AuditEventReport>, sqlx::Error> {
    let rows = sqlx::query_as::<_, AuditEventRow>(
        r#"
        SELECT id, actor_user_id, action, entity_type, entity_id, metadata_json, created_at
        FROM audit_log
        ORDER BY id DESC
        LIMIT 10
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| AuditEventReport {
            id: row.id,
            actor_user_id: row.actor_user_id,
            action: row.action,
            entity_type: row.entity_type,
            entity_id: row.entity_id,
            metadata_json: row.metadata_json,
            created_at: row.created_at,
        })
        .collect())
}

async fn file_operation_report(db: &SqlitePool) -> Result<FileOperationsReport, sqlx::Error> {
    let counts_by_state = file_operations::counts_by_state(db)
        .await?
        .into_iter()
        .map(|(state, count)| FileOperationStateCount { state, count })
        .collect();
    let pending = file_operations::pending(db)
        .await?
        .into_iter()
        .map(|operation| PendingFileOperationReport {
            id: operation.id,
            kind: operation.kind,
            state: operation.state,
            error_message: operation.error_message,
            created_at: operation.created_at,
            updated_at: operation.updated_at,
        })
        .collect();
    Ok(FileOperationsReport {
        counts_by_state,
        pending,
    })
}

fn log_format(log_format: LogFormat) -> &'static str {
    match log_format {
        LogFormat::Compact => "compact",
        LogFormat::Json => "json",
        LogFormat::Pretty => "pretty",
    }
}

#[derive(Debug, FromRow)]
struct MigrationSummaryRow {
    migration_count: i64,
    latest_migration_version: Option<i64>,
}

#[derive(Debug, FromRow)]
struct LibraryRootRow {
    id: i64,
    name: String,
    root_path: String,
    writable: i64,
    file_count: i64,
    total_bytes: i64,
}

#[derive(Debug, FromRow)]
struct LibrarySummaryRow {
    total_roms: i64,
    total_files: i64,
    total_file_bytes: i64,
    metadata_rows: i64,
    cover_asset_rows: i64,
    cover_asset_bytes: i64,
}

#[derive(Debug, FromRow)]
struct PlatformReportRow {
    id: i64,
    slug: String,
    fs_slug: String,
    display_name: String,
    rom_count: i64,
    file_count: i64,
    total_file_bytes: i64,
}

#[derive(Debug, FromRow)]
struct AuditEventRow {
    id: i64,
    actor_user_id: Option<i64>,
    action: String,
    entity_type: Option<String>,
    entity_id: Option<i64>,
    metadata_json: Option<String>,
    created_at: String,
}
