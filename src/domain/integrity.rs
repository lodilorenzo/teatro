use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;

use crate::domain::workflow::{
    DatMatchMethod, FileHashStatus, IntegrityJobKind, IntegrityJobStatus, RomIntegrityStatus,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatSource {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub imported_file_name: String,
    pub file_sha256: String,
    pub entry_count: i64,
    pub imported_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDatSource {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub imported_file_name: String,
    pub file_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDatEntry {
    pub game_name: String,
    pub description: Option<String>,
    pub rom_name: String,
    pub file_size_bytes: Option<i64>,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub serial: Option<String>,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityJob {
    pub id: i64,
    pub kind: IntegrityJobKind,
    pub status: IntegrityJobStatus,
    pub rom_id: Option<i64>,
    pub force: bool,
    pub total_files: i64,
    pub processed_files: i64,
    pub hashed_files: i64,
    pub matched_files: i64,
    pub error_count: i64,
    pub error: Option<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrityFile {
    pub id: i64,
    pub rom_id: i64,
    pub root_path: PathBuf,
    pub relative_path: String,
    pub file_name: String,
    pub file_size_bytes: i64,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
    pub hash_status: FileHashStatus,
}

impl IntegrityFile {
    pub fn has_complete_hashes(&self) -> bool {
        self.hash_status == FileHashStatus::Complete
            && self.crc32.is_some()
            && self.md5.is_some()
            && self.sha1.is_some()
            && self.sha256.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileHashes {
    pub crc32: String,
    pub md5: String,
    pub sha1: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatMatch {
    pub file_id: i64,
    pub dat_entry_id: i64,
    pub source_id: i64,
    pub source_name: String,
    pub source_version: Option<String>,
    pub game_name: String,
    pub description: Option<String>,
    pub rom_name: String,
    pub matched_by: DatMatchMethod,
    pub serial: Option<String>,
    pub regions: Vec<String>,
    pub languages: Vec<String>,
    pub verified_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileIntegrityReport {
    pub file_id: i64,
    pub file_name: String,
    pub file_size_bytes: i64,
    pub hash_status: FileHashStatus,
    pub hashes: IntegrityHashes,
    pub hashed_at: Option<String>,
    pub hash_error: Option<String>,
    pub matches: Vec<DatMatch>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IntegrityHashes {
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RomIntegrityReport {
    pub rom_id: i64,
    pub status: RomIntegrityStatus,
    pub files: Vec<FileIntegrityReport>,
}
