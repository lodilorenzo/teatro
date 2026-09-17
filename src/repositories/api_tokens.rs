use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::{
    domain::{
        api_token::{ApiToken, ApiTokenScope},
        user::{InvalidUserRole, PublicUser, UserRole},
    },
    services::api_tokens::{self, ParseScopesError},
};

#[derive(Debug, Error)]
pub enum ApiTokenRepositoryError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    InvalidRole(#[from] InvalidUserRole),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    InvalidScopes(#[from] ParseScopesError),

    #[error("API token was not found")]
    NotFound,
}

pub struct CreateApiTokenParams<'a> {
    pub user_id: i64,
    pub name: &'a str,
    pub token_hash: &'a str,
    pub token_prefix: &'a str,
    pub scopes: &'a [ApiTokenScope],
    pub expires_at: Option<&'a str>,
}

pub async fn create(
    db: &SqlitePool,
    params: CreateApiTokenParams<'_>,
) -> Result<ApiToken, ApiTokenRepositoryError> {
    let scopes_json = api_tokens::scopes_to_json(params.scopes)?;
    let result = sqlx::query(
        r#"
        INSERT INTO api_tokens (
            user_id,
            name,
            token_hash,
            token_prefix,
            scopes_json,
            expires_at
        )
        VALUES (?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(params.user_id)
    .bind(params.name)
    .bind(params.token_hash)
    .bind(params.token_prefix)
    .bind(scopes_json)
    .bind(params.expires_at)
    .execute(db)
    .await?;

    find_by_id(db, result.last_insert_rowid())
        .await?
        .ok_or(ApiTokenRepositoryError::NotFound)
}

pub async fn list(db: &SqlitePool) -> Result<Vec<ApiToken>, ApiTokenRepositoryError> {
    let sql = select_sql_with_filter("1 = 1");
    let rows = sqlx::query_as::<_, ApiTokenRow>(&sql).fetch_all(db).await?;

    rows.into_iter().map(ApiTokenRow::try_into).collect()
}

pub async fn find_by_id(
    db: &SqlitePool,
    id: i64,
) -> Result<Option<ApiToken>, ApiTokenRepositoryError> {
    let sql = select_sql_with_filter("t.id = ?");
    let row = sqlx::query_as::<_, ApiTokenRow>(&sql)
        .bind(id)
        .fetch_optional(db)
        .await?;

    row.map(ApiTokenRow::try_into).transpose()
}

pub async fn find_active_by_hash(
    db: &SqlitePool,
    token_hash: &str,
) -> Result<Option<ApiToken>, ApiTokenRepositoryError> {
    let sql = select_sql_with_filter(
        "t.token_hash = ? AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
    );
    let row = sqlx::query_as::<_, ApiTokenRow>(&sql)
        .bind(token_hash)
        .fetch_optional(db)
        .await?;

    row.map(ApiTokenRow::try_into).transpose()
}

pub async fn touch_last_used_if_stale(
    db: &SqlitePool,
    id: i64,
    interval_seconds: u64,
) -> Result<(), ApiTokenRepositoryError> {
    sqlx::query(
        r#"
        UPDATE api_tokens
        SET
            last_used_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
          AND (
              last_used_at IS NULL
              OR julianday(last_used_at) <= julianday('now', ?)
          )
        "#,
    )
    .bind(id)
    .bind(format!("-{interval_seconds} seconds"))
    .execute(db)
    .await?;

    Ok(())
}

pub async fn revoke(db: &SqlitePool, id: i64) -> Result<ApiToken, ApiTokenRepositoryError> {
    sqlx::query(
        r#"
        UPDATE api_tokens
        SET
            revoked_at = COALESCE(revoked_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(id)
    .execute(db)
    .await?;

    find_by_id(db, id)
        .await?
        .ok_or(ApiTokenRepositoryError::NotFound)
}

fn select_sql_with_filter(filter: &str) -> String {
    format!(
        r#"
        SELECT
            t.id,
            t.user_id,
            u.username,
            u.role,
            t.name,
            t.token_prefix,
            t.scopes_json,
            t.expires_at,
            t.revoked_at,
            t.last_used_at,
            t.created_at
        FROM api_tokens t
        JOIN users u ON u.id = t.user_id
        WHERE {filter}
        ORDER BY t.created_at DESC, t.id DESC
        "#
    )
}

#[derive(Debug, FromRow)]
struct ApiTokenRow {
    id: i64,
    user_id: i64,
    username: String,
    role: String,
    name: String,
    token_prefix: String,
    scopes_json: String,
    expires_at: Option<String>,
    revoked_at: Option<String>,
    last_used_at: Option<String>,
    created_at: String,
}

impl TryFrom<ApiTokenRow> for ApiToken {
    type Error = ApiTokenRepositoryError;

    fn try_from(row: ApiTokenRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            user: PublicUser {
                id: row.user_id,
                username: row.username,
                role: row.role.parse::<UserRole>()?,
            },
            name: row.name,
            token_prefix: row.token_prefix,
            scopes: api_tokens::parse_scopes(&row.scopes_json)?,
            expires_at: row.expires_at,
            revoked_at: row.revoked_at,
            last_used_at: row.last_used_at,
            created_at: row.created_at,
        })
    }
}
