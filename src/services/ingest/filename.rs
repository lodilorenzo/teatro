use crate::domain::ingest::{
    DiscMarker, DiscMarkerKind, DumpFlags, ParseConfidence, ParsedFilename, SplitArchiveVolume,
    TranslationFlag,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokenDelimiter {
    Parentheses,
    Brackets,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawToken {
    delimiter: TokenDelimiter,
    value: String,
}

#[derive(Debug, Default)]
struct ParseState {
    regions: Vec<String>,
    revision: Option<i32>,
    version: Option<String>,
    languages: Vec<String>,
    disc: Option<DiscMarker>,
    dump_flags: DumpFlags,
    special_hardware: Vec<String>,
    unknown_tokens: Vec<String>,
    recognized_tokens: usize,
    goodtools_tokens: usize,
    no_intro_tokens: usize,
}

const KNOWN_EXTENSIONS: &[&str] = &[
    "zip", "7z", "rar", "nes", "sfc", "smc", "gen", "md", "gb", "gbc", "gba", "n64", "z64", "v64",
    "32x", "bin", "iso", "img", "cue", "gdi", "chd", "m3u", "raw",
];

pub fn parse(original_filename: &str) -> ParsedFilename {
    let file_name = basename(original_filename).trim().to_string();
    let split_archive = detect_split_archive(&file_name);
    let base_name = split_archive
        .as_ref()
        .map(|volume| volume.set_name.clone())
        .unwrap_or_else(|| strip_known_extensions(&file_name));
    let base_name = base_name.trim().to_string();

    let (mut title_source, tokens) = extract_trailing_tokens(&base_name);
    let mut state = ParseState::default();

    for token in tokens {
        classify_token(&mut state, &token);
    }

    if state.disc.is_none()
        && let Some((stripped_title, disc)) = strip_trailing_disc_marker(&title_source)
    {
        title_source = stripped_title;
        state.disc = Some(disc);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
    }

    let clean_title = clean_title(&title_source);
    let naming_convention_guess = if state.goodtools_tokens > 0 {
        Some("goodtools".to_string())
    } else if state.no_intro_tokens > 0 {
        Some("no-intro".to_string())
    } else {
        None
    };
    let confidence = confidence(&state, split_archive.as_ref());

    ParsedFilename {
        original_filename: file_name,
        base_name,
        clean_title,
        naming_convention_guess,
        regions: state.regions,
        revision: state.revision,
        version: state.version,
        languages: state.languages,
        disc: state.disc,
        dump_flags: state.dump_flags,
        special_hardware: state.special_hardware,
        unknown_tokens: state.unknown_tokens,
        split_archive,
        confidence,
    }
}

pub fn detect_split_archive(file_name: &str) -> Option<SplitArchiveVolume> {
    let file_name = basename(file_name).trim();
    let lower = file_name.to_ascii_lowercase();

    if let Some((stem, numeric_suffix)) = lower.rsplit_once('.') {
        if numeric_suffix.len() == 3
            && numeric_suffix
                .chars()
                .all(|character| character.is_ascii_digit())
            && let Some((_set_name_lower, archive_format)) = stem.rsplit_once('.')
            && matches!(archive_format, "7z" | "zip" | "rar")
        {
            let suffix_start = file_name.len() - numeric_suffix.len();
            let stem_original = &file_name[..suffix_start - 1];
            let set_name_len = stem_original.len() - archive_format.len() - 1;
            let set_name = file_name[..set_name_len].to_string();
            let volume_index = numeric_suffix.parse::<u32>().ok()?;
            if volume_index > 0 {
                return Some(SplitArchiveVolume {
                    raw: file_name.to_string(),
                    archive_format: archive_format.to_string(),
                    set_name: set_name.trim().to_string(),
                    volume_index,
                    is_first_volume: volume_index == 1,
                });
            }
        }

        if numeric_suffix.len() == 3
            && numeric_suffix.starts_with('z')
            && numeric_suffix[1..]
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            let volume_index = numeric_suffix[1..].parse::<u32>().ok()?;
            if volume_index > 0 {
                let set_name = file_name[..file_name.len() - numeric_suffix.len() - 1].to_string();
                return Some(SplitArchiveVolume {
                    raw: file_name.to_string(),
                    archive_format: "zip".to_string(),
                    set_name: set_name.trim().to_string(),
                    volume_index,
                    is_first_volume: volume_index == 1,
                });
            }
        }

        if numeric_suffix.len() == 3
            && numeric_suffix.starts_with('r')
            && numeric_suffix[1..]
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            let legacy_index = numeric_suffix[1..].parse::<u32>().ok()?;
            let set_name = file_name[..file_name.len() - numeric_suffix.len() - 1].to_string();
            return Some(SplitArchiveVolume {
                raw: file_name.to_string(),
                archive_format: "rar".to_string(),
                set_name: set_name.trim().to_string(),
                volume_index: legacy_index + 2,
                is_first_volume: false,
            });
        }
    }

    if lower.ends_with(".rar") {
        let without_rar = &file_name[..file_name.len() - 4];
        let lower_without_rar = &lower[..lower.len() - 4];
        if let Some(part_index) = lower_without_rar.rfind(".part") {
            let digits = &lower_without_rar[part_index + 5..];
            if !digits.is_empty() && digits.chars().all(|character| character.is_ascii_digit()) {
                let volume_index = digits.parse::<u32>().ok()?;
                if volume_index > 0 {
                    return Some(SplitArchiveVolume {
                        raw: file_name.to_string(),
                        archive_format: "rar".to_string(),
                        set_name: without_rar[..part_index].trim().to_string(),
                        volume_index,
                        is_first_volume: volume_index == 1,
                    });
                }
            }
        }
    }

    None
}

#[cfg(test)]
fn parse_disc_marker(raw: &str) -> Option<DiscMarker> {
    parse_disc_marker_text(raw.trim())
}

fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn strip_known_extensions(file_name: &str) -> String {
    let mut current = file_name.trim().to_string();

    for _ in 0..8 {
        let Some(dot_index) = current.rfind('.') else {
            break;
        };
        if dot_index == 0 || dot_index >= current.len() - 1 {
            break;
        }

        let extension = current[dot_index + 1..].to_ascii_lowercase();
        if KNOWN_EXTENSIONS.contains(&extension.as_str()) {
            current.truncate(dot_index);
            current = current.trim_end().to_string();
        } else {
            break;
        }
    }

    current
}

fn extract_trailing_tokens(base_name: &str) -> (String, Vec<RawToken>) {
    let mut remaining = base_name.trim_end().to_string();
    let mut tokens = Vec::new();

    loop {
        let trimmed = remaining.trim_end();
        if trimmed.len() != remaining.len() {
            remaining.truncate(trimmed.len());
        }

        let Some(last) = remaining.chars().last() else {
            break;
        };

        let (opening, delimiter) = match last {
            ')' => ('(', TokenDelimiter::Parentheses),
            ']' => ('[', TokenDelimiter::Brackets),
            _ => break,
        };

        let Some(open_index) = remaining.rfind(opening) else {
            break;
        };
        let value = remaining[open_index + 1..remaining.len() - 1].trim();
        tokens.push(RawToken {
            delimiter,
            value: value.to_string(),
        });
        remaining.truncate(open_index);
    }

    tokens.reverse();
    (remaining.trim_end().to_string(), tokens)
}

fn classify_token(state: &mut ParseState, token: &RawToken) {
    match token.delimiter {
        TokenDelimiter::Parentheses => classify_parenthesized_token(state, token.value.as_str()),
        TokenDelimiter::Brackets => classify_bracketed_token(state, token.value.as_str()),
    }
}

fn classify_parenthesized_token(state: &mut ParseState, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        state.unknown_tokens.push("()".to_string());
        return;
    }

    if let Some(disc) = parse_disc_marker_text(raw) {
        state.disc = Some(disc);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
        return;
    }

    if let Some(regions) = parse_region_token(raw) {
        add_unique_many(&mut state.regions, regions);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
        return;
    }

    if let Some(revision) = parse_revision_token(raw) {
        state.revision = Some(revision);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
        return;
    }

    if let Some(version) = parse_version_token(raw) {
        state.version = Some(version);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
        return;
    }

    if raw.eq_ignore_ascii_case("unl") || raw.eq_ignore_ascii_case("unlicensed") {
        state.dump_flags.unlicensed = true;
        state.recognized_tokens += 1;
        state.goodtools_tokens += 1;
        return;
    }

    if let Some(hardware) = parse_special_hardware(raw) {
        add_unique(&mut state.special_hardware, hardware);
        state.recognized_tokens += 1;
        state.no_intro_tokens += 1;
        return;
    }

    state.unknown_tokens.push(format!("({raw})"));
}

fn classify_bracketed_token(state: &mut ParseState, raw: &str) {
    let raw = raw.trim();
    if raw.is_empty() {
        state.unknown_tokens.push("[]".to_string());
        return;
    }

    let lower = raw.to_ascii_lowercase();

    if raw == "!" {
        state.dump_flags.verified_good_dump = true;
        recognize_dump_token(state, raw);
        return;
    }
    if raw == "C" {
        add_unique(&mut state.special_hardware, "Game Boy Color".to_string());
        recognize_dump_token(state, raw);
        return;
    }
    if raw == "S" {
        add_unique(&mut state.special_hardware, "Super Game Boy".to_string());
        recognize_dump_token(state, raw);
        return;
    }
    if raw == "BF" {
        add_unique(&mut state.special_hardware, "Bung Fix".to_string());
        recognize_dump_token(state, raw);
        return;
    }
    if lower == "!p" {
        state.dump_flags.pending_dump = true;
        recognize_dump_token(state, raw);
        return;
    }
    if lower == "unl" {
        state.dump_flags.unlicensed = true;
        recognize_dump_token(state, raw);
        return;
    }

    if raw.starts_with("T+") || raw.starts_with("T-") {
        let current = raw.starts_with("T+");
        let language = parse_translation_language(&raw[2..]);
        if let Some(language) = language.as_ref() {
            add_unique(&mut state.languages, language.clone());
        }
        state.dump_flags.translation = Some(TranslationFlag {
            raw: raw.to_string(),
            current,
            language,
        });
        recognize_dump_token(state, raw);
        return;
    }

    if let Some(count) = lower
        .strip_prefix('m')
        .filter(|suffix| !suffix.is_empty())
        .and_then(|suffix| suffix.parse::<u32>().ok())
    {
        state.dump_flags.multilingual_count = Some(count);
        recognize_dump_token(state, raw);
        return;
    }

    if raw.chars().all(|character| character.is_ascii_digit()) && raw.len() >= 3 {
        state.dump_flags.checksum = Some(raw.to_string());
        recognize_dump_token(state, raw);
        return;
    }

    if (lower.ends_with('k')
        && lower[..lower.len() - 1]
            .chars()
            .all(|character| character.is_ascii_digit() || character == '?'))
        || lower == "-"
        || lower.starts_with("zzz")
    {
        state.dump_flags.rom_size = Some(raw.to_string());
        recognize_dump_token(state, raw);
        return;
    }

    if raw == "c" {
        state.dump_flags.checksum_good = true;
        recognize_dump_token(state, raw);
        return;
    }
    if raw == "x" {
        state.dump_flags.checksum_bad = true;
        recognize_dump_token(state, raw);
        return;
    }

    if let Some(flag) = lower.chars().next() {
        match flag {
            'a' => {
                state.dump_flags.alternate = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            'b' => {
                state.dump_flags.bad_dump = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            'f' => {
                state.dump_flags.fixed = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            'h' => {
                state.dump_flags.hack = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            'o' => {
                state.dump_flags.overdump = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            't' => {
                state.dump_flags.trainer = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            'p' => {
                state.dump_flags.pirate = Some(raw.to_string());
                recognize_dump_token(state, raw);
            }
            _ => state.unknown_tokens.push(format!("[{raw}]")),
        }
    }
}

fn recognize_dump_token(state: &mut ParseState, raw: &str) {
    state.dump_flags.raw_tokens.push(format!("[{raw}]"));
    state.recognized_tokens += 1;
    state.goodtools_tokens += 1;
}

fn parse_region_token(raw: &str) -> Option<Vec<String>> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return None;
    }

    if normalized.contains(',') || normalized.contains('/') {
        let mut regions = Vec::new();
        for part in normalized.split([',', '/']) {
            let part = part.trim();
            let region = map_region_name_or_code(part)?;
            add_unique(&mut regions, region.to_string());
        }
        return (!regions.is_empty()).then_some(regions);
    }

    if let Some(region) = map_region_name_or_code(normalized) {
        return Some(vec![region.to_string()]);
    }

    let compact = normalized.to_ascii_uppercase();
    if compact.len() > 1
        && compact
            .chars()
            .all(|character| character.is_ascii_alphabetic())
    {
        let mut regions = Vec::new();
        for character in compact.chars() {
            let region = map_single_letter_region(character)?;
            add_unique(&mut regions, region.to_string());
        }
        return (!regions.is_empty()).then_some(regions);
    }

    None
}

fn map_region_name_or_code(raw: &str) -> Option<&'static str> {
    let key = raw.trim().to_ascii_lowercase();
    match key.as_str() {
        "u" | "usa" | "us" | "united states" | "united states of america" => Some("USA"),
        "e" | "eu" | "eur" | "europe" | "european union" | "pal" => Some("Europe"),
        "j" | "jp" | "jpn" | "japan" => Some("Japan"),
        "w" | "world" => Some("World"),
        "a" | "aus" | "australia" => Some("Australia"),
        "c" | "china" => Some("China"),
        "f" | "fr" | "fra" | "france" => Some("France"),
        "fc" | "french canadian" => Some("French Canadian"),
        "fn" | "finland" => Some("Finland"),
        "g" | "de" | "ger" | "germany" => Some("Germany"),
        "gr" | "greece" => Some("Greece"),
        "hk" | "hong kong" => Some("Hong Kong"),
        "h" | "holland" => Some("Holland"),
        "i" | "it" | "ita" | "italy" => Some("Italy"),
        "k" | "kor" | "korea" => Some("Korea"),
        "nl" | "netherlands" => Some("Netherlands"),
        "pd" | "public domain" => Some("Public Domain"),
        "s" | "sp" | "spa" | "spain" => Some("Spain"),
        "sw" | "sweden" => Some("Sweden"),
        "uk" | "england" | "united kingdom" => Some("United Kingdom"),
        "unk" | "unknown" => Some("Unknown"),
        "1" => Some("Japan/Korea"),
        "4" => Some("USA/Brazil NTSC"),
        "5" | "ntsc" => Some("NTSC"),
        "8" => Some("PAL"),
        "b" => Some("Non-USA"),
        _ => None,
    }
}

fn map_single_letter_region(character: char) -> Option<&'static str> {
    match character {
        'U' => Some("USA"),
        'E' => Some("Europe"),
        'J' => Some("Japan"),
        'W' => Some("World"),
        'A' => Some("Australia"),
        'C' => Some("China"),
        'F' => Some("France"),
        'G' => Some("Germany"),
        'H' => Some("Holland"),
        'I' => Some("Italy"),
        'K' => Some("Korea"),
        'S' => Some("Spain"),
        'B' => Some("Non-USA"),
        _ => None,
    }
}

fn parse_revision_token(raw: &str) -> Option<i32> {
    let compact: String = raw
        .chars()
        .filter(|character| !character.is_whitespace() && *character != '-' && *character != '_')
        .collect::<String>()
        .to_ascii_uppercase();

    if let Some(rest) = compact.strip_prefix("REV") {
        return parse_revision_value(rest);
    }
    if let Some(rest) = compact.strip_prefix("PRG") {
        return parse_revision_value(rest);
    }

    None
}

fn parse_revision_value(value: &str) -> Option<i32> {
    if value.is_empty() {
        return None;
    }

    if let Ok(number) = value.parse::<i32>() {
        return Some(number);
    }

    if value.len() == 1 {
        let character = value.chars().next()?;
        if character.is_ascii_alphabetic() {
            return Some((character as u8 - b'A' + 1).into());
        }
    }

    None
}

fn parse_version_token(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let compact = trimmed.to_ascii_lowercase();

    if let Some(version) = trimmed
        .strip_prefix('V')
        .or_else(|| trimmed.strip_prefix('v'))
        .filter(|version| {
            version
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        })
    {
        return Some(version.to_string());
    }

    match compact.as_str() {
        "alpha" => Some("Alpha".to_string()),
        "beta" => Some("Beta".to_string()),
        "prototype" | "proto" => Some("Prototype".to_string()),
        "pre-release" | "prerelease" => Some("Pre-Release".to_string()),
        "old" => Some("Old".to_string()),
        _ => None,
    }
}

fn parse_special_hardware(raw: &str) -> Option<String> {
    let key = raw.trim().to_ascii_lowercase();
    match key.as_str() {
        "j-cart" => Some("J-Cart".to_string()),
        "pc10" => Some("PlayChoice-10".to_string()),
        "vs" => Some("Nintendo VS.".to_string()),
        "bs" => Some("Broadcast Satellaview".to_string()),
        "st" => Some("Sufami Turbo".to_string()),
        "np" => Some("Nintendo Power".to_string()),
        "adam" => Some("Coleco ADAM".to_string()),
        _ => None,
    }
}

fn parse_translation_language(raw: &str) -> Option<String> {
    let lower = raw.to_ascii_lowercase();
    let language =
        if lower.starts_with("fre") || lower.starts_with("fra") || lower.starts_with("fr") {
            "French"
        } else if lower.starts_with("eng") || lower.starts_with("en") {
            "English"
        } else if lower.starts_with("ger") || lower.starts_with("de") {
            "German"
        } else if lower.starts_with("spa") || lower.starts_with("es") {
            "Spanish"
        } else if lower.starts_with("ita") || lower.starts_with("it") {
            "Italian"
        } else if lower.starts_with("por") || lower.starts_with("pt") {
            "Portuguese"
        } else if lower.starts_with("dut") || lower.starts_with("nl") {
            "Dutch"
        } else if lower.starts_with("rus") || lower.starts_with("ru") {
            "Russian"
        } else if lower.starts_with("jap") || lower.starts_with("jpn") || lower.starts_with("ja") {
            "Japanese"
        } else if lower.starts_with("chi") || lower.starts_with("zh") {
            "Chinese"
        } else if lower.starts_with("kor") || lower.starts_with("ko") {
            "Korean"
        } else {
            return None;
        };

    Some(language.to_string())
}

fn parse_disc_marker_text(raw: &str) -> Option<DiscMarker> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let lower = trimmed.to_ascii_lowercase();
    for (prefix, kind) in [
        ("disc", DiscMarkerKind::Disc),
        ("disk", DiscMarkerKind::Disk),
        ("cd", DiscMarkerKind::Cd),
        ("side", DiscMarkerKind::Side),
    ] {
        let Some(rest) = lower.strip_prefix(prefix) else {
            continue;
        };
        let rest_original = &trimmed[prefix.len()..];
        let rest = rest.trim_start_matches([' ', '-', '_']);
        let rest_original = rest_original.trim_start_matches([' ', '-', '_']);
        let (index, side, remainder) =
            parse_marker_index(rest, rest_original, matches!(kind, DiscMarkerKind::Side))?;
        let count = parse_marker_count(remainder);

        return Some(DiscMarker {
            raw: trimmed.to_string(),
            kind,
            index: Some(index),
            count,
            side,
        });
    }

    None
}

fn parse_marker_index<'a>(
    rest_lower: &'a str,
    rest_original: &'a str,
    prefer_side: bool,
) -> Option<(u32, Option<String>, &'a str)> {
    let mut chars = rest_lower.char_indices();
    let (_, first) = chars.next()?;

    if first.is_ascii_digit() {
        let end = rest_lower
            .char_indices()
            .take_while(|(_, character)| character.is_ascii_digit())
            .last()
            .map(|(index, character)| index + character.len_utf8())
            .unwrap_or(first.len_utf8());
        let value = rest_lower[..end].parse::<u32>().ok()?;
        if value == 0 {
            return None;
        }
        return Some((value, None, rest_lower[end..].trim_start()));
    }

    if first.is_ascii_alphabetic() {
        let letter = first.to_ascii_uppercase();
        if !(('A'..='D').contains(&letter)) {
            return None;
        }
        let end = first.len_utf8();
        let remainder = &rest_lower[end..];
        if remainder
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        {
            return None;
        }
        let original_letter = rest_original
            .chars()
            .next()
            .map(|character| character.to_ascii_uppercase().to_string())
            .unwrap_or_else(|| letter.to_string());
        let index = (letter as u8 - b'A' + 1).into();
        let side = prefer_side.then_some(original_letter);
        return Some((index, side, remainder.trim_start()));
    }

    None
}

fn parse_marker_count(remainder: &str) -> Option<u32> {
    let remainder = remainder.trim_start_matches([' ', '-', '_']);
    let rest = remainder
        .strip_prefix("of")?
        .trim_start_matches([' ', '-', '_']);
    let digits: String = rest
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok().filter(|count| *count > 0)
    }
}

fn strip_trailing_disc_marker(title: &str) -> Option<(String, DiscMarker)> {
    for (index, _) in title.char_indices().rev() {
        let suffix = &title[index..];
        let trimmed_suffix = suffix.trim_start_matches([' ', '-', '_']);
        if trimmed_suffix.len() == suffix.len() {
            continue;
        }

        if let Some(disc) = parse_disc_marker_text(trimmed_suffix) {
            let stripped = clean_title(&title[..index]);
            if !stripped.is_empty() {
                return Some((stripped, disc));
            }
        }
    }

    None
}

fn clean_title(raw: &str) -> String {
    let title = raw
        .replace('_', " ")
        .trim_matches(|character: char| {
            character.is_whitespace() || character == '-' || character == '_'
        })
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if title.is_empty() {
        "Untitled ROM".to_string()
    } else {
        title
    }
}

fn confidence(state: &ParseState, split_archive: Option<&SplitArchiveVolume>) -> ParseConfidence {
    if state.dump_flags.verified_good_dump
        || !state.regions.is_empty()
        || state.revision.is_some()
        || state.recognized_tokens >= 2
    {
        ParseConfidence::High
    } else if state.recognized_tokens > 0 || split_archive.is_some() {
        ParseConfidence::Medium
    } else {
        ParseConfidence::Low
    }
}

fn add_unique(values: &mut Vec<String>, value: String) {
    if !values
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&value))
    {
        values.push(value);
    }
}

fn add_unique_many(values: &mut Vec<String>, incoming: Vec<String>) {
    for value in incoming {
        add_unique(values, value);
    }
}

#[cfg(test)]
mod tests {
    use super::{detect_split_archive, parse, parse_disc_marker};
    use crate::domain::ingest::{DiscMarkerKind, ParseConfidence};

    #[test]
    fn parses_goodtools_and_no_intro_examples() {
        let parsed = parse("Zoop (U) [!].gen");
        assert_eq!(parsed.base_name, "Zoop (U) [!]");
        assert_eq!(parsed.clean_title, "Zoop");
        assert_eq!(parsed.regions, vec!["USA"]);
        assert!(parsed.dump_flags.verified_good_dump);
        assert_eq!(parsed.naming_convention_guess.as_deref(), Some("goodtools"));
        assert_eq!(parsed.confidence, ParseConfidence::High);

        let parsed = parse("Parasol Stars - The Story of Bubble Bobble 3 (E) [!].nes");
        assert_eq!(
            parsed.clean_title,
            "Parasol Stars - The Story of Bubble Bobble 3"
        );
        assert_eq!(parsed.regions, vec!["Europe"]);
        assert!(parsed.dump_flags.verified_good_dump);

        let parsed = parse("Bahamut Lagoon (J) [T+FreBeta4_Terminus].smc");
        assert_eq!(parsed.clean_title, "Bahamut Lagoon");
        assert_eq!(parsed.regions, vec!["Japan"]);
        assert_eq!(parsed.languages, vec!["French"]);
        assert!(parsed.dump_flags.translation.as_ref().unwrap().current);

        let parsed = parse("Addams Family, The (Beta) [b] [UI]");
        assert_eq!(parsed.clean_title, "Addams Family, The");
        assert_eq!(parsed.version.as_deref(), Some("Beta"));
        assert_eq!(parsed.dump_flags.bad_dump.as_deref(), Some("b"));
        assert_eq!(parsed.unknown_tokens, vec!["[UI]"]);

        let parsed = parse("Micro Machines - Turbo Tournament '96 (V1.1) (E) (J-Cart) [h1C].gen");
        assert_eq!(parsed.clean_title, "Micro Machines - Turbo Tournament '96");
        assert_eq!(parsed.version.as_deref(), Some("1.1"));
        assert_eq!(parsed.regions, vec!["Europe"]);
        assert_eq!(parsed.special_hardware, vec!["J-Cart"]);
        assert_eq!(parsed.dump_flags.hack.as_deref(), Some("h1C"));

        let parsed = parse("Alien 3 (UE) (REV03) [h1C][o1].gen");
        assert_eq!(parsed.clean_title, "Alien 3");
        assert_eq!(parsed.regions, vec!["USA", "Europe"]);
        assert_eq!(parsed.revision, Some(3));
        assert_eq!(parsed.dump_flags.hack.as_deref(), Some("h1C"));
        assert_eq!(parsed.dump_flags.overdump.as_deref(), Some("o1"));

        let parsed = parse("Great Volleyball (USA, Europe).zip");
        assert_eq!(parsed.clean_title, "Great Volleyball");
        assert_eq!(parsed.regions, vec!["USA", "Europe"]);

        let parsed = parse("Action 52 (Active Enterprises) (REVA) [!].nes");
        assert_eq!(parsed.clean_title, "Action 52");
        assert_eq!(parsed.revision, Some(1));
        assert_eq!(parsed.unknown_tokens, vec!["(Active Enterprises)"]);
        assert!(parsed.dump_flags.verified_good_dump);

        let parsed = parse("Untouchables, The (U) (PRG1) [!].nes");
        assert_eq!(parsed.clean_title, "Untouchables, The");
        assert_eq!(parsed.regions, vec!["USA"]);
        assert_eq!(parsed.revision, Some(1));
        assert!(parsed.dump_flags.verified_good_dump);
    }

    #[test]
    fn parses_disc_markers_from_tokens_and_suffixes() {
        let parsed = parse("Final Fantasy VII (USA) (Disc 1 of 3).chd");
        assert_eq!(parsed.clean_title, "Final Fantasy VII");
        assert_eq!(parsed.regions, vec!["USA"]);
        let disc = parsed.disc.unwrap();
        assert_eq!(disc.kind, DiscMarkerKind::Disc);
        assert_eq!(disc.index, Some(1));
        assert_eq!(disc.count, Some(3));

        let parsed = parse("Family Games Compendium (Disc 3).cue");
        assert_eq!(parsed.clean_title, "Family Games Compendium");
        assert_eq!(parsed.disc.unwrap().index, Some(3));

        let parsed = parse("Game - CD2.iso");
        assert_eq!(parsed.clean_title, "Game");
        let disc = parsed.disc.unwrap();
        assert_eq!(disc.kind, DiscMarkerKind::Cd);
        assert_eq!(disc.index, Some(2));

        let parsed = parse("Dragon Fantasy VII - Disk1.chd");
        assert_eq!(parsed.clean_title, "Dragon Fantasy VII");
        assert_eq!(parsed.disc.unwrap().index, Some(1));

        let parsed = parse("Game Side A.bin");
        assert_eq!(parsed.clean_title, "Game");
        let disc = parsed.disc.unwrap();
        assert_eq!(disc.kind, DiscMarkerKind::Side);
        assert_eq!(disc.index, Some(1));
        assert_eq!(disc.side.as_deref(), Some("A"));

        assert_eq!(parse_disc_marker("CD2").unwrap().index, Some(2));
        assert!(parse_disc_marker("CD Player").is_none());
    }

    #[test]
    fn parses_multi_part_research_examples() {
        let parsed = parse("Ridge Racer (USA).cue");
        assert_eq!(parsed.clean_title, "Ridge Racer");
        assert_eq!(parsed.regions, vec!["USA"]);

        let parsed = parse("Ridge Racer (USA) (Track 01).bin");
        assert_eq!(parsed.clean_title, "Ridge Racer");
        assert_eq!(parsed.regions, vec!["USA"]);
        assert_eq!(parsed.unknown_tokens, vec!["(Track 01)"]);

        let parsed = parse("Metal Gear Solid (USA) (Disc 2).cue");
        assert_eq!(parsed.clean_title, "Metal Gear Solid");
        assert_eq!(parsed.regions, vec!["USA"]);
        assert_eq!(parsed.disc.unwrap().index, Some(2));

        let parsed = parse("Fear Effect (Disc 1) (USA).chd");
        assert_eq!(parsed.clean_title, "Fear Effect");
        assert_eq!(parsed.regions, vec!["USA"]);
        assert_eq!(parsed.disc.unwrap().index, Some(1));

        let parsed = parse("Final Fantasy VII (USA).m3u");
        assert_eq!(parsed.clean_title, "Final Fantasy VII");
        assert_eq!(parsed.regions, vec!["USA"]);
    }

    #[test]
    fn detects_split_archive_volume_patterns() {
        let volume = detect_split_archive("Large Game.7z.001").unwrap();
        assert_eq!(volume.archive_format, "7z");
        assert_eq!(volume.set_name, "Large Game");
        assert_eq!(volume.volume_index, 1);
        assert!(volume.is_first_volume);

        let volume = detect_split_archive("Large Game.7z.002").unwrap();
        assert_eq!(volume.volume_index, 2);
        assert!(!volume.is_first_volume);

        let volume = detect_split_archive("volname.part001.rar").unwrap();
        assert_eq!(volume.archive_format, "rar");
        assert_eq!(volume.set_name, "volname");
        assert_eq!(volume.volume_index, 1);

        let volume = detect_split_archive("volname.r00").unwrap();
        assert_eq!(volume.archive_format, "rar");
        assert_eq!(volume.set_name, "volname");
        assert_eq!(volume.volume_index, 2);

        let volume = detect_split_archive("volname.z01").unwrap();
        assert_eq!(volume.archive_format, "zip");
        assert_eq!(volume.set_name, "volname");
        assert_eq!(volume.volume_index, 1);

        assert!(detect_split_archive("single-file.rar").is_none());
    }
}
