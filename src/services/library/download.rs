use std::{
    collections::{BTreeSet, HashMap},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    pin::Pin,
    sync::Mutex,
    task::{Context, Poll},
    time::{Duration, Instant},
};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::OwnedSemaphorePermit,
};
use zip::{CompressionMethod, DateTime, ZipWriter, write::SimpleFileOptions};

use crate::{
    repositories::roms,
    state::AppState,
    storage::{
        file_store::FileStoreError,
        paths::{PathSafetyError, safe_content_disposition_filename},
    },
};

const ARCHIVE_ENTRY_OVERHEAD_BYTES: u64 = 1_024;
const ARCHIVE_FIXED_OVERHEAD_BYTES: u64 = 65_536;
const MAX_ARCHIVE_ENTRY_NAME_BYTES: usize = 255;
const DOWNLOAD_TICKET_PREFIX: &str = "teatro_dl_";
const DOWNLOAD_TICKET_RANDOM_BYTES: usize = 32;
const MAX_ACTIVE_DOWNLOAD_TICKETS: usize = 64;
pub(crate) const DOWNLOAD_TICKET_TTL_SECONDS: u64 = 60;

// ponytail: one-minute browser handoffs do not need durable ticket storage; restarts invalidate them safely.
pub(crate) struct DownloadTicketRegistry<T> {
    tickets: Mutex<HashMap<[u8; 32], DownloadTicket<T>>>,
}

impl<T> Default for DownloadTicketRegistry<T> {
    fn default() -> Self {
        Self {
            tickets: Mutex::new(HashMap::new()),
        }
    }
}

struct DownloadTicket<T> {
    download: T,
    expires_at: Instant,
}

#[derive(Debug, Error)]
pub(crate) enum DownloadTicketError {
    #[error("too many download tickets are active; retry shortly")]
    Busy,
}

impl<T> DownloadTicketRegistry<T> {
    pub(crate) fn issue(&self, download: T) -> Result<String, DownloadTicketError> {
        self.issue_at(download, Instant::now())
    }

    fn issue_at(&self, download: T, now: Instant) -> Result<String, DownloadTicketError> {
        let mut random_bytes = [0_u8; DOWNLOAD_TICKET_RANDOM_BYTES];
        OsRng.fill_bytes(&mut random_bytes);
        let ticket = format!(
            "{DOWNLOAD_TICKET_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(random_bytes)
        );
        let ticket_hash = hash_download_ticket(&ticket);

        let mut tickets = self.tickets.lock().expect("download ticket mutex poisoned");
        tickets.retain(|_, record| record.expires_at > now);
        if tickets.len() >= MAX_ACTIVE_DOWNLOAD_TICKETS || tickets.contains_key(&ticket_hash) {
            return Err(DownloadTicketError::Busy);
        }
        tickets.insert(
            ticket_hash,
            DownloadTicket {
                download,
                expires_at: now + Duration::from_secs(DOWNLOAD_TICKET_TTL_SECONDS),
            },
        );
        Ok(ticket)
    }

    pub(crate) fn consume(&self, ticket: &str) -> Option<T> {
        self.consume_at(ticket, Instant::now())
    }

    fn consume_at(&self, ticket: &str, now: Instant) -> Option<T> {
        if !valid_download_ticket(ticket) {
            return None;
        }
        let ticket_hash = hash_download_ticket(ticket);
        let mut tickets = self.tickets.lock().expect("download ticket mutex poisoned");
        tickets.retain(|_, record| record.expires_at > now);
        tickets.remove(&ticket_hash).map(|record| record.download)
    }

    pub(crate) fn prune_expired(&self) {
        self.prune_expired_at(Instant::now());
    }

    fn prune_expired_at(&self, now: Instant) {
        self.tickets
            .lock()
            .expect("download ticket mutex poisoned")
            .retain(|_, record| record.expires_at > now);
    }
}

fn valid_download_ticket(ticket: &str) -> bool {
    let Some(random_part) = ticket.strip_prefix(DOWNLOAD_TICKET_PREFIX) else {
        return false;
    };
    random_part.len() == 43
        && random_part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn hash_download_ticket(ticket: &str) -> [u8; 32] {
    Sha256::digest(ticket.as_bytes()).into()
}

pub(crate) struct PreparedDownloadArchive {
    pub(crate) reader: DownloadArchiveReader,
    pub(crate) file_name: String,
    pub(crate) file_size_bytes: u64,
}

pub(crate) struct DownloadArchiveReader {
    file: tokio::fs::File,
    _permit: OwnedSemaphorePermit,
}

impl AsyncRead for DownloadArchiveReader {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().file).poll_read(context, buffer)
    }
}

#[derive(Debug, Error)]
pub(crate) enum DownloadArchiveError {
    #[error("ROM not found")]
    RomNotFound,

    #[error("archive downloads require a ROM with more than one file")]
    RequiresMultipleFiles,

    #[error("the ROM has more than the configured {max_files} archive files")]
    TooManyFiles { max_files: usize },

    #[error("the ROM exceeds the configured {max_bytes}-byte archive source limit")]
    TooLarge { max_bytes: u64 },

    #[error("another game archive is already being prepared; retry shortly")]
    Busy,

    #[error("there is not enough free space to prepare the game archive")]
    InsufficientStorage,

    #[error("the ROM contains an invalid or colliding archive filename")]
    InvalidFileName,

    #[error("a ROM file changed while its archive was being prepared")]
    SourceChanged,

    #[error("the archive worker failed")]
    WorkerFailed,

    #[error(transparent)]
    PathSafety(#[from] PathSafetyError),

    #[error(transparent)]
    FileStore(#[from] FileStoreError),

    #[error(transparent)]
    Database(#[from] sqlx::Error),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Zip(#[from] zip::result::ZipError),
}

struct ArchiveSource {
    file_name: String,
    expected_bytes: u64,
    file: File,
}

pub(crate) async fn prepare_download_archive(
    state: &AppState,
    rom_id: i64,
) -> Result<PreparedDownloadArchive, DownloadArchiveError> {
    let rom = roms::find_by_id(state.db(), rom_id)
        .await?
        .ok_or(DownloadArchiveError::RomNotFound)?;
    let config = state.config().download_archives;

    if rom.files.len() < 2 {
        return Err(DownloadArchiveError::RequiresMultipleFiles);
    }
    if rom.files.len() > config.max_files {
        return Err(DownloadArchiveError::TooManyFiles {
            max_files: config.max_files,
        });
    }

    let mut expected_total_bytes = 0_u64;
    let mut names = BTreeSet::new();
    for file in &rom.files {
        validate_file_name(&file.file_name, &mut names)?;
        let file_bytes =
            u64::try_from(file.file_size_bytes).map_err(|_| DownloadArchiveError::SourceChanged)?;
        expected_total_bytes =
            expected_total_bytes
                .checked_add(file_bytes)
                .ok_or(DownloadArchiveError::TooLarge {
                    max_bytes: config.max_source_bytes,
                })?;
    }
    if expected_total_bytes > config.max_source_bytes {
        return Err(DownloadArchiveError::TooLarge {
            max_bytes: config.max_source_bytes,
        });
    }

    let permit = state
        .try_acquire_download_archive_permit()
        .ok_or(DownloadArchiveError::Busy)?;
    ensure_archive_space(
        &state.config().data_dir,
        expected_total_bytes,
        rom.files.len(),
        config.free_space_margin_bytes,
    )?;

    let mut sources = Vec::with_capacity(rom.files.len());
    for file in rom.files {
        let opened = state
            .file_store()
            .open_existing(&file.root_path, &file.relative_path)
            .await?;
        let actual_bytes = opened.metadata().await?.len();
        let expected_bytes =
            u64::try_from(file.file_size_bytes).map_err(|_| DownloadArchiveError::SourceChanged)?;
        if actual_bytes != expected_bytes {
            return Err(DownloadArchiveError::SourceChanged);
        }
        sources.push(ArchiveSource {
            file_name: file.file_name,
            expected_bytes,
            file: opened.into_std().await,
        });
    }

    let archive_name = format!(
        "{}.zip",
        safe_content_disposition_filename(rom.name.trim_end_matches(".zip"))
    );
    let data_dir = state.config().data_dir.clone();
    let task = tokio::task::spawn_blocking(move || {
        let output = tempfile::tempfile_in(data_dir)?;
        let (file, file_size_bytes) = build_archive(output, sources)?;
        Ok::<_, DownloadArchiveError>((file, file_size_bytes, permit))
    });
    let (file, file_size_bytes, permit) = task.await.map_err(|error| {
        tracing::error!(rom_id, ?error, "download archive worker failed");
        DownloadArchiveError::WorkerFailed
    })??;

    tracing::info!(
        rom_id,
        source_file_count = names.len(),
        source_bytes = expected_total_bytes,
        archive_bytes = file_size_bytes,
        "prepared temporary game download archive"
    );

    Ok(PreparedDownloadArchive {
        reader: DownloadArchiveReader {
            file: tokio::fs::File::from_std(file),
            _permit: permit,
        },
        file_name: archive_name,
        file_size_bytes,
    })
}

fn ensure_archive_space(
    data_dir: &Path,
    source_bytes: u64,
    file_count: usize,
    free_space_margin_bytes: u64,
) -> Result<(), DownloadArchiveError> {
    let entry_overhead = u64::try_from(file_count)
        .unwrap_or(u64::MAX)
        .saturating_mul(ARCHIVE_ENTRY_OVERHEAD_BYTES);
    let required_bytes = source_bytes
        .saturating_add(entry_overhead)
        .saturating_add(ARCHIVE_FIXED_OVERHEAD_BYTES)
        .saturating_add(free_space_margin_bytes);
    if fs2::available_space(data_dir)? < required_bytes {
        return Err(DownloadArchiveError::InsufficientStorage);
    }
    Ok(())
}

fn validate_file_name(
    file_name: &str,
    case_folded_names: &mut BTreeSet<String>,
) -> Result<(), DownloadArchiveError> {
    if file_name.is_empty()
        || file_name == "."
        || file_name == ".."
        || file_name.len() > MAX_ARCHIVE_ENTRY_NAME_BYTES
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.chars().any(char::is_control)
        || Path::new(file_name).components().count() != 1
        || !case_folded_names.insert(file_name.to_lowercase())
    {
        return Err(DownloadArchiveError::InvalidFileName);
    }
    Ok(())
}

fn build_archive(
    output: File,
    sources: Vec<ArchiveSource>,
) -> Result<(File, u64), DownloadArchiveError> {
    let timestamp = DateTime::default();
    let mut zip = ZipWriter::new(output);
    for source in sources {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(timestamp)
            .large_file(source.expected_bytes > u32::MAX as u64)
            .unix_permissions(0o644);
        zip.start_file(&source.file_name, options)?;
        let mut bounded = source.file.take(source.expected_bytes.saturating_add(1));
        let copied = io::copy(&mut bounded, &mut zip)?;
        if copied != source.expected_bytes {
            return Err(DownloadArchiveError::SourceChanged);
        }
    }

    let mut output = zip.finish()?;
    output.sync_all()?;
    let file_size_bytes = output.metadata()?.len();
    output.seek(SeekFrom::Start(0))?;
    Ok((output, file_size_bytes))
}

#[cfg(test)]
mod tests {
    use std::{
        fs::File,
        io::{Cursor, Read},
        sync::Arc,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;
    use tokio::sync::Semaphore;
    use zip::ZipArchive;

    use super::{
        ArchiveSource, DOWNLOAD_TICKET_TTL_SECONDS, DownloadArchiveError, DownloadArchiveReader,
        DownloadTicketRegistry, PreparedDownloadArchive, build_archive, validate_file_name,
    };

    fn prepared_archive() -> PreparedDownloadArchive {
        let permit = Arc::new(Semaphore::new(1)).try_acquire_owned().unwrap();
        PreparedDownloadArchive {
            reader: DownloadArchiveReader {
                file: tokio::fs::File::from_std(tempfile::tempfile().unwrap()),
                _permit: permit,
            },
            file_name: "Game.zip".to_string(),
            file_size_bytes: 0,
        }
    }

    #[test]
    fn download_tickets_are_single_use_and_expire() {
        let registry = DownloadTicketRegistry::default();
        let now = Instant::now();
        let ticket = registry.issue_at(prepared_archive(), now).unwrap();

        assert!(registry.consume_at(&ticket, now).is_some());
        assert!(registry.consume_at(&ticket, now).is_none());

        let expired = registry.issue_at(prepared_archive(), now).unwrap();
        assert!(
            registry
                .consume_at(
                    &expired,
                    now + Duration::from_secs(DOWNLOAD_TICKET_TTL_SECONDS),
                )
                .is_none()
        );
    }

    #[test]
    fn tickets_are_bounded_hashed_and_drop_expired_resources() {
        let registry = DownloadTicketRegistry::default();
        let now = Instant::now();
        let resource = Arc::new(());
        let mut tickets = Vec::new();
        for _ in 0..super::MAX_ACTIVE_DOWNLOAD_TICKETS {
            tickets.push(registry.issue_at(resource.clone(), now).unwrap());
        }
        assert!(registry.issue_at(resource.clone(), now).is_err());
        assert_eq!(Arc::strong_count(&resource), 65);
        assert!(
            registry
                .tickets
                .lock()
                .unwrap()
                .contains_key(&super::hash_download_ticket(&tickets[0]))
        );
        let other = DownloadTicketRegistry::<Arc<()>>::default();
        assert!(other.consume_at(&tickets[0], now).is_none());
        let consumed = registry.consume_at(&tickets[0], now).unwrap();
        assert!(registry.consume_at(&tickets[0], now).is_none());
        registry.prune_expired_at(now + Duration::from_secs(DOWNLOAD_TICKET_TTL_SECONDS));
        assert!(registry.tickets.lock().unwrap().is_empty());
        assert_eq!(Arc::strong_count(&resource), 2);
        drop(consumed);
        assert_eq!(Arc::strong_count(&resource), 1);
    }

    #[test]
    fn builds_a_flat_stored_archive_in_input_order() {
        let temp = TempDir::new().unwrap();
        let first_path = temp.path().join("Game.m3u");
        let second_path = temp.path().join("Disc 1.chd");
        std::fs::write(&first_path, b"Disc 1.chd\n").unwrap();
        std::fs::write(&second_path, b"disc bytes").unwrap();
        let output = tempfile::tempfile_in(temp.path()).unwrap();

        let (mut output, _) = build_archive(
            output,
            vec![
                ArchiveSource {
                    file_name: "Game.m3u".to_string(),
                    expected_bytes: 11,
                    file: File::open(first_path).unwrap(),
                },
                ArchiveSource {
                    file_name: "Disc 1.chd".to_string(),
                    expected_bytes: 10,
                    file: File::open(second_path).unwrap(),
                },
            ],
        )
        .unwrap();
        let mut bytes = Vec::new();
        output.read_to_end(&mut bytes).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(bytes)).unwrap();

        assert_eq!(archive.len(), 2);
        assert_eq!(archive.by_index(0).unwrap().name(), "Game.m3u");
        let mut disc = archive.by_index(1).unwrap();
        assert_eq!(disc.name(), "Disc 1.chd");
        let mut contents = Vec::new();
        disc.read_to_end(&mut contents).unwrap();
        assert_eq!(contents, b"disc bytes");
    }

    #[test]
    fn rejects_changed_sources_and_case_colliding_names() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("game.bin");
        std::fs::write(&path, b"too long").unwrap();
        let error = build_archive(
            tempfile::tempfile_in(temp.path()).unwrap(),
            vec![ArchiveSource {
                file_name: "game.bin".to_string(),
                expected_bytes: 3,
                file: File::open(path).unwrap(),
            }],
        )
        .unwrap_err();
        assert!(matches!(error, DownloadArchiveError::SourceChanged));

        let mut names = std::collections::BTreeSet::new();
        validate_file_name("Game.bin", &mut names).unwrap();
        assert!(matches!(
            validate_file_name("game.BIN", &mut names),
            Err(DownloadArchiveError::InvalidFileName)
        ));
    }
}
