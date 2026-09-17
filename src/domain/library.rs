use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRoot {
    pub id: i64,
    pub name: String,
    pub root_path: PathBuf,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStats {
    pub total_roms: i64,
    pub total_files: i64,
    pub total_file_bytes: i64,
    pub platforms_with_roms: i64,
    pub library_roots: Vec<LibraryRootStats>,
    pub platforms: Vec<PlatformLibraryStats>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryRootStats {
    pub id: i64,
    pub name: String,
    pub root_path: PathBuf,
    pub writable: bool,
    pub file_count: i64,
    pub total_file_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformLibraryStats {
    pub id: i64,
    pub slug: String,
    pub display_name: String,
    pub rom_count: i64,
    pub file_count: i64,
    pub total_file_bytes: i64,
}
