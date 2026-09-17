//! SQL persistence boundary for ROMs, files, groups, dependencies, and library statistics.

mod params;
mod read;
mod rows;
mod stats;
mod write;

pub use read::{file_groups_for_rom, find_by_id, list};

pub(crate) use params::{
    CoverAssetUpsert, CreateGroupedRomParams, CreateRomFile, CreateRomFileDependency,
    CreateRomFileGroup, CreateRomWithFileParams, SaveCoverParams, SaveIgdbMetadataParams,
    UpdateRomMetadataParams,
};
pub(crate) use read::{
    all_ids, cover_asset_paths_for_rom_ids, cover_paths_for_rom, existing_file_names, exists,
    file_dependencies_for_rom, find_file_by_hashes, find_file_by_id, folder_has_files_outside_roms,
    ids_by_platform, indexed_relative_paths, relative_path_exists, slug_exists,
    slug_exists_excluding,
};
pub(crate) use stats::library_stats;
#[cfg(test)]
pub(crate) use write::create_stub;
pub(crate) use write::{
    create_grouped_roms, create_with_file, delete_ids_tx, save_cover, save_igdb_metadata,
    update_admin_metadata,
};
