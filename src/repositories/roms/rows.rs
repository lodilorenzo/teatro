use std::path::PathBuf;

use sqlx::FromRow;

use crate::domain::{
    library::{LibraryRootStats, PlatformLibraryStats},
    rom::{Rom, RomFile, RomFileDependency, RomFileGroup},
};

pub(super) fn rom_select_sql() -> String {
    format!(
        r#"
        SELECT
            r.id,
            r.name,
            r.slug,
            r.platform_id,
            p.slug AS platform_slug,
            p.display_name AS platform_display_name,
            r.regions_json,
            COALESCE(m.metadata_json, '{{}}') AS metadata_json,
            r.summary,
            pf.file_name AS fs_name,
            (
                SELECT SUM(size_file.file_size_bytes)
                FROM rom_files size_file
                WHERE size_file.rom_id = r.id
            ) AS fs_size_bytes,
            r.path_cover_large,
            r.path_cover_small,
            r.url_cover
        FROM roms r
        JOIN platforms p ON p.id = r.platform_id
        LEFT JOIN rom_metadata m ON m.rom_id = r.id
        LEFT JOIN rom_files pf ON pf.id = (
            SELECT candidate.id
            FROM rom_files candidate
            WHERE candidate.rom_id = r.id
            ORDER BY {}
            LIMIT 1
        )
        "#,
        preferred_file_sort_sql("candidate")
    )
}

pub(super) fn preferred_file_sort_sql(alias: &str) -> String {
    format!(
        r#"
        CASE
            WHEN {alias}.launchable = 1 AND {alias}.role = 'content' AND {alias}.is_primary = 1 THEN 0
            WHEN {alias}.launchable = 1 AND {alias}.role = 'launch_manifest' THEN 1
            WHEN {alias}.launchable = 1 AND {alias}.role IN ('descriptor', 'disc_image') THEN 2
            WHEN {alias}.launchable = 1 THEN 3
            WHEN {alias}.role IN ('track', 'archive_volume', 'metadata_sidecar') THEN 9
            ELSE 8
        END,
        {alias}.is_primary DESC,
        {alias}.sort_index,
        {alias}.disc_index IS NULL,
        {alias}.disc_index,
        {alias}.track_index IS NULL,
        {alias}.track_index,
        {alias}.file_name COLLATE NOCASE,
        {alias}.id
        "#
    )
}

#[derive(Debug, FromRow)]
pub(super) struct RomRow {
    id: i64,
    name: String,
    slug: String,
    platform_id: i64,
    platform_slug: String,
    platform_display_name: String,
    regions_json: String,
    metadata_json: String,
    summary: Option<String>,
    fs_name: Option<String>,
    fs_size_bytes: Option<i64>,
    path_cover_large: Option<String>,
    path_cover_small: Option<String>,
    url_cover: Option<String>,
}

impl TryFrom<RomRow> for Rom {
    type Error = sqlx::Error;

    fn try_from(row: RomRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            name: row.name,
            slug: row.slug,
            platform_id: row.platform_id,
            platform_slug: row.platform_slug,
            platform_display_name: row.platform_display_name,
            regions: decode_json("roms.regions_json", &row.regions_json)?,
            metadata: decode_json("rom_metadata.metadata_json", &row.metadata_json)?,
            summary: row.summary,
            fs_name: row.fs_name,
            fs_size_bytes: row.fs_size_bytes,
            path_cover_large: row.path_cover_large,
            path_cover_small: row.path_cover_small,
            url_cover: row.url_cover,
            files: Vec::new(),
        })
    }
}

#[derive(Debug, FromRow)]
pub(super) struct RomFileRow {
    id: i64,
    rom_id: i64,
    root_id: i64,
    root_path: String,
    relative_path: String,
    file_name: String,
    file_size_bytes: i64,
    crc32: Option<String>,
    md5: Option<String>,
    sha1: Option<String>,
    sha256: Option<String>,
    hash_status: String,
    hashed_at: Option<String>,
    hash_error: Option<String>,
    is_primary: i64,
    group_id: Option<i64>,
    original_file_name: Option<String>,
    role: String,
    sort_index: i64,
    disc_index: Option<i64>,
    track_index: Option<i64>,
    launchable: i64,
    metadata_json: String,
}

impl TryFrom<RomFileRow> for RomFile {
    type Error = sqlx::Error;

    fn try_from(row: RomFileRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            rom_id: row.rom_id,
            root_id: row.root_id,
            root_path: PathBuf::from(row.root_path),
            relative_path: row.relative_path,
            file_name: row.file_name,
            file_size_bytes: row.file_size_bytes,
            crc32: row.crc32,
            md5: row.md5,
            sha1: row.sha1,
            sha256: row.sha256,
            hash_status: row.hash_status.parse().map_err(decode_workflow_value)?,
            hashed_at: row.hashed_at,
            hash_error: row.hash_error,
            is_primary: row.is_primary != 0,
            group_id: row.group_id,
            original_file_name: row.original_file_name,
            role: row.role.parse().map_err(decode_relationship_value)?,
            sort_index: row.sort_index,
            disc_index: row.disc_index,
            track_index: row.track_index,
            launchable: row.launchable != 0,
            metadata: decode_json("rom_files.metadata_json", &row.metadata_json)?,
        })
    }
}

#[derive(Debug, FromRow)]
pub(super) struct RomFileGroupRow {
    id: i64,
    rom_id: i64,
    kind: String,
    display_name: String,
    group_key: Option<String>,
    disc_index: Option<i64>,
    disc_count: Option<i64>,
    launchable: i64,
    metadata_json: String,
}

impl TryFrom<RomFileGroupRow> for RomFileGroup {
    type Error = sqlx::Error;

    fn try_from(row: RomFileGroupRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            rom_id: row.rom_id,
            kind: row.kind.parse().map_err(decode_relationship_value)?,
            display_name: row.display_name,
            group_key: row.group_key,
            disc_index: row.disc_index,
            disc_count: row.disc_count,
            launchable: row.launchable != 0,
            metadata: decode_json("rom_file_groups.metadata_json", &row.metadata_json)?,
        })
    }
}

#[derive(Debug, FromRow)]
pub(super) struct RomFileDependencyRow {
    parent_file_id: i64,
    child_file_id: i64,
    dependency_kind: String,
    sort_index: i64,
}

impl TryFrom<RomFileDependencyRow> for RomFileDependency {
    type Error = sqlx::Error;

    fn try_from(row: RomFileDependencyRow) -> Result<Self, Self::Error> {
        Ok(Self {
            parent_file_id: row.parent_file_id,
            child_file_id: row.child_file_id,
            dependency_kind: row
                .dependency_kind
                .parse()
                .map_err(decode_relationship_value)?,
            sort_index: row.sort_index,
        })
    }
}

fn decode_json<T: serde::de::DeserializeOwned>(
    column: &str,
    value: &str,
) -> Result<T, sqlx::Error> {
    serde_json::from_str(value).map_err(|error| {
        sqlx::Error::Decode(format!("invalid JSON persisted in {column}: {error}").into())
    })
}

fn decode_relationship_value(
    error: crate::domain::rom::InvalidRomRelationshipValue,
) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

fn decode_workflow_value(error: crate::domain::workflow::InvalidWorkflowValue) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

#[derive(Debug, FromRow)]
pub(super) struct LibraryTotalsRow {
    pub(super) total_roms: i64,
    pub(super) total_files: i64,
    pub(super) total_file_bytes: i64,
    pub(super) platforms_with_roms: i64,
}

#[derive(Debug, FromRow)]
pub(super) struct LibraryRootStatsRow {
    id: i64,
    name: String,
    root_path: String,
    writable: i64,
    file_count: i64,
    total_file_bytes: i64,
}

impl From<LibraryRootStatsRow> for LibraryRootStats {
    fn from(row: LibraryRootStatsRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            root_path: PathBuf::from(row.root_path),
            writable: row.writable != 0,
            file_count: row.file_count,
            total_file_bytes: row.total_file_bytes,
        }
    }
}

#[derive(Debug, FromRow)]
pub(super) struct PlatformStatsRow {
    id: i64,
    slug: String,
    display_name: String,
    rom_count: i64,
    file_count: i64,
    total_file_bytes: i64,
}

impl From<PlatformStatsRow> for PlatformLibraryStats {
    fn from(row: PlatformStatsRow) -> Self {
        Self {
            id: row.id,
            slug: row.slug,
            display_name: row.display_name,
            rom_count: row.rom_count,
            file_count: row.file_count,
            total_file_bytes: row.total_file_bytes,
        }
    }
}

#[cfg(test)]
mod query_shape_tests {
    use super::rom_select_sql;

    #[test]
    fn preferred_file_lookup_is_correlated_to_selected_roms() {
        let sql = rom_select_sql();
        assert!(sql.contains("WHERE candidate.rom_id = r.id"));
        assert!(sql.contains("WHERE size_file.rom_id = r.id"));
        assert!(sql.contains("SUM(size_file.file_size_bytes)"));
        assert!(sql.contains("LIMIT 1"));
        assert!(!sql.contains("ROW_NUMBER"));
        assert!(!sql.contains("ranked_files"));
    }
}
