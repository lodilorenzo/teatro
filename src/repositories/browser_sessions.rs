use sqlx::SqlitePool;

use super::users::UserRepositoryError;
use crate::domain::user::PublicUser;

pub async fn create(
    db: &SqlitePool,
    token_hash: &str,
    user_id: i64,
    password_hash: &str,
    expires_at: i64,
) -> Result<bool, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "DELETE FROM browser_sessions WHERE expires_at <= unixepoch() \
         OR (user_id = ? AND password_hash != (SELECT password_hash FROM users WHERE id = ?))",
    )
    .bind(user_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    let result = sqlx::query(
        "INSERT INTO browser_sessions (token_hash, user_id, password_hash, expires_at) \
         SELECT ?, ?, ?, ? WHERE (SELECT COUNT(*) FROM browser_sessions WHERE user_id = ?) < 64",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(password_hash)
    .bind(expires_at)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(result.rows_affected() == 1)
}

pub async fn find_active_user(
    db: &SqlitePool,
    token_hash: &str,
) -> Result<Option<PublicUser>, UserRepositoryError> {
    let row: Option<(i64, String, String)> = sqlx::query_as(
        "SELECT u.id, u.username, u.role FROM browser_sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE s.token_hash = ? AND s.expires_at > unixepoch() \
         AND s.password_hash = u.password_hash",
    )
    .bind(token_hash)
    .fetch_optional(db)
    .await?;
    row.map(|(id, username, role)| {
        Ok(PublicUser {
            id,
            username,
            role: role.parse()?,
        })
    })
    .transpose()
}

pub async fn revoke(db: &SqlitePool, token_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM browser_sessions WHERE token_hash = ?")
        .bind(token_hash)
        .execute(db)
        .await?;
    Ok(())
}
