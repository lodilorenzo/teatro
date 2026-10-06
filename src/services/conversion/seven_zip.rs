use super::{InputFile, wua::BoundedOutput};
use sevenz_rust2::{ArchiveEntry, ArchiveReader, ArchiveWriter, Password};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

// Explicit native-ROM extensions keep disc images, archives and multi-file games out.
pub(crate) const PLATFORMS: &[(&str, &[&str])] = &[
    ("atarijaguar", &["j64", "jag", "rom", "abs", "cof", "bin"]),
    ("atarilynx", &["lnx", "lyx"]),
    ("colecovision", &["col", "rom", "bin"]),
    ("famicom", &["nes", "unf", "unif"]),
    ("nes", &["nes", "unf", "unif"]),
    ("gamegear", &["gg", "bin"]),
    ("gb", &["gb"]),
    ("gbc", &["gb", "gbc"]),
    ("gba", &["gba"]),
    ("genesis", &["md", "gen", "smd", "bin"]),
    ("megadrive", &["md", "gen", "smd", "bin"]),
    ("intellivision", &["int", "rom", "bin"]),
    ("mastersystem", &["sms", "bin"]),
    ("n3ds", &["3ds", "cci", "cxi", "cia"]),
    ("n64", &["z64", "n64", "v64"]),
    ("nds", &["nds"]),
    ("ngp", &["ngp", "npc"]),
    ("ngpc", &["ngp", "ngc", "npc"]),
    ("pcengine", &["pce"]),
    ("tg16", &["pce"]),
    ("supergrafx", &["pce", "sgx"]),
    ("satellaview", &["bs", "sfc", "smc"]),
    ("sfc", &["sfc", "smc", "swc", "fig"]),
    ("snes", &["sfc", "smc", "swc", "fig"]),
    ("snesna", &["sfc", "smc", "swc", "fig"]),
    ("sgb", &["gb", "gbc"]),
    ("sega32x", &["32x", "bin"]),
    ("sega32xjp", &["32x", "bin"]),
    ("sega32xna", &["32x", "bin"]),
    ("switch", &["xci", "nsp"]),
    ("virtualboy", &["vb", "vboy", "bin"]),
    ("wonderswan", &["ws"]),
    ("wonderswancolor", &["ws", "wsc"]),
];

pub(super) struct Inventory {
    path: PathBuf,
    name: String,
    pub bytes: u64,
}

pub(super) fn inspect(root: &Path, inputs: &[InputFile], platform: &str) -> io::Result<Inventory> {
    let invalid =
        || io::Error::other("Expected one uncompressed single-file ROM for a non-disc platform");
    let [input] = inputs else {
        return Err(invalid());
    };
    super::wua::safe_path(&input.path)?;
    let extensions = PLATFORMS
        .iter()
        .find(|&&(slug, _)| slug == platform)
        .ok_or_else(invalid)?
        .1;
    let extension = Path::new(&input.path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if input.path.contains('/') || !extensions.contains(&extension.as_str()) || input.bytes == 0 {
        return Err(invalid());
    }
    let path = root.join(&input.path);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.is_file() || meta.len() != input.bytes {
        return Err(invalid());
    }
    let mut header = [0; 8];
    let n = File::open(&path)?.read(&mut header)?;
    if [
        b"PK\x03\x04".as_slice(),
        b"PK\x05\x06",
        b"7z\xbc\xaf\x27\x1c",
        b"Rar!",
        b"\x1f\x8b",
        b"BZh",
        b"\xfd7zXZ\0",
        b"\x28\xb5\x2f\xfd",
    ]
    .iter()
    .any(|magic| header[..n].starts_with(magic))
    {
        return Err(invalid());
    }
    Ok(Inventory {
        path,
        name: input.path.clone(),
        bytes: input.bytes,
    })
}

struct CheckedInput<'a> {
    file: File,
    hash: Sha256,
    bytes: u64,
    total: u64,
    cancel: &'a AtomicBool,
    progress: &'a dyn Fn(u64, u64),
}
impl Read for CheckedInput<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Cancelled"));
        }
        let n = self.file.read(buffer)?;
        self.bytes += n as u64;
        if self.bytes > self.total || (n == 0 && self.bytes != self.total) {
            return Err(io::Error::other("Input changed"));
        }
        self.hash.update(&buffer[..n]);
        (self.progress)(self.bytes, self.total);
        Ok(n)
    }
}

pub(super) fn pack(
    inventory: Inventory,
    output: &Path,
    limit: u64,
    margin: u64,
    cancel: Arc<AtomicBool>,
    progress: impl Fn(u64, u64),
    phase: impl Fn(&'static str),
) -> io::Result<u64> {
    let sink = BoundedOutput {
        file: fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)?,
        path: output.to_path_buf(),
        written: 0,
        limit,
        margin,
        cancel: cancel.clone(),
    };
    let mut archive = ArchiveWriter::new(sink).map_err(io::Error::other)?;
    let mut source = CheckedInput {
        file: File::open(&inventory.path)?,
        hash: Sha256::new(),
        bytes: 0,
        total: inventory.bytes,
        cancel: &cancel,
        progress: &progress,
    };
    archive
        .push_archive_entry(
            ArchiveEntry::from_path(&inventory.path, inventory.name.clone()),
            Some(&mut source),
        )
        .map_err(io::Error::other)?;
    archive.finish().map_err(io::Error::other)?;
    let expected = source.hash.finalize();
    phase("verifying");
    let mut archive = ArchiveReader::open(output, Password::empty()).map_err(io::Error::other)?;
    let mut entries = 0;
    archive
        .for_each_entries(|entry, reader| {
            entries += 1;
            if entries != 1
                || entry.name() != inventory.name
                || entry.is_directory()
                || entry.size() != inventory.bytes
            {
                return Err(io::Error::other("Archive inventory changed").into());
            }
            let mut hash = Sha256::new();
            let mut bytes = 0_u64;
            let mut buffer = [0; 64 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(io::Error::other("Cancelled").into());
                }
                let n = reader.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                bytes += n as u64;
                if bytes > inventory.bytes {
                    return Err(io::Error::other("Invalid decoded size").into());
                }
                hash.update(&buffer[..n]);
                progress(bytes, inventory.bytes);
            }
            if bytes != inventory.bytes || hash.finalize() != expected {
                return Err(io::Error::other("Archive verification failed").into());
            }
            Ok(true)
        })
        .map_err(io::Error::other)?;
    if entries != 1 {
        return Err(io::Error::other("Archive is empty"));
    }
    Ok(fs::metadata(output)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn single_rom_roundtrip_and_guards() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = vec![42; 128 * 1024];
        fs::write(dir.path().join("Game.NES"), &bytes).unwrap();
        let inputs = vec![InputFile {
            path: "Game.NES".into(),
            bytes: bytes.len() as u64,
        }];
        assert!(inspect(dir.path(), &inputs, "psx").is_err());
        assert!(inspect(dir.path(), &inputs, "fds").is_err());
        assert!(inspect(dir.path(), &[inputs[0].clone(), inputs[0].clone()], "nes").is_err());
        let output = dir.path().join("game.7z");
        let inventory = inspect(dir.path(), &inputs, "nes").unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let size = pack(
            inventory,
            &output,
            1024 * 1024,
            0,
            cancel.clone(),
            |_, _| {},
            |_| {},
        )
        .unwrap();
        assert!(size < bytes.len() as u64);
        let mut decoded = Vec::new();
        ArchiveReader::open(&output, Password::empty())
            .unwrap()
            .for_each_entries(|entry, reader| {
                assert_eq!(entry.name(), "Game.NES");
                reader.read_to_end(&mut decoded)?;
                Ok(true)
            })
            .unwrap();
        assert_eq!(decoded, bytes);
        for (name, limit, cancelled) in [
            ("limited.7z", 10, false),
            ("cancelled.7z", 1024 * 1024, true),
        ] {
            cancel.store(cancelled, Ordering::Relaxed);
            assert!(
                pack(
                    inspect(dir.path(), &inputs, "nes").unwrap(),
                    &dir.path().join(name),
                    limit,
                    0,
                    cancel.clone(),
                    |_, _| {},
                    |_| {}
                )
                .is_err()
            );
        }
        fs::write(dir.path().join("Game.NES"), b"7z\xbc\xaf\x27\x1c").unwrap();
        assert!(
            inspect(
                dir.path(),
                &[InputFile {
                    path: "Game.NES".into(),
                    bytes: 6
                }],
                "nes"
            )
            .is_err()
        );
    }
}
