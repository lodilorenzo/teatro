const PLATFORM_IDS: &[(&str, &[i64])] = &[
    ("3do", &[50]),
    ("amigacd32", &[114]),
    ("atarijaguar", &[62]),
    ("atarijaguarcd", &[410]),
    ("atarilynx", &[61]),
    ("cdimono1", &[117]),
    ("colecovision", &[68]),
    ("dreamcast", &[23]),
    ("famicom", &[99]),
    ("fds", &[51]),
    ("gamegear", &[35]),
    ("gb", &[33]),
    ("gba", &[24]),
    ("gbc", &[22]),
    ("gc", &[21]),
    ("genesis", &[29]),
    ("intellivision", &[67]),
    ("mastersystem", &[64]),
    ("megacd", &[78]),
    ("megacdjp", &[78]),
    ("megadrive", &[29]),
    ("n3ds", &[37, 137]),
    ("n64", &[4]),
    ("n64dd", &[416]),
    ("nds", &[20, 159]),
    ("neogeo", &[79, 80]),
    ("neogeocd", &[136]),
    ("nes", &[18]),
    ("ngp", &[119]),
    ("ngpc", &[120]),
    ("pcengine", &[86]),
    ("pcenginecd", &[150]),
    ("pcfx", &[274]),
    ("ps2", &[8]),
    ("ps3", &[9]),
    ("ps4", &[48]),
    ("psp", &[38]),
    ("psvita", &[46]),
    ("psx", &[7]),
    ("satellaview", &[306]),
    ("saturn", &[32]),
    ("saturnjp", &[32]),
    ("sega32x", &[30]),
    ("sega32xjp", &[30]),
    ("sega32xna", &[30]),
    ("segacd", &[78]),
    ("sfc", &[58]),
    ("snes", &[19]),
    ("snesna", &[19]),
    ("supergrafx", &[128]),
    ("switch", &[130]),
    ("tg-cd", &[150]),
    ("tg16", &[86]),
    ("virtualboy", &[87]),
    ("wii", &[5]),
    ("wiiu", &[41]),
    ("win", &[6]),
    ("wonderswan", &[57]),
    ("wonderswancolor", &[123]),
    ("xbox", &[11]),
    ("xbox360", &[12]),
];

// Super Game Boy has no approved exact IGDB mapping; a Game Boy adapter proxy is deferred.
const NO_AUTOMATIC_MAPPING: &[&str] = &["sgb"];

pub(super) fn for_slug(slug: &str) -> Option<&'static [i64]> {
    // ponytail: a linear scan stays simpler until the static catalog becomes meaningfully large.
    PLATFORM_IDS
        .iter()
        .find_map(|(candidate, ids)| (*candidate == slug).then_some(*ids))
}

pub(super) fn is_deliberately_unmapped(slug: &str) -> bool {
    NO_AUTOMATIC_MAPPING.binary_search(&slug).is_ok()
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, str::FromStr};

    use sqlx::{
        ConnectOptions,
        sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    };

    use super::{NO_AUTOMATIC_MAPPING, PLATFORM_IDS, for_slug};

    #[test]
    fn approved_platform_ids_cover_direct_regional_family_and_alias_mappings() {
        assert_eq!(for_slug("genesis"), Some(&[29][..]));
        assert_eq!(for_slug("nes"), Some(&[18][..]));
        assert_eq!(for_slug("famicom"), Some(&[99][..]));
        assert_eq!(for_slug("snes"), Some(&[19][..]));
        assert_eq!(for_slug("sfc"), Some(&[58][..]));
        assert_eq!(for_slug("n3ds"), Some(&[37, 137][..]));
        assert_eq!(for_slug("neogeo"), Some(&[79, 80][..]));
        assert_eq!(for_slug("megacdjp"), Some(&[78][..]));
        assert_eq!(for_slug("segacd"), Some(&[78][..]));
        assert_eq!(for_slug("sgb"), None);
    }

    #[tokio::test]
    async fn mapping_is_sorted_valid_disjoint_and_covers_the_seeded_catalog() {
        assert!(PLATFORM_IDS.windows(2).all(|rows| rows[0].0 < rows[1].0));
        assert!(
            NO_AUTOMATIC_MAPPING
                .windows(2)
                .all(|slugs| slugs[0] < slugs[1])
        );

        let mut classified = BTreeSet::new();
        for (slug, ids) in PLATFORM_IDS {
            assert!(
                classified.insert((*slug).to_string()),
                "duplicate mapped slug: {slug}"
            );
            assert!(!ids.is_empty(), "empty ID mapping: {slug}");
            assert!(ids.iter().all(|id| *id > 0), "non-positive ID: {slug}");
            assert!(
                ids.windows(2).all(|ids| ids[0] < ids[1]),
                "unsorted or duplicate IDs: {slug}"
            );
        }
        for slug in NO_AUTOMATIC_MAPPING {
            assert!(
                classified.insert((*slug).to_string()),
                "mapped and unmapped slug: {slug}"
            );
        }

        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .disable_statement_logging();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let seeded = sqlx::query_scalar::<_, String>("SELECT slug FROM platforms ORDER BY slug")
            .fetch_all(&pool)
            .await
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>();

        assert_eq!(classified, seeded);
    }
}
