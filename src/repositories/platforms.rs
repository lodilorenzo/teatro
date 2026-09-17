use sqlx::{FromRow, SqlitePool};

use crate::domain::platform::Platform;

pub async fn list_with_rom_counts(db: &SqlitePool) -> Result<Vec<Platform>, sqlx::Error> {
    let platforms = sqlx::query_as::<_, PlatformRow>(
        r#"
        SELECT
            p.id,
            p.name,
            p.display_name,
            p.slug,
            p.fs_slug,
            COUNT(r.id) AS rom_count
        FROM platforms p
        LEFT JOIN roms r ON r.platform_id = p.id
        GROUP BY p.id, p.name, p.display_name, p.slug, p.fs_slug
        ORDER BY p.display_name COLLATE NOCASE, p.id
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(platforms.into_iter().map(PlatformRow::into).collect())
}

pub async fn find_by_id(db: &SqlitePool, id: i64) -> Result<Option<Platform>, sqlx::Error> {
    let sql = platform_select_sql_with_filter("p.id = ?");
    let row = sqlx::query_as::<_, PlatformRow>(&sql)
        .bind(id)
        .fetch_optional(db)
        .await?;

    Ok(row.map(PlatformRow::into))
}

pub async fn find_by_slug(db: &SqlitePool, slug: &str) -> Result<Option<Platform>, sqlx::Error> {
    let sql = platform_select_sql_with_filter("p.slug = ?");
    let row = sqlx::query_as::<_, PlatformRow>(&sql)
        .bind(slug)
        .fetch_optional(db)
        .await?;

    Ok(row.map(PlatformRow::into))
}

fn platform_select_sql_with_filter(filter: &str) -> String {
    format!(
        r#"
        SELECT
            p.id,
            p.name,
            p.display_name,
            p.slug,
            p.fs_slug,
            COUNT(r.id) AS rom_count
        FROM platforms p
        LEFT JOIN roms r ON r.platform_id = p.id
        WHERE {filter}
        GROUP BY p.id, p.name, p.display_name, p.slug, p.fs_slug
        LIMIT 1
        "#
    )
}

#[derive(Debug, FromRow)]
struct PlatformRow {
    id: i64,
    name: String,
    display_name: String,
    slug: String,
    fs_slug: String,
    rom_count: i64,
}

impl From<PlatformRow> for Platform {
    fn from(row: PlatformRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            display_name: row.display_name,
            slug: row.slug,
            fs_slug: row.fs_slug,
            rom_count: row.rom_count,
        }
    }
}
