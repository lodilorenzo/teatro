//! Persistence boundary for DAT catalogs, hash jobs, file hashes, and match reports.
//!
//! Persisted JSON and workflow strings are decoded fallibly before entering services.

use std::{collections::HashMap, path::PathBuf};

use sqlx::{FromRow, SqlitePool};

use crate::domain::{
    integrity::{
        DatMatch, DatSource, FileHashes, FileIntegrityReport, IntegrityFile, IntegrityHashes,
        IntegrityJob, NewDatEntry, NewDatSource, RomIntegrityReport,
    },
    workflow::{FileHashStatus, InvalidWorkflowValue, RomIntegrityStatus},
};

pub async fn find_dat_source_by_hash(
    db: &SqlitePool,
    file_sha256: &str,
) -> Result<Option<DatSource>, sqlx::Error> {
    let row = sqlx::query_as::<_, DatSourceRow>(
        r#"
        SELECT id, name, description, version, author, homepage, imported_file_name,
               file_sha256, entry_count, imported_at
        FROM dat_sources
        WHERE file_sha256 = ?
        "#,
    )
    .bind(file_sha256)
    .fetch_optional(db)
    .await?;

    Ok(row.map(DatSourceRow::into))
}

pub async fn import_dat(
    db: &SqlitePool,
    source: NewDatSource,
    entries: Vec<NewDatEntry>,
) -> Result<DatSource, sqlx::Error> {
    let mut tx = db.begin().await?;
    let entry_count = i64::try_from(entries.len()).unwrap_or(i64::MAX);
    let source_id = sqlx::query(
        r#"
        INSERT INTO dat_sources (
            name, description, version, author, homepage, imported_file_name,
            file_sha256, entry_count
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&source.name)
    .bind(source.description.as_deref())
    .bind(source.version.as_deref())
    .bind(source.author.as_deref())
    .bind(source.homepage.as_deref())
    .bind(&source.imported_file_name)
    .bind(&source.file_sha256)
    .bind(entry_count)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    for entry in entries {
        let regions_json = serialize_json("DAT entry regions", &entry.regions)?;
        let languages_json = serialize_json("DAT entry languages", &entry.languages)?;
        sqlx::query(
            r#"
            INSERT INTO dat_entries (
                source_id, game_name, description, rom_name, file_size_bytes,
                crc32, md5, sha1, sha256, serial, regions_json, languages_json,
                metadata_json
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(source_id)
        .bind(&entry.game_name)
        .bind(entry.description.as_deref())
        .bind(&entry.rom_name)
        .bind(entry.file_size_bytes)
        .bind(entry.crc32.as_deref())
        .bind(entry.md5.as_deref())
        .bind(entry.sha1.as_deref())
        .bind(entry.sha256.as_deref())
        .bind(entry.serial.as_deref())
        .bind(regions_json)
        .bind(languages_json)
        .bind(entry.metadata.to_string())
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    find_dat_source_by_id(db, source_id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn list_dat_sources(db: &SqlitePool) -> Result<Vec<DatSource>, sqlx::Error> {
    let rows = sqlx::query_as::<_, DatSourceRow>(
        r#"
        SELECT id, name, description, version, author, homepage, imported_file_name,
               file_sha256, entry_count, imported_at
        FROM dat_sources
        ORDER BY imported_at DESC, id DESC
        "#,
    )
    .fetch_all(db)
    .await?;

    Ok(rows.into_iter().map(DatSourceRow::into).collect())
}

async fn find_dat_source_by_id(db: &SqlitePool, id: i64) -> Result<Option<DatSource>, sqlx::Error> {
    let row = sqlx::query_as::<_, DatSourceRow>(
        r#"
        SELECT id, name, description, version, author, homepage, imported_file_name,
               file_sha256, entry_count, imported_at
        FROM dat_sources
        WHERE id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;

    Ok(row.map(DatSourceRow::into))
}

pub async fn rom_exists(db: &SqlitePool, rom_id: i64) -> Result<bool, sqlx::Error> {
    let exists: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM roms WHERE id = ?)")
        .bind(rom_id)
        .fetch_one(db)
        .await?;
    Ok(exists != 0)
}

pub async fn files_for_job(
    db: &SqlitePool,
    rom_id: Option<i64>,
) -> Result<Vec<IntegrityFile>, sqlx::Error> {
    let rows = sqlx::query_as::<_, IntegrityFileRow>(
        r#"
        SELECT rf.id, rf.rom_id, lr.root_path, rf.relative_path, rf.file_name,
               rf.file_size_bytes, rf.crc32, rf.md5, rf.sha1, rf.sha256,
               rf.hash_status
        FROM rom_files rf
        JOIN library_roots lr ON lr.id = rf.root_id
        WHERE (? IS NULL OR rf.rom_id = ?)
        ORDER BY rf.rom_id, rf.sort_index, rf.id
        "#,
    )
    .bind(rom_id)
    .bind(rom_id)
    .fetch_all(db)
    .await?;

    rows.into_iter().map(IntegrityFile::try_from).collect()
}

pub enum IntegrityJobClaim {
    Created(IntegrityJob),
    Existing(IntegrityJob),
}

pub async fn create_job(
    db: &SqlitePool,
    rom_id: Option<i64>,
    force: bool,
) -> Result<IntegrityJobClaim, sqlx::Error> {
    // The partial unique index on active jobs is the cross-process admission
    // control. INSERT OR IGNORE turns simultaneous starts into an idempotent
    // reference to the winner instead of a queued full-library scan.
    for _ in 0..3 {
        let result = sqlx::query(
            r#"
            INSERT OR IGNORE INTO integrity_jobs (rom_id, force, total_files)
            VALUES (?, ?, ?)
            "#,
        )
        .bind(rom_id)
        .bind(if force { 1_i64 } else { 0_i64 })
        .bind(0_i64)
        .execute(db)
        .await?;

        if result.rows_affected() == 1 {
            let job = find_job(db, result.last_insert_rowid())
                .await?
                .ok_or(sqlx::Error::RowNotFound)?;
            return Ok(IntegrityJobClaim::Created(job));
        }

        if let Some(job) = find_active_job(db).await? {
            return Ok(IntegrityJobClaim::Existing(job));
        }
        // The winning job may have completed between our ignored insert and
        // lookup. Retry admission rather than reporting a phantom conflict.
    }

    Err(sqlx::Error::RowNotFound)
}

pub async fn set_job_total(
    db: &SqlitePool,
    id: i64,
    total_files: i64,
) -> Result<IntegrityJob, sqlx::Error> {
    let result = sqlx::query("UPDATE integrity_jobs SET total_files = ? WHERE id = ?")
        .bind(total_files)
        .bind(id)
        .execute(db)
        .await?;
    if result.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    find_job(db, id).await?.ok_or(sqlx::Error::RowNotFound)
}

async fn find_active_job(db: &SqlitePool) -> Result<Option<IntegrityJob>, sqlx::Error> {
    let row = sqlx::query_as::<_, IntegrityJobRow>(
        r#"
        SELECT id, kind, status, rom_id, force, total_files, processed_files,
               hashed_files, matched_files, error_count, error, created_at,
               started_at, completed_at
        FROM integrity_jobs
        WHERE status IN ('queued', 'running')
        ORDER BY id
        LIMIT 1
        "#,
    )
    .fetch_optional(db)
    .await?;

    row.map(IntegrityJob::try_from).transpose()
}

pub async fn list_jobs(db: &SqlitePool) -> Result<Vec<IntegrityJob>, sqlx::Error> {
    let rows = sqlx::query_as::<_, IntegrityJobRow>(
        r#"
        SELECT id, kind, status, rom_id, force, total_files, processed_files,
               hashed_files, matched_files, error_count, error, created_at,
               started_at, completed_at
        FROM integrity_jobs
        ORDER BY id DESC
        LIMIT 100
        "#,
    )
    .fetch_all(db)
    .await?;

    rows.into_iter().map(IntegrityJob::try_from).collect()
}

pub async fn find_job(db: &SqlitePool, id: i64) -> Result<Option<IntegrityJob>, sqlx::Error> {
    let row = sqlx::query_as::<_, IntegrityJobRow>(
        r#"
        SELECT id, kind, status, rom_id, force, total_files, processed_files,
               hashed_files, matched_files, error_count, error, created_at,
               started_at, completed_at
        FROM integrity_jobs
        WHERE id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(db)
    .await?;

    row.map(IntegrityJob::try_from).transpose()
}

pub async fn mark_job_running(db: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrity_jobs
        SET status = 'running', started_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn advance_job(
    db: &SqlitePool,
    id: i64,
    hashed: bool,
    matched: bool,
    failed: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrity_jobs
        SET processed_files = processed_files + 1,
            hashed_files = hashed_files + ?,
            matched_files = matched_files + ?,
            error_count = error_count + ?
        WHERE id = ?
        "#,
    )
    .bind(if hashed { 1_i64 } else { 0_i64 })
    .bind(if matched { 1_i64 } else { 0_i64 })
    .bind(if failed { 1_i64 } else { 0_i64 })
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn complete_job(db: &SqlitePool, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrity_jobs
        SET status = 'completed', completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn fail_job(db: &SqlitePool, id: i64, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrity_jobs
        SET status = 'failed', error = ?,
            completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(error)
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn fail_interrupted_jobs(db: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE integrity_jobs
        SET status = 'failed', error = 'Teatro restarted before the job completed',
            completed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE status IN ('queued', 'running')
        "#,
    )
    .execute(db)
    .await?;
    Ok(())
}

pub async fn mark_file_hashing(db: &SqlitePool, file_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE rom_files SET hash_status = 'hashing', hash_error = NULL WHERE id = ?")
        .bind(file_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn save_file_hashes(
    db: &SqlitePool,
    file_id: i64,
    hashes: &FileHashes,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rom_files
        SET crc32 = ?, md5 = ?, sha1 = ?, sha256 = ?, hash_status = 'complete',
            hashed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), hash_error = NULL,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(&hashes.crc32)
    .bind(&hashes.md5)
    .bind(&hashes.sha1)
    .bind(&hashes.sha256)
    .bind(file_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn fail_file_hash(db: &SqlitePool, file_id: i64, error: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE rom_files
        SET hash_status = 'failed', hash_error = ?,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = ?
        "#,
    )
    .bind(error)
    .bind(file_id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn clear_file_matches(db: &SqlitePool, file_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM rom_file_dat_matches WHERE file_id = ?")
        .bind(file_id)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn replace_file_matches(
    db: &SqlitePool,
    file_id: i64,
    file_size_bytes: i64,
    hashes: &FileHashes,
) -> Result<Vec<DatMatch>, sqlx::Error> {
    let mut tx = db.begin().await?;
    sqlx::query("DELETE FROM rom_file_dat_matches WHERE file_id = ?")
        .bind(file_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        r#"
        INSERT INTO rom_file_dat_matches (file_id, dat_entry_id, matched_by)
        SELECT ?, de.id,
            CASE
                WHEN de.sha256 IS NOT NULL THEN 'sha256'
                WHEN de.sha1 IS NOT NULL THEN 'sha1'
                WHEN de.md5 IS NOT NULL THEN 'md5'
                ELSE 'crc32'
            END
        FROM dat_entries de
        WHERE (de.file_size_bytes IS NULL OR de.file_size_bytes = ?)
          AND (de.sha256 IS NULL OR de.sha256 = ?)
          AND (de.sha1 IS NULL OR de.sha1 = ?)
          AND (de.md5 IS NULL OR de.md5 = ?)
          AND (de.crc32 IS NULL OR de.crc32 = ?)
          AND (de.sha256 IS NOT NULL OR de.sha1 IS NOT NULL OR de.md5 IS NOT NULL OR de.crc32 IS NOT NULL)
        "#,
    )
    .bind(file_id)
    .bind(file_size_bytes)
    .bind(&hashes.sha256)
    .bind(&hashes.sha1)
    .bind(&hashes.md5)
    .bind(&hashes.crc32)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    matches_for_file(db, file_id).await
}

pub async fn matches_for_file(db: &SqlitePool, file_id: i64) -> Result<Vec<DatMatch>, sqlx::Error> {
    let rows = sqlx::query_as::<_, DatMatchRow>(
        &format!("{} WHERE m.file_id = ? ORDER BY ds.name COLLATE NOCASE, de.game_name COLLATE NOCASE, de.id", dat_match_select()),
    )
    .bind(file_id)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(DatMatch::try_from).collect()
}

pub async fn matches_for_rom(db: &SqlitePool, rom_id: i64) -> Result<Vec<DatMatch>, sqlx::Error> {
    let rows = sqlx::query_as::<_, DatMatchRow>(
        &format!("{} JOIN rom_files rf ON rf.id = m.file_id WHERE rf.rom_id = ? ORDER BY rf.sort_index, rf.id, ds.name COLLATE NOCASE, de.id", dat_match_select()),
    )
    .bind(rom_id)
    .fetch_all(db)
    .await?;
    rows.into_iter().map(DatMatch::try_from).collect()
}

pub async fn save_rom_integrity_metadata(
    db: &SqlitePool,
    rom_id: i64,
    regions_json: &str,
    metadata_source: &str,
    metadata_json: &str,
    has_authoritative_regions: bool,
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;
    if has_authoritative_regions {
        sqlx::query(
            r#"
            UPDATE roms
            SET regions_json = ?, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE id = ?
            "#,
        )
        .bind(regions_json)
        .bind(rom_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query(
        r#"
        INSERT INTO rom_metadata (rom_id, source, metadata_json, schema_version, fetched_at)
        VALUES (?, ?, ?, 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
        ON CONFLICT(rom_id) DO UPDATE SET
            source = CASE
                WHEN rom_metadata.source IN ('filename', 'integrity', 'dat') THEN excluded.source
                ELSE rom_metadata.source
            END,
            metadata_json = excluded.metadata_json,
            schema_version = MAX(rom_metadata.schema_version, excluded.schema_version),
            fetched_at = excluded.fetched_at,
            updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        "#,
    )
    .bind(rom_id)
    .bind(metadata_source)
    .bind(metadata_json)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn report_for_rom(
    db: &SqlitePool,
    rom_id: i64,
) -> Result<Option<RomIntegrityReport>, sqlx::Error> {
    if !rom_exists(db, rom_id).await? {
        return Ok(None);
    }

    let rows = sqlx::query_as::<_, IntegrityReportFileRow>(
        r#"
        SELECT id, file_name, file_size_bytes, hash_status, crc32, md5, sha1,
               sha256, hashed_at, hash_error
        FROM rom_files
        WHERE rom_id = ?
        ORDER BY sort_index, id
        "#,
    )
    .bind(rom_id)
    .fetch_all(db)
    .await?;

    let all_matches = matches_for_rom(db, rom_id).await?;
    let mut matches_by_file: HashMap<i64, Vec<DatMatch>> = HashMap::new();
    for dat_match in all_matches {
        matches_by_file
            .entry(dat_match.file_id)
            .or_default()
            .push(dat_match);
    }

    let files: Vec<FileIntegrityReport> = rows
        .into_iter()
        .map(|row| {
            Ok(FileIntegrityReport {
                file_id: row.id,
                file_name: row.file_name,
                file_size_bytes: row.file_size_bytes,
                hash_status: decode_workflow_value(row.hash_status.parse())?,
                hashes: IntegrityHashes {
                    crc32: row.crc32,
                    md5: row.md5,
                    sha1: row.sha1,
                    sha256: row.sha256,
                },
                hashed_at: row.hashed_at,
                hash_error: row.hash_error,
                matches: matches_by_file.remove(&row.id).unwrap_or_default(),
            })
        })
        .collect::<Result<_, sqlx::Error>>()?;

    let status = if files.iter().any(|file| !file.matches.is_empty()) {
        RomIntegrityStatus::Verified
    } else if files
        .iter()
        .any(|file| file.hash_status == FileHashStatus::Failed)
    {
        RomIntegrityStatus::Failed
    } else if !files.is_empty()
        && files
            .iter()
            .all(|file| file.hash_status == FileHashStatus::Complete)
    {
        RomIntegrityStatus::Unmatched
    } else if files
        .iter()
        .any(|file| file.hash_status == FileHashStatus::Hashing)
    {
        RomIntegrityStatus::Hashing
    } else {
        RomIntegrityStatus::Pending
    };

    Ok(Some(RomIntegrityReport {
        rom_id,
        status,
        files,
    }))
}

fn dat_match_select() -> &'static str {
    r#"
    SELECT m.file_id, m.dat_entry_id, de.source_id, ds.name AS source_name,
           ds.version AS source_version, de.game_name, de.description, de.rom_name,
           m.matched_by, de.serial, de.regions_json, de.languages_json, m.verified_at
    FROM rom_file_dat_matches m
    JOIN dat_entries de ON de.id = m.dat_entry_id
    JOIN dat_sources ds ON ds.id = de.source_id
    "#
}

#[derive(Debug, FromRow)]
struct DatSourceRow {
    id: i64,
    name: String,
    description: Option<String>,
    version: Option<String>,
    author: Option<String>,
    homepage: Option<String>,
    imported_file_name: String,
    file_sha256: String,
    entry_count: i64,
    imported_at: String,
}

impl From<DatSourceRow> for DatSource {
    fn from(row: DatSourceRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            description: row.description,
            version: row.version,
            author: row.author,
            homepage: row.homepage,
            imported_file_name: row.imported_file_name,
            file_sha256: row.file_sha256,
            entry_count: row.entry_count,
            imported_at: row.imported_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct IntegrityFileRow {
    id: i64,
    rom_id: i64,
    root_path: String,
    relative_path: String,
    file_name: String,
    file_size_bytes: i64,
    crc32: Option<String>,
    md5: Option<String>,
    sha1: Option<String>,
    sha256: Option<String>,
    hash_status: String,
}

impl TryFrom<IntegrityFileRow> for IntegrityFile {
    type Error = sqlx::Error;

    fn try_from(row: IntegrityFileRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            rom_id: row.rom_id,
            root_path: PathBuf::from(row.root_path),
            relative_path: row.relative_path,
            file_name: row.file_name,
            file_size_bytes: row.file_size_bytes,
            crc32: row.crc32,
            md5: row.md5,
            sha1: row.sha1,
            sha256: row.sha256,
            hash_status: decode_workflow_value(row.hash_status.parse())?,
        })
    }
}

#[derive(Debug, FromRow)]
struct IntegrityJobRow {
    id: i64,
    kind: String,
    status: String,
    rom_id: Option<i64>,
    force: i64,
    total_files: i64,
    processed_files: i64,
    hashed_files: i64,
    matched_files: i64,
    error_count: i64,
    error: Option<String>,
    created_at: String,
    started_at: Option<String>,
    completed_at: Option<String>,
}

impl TryFrom<IntegrityJobRow> for IntegrityJob {
    type Error = sqlx::Error;

    fn try_from(row: IntegrityJobRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            kind: decode_workflow_value(row.kind.parse())?,
            status: decode_workflow_value(row.status.parse())?,
            rom_id: row.rom_id,
            force: row.force != 0,
            total_files: row.total_files,
            processed_files: row.processed_files,
            hashed_files: row.hashed_files,
            matched_files: row.matched_files,
            error_count: row.error_count,
            error: row.error,
            created_at: row.created_at,
            started_at: row.started_at,
            completed_at: row.completed_at,
        })
    }
}

#[derive(Debug, FromRow)]
struct DatMatchRow {
    file_id: i64,
    dat_entry_id: i64,
    source_id: i64,
    source_name: String,
    source_version: Option<String>,
    game_name: String,
    description: Option<String>,
    rom_name: String,
    matched_by: String,
    serial: Option<String>,
    regions_json: String,
    languages_json: String,
    verified_at: String,
}

impl TryFrom<DatMatchRow> for DatMatch {
    type Error = sqlx::Error;

    fn try_from(row: DatMatchRow) -> Result<Self, Self::Error> {
        Ok(Self {
            file_id: row.file_id,
            dat_entry_id: row.dat_entry_id,
            source_id: row.source_id,
            source_name: row.source_name,
            source_version: row.source_version,
            game_name: row.game_name,
            description: row.description,
            rom_name: row.rom_name,
            matched_by: decode_workflow_value(row.matched_by.parse())?,
            serial: row.serial,
            regions: decode_dat_match_json("dat_entries.regions_json", &row.regions_json)?,
            languages: decode_dat_match_json("dat_entries.languages_json", &row.languages_json)?,
            verified_at: row.verified_at,
        })
    }
}

fn decode_dat_match_json<T: serde::de::DeserializeOwned>(
    column: &str,
    value: &str,
) -> Result<T, sqlx::Error> {
    serde_json::from_str(value).map_err(|error| {
        sqlx::Error::Decode(format!("invalid JSON persisted in {column}: {error}").into())
    })
}

fn decode_workflow_value<T>(value: Result<T, InvalidWorkflowValue>) -> Result<T, sqlx::Error> {
    value.map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

fn serialize_json<T: serde::Serialize>(context: &str, value: &T) -> Result<String, sqlx::Error> {
    serde_json::to_string(value).map_err(|error| {
        sqlx::Error::Encode(format!("failed to serialize {context}: {error}").into())
    })
}

#[derive(Debug, FromRow)]
struct IntegrityReportFileRow {
    id: i64,
    file_name: String,
    file_size_bytes: i64,
    hash_status: String,
    crc32: Option<String>,
    md5: Option<String>,
    sha1: Option<String>,
    sha256: Option<String>,
    hashed_at: Option<String>,
    hash_error: Option<String>,
}
