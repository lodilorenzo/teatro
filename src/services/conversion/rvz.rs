//! GameCube ISO/GCM and Wii ISO/single-file WBFS imports through the pinned typed RVZ API.
use super::{InputFile, wua::safe_path};
use rom_converto_lib::{
    nintendo::{
        disc::{
            input::open_disc_input,
            rvz::{
                RvzCompressOptions, compress_disc, decompress_disc, verify::verify_rvz_structure,
            },
            wbfs::{
                WbfsReader,
                format::{WII_DUAL_LAYER_SIZE, disc_info_size, wbfs_sectors_per_disc},
            },
        },
        dol::{is_gamecube, models::boot_bin::GcBootBin},
        rvl::{is_wii, partition::read_partition_info},
    },
    util::{CancelToken, NoProgress, ProgressReporter},
};
use std::{
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

const CHUNK: u64 = 128 * 1024;
const MAX_DISC_BYTES: u64 = 1_459_978_240;
pub(super) struct Inventory {
    pub path: PathBuf,
    pub bytes: u64,
    pub logical_bytes: u64,
    pub wii: bool,
}

pub(super) fn inspect(root: &Path, inputs: &[InputFile], platform: &str) -> io::Result<Inventory> {
    if inputs.len() != 1 {
        return Err(io::Error::other("Select one disc at a time"));
    }
    let input = &inputs[0];
    safe_path(&input.path)?;
    let name = input.path.to_ascii_lowercase();
    let wii = platform == "wii";
    let wbfs = wii && name.ends_with(".wbfs");
    if !(name.ends_with(".iso") || if wii { wbfs } else { name.ends_with(".gcm") })
        || name.ends_with(".nkit.iso")
        || name.ends_with(".nkit.wbfs")
    {
        return Err(io::Error::other("Unsupported disc format"));
    }
    let path = root.join(&input.path);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file()
        || meta.len() != input.bytes
        || input.bytes
            > if wii {
                WII_DUAL_LAYER_SIZE
            } else {
                MAX_DISC_BYTES
            }
    {
        return Err(io::Error::other("Invalid disc image size"));
    }
    // Validate WBFS geometry before upstream shift/allocation and auto-reader dispatch.
    let logical_bytes;
    let mut header = [0; 0x440];
    if wbfs {
        validate_wbfs(&path, input.bytes)?;
        let mut reader = WbfsReader::open(&path).map_err(io::Error::other)?;
        logical_bytes = reader.disc_size();
        reader.read_exact(&mut header)?;
    } else {
        File::open(&path)?.read_exact(&mut header)?;
        logical_bytes = input.bytes;
    }
    let head: &[u8; 128] = header[..128].try_into().unwrap();
    if !(if wii {
        is_wii(head) && header[0x1c..0x20] == [0; 4]
    } else {
        is_gamecube(head) && header[0x18..0x1c] == [0; 4]
    }) || (!wbfs && &header[..4] == b"WBFS")
        || &header[0x200..0x204] == b"NKIT"
        || !header[..6].iter().all(u8::is_ascii_alphanumeric)
        || logical_bytes
            > if wii {
                WII_DUAL_LAYER_SIZE
            } else {
                MAX_DISC_BYTES
            }
    {
        return Err(io::Error::other(
            "Wrong platform or unsupported disc container",
        ));
    }
    if wii {
        // Identity and container geometry are checked before upstream auto-reader dispatch.
        let mut source = open_disc_input(&path).map_err(io::Error::other)?;
        validate_wii(&mut source, logical_bytes)?;
    } else {
        let boot = GcBootBin::read(&mut File::open(&path)?).map_err(io::Error::other)?;
        if boot.fst_offset < 0x440
            || boot.fst_size == 0
            || u64::from(boot.fst_offset) + u64::from(boot.fst_size) > input.bytes
        {
            return Err(io::Error::other("GameCube file table is out of bounds"));
        }
    }
    Ok(Inventory {
        path,
        bytes: input.bytes,
        logical_bytes,
        wii,
    })
}

fn be32(bytes: &[u8]) -> u64 {
    u32::from_be_bytes(bytes[..4].try_into().unwrap()).into()
}

fn validate_wbfs(path: &Path, bytes: u64) -> io::Result<()> {
    let mut file = File::open(path)?;
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[..4] != b"WBFS" || !(9..=13).contains(&header[8]) || !(15..=26).contains(&header[9])
    {
        return Err(io::Error::other("Invalid WBFS sector sizes"));
    }
    let hd_size = 1u64 << header[8];
    let block_size = 1u64 << header[9];
    let mut slots = vec![0; hd_size as usize - 12];
    file.read_exact(&mut slots)?;
    if slots.iter().filter(|&&slot| slot != 0).count() != 1 {
        return Err(io::Error::other("WBFS must contain exactly one disc"));
    }
    let slot = slots.iter().position(|&slot| slot != 0).unwrap() as u64;
    let info = hd_size + slot * disc_info_size(header[9], hd_size);
    let count = wbfs_sectors_per_disc(header[9]) as usize;
    if info + 0x100 + count as u64 * 2 > block_size || block_size > bytes {
        return Err(io::Error::other("WBFS metadata is out of bounds"));
    }
    file.seek(SeekFrom::Start(info))?;
    let mut disc_header = [0; 0x100];
    file.read_exact(&mut disc_header)?;
    let mut table = vec![0; count * 2];
    file.read_exact(&mut table)?;
    for entry in table.chunks_exact(2) {
        let block = u16::from_be_bytes(entry.try_into().unwrap()) as u64;
        if block != 0 && (block + 1) * block_size > bytes {
            return Err(io::Error::other("Truncated or split WBFS image"));
        }
    }
    let first = u16::from_be_bytes(table[..2].try_into().unwrap()) as u64;
    if first == 0 {
        return Err(io::Error::other("Missing WBFS disc header"));
    }
    file.seek(SeekFrom::Start(first * block_size))?;
    let mut actual = [0; 0x100];
    file.read_exact(&mut actual)?;
    if actual != disc_header {
        return Err(io::Error::other("WBFS disc identity mismatch"));
    }
    Ok(())
}

fn validate_wii(source: &mut (impl Read + Seek), bytes: u64) -> io::Result<()> {
    source.seek(SeekFrom::Start(0x40000))?;
    let mut groups = [0; 32];
    source.read_exact(&mut groups)?;
    let mut ranges = Vec::new();
    let mut game = false;
    for group in groups.chunks_exact(8) {
        let count = be32(group);
        let table = be32(&group[4..]) * 4;
        if count == 0 {
            continue;
        }
        if count > 16 || table < 0x40020 || table + count * 8 > bytes {
            return Err(io::Error::other("Wii partition table is out of bounds"));
        }
        source.seek(SeekFrom::Start(table))?;
        let mut entries = vec![0; count as usize * 8];
        source.read_exact(&mut entries)?;
        for entry in entries.chunks_exact(8) {
            let offset = be32(entry) * 4;
            let kind = be32(&entry[4..]) as u32;
            if offset < 0x40020 || offset + 0x2c0 > bytes {
                return Err(io::Error::other("Wii partition header is out of bounds"));
            }
            let part = read_partition_info(source, offset, 0, kind).map_err(io::Error::other)?;
            let end = part.data_start() + part.data_size;
            if part.data_offset < 0x2c0
                || part.data_start() % 0x8000 != 0
                || part.data_size == 0
                || part.data_size % 0x8000 != 0
                || end > bytes
                || ranges
                    .iter()
                    .any(|&(start, stop)| offset < stop && end > start)
            {
                return Err(io::Error::other(
                    "Invalid or overlapping Wii partition data",
                ));
            }
            ranges.push((offset, end));
            game |= kind == 0;
        }
    }
    if !game {
        return Err(io::Error::other("Missing Wii game partition"));
    }
    Ok(())
}

pub(super) struct Progress<F> {
    pub report: F,
    pub current: AtomicU64,
    pub total: AtomicU64,
}
impl<F: Fn(u64, u64) + Send + Sync> ProgressReporter for Progress<F> {
    fn start(&self, total: u64, _: &str) {
        self.current.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
        (self.report)(0, total);
    }
    fn inc(&self, delta: u64) {
        let current = self.current.fetch_add(delta, Ordering::Relaxed) + delta;
        (self.report)(current, self.total.load(Ordering::Relaxed));
    }
    fn finish(&self) {}
}

pub(super) async fn pack(
    inventory: Inventory,
    output: &Path,
    limit: u64,
    margin: u64,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(u64, u64) + Send + Sync,
    phase: impl Fn(&'static str),
) -> io::Result<u64> {
    // Bound the logical disc, not WBFS storage bytes. Wii hash exceptions need extra room.
    // 6.25% covers even all 47 hashes per sector differing, plus chunk padding/tables.
    let bound = inventory.logical_bytes
        + inventory.logical_bytes.div_ceil(CHUNK) * 64
        + if inventory.wii {
            inventory.logical_bytes.div_ceil(16)
        } else {
            0
        }
        + 1024 * 1024;
    let headroom = margin.saturating_add(8 * 1024 * 1024);
    if bound > limit
        || fs2::available_space(output.parent().unwrap())?
            < bound
                .saturating_add(inventory.logical_bytes)
                .saturating_add(headroom)
    {
        return Err(io::Error::other(
            "Not enough space or output allowance for RVZ",
        ));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::other("Cancelled"));
    }
    let token = CancelToken::new();
    let reporter = Progress {
        report: progress,
        current: AtomicU64::new(0),
        total: AtomicU64::new(inventory.logical_bytes),
    };
    let options = RvzCompressOptions {
        compression_level: 22,
        chunk_size: CHUNK as u32,
        use_rvz_packing: false,
    };
    // ponytail: upstream owns an all-core path writer; monitor space cooperatively until it exposes bounded writes/worker limits.
    let writer = compress_disc(&inventory.path, output, options, &reporter, token.clone());
    supervise(writer, output, &cancel, &token, headroom).await?;
    // The upstream future joins its blocking pipeline, including codec workers, before returning.
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::other("Cancelled"));
    }
    let bytes = fs::metadata(output)?.len();
    if bytes > bound || bytes > limit {
        return Err(io::Error::other("RVZ output limit reached"));
    }
    File::options().write(true).open(output)?.sync_all()?;
    phase("verifying");
    if !verify_rvz_structure(output, &token)
        .map_err(io::Error::other)?
        .ok()
    {
        return Err(io::Error::other("RVZ structural verification failed"));
    }
    // The upstream seek reader rebuilds all raw work items on each read, quadratic
    // for Wii-sized discs. Its full decoder streams once through the worker pool.
    let restored_path = output.with_extension("verified.iso");
    supervise(
        decompress_disc(output, &restored_path, &NoProgress, token.clone()),
        output,
        &cancel,
        &token,
        headroom,
    )
    .await?;
    let mut decoded = File::open(&restored_path)?;
    if decoded.metadata()?.len() != inventory.logical_bytes {
        return Err(io::Error::other("RVZ decoded size mismatch"));
    }
    let mut original = open_disc_input(&inventory.path).map_err(io::Error::other)?;
    let mut source = vec![0; CHUNK as usize];
    let mut restored = vec![0; CHUNK as usize];
    let mut remaining = inventory.logical_bytes;
    while remaining > 0 {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Cancelled"));
        }
        let count = remaining.min(CHUNK) as usize;
        original.read_exact(&mut source[..count])?;
        decoded.read_exact(&mut restored[..count])?;
        if source[..count] != restored[..count] {
            return Err(io::Error::other("RVZ decoded content mismatch"));
        }
        remaining -= count as u64;
    }
    if original.read(&mut source[..1])? != 0 || decoded.read(&mut restored[..1])? != 0 {
        return Err(io::Error::other("Disc size changed"));
    }
    drop(decoded);
    fs::remove_file(restored_path)?;
    Ok(bytes)
}

// Both codec futures join their blocking workers before returning, even on cancellation.
pub(super) async fn supervise<E: std::error::Error + Send + Sync + 'static>(
    work: impl std::future::Future<Output = Result<(), E>>,
    output: &Path,
    cancel: &AtomicBool,
    token: &CancelToken,
    headroom: u64,
) -> io::Result<()> {
    tokio::pin!(work);
    let mut interval = tokio::time::interval(Duration::from_millis(100));
    loop {
        tokio::select! {
            result = &mut work => return result.map_err(io::Error::other),
            _ = interval.tick() => {
                if cancel.load(Ordering::Relaxed)
                    || fs2::available_space(output.parent().unwrap()).map_or(true, |space| space < headroom)
                { token.cancel(); }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn output_allowance_and_cancellation_fail_before_verification() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("disc.iso");
        let output = dir.path().join("disc.rvz");
        let mut disc = vec![42; 4 * 1024 * 1024];
        disc[..128].fill(0);
        disc[0x1c..0x20].copy_from_slice(&0xc2339f3du32.to_be_bytes());
        fs::write(&input, &disc).unwrap();
        let inventory = || Inventory {
            path: input.clone(),
            bytes: disc.len() as u64,
            logical_bytes: disc.len() as u64,
            wii: false,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        assert!(
            pack(
                inventory(),
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
        assert!(!output.exists());
        let verifying = AtomicBool::new(false);
        // Cancellation starts after the codec span starts, not before pack admission.
        assert!(
            pack(
                inventory(),
                &output,
                8 * 1024 * 1024,
                0,
                cancel.clone(),
                |_, _| cancel.store(true, Ordering::Relaxed),
                |_| verifying.store(true, Ordering::Relaxed),
            )
            .await
            .is_err()
        );
        assert!(!verifying.load(Ordering::Relaxed));
        cancel.store(false, Ordering::Relaxed);
        assert!(
            pack(
                inventory(),
                &output,
                8 * 1024 * 1024,
                0,
                cancel.clone(),
                |_, _| {},
                |_| {
                    verifying.store(true, Ordering::Relaxed);
                    cancel.store(true, Ordering::Relaxed);
                },
            )
            .await
            .is_err()
        );
        assert!(
            verifying.load(Ordering::Relaxed),
            "verification cancellation must join the decoder"
        );
        assert!(
            fs::read_dir(dir.path()).unwrap().all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")),
            "upstream scratch writers must be joined before cleanup"
        );
    }
}
