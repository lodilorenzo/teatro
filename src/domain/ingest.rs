use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanFileKey {
    Uploaded(usize),
    GeneratedM3u(usize),
}

impl std::fmt::Display for PlanFileKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Uploaded(index) => write!(formatter, "file-{index}"),
            Self::GeneratedM3u(rom_index) => {
                write!(formatter, "generated-rom-{}-m3u", rom_index + 1)
            }
        }
    }
}

impl Serialize for PlanFileKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PlanFileKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if let Some(index) = value
            .strip_prefix("file-")
            .and_then(|index| index.parse().ok())
        {
            return Ok(Self::Uploaded(index));
        }
        if let Some(index) = value
            .strip_prefix("generated-rom-")
            .and_then(|value| value.strip_suffix("-m3u"))
            .and_then(|index| index.parse::<usize>().ok())
            .and_then(|index| index.checked_sub(1))
        {
            return Ok(Self::GeneratedM3u(index));
        }

        Err(serde::de::Error::custom("invalid planned file key"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedFilename {
    pub original_filename: String,
    pub base_name: String,
    pub clean_title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub naming_convention_guess: Option<String>,
    pub regions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub languages: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disc: Option<DiscMarker>,
    pub dump_flags: DumpFlags,
    pub special_hardware: Vec<String>,
    pub unknown_tokens: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_archive: Option<SplitArchiveVolume>,
    pub confidence: ParseConfidence,
}

impl ParsedFilename {
    pub fn has_metadata(&self) -> bool {
        self.naming_convention_guess.is_some()
            || !self.regions.is_empty()
            || self.revision.is_some()
            || self.version.is_some()
            || !self.languages.is_empty()
            || self.disc.is_some()
            || self.dump_flags.has_flags()
            || !self.special_hardware.is_empty()
            || !self.unknown_tokens.is_empty()
            || self.split_archive.is_some()
            || !matches!(self.confidence, ParseConfidence::Low)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscMarker {
    pub raw: String,
    pub kind: DiscMarkerKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscMarkerKind {
    Disc,
    Disk,
    Cd,
    Side,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitArchiveVolume {
    pub raw: String,
    pub archive_format: String,
    pub set_name: String,
    pub volume_index: u32,
    pub is_first_volume: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DumpFlags {
    pub verified_good_dump: bool,
    pub pending_dump: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alternate: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bad_dump: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hack: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overdump: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trainer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pirate: Option<String>,
    pub unlicensed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub translation: Option<TranslationFlag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multilingual_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
    pub checksum_good: bool,
    pub checksum_bad: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rom_size: Option<String>,
    pub raw_tokens: Vec<String>,
}

impl DumpFlags {
    pub fn has_flags(&self) -> bool {
        self.verified_good_dump
            || self.pending_dump
            || self.alternate.is_some()
            || self.bad_dump.is_some()
            || self.fixed.is_some()
            || self.hack.is_some()
            || self.overdump.is_some()
            || self.trainer.is_some()
            || self.pirate.is_some()
            || self.unlicensed
            || self.translation.is_some()
            || self.multilingual_count.is_some()
            || self.checksum.is_some()
            || self.checksum_good
            || self.checksum_bad
            || self.rom_size.is_some()
            || !self.raw_tokens.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranslationFlag {
    pub raw: String,
    pub current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}
