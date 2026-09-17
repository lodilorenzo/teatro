use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ManifestParseError {
    #[error("manifest path is empty")]
    EmptyPath,
    #[error("manifest path is absolute")]
    AbsolutePath,
    #[error("manifest path contains parent traversal")]
    ParentTraversal,
    #[error("manifest path contains an unsafe character")]
    UnsafeCharacter,
    #[error("GDI track count is invalid")]
    InvalidGdiTrackCount,
    #[error("GDI track row is invalid")]
    InvalidGdiTrackRow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GdiTrackReference {
    pub track_number: u32,
    pub file_name: String,
}

pub fn parse_m3u_lines(contents: &str) -> Result<Vec<String>, ManifestParseError> {
    contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(validate_relative_manifest_path)
        .collect()
}

pub fn parse_cue_file_references(contents: &str) -> Result<Vec<String>, ManifestParseError> {
    let mut references = Vec::new();

    for line in contents.lines() {
        let line = line.trim_start();
        if !line
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("FILE"))
        {
            continue;
        }

        let rest = line[4..].trim_start();
        let Some(file_name) = parse_cue_file_operand(rest) else {
            continue;
        };
        references.push(validate_relative_manifest_path(file_name)?);
    }

    Ok(references)
}

pub fn parse_gdi_track_references(
    contents: &str,
) -> Result<Vec<GdiTrackReference>, ManifestParseError> {
    let mut lines = contents
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let track_count = lines
        .next()
        .ok_or(ManifestParseError::InvalidGdiTrackCount)?
        .parse::<usize>()
        .map_err(|_| ManifestParseError::InvalidGdiTrackCount)?;

    let mut tracks = Vec::new();
    for line in lines.take(track_count) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            return Err(ManifestParseError::InvalidGdiTrackRow);
        }

        let track_number = parts[0]
            .parse::<u32>()
            .map_err(|_| ManifestParseError::InvalidGdiTrackRow)?;
        let file_name = validate_relative_manifest_path(parts[4])?;
        tracks.push(GdiTrackReference {
            track_number,
            file_name,
        });
    }
    if tracks.len() != track_count {
        return Err(ManifestParseError::InvalidGdiTrackCount);
    }

    Ok(tracks)
}

fn parse_cue_file_operand(rest: &str) -> Option<&str> {
    if let Some(after_quote) = rest.strip_prefix('"') {
        let end = after_quote.find('"')?;
        return Some(&after_quote[..end]);
    }

    rest.split_whitespace().next()
}

fn validate_relative_manifest_path(path: &str) -> Result<String, ManifestParseError> {
    let path = path.trim();
    if path.is_empty() {
        return Err(ManifestParseError::EmptyPath);
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        return Err(ManifestParseError::AbsolutePath);
    }
    if path
        .chars()
        .any(|character| character == '\0' || character.is_control())
    {
        return Err(ManifestParseError::UnsafeCharacter);
    }
    if path
        .split(['/', '\\'])
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(ManifestParseError::ParentTraversal);
    }

    Ok(path.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        ManifestParseError, parse_cue_file_references, parse_gdi_track_references, parse_m3u_lines,
    };

    #[test]
    fn parses_m3u_lines_and_rejects_escaping_paths() {
        let lines = parse_m3u_lines(
            r#"
# comment
Metal Gear Solid (USA) (Disc 1).cue
Metal Gear Solid (USA) (Disc 2).cue
"#,
        )
        .unwrap();
        assert_eq!(
            lines,
            vec![
                "Metal Gear Solid (USA) (Disc 1).cue",
                "Metal Gear Solid (USA) (Disc 2).cue"
            ]
        );

        assert_eq!(
            parse_m3u_lines("../secret.bin").unwrap_err(),
            ManifestParseError::ParentTraversal
        );
    }

    #[test]
    fn parses_cue_file_references_and_rejects_absolute_paths() {
        let files = parse_cue_file_references(
            r#"
FILE "Game (Track 01).bin" BINARY
  TRACK 01 MODE2/2352
    INDEX 01 00:00:00
FILE "Game (Track 02).bin" BINARY
  TRACK 02 AUDIO
"#,
        )
        .unwrap();
        assert_eq!(files, vec!["Game (Track 01).bin", "Game (Track 02).bin"]);

        assert_eq!(
            parse_cue_file_references("FILE \"/tmp/track.bin\" BINARY").unwrap_err(),
            ManifestParseError::AbsolutePath
        );
    }

    #[test]
    fn parses_gdi_track_references_and_rejects_parent_paths() {
        let tracks = parse_gdi_track_references(
            r#"
3
1 0 4 2352 track01.bin 0
2 600 0 2352 track02.raw 0
3 45000 4 2352 track03.bin 0
"#,
        )
        .unwrap();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[1].track_number, 2);
        assert_eq!(tracks[1].file_name, "track02.raw");

        assert_eq!(
            parse_gdi_track_references("1\n1 0 4 2352 ../track01.bin 0\n").unwrap_err(),
            ManifestParseError::ParentTraversal
        );
        assert_eq!(
            parse_gdi_track_references("2\n1 0 4 2352 track01.bin 0\n").unwrap_err(),
            ManifestParseError::InvalidGdiTrackCount
        );
    }
}
