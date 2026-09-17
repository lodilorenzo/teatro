use crate::domain::{
    ingest::PlanFileKey,
    rom::{DependencyKind, FileGroupKind, FileRole},
};

pub(crate) struct CreateRomWithFileParams<'a> {
    pub platform_id: i64,
    pub name: &'a str,
    pub slug: &'a str,
    pub root_id: i64,
    pub relative_path: &'a str,
    pub file_name: &'a str,
    pub original_file_name: &'a str,
    pub file_size_bytes: i64,
    pub regions_json: &'a str,
    pub metadata_json: Option<&'a str>,
    pub file_operation_id: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct CreateGroupedRomParams {
    pub platform_id: i64,
    pub name: String,
    pub slug: String,
    pub regions_json: String,
    pub metadata_json: Option<String>,
    pub groups: Vec<CreateRomFileGroup>,
    pub files: Vec<CreateRomFile>,
    pub dependencies: Vec<CreateRomFileDependency>,
}

#[derive(Debug, Clone)]
pub(crate) struct CreateRomFileGroup {
    pub key: String,
    pub kind: FileGroupKind,
    pub display_name: String,
    pub group_key: Option<String>,
    pub disc_index: Option<i64>,
    pub disc_count: Option<i64>,
    pub launchable: bool,
    pub metadata_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CreateRomFile {
    pub key: PlanFileKey,
    pub group_key: Option<String>,
    pub root_id: i64,
    pub relative_path: String,
    pub file_name: String,
    pub original_file_name: String,
    pub file_size_bytes: i64,
    pub is_primary: bool,
    pub role: FileRole,
    pub sort_index: i64,
    pub disc_index: Option<i64>,
    pub track_index: Option<i64>,
    pub launchable: bool,
    pub metadata_json: String,
}

#[derive(Debug, Clone)]
pub(crate) struct CreateRomFileDependency {
    pub parent_file_key: PlanFileKey,
    pub child_file_key: PlanFileKey,
    pub dependency_kind: DependencyKind,
    pub sort_index: i64,
}

pub(crate) struct CoverAssetUpsert<'a> {
    pub resource_path: &'a str,
    pub media_type: &'a str,
    pub file_size_bytes: i64,
}

pub(crate) struct SaveCoverParams<'a> {
    pub rom_id: i64,
    pub metadata_source: &'a str,
    pub metadata_json: &'a str,
    pub path: &'a str,
    pub media_type: &'a str,
    pub file_size_bytes: i64,
    pub file_operation_id: &'a str,
}

pub(crate) struct SaveIgdbMetadataParams<'a> {
    pub rom_id: i64,
    pub metadata_json: &'a str,
    pub schema_version: i64,
    pub summary: Option<&'a str>,
    pub url_cover: Option<&'a str>,
    pub path_cover_large: Option<&'a str>,
    pub path_cover_small: Option<&'a str>,
    pub cover_assets: Vec<CoverAssetUpsert<'a>>,
    pub file_operation_id: Option<&'a str>,
}

pub(crate) struct UpdateRomMetadataParams<'a> {
    pub rom_id: i64,
    pub platform_id: i64,
    pub name: &'a str,
    pub slug: &'a str,
    pub summary: Option<&'a str>,
    pub regions_json: &'a str,
    pub metadata_source: &'a str,
    pub metadata_json: &'a str,
    pub schema_version: i64,
}
