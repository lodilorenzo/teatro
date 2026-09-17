use std::path::{Path, PathBuf};

use sqlx::{FromRow, SqlitePool};

use crate::domain::library::LibraryRoot;

pub async fn ensure_default_root(db: &SqlitePool, root_path: &Path) -> Result<(), sqlx::Error> {
    let root_path = root_path.to_string_lossy();

    sqlx::query(
        r#"
        INSERT INTO library_roots (name, root_path, writable)
        VALUES ('Default library', ?, 1)
        ON CONFLICT(root_path) DO UPDATE SET
            name = excluded.name,
            writable = excluded.writable,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(root_path.as_ref())
    .execute(db)
    .await?;

    Ok(())
}

pub async fn find_by_id(db: &SqlitePool, id: i64) -> Result<Option<LibraryRoot>, sqlx::Error> {
    let row = sqlx::query_as::<_, LibraryRootRow>(
        r#"
        SELECT id, name, root_path, writable
        FROM library_roots
        WHERE id = ?
        LIMIT 1
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(LibraryRootRow::into))
}

pub async fn find_by_path(
    db: &SqlitePool,
    root_path: &Path,
) -> Result<Option<LibraryRoot>, sqlx::Error> {
    let row = sqlx::query_as::<_, LibraryRootRow>(
        r#"
        SELECT id, name, root_path, writable
        FROM library_roots
        WHERE root_path = ?
        LIMIT 1
        "#,
    )
    .bind(root_path.to_string_lossy().as_ref())
    .fetch_optional(db)
    .await?;

    Ok(row.map(LibraryRootRow::into))
}

#[derive(Debug, FromRow)]
struct LibraryRootRow {
    id: i64,
    name: String,
    root_path: String,
    writable: i64,
}

impl From<LibraryRootRow> for LibraryRoot {
    fn from(row: LibraryRootRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            root_path: PathBuf::from(row.root_path),
            writable: row.writable != 0,
        }
    }
}
