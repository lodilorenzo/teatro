use std::collections::BTreeSet;

use serde::Deserialize;
use serde_json::json;
use thiserror::Error;

use crate::domain::integrity::NewDatEntry;

use super::filename;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDat {
    pub name: Option<String>,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author: Option<String>,
    pub homepage: Option<String>,
    pub entries: Vec<NewDatEntry>,
    pub skipped_entries: usize,
}

#[derive(Debug, Error)]
pub enum DatParseError {
    #[error("DAT XML is invalid: {0}")]
    InvalidXml(#[from] quick_xml::DeError),

    #[error("DAT XML does not contain any game or machine entries")]
    Empty,

    #[error("DAT XML does not contain any ROM entries with a supported checksum")]
    NoChecksums,
}

pub fn parse_logiqx_xml(contents: &str) -> Result<ParsedDat, DatParseError> {
    let document: DatafileXml = quick_xml::de::from_str(contents)?;
    let mut games = document.games;
    games.extend(document.machines);
    if games.is_empty() {
        return Err(DatParseError::Empty);
    }

    let mut entries = Vec::new();
    let mut skipped_entries = 0;

    for game in games {
        let game_name = clean_text(Some(game.name.as_str())).unwrap_or_else(|| "Unknown".into());
        let description = clean_text(game.description.as_deref());
        let inferred = filename::parse(description.as_deref().unwrap_or(&game_name));
        let serial =
            clean_text(game.serial.as_deref()).or_else(|| clean_text(game.serial_attr.as_deref()));
        let mut regions: BTreeSet<String> = inferred.regions.into_iter().collect();
        let mut languages: BTreeSet<String> = inferred.languages.into_iter().collect();

        extend_csv(&mut regions, game.region.as_deref());
        extend_csv(&mut regions, game.region_attr.as_deref());
        extend_csv(&mut languages, game.language.as_deref());
        extend_csv(&mut languages, game.language_attr.as_deref());
        for release in &game.releases {
            extend_csv(&mut regions, release.region.as_deref());
            extend_csv(&mut languages, release.language.as_deref());
        }

        for rom in game.roms {
            let crc32 = normalize_hash(rom.crc.as_deref(), 8);
            let md5 = normalize_hash(rom.md5.as_deref(), 32);
            let sha1 = normalize_hash(rom.sha1.as_deref(), 40);
            let sha256 = normalize_hash(rom.sha256.as_deref(), 64);
            if crc32.is_none() && md5.is_none() && sha1.is_none() && sha256.is_none() {
                skipped_entries += 1;
                continue;
            }

            let Some(rom_name) = clean_text(Some(rom.name.as_str())) else {
                skipped_entries += 1;
                continue;
            };
            let file_size_bytes = rom.size.and_then(|size| i64::try_from(size).ok());
            entries.push(NewDatEntry {
                game_name: game_name.clone(),
                description: description.clone(),
                rom_name,
                file_size_bytes,
                crc32,
                md5,
                sha1,
                sha256,
                serial: serial.clone(),
                regions: regions.iter().cloned().collect(),
                languages: languages.iter().cloned().collect(),
                metadata: json!({
                    "release_names": game.releases.iter().filter_map(|release| clean_text(release.name.as_deref())).collect::<Vec<_>>()
                }),
            });
        }
    }

    if entries.is_empty() {
        return Err(DatParseError::NoChecksums);
    }

    Ok(ParsedDat {
        name: document
            .header
            .as_ref()
            .and_then(|header| clean_text(header.name.as_deref())),
        description: document
            .header
            .as_ref()
            .and_then(|header| clean_text(header.description.as_deref())),
        version: document
            .header
            .as_ref()
            .and_then(|header| clean_text(header.version.as_deref())),
        author: document
            .header
            .as_ref()
            .and_then(|header| clean_text(header.author.as_deref())),
        homepage: document.header.as_ref().and_then(|header| {
            clean_text(header.homepage.as_deref()).or_else(|| clean_text(header.url.as_deref()))
        }),
        entries,
        skipped_entries,
    })
}

fn normalize_hash(value: Option<&str>, expected_len: usize) -> Option<String> {
    let value = value?.trim();
    if value.len() != expected_len || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    Some(value.to_ascii_lowercase())
}

fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn extend_csv(values: &mut BTreeSet<String>, input: Option<&str>) {
    let Some(input) = input else { return };
    for value in input.split([',', ';', '/']) {
        let value = value.trim();
        if !value.is_empty() {
            values.insert(value.to_string());
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename = "datafile")]
struct DatafileXml {
    #[serde(default)]
    header: Option<HeaderXml>,
    #[serde(rename = "game", default)]
    games: Vec<GameXml>,
    #[serde(rename = "machine", default)]
    machines: Vec<GameXml>,
}

#[derive(Debug, Deserialize)]
struct HeaderXml {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    homepage: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GameXml {
    #[serde(rename = "@name")]
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    serial: Option<String>,
    #[serde(rename = "@serial", default)]
    serial_attr: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(rename = "@region", default)]
    region_attr: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(rename = "@language", default)]
    language_attr: Option<String>,
    #[serde(rename = "release", default)]
    releases: Vec<ReleaseXml>,
    #[serde(rename = "rom", default)]
    roms: Vec<RomXml>,
}

#[derive(Debug, Deserialize)]
struct ReleaseXml {
    #[serde(rename = "@name", default)]
    name: Option<String>,
    #[serde(rename = "@region", default)]
    region: Option<String>,
    #[serde(rename = "@language", alias = "@languages", default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RomXml {
    #[serde(rename = "@name")]
    name: String,
    #[serde(rename = "@size", default)]
    size: Option<u64>,
    #[serde(rename = "@crc", default)]
    crc: Option<String>,
    #[serde(rename = "@md5", default)]
    md5: Option<String>,
    #[serde(rename = "@sha1", default)]
    sha1: Option<String>,
    #[serde(rename = "@sha256", default)]
    sha256: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::parse_logiqx_xml;

    #[test]
    fn parses_logiqx_games_and_authoritative_disc_metadata() {
        let dat = parse_logiqx_xml(
            r#"<?xml version="1.0"?>
<datafile>
  <header><name>Redump Test</name><version>2026-07-09</version></header>
  <game name="Verified Game (USA)">
    <description>Verified Game (USA)</description>
    <serial>SLUS-12345</serial>
    <release name="Verified Game" region="USA" language="En,Fr" />
    <rom name="Verified Game.bin" size="4" crc="9B0D08F1" md5="098F6BCD4621D373CADE4E832627B4F6" sha1="A94A8FE5CCB19BA61C4C0873D391E987982FBBD3" />
  </game>
</datafile>"#,
        )
        .unwrap();

        assert_eq!(dat.name.as_deref(), Some("Redump Test"));
        assert_eq!(dat.entries.len(), 1);
        assert_eq!(dat.entries[0].serial.as_deref(), Some("SLUS-12345"));
        assert_eq!(dat.entries[0].regions, vec!["USA"]);
        assert_eq!(dat.entries[0].languages, vec!["En", "Fr"]);
        assert_eq!(
            dat.entries[0].sha1.as_deref(),
            Some("a94a8fe5ccb19ba61c4c0873d391e987982fbbd3")
        );
    }

    #[test]
    fn rejects_dats_without_supported_checksums() {
        let error = parse_logiqx_xml(
            r#"<datafile><game name="No Hash"><rom name="game.bin" size="1" /></game></datafile>"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("supported checksum"));
    }
}
