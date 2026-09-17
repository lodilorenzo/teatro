use std::collections::{HashMap, HashSet};

use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::domain::rom::{
    PaginatedRoms, Rom, RomFile, RomFileDependency, RomFileGroup, RomListParams,
};

use super::rows::{
    RomFileDependencyRow, RomFileGroupRow, RomFileRow, RomRow, preferred_file_sort_sql,
    rom_select_sql,
};

pub async fn list(
    db: &SqlitePool,
    params: RomListParams,
    platform_ids: &[i64],
) -> Result<PaginatedRoms, sqlx::Error> {
    let mut count_builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT COUNT(*)
        FROM roms r
        WHERE 1 = 1
        "#,
    );
    push_platform_filter(&mut count_builder, platform_ids);
    push_search_filter(&mut count_builder, params.search.as_deref());
    push_missing_cover_filter(&mut count_builder, params.missing_cover);
    let total = count_builder
        .build_query_scalar::<i64>()
        .fetch_one(db)
        .await?;

    // Select the bounded ROM page before looking up each ROM's preferred file.
    // The previous ranked_files window processed every file in the library for
    // every page request, even when the response contained only a few ROMs.
    let ordering = if params.newest_first {
        "r.created_at DESC, r.id DESC"
    } else {
        "r.name COLLATE NOCASE, r.id"
    };
    let mut builder = QueryBuilder::<Sqlite>::new(
        "WITH selected_roms(id) AS (SELECT r.id FROM roms r WHERE 1 = 1",
    );
    push_platform_filter(&mut builder, platform_ids);
    push_search_filter(&mut builder, params.search.as_deref());
    push_missing_cover_filter(&mut builder, params.missing_cover);
    builder
        .push(" ORDER BY ")
        .push(ordering)
        .push(" LIMIT ")
        .push_bind(params.limit)
        .push(" OFFSET ")
        .push_bind(params.offset)
        .push(") ")
        .push(rom_select_sql())
        .push(" WHERE r.id IN (SELECT id FROM selected_roms)")
        .push(" ORDER BY ")
        .push(ordering);

    let rows = builder.build_query_as::<RomRow>().fetch_all(db).await?;
    let mut items: Vec<Rom> = rows
        .into_iter()
        .map(Rom::try_from)
        .collect::<Result<_, _>>()?;
    let rom_ids: Vec<i64> = items.iter().map(|rom| rom.id).collect();
    let mut files_by_rom = files_for_rom_ids(db, &rom_ids).await?;

    for rom in &mut items {
        rom.files = files_by_rom.remove(&rom.id).unwrap_or_default();
    }

    Ok(PaginatedRoms {
        items,
        total,
        limit: params.limit,
        offset: params.offset,
    })
}

pub async fn find_by_id(db: &SqlitePool, id: i64) -> Result<Option<Rom>, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new(rom_select_sql());
    builder.push(" WHERE r.id = ").push_bind(id);

    let Some(row) = builder
        .build_query_as::<RomRow>()
        .fetch_optional(db)
        .await?
    else {
        return Ok(None);
    };

    let mut rom = Rom::try_from(row)?;
    rom.files = files_for_rom(db, rom.id).await?;

    Ok(Some(rom))
}

pub(crate) async fn exists(db: &SqlitePool, rom_id: i64) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roms WHERE id = ?)")
        .bind(rom_id)
        .fetch_one(db)
        .await?;
    Ok(exists != 0)
}

pub(crate) async fn cover_paths_for_rom(
    db: &SqlitePool,
    rom_id: i64,
) -> Result<Option<(Option<String>, Option<String>)>, sqlx::Error> {
    sqlx::query_as("SELECT path_cover_large, path_cover_small FROM roms WHERE id = ?")
        .bind(rom_id)
        .fetch_optional(db)
        .await
}

async fn files_for_rom(db: &SqlitePool, rom_id: i64) -> Result<Vec<RomFile>, sqlx::Error> {
    let sql = format!(
        r#"
        SELECT
            rf.id,
            rf.rom_id,
            rf.root_id,
            lr.root_path,
            rf.relative_path,
            rf.file_name,
            rf.file_size_bytes,
            rf.crc32,
            rf.md5,
            rf.sha1,
            rf.sha256,
            rf.hash_status,
            rf.hashed_at,
            rf.hash_error,
            rf.is_primary,
            rf.group_id,
            rf.original_file_name,
            rf.role,
            rf.sort_index,
            rf.disc_index,
            rf.track_index,
            rf.launchable,
            rf.metadata_json
        FROM rom_files rf
        JOIN library_roots lr ON lr.id = rf.root_id
        WHERE rf.rom_id = ?
        ORDER BY {}
        "#,
        preferred_file_sort_sql("rf")
    );
    let rows = sqlx::query_as::<_, RomFileRow>(&sql)
        .bind(rom_id)
        .fetch_all(db)
        .await?;

    rows.into_iter().map(RomFile::try_from).collect()
}

async fn files_for_rom_ids(
    db: &SqlitePool,
    rom_ids: &[i64],
) -> Result<HashMap<i64, Vec<RomFile>>, sqlx::Error> {
    if rom_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        r#"
        SELECT
            rf.id,
            rf.rom_id,
            rf.root_id,
            lr.root_path,
            rf.relative_path,
            rf.file_name,
            rf.file_size_bytes,
            rf.crc32,
            rf.md5,
            rf.sha1,
            rf.sha256,
            rf.hash_status,
            rf.hashed_at,
            rf.hash_error,
            rf.is_primary,
            rf.group_id,
            rf.original_file_name,
            rf.role,
            rf.sort_index,
            rf.disc_index,
            rf.track_index,
            rf.launchable,
            rf.metadata_json
        FROM rom_files rf
        JOIN library_roots lr ON lr.id = rf.root_id
        WHERE rf.rom_id IN (
        "#,
    );
    let mut separated = builder.separated(", ");
    for rom_id in rom_ids {
        separated.push_bind(*rom_id);
    }
    separated.push_unseparated(") ORDER BY rf.rom_id, ");
    builder.push(preferred_file_sort_sql("rf"));

    let rows = builder.build_query_as::<RomFileRow>().fetch_all(db).await?;
    let mut files_by_rom: HashMap<i64, Vec<RomFile>> = HashMap::new();
    for row in rows {
        let file = RomFile::try_from(row)?;
        files_by_rom.entry(file.rom_id).or_default().push(file);
    }

    Ok(files_by_rom)
}

pub(crate) async fn find_file_by_id(
    db: &SqlitePool,
    rom_id: i64,
    file_id: i64,
) -> Result<Option<RomFile>, sqlx::Error> {
    let row = sqlx::query_as::<_, RomFileRow>(
        r#"
        SELECT
            rf.id,
            rf.rom_id,
            rf.root_id,
            lr.root_path,
            rf.relative_path,
            rf.file_name,
            rf.file_size_bytes,
            rf.crc32,
            rf.md5,
            rf.sha1,
            rf.sha256,
            rf.hash_status,
            rf.hashed_at,
            rf.hash_error,
            rf.is_primary,
            rf.group_id,
            rf.original_file_name,
            rf.role,
            rf.sort_index,
            rf.disc_index,
            rf.track_index,
            rf.launchable,
            rf.metadata_json
        FROM rom_files rf
        JOIN library_roots lr ON lr.id = rf.root_id
        WHERE rf.rom_id = ? AND rf.id = ?
        LIMIT 1
        "#,
    )
    .bind(rom_id)
    .bind(file_id)
    .fetch_optional(db)
    .await?;

    row.map(RomFile::try_from).transpose()
}

pub async fn file_groups_for_rom(
    db: &SqlitePool,
    rom_id: i64,
) -> Result<Vec<RomFileGroup>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RomFileGroupRow>(
        r#"
        SELECT
            id,
            rom_id,
            kind,
            display_name,
            group_key,
            disc_index,
            disc_count,
            launchable,
            metadata_json
        FROM rom_file_groups
        WHERE rom_id = ?
        ORDER BY
            CASE kind
                WHEN 'single' THEN 0
                WHEN 'playlist' THEN 1
                WHEN 'disc' THEN 2
                WHEN 'track_set' THEN 3
                WHEN 'archive_volume_set' THEN 8
                ELSE 7
            END,
            disc_index IS NULL,
            disc_index,
            display_name COLLATE NOCASE,
            id
        "#,
    )
    .bind(rom_id)
    .fetch_all(db)
    .await?;

    rows.into_iter().map(RomFileGroup::try_from).collect()
}

pub(crate) async fn cover_asset_paths_for_rom_ids(
    db: &SqlitePool,
    rom_ids: &[i64],
) -> Result<HashMap<i64, Vec<String>>, sqlx::Error> {
    if rom_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT rom_id, resource_path FROM cover_assets WHERE rom_id IN (",
    );
    let mut separated = builder.separated(", ");
    for rom_id in rom_ids {
        separated.push_bind(*rom_id);
    }
    separated.push_unseparated(") ORDER BY rom_id, resource_path");

    let rows = builder
        .build_query_as::<(i64, String)>()
        .fetch_all(db)
        .await?;
    let mut paths_by_rom: HashMap<i64, Vec<String>> = HashMap::new();
    for (rom_id, resource_path) in rows {
        paths_by_rom.entry(rom_id).or_default().push(resource_path);
    }
    Ok(paths_by_rom)
}

pub(crate) async fn file_dependencies_for_rom(
    db: &SqlitePool,
    rom_id: i64,
) -> Result<Vec<RomFileDependency>, sqlx::Error> {
    let rows = sqlx::query_as::<_, RomFileDependencyRow>(
        r#"
        SELECT
            d.parent_file_id,
            d.child_file_id,
            d.dependency_kind,
            d.sort_index
        FROM rom_file_dependencies d
        JOIN rom_files parent ON parent.id = d.parent_file_id
        JOIN rom_files child ON child.id = d.child_file_id
        WHERE parent.rom_id = ? AND child.rom_id = ?
        ORDER BY d.sort_index, d.dependency_kind, d.parent_file_id, d.child_file_id
        "#,
    )
    .bind(rom_id)
    .bind(rom_id)
    .fetch_all(db)
    .await?;

    rows.into_iter().map(RomFileDependency::try_from).collect()
}

pub(crate) async fn slug_exists(
    db: &SqlitePool,
    platform_id: i64,
    slug: &str,
) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM roms
            WHERE platform_id = ? AND slug = ?
        )
        "#,
    )
    .bind(platform_id)
    .bind(slug)
    .fetch_one(db)
    .await?;

    Ok(exists != 0)
}

pub(crate) async fn ids_by_platform(
    db: &SqlitePool,
    platform_id: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT id
        FROM roms
        WHERE platform_id = ?
        ORDER BY id
        "#,
    )
    .bind(platform_id)
    .fetch_all(db)
    .await
}

pub(crate) async fn all_ids(db: &SqlitePool) -> Result<Vec<i64>, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT id
        FROM roms
        ORDER BY id
        "#,
    )
    .fetch_all(db)
    .await
}

pub(crate) async fn existing_file_names(
    db: &SqlitePool,
    platform_id: i64,
    file_names: &[String],
) -> Result<HashSet<String>, sqlx::Error> {
    if file_names.is_empty() {
        return Ok(HashSet::new());
    }

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT rf.file_name FROM rom_files rf \
         JOIN roms r ON r.id = rf.rom_id \
         WHERE r.platform_id = ",
    );
    builder
        .push_bind(platform_id)
        .push(" AND rf.file_name IN (");
    let mut separated = builder.separated(", ");
    for file_name in file_names {
        separated.push_bind(file_name);
    }
    separated.push_unseparated(")");

    Ok(builder
        .build_query_scalar::<String>()
        .fetch_all(db)
        .await?
        .into_iter()
        .collect())
}

/// A local file whose stored hash matches a candidate hash, used for duplicate refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HashMatchedFile {
    pub rom_id: i64,
    pub rom_name: String,
    pub file_name: String,
    pub algorithm: &'static str,
}

/// Finds the first local file matching any supplied hash, strongest algorithm first.
///
/// Identical bytes anywhere in the library count, so this deliberately ignores the platform.
pub(crate) async fn find_file_by_hashes(
    db: &SqlitePool,
    sha256: &[String],
    sha1: &[String],
    md5: &[String],
    crc32: &[String],
) -> Result<Option<HashMatchedFile>, sqlx::Error> {
    for (algorithm, column, values) in [
        ("sha256", "rf.sha256", sha256),
        ("sha1", "rf.sha1", sha1),
        ("md5", "rf.md5", md5),
        ("crc32", "rf.crc32", crc32),
    ] {
        let values = values
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        if values.is_empty() {
            continue;
        }

        let mut builder = QueryBuilder::<Sqlite>::new(
            "SELECT r.id, r.name, rf.file_name FROM rom_files rf \
             JOIN roms r ON r.id = rf.rom_id WHERE lower(",
        );
        builder.push(column).push(") IN (");
        let mut separated = builder.separated(", ");
        for value in &values {
            separated.push_bind(value.clone());
        }
        separated.push_unseparated(") LIMIT 1");

        if let Some((rom_id, rom_name, file_name)) = builder
            .build_query_as::<(i64, String, String)>()
            .fetch_optional(db)
            .await?
        {
            return Ok(Some(HashMatchedFile {
                rom_id,
                rom_name,
                file_name,
                algorithm,
            }));
        }
    }
    Ok(None)
}

pub(crate) async fn folder_has_files_outside_roms(
    db: &SqlitePool,
    root_id: i64,
    folder_relative_path: &str,
    rom_ids: &[i64],
) -> Result<bool, sqlx::Error> {
    if rom_ids.is_empty() {
        return Ok(false);
    }
    let prefix = format!("{folder_relative_path}/");
    let prefix_characters = i64::try_from(prefix.chars().count())
        .map_err(|_| sqlx::Error::Protocol("managed folder path is too long".into()))?;
    let mut builder =
        QueryBuilder::<Sqlite>::new("SELECT EXISTS(SELECT 1 FROM rom_files WHERE root_id = ");
    builder
        .push_bind(root_id)
        .push(" AND substr(relative_path, 1, ")
        .push_bind(prefix_characters)
        .push(") = ")
        .push_bind(prefix)
        .push(" AND rom_id NOT IN (");
    let mut separated = builder.separated(", ");
    for rom_id in rom_ids {
        separated.push_bind(*rom_id);
    }
    separated.push_unseparated("))");
    Ok(builder.build_query_scalar::<i64>().fetch_one(db).await? != 0)
}

pub(crate) async fn indexed_relative_paths(
    db: &SqlitePool,
    root_id: i64,
) -> Result<HashSet<String>, sqlx::Error> {
    Ok(
        sqlx::query_scalar("SELECT relative_path FROM rom_files WHERE root_id = ?")
            .bind(root_id)
            .fetch_all(db)
            .await?
            .into_iter()
            .collect(),
    )
}

pub(crate) async fn relative_path_exists(
    db: &SqlitePool,
    root_id: i64,
    relative_path: &str,
) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM rom_files
            WHERE root_id = ? AND relative_path = ?
        )
        "#,
    )
    .bind(root_id)
    .bind(relative_path)
    .fetch_one(db)
    .await?;

    Ok(exists != 0)
}

pub(crate) async fn slug_exists_excluding(
    db: &SqlitePool,
    platform_id: i64,
    slug: &str,
    excluded_rom_id: i64,
) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar(
        r#"
        SELECT EXISTS(
            SELECT 1
            FROM roms
            WHERE platform_id = ? AND slug = ? AND id != ?
        )
        "#,
    )
    .bind(platform_id)
    .bind(slug)
    .bind(excluded_rom_id)
    .fetch_one(db)
    .await?;

    Ok(exists != 0)
}

fn push_search_filter(builder: &mut QueryBuilder<'_, Sqlite>, search: Option<&str>) {
    let Some(search) = search.map(str::trim).filter(|search| !search.is_empty()) else {
        return;
    };

    builder
        .push(" AND r.name LIKE ")
        .push_bind(format!("%{search}%"));
}

fn push_missing_cover_filter(builder: &mut QueryBuilder<'_, Sqlite>, missing_cover: bool) {
    if missing_cover {
        builder.push(
            " AND NULLIF(TRIM(r.path_cover_large), '') IS NULL AND NULLIF(TRIM(r.path_cover_small), '') IS NULL",
        );
    }
}

fn push_platform_filter(builder: &mut QueryBuilder<'_, Sqlite>, platform_ids: &[i64]) {
    if platform_ids.is_empty() {
        return;
    }

    builder.push(" AND r.platform_id IN (");
    for (index, platform_id) in platform_ids.iter().enumerate() {
        if index > 0 {
            builder.push(", ");
        }
        builder.push_bind(*platform_id);
    }
    builder.push(")");
}
