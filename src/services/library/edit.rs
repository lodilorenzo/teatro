use serde_json::{Map, Value, json};

use crate::{
    domain::{library::LibraryStats, rom::Rom},
    repositories::{platforms, roms},
    state::AppState,
};

use super::{
    normalization::{
        clean_optional_text, normalize_metadata_list, normalize_release_year,
        normalized_edit_title, resolve_platform, slugify, unique_slug_excluding,
    },
    types::{LibraryServiceError, UpdateRomDraft},
};

pub async fn update_rom(
    state: &AppState,
    rom_id: i64,
    draft: UpdateRomDraft,
) -> Result<Rom, LibraryServiceError> {
    let current = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(LibraryServiceError::RomNotFound)?;

    let platform = if draft.platform_id.is_some()
        || draft
            .platform_slug
            .as_deref()
            .is_some_and(|slug| !slug.trim().is_empty())
    {
        resolve_platform(state, draft.platform_id, draft.platform_slug.as_deref()).await?
    } else {
        platforms::find_by_id(state.db(), current.platform_id)
            .await?
            .ok_or(LibraryServiceError::PlatformNotFound)?
    };

    let name = match draft.name.as_deref() {
        Some(name) => normalized_edit_title(name)?,
        None => current.name.clone(),
    };

    let slug = if platform.id == current.platform_id && name == current.name {
        current.slug.clone()
    } else {
        unique_slug_excluding(state, platform.id, &slugify(&name), current.id).await?
    };

    let summary_was_provided = draft.summary.is_some();
    let regions_update = draft.regions.clone();

    let summary = match draft.summary {
        Some(Some(summary)) => clean_optional_text(&summary)?,
        Some(None) => None,
        None => current.summary.clone(),
    };

    let regions = match regions_update.clone() {
        Some(regions) => normalize_metadata_list(regions)?,
        None => current.regions.clone(),
    };
    let regions_json = serde_json::to_string(&regions)?;

    let mut metadata = match current.metadata {
        Value::Object(object) => object,
        _ => Map::new(),
    };

    if metadata
        .get("source")
        .and_then(Value::as_str)
        .is_none_or(|source| source.trim().is_empty())
    {
        metadata.insert("source".to_string(), Value::String("manual".to_string()));
    }
    metadata.insert("schema_version".to_string(), json!(1));
    metadata.insert("name".to_string(), Value::String(name.clone()));

    if let Some(summary) = summary.as_deref() {
        metadata.insert("summary".to_string(), Value::String(summary.to_string()));
    } else if summary_was_provided {
        metadata.insert("summary".to_string(), Value::Null);
    }

    if let Some(regions) = regions_update {
        metadata.insert(
            "regions".to_string(),
            json!(normalize_metadata_list(regions)?),
        );
    }
    if let Some(genres) = draft.genres {
        metadata.insert(
            "genres".to_string(),
            json!(normalize_metadata_list(genres)?),
        );
    }
    if let Some(developers) = draft.developers {
        metadata.insert(
            "developers".to_string(),
            json!(normalize_metadata_list(developers)?),
        );
    }
    if let Some(publishers) = draft.publishers {
        metadata.insert(
            "publishers".to_string(),
            json!(normalize_metadata_list(publishers)?),
        );
    }
    if let Some(release_year) = draft.release_year {
        match normalize_release_year(release_year)? {
            Some(release_year) => {
                metadata.insert("release_year".to_string(), json!(release_year));
            }
            None => {
                metadata.insert("release_year".to_string(), Value::Null);
            }
        }
    }

    let metadata_source = metadata
        .get("source")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|source| !source.is_empty())
        .unwrap_or("manual")
        .to_string();
    let metadata_json = Value::Object(metadata).to_string();

    Ok(roms::update_admin_metadata(
        state.db(),
        roms::UpdateRomMetadataParams {
            rom_id: current.id,
            platform_id: platform.id,
            name: &name,
            slug: &slug,
            summary: summary.as_deref(),
            regions_json: &regions_json,
            metadata_source: &metadata_source,
            metadata_json: &metadata_json,
            schema_version: 1,
        },
    )
    .await?)
}

pub async fn stats(state: &AppState) -> Result<LibraryStats, LibraryServiceError> {
    Ok(roms::library_stats(state.db()).await?)
}

pub(super) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
