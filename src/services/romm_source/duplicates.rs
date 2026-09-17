//! Duplicate refusal for RomM imports.
//!
//! Teatro does not import duplicates. There is no force flag and no override: an operator who
//! genuinely wants a re-import deletes the local ROM first. The same check runs independently at
//! three checkpoints — browse (advisory), admission (before any byte moves), and post-download
//! (using locally computed hashes) — so no single missed check can produce a second copy.

use crate::{repositories::roms, state::AppState};

use super::RommSourceError;

/// One candidate file being evaluated against the local library.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CandidateFile {
    pub file_name: String,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

/// Why an import was refused, and which local ROM already holds the game.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct DuplicateMatch {
    /// `content_hash`, `file_name`, or `title_slug`.
    pub rule: &'static str,
    pub detail: String,
    pub existing_rom_id: Option<i64>,
    pub existing_rom_name: Option<String>,
}

/// Evaluates every rule against the target platform, strongest signal first.
///
/// Grouped games are all-or-nothing: a hit on any member refuses the whole game, because the
/// existing group's manifest and relationship rows are immutable by design.
pub(crate) async fn find_duplicate(
    state: &AppState,
    platform_id: i64,
    title: &str,
    files: &[CandidateFile],
) -> Result<Option<DuplicateMatch>, RommSourceError> {
    let collect = |select: fn(&CandidateFile) -> Option<&String>| {
        files.iter().filter_map(select).cloned().collect::<Vec<_>>()
    };

    // 1. Content hash. Platform-independent: identical bytes anywhere are already stored.
    if let Some(matched) = roms::find_file_by_hashes(
        state.db(),
        &collect(|file| file.sha256.as_ref()),
        &collect(|file| file.sha1.as_ref()),
        &collect(|file| file.md5.as_ref()),
        &collect(|file| file.crc32.as_ref()),
    )
    .await?
    {
        return Ok(Some(DuplicateMatch {
            rule: "content_hash",
            detail: format!(
                "The {} of a selected file matches “{}” already stored as {}.",
                matched.algorithm, matched.rom_name, matched.file_name
            ),
            existing_rom_id: Some(matched.rom_id),
            existing_rom_name: Some(matched.rom_name),
        }));
    }

    // 2. Normalized filename on the target platform.
    let file_names = files
        .iter()
        .map(|file| file.file_name.clone())
        .collect::<Vec<_>>();
    let existing = roms::existing_file_names(state.db(), platform_id, &file_names).await?;
    if let Some(file_name) = file_names.iter().find(|name| existing.contains(*name)) {
        return Ok(Some(DuplicateMatch {
            rule: "file_name",
            detail: format!("{file_name} already exists on the target platform."),
            existing_rom_id: None,
            existing_rom_name: None,
        }));
    }

    // 3. Normalized title plus platform.
    let slug = crate::services::library::slugify(title);
    if !slug.is_empty() && roms::slug_exists(state.db(), platform_id, &slug).await? {
        return Ok(Some(DuplicateMatch {
            rule: "title_slug",
            detail: format!("“{title}” already exists on the target platform."),
            existing_rom_id: None,
            existing_rom_name: None,
        }));
    }

    Ok(None)
}

/// Names the hash algorithms that were actually available for the check, for honest reporting.
pub(crate) fn hash_signals(files: &[CandidateFile]) -> Vec<&'static str> {
    let mut signals = Vec::new();
    for (name, present) in [
        ("sha256", files.iter().any(|file| file.sha256.is_some())),
        ("sha1", files.iter().any(|file| file.sha1.is_some())),
        ("md5", files.iter().any(|file| file.md5.is_some())),
        ("crc32", files.iter().any(|file| file.crc32.is_some())),
    ] {
        if present {
            signals.push(name);
        }
    }
    signals
}
