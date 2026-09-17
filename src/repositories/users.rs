use sqlx::{FromRow, SqlitePool};
use thiserror::Error;

use crate::domain::user::{InvalidUserRole, User, UserRole};

#[derive(Debug, Error)]
pub enum UserRepositoryError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    InvalidRole(#[from] InvalidUserRole),

    #[error("invalid username {username:?}: {reason}")]
    InvalidUsername { username: String, reason: String },

    #[error("user {0:?} already exists")]
    AlreadyExists(String),

    #[error("user was not found")]
    NotFound,

    #[error("operation would remove or demote the last admin user")]
    LastAdmin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateAdminOutcome {
    Created,
    PasswordReset,
}

pub async fn list(db: &SqlitePool) -> Result<Vec<User>, UserRepositoryError> {
    let rows = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT id, username, password_hash, role
        FROM users
        ORDER BY username COLLATE NOCASE, id
        "#,
    )
    .fetch_all(db)
    .await?;

    rows.into_iter().map(UserRow::try_into).collect()
}

pub async fn has_admin(db: &SqlitePool) -> Result<bool, UserRepositoryError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE role = 'admin')")
            .fetch_one(db)
            .await?;

    Ok(exists)
}

pub async fn find_by_id(db: &SqlitePool, id: i64) -> Result<Option<User>, UserRepositoryError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT id, username, password_hash, role
        FROM users
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;

    row.map(UserRow::try_into).transpose()
}

pub async fn find_by_username(
    db: &SqlitePool,
    username: &str,
) -> Result<Option<User>, UserRepositoryError> {
    let row = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT id, username, password_hash, role
        FROM users
        WHERE username = ?
        LIMIT 1
        "#,
    )
    .bind(username)
    .fetch_optional(db)
    .await?;

    row.map(UserRow::try_into).transpose()
}

pub async fn create(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
    role: UserRole,
) -> Result<User, UserRepositoryError> {
    validate_username(username)?;

    if find_by_username(db, username).await?.is_some() {
        return Err(UserRepositoryError::AlreadyExists(username.to_string()));
    }

    let result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, role)
        VALUES (?, ?, ?)
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .bind(role.as_str())
    .execute(db)
    .await?;

    find_by_id(db, result.last_insert_rowid())
        .await?
        .ok_or(UserRepositoryError::NotFound)
}

pub async fn create_admin(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
    reset_existing_password: bool,
) -> Result<CreateAdminOutcome, UserRepositoryError> {
    validate_username(username)?;

    if let Some(existing) = find_by_username(db, username).await? {
        if !reset_existing_password {
            return Err(UserRepositoryError::AlreadyExists(username.to_string()));
        }

        update_password(db, existing.id, password_hash).await?;
        set_role(db, existing.id, UserRole::Admin).await?;

        return Ok(CreateAdminOutcome::PasswordReset);
    }

    create(db, username, password_hash, UserRole::Admin).await?;

    Ok(CreateAdminOutcome::Created)
}

pub async fn create_initial_admin(
    db: &SqlitePool,
    username: &str,
    password_hash: &str,
) -> Result<Option<User>, UserRepositoryError> {
    validate_username(username)?;

    if find_by_username(db, username).await?.is_some() {
        return Err(UserRepositoryError::AlreadyExists(username.to_string()));
    }

    let result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, role)
        SELECT ?, ?, 'admin'
        WHERE NOT EXISTS (SELECT 1 FROM users WHERE role = 'admin')
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .execute(db)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    find_by_id(db, result.last_insert_rowid()).await
}

pub async fn update_password(
    db: &SqlitePool,
    id: i64,
    password_hash: &str,
) -> Result<User, UserRepositoryError> {
    sqlx::query(
        r#"
        UPDATE users
        SET password_hash = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(password_hash)
    .bind(id)
    .execute(db)
    .await?;

    find_by_id(db, id)
        .await?
        .ok_or(UserRepositoryError::NotFound)
}

pub async fn set_role(
    db: &SqlitePool,
    id: i64,
    role: UserRole,
) -> Result<User, UserRepositoryError> {
    let existing = find_by_id(db, id)
        .await?
        .ok_or(UserRepositoryError::NotFound)?;

    let result = sqlx::query(
        r#"
        UPDATE users
        SET role = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
          AND (
              role != 'admin'
              OR ? = 'admin'
              OR (SELECT COUNT(*) FROM users WHERE role = 'admin') > 1
          )
        "#,
    )
    .bind(role.as_str())
    .bind(id)
    .bind(role.as_str())
    .execute(db)
    .await?;

    if result.rows_affected() == 0 && existing.role.is_admin() && !role.is_admin() {
        return Err(UserRepositoryError::LastAdmin);
    }

    find_by_id(db, id)
        .await?
        .ok_or(UserRepositoryError::NotFound)
}

pub async fn delete(db: &SqlitePool, id: i64) -> Result<User, UserRepositoryError> {
    let existing = find_by_id(db, id)
        .await?
        .ok_or(UserRepositoryError::NotFound)?;

    let result = sqlx::query(
        r#"
        DELETE FROM users
        WHERE id = ?
          AND (
              role != 'admin'
              OR (SELECT COUNT(*) FROM users WHERE role = 'admin') > 1
          )
        "#,
    )
    .bind(id)
    .execute(db)
    .await?;

    if result.rows_affected() == 0 && existing.role.is_admin() {
        return Err(UserRepositoryError::LastAdmin);
    }

    Ok(existing)
}

fn validate_username(username: &str) -> Result<(), UserRepositoryError> {
    if username.is_empty() {
        return Err(UserRepositoryError::InvalidUsername {
            username: username.to_string(),
            reason: "username cannot be empty".to_string(),
        });
    }

    if username.trim() != username {
        return Err(UserRepositoryError::InvalidUsername {
            username: username.to_string(),
            reason: "leading or trailing whitespace is not allowed".to_string(),
        });
    }

    if username.len() > 128 {
        return Err(UserRepositoryError::InvalidUsername {
            username: username.to_string(),
            reason: "username is too long".to_string(),
        });
    }

    if username.contains(':') {
        return Err(UserRepositoryError::InvalidUsername {
            username: username.to_string(),
            reason: "colon is not allowed because it is reserved by HTTP Basic Auth".to_string(),
        });
    }

    Ok(())
}

#[derive(Debug, FromRow)]
struct UserRow {
    id: i64,
    username: String,
    password_hash: String,
    role: String,
}

impl TryFrom<UserRow> for User {
    type Error = UserRepositoryError;

    fn try_from(row: UserRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            username: row.username,
            password_hash: row.password_hash,
            role: row.role.parse()?,
        })
    }
}
