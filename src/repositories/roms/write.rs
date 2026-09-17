use std::collections::HashMap;

use sqlx::{QueryBuilder, Sqlite, SqliteConnection, SqlitePool};

use crate::{
    domain::{ingest::PlanFileKey, rom::Rom, workflow::FileOperationState},
    repositories::file_operations,
};

use super::{
    params::{
        CreateGroupedRomParams, CreateRomFile, CreateRomFileDependency, CreateRomFileGroup,
        CreateRomWithFileParams, SaveCoverParams, SaveIgdbMetadataParams, UpdateRomMetadataParams,
    },
    read::find_by_id,
};

#[cfg(test)]
pub(crate) async fn create_stub(
    db: &SqlitePool,
    platform_id: i64,
    name: &str,
    slug: &str,
) -> Result<i64, sqlx::Error> {
    Ok(sqlx::query(
        "INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, ?, ?, '[]')",
    )
    .bind(platform_id)
    .bind(name)
    .bind(slug)
    .execute(db)
    .await?
    .last_insert_rowid())
}

pub(crate) async fn create_with_file(
    db: &SqlitePool,
    params: CreateRomWithFileParams<'_>,
) -> Result<(i64, i64), sqlx::Error> {
    let mut tx = db.begin().await?;

    let rom_id = sqlx::query(
        r#"
        INSERT INTO roms (platform_id, name, slug, regions_json)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(params.platform_id)
    .bind(params.name)
    .bind(params.slug)
    .bind(params.regions_json)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    let group_id = sqlx::query(
        r#"
        INSERT INTO rom_file_groups (
            rom_id,
            kind,
            display_name,
            group_key,
            disc_index,
            disc_count,
            launchable,
            metadata_json
        )
        VALUES (?, 'single', ?, ?, NULL, NULL, 1, '{}')
        "#,
    )
    .bind(rom_id)
    .bind(params.name)
    .bind(params.slug)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    let file_id = sqlx::query(
        r#"
        INSERT INTO rom_files (
            rom_id,
            root_id,
            relative_path,
            file_name,
            file_size_bytes,
            sha256,
            is_primary,
            group_id,
            original_file_name,
            role,
            sort_index,
            disc_index,
            track_index,
            launchable,
            metadata_json
        )
        VALUES (?, ?, ?, ?, ?, NULL, 1, ?, ?, 'content', 0, NULL, NULL, 1, '{}')
        "#,
    )
    .bind(rom_id)
    .bind(params.root_id)
    .bind(params.relative_path)
    .bind(params.file_name)
    .bind(params.file_size_bytes)
    .bind(group_id)
    .bind(params.original_file_name)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    if let Some(metadata_json) = params.metadata_json {
        sqlx::query(
            r#"
            INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
            VALUES (?, 'filename', ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            "#,
        )
        .bind(rom_id)
        .bind(metadata_json)
        .execute(&mut *tx)
        .await?;
    }

    file_operations::set_state_tx(
        &mut tx,
        params.file_operation_id,
        FileOperationState::DbCommitted,
    )
    .await?;
    tx.commit().await?;

    Ok((rom_id, file_id))
}

pub(crate) async fn create_grouped_roms(
    db: &SqlitePool,
    params: Vec<CreateGroupedRomParams>,
    file_operation_id: Option<&str>,
) -> Result<Vec<i64>, sqlx::Error> {
    let mut tx = db.begin().await?;
    let mut rom_ids = Vec::with_capacity(params.len());
    for rom in params {
        rom_ids.push(create_grouped_rom(&mut tx, rom).await?);
    }
    if let Some(file_operation_id) = file_operation_id {
        file_operations::set_state_tx(&mut tx, file_operation_id, FileOperationState::DbCommitted)
            .await?;
    }
    tx.commit().await?;
    Ok(rom_ids)
}

async fn create_grouped_rom(
    connection: &mut SqliteConnection,
    rom: CreateGroupedRomParams,
) -> Result<i64, sqlx::Error> {
    let rom_id =
        sqlx::query("INSERT INTO roms (platform_id, name, slug, regions_json) VALUES (?, ?, ?, ?)")
            .bind(rom.platform_id)
            .bind(&rom.name)
            .bind(&rom.slug)
            .bind(&rom.regions_json)
            .execute(&mut *connection)
            .await?
            .last_insert_rowid();
    let group_ids = insert_file_groups(connection, rom_id, rom.groups).await?;
    let file_ids = insert_rom_files(connection, rom_id, rom.files, &group_ids).await?;
    insert_file_dependencies(connection, rom.dependencies, &file_ids).await?;
    if let Some(metadata_json) = rom.metadata_json {
        insert_filename_metadata(connection, rom_id, &metadata_json).await?;
    }
    Ok(rom_id)
}

async fn insert_file_groups(
    connection: &mut SqliteConnection,
    rom_id: i64,
    groups: Vec<CreateRomFileGroup>,
) -> Result<HashMap<String, i64>, sqlx::Error> {
    let mut group_ids = HashMap::with_capacity(groups.len());
    for group in groups {
        let group_id = sqlx::query(
            r#"
            INSERT INTO rom_file_groups (
                rom_id, kind, display_name, group_key, disc_index, disc_count,
                launchable, metadata_json
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(rom_id)
        .bind(group.kind.as_str())
        .bind(&group.display_name)
        .bind(group.group_key.as_deref())
        .bind(group.disc_index)
        .bind(group.disc_count)
        .bind(if group.launchable { 1_i64 } else { 0_i64 })
        .bind(&group.metadata_json)
        .execute(&mut *connection)
        .await?
        .last_insert_rowid();
        group_ids.insert(group.key, group_id);
    }
    Ok(group_ids)
}

async fn insert_rom_files(
    connection: &mut SqliteConnection,
    rom_id: i64,
    files: Vec<CreateRomFile>,
    group_ids: &HashMap<String, i64>,
) -> Result<HashMap<PlanFileKey, i64>, sqlx::Error> {
    let mut file_ids = HashMap::with_capacity(files.len());
    for file in files {
        let group_id = file
            .group_key
            .as_ref()
            .map(|key| {
                group_ids.get(key).copied().ok_or_else(|| {
                    sqlx::Error::Protocol(format!(
                        "planned file references unknown group key {key:?}"
                    ))
                })
            })
            .transpose()?;
        let file_id = sqlx::query(
            r#"
            INSERT INTO rom_files (
                rom_id, root_id, relative_path, file_name, file_size_bytes, sha256,
                is_primary, group_id, original_file_name, role, sort_index,
                disc_index, track_index, launchable, metadata_json
            )
            VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(rom_id)
        .bind(file.root_id)
        .bind(&file.relative_path)
        .bind(&file.file_name)
        .bind(file.file_size_bytes)
        .bind(if file.is_primary { 1_i64 } else { 0_i64 })
        .bind(group_id)
        .bind(&file.original_file_name)
        .bind(file.role.as_str())
        .bind(file.sort_index)
        .bind(file.disc_index)
        .bind(file.track_index)
        .bind(if file.launchable { 1_i64 } else { 0_i64 })
        .bind(&file.metadata_json)
        .execute(&mut *connection)
        .await?
        .last_insert_rowid();
        file_ids.insert(file.key, file_id);
    }
    Ok(file_ids)
}

async fn insert_file_dependencies(
    connection: &mut SqliteConnection,
    dependencies: Vec<CreateRomFileDependency>,
    file_ids: &HashMap<PlanFileKey, i64>,
) -> Result<(), sqlx::Error> {
    for dependency in dependencies {
        let parent_id = file_ids
            .get(&dependency.parent_file_key)
            .copied()
            .ok_or(sqlx::Error::RowNotFound)?;
        let child_id = file_ids
            .get(&dependency.child_file_key)
            .copied()
            .ok_or(sqlx::Error::RowNotFound)?;
        sqlx::query(
            r#"
            INSERT INTO rom_file_dependencies (
                parent_file_id, child_file_id, dependency_kind, sort_index
            )
            VALUES (?, ?, ?, ?)
            "#,
        )
        .bind(parent_id)
        .bind(child_id)
        .bind(dependency.dependency_kind.as_str())
        .bind(dependency.sort_index)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

async fn insert_filename_metadata(
    connection: &mut SqliteConnection,
    rom_id: i64,
    metadata_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
        VALUES (?, 'filename', ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        "#,
    )
    .bind(rom_id)
    .bind(metadata_json)
    .execute(connection)
    .await?;
    Ok(())
}

pub(crate) async fn delete_ids_tx(
    connection: &mut SqliteConnection,
    rom_ids: &[i64],
) -> Result<u64, sqlx::Error> {
    if rom_ids.is_empty() {
        return Ok(0);
    }

    let mut builder = QueryBuilder::<Sqlite>::new("DELETE FROM roms WHERE id IN (");
    let mut separated = builder.separated(", ");
    for rom_id in rom_ids {
        separated.push_bind(*rom_id);
    }
    separated.push_unseparated(")");
    let result = builder.build().execute(connection).await?;
    Ok(result.rows_affected())
}

pub(crate) async fn update_admin_metadata(
    db: &SqlitePool,
    params: UpdateRomMetadataParams<'_>,
) -> Result<Rom, sqlx::Error> {
    let mut tx = db.begin().await?;

    let result = sqlx::query(
        r#"
        UPDATE roms
        SET
            platform_id = ?,
            name = ?,
            slug = ?,
            summary = ?,
            regions_json = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(params.platform_id)
    .bind(params.name)
    .bind(params.slug)
    .bind(params.summary)
    .bind(params.regions_json)
    .bind(params.rom_id)
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(sqlx::Error::RowNotFound);
    }

    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
        VALUES (?, ?, ?, ?, NULL)
        ON CONFLICT(rom_id) DO UPDATE SET
            source = excluded.source,
            metadata_json = excluded.metadata_json,
            schema_version = excluded.schema_version,
            fetched_at = excluded.fetched_at,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(params.rom_id)
    .bind(params.metadata_source)
    .bind(params.metadata_json)
    .bind(params.schema_version)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    find_by_id(db, params.rom_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub(crate) async fn save_cover(
    db: &SqlitePool,
    params: SaveCoverParams<'_>,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
        VALUES (?, ?, ?, 1, NULL)
        ON CONFLICT(rom_id) DO UPDATE SET
            source = excluded.source,
            metadata_json = excluded.metadata_json,
            schema_version = excluded.schema_version,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(params.rom_id)
    .bind(params.metadata_source)
    .bind(params.metadata_json)
    .execute(&mut *tx)
    .await?;

    let result = sqlx::query(
        r#"
        UPDATE roms
        SET
            url_cover = NULL,
            path_cover_large = ?,
            path_cover_small = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(params.path)
    .bind(params.path)
    .bind(params.rom_id)
    .execute(&mut *tx)
    .await?;

    if result.rows_affected() == 0 {
        tx.rollback().await?;
        return Err(sqlx::Error::RowNotFound);
    }

    sqlx::query("DELETE FROM cover_assets WHERE rom_id = ?")
        .bind(params.rom_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        r#"
        INSERT INTO cover_assets (rom_id, resource_path, media_type, file_size_bytes)
        VALUES (?, ?, ?, ?)
        "#,
    )
    .bind(params.rom_id)
    .bind(params.path)
    .bind(params.media_type)
    .bind(params.file_size_bytes)
    .execute(&mut *tx)
    .await?;

    file_operations::set_state_tx(
        &mut tx,
        params.file_operation_id,
        FileOperationState::DbCommitted,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(crate) async fn save_igdb_metadata(
    db: &SqlitePool,
    params: SaveIgdbMetadataParams<'_>,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
        VALUES (?, 'igdb', ?, ?, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ON CONFLICT(rom_id) DO UPDATE SET
            source = excluded.source,
            metadata_json = excluded.metadata_json,
            schema_version = excluded.schema_version,
            fetched_at = excluded.fetched_at,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(params.rom_id)
    .bind(params.metadata_json)
    .bind(params.schema_version)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        UPDATE roms
        SET
            summary = COALESCE(?, summary),
            url_cover = COALESCE(?, url_cover),
            path_cover_large = COALESCE(?, path_cover_large),
            path_cover_small = COALESCE(?, path_cover_small),
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(params.summary)
    .bind(params.url_cover)
    .bind(params.path_cover_large)
    .bind(params.path_cover_small)
    .bind(params.rom_id)
    .execute(&mut *tx)
    .await?;

    if !params.cover_assets.is_empty() {
        sqlx::query("DELETE FROM cover_assets WHERE rom_id = ?")
            .bind(params.rom_id)
            .execute(&mut *tx)
            .await?;
    }

    for asset in params.cover_assets {
        sqlx::query(
            r#"
            INSERT INTO cover_assets (rom_id, resource_path, media_type, file_size_bytes)
            VALUES (?, ?, ?, ?)
            ON CONFLICT(resource_path) DO UPDATE SET
                rom_id = excluded.rom_id,
                media_type = excluded.media_type,
                file_size_bytes = excluded.file_size_bytes,
                updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind(params.rom_id)
        .bind(asset.resource_path)
        .bind(asset.media_type)
        .bind(asset.file_size_bytes)
        .execute(&mut *tx)
        .await?;
    }

    if let Some(operation_id) = params.file_operation_id {
        file_operations::set_state_tx(&mut tx, operation_id, FileOperationState::DbCommitted)
            .await?;
    }

    tx.commit().await?;
    Ok(())
}
