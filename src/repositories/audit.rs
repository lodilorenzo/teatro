use sqlx::{SqliteConnection, SqlitePool};

pub struct AuditEvent<'a> {
    pub actor_user_id: Option<i64>,
    pub action: &'a str,
    pub entity_type: Option<&'a str>,
    pub entity_id: Option<i64>,
    pub metadata_json: Option<&'a str>,
}

pub async fn record(db: &SqlitePool, event: AuditEvent<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata_json)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(event.actor_user_id)
    .bind(event.action)
    .bind(event.entity_type)
    .bind(event.entity_id)
    .bind(event.metadata_json)
    .execute(db)
    .await?;

    Ok(())
}

pub async fn record_tx(
    connection: &mut SqliteConnection,
    event: AuditEvent<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO audit_log (actor_user_id, action, entity_type, entity_id, metadata_json)
        VALUES (?, ?, ?, ?, ?)
        "#,
    )
    .bind(event.actor_user_id)
    .bind(event.action)
    .bind(event.entity_type)
    .bind(event.entity_id)
    .bind(event.metadata_json)
    .execute(connection)
    .await?;
    Ok(())
}
