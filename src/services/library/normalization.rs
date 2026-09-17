use std::{
    collections::{BTreeSet, HashSet},
    path::Path,
};

use serde_json::{Map, Value, json};

use crate::{
    domain::{
        ingest::{ParsedFilename, PlanFileKey},
        platform::Platform,
        rom::{DependencyKind, FileRole},
    },
    repositories::{platforms, roms},
    services::ingest::filename,
    state::AppState,
};

use super::types::{IngestPlanRom, LibraryServiceError, PreparedPlanFile};

pub(super) fn file_key(index: usize) -> PlanFileKey {
    PlanFileKey::Uploaded(index)
}

pub(super) fn generated_file_key(rom_index: usize) -> PlanFileKey {
    PlanFileKey::GeneratedM3u(rom_index)
}

pub(super) fn generated_m3u_file_name(
    first_disc: &PreparedPlanFile,
    reserved_file_names: &HashSet<String>,
) -> String {
    let base_name = strip_disc_marker_from_base_name(first_disc)
        .filter(|base_name| !base_name.trim().is_empty())
        .unwrap_or_else(|| first_disc.parsed.clean_title.clone());
    let desired_file_name = format!("{}.m3u", clean_generated_base_name(&base_name));

    for attempt in 0..1000 {
        let file_name = collision_file_name(&desired_file_name, attempt);
        if !reserved_file_names.contains(&file_name.to_ascii_lowercase())
            && sanitize_upload_file_name(&file_name).is_ok()
        {
            return file_name;
        }
    }

    "generated.m3u".to_string()
}

pub(super) fn strip_disc_marker_from_base_name(file: &PreparedPlanFile) -> Option<String> {
    let disc = file.parsed.disc.as_ref()?;
    let mut base_name = file.parsed.base_name.clone();
    let raw = disc.raw.trim();

    for token in [format!("({raw})"), format!("[{raw}]")] {
        if let Some(index) = base_name.find(&token) {
            base_name.replace_range(index..index + token.len(), "");
            return Some(clean_generated_base_name(&base_name));
        }
    }

    let lower_base = base_name.to_ascii_lowercase();
    let lower_raw = raw.to_ascii_lowercase();
    if let Some(index) = lower_base.rfind(&lower_raw) {
        let before = &base_name[..index];
        let after = &base_name[index + raw.len()..];
        let marker_is_suffix = after
            .chars()
            .all(|character| character.is_whitespace() || matches!(character, '-' | '_'));
        let has_separator = before
            .chars()
            .last()
            .is_some_and(|character| character.is_whitespace() || matches!(character, '-' | '_'));
        if marker_is_suffix && has_separator {
            base_name.replace_range(index..index + raw.len(), "");
            return Some(clean_generated_base_name(&base_name));
        }
    }

    None
}

fn clean_generated_base_name(base_name: &str) -> String {
    let mut cleaned = String::new();
    let mut previous_space = false;

    for character in base_name.trim().chars() {
        if character.is_whitespace() {
            if !previous_space {
                cleaned.push(' ');
                previous_space = true;
            }
        } else {
            cleaned.push(character);
            previous_space = false;
        }
    }

    let cleaned = cleaned
        .trim_matches(|character: char| character.is_whitespace() || matches!(character, '-' | '_'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if cleaned.is_empty() {
        "generated".to_string()
    } else {
        cleaned
    }
}

pub(super) fn extension_lower(file_name: &str) -> String {
    file_name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default()
}

pub(super) fn is_manifest_file_name(file_name: &str) -> bool {
    is_manifest_extension(&extension_lower(file_name))
}

pub(super) fn m3u_directory_name(
    planned_rom: &IngestPlanRom,
) -> Result<Option<String>, LibraryServiceError> {
    planned_rom
        .files
        .iter()
        .find(|file| {
            file.role == FileRole::LaunchManifest
                && extension_lower(&file.original_file_name) == "m3u"
        })
        .map(|file| sanitize_upload_file_name(&file.original_file_name))
        .transpose()
}

pub(super) fn is_manifest_extension(extension: &str) -> bool {
    matches!(extension, "m3u" | "cue" | "gdi")
}

pub(super) fn is_disc_image_extension(extension: &str) -> bool {
    matches!(extension, "chd" | "iso" | "img" | "cso" | "rvz")
}

pub(super) fn independent_role(extension: &str) -> FileRole {
    match extension {
        "m3u" => FileRole::LaunchManifest,
        "cue" | "gdi" => FileRole::Descriptor,
        "chd" | "iso" | "img" | "cso" | "rvz" => FileRole::DiscImage,
        _ => FileRole::Content,
    }
}

pub(super) fn descriptor_dependency_kind(extension: &str) -> DependencyKind {
    match extension {
        "gdi" => DependencyKind::GdiTrack,
        _ => DependencyKind::CueFile,
    }
}

pub(super) fn upload_filename_metadata(
    title: &str,
    parsed_filename: &ParsedFilename,
) -> Option<String> {
    if !parsed_filename.has_metadata() {
        return None;
    }

    let mut metadata = Map::new();
    metadata.insert("source".to_string(), Value::String("filename".to_string()));
    metadata.insert("schema_version".to_string(), json!(1));
    metadata.insert("name".to_string(), Value::String(title.to_string()));
    metadata.insert("filename".to_string(), json!(parsed_filename));

    if !parsed_filename.regions.is_empty() {
        metadata.insert("regions".to_string(), json!(parsed_filename.regions));
    }
    if let Some(revision) = parsed_filename.revision {
        metadata.insert("revision".to_string(), json!(revision));
    }
    if let Some(version) = parsed_filename.version.as_deref() {
        metadata.insert("version".to_string(), Value::String(version.to_string()));
    }

    Some(Value::Object(metadata).to_string())
}

pub(super) async fn resolve_platform(
    state: &AppState,
    platform_id: Option<i64>,
    platform_slug: Option<&str>,
) -> Result<Platform, LibraryServiceError> {
    match (
        platform_id,
        platform_slug.filter(|slug| !slug.trim().is_empty()),
    ) {
        (Some(id), Some(slug)) => {
            let by_id = platforms::find_by_id(state.db(), id)
                .await?
                .ok_or(LibraryServiceError::PlatformNotFound)?;
            let by_slug = platforms::find_by_slug(state.db(), slug.trim())
                .await?
                .ok_or(LibraryServiceError::PlatformNotFound)?;

            if by_id.id != by_slug.id {
                return Err(LibraryServiceError::PlatformMismatch);
            }

            Ok(by_id)
        }
        (Some(id), None) => platforms::find_by_id(state.db(), id)
            .await?
            .ok_or(LibraryServiceError::PlatformNotFound),
        (None, Some(slug)) => platforms::find_by_slug(state.db(), slug.trim())
            .await?
            .ok_or(LibraryServiceError::PlatformNotFound),
        (None, None) => Err(LibraryServiceError::MissingPlatform),
    }
}

pub(super) async fn unique_slug(
    state: &AppState,
    platform_id: i64,
    base_slug: &str,
) -> Result<String, LibraryServiceError> {
    let base_slug = if base_slug.is_empty() {
        "rom"
    } else {
        base_slug
    };

    for attempt in 0..1000 {
        let slug = if attempt == 0 {
            base_slug.to_string()
        } else {
            format!("{base_slug}-{}", attempt + 1)
        };

        if !roms::slug_exists(state.db(), platform_id, &slug).await? {
            return Ok(slug);
        }
    }

    Err(LibraryServiceError::NoAvailableFileName)
}

pub(super) async fn unique_slug_excluding(
    state: &AppState,
    platform_id: i64,
    base_slug: &str,
    excluded_rom_id: i64,
) -> Result<String, LibraryServiceError> {
    let base_slug = if base_slug.is_empty() {
        "rom"
    } else {
        base_slug
    };

    for attempt in 0..1000 {
        let slug = if attempt == 0 {
            base_slug.to_string()
        } else {
            format!("{base_slug}-{}", attempt + 1)
        };

        if !roms::slug_exists_excluding(state.db(), platform_id, &slug, excluded_rom_id).await? {
            return Ok(slug);
        }
    }

    Err(LibraryServiceError::NoAvailableFileName)
}

pub(super) async fn available_file_name(
    state: &AppState,
    root_id: i64,
    root_path: &Path,
    platform_relative: &Path,
    original_file_name: &str,
) -> Result<String, LibraryServiceError> {
    for attempt in 0..1000 {
        let file_name = collision_file_name(original_file_name, attempt);
        let relative_path = join_relative_path(platform_relative, &file_name);
        let file_exists = state.file_store().exists(root_path, &relative_path).await?;
        let db_exists = roms::relative_path_exists(state.db(), root_id, &relative_path).await?;
        if !file_exists && !db_exists {
            return Ok(file_name);
        }
    }

    Err(LibraryServiceError::NoAvailableFileName)
}

pub(crate) fn sanitize_upload_file_name(file_name: &str) -> Result<String, LibraryServiceError> {
    let file_name = file_name.trim();
    if file_name.is_empty() || file_name == "." || file_name == ".." {
        return Err(LibraryServiceError::InvalidFileName);
    }

    if file_name.contains('/')
        || file_name.contains('\\')
        || file_name.contains('\0')
        || file_name.chars().any(char::is_control)
    {
        return Err(LibraryServiceError::InvalidFileName);
    }

    if Path::new(file_name).components().count() != 1 {
        return Err(LibraryServiceError::InvalidFileName);
    }

    Ok(file_name.to_string())
}

pub(crate) fn normalized_title(
    requested_title: Option<&str>,
    file_name: &str,
) -> Result<String, LibraryServiceError> {
    let title = requested_title
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| title_from_file_name(file_name));

    if title
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(LibraryServiceError::InvalidTitle);
    }

    Ok(title)
}

pub(super) fn normalized_edit_title(title: &str) -> Result<String, LibraryServiceError> {
    let title = title.trim();
    if title.is_empty()
        || title
            .chars()
            .any(|character| character == '\0' || character.is_control())
    {
        return Err(LibraryServiceError::InvalidTitle);
    }

    Ok(title.to_string())
}

pub(super) fn clean_optional_text(value: &str) -> Result<Option<String>, LibraryServiceError> {
    let value = value.trim();
    if value.contains('\0') {
        return Err(LibraryServiceError::InvalidMetadataField);
    }

    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value.to_string()))
    }
}

pub(super) fn normalize_metadata_list(
    values: Vec<String>,
) -> Result<Vec<String>, LibraryServiceError> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();

    for value in values {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if value
            .chars()
            .any(|character| character == '\0' || character.is_control())
        {
            return Err(LibraryServiceError::InvalidMetadataField);
        }

        let key = value.to_ascii_lowercase();
        if seen.insert(key) {
            normalized.push(value.to_string());
        }
    }

    Ok(normalized)
}

pub(super) fn normalize_release_year(
    value: Option<i32>,
) -> Result<Option<i32>, LibraryServiceError> {
    if let Some(year) = value
        && !(0..=9999).contains(&year)
    {
        return Err(LibraryServiceError::InvalidMetadataField);
    }

    Ok(value)
}

fn title_from_file_name(file_name: &str) -> String {
    filename::parse(file_name).clean_title
}

pub(crate) fn slugify(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;

    for character in title.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
    }

    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        "rom".to_string()
    } else {
        slug
    }
}

fn collision_file_name(file_name: &str, attempt: usize) -> String {
    if attempt == 0 {
        return file_name.to_string();
    }

    let (stem, extension) = split_extension(file_name);
    match extension {
        Some(extension) => format!("{stem} ({attempt}).{extension}"),
        None => format!("{stem} ({attempt})"),
    }
}

fn split_extension(file_name: &str) -> (&str, Option<&str>) {
    match file_name.rfind('.') {
        Some(index) if index > 0 && index < file_name.len() - 1 => {
            (&file_name[..index], Some(&file_name[index + 1..]))
        }
        _ => (file_name, None),
    }
}

pub(super) fn join_relative_path(parent: &Path, file_name: &str) -> String {
    let parent = parent.to_string_lossy().replace('\\', "/");
    if parent.is_empty() {
        file_name.to_string()
    } else {
        format!("{parent}/{file_name}")
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::rom::FileRole;

    use super::{
        collision_file_name, independent_role, is_disc_image_extension, sanitize_upload_file_name,
        slugify, title_from_file_name,
    };

    #[test]
    fn rejects_upload_filename_traversal() {
        assert!(sanitize_upload_file_name("../secret.bin").is_err());
        assert!(sanitize_upload_file_name("nested/game.bin").is_err());
        assert!(sanitize_upload_file_name("..\\secret.bin").is_err());
    }

    #[test]
    fn detects_img_disc_images() {
        assert!(is_disc_image_extension("img"));
        assert_eq!(independent_role("img"), FileRole::DiscImage);
        assert_eq!(title_from_file_name("Game (Disc 1).img"), "Game");
    }

    #[test]
    fn derives_titles_slugs_and_collision_names() {
        assert_eq!(
            title_from_file_name("Sonic The Hedgehog (USA).bin"),
            "Sonic The Hedgehog"
        );
        assert_eq!(slugify("Sonic The Hedgehog"), "sonic-the-hedgehog");
        assert_eq!(collision_file_name("game.bin", 2), "game (2).bin");
        assert_eq!(collision_file_name("game", 1), "game (1)");
    }
}
