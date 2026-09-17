//! Persistence boundary for the managed-filesystem operation journal.
//!
//! Rows are decoded into typed kind/state values before the recovery service sees
//! them, preventing unknown persisted strings from entering the state machine.

use sqlx::{FromRow, SqliteConnection, SqlitePool};

use crate::domain::workflow::{FileOperationKind, FileOperationState, InvalidWorkflowValue};

#[derive(Debug, Clone)]
pub struct FileOperation {
    pub id: String,
    pub kind: FileOperationKind,
    pub state: FileOperationState,
    pub payload_json: String,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn create(
    db: &SqlitePool,
    id: &str,
    kind: FileOperationKind,
    payload_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO file_operations (id, kind, state, payload_json)
        VALUES (?, ?, 'prepared', ?)
        "#,
    )
    .bind(id)
    .bind(kind.as_str())
    .bind(payload_json)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn set_state(
    db: &SqlitePool,
    id: &str,
    state: FileOperationState,
    error_message: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE file_operations
        SET state = ?, error_message = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
          AND (state != 'completed' OR ? = 'completed')
        "#,
    )
    .bind(state.as_str())
    .bind(error_message)
    .bind(id)
    .bind(state.as_str())
    .execute(db)
    .await?;
    Ok(())
}

pub async fn state_by_id(
    db: &SqlitePool,
    id: &str,
) -> Result<Option<FileOperationState>, sqlx::Error> {
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM file_operations WHERE id = ?")
            .bind(id)
            .fetch_optional(db)
            .await?;
    state
        .map(|value| decode_workflow_value(value.parse()))
        .transpose()
}

pub async fn set_state_tx(
    connection: &mut SqliteConnection,
    id: &str,
    state: FileOperationState,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query(
        r#"
        UPDATE file_operations
        SET state = ?, error_message = NULL, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(state.as_str())
    .bind(id)
    .execute(connection)
    .await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    Ok(())
}

pub async fn pending(db: &SqlitePool) -> Result<Vec<FileOperation>, sqlx::Error> {
    let rows = sqlx::query_as::<_, FileOperationRow>(
        r#"
        SELECT id, kind, state, payload_json, error_message, created_at, updated_at
        FROM file_operations
        WHERE state != 'completed'
        ORDER BY created_at, id
        "#,
    )
    .fetch_all(db)
    .await?;
    rows.into_iter().map(FileOperation::try_from).collect()
}

pub async fn counts_by_state(
    db: &SqlitePool,
) -> Result<Vec<(FileOperationState, i64)>, sqlx::Error> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        r#"
        SELECT state, COUNT(*)
        FROM file_operations
        GROUP BY state
        ORDER BY state
        "#,
    )
    .fetch_all(db)
    .await?;
    rows.into_iter()
        .map(|(state, count)| Ok((decode_workflow_value(state.parse())?, count)))
        .collect()
}

pub async fn prune_completed(db: &SqlitePool, retain: i64) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        r#"
        DELETE FROM file_operations
        WHERE state = 'completed'
          AND id NOT IN (
              SELECT id
              FROM file_operations
              WHERE state = 'completed'
              ORDER BY created_at DESC, id DESC
              LIMIT ?
          )
        "#,
    )
    .bind(retain.max(0))
    .execute(db)
    .await?;
    Ok(result.rows_affected())
}

#[derive(Debug, FromRow)]
struct FileOperationRow {
    id: String,
    kind: String,
    state: String,
    payload_json: String,
    error_message: Option<String>,
    created_at: String,
    updated_at: String,
}

impl TryFrom<FileOperationRow> for FileOperation {
    type Error = sqlx::Error;

    fn try_from(row: FileOperationRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            kind: decode_workflow_value(row.kind.parse())?,
            state: decode_workflow_value(row.state.parse())?,
            payload_json: row.payload_json,
            error_message: row.error_message,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

fn decode_workflow_value<T>(value: Result<T, InvalidWorkflowValue>) -> Result<T, sqlx::Error> {
    value.map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
