use std::{
    ffi::OsString,
    mem,
    path::Path,
    process::ExitStatus,
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
    time::timeout,
};

use crate::config::GogImportConfig;

use super::{
    GogImportError, GogImportPhase, GogImportProgress, GogImportWorkflowReporter,
    jobs::{GogImportOutputStream, MAX_DISPLAY_ENTRY_BYTES, MAX_PROGRESS_UPDATES_PER_SECOND},
};

const MAX_CAPTURED_DIAGNOSTIC_BYTES: usize = 256 * 1024;
const DIAGNOSTIC_SCAN_OVERLAP_BYTES: usize = 32;
const MAX_PENDING_TERMINAL_BYTES: usize = MAX_DISPLAY_ENTRY_BYTES * 4;
const REDACTED_PATH: &str = "[redacted]";
const OUTPUT_UPDATE_INTERVAL: Duration =
    Duration::from_millis(1000 / MAX_PROGRESS_UPDATES_PER_SECOND as u64);

#[derive(Debug)]
pub(super) struct ProcessOutput {
    pub(super) status: ExitStatus,
    pub(super) stdout: String,
    pub(super) suspicious_diagnostics: bool,
    pub(super) truncated: bool,
}

pub(super) struct ProcessReport<'a> {
    reporter: &'a GogImportWorkflowReporter,
    phase: GogImportPhase,
    private_paths: &'a [&'a Path],
    parse_progress: bool,
}

impl<'a> ProcessReport<'a> {
    pub(super) const fn new(
        reporter: &'a GogImportWorkflowReporter,
        phase: GogImportPhase,
        private_paths: &'a [&'a Path],
    ) -> Self {
        Self {
            reporter,
            phase,
            private_paths,
            parse_progress: false,
        }
    }

    pub(super) const fn with_progress(
        reporter: &'a GogImportWorkflowReporter,
        phase: GogImportPhase,
        private_paths: &'a [&'a Path],
    ) -> Self {
        Self {
            reporter,
            phase,
            private_paths,
            parse_progress: true,
        }
    }
}

#[derive(Debug)]
struct CapturedStream {
    bytes: Vec<u8>,
    total_bytes: usize,
    suspicious: bool,
}

#[cfg(unix)]
struct ProcessGroupGuard {
    pid: Option<Pid>,
}

#[cfg(unix)]
impl ProcessGroupGuard {
    fn new(pid: Option<u32>) -> Self {
        Self {
            pid: pid
                .and_then(|pid| i32::try_from(pid).ok())
                .map(Pid::from_raw),
        }
    }

    fn terminate(&mut self) {
        if let Some(pid) = self.pid.take() {
            let _ = kill(Pid::from_raw(-pid.as_raw()), Signal::SIGKILL);
        }
    }

    fn disarm(&mut self) {
        self.pid = None;
    }
}

#[cfg(unix)]
impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub(super) async fn verify_extractor_hash(config: &GogImportConfig) -> Result<(), GogImportError> {
    let path = config
        .innoextract_path
        .as_deref()
        .ok_or(GogImportError::NotConfigured)?;
    let expected = config
        .innoextract_sha256
        .as_deref()
        .ok_or(GogImportError::NotConfigured)?;
    let metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|_| GogImportError::ExtractorUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(GogImportError::ExtractorUnavailable);
    }

    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| GogImportError::ExtractorUnavailable)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .await
            .map_err(|_| GogImportError::ExtractorUnavailable)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(GogImportError::ExtractorHashMismatch);
    }
    Ok(())
}

pub(super) async fn run_innoextract(
    config: &GogImportConfig,
    working_directory: &Path,
    arguments: &[OsString],
    phase: &'static str,
    phase_timeout: Duration,
    report: ProcessReport<'_>,
) -> Result<ProcessOutput, GogImportError> {
    let executable = config
        .innoextract_path
        .as_deref()
        .ok_or(GogImportError::NotConfigured)?;
    let temporary_directory = working_directory.join("tmp");
    tokio::fs::create_dir_all(&temporary_directory).await?;

    let mut redacted_paths = report.private_paths.to_vec();
    redacted_paths.push(working_directory);
    redacted_paths.push(&temporary_directory);
    redacted_paths.push(executable);
    if let Some(parent) = working_directory.parent() {
        redacted_paths.push(parent);
    }
    if let Some(helper_path) = config.helper_path.as_deref() {
        redacted_paths.push(helper_path);
    }
    let redactor = Arc::new(PathRedactor::new(&redacted_paths));

    let mut command = Command::new(executable);
    command
        .args(arguments)
        .current_dir(working_directory)
        .kill_on_drop(true)
        .env_clear()
        .env(
            "PATH",
            config
                .helper_path
                .as_deref()
                .map(|path| path.as_os_str())
                .unwrap_or_default(),
        )
        .env("HOME", working_directory)
        .env("TMPDIR", &temporary_directory)
        .env("TMP", &temporary_directory)
        .env("TEMP", &temporary_directory)
        .env("LC_ALL", "C.UTF-8")
        .env("LANG", "C.UTF-8")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    command.as_std_mut().process_group(0);

    #[cfg(windows)]
    {
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        if let Some(windir) = std::env::var_os("WINDIR") {
            command.env("WINDIR", windir);
        }
    }

    let mut child = command
        .spawn()
        .map_err(|_| GogImportError::ExtractorUnavailable)?;
    #[cfg(unix)]
    let mut process_group = ProcessGroupGuard::new(child.id());
    let stdout = child
        .stdout
        .take()
        .ok_or(GogImportError::ExtractorUnavailable)?;
    let stderr = child
        .stderr
        .take()
        .ok_or(GogImportError::ExtractorUnavailable)?;
    let stdout_decoder = TerminalOutputDecoder::new(
        report.reporter.clone(),
        report.phase,
        GogImportOutputStream::Stdout,
        Arc::clone(&redactor),
        report.parse_progress,
    );
    let stderr_decoder = TerminalOutputDecoder::new(
        report.reporter.clone(),
        report.phase,
        GogImportOutputStream::Stderr,
        redactor,
        false,
    );
    let stdout_task = tokio::spawn(capture_stream(stdout, stdout_decoder));
    let stderr_task = tokio::spawn(capture_stream(stderr, stderr_decoder));

    let status = match timeout(phase_timeout, child.wait()).await {
        Ok(Ok(status)) => {
            #[cfg(unix)]
            process_group.disarm();
            status
        }
        Ok(Err(_)) => {
            #[cfg(unix)]
            process_group.terminate();
            #[cfg(not(unix))]
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(GogImportError::ExtractorUnavailable);
        }
        Err(_) => {
            #[cfg(unix)]
            process_group.terminate();
            #[cfg(not(unix))]
            let _ = child.kill().await;
            let _ = child.wait().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(GogImportError::ExtractorTimedOut { phase });
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|_| GogImportError::ExtractorUnavailable)??;
    let stderr = stderr_task
        .await
        .map_err(|_| GogImportError::ExtractorUnavailable)??;
    Ok(ProcessOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
        suspicious_diagnostics: stdout.suspicious || stderr.suspicious,
        truncated: stdout.total_bytes > MAX_CAPTURED_DIAGNOSTIC_BYTES
            || stderr.total_bytes > MAX_CAPTURED_DIAGNOSTIC_BYTES,
    })
}

async fn capture_stream(
    mut stream: impl AsyncRead + Unpin,
    decoder: TerminalOutputDecoder,
) -> Result<CapturedStream, std::io::Error> {
    let mut capture = StreamCapture::new(decoder);
    let mut buffer = [0_u8; 16 * 1024];

    loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        capture.consume(&buffer[..read]);
    }

    Ok(capture.finish())
}

struct StreamCapture {
    captured: Vec<u8>,
    total_bytes: usize,
    suspicious: bool,
    overlap: Vec<u8>,
    decoder: TerminalOutputDecoder,
}

impl StreamCapture {
    fn new(decoder: TerminalOutputDecoder) -> Self {
        Self {
            captured: Vec::new(),
            total_bytes: 0,
            suspicious: false,
            overlap: Vec::new(),
            decoder,
        }
    }

    fn consume(&mut self, bytes: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());

        let remaining = MAX_CAPTURED_DIAGNOSTIC_BYTES.saturating_sub(self.captured.len());
        let retained = bytes.len().min(remaining);
        self.captured.extend_from_slice(&bytes[..retained]);
        self.decoder.consume(&bytes[..retained]);
        if retained < bytes.len() {
            self.decoder.mark_truncated();
        }

        let mut scan = mem::take(&mut self.overlap);
        scan.extend(bytes.iter().map(u8::to_ascii_lowercase));
        self.suspicious |= contains_ascii(&scan, b"warning")
            || contains_ascii(&scan, b"error")
            || contains_ascii(&scan, b"done with");
        let keep = scan.len().min(DIAGNOSTIC_SCAN_OVERLAP_BYTES);
        self.overlap.extend_from_slice(&scan[scan.len() - keep..]);
    }

    fn finish(mut self) -> CapturedStream {
        self.decoder.finish();
        CapturedStream {
            bytes: self.captured,
            total_bytes: self.total_bytes,
            suspicious: self.suspicious,
        }
    }
}

struct TerminalOutputDecoder {
    reporter: GogImportWorkflowReporter,
    phase: GogImportPhase,
    stream: GogImportOutputStream,
    redactor: Arc<PathRedactor>,
    parse_progress: bool,
    pending: Vec<u8>,
    pending_truncated: bool,
    previous_delimiter_was_cr: bool,
    last_percent: Option<f64>,
    last_output_at: Option<Instant>,
    deferred_output: Option<String>,
    truncation_reported: bool,
}

impl TerminalOutputDecoder {
    fn new(
        reporter: GogImportWorkflowReporter,
        phase: GogImportPhase,
        stream: GogImportOutputStream,
        redactor: Arc<PathRedactor>,
        parse_progress: bool,
    ) -> Self {
        Self {
            reporter,
            phase,
            stream,
            redactor,
            parse_progress,
            pending: Vec::new(),
            pending_truncated: false,
            previous_delimiter_was_cr: false,
            last_percent: None,
            last_output_at: None,
            deferred_output: None,
            truncation_reported: false,
        }
    }

    fn consume(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            match byte {
                b'\r' => {
                    self.finish_record();
                    self.previous_delimiter_was_cr = true;
                }
                b'\n' if self.previous_delimiter_was_cr => {
                    self.previous_delimiter_was_cr = false;
                }
                b'\n' => {
                    self.finish_record();
                    self.previous_delimiter_was_cr = false;
                }
                _ => {
                    self.previous_delimiter_was_cr = false;
                    if self.pending.len() < MAX_PENDING_TERMINAL_BYTES {
                        self.pending.push(byte);
                    } else {
                        self.pending_truncated = true;
                    }
                }
            }
        }
    }

    fn finish(&mut self) {
        if !self.pending.is_empty() || self.pending_truncated {
            self.finish_record();
        }
        if let Some(text) = self.deferred_output.take() {
            self.append_output(&text);
        }
    }

    fn finish_record(&mut self) {
        let raw = mem::take(&mut self.pending);
        let raw_truncated = mem::take(&mut self.pending_truncated);
        let sanitized = sanitize_terminal_record(&raw, &self.redactor);
        if raw_truncated || sanitized.truncated {
            self.mark_truncated();
        }
        if sanitized.text.is_empty() {
            return;
        }

        if self.parse_progress
            && let Some(percent) = parse_pinned_progress(&sanitized.text)
        {
            let monotonic = self.last_percent.is_none_or(|last| percent >= last);
            if monotonic && let Ok(progress) = GogImportProgress::percent(percent) {
                let _ = self.reporter.set_progress(self.phase, progress);
                self.last_percent = Some(percent);
            }
        }

        self.publish_output(sanitized.text);
    }

    fn publish_output(&mut self, text: String) {
        let now = Instant::now();
        if contains_suspicious_diagnostic(text.as_bytes()) {
            if let Some(pending) = self.deferred_output.take() {
                self.append_output(&pending);
            }
            self.append_output(&text);
            self.last_output_at = Some(now);
            return;
        }

        let publish_now = self
            .last_output_at
            .is_none_or(|last| now.saturating_duration_since(last) >= OUTPUT_UPDATE_INTERVAL);
        if publish_now {
            self.append_output(&text);
            self.last_output_at = Some(now);
        } else {
            self.deferred_output = Some(text);
        }
    }

    fn append_output(&self, text: &str) {
        let _ = self.reporter.append_output(self.phase, self.stream, text);
    }

    fn mark_truncated(&mut self) {
        if self.truncation_reported {
            return;
        }
        self.truncation_reported = true;
        let _ = self.reporter.mark_output_truncated(self.phase);
    }
}

#[derive(Debug)]
struct SanitizedRecord {
    text: String,
    truncated: bool,
}

fn sanitize_terminal_record(raw: &[u8], redactor: &PathRedactor) -> SanitizedRecord {
    let without_ansi = strip_terminal_sequences(raw);
    let decoded = String::from_utf8_lossy(&without_ansi);
    let mut safe = String::with_capacity(decoded.len());
    for character in decoded.chars() {
        match character {
            '\t' => safe.push(' '),
            character if character.is_control() => safe.push('\u{fffd}'),
            character => safe.push(character),
        }
    }

    let safe = redactor.redact(safe.trim());
    let (text, truncated) = truncate_utf8(safe, MAX_DISPLAY_ENTRY_BYTES);
    SanitizedRecord { text, truncated }
}

#[derive(Debug, Clone, Copy)]
enum EscapeState {
    Ground,
    Escape,
    Csi,
    ControlString,
    ControlStringEscape,
}

fn strip_terminal_sequences(raw: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(raw.len());
    let mut state = EscapeState::Ground;

    for &byte in raw {
        state = match state {
            EscapeState::Ground if byte == 0x1b => EscapeState::Escape,
            EscapeState::Ground => {
                if byte < 0x20 || byte == 0x7f {
                    output.extend_from_slice("\u{fffd}".as_bytes());
                } else {
                    output.push(byte);
                }
                EscapeState::Ground
            }
            EscapeState::Escape => match byte {
                b'[' => EscapeState::Csi,
                b']' | b'P' | b'^' | b'_' => EscapeState::ControlString,
                _ => EscapeState::Ground,
            },
            EscapeState::Csi if (0x40..=0x7e).contains(&byte) => EscapeState::Ground,
            EscapeState::Csi => EscapeState::Csi,
            EscapeState::ControlString if byte == 0x07 => EscapeState::Ground,
            EscapeState::ControlString if byte == 0x1b => EscapeState::ControlStringEscape,
            EscapeState::ControlString => EscapeState::ControlString,
            EscapeState::ControlStringEscape if byte == b'\\' => EscapeState::Ground,
            EscapeState::ControlStringEscape if byte == 0x1b => EscapeState::ControlStringEscape,
            EscapeState::ControlStringEscape => EscapeState::ControlString,
        };
    }

    output
}

fn truncate_utf8(mut text: String, max_bytes: usize) -> (String, bool) {
    if text.len() <= max_bytes {
        return (text, false);
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    (text, true)
}

#[derive(Debug)]
struct PathRedactor {
    patterns: Vec<String>,
}

impl PathRedactor {
    fn new(paths: &[&Path]) -> Self {
        let mut patterns = Vec::new();
        for path in paths {
            let pattern = path.to_string_lossy();
            Self::push_pattern(&mut patterns, pattern.as_ref());
            let forward_slashes = pattern.replace('\\', "/");
            Self::push_pattern(&mut patterns, &forward_slashes);
            let backward_slashes = pattern.replace('/', "\\");
            Self::push_pattern(&mut patterns, &backward_slashes);
        }
        patterns.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
        patterns.dedup();
        Self { patterns }
    }

    fn push_pattern(patterns: &mut Vec<String>, pattern: &str) {
        if !pattern.is_empty() && !matches!(pattern, "/" | "\\") {
            patterns.push(pattern.to_string());
        }
    }

    fn redact(&self, input: &str) -> String {
        self.patterns
            .iter()
            .fold(input.to_string(), |text, pattern| {
                text.replace(pattern, REDACTED_PATH)
            })
    }
}

fn parse_pinned_progress(record: &str) -> Option<f64> {
    let bytes = record.as_bytes();
    for percent_index in bytes
        .iter()
        .enumerate()
        .filter_map(|(index, byte)| (*byte == b'%').then_some(index))
    {
        if bytes
            .get(percent_index + 1)
            .is_some_and(|byte| !byte.is_ascii_whitespace())
        {
            continue;
        }

        let mut start = percent_index;
        while start > 0 && (bytes[start - 1].is_ascii_digit() || bytes[start - 1] == b'.') {
            start -= 1;
        }
        if start == percent_index
            || start
                .checked_sub(1)
                .and_then(|index| bytes.get(index))
                .is_some_and(|byte| !byte.is_ascii_whitespace() && *byte != b']')
        {
            continue;
        }

        let token = &record[start..percent_index];
        let Some((integer, fraction)) = token.split_once('.') else {
            continue;
        };
        if integer.is_empty()
            || integer.len() > 3
            || fraction.len() != 1
            || !integer.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
            || (integer.len() > 1 && integer.starts_with('0'))
        {
            continue;
        }
        let value = token.parse::<f64>().ok()?;
        if value.is_finite() && (0.0..=100.0).contains(&value) {
            return Some(value);
        }
    }
    None
}

fn contains_suspicious_diagnostic(bytes: &[u8]) -> bool {
    let lowercase = bytes.iter().map(u8::to_ascii_lowercase).collect::<Vec<_>>();
    contains_ascii(&lowercase, b"warning")
        || contains_ascii(&lowercase, b"error")
        || contains_ascii(&lowercase, b"done with")
}

fn contains_ascii(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::services::gog_import::{GogImportJobId, GogImportJobRegistry};

    fn active_reporter(
        phase: GogImportPhase,
    ) -> (
        Arc<GogImportJobRegistry>,
        GogImportJobId,
        GogImportWorkflowReporter,
    ) {
        let registry = Arc::new(GogImportJobRegistry::new());
        let reservation = registry.reserve();
        let id = reservation.snapshot.id;
        reservation.reporter.start().unwrap();
        let reporter = GogImportWorkflowReporter::new(reservation.reporter);
        reporter.set_phase(phase).unwrap();
        (registry, id, reporter)
    }

    fn output_texts(snapshot: &super::super::GogImportJobSnapshot) -> Vec<&str> {
        snapshot
            .events
            .iter()
            .filter(|event| event.stream.is_some())
            .map(|event| event.text.as_str())
            .collect()
    }

    #[test]
    fn decoder_handles_split_ansi_crlf_non_utf8_redaction_and_progress() {
        let phase = GogImportPhase::ExtractInstallerPayload;
        let (registry, id, reporter) = active_reporter(phase);
        let workspace = Path::new("/private/workspace");
        let setup = Path::new("/private/workspace/input/setup_game.exe");
        let redactor = Arc::new(PathRedactor::new(&[workspace, setup]));
        let mut decoder = TerminalOutputDecoder::new(
            reporter,
            phase,
            GogImportOutputStream::Stdout,
            redactor,
            true,
        );

        decoder.consume(b"\r\x1b[");
        decoder.consume(b"K[====>] 42.5% 38.1 MiB/s\r");
        decoder.consume(b"\n");
        decoder.consume(b"\x1b[31mWarning at /private/workspace/input/setup_game.exe: \xff");
        decoder.consume(b"\x1b[0m\n");
        decoder.finish();

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.progress.as_ref().unwrap().percent, 42.5);
        let output = output_texts(&snapshot);
        assert_eq!(output.len(), 2);
        assert!(output[0].contains("42.5%"));
        assert!(output[1].contains("Warning at [redacted]: �"));
        assert!(!output.iter().any(|text| text.contains("/private")));
        assert!(output.iter().all(|text| !text.contains('\u{1b}')));
        assert!(
            output
                .iter()
                .all(|text| !text.chars().any(char::is_control))
        );
        assert!(!snapshot.output_truncated);
    }

    #[test]
    fn malformed_out_of_range_and_decreasing_progress_fail_soft() {
        for record in [
            "42%", "NaN%", "101.0%", "-1.0%", "01.0%", "42.50%", "x42.5%", "42.5%x",
        ] {
            assert_eq!(parse_pinned_progress(record), None, "{record}");
        }
        assert_eq!(
            parse_pinned_progress("[====>]100.0% 1.0 MiB/s"),
            Some(100.0)
        );

        let phase = GogImportPhase::TestInstallerIntegrity;
        let (registry, id, reporter) = active_reporter(phase);
        let mut decoder = TerminalOutputDecoder::new(
            reporter,
            phase,
            GogImportOutputStream::Stdout,
            Arc::new(PathRedactor::new(&[])),
            true,
        );
        decoder.consume(b"50.0%\r40.0%\rNaN%\r101.0%\r");
        decoder.finish();

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert_eq!(snapshot.progress.unwrap().percent, 50.0);
    }

    #[test]
    fn overlong_records_are_bounded_and_mark_the_transcript_truncated() {
        let phase = GogImportPhase::ExtractInstallerPayload;
        let (registry, id, reporter) = active_reporter(phase);
        let mut decoder = TerminalOutputDecoder::new(
            reporter,
            phase,
            GogImportOutputStream::Stdout,
            Arc::new(PathRedactor::new(&[])),
            false,
        );
        let mut record = vec![b'x'; MAX_PENDING_TERMINAL_BYTES + 1024];
        record.push(b'\n');
        decoder.consume(&record);
        decoder.finish();

        let snapshot = registry.snapshot(&id, 0).unwrap();
        assert!(snapshot.output_truncated);
        let output = output_texts(&snapshot);
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].len(), MAX_DISPLAY_ENTRY_BYTES);
    }

    #[test]
    fn diagnostic_scanning_remains_bounded_and_crosses_arbitrary_chunks() {
        let phase = GogImportPhase::TestInstallerIntegrity;
        let (registry, id, reporter) = active_reporter(phase);
        let decoder = TerminalOutputDecoder::new(
            reporter,
            phase,
            GogImportOutputStream::Stderr,
            Arc::new(PathRedactor::new(&[])),
            false,
        );
        let mut capture = StreamCapture::new(decoder);
        capture.consume(&vec![b'x'; MAX_CAPTURED_DIAGNOSTIC_BYTES]);
        capture.consume(b"War");
        capture.consume(b"ning: split diagnostic\n");
        let captured = capture.finish();

        assert_eq!(captured.bytes.len(), MAX_CAPTURED_DIAGNOSTIC_BYTES);
        assert_eq!(
            captured.total_bytes,
            MAX_CAPTURED_DIAGNOSTIC_BYTES + b"Warning: split diagnostic\n".len()
        );
        assert!(captured.suspicious);
        assert!(registry.snapshot(&id, 0).unwrap().output_truncated);
    }

    #[test]
    fn diagnostic_scan_is_literal_and_case_normalized_by_the_caller() {
        assert!(contains_ascii(b"done with 1 warning", b"warning"));
        assert!(!contains_ascii(b"all files verified", b"warning"));
    }
}
