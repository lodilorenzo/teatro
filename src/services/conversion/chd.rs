//! Experimental, lossless CD/DVD imports through pinned typed CHD APIs.
use super::{
    InputFile, OutputFile,
    rvz::{Progress, supervise},
    wua::safe_path,
};
use crate::{
    domain::rom::{DependencyKind, FileRole},
    services::library,
};
use rom_converto_lib::{
    disc::{
        chd::{self, ChdOptions, DiscMode},
        cue::{
            CueParser,
            merge::merge_bin,
            models::{CueSheet, FileType, TrackType},
        },
    },
    util::{
        CancelToken, NoProgress,
        iso9660::{self, DiscKind, SectorSource},
    },
};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

pub(crate) const PLATFORMS: &[(&str, &str)] = &[
    ("psx", "DuckStation"),
    ("ps2", "PCSX2"),
    ("psp", "PPSSPP"),
    ("saturn", "Beetle Saturn"),
    ("saturnjp", "Beetle Saturn"),
    ("segacd", "Genesis Plus GX"),
    ("megacd", "Genesis Plus GX"),
    ("megacdjp", "Genesis Plus GX"),
    ("pcenginecd", "Beetle PCE Fast"),
    ("tg-cd", "Beetle PCE Fast"),
    ("neogeocd", "NeoCD"),
];
pub(super) struct Inventory {
    pub bytes: u64,
    discs: Vec<Disc>,
    companions: Vec<OutputFile>,
    playlist: Option<(String, String)>,
}
struct Disc {
    path: PathBuf,
    name: String,
    bytes: u64,
    sheet: Option<CueSheet>,
    sources: Vec<PathBuf>,
    mode: DiscMode,
    psp: bool,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::other(message)
}
fn extension(path: &str) -> String {
    Path::new(path)
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}

pub(super) fn inspect(root: &Path, inputs: &[InputFile], platform: &str) -> io::Result<Inventory> {
    if !PLATFORMS.iter().any(|&(slug, _)| slug == platform) {
        return Err(invalid("Unsupported CHD platform"));
    }
    let mut files = Vec::new();
    let mut names = HashSet::new();
    for (index, input) in inputs.iter().enumerate() {
        safe_path(&input.path)?;
        // Reuse Teatro's flat CUE/M3U planner and its dependency closure.
        if input.path.contains('/')
            || input.path.trim() != input.path
            || !names.insert(input.path.to_ascii_lowercase())
        {
            return Err(invalid("CHD imports require unique flat filenames"));
        }
        let meta = fs::symlink_metadata(root.join(&input.path))?;
        if !meta.is_file() || meta.len() != input.bytes || input.bytes == 0 {
            return Err(invalid("Invalid CHD input file"));
        }
        let ext = extension(&input.path);
        if !matches!(ext.as_str(), "cue" | "bin" | "iso" | "img" | "m3u" | "sbi")
            || (platform == "psp" && matches!(ext.as_str(), "cue" | "bin" | "sbi"))
            || (ext == "iso" && !matches!(platform, "psx" | "ps2" | "psp"))
        {
            return Err(invalid("Unsupported CHD input format"));
        }
        let manifest_contents = if matches!(ext.as_str(), "cue" | "m3u") {
            if input.bytes > 1024 * 1024 {
                return Err(invalid("CHD descriptor exceeds 1 MiB"));
            }
            Some(fs::read_to_string(root.join(&input.path))?)
        } else {
            None
        };
        files.push(library::PlanInputFile {
            index,
            original_file_name: input.path.clone(),
            file_size_bytes: Some(input.bytes),
            staged_path: None,
            manifest_contents,
        });
    }
    let plan = library::build_ingest_plan(0, platform, files, None);
    if !plan.errors.is_empty() || plan.roms.len() != 1 {
        return Err(invalid(
            "Select one complete game and all its discs and tracks",
        ));
    }
    let rom = &plan.roms[0];
    let mut discs = Vec::new();
    let mut companions = Vec::new();
    let mut replacements = HashMap::new();
    for input in inputs {
        let ext = extension(&input.path);
        let path = root.join(&input.path);
        if ext == "sbi" {
            if !rom.files.iter().any(|file| {
                file.original_file_name == input.path && file.role == FileRole::MetadataSidecar
            }) {
                return Err(invalid("SBI companion has no matching disc"));
            }
            companions.push(OutputFile {
                path,
                name: input.path.clone(),
                bytes: input.bytes,
            });
            continue;
        }
        if !matches!(ext.as_str(), "cue" | "iso" | "img")
            || rom
                .files
                .iter()
                .any(|file| file.original_file_name == input.path && file.role == FileRole::Track)
        {
            continue;
        }
        let name = Path::new(&input.path)
            .with_extension("chd")
            .to_string_lossy()
            .into_owned();
        if replacements
            .values()
            .any(|value: &String| value.eq_ignore_ascii_case(&name))
        {
            return Err(invalid("CHD output name collision"));
        }
        replacements.insert(input.path.clone(), name.clone());
        let (sheet, sources, bytes, mode) = if ext == "cue" {
            let contents = fs::read_to_string(&path)?;
            // The pinned writer ignores INDEX 00, POSTGAP and track flags. Never drop them.
            if contents.lines().any(|line| {
                !matches!(
                    line.split_whitespace().next(),
                    None | Some("REM" | "FILE" | "TRACK" | "INDEX" | "PREGAP")
                )
            }) {
                return Err(invalid("Unsupported CUE directive"));
            }
            let sheet = CueParser::new(&path)
                .parse_bytes(contents.as_bytes())
                .map_err(io::Error::other)?;
            let (sources, bytes) = validate_cue(root, &sheet, &names)?;
            validate_cd_platform(&sources[0], sheet.tracks[0].track_type, platform)?;
            (Some(sheet), sources, bytes, DiscMode::Cd)
        } else if raw_img_mode(&path)?.is_some() {
            if platform == "psp" {
                return Err(invalid("PSP requires a plain 2048-byte image"));
            }
            let track_type = validate_raw_img(&path, input.bytes)?;
            validate_cd_platform(&path, track_type, platform)?;
            if matches!(platform, "psx" | "ps2") {
                let mut source = RawSectors {
                    file: File::open(&path)?,
                    mode: track_type,
                    sectors: input.bytes / 2352,
                };
                validate_volume(&mut source)?;
            }
            let contents = format!(
                "FILE \"{}\" BINARY\n TRACK 01 {}\n INDEX 01 00:00:00\n",
                input.path,
                track_type.cue_string()
            );
            let sheet = CueParser::new(&path)
                .parse_bytes(contents.as_bytes())
                .map_err(io::Error::other)?;
            (Some(sheet), vec![path.clone()], input.bytes, DiscMode::Cd)
        } else {
            if !matches!(platform, "psx" | "ps2" | "psp") || input.bytes % 2048 != 0 {
                return Err(invalid("ISO is not sector aligned"));
            }
            validate_iso(&path)?;
            let kind = iso9660::detect_disc_kind(&path)?;
            if !matches!(
                (platform, kind),
                ("psx", DiscKind::Ps1)
                    | ("ps2", DiscKind::Ps2Cd | DiscKind::Ps2Dvd)
                    | ("psp", DiscKind::Psp)
            ) {
                return Err(invalid(
                    "ISO does not match the selected platform and medium",
                ));
            }
            let mode = if matches!(kind, DiscKind::Ps1 | DiscKind::Ps2Cd) {
                if input.bytes / 2048 > 450_000 {
                    return Err(invalid("CD exceeds 100 minutes"));
                }
                DiscMode::Cd
            } else {
                DiscMode::Dvd
            };
            (None, vec![path.clone()], input.bytes, mode)
        };
        discs.push(Disc {
            path,
            name,
            bytes,
            sheet,
            sources,
            mode,
            psp: platform == "psp",
        });
    }
    if discs.is_empty() {
        return Err(invalid("No compressible disc"));
    }
    let playlist = rom
        .files
        .iter()
        .find(|file| file.role == FileRole::LaunchManifest)
        .map(|file| {
            let mut entries: Vec<_> = rom
                .dependencies
                .iter()
                .filter(|dep| {
                    dep.parent_file_key == file.key
                        && dep.dependency_kind == DependencyKind::PlaylistEntry
                })
                .collect();
            entries.sort_by_key(|dep| dep.sort_index);
            let lines: io::Result<Vec<_>> = entries
                .iter()
                .map(|dep| {
                    let source = rom
                        .files
                        .iter()
                        .find(|child| child.key == dep.child_file_key)
                        .ok_or_else(|| invalid("Missing playlist disc"))?;
                    replacements
                        .get(&source.original_file_name)
                        .cloned()
                        .ok_or_else(|| invalid("Unsupported playlist disc"))
                })
                .collect();
            Ok::<_, io::Error>((file.original_file_name.clone(), lines?.join("\n") + "\n"))
        })
        .transpose()?;
    if discs.len() > 1 && playlist.is_none() {
        return Err(invalid("Multi-disc input needs a complete disc set or M3U"));
    }
    Ok(Inventory {
        bytes: inputs.iter().map(|input| input.bytes).sum(),
        discs,
        companions,
        playlist,
    })
}

fn validate_iso(path: &Path) -> io::Result<()> {
    validate_volume(&mut &File::open(path)?)
}

fn validate_volume(source: &mut impl SectorSource) -> io::Result<()> {
    let bytes = source.total_sectors() * 2048;
    let mut pvd = [0; 2048];
    source.read_sector(16, &mut pvd)?;
    let sectors = u32::from_le_bytes(pvd[80..84].try_into().unwrap());
    let root = u32::from_le_bytes(pvd[158..162].try_into().unwrap());
    let root_size = u32::from_le_bytes(pvd[166..170].try_into().unwrap());
    if pvd[..7] != [1, b'C', b'D', b'0', b'0', b'1', 1]
        || sectors == 0
        || u64::from(sectors) * 2048 > bytes
        || pvd[84..88] != sectors.to_be_bytes()
        || pvd[128..132] != [0, 8, 8, 0]
        || pvd[156] != 34
        || root < 17
        || root_size == 0
        || u64::from(root) * 2048 + u64::from(root_size) > bytes
    {
        return Err(invalid("ISO volume geometry is invalid or truncated"));
    }
    Ok(())
}

fn raw_img_mode(path: &Path) -> io::Result<Option<TrackType>> {
    let mut header = [0; 16];
    File::open(path)?.read_exact(&mut header)?;
    Ok(
        if header[..12] == [0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0] {
            match header[15] {
                1 => Some(TrackType::Mode1_2352),
                2 => Some(TrackType::Mode2_2352),
                _ => return Err(invalid("Unsupported raw IMG sector mode; supply its CUE")),
            }
        } else {
            None
        },
    )
}

// ponytail: header-validated raw images are one data track; use CUE for extra track indices.
fn validate_raw_img(path: &Path, bytes: u64) -> io::Result<TrackType> {
    if !bytes.is_multiple_of(2352) || bytes / 2352 > 450_000 {
        return Err(invalid(
            "IMG is truncated or exceeds 100 minutes; supply its CUE",
        ));
    }
    let mode = raw_img_mode(path)?.ok_or_else(|| invalid("IMG needs its CUE"))?;
    let mut reader = BufReader::with_capacity(128 * 1024, File::open(path)?);
    let mut sector = [0; 2352];
    for _ in 0..bytes / 2352 {
        reader.read_exact(&mut sector)?;
        if sector[..12] != [0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]
            || sector[15]
                != if matches!(mode, TrackType::Mode1_2352) {
                    1
                } else {
                    2
                }
        {
            return Err(invalid(
                "IMG contains audio or mixed sector modes; supply its CUE",
            ));
        }
    }
    Ok(mode)
}

fn validate_cue(
    root: &Path,
    sheet: &CueSheet,
    names: &HashSet<String>,
) -> io::Result<(Vec<PathBuf>, u64)> {
    if sheet.files.is_empty() || sheet.tracks.is_empty() || sheet.tracks.len() > 99 {
        return Err(invalid("Invalid CUE track count"));
    }
    let mut sources = Vec::new();
    let mut frames = Vec::new();
    let mut total = 0;
    let mut seen = HashSet::new();
    for file in &sheet.files {
        safe_path(&file.filename)?;
        if file.filename.contains('/')
            || !matches!(file.file_type, FileType::Binary)
            || !matches!(extension(&file.filename).as_str(), "bin" | "img")
            || !names.contains(&file.filename.to_ascii_lowercase())
            || !seen.insert(&file.filename)
        {
            return Err(invalid(
                "CUE requires uploaded, unique BINARY BIN/IMG tracks",
            ));
        }
        let path = root.join(&file.filename);
        let size = fs::metadata(&path)?.len();
        if size == 0 || size % 2352 != 0 {
            return Err(invalid("BIN is truncated or not a raw 2352-byte track"));
        }
        total += size;
        frames.push(size / 2352);
        sources.push(path);
    }
    if total / 2352 > 450_000 {
        return Err(invalid("CD exceeds 100 minutes"));
    }
    let mut previous = None;
    for (at, track) in sheet.tracks.iter().enumerate() {
        let [index] = track.indices.as_slice() else {
            return Err(invalid("Only INDEX 01 CUE tracks are supported"));
        };
        if track.number as usize != at + 1
            || track.file_index >= frames.len()
            || !matches!(
                track.track_type,
                TrackType::Audio | TrackType::Mode1_2352 | TrackType::Mode2_2352
            )
            || index.number != 1
            || index.position.seconds >= 60
            || index.position.frames >= 75
            || u64::from(index.position.to_lba()) >= frames[track.file_index]
            || track.postgap.is_some()
            || track
                .pregap
                .is_some_and(|gap| gap.seconds >= 60 || gap.frames >= 75)
        {
            return Err(invalid("Unsupported CUE track geometry"));
        }
        let current = (track.file_index, index.position.to_lba());
        if previous.is_none_or(|prev: (usize, u32)| current.0 != prev.0) && current.1 != 0
            || previous.is_some_and(|prev| current <= prev || current.0 > prev.0 + 1)
            || (previous.is_none() && current.0 != 0)
        {
            return Err(invalid("CUE tracks must cover their BIN files in order"));
        }
        previous = Some(current);
    }
    if previous.unwrap().0 + 1 != sources.len() {
        return Err(invalid("Unused CUE BIN file"));
    }
    Ok((sources, total))
}

struct RawSectors {
    file: File,
    mode: TrackType,
    sectors: u64,
}
impl SectorSource for RawSectors {
    fn total_sectors(&self) -> u64 {
        self.sectors
    }
    fn read_sector(&mut self, lba: u32, buf: &mut [u8; 2048]) -> io::Result<()> {
        self.file.seek(SeekFrom::Start(u64::from(lba) * 2352))?;
        let mut raw = [0; 2352];
        self.file.read_exact(&mut raw)?;
        let offset = if matches!(self.mode, TrackType::Mode1_2352) {
            16
        } else {
            24
        };
        if raw[..12] != [0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]
            || raw[15] != if offset == 16 { 1 } else { 2 }
        {
            return Err(invalid("Invalid raw data sector"));
        }
        buf.copy_from_slice(&raw[offset..offset + 2048]);
        Ok(())
    }
}
fn validate_cd_platform(path: &Path, mode: TrackType, platform: &str) -> io::Result<()> {
    if matches!(mode, TrackType::Audio) {
        return Err(invalid("The first track must identify a data disc"));
    }
    let file = File::open(path)?;
    let mut source = RawSectors {
        sectors: file.metadata()?.len() / 2352,
        file,
        mode,
    };
    let mut head = [0; 2048];
    source.read_sector(0, &mut head)?;
    let kind = iso9660::read_volume(&mut source)?.map(|volume| volume.kind);
    let identified = if head.starts_with(b"SEGA SEGASATURN") {
        Some("saturn")
    } else if head.starts_with(b"SEGADISCSYSTEM") || head.starts_with(b"SEGA SEGADISCSYSTEM") {
        Some("segacd")
    } else {
        match kind {
            Some(DiscKind::Ps1) => Some("psx"),
            Some(DiscKind::Ps2Cd) => Some("ps2"),
            Some(DiscKind::Psp) => Some("psp"),
            _ => None,
        }
    };
    let family = match platform {
        "saturnjp" => "saturn",
        "megacd" | "megacdjp" => "segacd",
        _ => platform,
    };
    if identified.is_some_and(|slug| slug != family)
        || (matches!(family, "psx" | "ps2" | "saturn" | "segacd") && identified != Some(family))
    {
        return Err(invalid("CD identity does not match the selected platform"));
    }
    // PCE/Neo Geo CD lack a reliable probe here. Keep the explicit platform choice,
    // but validate the complete raw track layout and reject identified foreign discs.
    Ok(())
}

pub(super) async fn pack(
    inventory: Inventory,
    output: &Path,
    limit: u64,
    margin: u64,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(u64, u64) + Send + Sync,
    phase: impl Fn(&'static str),
) -> io::Result<Vec<OutputFile>> {
    let headroom = margin.saturating_add(8 * 1024 * 1024);
    // Cooked CD frames grow from 2048 to 2448, plus padding, maps and metadata.
    let bound = |disc: &Disc| {
        disc.bytes
            .saturating_add(disc.bytes / 4)
            .saturating_add(1024 * 1024)
    };
    let output_bound: u64 = inventory.discs.iter().map(bound).sum::<u64>()
        + inventory
            .companions
            .iter()
            .map(|file| file.bytes)
            .sum::<u64>()
        + inventory
            .playlist
            .as_ref()
            .map_or(0, |(_, text)| text.len() as u64);
    let scratch = inventory
        .discs
        .iter()
        .map(|disc| {
            disc.bytes
                * if disc
                    .sheet
                    .as_ref()
                    .is_some_and(|sheet| sheet.files.len() > 1)
                {
                    2
                } else {
                    1
                }
        })
        .max()
        .unwrap_or(0);
    if output_bound > limit
        || fs2::available_space(output.parent().unwrap())?
            < output_bound
                .saturating_add(scratch)
                .saturating_add(headroom)
    {
        return Err(invalid("Not enough space or output allowance for CHD"));
    }
    let dir = output.parent().unwrap().join("chd-output");
    fs::create_dir(&dir)?;
    let token = CancelToken::new();
    let mut outputs = Vec::new();
    let total: u64 = inventory.discs.iter().map(|disc| disc.bytes).sum();
    let mut done = 0;
    phase("compressing_and_verifying");
    for (at, disc) in inventory.discs.into_iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(invalid("Cancelled"));
        }
        let target = dir.join(&disc.name);
        let merged = dir.join(format!("merged-{at}.cue"));
        let standalone = dir.join(format!("standalone-{at}"));
        let input = if let Some(sheet) = disc.sheet.as_ref()
            && extension(&disc.path.to_string_lossy()) != "cue"
        {
            // A private hard link avoids copying the raw image or inventing audio tracks.
            fs::create_dir(&standalone)?;
            fs::hard_link(&disc.path, standalone.join("disc.bin"))?;
            let cue = standalone.join("disc.cue");
            fs::write(
                &cue,
                format!(
                    "FILE \"disc.bin\" BINARY\n TRACK 01 {}\n INDEX 01 00:00:00\n",
                    sheet.tracks[0].track_type.cue_string()
                ),
            )?;
            cue
        } else if disc
            .sheet
            .as_ref()
            .is_some_and(|sheet| sheet.files.len() > 1)
        {
            supervise(
                merge_bin(
                    &NoProgress,
                    disc.path.clone(),
                    merged.clone(),
                    false,
                    token.clone(),
                ),
                &target,
                &cancel,
                &token,
                headroom,
            )
            .await?;
            merged.clone()
        } else {
            disc.path.clone()
        };
        let reporter = Progress {
            report: |current: u64, _: u64| progress(done + current.min(disc.bytes), total),
            current: AtomicU64::new(0),
            total: AtomicU64::new(disc.bytes),
        };
        // chdman-compatible default codecs, maximum per-codec levels (LZMA/zlib cap at 9).
        let options = ChdOptions {
            level: Some(22),
            hunk_size: disc.psp.then_some(2048),
            ..Default::default()
        };
        supervise(
            chd::convert_disc_to_chd(
                &reporter,
                input.clone(),
                target.clone(),
                Some(disc.mode),
                options,
                token.clone(),
            ),
            &target,
            &cancel,
            &token,
            headroom,
        )
        .await?;
        let bytes = fs::metadata(&target)?.len();
        if bytes > bound(&disc) || cancel.load(Ordering::Relaxed) {
            return Err(invalid("CHD output limit reached or cancelled"));
        }
        File::options().write(true).open(&target)?.sync_all()?;
        supervise(
            chd::verify_chd(&NoProgress, target.clone(), None, false, token.clone()),
            &target,
            &cancel,
            &token,
            headroom,
        )
        .await?;
        let restored = dir.join(if matches!(disc.mode, DiscMode::Cd) {
            "verified.cue"
        } else {
            "verified.iso"
        });
        supervise(
            chd::extract_from_chd(
                &NoProgress,
                target.clone(),
                restored.clone(),
                None,
                token.clone(),
            ),
            &target,
            &cancel,
            &token,
            headroom,
        )
        .await?;
        if matches!(disc.mode, DiscMode::Cd) {
            let expected = if disc.sheet.is_some() {
                CueParser::new(&input)
                    .parse_bytes(&fs::read(&input)?)
                    .map_err(io::Error::other)?
            } else {
                CueParser::new(&input)
                    .parse_bytes(
                        b"FILE \"disc.bin\" BINARY\n TRACK 01 MODE1/2048\n INDEX 01 00:00:00\n",
                    )
                    .map_err(io::Error::other)?
            };
            let decoded = CueParser::new(&restored)
                .parse_bytes(&fs::read(&restored)?)
                .map_err(io::Error::other)?;
            if expected.tracks.len() != decoded.tracks.len()
                || expected.tracks.iter().zip(&decoded.tracks).any(|(a, b)| {
                    a.number != b.number
                        || a.track_type.cue_string() != b.track_type.cue_string()
                        || a.primary_index_lba() != b.primary_index_lba()
                        || a.pregap.map_or(0, |p| p.to_lba()) != b.pregap.map_or(0, |p| p.to_lba())
                })
            {
                return Err(invalid("CHD decoded track layout mismatch"));
            }
            compare_sources(&disc.sources, &restored.with_extension("bin"), &cancel)?;
            fs::remove_file(restored.with_extension("bin"))?;
        } else {
            compare_sources(&disc.sources, &restored, &cancel)?;
        }
        fs::remove_file(restored)?;
        if merged.exists() {
            fs::remove_file(&merged)?;
            fs::remove_file(merged.with_extension("bin"))?;
        }
        if standalone.exists() {
            fs::remove_dir_all(standalone)?;
        }
        done += disc.bytes;
        progress(done, total);
        outputs.push(OutputFile {
            path: target,
            name: disc.name,
            bytes,
        });
    }
    outputs.extend(inventory.companions);
    if let Some((name, text)) = inventory.playlist {
        let path = dir.join(&name);
        fs::write(&path, &text)?;
        outputs.push(OutputFile {
            path,
            name,
            bytes: text.len() as u64,
        });
    }
    let files = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            Ok(library::PlanInputFile {
                index,
                original_file_name: output.name.clone(),
                file_size_bytes: Some(output.bytes),
                staged_path: None,
                manifest_contents: if extension(&output.name) == "m3u" {
                    Some(fs::read_to_string(&output.path)?)
                } else {
                    None
                },
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let plan = library::build_ingest_plan(0, "chd", files, None);
    if !plan.errors.is_empty() || plan.roms.len() != 1 {
        return Err(invalid("CHD output grouping changed"));
    }
    Ok(outputs)
}
fn compare_sources(sources: &[PathBuf], restored: &Path, cancel: &AtomicBool) -> io::Result<()> {
    let mut decoded = File::open(restored)?;
    let mut original = vec![0; 128 * 1024];
    let mut actual = vec![0; original.len()];
    for path in sources {
        let mut source = File::open(path)?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(invalid("Cancelled"));
            }
            let n = source.read(&mut original)?;
            if n == 0 {
                break;
            }
            decoded.read_exact(&mut actual[..n])?;
            if original[..n] != actual[..n] {
                return Err(invalid("CHD decoded content mismatch"));
            }
        }
    }
    if decoded.read(&mut actual[..1])? != 0 {
        return Err(invalid("CHD decoded size mismatch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn iso(path: &Path, ps2: bool) -> u64 {
        let sectors = if ps2 { 453_000u32 } else { 64 };
        let mut file = File::create(path).unwrap();
        file.set_len(u64::from(sectors) * 2048).unwrap();
        let mut pvd = [0; 2048];
        pvd[..7].copy_from_slice(&[1, b'C', b'D', b'0', b'0', b'1', 1]);
        if !ps2 {
            pvd[8..16].copy_from_slice(b"PSP GAME");
        }
        pvd[80..84].copy_from_slice(&sectors.to_le_bytes());
        pvd[84..88].copy_from_slice(&sectors.to_be_bytes());
        pvd[128..132].copy_from_slice(&[0, 8, 8, 0]);
        pvd[156] = 34;
        pvd[158..162].copy_from_slice(&20u32.to_le_bytes());
        pvd[166..170].copy_from_slice(&2048u32.to_le_bytes());
        file.seek(SeekFrom::Start(16 * 2048)).unwrap();
        file.write_all(&pvd).unwrap();
        let mut root = [0; 2048];
        root[0] = 45;
        root[2..6].copy_from_slice(&21u32.to_le_bytes());
        root[10..14].copy_from_slice(&32u32.to_le_bytes());
        root[32] = 12;
        root[33..45].copy_from_slice(b"SYSTEM.CNF;1");
        file.seek(SeekFrom::Start(20 * 2048)).unwrap();
        file.write_all(&root).unwrap();
        file.seek(SeekFrom::Start(21 * 2048)).unwrap();
        file.write_all(if ps2 { b"BOOT2" } else { b"BOOT " })
            .unwrap();
        u64::from(sectors) * 2048
    }
    #[test]
    fn ps2_dvd_requires_real_volume_extent_and_correct_platform() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Disc.iso");
        let bytes = iso(&path, true);
        let inputs = [InputFile {
            path: "Disc.iso".into(),
            bytes,
        }];
        assert!(inspect(dir.path(), &inputs, "ps2").is_ok());
        assert!(inspect(dir.path(), &inputs, "psp").is_err());
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(64 * 2048)
            .unwrap();
        assert!(
            inspect(
                dir.path(),
                &[InputFile {
                    path: "Disc.iso".into(),
                    bytes: 64 * 2048
                }],
                "ps2"
            )
            .is_err()
        );
    }
    #[tokio::test]
    async fn chd_allowance_and_joined_codec_cancellation_leave_no_publication() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = iso(&dir.path().join("Disc.iso"), false);
        let inputs = [InputFile {
            path: "Disc.iso".into(),
            bytes,
        }];
        let cancel = Arc::new(AtomicBool::new(false));
        let output = dir.path().join("game.chd");
        assert!(
            pack(
                inspect(dir.path(), &inputs, "psp").unwrap(),
                &output,
                1,
                0,
                cancel.clone(),
                |_, _| {},
                |_| {}
            )
            .await
            .is_err()
        );
        assert!(!dir.path().join("chd-output").exists());
        assert!(
            pack(
                inspect(dir.path(), &inputs, "psp").unwrap(),
                &output,
                2 * 1024 * 1024,
                0,
                cancel.clone(),
                |_, _| cancel.store(true, Ordering::Relaxed),
                |_| {}
            )
            .await
            .is_err()
        );
        // A fast writer may finish privately before cancellation is observed.
        // Its joined output is never published and the workspace owner removes it.
        assert!(
            fs::read_dir(dir.path().join("chd-output"))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
        let path = dir.path().to_path_buf();
        drop(dir);
        assert!(!path.exists());
    }
}
