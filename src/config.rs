use std::{
    env,
    ffi::OsStr,
    fs,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
};

use crate::error::ConfigError;

const DEFAULT_BIND_ADDR: &str = "127.0.0.1:4440";
const DEFAULT_DATABASE_URL: &str = "sqlite://data/teatro.sqlite3";
const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_MAX_UPLOAD_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const DEFAULT_MAX_UPLOAD_BATCH_FILES: usize = 256;
const DEFAULT_MAX_UPLOAD_BATCH_BYTES: u64 = 128 * 1024 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_UPLOADS: usize = 2;
const DEFAULT_MAX_MULTIPART_TEXT_BYTES: usize = 4_096;
const DEFAULT_UPLOAD_FREE_SPACE_MARGIN_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_UPLOAD_DISK_CHECK_INTERVAL_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_STALE_UPLOAD_AGE_SECONDS: u64 = 24 * 60 * 60;
const DEFAULT_MAX_DOWNLOAD_ARCHIVE_FILES: usize = 256;
const DEFAULT_MAX_DOWNLOAD_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024 * 1024;
const DEFAULT_MAX_CONCURRENT_DOWNLOAD_ARCHIVES: usize = 1;
const DEFAULT_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_GOG_IMPORT_TIMEOUT_SECONDS: u64 = 30 * 60;
pub(crate) const SIPARIO_MAX_ARCHIVE_BYTES: u64 = 50 * 1024 * 1024 * 1024;
pub(crate) const SIPARIO_MAX_ARCHIVE_ENTRIES: usize = 20_000;
pub(crate) const SIPARIO_MAX_COMPRESSION_RATIO: u64 = 1_000;
const DEFAULT_GOG_IMPORT_MAX_EXTRACTED_BYTES: u64 = SIPARIO_MAX_ARCHIVE_BYTES;
const DEFAULT_GOG_IMPORT_MAX_EXTRACTED_FILES: usize = SIPARIO_MAX_ARCHIVE_ENTRIES;
const GOG_IMPORT_STAGING_DIRECTORY: &str = "gog-import-staging";
const DEFAULT_ROMM_SOURCE_TIMEOUT_SECONDS: u64 = 30;
const DEFAULT_ROMM_IMPORT_TIMEOUT_SECONDS: u64 = 60 * 60;
const DEFAULT_ROMM_IMPORT_MAX_FILES: usize = 64;
const BUNDLED_INNOEXTRACT_DIRECTORY: &str = "tools";
const BUNDLED_INNOEXTRACT_FILE: &str = "innoextract";
const BUNDLED_INNOEXTRACT_CHECKSUM_FILE: &str = "innoextract.sha256";
const BUNDLED_INNOEXTRACT_CONFIG_KEY: &str = "bundled innoextract sidecar";
const DEFAULT_LOG_FORMAT: LogFormat = LogFormat::Pretty;
const DEFAULT_DISCOVERY_NAME: &str = "Teatro";
const MAX_DISCOVERY_NAME_BYTES: usize = 48;
const DEFAULT_AUTH_RATE_LIMIT_MAX_FAILURES: u32 = 10;
const DEFAULT_AUTH_RATE_LIMIT_WINDOW_SECONDS: u64 = 300;
const DEFAULT_AUTH_RATE_LIMIT_LOCKOUT_SECONDS: u64 = 60;
const DEFAULT_AUTH_RATE_LIMIT_MAX_ENTRIES: usize = 10_000;
const DEFAULT_PASSWORD_CONCURRENCY: usize = 2;
const DEFAULT_PASSWORD_QUEUE_DEPTH: usize = 16;
const DEFAULT_PASSWORD_QUEUE_TIMEOUT_SECONDS: u64 = 5;
const DEFAULT_MAX_USERNAME_BYTES: usize = 128;
const DEFAULT_MAX_PASSWORD_BYTES: usize = 1_024;
const DEFAULT_TOKEN_TOUCH_INTERVAL_SECONDS: u64 = 300;
const DEFAULT_IGDB_TOKEN_URL: &str = "https://id.twitch.tv/oauth2/token";
const DEFAULT_IGDB_API_URL: &str = "https://api.igdb.com/v4";
const DEFAULT_IGDB_IMAGE_BASE_URL: &str = "https://images.igdb.com/igdb/image/upload";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub data_dir: PathBuf,
    pub default_library_root: PathBuf,
    pub asset_root: PathBuf,
    pub max_upload_bytes: u64,
    pub uploads: UploadConfig,
    pub download_archives: DownloadArchiveConfig,
    pub gog_import: GogImportConfig,
    pub romm_source: RommSourceConfig,
    pub log_format: LogFormat,
    pub lan_discovery: LanDiscoveryConfig,
    pub auth: AuthConfig,
    pub igdb: IgdbConfig,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        if let Some(key) = first_legacy_env_name(env::vars_os().map(|(key, _)| key)) {
            return Err(ConfigError::invalid_value(
                "legacy BEACON_* environment variable",
                key,
                "rename it to TEATRO_* before starting Teatro",
            ));
        }
        Self::from_env_vars(|key| env::var(key).ok())
    }

    pub fn ensure_runtime_paths(&self) -> Result<(), ConfigError> {
        if let Some(database_path) = sqlite_database_path(&self.database_url)
            && database_path.file_name() == Some(OsStr::new("teatro.sqlite3"))
        {
            let legacy_database = database_path.with_file_name("beacon.sqlite3");
            if path_exists("TEATRO_DATABASE_URL", &legacy_database)?
                && !path_exists("TEATRO_DATABASE_URL", &database_path)?
            {
                return Err(ConfigError::invalid_value(
                    "TEATRO_DATABASE_URL",
                    self.database_url.clone(),
                    "a sibling beacon.sqlite3 exists; complete the documented offline migration before starting Teatro",
                ));
            }
        }

        let legacy_lock = self.default_library_root.join(".beacon.lock");
        if path_exists("TEATRO_DEFAULT_LIBRARY_ROOT", &legacy_lock)? {
            return Err(ConfigError::invalid_value(
                "TEATRO_DEFAULT_LIBRARY_ROOT",
                self.default_library_root.display().to_string(),
                "a legacy .beacon.lock exists; stop Beacon and complete the documented offline migration",
            ));
        }

        create_dir(&self.data_dir)?;
        create_dir(&self.default_library_root)?;
        create_dir(&self.asset_root)?;
        if self.gog_import.enabled {
            self.gog_import.validate_runtime()?;
            create_dir(self.gog_import.staging_root(&self.data_dir))?;
        }

        if let Some(database_path) = sqlite_database_path(&self.database_url)
            && let Some(parent) = database_path.parent()
            && !parent.as_os_str().is_empty()
        {
            create_dir(parent)?;
        }

        Ok(())
    }

    pub(crate) fn from_env_vars<F>(get: F) -> Result<Self, ConfigError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let data_dir = prefixed_env(&get, "DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR));

        let bind_addr = parse_socket_addr(
            "TEATRO_BIND_ADDR",
            prefixed_env(&get, "BIND_ADDR").unwrap_or_else(|| DEFAULT_BIND_ADDR.to_string()),
        )?;

        let database_url =
            prefixed_env(&get, "DATABASE_URL").unwrap_or_else(|| DEFAULT_DATABASE_URL.to_string());

        let default_library_root = prefixed_env(&get, "DEFAULT_LIBRARY_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("roms"));

        let asset_root = prefixed_env(&get, "ASSET_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("assets"));

        let max_upload_bytes = parse_u64(
            "TEATRO_MAX_UPLOAD_BYTES",
            prefixed_env(&get, "MAX_UPLOAD_BYTES")
                .unwrap_or_else(|| DEFAULT_MAX_UPLOAD_BYTES.to_string()),
        )?;

        let uploads = UploadConfig {
            max_batch_files: parse_usize(
                "TEATRO_MAX_UPLOAD_BATCH_FILES",
                prefixed_env(&get, "MAX_UPLOAD_BATCH_FILES")
                    .unwrap_or_else(|| DEFAULT_MAX_UPLOAD_BATCH_FILES.to_string()),
            )?,
            max_batch_bytes: parse_u64(
                "TEATRO_MAX_UPLOAD_BATCH_BYTES",
                prefixed_env(&get, "MAX_UPLOAD_BATCH_BYTES")
                    .unwrap_or_else(|| DEFAULT_MAX_UPLOAD_BATCH_BYTES.to_string()),
            )?,
            max_concurrent_uploads: parse_usize(
                "TEATRO_MAX_CONCURRENT_UPLOADS",
                prefixed_env(&get, "MAX_CONCURRENT_UPLOADS")
                    .unwrap_or_else(|| DEFAULT_MAX_CONCURRENT_UPLOADS.to_string()),
            )?,
            max_text_field_bytes: parse_usize(
                "TEATRO_MAX_MULTIPART_TEXT_BYTES",
                prefixed_env(&get, "MAX_MULTIPART_TEXT_BYTES")
                    .unwrap_or_else(|| DEFAULT_MAX_MULTIPART_TEXT_BYTES.to_string()),
            )?,
            free_space_margin_bytes: parse_u64(
                "TEATRO_UPLOAD_FREE_SPACE_MARGIN_BYTES",
                prefixed_env(&get, "UPLOAD_FREE_SPACE_MARGIN_BYTES")
                    .unwrap_or_else(|| DEFAULT_UPLOAD_FREE_SPACE_MARGIN_BYTES.to_string()),
            )?,
            disk_check_interval_bytes: parse_u64(
                "TEATRO_UPLOAD_DISK_CHECK_INTERVAL_BYTES",
                prefixed_env(&get, "UPLOAD_DISK_CHECK_INTERVAL_BYTES")
                    .unwrap_or_else(|| DEFAULT_UPLOAD_DISK_CHECK_INTERVAL_BYTES.to_string()),
            )?,
            stale_part_age_seconds: parse_u64(
                "TEATRO_STALE_UPLOAD_AGE_SECONDS",
                prefixed_env(&get, "STALE_UPLOAD_AGE_SECONDS")
                    .unwrap_or_else(|| DEFAULT_STALE_UPLOAD_AGE_SECONDS.to_string()),
            )?,
        };

        let download_archives = DownloadArchiveConfig {
            max_files: parse_usize(
                "TEATRO_MAX_DOWNLOAD_ARCHIVE_FILES",
                prefixed_env(&get, "MAX_DOWNLOAD_ARCHIVE_FILES")
                    .unwrap_or_else(|| DEFAULT_MAX_DOWNLOAD_ARCHIVE_FILES.to_string()),
            )?,
            max_source_bytes: parse_u64(
                "TEATRO_MAX_DOWNLOAD_ARCHIVE_BYTES",
                prefixed_env(&get, "MAX_DOWNLOAD_ARCHIVE_BYTES")
                    .unwrap_or_else(|| DEFAULT_MAX_DOWNLOAD_ARCHIVE_BYTES.to_string()),
            )?,
            max_concurrent: parse_usize(
                "TEATRO_MAX_CONCURRENT_DOWNLOAD_ARCHIVES",
                prefixed_env(&get, "MAX_CONCURRENT_DOWNLOAD_ARCHIVES")
                    .unwrap_or_else(|| DEFAULT_MAX_CONCURRENT_DOWNLOAD_ARCHIVES.to_string()),
            )?,
            free_space_margin_bytes: parse_u64(
                "TEATRO_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES",
                prefixed_env(&get, "DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES").unwrap_or_else(
                    || DEFAULT_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES.to_string(),
                ),
            )?,
        };
        download_archives.validate_values()?;

        let gog_import_enabled = parse_bool(
            "TEATRO_GOG_IMPORT_ENABLED",
            prefixed_env(&get, "GOG_IMPORT_ENABLED").unwrap_or_else(|| "false".to_string()),
        )?;
        let mut innoextract_path = prefixed_env(&get, "INNOEXTRACT_PATH")
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from);
        let mut innoextract_sha256 = prefixed_env(&get, "INNOEXTRACT_SHA256")
            .filter(|value| !value.trim().is_empty())
            .map(|value| parse_sha256("TEATRO_INNOEXTRACT_SHA256", value))
            .transpose()?;
        let mut bundled_extractor = false;
        if gog_import_enabled
            && innoextract_path.is_none()
            && innoextract_sha256.is_none()
            && let Ok(executable) = env::current_exe()
            && let Some(bundled) = discover_bundled_innoextract(&executable)?
        {
            innoextract_path = Some(bundled.path);
            innoextract_sha256 = Some(bundled.sha256);
            bundled_extractor = true;
        }

        let gog_import = GogImportConfig {
            enabled: gog_import_enabled,
            innoextract_path,
            innoextract_sha256,
            bundled_extractor,
            helper_path: prefixed_env(&get, "GOG_IMPORT_HELPER_PATH")
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from),
            timeout_seconds: parse_u64(
                "TEATRO_GOG_IMPORT_TIMEOUT_SECONDS",
                prefixed_env(&get, "GOG_IMPORT_TIMEOUT_SECONDS")
                    .unwrap_or_else(|| DEFAULT_GOG_IMPORT_TIMEOUT_SECONDS.to_string()),
            )?,
            max_extracted_bytes: parse_u64(
                "TEATRO_GOG_IMPORT_MAX_EXTRACTED_BYTES",
                prefixed_env(&get, "GOG_IMPORT_MAX_EXTRACTED_BYTES")
                    .unwrap_or_else(|| DEFAULT_GOG_IMPORT_MAX_EXTRACTED_BYTES.to_string()),
            )?,
            max_extracted_files: parse_usize(
                "TEATRO_GOG_IMPORT_MAX_EXTRACTED_FILES",
                prefixed_env(&get, "GOG_IMPORT_MAX_EXTRACTED_FILES")
                    .unwrap_or_else(|| DEFAULT_GOG_IMPORT_MAX_EXTRACTED_FILES.to_string()),
            )?,
        };
        gog_import.validate_values()?;

        let romm_source = RommSourceConfig {
            enabled: parse_bool(
                "TEATRO_ROMM_SOURCE_ENABLED",
                prefixed_env(&get, "ROMM_SOURCE_ENABLED").unwrap_or_else(|| "false".to_string()),
            )?,
            timeout_seconds: parse_u64(
                "TEATRO_ROMM_SOURCE_TIMEOUT_SECONDS",
                prefixed_env(&get, "ROMM_SOURCE_TIMEOUT_SECONDS")
                    .unwrap_or_else(|| DEFAULT_ROMM_SOURCE_TIMEOUT_SECONDS.to_string()),
            )?,
            import_timeout_seconds: parse_u64(
                "TEATRO_ROMM_IMPORT_TIMEOUT_SECONDS",
                prefixed_env(&get, "ROMM_IMPORT_TIMEOUT_SECONDS")
                    .unwrap_or_else(|| DEFAULT_ROMM_IMPORT_TIMEOUT_SECONDS.to_string()),
            )?,
            // An unset or blank ceiling reuses the existing per-file upload ceiling.
            max_import_bytes: parse_u64(
                "TEATRO_ROMM_IMPORT_MAX_BYTES",
                prefixed_env(&get, "ROMM_IMPORT_MAX_BYTES")
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or_else(|| max_upload_bytes.to_string()),
            )?,
            max_import_files: parse_usize(
                "TEATRO_ROMM_IMPORT_MAX_FILES",
                prefixed_env(&get, "ROMM_IMPORT_MAX_FILES")
                    .unwrap_or_else(|| DEFAULT_ROMM_IMPORT_MAX_FILES.to_string()),
            )?,
        };
        romm_source.validate_values()?;

        let log_format = prefixed_env(&get, "LOG_FORMAT")
            .map(|value| parse_log_format("TEATRO_LOG_FORMAT", value))
            .transpose()?
            .unwrap_or(DEFAULT_LOG_FORMAT);

        let lan_discovery = LanDiscoveryConfig {
            enabled: parse_bool(
                "TEATRO_LAN_DISCOVERY_ENABLED",
                prefixed_env(&get, "LAN_DISCOVERY_ENABLED").unwrap_or_else(|| "false".to_string()),
            )?,
            name: parse_discovery_name(
                prefixed_env(&get, "DISCOVERY_NAME")
                    .unwrap_or_else(|| DEFAULT_DISCOVERY_NAME.to_string()),
            )?,
        };

        let auth = AuthConfig {
            rate_limit_max_failures: parse_u32(
                "TEATRO_AUTH_RATE_LIMIT_MAX_FAILURES",
                prefixed_env(&get, "AUTH_RATE_LIMIT_MAX_FAILURES")
                    .unwrap_or_else(|| DEFAULT_AUTH_RATE_LIMIT_MAX_FAILURES.to_string()),
            )?,
            rate_limit_window_seconds: parse_u64(
                "TEATRO_AUTH_RATE_LIMIT_WINDOW_SECONDS",
                prefixed_env(&get, "AUTH_RATE_LIMIT_WINDOW_SECONDS")
                    .unwrap_or_else(|| DEFAULT_AUTH_RATE_LIMIT_WINDOW_SECONDS.to_string()),
            )?,
            rate_limit_lockout_seconds: parse_u64(
                "TEATRO_AUTH_RATE_LIMIT_LOCKOUT_SECONDS",
                prefixed_env(&get, "AUTH_RATE_LIMIT_LOCKOUT_SECONDS")
                    .unwrap_or_else(|| DEFAULT_AUTH_RATE_LIMIT_LOCKOUT_SECONDS.to_string()),
            )?,
            rate_limit_max_entries: parse_usize(
                "TEATRO_AUTH_RATE_LIMIT_MAX_ENTRIES",
                prefixed_env(&get, "AUTH_RATE_LIMIT_MAX_ENTRIES")
                    .unwrap_or_else(|| DEFAULT_AUTH_RATE_LIMIT_MAX_ENTRIES.to_string()),
            )?,
            password_concurrency: parse_usize(
                "TEATRO_PASSWORD_CONCURRENCY",
                prefixed_env(&get, "PASSWORD_CONCURRENCY")
                    .unwrap_or_else(|| DEFAULT_PASSWORD_CONCURRENCY.to_string()),
            )?,
            password_queue_depth: parse_usize(
                "TEATRO_PASSWORD_QUEUE_DEPTH",
                prefixed_env(&get, "PASSWORD_QUEUE_DEPTH")
                    .unwrap_or_else(|| DEFAULT_PASSWORD_QUEUE_DEPTH.to_string()),
            )?,
            password_queue_timeout_seconds: parse_u64(
                "TEATRO_PASSWORD_QUEUE_TIMEOUT_SECONDS",
                prefixed_env(&get, "PASSWORD_QUEUE_TIMEOUT_SECONDS")
                    .unwrap_or_else(|| DEFAULT_PASSWORD_QUEUE_TIMEOUT_SECONDS.to_string()),
            )?,
            max_username_bytes: parse_usize(
                "TEATRO_MAX_USERNAME_BYTES",
                prefixed_env(&get, "MAX_USERNAME_BYTES")
                    .unwrap_or_else(|| DEFAULT_MAX_USERNAME_BYTES.to_string()),
            )?,
            max_password_bytes: parse_usize(
                "TEATRO_MAX_PASSWORD_BYTES",
                prefixed_env(&get, "MAX_PASSWORD_BYTES")
                    .unwrap_or_else(|| DEFAULT_MAX_PASSWORD_BYTES.to_string()),
            )?,
            token_touch_interval_seconds: parse_u64(
                "TEATRO_TOKEN_TOUCH_INTERVAL_SECONDS",
                prefixed_env(&get, "TOKEN_TOUCH_INTERVAL_SECONDS")
                    .unwrap_or_else(|| DEFAULT_TOKEN_TOUCH_INTERVAL_SECONDS.to_string()),
            )?,
            trusted_proxy_ips: parse_ip_list(
                "TEATRO_TRUSTED_PROXY_IPS",
                prefixed_env(&get, "TRUSTED_PROXY_IPS").unwrap_or_default(),
            )?,
        };

        let igdb = IgdbConfig {
            client_id: prefixed_env(&get, "IGDB_CLIENT_ID").filter(|value| !value.is_empty()),
            client_secret: prefixed_env(&get, "IGDB_CLIENT_SECRET")
                .filter(|value| !value.is_empty()),
            ..IgdbConfig::default()
        };

        Ok(Self {
            bind_addr,
            database_url,
            data_dir,
            default_library_root,
            asset_root,
            max_upload_bytes,
            uploads,
            download_archives,
            gog_import,
            romm_source,
            log_format,
            lan_discovery,
            auth,
            igdb,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadConfig {
    pub max_batch_files: usize,
    pub max_batch_bytes: u64,
    pub max_concurrent_uploads: usize,
    pub max_text_field_bytes: usize,
    pub free_space_margin_bytes: u64,
    pub disk_check_interval_bytes: u64,
    pub stale_part_age_seconds: u64,
}

impl Default for UploadConfig {
    fn default() -> Self {
        Self {
            max_batch_files: DEFAULT_MAX_UPLOAD_BATCH_FILES,
            max_batch_bytes: DEFAULT_MAX_UPLOAD_BATCH_BYTES,
            max_concurrent_uploads: DEFAULT_MAX_CONCURRENT_UPLOADS,
            max_text_field_bytes: DEFAULT_MAX_MULTIPART_TEXT_BYTES,
            free_space_margin_bytes: DEFAULT_UPLOAD_FREE_SPACE_MARGIN_BYTES,
            disk_check_interval_bytes: DEFAULT_UPLOAD_DISK_CHECK_INTERVAL_BYTES,
            stale_part_age_seconds: DEFAULT_STALE_UPLOAD_AGE_SECONDS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadArchiveConfig {
    pub max_files: usize,
    pub max_source_bytes: u64,
    pub max_concurrent: usize,
    pub free_space_margin_bytes: u64,
}

impl DownloadArchiveConfig {
    fn validate_values(&self) -> Result<(), ConfigError> {
        for (key, value) in [
            ("TEATRO_MAX_DOWNLOAD_ARCHIVE_FILES", self.max_files),
            (
                "TEATRO_MAX_CONCURRENT_DOWNLOAD_ARCHIVES",
                self.max_concurrent,
            ),
        ] {
            if value == 0 {
                return Err(ConfigError::invalid_value(
                    key,
                    "0",
                    "must be greater than zero",
                ));
            }
        }
        if self.max_source_bytes == 0 {
            return Err(ConfigError::invalid_value(
                "TEATRO_MAX_DOWNLOAD_ARCHIVE_BYTES",
                "0",
                "must be greater than zero",
            ));
        }
        Ok(())
    }
}

impl Default for DownloadArchiveConfig {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_DOWNLOAD_ARCHIVE_FILES,
            max_source_bytes: DEFAULT_MAX_DOWNLOAD_ARCHIVE_BYTES,
            max_concurrent: DEFAULT_MAX_CONCURRENT_DOWNLOAD_ARCHIVES,
            free_space_margin_bytes: DEFAULT_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GogImportConfig {
    pub enabled: bool,
    pub innoextract_path: Option<PathBuf>,
    pub innoextract_sha256: Option<String>,
    pub bundled_extractor: bool,
    pub helper_path: Option<PathBuf>,
    pub timeout_seconds: u64,
    pub max_extracted_bytes: u64,
    pub max_extracted_files: usize,
}

impl GogImportConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && self.innoextract_path.is_some() && self.innoextract_sha256.is_some()
    }

    pub fn staging_root(&self, data_dir: &std::path::Path) -> PathBuf {
        data_dir.join(GOG_IMPORT_STAGING_DIRECTORY)
    }

    fn validate_values(&self) -> Result<(), ConfigError> {
        if self.timeout_seconds == 0 {
            return Err(ConfigError::invalid_value(
                "TEATRO_GOG_IMPORT_TIMEOUT_SECONDS",
                "0",
                "must be greater than zero",
            ));
        }
        if self.max_extracted_bytes == 0 || self.max_extracted_bytes > SIPARIO_MAX_ARCHIVE_BYTES {
            return Err(ConfigError::invalid_value(
                "TEATRO_GOG_IMPORT_MAX_EXTRACTED_BYTES",
                self.max_extracted_bytes.to_string(),
                "must be greater than zero and no larger than the 50 GiB archive limit",
            ));
        }
        if self.max_extracted_files == 0 || self.max_extracted_files > SIPARIO_MAX_ARCHIVE_ENTRIES {
            return Err(ConfigError::invalid_value(
                "TEATRO_GOG_IMPORT_MAX_EXTRACTED_FILES",
                self.max_extracted_files.to_string(),
                "must be greater than zero and no larger than the 20,000-entry archive limit",
            ));
        }
        if !self.enabled {
            return Ok(());
        }

        let extractor = self.innoextract_path.as_ref().ok_or_else(|| {
            ConfigError::invalid_value(
                "TEATRO_INNOEXTRACT_PATH",
                "",
                "is required when GOG import is enabled",
            )
        })?;
        if !extractor.is_absolute() {
            return Err(ConfigError::invalid_value(
                "TEATRO_INNOEXTRACT_PATH",
                extractor.display().to_string(),
                "must be an absolute path",
            ));
        }
        let extractor_sha256 = self.innoextract_sha256.as_deref().ok_or_else(|| {
            ConfigError::invalid_value(
                "TEATRO_INNOEXTRACT_SHA256",
                "",
                "is required when GOG import is enabled",
            )
        })?;
        if extractor_sha256.len() != 64
            || !extractor_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(ConfigError::invalid_value(
                "TEATRO_INNOEXTRACT_SHA256",
                extractor_sha256,
                "expected a 64-character hexadecimal SHA-256 digest",
            ));
        }
        if let Some(helper_path) = &self.helper_path
            && !helper_path.is_absolute()
        {
            return Err(ConfigError::invalid_value(
                "TEATRO_GOG_IMPORT_HELPER_PATH",
                helper_path.display().to_string(),
                "must be an absolute directory path",
            ));
        }
        Ok(())
    }

    fn validate_runtime(&self) -> Result<(), ConfigError> {
        self.validate_values()?;
        let extractor = self
            .innoextract_path
            .as_ref()
            .expect("enabled GOG import configuration has an extractor path");
        validate_regular_non_symlink("TEATRO_INNOEXTRACT_PATH", extractor)?;
        if let Some(helper_path) = &self.helper_path {
            validate_directory_non_symlink("TEATRO_GOG_IMPORT_HELPER_PATH", helper_path)?;
        }
        Ok(())
    }
}

impl Default for GogImportConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            innoextract_path: None,
            innoextract_sha256: None,
            bundled_extractor: false,
            helper_path: None,
            timeout_seconds: DEFAULT_GOG_IMPORT_TIMEOUT_SECONDS,
            max_extracted_bytes: DEFAULT_GOG_IMPORT_MAX_EXTRACTED_BYTES,
            max_extracted_files: DEFAULT_GOG_IMPORT_MAX_EXTRACTED_FILES,
        }
    }
}

/// Process-wide bounds for the outbound RomM source client and importer.
///
/// The base URL, username, and secret are deliberately *not* environment variables: they are
/// runtime settings so an operator can change sources without a restart and so the secret never
/// lives in a process environment or shell history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RommSourceConfig {
    pub enabled: bool,
    pub timeout_seconds: u64,
    pub import_timeout_seconds: u64,
    pub max_import_bytes: u64,
    pub max_import_files: usize,
}

impl RommSourceConfig {
    fn validate_values(&self) -> Result<(), ConfigError> {
        if self.timeout_seconds == 0 {
            return Err(ConfigError::invalid_value(
                "TEATRO_ROMM_SOURCE_TIMEOUT_SECONDS",
                "0",
                "must be greater than zero",
            ));
        }
        if self.import_timeout_seconds == 0 {
            return Err(ConfigError::invalid_value(
                "TEATRO_ROMM_IMPORT_TIMEOUT_SECONDS",
                "0",
                "must be greater than zero",
            ));
        }
        if self.max_import_bytes == 0 {
            return Err(ConfigError::invalid_value(
                "TEATRO_ROMM_IMPORT_MAX_BYTES",
                "0",
                "must be greater than zero",
            ));
        }
        if self.max_import_files == 0 || self.max_import_files > SIPARIO_MAX_ARCHIVE_ENTRIES {
            return Err(ConfigError::invalid_value(
                "TEATRO_ROMM_IMPORT_MAX_FILES",
                self.max_import_files.to_string(),
                "must be greater than zero and no larger than 20,000",
            ));
        }
        Ok(())
    }
}

impl Default for RommSourceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            timeout_seconds: DEFAULT_ROMM_SOURCE_TIMEOUT_SECONDS,
            import_timeout_seconds: DEFAULT_ROMM_IMPORT_TIMEOUT_SECONDS,
            max_import_bytes: DEFAULT_MAX_UPLOAD_BYTES,
            max_import_files: DEFAULT_ROMM_IMPORT_MAX_FILES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanDiscoveryConfig {
    pub enabled: bool,
    pub name: String,
}

impl Default for LanDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            name: DEFAULT_DISCOVERY_NAME.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthConfig {
    pub rate_limit_max_failures: u32,
    pub rate_limit_window_seconds: u64,
    pub rate_limit_lockout_seconds: u64,
    pub rate_limit_max_entries: usize,
    pub password_concurrency: usize,
    pub password_queue_depth: usize,
    pub password_queue_timeout_seconds: u64,
    pub max_username_bytes: usize,
    pub max_password_bytes: usize,
    pub token_touch_interval_seconds: u64,
    pub trusted_proxy_ips: Vec<IpAddr>,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            rate_limit_max_failures: DEFAULT_AUTH_RATE_LIMIT_MAX_FAILURES,
            rate_limit_window_seconds: DEFAULT_AUTH_RATE_LIMIT_WINDOW_SECONDS,
            rate_limit_lockout_seconds: DEFAULT_AUTH_RATE_LIMIT_LOCKOUT_SECONDS,
            rate_limit_max_entries: DEFAULT_AUTH_RATE_LIMIT_MAX_ENTRIES,
            password_concurrency: DEFAULT_PASSWORD_CONCURRENCY,
            password_queue_depth: DEFAULT_PASSWORD_QUEUE_DEPTH,
            password_queue_timeout_seconds: DEFAULT_PASSWORD_QUEUE_TIMEOUT_SECONDS,
            max_username_bytes: DEFAULT_MAX_USERNAME_BYTES,
            max_password_bytes: DEFAULT_MAX_PASSWORD_BYTES,
            token_touch_interval_seconds: DEFAULT_TOKEN_TOUCH_INTERVAL_SECONDS,
            trusted_proxy_ips: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Compact,
    Json,
    Pretty,
}

impl FromStr for LogFormat {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "compact" => Ok(Self::Compact),
            "json" => Ok(Self::Json),
            "pretty" => Ok(Self::Pretty),
            _ => Err("expected one of: compact, json, pretty"),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct IgdbConfig {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub token_url: String,
    pub api_url: String,
    pub image_base_url: String,
}

impl IgdbConfig {
    pub fn is_configured(&self) -> bool {
        self.client_id.is_some() && self.client_secret.is_some()
    }
}

impl Default for IgdbConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            token_url: DEFAULT_IGDB_TOKEN_URL.to_string(),
            api_url: DEFAULT_IGDB_API_URL.to_string(),
            image_base_url: DEFAULT_IGDB_IMAGE_BASE_URL.to_string(),
        }
    }
}

impl std::fmt::Debug for IgdbConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IgdbConfig")
            .field("client_id_configured", &self.client_id.is_some())
            .field("client_secret_configured", &self.client_secret.is_some())
            .field("token_url", &self.token_url)
            .field("api_url", &self.api_url)
            .field("image_base_url", &self.image_base_url)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct BundledInnoextract {
    path: PathBuf,
    sha256: String,
}

fn discover_bundled_innoextract(
    teatro_executable: &std::path::Path,
) -> Result<Option<BundledInnoextract>, ConfigError> {
    let executable_directory = teatro_executable.parent().ok_or_else(|| {
        ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            teatro_executable.display().to_string(),
            "Teatro executable has no parent directory",
        )
    })?;
    let tools_directory = executable_directory.join(BUNDLED_INNOEXTRACT_DIRECTORY);
    let extractor = tools_directory.join(BUNDLED_INNOEXTRACT_FILE);
    let checksum = tools_directory.join(BUNDLED_INNOEXTRACT_CHECKSUM_FILE);
    let extractor_exists = match fs::symlink_metadata(&extractor) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(ConfigError::invalid_value(
                BUNDLED_INNOEXTRACT_CONFIG_KEY,
                extractor.display().to_string(),
                format!("cannot inspect packaged extractor: {error}"),
            ));
        }
    };
    let checksum_metadata = match fs::symlink_metadata(&checksum) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(ConfigError::invalid_value(
                BUNDLED_INNOEXTRACT_CONFIG_KEY,
                checksum.display().to_string(),
                format!("cannot inspect packaged checksum: {error}"),
            ));
        }
    };
    if !extractor_exists && checksum_metadata.is_none() {
        return Ok(None);
    }
    if !extractor_exists || checksum_metadata.is_none() {
        return Err(ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            tools_directory.display().to_string(),
            "packaged extractor and checksum must both be present",
        ));
    }
    validate_regular_non_symlink(BUNDLED_INNOEXTRACT_CONFIG_KEY, &extractor)?;
    let checksum_metadata = checksum_metadata.expect("checked bundled checksum presence");
    if checksum_metadata.file_type().is_symlink()
        || !checksum_metadata.is_file()
        || checksum_metadata.len() > 256
    {
        return Err(ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            checksum.display().to_string(),
            "checksum must be a small regular non-symlink file",
        ));
    }
    let checksum_contents = fs::read_to_string(&checksum).map_err(|error| {
        ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            checksum.display().to_string(),
            format!("cannot read packaged checksum: {error}"),
        )
    })?;
    let mut lines = checksum_contents
        .lines()
        .filter(|line| !line.trim().is_empty());
    let line = lines.next().ok_or_else(|| {
        ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            checksum.display().to_string(),
            "checksum manifest is empty",
        )
    })?;
    if lines.next().is_some() {
        return Err(ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            checksum.display().to_string(),
            "checksum manifest must contain exactly one entry",
        ));
    }
    let mut fields = line.split_ascii_whitespace();
    let digest = fields.next().unwrap_or_default();
    let file_name = fields.next().unwrap_or_default();
    if file_name != BUNDLED_INNOEXTRACT_FILE || fields.next().is_some() {
        return Err(ConfigError::invalid_value(
            BUNDLED_INNOEXTRACT_CONFIG_KEY,
            line,
            "expected '<sha256>  innoextract'",
        ));
    }
    let sha256 = parse_sha256(BUNDLED_INNOEXTRACT_CONFIG_KEY, digest.to_string())?;
    Ok(Some(BundledInnoextract {
        path: extractor,
        sha256,
    }))
}

fn prefixed_env<F>(get: &F, name: &'static str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    get(&format!("TEATRO_{name}"))
}

fn first_legacy_env_name<I, S>(names: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    names.into_iter().find_map(|name| {
        let name = name.as_ref().to_string_lossy();
        name.starts_with("BEACON_").then(|| name.into_owned())
    })
}

fn path_exists(key: &'static str, path: &Path) -> Result<bool, ConfigError> {
    path.try_exists().map_err(|error| {
        ConfigError::invalid_value(
            key,
            path.display().to_string(),
            format!("cannot inspect legacy path: {error}"),
        )
    })
}

fn parse_socket_addr(key: &'static str, value: String) -> Result<SocketAddr, ConfigError> {
    value
        .parse()
        .map_err(|error| ConfigError::invalid_value(key, value, format!("{error}")))
}

fn parse_bool(key: &'static str, value: String) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::invalid_value(
            key,
            value,
            "expected true/false, 1/0, yes/no, or on/off",
        )),
    }
}

fn parse_discovery_name(value: String) -> Result<String, ConfigError> {
    if value.chars().any(char::is_control) {
        return Err(ConfigError::invalid_value(
            "TEATRO_DISCOVERY_NAME",
            value,
            "control characters are not allowed",
        ));
    }
    let name = value.trim();
    if name.is_empty() || name.len() > MAX_DISCOVERY_NAME_BYTES {
        return Err(ConfigError::invalid_value(
            "TEATRO_DISCOVERY_NAME",
            value,
            "must contain 1 to 48 UTF-8 bytes after trimming",
        ));
    }
    Ok(name.to_string())
}

fn parse_sha256(key: &'static str, value: String) -> Result<String, ConfigError> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() == 64 && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(normalized)
    } else {
        Err(ConfigError::invalid_value(
            key,
            value,
            "expected a 64-character hexadecimal SHA-256 digest",
        ))
    }
}

fn validate_regular_non_symlink(
    key: &'static str,
    path: &std::path::Path,
) -> Result<(), ConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ConfigError::invalid_value(key, path.display().to_string(), format!("{error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ConfigError::invalid_value(
            key,
            path.display().to_string(),
            "must identify a regular non-symlink file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ConfigError::invalid_value(
                key,
                path.display().to_string(),
                "must identify an executable file",
            ));
        }
    }
    Ok(())
}

fn validate_directory_non_symlink(
    key: &'static str,
    path: &std::path::Path,
) -> Result<(), ConfigError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ConfigError::invalid_value(key, path.display().to_string(), format!("{error}"))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ConfigError::invalid_value(
            key,
            path.display().to_string(),
            "must identify a non-symlink directory",
        ));
    }
    Ok(())
}

fn parse_u64(key: &'static str, value: String) -> Result<u64, ConfigError> {
    value
        .parse()
        .map_err(|error| ConfigError::invalid_value(key, value, format!("{error}")))
}

fn parse_usize(key: &'static str, value: String) -> Result<usize, ConfigError> {
    value
        .parse()
        .map_err(|error| ConfigError::invalid_value(key, value, format!("{error}")))
}

fn parse_ip_list(key: &'static str, value: String) -> Result<Vec<IpAddr>, ConfigError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|ip| {
            ip.parse()
                .map_err(|error| ConfigError::invalid_value(key, ip, format!("{error}")))
        })
        .collect()
}

fn parse_u32(key: &'static str, value: String) -> Result<u32, ConfigError> {
    value
        .parse()
        .map_err(|error| ConfigError::invalid_value(key, value, format!("{error}")))
}

fn parse_log_format(key: &'static str, value: String) -> Result<LogFormat, ConfigError> {
    value
        .parse()
        .map_err(|error| ConfigError::invalid_value(key, value, error))
}

fn create_dir(path: impl Into<PathBuf>) -> Result<(), ConfigError> {
    let path = path.into();
    fs::create_dir_all(&path).map_err(|source| ConfigError::create_dir(path, source))
}

fn sqlite_database_path(database_url: &str) -> Option<PathBuf> {
    let without_scheme = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))?;

    let path = without_scheme.split('?').next().unwrap_or_default();
    if path.is_empty() || path == ":memory:" {
        return None;
    }

    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use std::{fs, net::SocketAddr, path::PathBuf};

    use super::{
        AppConfig, AuthConfig, DownloadArchiveConfig, GogImportConfig, LanDiscoveryConfig,
        LogFormat, discover_bundled_innoextract, first_legacy_env_name, sqlite_database_path,
    };

    #[test]
    fn defaults_use_teatro_names_and_localhost() {
        let config = AppConfig::from_env_vars(|_| None).expect("default config should load");

        assert_eq!(config.bind_addr, SocketAddr::from(([127, 0, 0, 1], 4440)));
        assert_eq!(config.database_url, "sqlite://data/teatro.sqlite3");
        assert_eq!(config.data_dir, PathBuf::from("./data"));
        assert_eq!(
            config.default_library_root,
            PathBuf::from("./data").join("roms")
        );
        assert_eq!(config.asset_root, PathBuf::from("./data").join("assets"));
        assert_eq!(config.uploads.max_batch_bytes, 128 * 1024 * 1024 * 1024);
        assert_eq!(config.log_format, LogFormat::Pretty);
        assert_eq!(config.lan_discovery, LanDiscoveryConfig::default());
        assert_eq!(config.auth, AuthConfig::default());
        assert_eq!(config.download_archives, DownloadArchiveConfig::default());
        assert_eq!(config.gog_import, GogImportConfig::default());
        assert_eq!(config.igdb, super::IgdbConfig::default());
    }

    #[test]
    fn teatro_env_vars_override_defaults() {
        let config = AppConfig::from_env_vars(|key| match key {
            "TEATRO_BIND_ADDR" => Some("127.0.0.1:4444".to_string()),
            "TEATRO_DATABASE_URL" => Some("sqlite://custom/teatro.sqlite3".to_string()),
            "TEATRO_DATA_DIR" => Some("/srv/teatro".to_string()),
            "TEATRO_MAX_UPLOAD_BYTES" => Some("123".to_string()),
            "TEATRO_MAX_DOWNLOAD_ARCHIVE_FILES" => Some("12".to_string()),
            "TEATRO_MAX_DOWNLOAD_ARCHIVE_BYTES" => Some("345".to_string()),
            "TEATRO_MAX_CONCURRENT_DOWNLOAD_ARCHIVES" => Some("2".to_string()),
            "TEATRO_DOWNLOAD_ARCHIVE_FREE_SPACE_MARGIN_BYTES" => Some("67".to_string()),
            "TEATRO_LOG_FORMAT" => Some("json".to_string()),
            "TEATRO_LAN_DISCOVERY_ENABLED" => Some("true".to_string()),
            "TEATRO_DISCOVERY_NAME" => Some(" Living Room ".to_string()),
            "TEATRO_GOG_IMPORT_ENABLED" => Some("true".to_string()),
            "TEATRO_INNOEXTRACT_PATH" => Some("/opt/teatro/innoextract".to_string()),
            "TEATRO_INNOEXTRACT_SHA256" => Some("AA".repeat(32)),
            "TEATRO_GOG_IMPORT_HELPER_PATH" => Some("/opt/teatro/helpers".to_string()),
            "TEATRO_GOG_IMPORT_TIMEOUT_SECONDS" => Some("45".to_string()),
            "TEATRO_GOG_IMPORT_MAX_EXTRACTED_BYTES" => Some("456".to_string()),
            "TEATRO_GOG_IMPORT_MAX_EXTRACTED_FILES" => Some("789".to_string()),
            "TEATRO_AUTH_RATE_LIMIT_MAX_FAILURES" => Some("3".to_string()),
            "TEATRO_AUTH_RATE_LIMIT_WINDOW_SECONDS" => Some("30".to_string()),
            "TEATRO_AUTH_RATE_LIMIT_LOCKOUT_SECONDS" => Some("10".to_string()),
            "TEATRO_PASSWORD_QUEUE_DEPTH" => Some("7".to_string()),
            "TEATRO_PASSWORD_QUEUE_TIMEOUT_SECONDS" => Some("9".to_string()),
            "TEATRO_IGDB_CLIENT_ID" => Some("client".to_string()),
            "TEATRO_IGDB_CLIENT_SECRET" => Some("secret".to_string()),
            _ => None,
        })
        .expect("env config should load");

        assert_eq!(config.bind_addr, SocketAddr::from(([127, 0, 0, 1], 4444)));
        assert_eq!(config.database_url, "sqlite://custom/teatro.sqlite3");
        assert_eq!(
            config.default_library_root,
            PathBuf::from("/srv/teatro").join("roms")
        );
        assert_eq!(config.max_upload_bytes, 123);
        assert_eq!(config.download_archives.max_files, 12);
        assert_eq!(config.download_archives.max_source_bytes, 345);
        assert_eq!(config.download_archives.max_concurrent, 2);
        assert_eq!(config.download_archives.free_space_margin_bytes, 67);
        assert_eq!(config.log_format, LogFormat::Json);
        assert_eq!(
            config.lan_discovery,
            LanDiscoveryConfig {
                enabled: true,
                name: "Living Room".to_string(),
            }
        );
        assert!(config.gog_import.is_configured());
        assert!(!config.gog_import.bundled_extractor);
        assert_eq!(config.gog_import.timeout_seconds, 45);
        assert_eq!(config.gog_import.max_extracted_bytes, 456);
        assert_eq!(config.gog_import.max_extracted_files, 789);
        assert_eq!(
            config.gog_import.innoextract_sha256.as_deref(),
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(config.auth.rate_limit_max_failures, 3);
        assert_eq!(config.auth.rate_limit_window_seconds, 30);
        assert_eq!(config.auth.rate_limit_lockout_seconds, 10);
        assert_eq!(config.auth.password_queue_depth, 7);
        assert_eq!(config.auth.password_queue_timeout_seconds, 9);
        assert!(config.igdb.is_configured());
    }

    #[test]
    fn legacy_environment_and_runtime_paths_fail_closed() {
        assert_eq!(
            first_legacy_env_name(["PATH", "BEACON_DATABASE_URL", "TEATRO_DATABASE_URL"]),
            Some("BEACON_DATABASE_URL".to_string())
        );

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("roms");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(".beacon.lock"), b"").unwrap();
        let config = AppConfig::from_env_vars(|key| match key {
            "TEATRO_DATABASE_URL" => Some("sqlite::memory:".to_string()),
            "TEATRO_DATA_DIR" => Some(temp.path().join("data").display().to_string()),
            "TEATRO_DEFAULT_LIBRARY_ROOT" => Some(root.display().to_string()),
            _ => None,
        })
        .unwrap();
        assert!(
            config
                .ensure_runtime_paths()
                .unwrap_err()
                .to_string()
                .contains(".beacon.lock")
        );

        fs::remove_file(root.join(".beacon.lock")).unwrap();
        let database = temp.path().join("teatro.sqlite3");
        fs::write(temp.path().join("beacon.sqlite3"), b"").unwrap();
        let database_url = format!("sqlite://{}", database.display());
        let config = AppConfig::from_env_vars(|key| match key {
            "TEATRO_DATABASE_URL" => Some(database_url.clone()),
            "TEATRO_DATA_DIR" => Some(temp.path().join("data").display().to_string()),
            "TEATRO_DEFAULT_LIBRARY_ROOT" => Some(root.display().to_string()),
            _ => None,
        })
        .unwrap();
        assert!(
            config
                .ensure_runtime_paths()
                .unwrap_err()
                .to_string()
                .contains("beacon.sqlite3")
        );
    }

    #[test]
    fn discovery_name_rejects_empty_overlong_and_control_values() {
        for value in ["   ".to_string(), "x".repeat(49), "Teatro\n".to_string()] {
            let error = AppConfig::from_env_vars(|key| {
                (key == "TEATRO_DISCOVERY_NAME").then(|| value.clone())
            })
            .unwrap_err();
            assert!(error.to_string().contains("TEATRO_DISCOVERY_NAME"));
        }
    }

    #[test]
    fn download_archive_limits_must_be_non_zero() {
        for key in [
            "TEATRO_MAX_DOWNLOAD_ARCHIVE_FILES",
            "TEATRO_MAX_DOWNLOAD_ARCHIVE_BYTES",
            "TEATRO_MAX_CONCURRENT_DOWNLOAD_ARCHIVES",
        ] {
            let error =
                AppConfig::from_env_vars(|candidate| (candidate == key).then(|| "0".to_string()))
                    .unwrap_err();
            assert!(error.to_string().contains(key));
        }
    }

    #[test]
    fn enabled_gog_import_requires_an_absolute_hash_pinned_extractor() {
        let error = AppConfig::from_env_vars(|key| match key {
            "TEATRO_GOG_IMPORT_ENABLED" => Some("true".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("TEATRO_INNOEXTRACT_PATH"));

        let error = AppConfig::from_env_vars(|key| match key {
            "TEATRO_GOG_IMPORT_ENABLED" => Some("true".to_string()),
            "TEATRO_INNOEXTRACT_PATH" => Some("relative/innoextract".to_string()),
            "TEATRO_INNOEXTRACT_SHA256" => Some("not-a-hash".to_string()),
            _ => None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("TEATRO_INNOEXTRACT_SHA256"));
    }

    #[test]
    fn packaged_innoextract_sidecar_is_discovered_from_the_teatro_executable() {
        let temp = tempfile::tempdir().unwrap();
        let teatro = temp.path().join("teatro");
        let tools = temp.path().join("tools");
        let extractor = tools.join("innoextract");
        fs::create_dir(&tools).unwrap();
        fs::write(&teatro, b"teatro").unwrap();
        fs::write(&extractor, b"innoextract").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&extractor, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let digest = "ab".repeat(32);
        fs::write(
            tools.join("innoextract.sha256"),
            format!("{digest}  innoextract\n"),
        )
        .unwrap();

        let bundled = discover_bundled_innoextract(&teatro).unwrap().unwrap();
        assert_eq!(bundled.path, extractor);
        assert_eq!(bundled.sha256, digest);

        fs::write(
            tools.join("innoextract.sha256"),
            format!("{}  wrong-name\n", "cd".repeat(32)),
        )
        .unwrap();
        assert!(discover_bundled_innoextract(&teatro).is_err());
    }

    #[test]
    fn partial_packaged_innoextract_sidecars_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let teatro = temp.path().join("teatro");
        let tools = temp.path().join("tools");
        fs::create_dir(&tools).unwrap();
        fs::write(&teatro, b"teatro").unwrap();
        fs::write(
            tools.join("innoextract.sha256"),
            format!("{}  innoextract\n", "ef".repeat(32)),
        )
        .unwrap();

        let error = discover_bundled_innoextract(&teatro).unwrap_err();
        assert!(error.to_string().contains("must both be present"));
    }

    #[test]
    fn sqlite_database_path_handles_relative_and_absolute_urls() {
        assert_eq!(
            sqlite_database_path("sqlite://data/teatro.sqlite3"),
            Some(PathBuf::from("data/teatro.sqlite3"))
        );
        assert_eq!(
            sqlite_database_path("sqlite:///tmp/teatro.sqlite3?mode=rwc"),
            Some(PathBuf::from("/tmp/teatro.sqlite3"))
        );
        assert_eq!(sqlite_database_path("sqlite::memory:"), None);
    }
}
