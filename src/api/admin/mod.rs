//! Administrative HTTP handlers and request/response translation.

mod audit;
mod dto;
pub(super) mod errors;
mod gog_import;
mod igdb;
mod integrity;
mod multipart;
mod query;
mod romm_source;
mod roms;
mod scan;
mod tokens;
pub(super) mod transfers;
mod uploads;
mod users;

pub(super) use gog_import::{
    cancel_gog_import_job, gog_import_job, gog_import_status, import_gog_setup,
};
pub(super) use igdb::{
    apply_igdb_metadata, clear_igdb_settings, get_igdb_settings, igdb_search, igdb_status,
    save_igdb_settings,
};
pub(super) use integrity::{
    import_dat, integrity_job, list_dats, list_integrity_jobs, rom_integrity, start_integrity_job,
};
pub(super) use romm_source::{
    cancel_romm_import_job, clear_romm_source_settings, create_romm_import,
    refresh_romm_source_index, romm_import_job, romm_source_platforms, romm_source_rom_cover,
    romm_source_rom_detail, romm_source_roms, romm_source_status, save_romm_source_settings,
    test_romm_source,
};
pub(super) use roms::{
    delete_all_roms, delete_platform_roms, delete_rom, rom_files, stats, update_cover, update_rom,
};
pub(super) use scan::{
    cancel_library_scan_job, cleanup_sidecars, create_library_scan, library_scan_job,
    preview_sidecar_cleanup,
};
pub(super) use tokens::{create_api_token, list_api_tokens, revoke_api_token};
pub(super) use uploads::{preview_upload_batch, upload_batch, upload_rom};
pub(super) use users::{create_user, delete_user, list_users, reset_password, set_role};
