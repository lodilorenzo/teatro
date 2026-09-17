//! Initialized application resources and bounded process-wide services.

use std::{
    fs::{File, OpenOptions},
    io,
    path::Path,
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use fs2::{FileExt, lock_contended_error};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use sqlx::{
    SqlitePool,
    migrate::Migrator,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

use crate::{
    config::{AppConfig, IgdbConfig},
    error::AppError,
    repositories::{igdb_settings, integrity, library_roots},
    services::{
        auth_rate_limit::AuthRateLimiter,
        background_transfers::BackgroundTransferRegistry,
        file_operations, gog_import,
        igdb::IgdbClient,
        library::{DownloadTicketRegistry, LibraryScanJobRegistry, PreparedDownloadArchive},
        password::PasswordService,
        romm_source::{RommClient, RommImportJobRegistry},
    },
    storage::file_store::FileStore,
};

const INSTANCE_LOCK_FILE: &str = ".teatro.instance.lock";
static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitializationMode {
    Server,
    Cli,
}

#[derive(Clone)]
pub struct AppState {
    config: Arc<AppConfig>,
    db: SqlitePool,
    _instance_lock: Option<Arc<File>>,
    auth_rate_limiter: Arc<AuthRateLimiter>,
    igdb_client: Arc<IgdbClient>,
    file_store: Arc<FileStore>,
    password_service: Arc<PasswordService>,
    upload_semaphore: Arc<Semaphore>,
    background_transfers: Arc<BackgroundTransferRegistry>,
    download_archive_semaphore: Arc<Semaphore>,
    download_archive_tickets: Arc<DownloadTicketRegistry<PreparedDownloadArchive>>,
    download_file_tickets: Arc<DownloadTicketRegistry<axum::response::Response>>,
    gog_import_jobs: Arc<gog_import::GogImportJobRegistry>,
    library_scan_jobs: Arc<LibraryScanJobRegistry>,
    romm_client: Arc<RommClient>,
    romm_import_jobs: Arc<RommImportJobRegistry>,
    started_at: DateTime<Utc>,
}

impl AppState {
    pub async fn initialize(config: AppConfig) -> Result<Self, AppError> {
        Self::initialize_with_mode(config, InitializationMode::Server).await
    }

    pub(crate) async fn initialize_for_cli(config: AppConfig) -> Result<Self, AppError> {
        Self::initialize_with_mode(config, InitializationMode::Cli).await
    }

    async fn initialize_with_mode(
        config: AppConfig,
        mode: InitializationMode,
    ) -> Result<Self, AppError> {
        config.ensure_runtime_paths()?;

        let lock_path = config.data_dir.canonicalize()?.join(INSTANCE_LOCK_FILE);
        let instance_lock = try_lock_instance(&lock_path)?;
        if mode == InitializationMode::Server && instance_lock.is_none() {
            return Err(AppError::InstanceAlreadyRunning(lock_path));
        }
        let owns_instance = instance_lock.is_some();

        let connect_options = SqliteConnectOptions::from_str(&config.database_url)?
            .create_if_missing(owns_instance)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true);

        let db = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(connect_options)
            .await?;

        if owns_instance {
            MIGRATOR.run(&db).await?;
            let default_library_root = config.default_library_root.canonicalize()?;
            library_roots::ensure_default_root(&db, &default_library_root).await?;
        } else {
            verify_current_migrations(&db).await?;
        }

        if mode == InitializationMode::Server {
            integrity::fail_interrupted_jobs(&db).await?;
            gog_import::cleanup_abandoned_workspaces(&config)
                .await
                .map_err(|error| AppError::Reconciliation(error.to_string()))?;
        }

        let password_service = Arc::new(PasswordService::new(
            config.auth.password_concurrency,
            config.auth.password_queue_depth,
            Duration::from_secs(config.auth.password_queue_timeout_seconds),
        ));
        let upload_semaphore =
            Arc::new(Semaphore::new(config.uploads.max_concurrent_uploads.max(1)));
        let download_archive_semaphore = Arc::new(Semaphore::new(
            config.download_archives.max_concurrent.max(1),
        ));
        let download_archive_tickets = Arc::new(DownloadTicketRegistry::default());
        let romm_client = Arc::new(RommClient::new(&config.romm_source));
        let state = Self {
            config: Arc::new(config),
            db,
            _instance_lock: instance_lock.map(Arc::new),
            auth_rate_limiter: Arc::new(AuthRateLimiter::default()),
            igdb_client: Arc::new(IgdbClient::new()),
            file_store: Arc::new(FileStore::new()),
            password_service,
            upload_semaphore,
            background_transfers: Arc::new(BackgroundTransferRegistry::new()),
            download_archive_semaphore,
            download_archive_tickets,
            download_file_tickets: Arc::new(DownloadTicketRegistry::default()),
            gog_import_jobs: Arc::new(gog_import::GogImportJobRegistry::new()),
            library_scan_jobs: Arc::new(LibraryScanJobRegistry::new("scan_")),
            romm_client,
            romm_import_jobs: Arc::new(RommImportJobRegistry::new("romm_")),
            started_at: Utc::now(),
        };

        if mode == InitializationMode::Server {
            file_operations::reconcile(&state).await?;
            state
                .file_store()
                .cleanup_stale_uploads(
                    &state.config.default_library_root,
                    state.config.uploads.stale_part_age_seconds,
                )
                .await?;

            let download_archive_tickets = Arc::downgrade(&state.download_archive_tickets);
            let download_file_tickets = Arc::downgrade(&state.download_file_tickets);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let (Some(archives), Some(files)) = (
                        download_archive_tickets.upgrade(),
                        download_file_tickets.upgrade(),
                    ) else {
                        break;
                    };
                    archives.prune_expired();
                    files.prune_expired();
                }
            });
        }

        Ok(state)
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn db(&self) -> &SqlitePool {
        &self.db
    }

    pub(crate) fn auth_rate_limiter(&self) -> &AuthRateLimiter {
        &self.auth_rate_limiter
    }

    pub(crate) fn igdb_client(&self) -> &IgdbClient {
        &self.igdb_client
    }

    pub fn file_store(&self) -> &FileStore {
        &self.file_store
    }

    pub(crate) fn password_service(&self) -> &PasswordService {
        &self.password_service
    }

    pub(crate) async fn acquire_upload_permit(&self) -> OwnedSemaphorePermit {
        self.upload_semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("upload semaphore is never closed")
    }

    pub(crate) fn background_transfers(&self) -> &Arc<BackgroundTransferRegistry> {
        &self.background_transfers
    }

    pub(crate) fn try_acquire_download_archive_permit(&self) -> Option<OwnedSemaphorePermit> {
        self.download_archive_semaphore
            .clone()
            .try_acquire_owned()
            .ok()
    }

    pub(crate) fn download_archive_tickets(
        &self,
    ) -> &Arc<DownloadTicketRegistry<PreparedDownloadArchive>> {
        &self.download_archive_tickets
    }

    pub(crate) fn download_file_tickets(
        &self,
    ) -> &DownloadTicketRegistry<axum::response::Response> {
        &self.download_file_tickets
    }

    pub(crate) fn gog_import_jobs(&self) -> &Arc<gog_import::GogImportJobRegistry> {
        &self.gog_import_jobs
    }

    pub(crate) fn library_scan_jobs(&self) -> &Arc<LibraryScanJobRegistry> {
        &self.library_scan_jobs
    }

    pub(crate) fn romm_client(&self) -> &Arc<RommClient> {
        &self.romm_client
    }

    pub(crate) fn romm_import_jobs(&self) -> &Arc<RommImportJobRegistry> {
        &self.romm_import_jobs
    }

    pub(crate) async fn igdb_config(&self) -> Result<IgdbConfig, sqlx::Error> {
        igdb_settings::effective_config(&self.db, &self.config.igdb).await
    }

    pub(crate) fn started_at(&self) -> DateTime<Utc> {
        self.started_at
    }
}

fn try_lock_instance(lock_path: &Path) -> Result<Option<File>, AppError> {
    match std::fs::symlink_metadata(lock_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(AppError::UnsafeInstanceLock(lock_path.to_path_buf()));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == lock_contended_error().kind() => Ok(None),
        Err(error) => Err(error.into()),
    }
}

async fn verify_current_migrations(db: &SqlitePool) -> Result<(), AppError> {
    let applied: Vec<(i64, i64, Vec<u8>)> = match sqlx::query_as(
        "SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version",
    )
    .fetch_all(db)
    .await
    {
        Ok(applied) => applied,
        Err(_) => return Err(AppError::OfflineMigrationRequired),
    };
    let expected: Vec<_> = MIGRATOR
        .iter()
        .filter(|migration| !migration.migration_type.is_down_migration())
        .collect();

    let matches = applied.len() == expected.len()
        && applied
            .iter()
            .zip(expected)
            .all(|((version, success, checksum), migration)| {
                *version == migration.version
                    && *success == 1
                    && checksum.as_slice() == migration.checksum.as_ref()
            });
    if !matches {
        return Err(AppError::OfflineMigrationRequired);
    }
    Ok(())
}
