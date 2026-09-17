use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::config::SIPARIO_MAX_COMPRESSION_RATIO;

use super::{
    GogImportByteProgress, GogImportError, GogImportPhase, GogImportPhaseLog, GogImportSummary,
    GogImportWorkflowReporter,
};

const MAX_ARCHIVE_PATH_BYTES: usize = 1_024;
const MAX_ARCHIVE_COMPONENT_BYTES: usize = 255;

#[derive(Debug)]
struct TreeEntry {
    source: PathBuf,
    archive_name: String,
    is_dir: bool,
    size: u64,
    launcher: bool,
}

struct ProgressReader<'a, R, F> {
    inner: R,
    cumulative_bytes: &'a mut u64,
    report: &'a mut F,
}

impl<R, F> Read for ProgressReader<'_, R, F>
where
    R: Read,
    F: FnMut(u64),
{
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        if read == 0 {
            return Ok(0);
        }
        let read_bytes = u64::try_from(read).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "source byte count overflow")
        })?;
        *self.cumulative_bytes =
            self.cumulative_bytes
                .checked_add(read_bytes)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "source byte count overflow")
                })?;
        (self.report)(*self.cumulative_bytes);
        Ok(read)
    }
}

fn copy_source_with_progress<R, W, F>(
    source: R,
    destination: &mut W,
    expected_file_bytes: u64,
    cumulative_bytes: &mut u64,
    report: &mut F,
) -> Result<(), GogImportError>
where
    R: Read,
    W: Write,
    F: FnMut(u64),
{
    let mut source = ProgressReader {
        inner: source,
        cumulative_bytes,
        report,
    };
    let copied = io::copy(&mut source, destination)?;
    if copied != expected_file_bytes {
        return Err(GogImportError::ExtractedOutputChanged);
    }
    Ok(())
}

pub(super) fn package_extracted_tree(
    source_root: &Path,
    archive_path: &Path,
    max_entries: usize,
    max_bytes: u64,
    free_space_margin_bytes: u64,
    reporter: &GogImportWorkflowReporter,
) -> Result<GogImportSummary, GogImportError> {
    let validation_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::ValidateExtractedTree);
    let mut entries = Vec::new();
    let mut case_folded_names = BTreeSet::new();
    let mut extracted_bytes = 0_u64;
    collect_entries(
        source_root,
        source_root,
        0,
        max_entries,
        max_bytes,
        &mut extracted_bytes,
        &mut entries,
        &mut case_folded_names,
    )?;
    entries.sort_by(|left, right| left.archive_name.cmp(&right.archive_name));

    let file_count = entries.iter().filter(|entry| !entry.is_dir).count();
    let launcher_count = entries.iter().filter(|entry| entry.launcher).count();
    if file_count == 0 {
        return Err(GogImportError::EmptyExtractedOutput);
    }
    if launcher_count == 0 {
        return Err(GogImportError::MissingLaunchCandidate);
    }
    let archive_parent = archive_path
        .parent()
        .ok_or(GogImportError::UnsafeExtractedOutput)?;
    let archive_overhead = u64::try_from(entries.len())
        .unwrap_or(u64::MAX)
        .saturating_mul(512);
    let required_space = extracted_bytes
        .saturating_add(archive_overhead)
        .saturating_add(free_space_margin_bytes);
    if fs2::available_space(archive_parent)? < required_space {
        return Err(GogImportError::InsufficientStorage);
    }
    validation_phase.complete();
    tracing::info!(
        phase = "validate_extracted_tree",
        extracted_file_count = file_count,
        extracted_entry_count = entries.len(),
        extracted_bytes,
        launcher_count,
        "Validated GOG extracted tree"
    );

    let zip_phase = GogImportPhaseLog::start(reporter, GogImportPhase::CreateWindowsZip);
    let archive_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(archive_path)?;
    let mut zip = ZipWriter::new(archive_file);
    let timestamp = DateTime::default();
    let mut progress = GogImportByteProgress::new(extracted_bytes, |current, total| {
        reporter.report_bytes(GogImportPhase::CreateWindowsZip, current, total);
    });
    progress.start();
    let mut copied_source_bytes = 0_u64;

    {
        let mut report_source_bytes = |current| progress.update(current);
        for entry in &entries {
            if entry.is_dir {
                let name = format!("{}/", entry.archive_name.trim_end_matches('/'));
                let options = SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Stored)
                    .last_modified_time(timestamp)
                    .unix_permissions(0o755);
                zip.add_directory(name, options)?;
                continue;
            }

            let permissions = if entry.launcher { 0o755 } else { 0o644 };
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .last_modified_time(timestamp)
                .large_file(entry.size > u32::MAX as u64)
                .unix_permissions(permissions);
            zip.start_file(&entry.archive_name, options)?;
            let source = File::open(&entry.source)?;
            copy_source_with_progress(
                source,
                &mut zip,
                entry.size,
                &mut copied_source_bytes,
                &mut report_source_bytes,
            )?;
        }
    }

    if copied_source_bytes != extracted_bytes {
        return Err(GogImportError::ExtractedOutputChanged);
    }
    progress.finish(copied_source_bytes);
    zip_phase.complete();

    let finalize_phase = GogImportPhaseLog::start(reporter, GogImportPhase::FinalizeWindowsZip);
    let archive_file = zip.finish()?;
    archive_file.sync_all()?;
    let archive_bytes = archive_file.metadata()?.len();
    drop(archive_file);
    finalize_phase.complete();
    tracing::info!(
        phase = GogImportPhase::FinalizeWindowsZip.as_str(),
        archive_bytes,
        "Created and finalized game-named Windows ZIP"
    );

    let compatibility_phase =
        GogImportPhaseLog::start(reporter, GogImportPhase::ValidateSiparioZip);
    validate_sipario_archive(archive_path, max_entries, max_bytes)?;
    compatibility_phase.complete();

    Ok(GogImportSummary {
        input_file_count: 0,
        input_bytes: 0,
        extracted_file_count: file_count,
        extracted_bytes,
        launcher_count,
        archive_bytes,
        extractor_version: String::new(),
        data_version: String::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn collect_entries(
    source_root: &Path,
    current: &Path,
    depth: usize,
    max_entries: usize,
    max_bytes: u64,
    extracted_bytes: &mut u64,
    entries: &mut Vec<TreeEntry>,
    case_folded_names: &mut BTreeSet<String>,
) -> Result<(), GogImportError> {
    if depth > 128 {
        return Err(GogImportError::UnsafeExtractedOutput);
    }

    let root_metadata = fs::symlink_metadata(current)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(GogImportError::UnsafeExtractedOutput);
    }

    let mut children = fs::read_dir(current)?.collect::<Result<Vec<_>, _>>()?;
    children.sort_by_key(|entry| entry.file_name());

    for child in children {
        if entries.len() >= max_entries {
            return Err(GogImportError::TooManyExtractedEntries);
        }
        let path = child.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(GogImportError::UnsafeExtractedOutput);
        }

        let relative = path
            .strip_prefix(source_root)
            .map_err(|_| GogImportError::UnsafeExtractedOutput)?;
        let archive_name = safe_archive_name(relative)?;
        let case_folded = archive_name.to_lowercase();
        if !case_folded_names.insert(case_folded) {
            return Err(GogImportError::CaseCollidingExtractedPaths);
        }

        if metadata.is_dir() {
            entries.push(TreeEntry {
                source: path.clone(),
                archive_name,
                is_dir: true,
                size: 0,
                launcher: false,
            });
            collect_entries(
                source_root,
                &path,
                depth + 1,
                max_entries,
                max_bytes,
                extracted_bytes,
                entries,
                case_folded_names,
            )?;
        } else if metadata.is_file() {
            #[cfg(unix)]
            if metadata.nlink() > 1 {
                return Err(GogImportError::UnsafeExtractedOutput);
            }
            let size = metadata.len();
            *extracted_bytes = extracted_bytes
                .checked_add(size)
                .ok_or(GogImportError::ExtractedOutputTooLarge)?;
            if *extracted_bytes > max_bytes {
                return Err(GogImportError::ExtractedOutputTooLarge);
            }
            let launcher = is_launch_candidate(&archive_name);
            entries.push(TreeEntry {
                source: path,
                archive_name,
                is_dir: false,
                size,
                launcher,
            });
        } else {
            return Err(GogImportError::UnsafeExtractedOutput);
        }
    }

    Ok(())
}

fn safe_archive_name(relative: &Path) -> Result<String, GogImportError> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(GogImportError::UnsafeExtractedOutput);
    }

    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(GogImportError::UnsafeExtractedOutput);
        };
        let component = component
            .to_str()
            .ok_or(GogImportError::UnsafeExtractedOutput)?;
        validate_windows_component(component)?;
        parts.push(component);
    }
    let archive_name = parts.join("/");
    if archive_name.len() > MAX_ARCHIVE_PATH_BYTES {
        return Err(GogImportError::UnsafeExtractedOutput);
    }
    Ok(archive_name)
}

fn validate_windows_component(component: &str) -> Result<(), GogImportError> {
    if component.is_empty()
        || component.len() > MAX_ARCHIVE_COMPONENT_BYTES
        || component.ends_with([' ', '.'])
        || component.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(GogImportError::UnsafeExtractedOutput);
    }

    let stem = component
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9'));
    if reserved {
        return Err(GogImportError::UnsafeExtractedOutput);
    }
    Ok(())
}

fn validate_sipario_archive(
    archive_path: &Path,
    max_entries: usize,
    max_bytes: u64,
) -> Result<(), GogImportError> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file)?;
    if archive.len() > max_entries {
        return Err(GogImportError::SiparioArchiveIncompatible);
    }

    let mut total_uncompressed = 0_u64;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .ok_or(GogImportError::SiparioArchiveIncompatible)?;
        safe_archive_name(&relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(GogImportError::SiparioArchiveIncompatible);
        }
        total_uncompressed = total_uncompressed
            .checked_add(entry.size())
            .ok_or(GogImportError::SiparioArchiveIncompatible)?;
        if total_uncompressed > max_bytes {
            return Err(GogImportError::SiparioArchiveIncompatible);
        }
        if entry.compressed_size() > 0
            && entry.size() / entry.compressed_size() > SIPARIO_MAX_COMPRESSION_RATIO
        {
            return Err(GogImportError::SiparioArchiveIncompatible);
        }
    }
    Ok(())
}

fn is_launch_candidate(archive_name: &str) -> bool {
    Path::new(archive_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("exe") || extension.eq_ignore_ascii_case("bat")
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::{Cursor, Read},
        sync::Arc,
    };

    use sha2::{Digest, Sha256};
    use tempfile::TempDir;
    use zip::ZipArchive;

    use super::{copy_source_with_progress, package_extracted_tree};
    use crate::services::gog_import::{
        GogImportError, GogImportJobRegistry, GogImportWorkflowReporter,
    };

    #[test]
    fn packages_a_deterministic_safe_tree_with_a_launcher() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("output");
        std::fs::create_dir_all(output.join("app/data")).unwrap();
        std::fs::write(output.join("app/Game.exe"), b"launcher").unwrap();
        std::fs::write(output.join("app/data/game.dat"), b"payload").unwrap();
        let archive = temp.path().join("Game.zip");
        let second_archive = temp.path().join("Game-copy.zip");

        let reporter = GogImportWorkflowReporter::noop();
        let summary = package_extracted_tree(&output, &archive, 10, 1024, 0, &reporter).unwrap();
        package_extracted_tree(&output, &second_archive, 10, 1024, 0, &reporter).unwrap();

        let archive_bytes = std::fs::read(&archive).unwrap();
        assert_eq!(archive_bytes, std::fs::read(second_archive).unwrap());
        assert_eq!(
            format!("{:x}", Sha256::digest(&archive_bytes)),
            "3b95a15c6921046c10a12d4b8cbe56ce393b296c898d4ed1bcc1c2e189e46dd4"
        );
        assert_eq!(summary.extracted_file_count, 2);
        assert_eq!(summary.launcher_count, 1);
        let mut zip = ZipArchive::new(File::open(archive).unwrap()).unwrap();
        let mut contents = String::new();
        zip.by_name("app/data/game.dat")
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert_eq!(contents, "payload");
    }

    #[test]
    fn packages_a_large_generated_tree_with_bounded_progress_inputs() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("large-output");
        std::fs::create_dir_all(output.join("app/data")).unwrap();
        std::fs::write(output.join("app/Game.exe"), b"launcher").unwrap();
        for index in 0..1_024 {
            std::fs::write(
                output.join(format!("app/data/chunk-{index:04}.bin")),
                vec![u8::try_from(index % 251).unwrap(); 1_024],
            )
            .unwrap();
        }

        let archive = temp.path().join("large.zip");
        let summary = package_extracted_tree(
            &output,
            &archive,
            1_100,
            2 * 1024 * 1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap();

        assert_eq!(summary.extracted_file_count, 1_025);
        assert_eq!(summary.extracted_bytes, 1_048_576 + 8);
        let zip = ZipArchive::new(File::open(archive).unwrap()).unwrap();
        assert_eq!(zip.len(), 1_027);
    }

    #[test]
    fn rejects_insufficient_archive_space_before_creating_a_zip() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("Game.exe"), b"launcher").unwrap();
        let archive = temp.path().join("never-created.zip");

        let error = package_extracted_tree(
            &output,
            &archive,
            10,
            1024,
            u64::MAX,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap_err();

        assert!(matches!(error, GogImportError::InsufficientStorage));
        assert!(!archive.exists());
    }

    #[test]
    fn source_copy_reports_monotonic_checked_bytes_and_rejects_size_changes() {
        let payload = vec![0x5a; 32 * 1024];
        let mut destination = Vec::new();
        let mut cumulative = 0_u64;
        let mut updates = Vec::new();
        copy_source_with_progress(
            Cursor::new(&payload),
            &mut destination,
            payload.len() as u64,
            &mut cumulative,
            &mut |current| updates.push(current),
        )
        .unwrap();

        assert_eq!(destination, payload);
        assert_eq!(cumulative, payload.len() as u64);
        assert_eq!(updates.last().copied(), Some(payload.len() as u64));
        assert!(updates.windows(2).all(|window| window[0] < window[1]));

        let mut destination = Vec::new();
        let mut cumulative = 0_u64;
        let error = copy_source_with_progress(
            Cursor::new(b"short"),
            &mut destination,
            6,
            &mut cumulative,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, GogImportError::ExtractedOutputChanged));

        let mut destination = Vec::new();
        let mut cumulative = 0_u64;
        let error = copy_source_with_progress(
            Cursor::new(b"longer"),
            &mut destination,
            5,
            &mut cumulative,
            &mut |_| {},
        )
        .unwrap_err();
        assert!(matches!(error, GogImportError::ExtractedOutputChanged));
    }

    #[test]
    fn packages_zero_byte_launchers_and_directories() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("zero-output");
        std::fs::create_dir_all(output.join("empty/directory")).unwrap();
        std::fs::write(output.join("Game.exe"), []).unwrap();
        let archive = temp.path().join("zero.zip");

        let summary = package_extracted_tree(
            &output,
            &archive,
            10,
            1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap();

        assert_eq!(summary.extracted_bytes, 0);
        let mut zip = ZipArchive::new(File::open(archive).unwrap()).unwrap();
        assert_eq!(zip.by_name("Game.exe").unwrap().size(), 0);
        assert!(zip.by_name("empty/directory/").unwrap().is_dir());
    }

    #[test]
    fn reporter_errors_do_not_fail_packaging() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("Game.exe"), b"launcher").unwrap();

        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let reporter = GogImportWorkflowReporter::new(reservation.reporter);
        let summary = package_extracted_tree(
            &output,
            &temp.path().join("reporter.zip"),
            10,
            1024,
            0,
            &reporter,
        )
        .unwrap();

        assert_eq!(summary.extracted_bytes, 8);
    }

    #[test]
    fn rejects_output_without_a_launch_candidate() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("readme.txt"), b"not launchable").unwrap();

        let error = package_extracted_tree(
            &output,
            &temp.path().join("game.zip"),
            10,
            1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap_err();

        assert!(matches!(error, GogImportError::MissingLaunchCandidate));
    }

    #[test]
    fn rejects_archives_that_exceed_sipario_compression_ratio() {
        let temp = TempDir::new().unwrap();
        let output = temp.path().join("ratio-output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("Game.exe"), b"launcher").unwrap();
        std::fs::write(output.join("zeros.dat"), vec![0_u8; 16 * 1024 * 1024]).unwrap();

        let error = package_extracted_tree(
            &output,
            &temp.path().join("ratio.zip"),
            10,
            32 * 1024 * 1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap_err();

        assert!(matches!(error, GogImportError::SiparioArchiveIncompatible));
        assert_eq!(
            error.to_string(),
            "the generated ZIP does not satisfy the archive safety limits"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinks_and_case_collisions() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let output = temp.path().join("symlink-output");
        std::fs::create_dir_all(&output).unwrap();
        symlink("/tmp", output.join("escape")).unwrap();
        let error = package_extracted_tree(
            &output,
            &temp.path().join("symlink.zip"),
            10,
            1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap_err();
        assert!(matches!(error, GogImportError::UnsafeExtractedOutput));

        let output = temp.path().join("collision-output");
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(output.join("Game.exe"), b"one").unwrap();
        std::fs::write(output.join("game.EXE"), b"two").unwrap();
        let error = package_extracted_tree(
            &output,
            &temp.path().join("collision.zip"),
            10,
            1024,
            0,
            &GogImportWorkflowReporter::noop(),
        )
        .unwrap_err();
        assert!(matches!(error, GogImportError::CaseCollidingExtractedPaths));
    }
}
