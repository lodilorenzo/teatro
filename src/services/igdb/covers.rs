use std::{collections::BTreeSet, path::Path};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    domain::{
        rom::Rom,
        workflow::{FileOperationKind, FileOperationState},
    },
    repositories::{file_operations as file_operation_repository, roms},
    services::file_operations::{self, CoverOperationEntry, CoverOperationPayload},
    state::AppState,
    storage::{
        file_store::FileStoreError,
        paths::{self, PathSafetyError},
    },
};

use super::models::{
    CachedCoverAsset, DownloadedCover, IgdbServiceError, MAX_COVER_BYTES, PreparedCoverAssets,
};

pub(super) fn validate_cover_image_id(image_id: &str) -> Result<(), IgdbServiceError> {
    let valid = !image_id.is_empty()
        && image_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        });

    if valid {
        Ok(())
    } else {
        Err(IgdbServiceError::InvalidCoverImageId)
    }
}

pub(crate) async fn replace_uploaded_cover(
    state: &AppState,
    rom_id: i64,
    bytes: Vec<u8>,
) -> Result<Rom, IgdbServiceError> {
    replace_cover_bytes(state, rom_id, bytes, "manual").await
}

/// Stores one caller-supplied image as a ROM's cover.
///
/// `cover_source` records where the image came from, so a cover fetched during a RomM import is
/// distinguishable from one an admin uploaded by hand. Whatever the source, the bytes are bounded
/// and their format is decided by magic bytes rather than by any declared content type.
pub(crate) async fn replace_cover_bytes(
    state: &AppState,
    rom_id: i64,
    bytes: Vec<u8>,
    cover_source: &str,
) -> Result<Rom, IgdbServiceError> {
    if bytes.len() as u64 > MAX_COVER_BYTES {
        return Err(IgdbServiceError::CoverTooLarge);
    }
    let (media_type, extension) =
        uploaded_cover_format(&bytes).ok_or(IgdbServiceError::InvalidCoverImage)?;
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(IgdbServiceError::RomNotFound)?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let resource_path = cover_resource_path(&rom, &format!("cover-{digest}.{extension}"));
    let asset_kind = if cover_source == "manual" {
        "uploaded".to_string()
    } else {
        cover_source.to_string()
    };
    let file_size_bytes = bytes.len() as i64;
    let prepared = prepare_cover_assets(
        state,
        &rom,
        vec![DownloadedCover {
            asset: CachedCoverAsset {
                kind: asset_kind,
                resource_path: resource_path.clone(),
                media_type: media_type.to_string(),
                file_size_bytes,
            },
            bytes,
        }],
    )
    .await?;

    let mut metadata = rom.metadata.as_object().cloned().unwrap_or_default();
    let metadata_source = metadata
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .unwrap_or("manual")
        .to_string();
    metadata.insert("source".to_string(), json!(&metadata_source));
    metadata.insert("schema_version".to_string(), json!(1));
    metadata.insert("cover_source".to_string(), json!(cover_source));
    metadata.insert("cover_image_id".to_string(), Value::Null);
    metadata.insert("cover_url".to_string(), Value::Null);
    metadata.insert("path_cover_large".to_string(), json!(&resource_path));
    metadata.insert("path_cover_small".to_string(), json!(&resource_path));
    let metadata_json = Value::Object(metadata).to_string();

    let save_result = roms::save_cover(
        state.db(),
        roms::SaveCoverParams {
            rom_id,
            metadata_source: &metadata_source,
            metadata_json: &metadata_json,
            path: &resource_path,
            media_type,
            file_size_bytes,
            file_operation_id: prepared
                .operation_id
                .as_deref()
                .expect("prepared cover has an operation id"),
        },
    )
    .await;

    match save_result {
        Ok(()) => {
            commit_prepared_covers(state, &prepared).await?;
            roms::find_by_id(state.db(), rom_id)
                .await?
                .ok_or(IgdbServiceError::RomNotFound)
        }
        Err(error) => {
            rollback_prepared_covers(state, &prepared).await;
            Err(error.into())
        }
    }
}

fn uploaded_cover_format(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\xff\xd8\xff") {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

pub(super) async fn prepare_cover_assets(
    state: &AppState,
    rom: &Rom,
    downloads: Vec<DownloadedCover>,
) -> Result<PreparedCoverAssets, IgdbServiceError> {
    let root = state.config().asset_root.canonicalize()?;
    let root_lock = state.file_store().lock_root(&root).await?;
    file_operations::reconcile_pending_covers_locked(state, &root_lock)
        .await
        .map_err(|error| IgdbServiceError::CoverRecovery(error.to_string()))?;
    let operation_id = file_operations::new_operation_id();
    let new_resource_paths: BTreeSet<_> = downloads
        .iter()
        .map(|download| download.asset.resource_path.clone())
        .collect();

    let mut replacement_entries = Vec::with_capacity(downloads.len());
    for (index, download) in downloads.iter().enumerate() {
        let relative = paths::clean_relative_path(&download.asset.resource_path)?;
        let parent = relative.parent().ok_or(PathSafetyError::Empty)?;
        state.file_store().create_dir_all(&root, parent).await?;
        let backup_relative_path = state
            .file_store()
            .exists(&root, &download.asset.resource_path)
            .await?
            .then(|| format!(".trash/{operation_id}/cover-{index}"));
        replacement_entries.push(CoverOperationEntry {
            resource_path: download.asset.resource_path.clone(),
            backup_relative_path,
        });
    }

    let previous_paths: Vec<String> = [rom.path_cover_large.clone(), rom.path_cover_small.clone()]
        .into_iter()
        .flatten()
        .collect();
    let mut previous_managed_paths: BTreeSet<String> = previous_paths.iter().cloned().collect();
    previous_managed_paths.extend(
        roms::cover_asset_paths_for_rom_ids(state.db(), &[rom.id])
            .await?
            .remove(&rom.id)
            .unwrap_or_default(),
    );

    let mut obsolete_entries = Vec::new();
    for resource_path in previous_managed_paths
        .into_iter()
        .filter(|path| !new_resource_paths.contains(path))
    {
        paths::clean_relative_path(&resource_path)?;
        if state.file_store().exists(&root, &resource_path).await? {
            let index = obsolete_entries.len();
            obsolete_entries.push(CoverOperationEntry {
                resource_path,
                backup_relative_path: Some(format!(".trash/{operation_id}/obsolete-cover-{index}")),
            });
        }
    }

    let payload = CoverOperationPayload::new(rom.id, previous_paths, replacement_entries)
        .with_obsolete_entries(obsolete_entries);
    let payload_json =
        serde_json::to_string(&payload).map_err(|source| IgdbServiceError::Json {
            service: "cover operation journal",
            source,
        })?;
    file_operation_repository::create(
        state.db(),
        &operation_id,
        FileOperationKind::CoverReplace,
        &payload_json,
    )
    .await?;

    for entry in &payload.obsolete_entries {
        let backup = entry
            .backup_relative_path
            .as_deref()
            .expect("obsolete cover entries always have a backup path");
        if let Err(error) = state
            .file_store()
            .move_managed(&root, &entry.resource_path, backup)
            .await
        {
            rollback_and_record_cover_failure(
                state,
                &root,
                &operation_id,
                &payload,
                &error.to_string(),
            )
            .await;
            return Err(error.into());
        }
    }

    for (download, entry) in downloads.iter().zip(&payload.entries) {
        if let Some(backup) = entry.backup_relative_path.as_deref()
            && let Err(error) = state
                .file_store()
                .move_managed(&root, &entry.resource_path, backup)
                .await
        {
            rollback_and_record_cover_failure(
                state,
                &root,
                &operation_id,
                &payload,
                &error.to_string(),
            )
            .await;
            return Err(error.into());
        }
        if let Err(error) = state
            .file_store()
            .write_new_atomic(&root, &entry.resource_path, &download.bytes)
            .await
        {
            rollback_and_record_cover_failure(
                state,
                &root,
                &operation_id,
                &payload,
                &error.to_string(),
            )
            .await;
            return Err(error.into());
        }
    }

    Ok(PreparedCoverAssets {
        assets: downloads
            .into_iter()
            .map(|download| download.asset)
            .collect(),
        operation_id: Some(operation_id),
        payload: Some(payload),
        _root_lock: Some(root_lock),
    })
}

async fn rollback_and_record_cover_failure(
    state: &AppState,
    root: &Path,
    operation_id: &str,
    payload: &CoverOperationPayload,
    original_error: &str,
) {
    match file_operations::rollback_cover_files(state, root, payload).await {
        Ok(()) => {
            if let Err(error) = file_operations::complete(state, operation_id).await {
                tracing::error!(
                    ?error,
                    operation_id,
                    "cover files were restored but the journal could not be completed"
                );
            }
        }
        Err(rollback_error) => {
            file_operations::fail(state, operation_id, &rollback_error.to_string()).await;
            tracing::error!(
                ?rollback_error,
                operation_id,
                original_error,
                "failed to restore a partially prepared cover operation"
            );
        }
    }
}

pub(super) async fn commit_prepared_covers(
    state: &AppState,
    prepared: &PreparedCoverAssets,
) -> Result<(), IgdbServiceError> {
    let (Some(operation_id), Some(payload)) =
        (prepared.operation_id.as_deref(), prepared.payload.as_ref())
    else {
        return Ok(());
    };
    debug_assert!(prepared._root_lock.is_some());

    let root = state.config().asset_root.canonicalize()?;
    if let Err(error) = file_operations::cleanup_cover_files(state, &root, payload, true).await {
        file_operation_repository::set_state(
            state.db(),
            operation_id,
            FileOperationState::CleanupPending,
            Some(&error.to_string()),
        )
        .await?;
        tracing::warn!(?error, operation_id, "cover backup cleanup remains pending");
    } else {
        file_operations::complete(state, operation_id).await?;
    }
    Ok(())
}

pub(super) async fn rollback_prepared_covers(state: &AppState, prepared: &PreparedCoverAssets) {
    let (Some(operation_id), Some(payload)) =
        (prepared.operation_id.as_deref(), prepared.payload.as_ref())
    else {
        return;
    };
    debug_assert!(prepared._root_lock.is_some());

    let result = async {
        let root = state.config().asset_root.canonicalize()?;
        file_operations::rollback_cover_files(state, &root, payload).await?;
        Ok::<_, FileStoreError>(())
    }
    .await;
    match result {
        Ok(()) => {
            if let Err(error) = file_operations::complete(state, operation_id).await {
                tracing::error!(
                    ?error,
                    operation_id,
                    "cover rollback succeeded but the journal could not be completed"
                );
            }
        }
        Err(error) => {
            file_operations::fail(state, operation_id, &error.to_string()).await;
            tracing::error!(?error, operation_id, "failed to roll back cover operation");
        }
    }
}

pub(super) fn cover_resource_path(rom: &Rom, file_name: &str) -> String {
    format!(
        "library/{}/{}/{}",
        safe_resource_segment(&rom.platform_slug),
        safe_resource_segment(&format!("{}-{}", rom.id, rom.slug)),
        file_name
    )
}

pub(super) fn safe_resource_segment(value: &str) -> String {
    let mut output = String::new();
    let mut previous_dash = false;

    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            output.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            output.push('-');
            previous_dash = true;
        }
    }

    let output = output.trim_matches('-').to_string();
    if output.is_empty() {
        "resource".to_string()
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{cover_resource_path, prepare_cover_assets, uploaded_cover_format};
    use crate::{
        config::{
            AppConfig, AuthConfig, DownloadArchiveConfig, GogImportConfig, IgdbConfig, LogFormat,
            RommSourceConfig, UploadConfig,
        },
        domain::workflow::FileOperationKind,
        repositories::{file_operations as file_operation_repository, platforms, roms},
        services::igdb::models::{CachedCoverAsset, DownloadedCover},
        state::AppState,
    };

    #[test]
    fn uploaded_covers_accept_only_supported_image_signatures() {
        assert_eq!(
            uploaded_cover_format(b"\xff\xd8\xffjpeg"),
            Some(("image/jpeg", "jpg"))
        );
        assert_eq!(
            uploaded_cover_format(b"\x89PNG\r\n\x1a\npng"),
            Some(("image/png", "png"))
        );
        assert_eq!(
            uploaded_cover_format(b"RIFF\x04\x00\x00\x00WEBPdata"),
            Some(("image/webp", "webp"))
        );
        assert_eq!(uploaded_cover_format(b"<svg onload=alert(1)>"), None);
    }

    #[tokio::test]
    async fn second_cover_write_failure_rolls_back_the_first_cover() {
        let temp = TempDir::new().unwrap();
        let config = AppConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            database_url: format!("sqlite://{}", temp.path().join("teatro.sqlite3").display()),
            data_dir: temp.path().join("data"),
            default_library_root: temp.path().join("roms"),
            asset_root: temp.path().join("assets"),
            max_upload_bytes: 1024,
            uploads: UploadConfig::default(),
            download_archives: DownloadArchiveConfig::default(),
            gog_import: GogImportConfig::default(),
            romm_source: RommSourceConfig::default(),
            log_format: LogFormat::Compact,
            lan_discovery: crate::config::LanDiscoveryConfig::default(),
            auth: AuthConfig::default(),
            igdb: IgdbConfig::default(),
        };
        let state = AppState::initialize(config).await.unwrap();
        let platform = platforms::find_by_slug(state.db(), "genesis")
            .await
            .unwrap()
            .unwrap();
        let rom_id = roms::create_stub(state.db(), platform.id, "Cover Test", "cover-test")
            .await
            .unwrap();
        let rom = roms::find_by_id(state.db(), rom_id).await.unwrap().unwrap();
        let large_path = cover_resource_path(&rom, "cover-large.jpg");
        let small_path = cover_resource_path(&rom, "cover-small.jpg");
        state.file_store().fail_after_mutations(2);

        let error = prepare_cover_assets(
            &state,
            &rom,
            vec![
                DownloadedCover {
                    asset: CachedCoverAsset {
                        kind: "large".to_string(),
                        resource_path: large_path.clone(),
                        media_type: "image/jpeg".to_string(),
                        file_size_bytes: 5,
                    },
                    bytes: b"large".to_vec(),
                },
                DownloadedCover {
                    asset: CachedCoverAsset {
                        kind: "small".to_string(),
                        resource_path: small_path.clone(),
                        media_type: "image/jpeg".to_string(),
                        file_size_bytes: 5,
                    },
                    bytes: b"small".to_vec(),
                },
            ],
        )
        .await
        .unwrap_err();

        assert!(error.to_string().contains("injected managed filesystem"));
        let root = state.config().asset_root.canonicalize().unwrap();
        assert!(!root.join(large_path).exists());
        assert!(!root.join(small_path).exists());
        let pending = file_operation_repository::pending(state.db())
            .await
            .unwrap()
            .into_iter()
            .filter(|operation| operation.kind == FileOperationKind::CoverReplace)
            .count();
        assert_eq!(pending, 0);
    }
}
