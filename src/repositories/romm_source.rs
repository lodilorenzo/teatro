//! Persistence for one configured RomM source and its complete local browse snapshot.

use sqlx::{FromRow, QueryBuilder, Sqlite, SqlitePool};

/// Programmatic authentication mode pinned for the configured RomM server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RommAuthMode {
    /// OAuth2 password grant against `POST {base}/api/token`.
    Token,
    /// HTTP Basic authentication on every request.
    Basic,
}

impl RommAuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Basic => "basic",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "token" => Some(Self::Token),
            "basic" => Some(Self::Basic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RommSourceSettings {
    pub base_url: String,
    pub username: Option<String>,
    pub secret: Option<String>,
    pub auth_mode: RommAuthMode,
    pub updated_at: String,
}

impl RommSourceSettings {
    pub fn has_credentials(&self) -> bool {
        match self.auth_mode {
            RommAuthMode::Token | RommAuthMode::Basic => {
                self.username.is_some() && self.secret.is_some()
            }
        }
    }
}

pub struct SaveRommSourceParams<'a> {
    pub base_url: &'a str,
    pub username: Option<&'a str>,
    pub secret: Option<&'a str>,
    pub auth_mode: RommAuthMode,
    /// A different endpoint, user, or auth mode may identify a different catalog.
    pub reset_index: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RommRemoteIndexStatus {
    pub refreshed_at: String,
    pub game_count: i64,
    pub platform_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRemotePlatform {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub rom_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRemoteRom {
    pub id: i64,
    pub name: String,
    pub platform_id: Option<i64>,
    pub platform_slug: Option<String>,
    pub platform_name: Option<String>,
    pub fs_name: Option<String>,
    pub file_size_bytes: Option<i64>,
    pub has_cover: bool,
    pub file_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedRemoteRomPage {
    pub items: Vec<IndexedRemoteRom>,
    pub total: i64,
    pub limit: u32,
    pub offset: u32,
}

pub async fn load(db: &SqlitePool) -> Result<Option<RommSourceSettings>, sqlx::Error> {
    let row = sqlx::query_as::<_, RommSourceSettingsRow>(
        r#"
        SELECT base_url, username, secret, auth_mode, updated_at
        FROM romm_source_settings
        WHERE id = 1
        "#,
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(RommSourceSettingsRow::into))
}

pub async fn save(
    db: &SqlitePool,
    params: SaveRommSourceParams<'_>,
) -> Result<RommSourceSettings, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(
        r#"
        INSERT INTO romm_source_settings (id, base_url, username, secret, auth_mode, updated_at)
        VALUES (1, ?, ?, ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ON CONFLICT(id) DO UPDATE SET
            base_url = excluded.base_url,
            username = excluded.username,
            secret = excluded.secret,
            auth_mode = excluded.auth_mode,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(params.base_url)
    .bind(clean_optional(params.username))
    .bind(clean_optional(params.secret))
    .bind(params.auth_mode.as_str())
    .execute(&mut *tx)
    .await?;
    if params.reset_index {
        clear_index_in(&mut tx).await?;
    }
    tx.commit().await?;

    load(db).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn clear(db: &SqlitePool) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    clear_index_in(&mut tx).await?;
    sqlx::query("DELETE FROM romm_source_settings WHERE id = 1")
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

pub async fn index_status(db: &SqlitePool) -> Result<Option<RommRemoteIndexStatus>, sqlx::Error> {
    sqlx::query_as::<_, RommRemoteIndexStatusRow>(
        "SELECT refreshed_at, game_count, platform_count FROM romm_remote_index WHERE id = 1",
    )
    .fetch_optional(db)
    .await
    .map(|row| row.map(Into::into))
}

/// Atomically replaces the browse snapshot after every remote page has downloaded successfully.
pub async fn replace_index(
    db: &SqlitePool,
    platforms: &[IndexedRemotePlatform],
    roms: &[IndexedRemoteRom],
) -> Result<RommRemoteIndexStatus, sqlx::Error> {
    let mut tx = db.begin().await?;
    clear_index_in(&mut tx).await?;

    for chunk in platforms.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO romm_remote_platforms (id, slug, name, rom_count) ",
        );
        query.push_values(chunk, |mut row, platform| {
            row.push_bind(platform.id)
                .push_bind(&platform.slug)
                .push_bind(&platform.name)
                .push_bind(platform.rom_count);
        });
        query.build().execute(&mut *tx).await?;
    }

    for chunk in roms.chunks(100) {
        let mut query = QueryBuilder::<Sqlite>::new(
            "INSERT INTO romm_remote_roms (id, name, platform_id, platform_slug, platform_name, fs_name, file_size_bytes, has_cover, file_count) ",
        );
        query.push_values(chunk, |mut row, rom| {
            row.push_bind(rom.id)
                .push_bind(&rom.name)
                .push_bind(rom.platform_id)
                .push_bind(&rom.platform_slug)
                .push_bind(&rom.platform_name)
                .push_bind(&rom.fs_name)
                .push_bind(rom.file_size_bytes)
                .push_bind(rom.has_cover)
                .push_bind(rom.file_count);
        });
        query.build().execute(&mut *tx).await?;
    }

    let game_count = i64::try_from(roms.len()).unwrap_or(i64::MAX);
    let platform_count = i64::try_from(platforms.len()).unwrap_or(i64::MAX);
    sqlx::query(
        r#"
        INSERT INTO romm_remote_index (id, refreshed_at, game_count, platform_count)
        VALUES (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), ?, ?)
        "#,
    )
    .bind(game_count)
    .bind(platform_count)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    index_status(db).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn indexed_platforms(db: &SqlitePool) -> Result<Vec<IndexedRemotePlatform>, sqlx::Error> {
    sqlx::query_as::<_, IndexedRemotePlatformRow>(
        r#"
        SELECT id, slug, name, rom_count
        FROM romm_remote_platforms
        ORDER BY name COLLATE NOCASE, id
        "#,
    )
    .fetch_all(db)
    .await
    .map(|rows| rows.into_iter().map(Into::into).collect())
}

pub async fn indexed_roms(
    db: &SqlitePool,
    platform_id: Option<i64>,
    search: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<IndexedRemoteRomPage, sqlx::Error> {
    let search = search.map(str::trim).filter(|value| !value.is_empty());
    let total = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT COUNT(*)
        FROM romm_remote_roms
        WHERE (? IS NULL OR platform_id = ?)
          AND (? IS NULL OR instr(lower(name), lower(?)) > 0)
        "#,
    )
    .bind(platform_id)
    .bind(platform_id)
    .bind(search)
    .bind(search)
    .fetch_one(db)
    .await?;

    let rows = sqlx::query_as::<_, IndexedRemoteRomRow>(
        r#"
        SELECT id, name, platform_id, platform_slug, platform_name, fs_name,
               file_size_bytes, has_cover, file_count
        FROM romm_remote_roms
        WHERE (? IS NULL OR platform_id = ?)
          AND (? IS NULL OR instr(lower(name), lower(?)) > 0)
        ORDER BY name COLLATE NOCASE, id
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(platform_id)
    .bind(platform_id)
    .bind(search)
    .bind(search)
    .bind(i64::from(limit))
    .bind(i64::from(offset))
    .fetch_all(db)
    .await?;

    Ok(IndexedRemoteRomPage {
        items: rows.into_iter().map(Into::into).collect(),
        total,
        limit,
        offset,
    })
}

async fn clear_index_in(tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM romm_remote_roms")
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM romm_remote_platforms")
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM romm_remote_index")
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, FromRow)]
struct RommSourceSettingsRow {
    base_url: String,
    username: Option<String>,
    secret: Option<String>,
    auth_mode: String,
    updated_at: String,
}

impl From<RommSourceSettingsRow> for RommSourceSettings {
    fn from(row: RommSourceSettingsRow) -> Self {
        Self {
            base_url: row.base_url,
            username: clean_optional(row.username.as_deref()),
            secret: clean_optional(row.secret.as_deref()),
            // A row whose mode is not recognized is treated as the pinned default rather than
            // failing the whole settings load; the connection test surfaces any mismatch.
            auth_mode: RommAuthMode::parse(&row.auth_mode).unwrap_or(RommAuthMode::Token),
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct RommRemoteIndexStatusRow {
    refreshed_at: String,
    game_count: i64,
    platform_count: i64,
}

impl From<RommRemoteIndexStatusRow> for RommRemoteIndexStatus {
    fn from(row: RommRemoteIndexStatusRow) -> Self {
        Self {
            refreshed_at: row.refreshed_at,
            game_count: row.game_count,
            platform_count: row.platform_count,
        }
    }
}

#[derive(Debug, FromRow)]
struct IndexedRemotePlatformRow {
    id: i64,
    slug: String,
    name: String,
    rom_count: i64,
}

impl From<IndexedRemotePlatformRow> for IndexedRemotePlatform {
    fn from(row: IndexedRemotePlatformRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            name: row.name,
            rom_count: row.rom_count,
        }
    }
}

#[derive(Debug, FromRow)]
struct IndexedRemoteRomRow {
    id: i64,
    name: String,
    platform_id: Option<i64>,
    platform_slug: Option<String>,
    platform_name: Option<String>,
    fs_name: Option<String>,
    file_size_bytes: Option<i64>,
    has_cover: bool,
    file_count: i64,
}

impl From<IndexedRemoteRomRow> for IndexedRemoteRom {
    fn from(row: IndexedRemoteRomRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            platform_id: row.platform_id,
            platform_slug: row.platform_slug,
            platform_name: row.platform_name,
            fs_name: row.fs_name,
            file_size_bytes: row.file_size_bytes,
            has_cover: row.has_cover,
            file_count: row.file_count,
        }
    }
}
