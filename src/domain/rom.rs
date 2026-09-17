use std::{fmt, path::PathBuf, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::domain::workflow::FileHashStatus;

macro_rules! persisted_enum {
    ($name:ident { $($variant:ident => $value:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = InvalidRomRelationshipValue;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                match value {
                    $($value => Ok(Self::$variant)),+,
                    _ => Err(InvalidRomRelationshipValue {
                        kind: stringify!($name),
                        value: value.to_string(),
                    }),
                }
            }
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid persisted {kind} value {value:?}")]
pub struct InvalidRomRelationshipValue {
    kind: &'static str,
    value: String,
}

persisted_enum!(FileRole {
    Content => "content",
    LaunchManifest => "launch_manifest",
    Descriptor => "descriptor",
    DiscImage => "disc_image",
    Track => "track",
    ArchiveVolume => "archive_volume",
    Manual => "manual",
    Patch => "patch",
    Dlc => "dlc",
    MetadataSidecar => "metadata_sidecar",
});

persisted_enum!(FileGroupKind {
    Single => "single",
    Playlist => "playlist",
    Disc => "disc",
    TrackSet => "track_set",
});

persisted_enum!(DependencyKind {
    PlaylistEntry => "playlist_entry",
    CueFile => "cue_file",
    GdiTrack => "gdi_track",
});

#[derive(Debug, Clone, PartialEq)]
pub struct Rom {
    pub id: i64,
    pub name: String,
    pub slug: String,
    pub platform_id: i64,
    pub platform_slug: String,
    pub platform_display_name: String,
    pub regions: Vec<String>,
    pub metadata: Value,
    pub summary: Option<String>,
    pub fs_name: Option<String>,
    pub fs_size_bytes: Option<i64>,
    pub path_cover_large: Option<String>,
    pub path_cover_small: Option<String>,
    pub url_cover: Option<String>,
    pub files: Vec<RomFile>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFile {
    pub id: i64,
    pub rom_id: i64,
    pub root_id: i64,
    pub root_path: PathBuf,
    pub relative_path: String,
    pub file_name: String,
    pub file_size_bytes: i64,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub hash_status: FileHashStatus,
    pub hashed_at: Option<String>,
    pub hash_error: Option<String>,
    pub is_primary: bool,
    pub group_id: Option<i64>,
    pub original_file_name: Option<String>,
    pub role: FileRole,
    pub sort_index: i64,
    pub disc_index: Option<i64>,
    pub track_index: Option<i64>,
    pub launchable: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RomFileGroup {
    pub id: i64,
    pub rom_id: i64,
    pub kind: FileGroupKind,
    pub display_name: String,
    pub group_key: Option<String>,
    pub disc_index: Option<i64>,
    pub disc_count: Option<i64>,
    pub launchable: bool,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomFileDependency {
    pub parent_file_id: i64,
    pub child_file_id: i64,
    pub dependency_kind: DependencyKind,
    pub sort_index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomListParams {
    pub limit: i64,
    pub offset: i64,
    pub search: Option<String>,
    pub newest_first: bool,
    pub missing_cover: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaginatedRoms {
    pub items: Vec<Rom>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

#[cfg(test)]
mod tests {
    use super::{DependencyKind, FileGroupKind, FileRole};

    #[test]
    fn relationship_values_keep_existing_json_strings() {
        assert_eq!(
            serde_json::to_string(&FileRole::LaunchManifest).unwrap(),
            "\"launch_manifest\""
        );
        assert_eq!(
            serde_json::to_string(&FileGroupKind::TrackSet).unwrap(),
            "\"track_set\""
        );
        assert_eq!(
            serde_json::to_string(&DependencyKind::PlaylistEntry).unwrap(),
            "\"playlist_entry\""
        );
    }

    #[test]
    fn unknown_relationship_values_are_rejected() {
        assert!("unknown".parse::<FileRole>().is_err());
        assert!("unknown".parse::<FileGroupKind>().is_err());
        assert!("unknown".parse::<DependencyKind>().is_err());
    }
}
