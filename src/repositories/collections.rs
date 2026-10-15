use serde::Serialize;
use sqlx::{FromRow, SqlitePool};

#[derive(Debug, Clone, Serialize, FromRow)]
pub(crate) struct Collection {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub lan_party_zone: bool,
    pub rom_count: i64,
}

const SELECT: &str = "SELECT c.*, (SELECT COUNT(*) FROM collection_roms cr WHERE cr.collection_id = c.id) AS rom_count FROM collections c";

pub(crate) async fn list(db: &SqlitePool) -> Result<Vec<Collection>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} ORDER BY c.name COLLATE NOCASE, c.id"))
        .fetch_all(db)
        .await
}

pub(crate) async fn find_by_slug(
    db: &SqlitePool,
    slug: &str,
) -> Result<Option<Collection>, sqlx::Error> {
    sqlx::query_as(&format!("{SELECT} WHERE c.slug = ?"))
        .bind(slug)
        .fetch_optional(db)
        .await
}

pub(crate) async fn rom_ids(db: &SqlitePool, id: i64) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT rom_id FROM collection_roms WHERE collection_id = ? ORDER BY rom_id")
        .bind(id)
        .fetch_all(db)
        .await
}

pub(crate) async fn contains(db: &SqlitePool, id: i64, rom_id: i64) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM collection_roms WHERE collection_id = ? AND rom_id = ?)",
    )
    .bind(id)
    .bind(rom_id)
    .fetch_one(db)
    .await
}

pub(crate) async fn contains_cover(
    db: &SqlitePool,
    id: i64,
    path: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM collection_roms cr JOIN roms r ON r.id = cr.rom_id WHERE cr.collection_id = ? AND (r.path_cover_large = ? OR r.path_cover_small = ?))")
        .bind(id)
        .bind(path)
        .bind(path)
        .fetch_one(db)
        .await
}

// Replace membership in the same transaction as the collection. Foreign keys reject missing games.
pub(crate) async fn save(
    db: &SqlitePool,
    id: Option<i64>,
    name: &str,
    slug: &str,
    lan_party_zone: bool,
    rom_ids: &[i64],
) -> Result<Option<i64>, sqlx::Error> {
    let mut tx = db.begin().await?;
    let id = if let Some(id) = id {
        let result = sqlx::query(
            "UPDATE collections SET name = ?, slug = ?, lan_party_zone = ? WHERE id = ?",
        )
        .bind(name)
        .bind(slug)
        .bind(lan_party_zone)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(None);
        }
        sqlx::query("DELETE FROM collection_roms WHERE collection_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        id
    } else {
        sqlx::query("INSERT INTO collections (name, slug, lan_party_zone) VALUES (?, ?, ?)")
            .bind(name)
            .bind(slug)
            .bind(lan_party_zone)
            .execute(&mut *tx)
            .await?
            .last_insert_rowid()
    };
    for rom_id in rom_ids {
        sqlx::query("INSERT INTO collection_roms (collection_id, rom_id) VALUES (?, ?)")
            .bind(id)
            .bind(rom_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Some(id))
}

pub(crate) async fn delete(db: &SqlitePool, id: i64) -> Result<bool, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM collections WHERE id = ?")
        .bind(id)
        .execute(db)
        .await?
        .rows_affected()
        != 0)
}
