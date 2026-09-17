use sqlx::SqlitePool;

use crate::domain::library::LibraryStats;

use super::rows::{LibraryRootStatsRow, LibraryTotalsRow, PlatformStatsRow};

pub(crate) async fn library_stats(db: &SqlitePool) -> Result<LibraryStats, sqlx::Error> {
    let totals = sqlx::query_as::<_, LibraryTotalsRow>(
        r#"
        SELECT
            (SELECT COUNT(*) FROM roms) AS total_roms,
            (SELECT COUNT(*) FROM rom_files) AS total_files,
            (SELECT COALESCE(SUM(file_size_bytes), 0) FROM rom_files) AS total_file_bytes,
            (SELECT COUNT(*) FROM platforms WHERE id IN (SELECT DISTINCT platform_id FROM roms)) AS platforms_with_roms
        "#,
    )
    .fetch_one(db)
    .await?;

    let root_rows = sqlx::query_as::<_, LibraryRootStatsRow>(
        r#"
        SELECT
            lr.id,
            lr.name,
            lr.root_path,
            lr.writable,
            COUNT(rf.id) AS file_count,
            COALESCE(SUM(rf.file_size_bytes), 0) AS total_file_bytes
        FROM library_roots lr
        LEFT JOIN rom_files rf ON rf.root_id = lr.id
        GROUP BY lr.id, lr.name, lr.root_path, lr.writable
        ORDER BY lr.id
        "#,
    )
    .fetch_all(db)
    .await?;

    let platform_rows = sqlx::query_as::<_, PlatformStatsRow>(
        r#"
        SELECT
            p.id,
            p.slug,
            p.display_name,
            COUNT(DISTINCT r.id) AS rom_count,
            COUNT(rf.id) AS file_count,
            COALESCE(SUM(rf.file_size_bytes), 0) AS total_file_bytes
        FROM platforms p
        LEFT JOIN roms r ON r.platform_id = p.id
        LEFT JOIN rom_files rf ON rf.rom_id = r.id
        GROUP BY p.id, p.slug, p.display_name
        ORDER BY p.display_name COLLATE NOCASE, p.id
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(LibraryStats {
        total_roms: totals.total_roms,
        total_files: totals.total_files,
        total_file_bytes: totals.total_file_bytes,
        platforms_with_roms: totals.platforms_with_roms,
        library_roots: root_rows
            .into_iter()
            .map(LibraryRootStatsRow::into)
            .collect(),
        platforms: platform_rows
            .into_iter()
            .map(PlatformStatsRow::into)
            .collect(),
    })
}
