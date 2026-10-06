use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: i64,
    pub name: String,
    pub root_path: PathBuf,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryStats {
    pub total_roms: i64,
    pub total_files: i64,
    pub total_file_bytes: i64,
    pub platforms_with_roms: i64,
    pub library_roots: Vec<LibraryRootStats>,
    pub platforms: Vec<PlatformLibraryStats>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LibraryRootStats {
    pub id: i64,
    pub name: String,
    #[serde(serialize_with = "serialize_display_path")]
    pub root_path: PathBuf,
    pub writable: bool,
    pub file_count: i64,
    pub total_file_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformLibraryStats {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub rom_count: i64,
    pub file_count: i64,
    pub total_file_bytes: i64,
}

fn serialize_display_path<S: Serializer>(path: &Path, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(&path.display())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    #[test]
    fn stats_root_path_keeps_lossy_display_serialization() {
        let stats = super::LibraryRootStats {
            id: 1,
            name: "library".into(),
            root_path: OsString::from_vec(b"/tmp/\xff".to_vec()).into(),
            writable: true,
            file_count: 0,
            total_file_bytes: 0,
        };
        assert_eq!(serde_json::to_value(stats).unwrap()["root_path"], "/tmp/�");
    }
}
