use std::collections::{BTreeSet, HashSet};

use crc32fast::Hasher as Crc32Hasher;
use md5::Md5;
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncReadExt;

use crate::{
    domain::integrity::{DatSource, FileHashes, IntegrityJob, NewDatSource, RomIntegrityReport},
    repositories::{integrity, roms},
    services::ingest::{
        dat::{self, DatParseError},
        filename,
    },
    state::AppState,
    storage::file_store::FileStoreError,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatImportOutcome {
    pub source: DatSource,
    pub imported_entries: usize,
    pub skipped_entries: usize,
    pub already_imported: bool,
}

#[derive(Debug, Error)]
pub enum IntegrityServiceError {
    #[error("ROM not found")]
    RomNotFound,

    #[error("integrity job not found")]
    JobNotFound,

    #[error("DAT filename is invalid")]
    InvalidDatFileName,

    #[error("DAT upload is too large")]
    DatTooLarge,

    #[error("DAT upload is not valid UTF-8")]
    InvalidDatEncoding,

    #[error(transparent)]
    DatParse(#[from] DatParseError),

    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub const MAX_DAT_BYTES: usize = 64 * 1024 * 1024;

pub async fn import_logiqx_dat(
    state: &AppState,
    imported_file_name: &str,
    bytes: &[u8],
) -> Result<DatImportOutcome, IntegrityServiceError> {
    if bytes.len() > MAX_DAT_BYTES {
        return Err(IntegrityServiceError::DatTooLarge);
    }
    let imported_file_name = safe_display_file_name(imported_file_name)?;
    let file_sha256 = format!("{:x}", Sha256::digest(bytes));
    if let Some(source) = integrity::find_dat_source_by_hash(state.db(), &file_sha256).await? {
        return Ok(DatImportOutcome {
            imported_entries: source.entry_count as usize,
            source,
            skipped_entries: 0,
            already_imported: true,
        });
    }

    let contents =
        std::str::from_utf8(bytes).map_err(|_| IntegrityServiceError::InvalidDatEncoding)?;
    let parsed = dat::parse_logiqx_xml(contents)?;
    let imported_entries = parsed.entries.len();
    let fallback_name = imported_file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem)
        .unwrap_or(&imported_file_name)
        .trim();
    let fallback_name = if fallback_name.is_empty() {
        "Imported DAT"
    } else {
        fallback_name
    };
    let source = integrity::import_dat(
        state.db(),
        NewDatSource {
            name: parsed.name.unwrap_or_else(|| fallback_name.to_string()),
            description: parsed.description,
            version: parsed.version,
            author: parsed.author,
            homepage: parsed.homepage,
            imported_file_name,
            file_sha256,
        },
        parsed.entries,
    )
    .await?;

    Ok(DatImportOutcome {
        source,
        imported_entries,
        skipped_entries: parsed.skipped_entries,
        already_imported: false,
    })
}

pub async fn list_dat_sources(state: &AppState) -> Result<Vec<DatSource>, IntegrityServiceError> {
    Ok(integrity::list_dat_sources(state.db()).await?)
}

pub async fn start_hash_job(
    state: AppState,
    rom_id: Option<i64>,
    force: bool,
) -> Result<IntegrityJob, IntegrityServiceError> {
    if let Some(rom_id) = rom_id
        && !integrity::rom_exists(state.db(), rom_id).await?
    {
        return Err(IntegrityServiceError::RomNotFound);
    }

    let claimed_job = match integrity::create_job(state.db(), rom_id, force).await? {
        integrity::IntegrityJobClaim::Existing(job) => return Ok(job),
        integrity::IntegrityJobClaim::Created(job) => job,
    };
    let job_id = claimed_job.id;
    let files = match integrity::files_for_job(state.db(), rom_id).await {
        Ok(files) => files,
        Err(error) => {
            if let Err(fail_error) =
                integrity::fail_job(state.db(), job_id, &error.to_string()).await
            {
                tracing::error!(?fail_error, job_id, "failed to release integrity job claim");
            }
            return Err(error.into());
        }
    };
    let total_files = i64::try_from(files.len()).unwrap_or(i64::MAX);
    let job = match integrity::set_job_total(state.db(), job_id, total_files).await {
        Ok(job) => job,
        Err(error) => {
            if let Err(fail_error) =
                integrity::fail_job(state.db(), job_id, &error.to_string()).await
            {
                tracing::error!(?fail_error, job_id, "failed to release integrity job claim");
            }
            return Err(error.into());
        }
    };

    tokio::spawn(async move {
        if let Err(error) = run_hash_job(&state, job_id, files, force).await {
            tracing::error!(?error, job_id, "integrity hash job failed");
            if let Err(database_error) =
                integrity::fail_job(state.db(), job_id, &error.to_string()).await
            {
                tracing::error!(
                    ?database_error,
                    job_id,
                    "failed to mark integrity job failed"
                );
            }
        }
    });

    Ok(job)
}

pub async fn list_jobs(state: &AppState) -> Result<Vec<IntegrityJob>, IntegrityServiceError> {
    Ok(integrity::list_jobs(state.db()).await?)
}

pub async fn find_job(state: &AppState, id: i64) -> Result<IntegrityJob, IntegrityServiceError> {
    integrity::find_job(state.db(), id)
        .await?
        .ok_or(IntegrityServiceError::JobNotFound)
}

pub async fn rom_report(
    state: &AppState,
    rom_id: i64,
) -> Result<RomIntegrityReport, IntegrityServiceError> {
    integrity::report_for_rom(state.db(), rom_id)
        .await?
        .ok_or(IntegrityServiceError::RomNotFound)
}

async fn run_hash_job(
    state: &AppState,
    job_id: i64,
    files: Vec<crate::domain::integrity::IntegrityFile>,
    force: bool,
) -> Result<(), IntegrityServiceError> {
    integrity::mark_job_running(state.db(), job_id).await?;
    let mut affected_roms = HashSet::new();

    for file in files {
        affected_roms.insert(file.rom_id);
        let needs_hash = force || !file.has_complete_hashes();
        let hashes = if needs_hash {
            integrity::mark_file_hashing(state.db(), file.id).await?;
            let opened_file = match state
                .file_store()
                .open_existing(&file.root_path, &file.relative_path)
                .await
            {
                Ok(file) => file,
                Err(error) => {
                    integrity::fail_file_hash(state.db(), file.id, &error.to_string()).await?;
                    integrity::clear_file_matches(state.db(), file.id).await?;
                    integrity::advance_job(state.db(), job_id, false, false, true).await?;
                    continue;
                }
            };
            match hash_file(opened_file).await {
                Ok(hashes) => {
                    integrity::save_file_hashes(state.db(), file.id, &hashes).await?;
                    hashes
                }
                Err(error) => {
                    integrity::fail_file_hash(state.db(), file.id, &error.to_string()).await?;
                    integrity::clear_file_matches(state.db(), file.id).await?;
                    integrity::advance_job(state.db(), job_id, false, false, true).await?;
                    continue;
                }
            }
        } else {
            FileHashes {
                crc32: file.crc32.expect("complete hash has CRC32"),
                md5: file.md5.expect("complete hash has MD5"),
                sha1: file.sha1.expect("complete hash has SHA-1"),
                sha256: file.sha256.expect("complete hash has SHA-256"),
            }
        };

        let matches =
            integrity::replace_file_matches(state.db(), file.id, file.file_size_bytes, &hashes)
                .await?;
        integrity::advance_job(state.db(), job_id, needs_hash, !matches.is_empty(), false).await?;
    }

    for rom_id in affected_roms {
        apply_authoritative_dat_metadata(state, rom_id).await?;
    }
    integrity::complete_job(state.db(), job_id).await?;
    Ok(())
}

async fn hash_file(mut file: tokio::fs::File) -> Result<FileHashes, std::io::Error> {
    let mut crc32 = Crc32Hasher::new();
    let mut md5 = Md5::new();
    let mut sha1 = Sha1::new();
    let mut sha256 = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];

    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let bytes = &buffer[..read];
        crc32.update(bytes);
        md5.update(bytes);
        sha1.update(bytes);
        sha256.update(bytes);
    }

    Ok(FileHashes {
        crc32: format!("{:08x}", crc32.finalize()),
        md5: format!("{:x}", md5.finalize()),
        sha1: format!("{:x}", sha1.finalize()),
        sha256: format!("{:x}", sha256.finalize()),
    })
}

async fn apply_authoritative_dat_metadata(
    state: &AppState,
    rom_id: i64,
) -> Result<(), IntegrityServiceError> {
    let matches = integrity::matches_for_rom(state.db(), rom_id).await?;
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(IntegrityServiceError::RomNotFound)?;
    let mut metadata = match rom.metadata {
        Value::Object(metadata) => metadata,
        _ => Map::new(),
    };
    metadata.insert("schema_version".into(), json!(1));

    if matches.is_empty() {
        metadata.insert(
            "integrity".into(),
            json!({
                "status": "unmatched",
                "authority": null,
                "filename_fallback": true,
                "verified_file_count": 0,
                "match_count": 0,
            }),
        );
        integrity::save_rom_integrity_metadata(
            state.db(),
            rom_id,
            &serde_json::to_string(&rom.regions)?,
            "integrity",
            &Value::Object(metadata).to_string(),
            false,
        )
        .await?;
        return Ok(());
    }

    if metadata
        .get("source")
        .and_then(Value::as_str)
        .is_none_or(|source| source == "filename")
    {
        metadata.insert("source".into(), Value::String("dat".into()));
    }
    if !metadata.contains_key("filename") {
        let file = rom
            .files
            .iter()
            .find(|file| matches.iter().any(|dat_match| dat_match.file_id == file.id))
            .or_else(|| rom.files.first());
        if let Some(file) = file {
            metadata.insert("filename".into(), json!(filename::parse(&file.file_name)));
        }
    }
    if let Some(filename) = metadata.get_mut("filename").and_then(Value::as_object_mut) {
        filename.insert("confidence".into(), Value::String("high".into()));
    }

    let mut regions = BTreeSet::new();
    let mut languages = BTreeSet::new();
    let mut serials = BTreeSet::new();
    let mut canonical_names = BTreeSet::new();
    for dat_match in &matches {
        regions.extend(dat_match.regions.iter().cloned());
        languages.extend(dat_match.languages.iter().cloned());
        if let Some(serial) = dat_match.serial.as_deref() {
            serials.insert(serial.to_string());
        }
        canonical_names.insert(
            dat_match
                .description
                .clone()
                .unwrap_or_else(|| dat_match.game_name.clone()),
        );
    }

    let regions: Vec<String> = regions.into_iter().collect();
    let languages: Vec<String> = languages.into_iter().collect();
    let serials: Vec<String> = serials.into_iter().collect();
    let canonical_names: Vec<String> = canonical_names.into_iter().collect();
    let verified_file_count = matches
        .iter()
        .map(|dat_match| dat_match.file_id)
        .collect::<HashSet<_>>()
        .len();
    metadata.insert(
        "integrity".into(),
        json!({
            "status": "verified",
            "authority": "dat",
            "filename_fallback": false,
            "verified_file_count": verified_file_count,
            "match_count": matches.len(),
            "canonical_names": canonical_names,
            "serials": serials,
            "regions": regions,
            "languages": languages,
        }),
    );
    if !regions.is_empty() {
        metadata.insert("regions".into(), json!(regions));
    }
    if !languages.is_empty() {
        metadata.insert("languages".into(), json!(languages));
    }
    if !serials.is_empty() {
        metadata.insert("serials".into(), json!(serials));
    }

    let effective_regions = if regions.is_empty() {
        rom.regions
    } else {
        regions.clone()
    };
    integrity::save_rom_integrity_metadata(
        state.db(),
        rom_id,
        &serde_json::to_string(&effective_regions)?,
        "dat",
        &Value::Object(metadata).to_string(),
        !regions.is_empty(),
    )
    .await?;
    Ok(())
}

fn safe_display_file_name(file_name: &str) -> Result<String, IntegrityServiceError> {
    let file_name = file_name.trim();
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.contains(['/', '\\', '\0'])
        || file_name.chars().any(char::is_control)
    {
        return Err(IntegrityServiceError::InvalidDatFileName);
    }
    Ok(file_name.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::hash_file;

    #[tokio::test]
    async fn computes_all_dat_hash_algorithms() {
        let mut file = NamedTempFile::new().unwrap();
        file.write_all(b"test").unwrap();
        let file = tokio::fs::File::open(file.path()).await.unwrap();
        let hashes = hash_file(file).await.unwrap();

        assert_eq!(hashes.crc32, "d87f7e0c");
        assert_eq!(hashes.md5, "098f6bcd4621d373cade4e832627b4f6");
        assert_eq!(hashes.sha1, "a94a8fe5ccb19ba61c4c0873d391e987982fbbd3");
        assert_eq!(
            hashes.sha256,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }
}
