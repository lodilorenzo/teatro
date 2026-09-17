//! Persistent, manually refreshed browse snapshot for one RomM source.

use std::collections::HashMap;

use sqlx::SqlitePool;

use crate::repositories::romm_source::{
    self, IndexedRemotePlatform, IndexedRemoteRom, RommSourceSettings,
};

use super::{RemotePlatform, RemoteRom, RemoteRomPage, RommClient, RommSourceError};

/// Downloads the complete remote list before atomically replacing the prior snapshot.
pub(crate) async fn refresh_catalog(
    db: &SqlitePool,
    client: &RommClient,
    settings: &RommSourceSettings,
) -> Result<(), RommSourceError> {
    let platforms = client.platforms(settings).await?;
    let by_id = platforms
        .iter()
        .map(|platform| {
            (
                platform.id,
                (platform.slug.as_str(), platform.name.as_str()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut roms = client.all_roms(settings).await?;
    for rom in &mut roms {
        if let Some((slug, name)) = rom.platform_id.and_then(|id| by_id.get(&id)) {
            rom.platform_slug.get_or_insert_with(|| (*slug).to_string());
            rom.platform_name.get_or_insert_with(|| (*name).to_string());
        }
    }

    let indexed_platforms = platforms
        .into_iter()
        .map(|platform| IndexedRemotePlatform {
            id: platform.id,
            slug: platform.slug,
            name: platform.name,
            rom_count: platform.rom_count,
        })
        .collect::<Vec<_>>();
    let indexed_roms = roms
        .into_iter()
        .map(|rom| IndexedRemoteRom {
            id: rom.id,
            name: rom.name,
            platform_id: rom.platform_id,
            platform_slug: rom.platform_slug,
            platform_name: rom.platform_name,
            fs_name: rom.fs_name,
            file_size_bytes: rom
                .file_size_bytes
                .and_then(|size| i64::try_from(size).ok()),
            has_cover: rom.has_cover,
            file_count: i64::try_from(rom.file_count).unwrap_or(i64::MAX),
        })
        .collect::<Vec<_>>();

    romm_source::replace_index(db, &indexed_platforms, &indexed_roms).await?;
    Ok(())
}

pub(crate) async fn cached_platforms(
    db: &SqlitePool,
) -> Result<Vec<RemotePlatform>, RommSourceError> {
    Ok(romm_source::indexed_platforms(db)
        .await?
        .into_iter()
        .map(|platform| RemotePlatform {
            id: platform.id,
            slug: platform.slug,
            name: platform.name,
            rom_count: platform.rom_count,
        })
        .collect())
}

pub(crate) async fn cached_roms(
    db: &SqlitePool,
    platform_id: Option<i64>,
    search: Option<&str>,
    limit: u32,
    offset: u32,
) -> Result<RemoteRomPage, RommSourceError> {
    let page = romm_source::indexed_roms(db, platform_id, search, limit, offset).await?;
    let returned = page.items.len();
    Ok(RemoteRomPage {
        items: page
            .items
            .into_iter()
            .map(|rom| RemoteRom {
                id: rom.id,
                name: rom.name,
                platform_id: rom.platform_id,
                platform_slug: rom.platform_slug,
                platform_name: rom.platform_name,
                fs_name: rom.fs_name,
                file_size_bytes: rom
                    .file_size_bytes
                    .and_then(|size| u64::try_from(size).ok()),
                has_cover: rom.has_cover,
                file_count: usize::try_from(rom.file_count).unwrap_or(usize::MAX),
            })
            .collect(),
        total: page.total,
        limit: page.limit,
        offset: page.offset,
        returned,
    })
}
