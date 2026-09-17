use sqlx::{FromRow, SqlitePool};

use crate::config::IgdbConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgdbSettings {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub updated_at: String,
}

pub struct SaveIgdbSettingsParams<'a> {
    pub client_id: Option<&'a str>,
    pub client_secret: Option<&'a str>,
}

pub async fn load(db: &SqlitePool) -> Result<Option<IgdbSettings>, sqlx::Error> {
    let row = sqlx::query_as::<_, IgdbSettingsRow>(
        r#"
        SELECT client_id, client_secret, updated_at
        FROM igdb_settings
        WHERE id = 1
        "#,
    )
    .fetch_optional(db)
    .await?;

    Ok(row.map(IgdbSettingsRow::into))
}

pub async fn save(
    db: &SqlitePool,
    params: SaveIgdbSettingsParams<'_>,
) -> Result<IgdbSettings, sqlx::Error> {
    let client_id = clean_optional(params.client_id);
    let client_secret = clean_optional(params.client_secret);

    sqlx::query(
        r#"
        INSERT INTO igdb_settings (id, client_id, client_secret)
        VALUES (1, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            client_id = excluded.client_id,
            client_secret = excluded.client_secret,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(client_id)
    .bind(client_secret)
    .execute(db)
    .await?;

    load(db).await?.ok_or(sqlx::Error::RowNotFound)
}

pub async fn clear(db: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM igdb_settings WHERE id = 1")
        .execute(db)
        .await?;

    Ok(())
}

pub async fn effective_config(
    db: &SqlitePool,
    env_config: &IgdbConfig,
) -> Result<IgdbConfig, sqlx::Error> {
    let stored = load(db).await?;

    Ok(IgdbConfig {
        client_id: stored
            .as_ref()
            .and_then(|settings| settings.client_id.clone())
            .or_else(|| env_config.client_id.clone()),
        client_secret: stored
            .as_ref()
            .and_then(|settings| settings.client_secret.clone())
            .or_else(|| env_config.client_secret.clone()),
        token_url: env_config.token_url.clone(),
        api_url: env_config.api_url.clone(),
        image_base_url: env_config.image_base_url.clone(),
    })
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, FromRow)]
struct IgdbSettingsRow {
    client_id: Option<String>,
    client_secret: Option<String>,
    updated_at: String,
}

impl From<IgdbSettingsRow> for IgdbSettings {
    fn from(row: IgdbSettingsRow) -> Self {
        Self {
            client_id: clean_optional(row.client_id.as_deref()),
            client_secret: clean_optional(row.client_secret.as_deref()),
            updated_at: row.updated_at,
        }
    }
}
