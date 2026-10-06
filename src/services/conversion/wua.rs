//! Bounded, key-free WUA packing with the upstream ZArchive codec.
//! Paths come only from the private upload inventory, never an upstream tree walker.
use rom_converto_lib::{
    nintendo::wup::LoadiineTitle,
    zar::{ZarReader, ZarWriter},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) const MAX_FILES: usize = 20_000;
const MAX_XML: u64 = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct InputFile {
    pub path: String,
    pub bytes: u64,
}
pub(super) struct Inventory {
    pub files: Vec<(String, PathBuf, u64)>,
    pub bytes: u64,
}

pub(super) fn safe_path(path: &str) -> io::Result<()> {
    // ponytail: ASCII paths avoid cross-platform Unicode-normalization collisions; widen only with a normalized-path policy.
    if path.is_empty() || path.len() > 240 || !path.is_ascii() || path.contains(['\\', ':']) {
        return Err(io::Error::other(
            "Use relative ASCII paths of at most 240 bytes",
        ));
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > 32
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || p.ends_with(['.', ' '])
                || p.bytes()
                    .any(|b| b < 32 || b == 127 || b"<>\"|?*".contains(&b))
        })
    {
        return Err(io::Error::other("Unsafe relative path"));
    }
    for part in parts {
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err(io::Error::other("Reserved filename"));
        }
    }
    Ok(())
}

fn title_header(xml: &str) -> io::Result<(u64, u32)> {
    #[derive(Deserialize)]
    struct Header {
        title_id: String,
        title_version: Version,
    }
    #[derive(Deserialize)]
    struct Version {
        #[serde(rename = "@type")]
        encoding: Option<String>,
        #[serde(rename = "$text")]
        value: String,
    }
    let header: Header = quick_xml::de::from_str(xml)
        .map_err(|_| io::Error::other("Missing title identity or version"))?;
    let radix = match header.title_version.encoding.as_deref() {
        Some("hexBinary") => 16,
        None | Some("unsignedInt") => 10,
        _ => return Err(io::Error::other("Unsupported title version encoding")),
    };
    let id = u64::from_str_radix(header.title_id.trim(), 16)
        .map_err(|_| io::Error::other("Invalid title identity"))?;
    let version = u32::from_str_radix(header.title_version.value.trim(), radix)
        .map_err(|_| io::Error::other("Invalid title version"))?;
    Ok((id, version))
}

pub(super) fn inspect(root: &Path, inputs: &[InputFile]) -> io::Result<Inventory> {
    let mut roots = BTreeSet::new();
    let mut bytes = 0_u64;
    for file in inputs {
        safe_path(&file.path)?;
        bytes = bytes
            .checked_add(file.bytes)
            .ok_or_else(|| io::Error::other("Input too large"))?;
        let parts: Vec<_> = file.path.split('/').collect();
        let at = parts
            .iter()
            .position(|p| ["code", "content", "meta"].contains(p))
            .ok_or_else(|| {
                io::Error::other("Only complete code/content/meta title trees are accepted")
            })?;
        if at + 1 == parts.len() {
            return Err(io::Error::other("Incomplete title path"));
        }
        roots.insert(parts[..at].join("/"));
        let metadata = fs::symlink_metadata(root.join(&file.path))?;
        if !metadata.is_file() || metadata.len() != file.bytes || metadata.file_type().is_symlink()
        {
            return Err(io::Error::other("Staged file changed"));
        }
    }
    if roots.is_empty() || roots.len() > 3 {
        return Err(io::Error::other(
            "Select one base game and at most one update and DLC",
        ));
    }
    let mut folders = BTreeMap::new();
    let mut family = None;
    let mut kinds = BTreeSet::new();
    for prefix in roots {
        let dir = root.join(&prefix);
        for xml in ["code/app.xml", "code/cos.xml", "meta/meta.xml"] {
            if fs::metadata(dir.join(xml))?.len() > MAX_XML {
                return Err(io::Error::other("Title XML exceeds 1 MiB"));
            }
            let text = fs::read_to_string(dir.join(xml))?;
            let mut reader = quick_xml::Reader::from_str(&text);
            reader.config_mut().check_comments = true;
            let mut saw_root = false;
            use quick_xml::events::Event;
            loop {
                match reader.read_event() {
                    Ok(Event::Start(start)) if !saw_root => {
                        saw_root = true;
                        reader
                            .read_to_end(start.name())
                            .map_err(|_| io::Error::other("Invalid title XML"))?;
                    }
                    Ok(Event::Empty(_)) if !saw_root => saw_root = true,
                    Ok(Event::Eof) if saw_root => break,
                    Ok(Event::Text(value))
                        if value
                            .iter()
                            .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n')) => {}
                    Ok(Event::Decl(_) | Event::PI(_) | Event::Comment(_) | Event::DocType(_)) => {}
                    _ => return Err(io::Error::other("Invalid title XML")),
                }
            }
        }
        if !dir.join("content").is_dir() {
            return Err(io::Error::other("Title content directory is missing"));
        }
        // Upstream's detector parses versions as decimal even for hexBinary fields.
        let (title_id, title_version) =
            title_header(&fs::read_to_string(dir.join("code/app.xml"))?)?;
        let title = LoadiineTitle {
            dir: dir.clone(),
            title_id,
            title_version,
        };
        let kind = match title.title_id >> 32 {
            0x0005_0000 => "base",
            0x0005_000e => "update",
            0x0005_000c => "dlc",
            _ => return Err(io::Error::other("Unsupported Wii U title type")),
        };
        let low = title.title_id as u32;
        let (meta_id, meta_version) =
            title_header(&fs::read_to_string(dir.join("meta/meta.xml"))?)?;
        // Update menus may identify the base game while app.xml identifies the patch.
        let base_id = 0x0005_0000_0000_0000 | u64::from(low);
        if (meta_id != title.title_id && !(kind == "update" && meta_id == base_id))
            || meta_version != title.title_version
        {
            return Err(io::Error::other("Conflicting title identity or version"));
        }
        if family.is_some_and(|value| value != low) || !kinds.insert(kind) {
            return Err(io::Error::other(
                "Titles must be one matching base/update/DLC family",
            ));
        }
        family = Some(low);
        folders.insert(prefix, title.archive_folder());
    }
    if !kinds.contains("base") {
        return Err(io::Error::other("A base game is required"));
    }
    let mut files = Vec::new();
    for input in inputs {
        let parts: Vec<_> = input.path.split('/').collect();
        let at = parts
            .iter()
            .position(|p| ["code", "content", "meta"].contains(p))
            .unwrap();
        let archive_path = format!(
            "{}/{}",
            folders[&parts[..at].join("/")],
            parts[at..].join("/")
        );
        files.push((archive_path, root.join(&input.path), input.bytes));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(Inventory { files, bytes })
}

pub(super) struct BoundedOutput {
    pub(super) file: File,
    pub(super) path: PathBuf,
    pub(super) written: u64,
    pub(super) limit: u64,
    pub(super) margin: u64,
    pub(super) cancel: Arc<AtomicBool>,
}
impl std::io::Seek for BoundedOutput {
    fn seek(&mut self, position: std::io::SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
impl Write for BoundedOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(io::Error::other("Cancelled"));
        }
        if self.written.saturating_add(bytes.len() as u64) > self.limit
            || fs2::available_space(&self.path)? < self.margin.saturating_add(bytes.len() as u64)
        {
            return Err(io::Error::other("Conversion output space limit reached"));
        }
        let n = self.file.write(bytes)?;
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
struct HashSink {
    hash: Sha256,
    bytes: u64,
    limit: u64,
    cancel: Arc<AtomicBool>,
}
impl Write for HashSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed)
            || self.bytes.saturating_add(bytes.len() as u64) > self.limit
        {
            return Err(io::Error::other("Cancelled or invalid decoded size"));
        }
        self.hash.update(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
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
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    let sink = BoundedOutput {
        file,
        path: output.to_path_buf(),
        written: 0,
        limit,
        margin,
        cancel: cancel.clone(),
    };
    // Use the public typed writer to enforce per-write limits and one codec worker, unlike the all-core unbounded path wrapper.
    let mut archive = ZarWriter::with_options(sink, 1, 22, None).map_err(io::Error::other)?;
    let mut digests = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut current = 0_u64;
    for (name, path, expected) in &inventory.files {
        archive.start_file(name).map_err(io::Error::other)?;
        let mut source = File::open(path)?;
        let mut hash = Sha256::new();
        let mut copied = 0_u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::other("Cancelled"));
            }
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > *expected {
                return Err(io::Error::other("Input changed"));
            }
            archive
                .append_data(&buffer[..n])
                .map_err(io::Error::other)?;
            hash.update(&buffer[..n]);
            current += n as u64;
            progress(current, inventory.bytes);
        }
        if copied != *expected {
            return Err(io::Error::other("Input changed"));
        }
        digests.push(hash.finalize());
    }
    phase("finalizing_wua");
    archive.finish().map_err(io::Error::other)?;
    fs::OpenOptions::new()
        .write(true)
        .open(output)?
        .sync_all()?;
    phase("verifying");
    let mut reader = ZarReader::open(File::open(output)?).map_err(io::Error::other)?;
    if reader
        .entries()
        .map_err(io::Error::other)?
        .iter()
        .filter(|e| e.is_file)
        .count()
        != inventory.files.len()
    {
        return Err(io::Error::other("Archive inventory mismatch"));
    }
    for ((name, _, size), expected) in inventory.files.iter().zip(digests) {
        let index = reader.lookup(name).map_err(io::Error::other)?;
        let mut decoded = HashSink {
            hash: Sha256::new(),
            bytes: 0,
            limit: *size,
            cancel: cancel.clone(),
        };
        reader
            .read_file(index, &mut decoded)
            .map_err(io::Error::other)?;
        if decoded.bytes != *size || decoded.hash.finalize() != expected {
            return Err(io::Error::other("Archive decoded content mismatch"));
        }
    }
    Ok(fs::metadata(output)?.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_versions_and_update_menu_identity_are_validated() {
        for (encoding, value, expected) in [
            ("hexBinary", "0041", 65),
            ("hexBinary", "000A", 10),
            ("unsignedInt", "65", 65),
        ] {
            let xml = format!(
                "<app><title_id>0005000e10102000</title_id><title_version type=\"{encoding}\">{value}</title_version></app>"
            );
            assert_eq!(
                title_header(&xml).unwrap(),
                (0x0005_000e_1010_2000, expected)
            );
        }
        for (encoding, value) in [
            ("hexBinary", "GG"),
            ("unsignedInt", "A"),
            ("string", "65"),
            ("hexBinary", "100000000"),
        ] {
            let xml = format!(
                "<app><title_id>0005000e10102000</title_id><title_version type=\"{encoding}\">{value}</title_version></app>"
            );
            assert!(title_header(&xml).is_err());
        }
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = Vec::new();
        let menu = |id, version| {
            format!(
                "<menu><title_id>{id}</title_id><title_version type=\"unsignedInt\">{version}</title_version></menu>"
            )
        };
        for (prefix, id, version, decimal) in [
            ("Base", "0005000010102000", "0000", 0),
            ("Update", "0005000e10102000", "0041", 65),
        ] {
            for (name, data) in [
                ("code/app.xml", format!("<app><title_id>{id}</title_id><title_version type=\"hexBinary\">{version}</title_version></app>").into_bytes()),
                ("code/cos.xml", b"<cos/>".to_vec()),
                ("meta/meta.xml", menu("0005000010102000", decimal).into_bytes()),
                ("content/data.bin", b"synthetic".to_vec()),
            ] {
                let relative = format!("{prefix}/{name}");
                let path = dir.path().join(&relative);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, &data).unwrap();
                inputs.push(InputFile { path: relative, bytes: data.len() as u64 });
            }
        }
        let inventory = inspect(dir.path(), &inputs).unwrap();
        assert!(
            inventory
                .files
                .iter()
                .any(|(name, _, _)| name == "0005000e10102000_v65/code/app.xml")
        );
        let path = dir.path().join("Update/meta/meta.xml");
        for (id, version, valid) in [
            ("0005000e10102000", 65, true),
            ("0005000010102001", 65, false),
            ("0005000c10102000", 65, false),
            ("0005000010102000", 64, false),
        ] {
            fs::write(&path, menu(id, version)).unwrap();
            assert_eq!(inspect(dir.path(), &inputs).is_ok(), valid);
        }
    }

    #[test]
    fn packs_verified_single_game_and_rejects_unsafe_paths_and_unrelated_titles() {
        for path in [
            "../code/app.xml",
            "/code/app.xml",
            "x\\code/app.xml",
            "C:/app.xml",
            "foo/CON.txt",
            "foo/a. ",
            "foo/é",
        ] {
            assert!(safe_path(path).is_err(), "{path}");
        }
        let dir = tempfile::tempdir().unwrap();
        let mut inputs = Vec::new();
        for (name, data) in [("code/app.xml", b"<app><title_id>0005000010102000</title_id><title_version>0</title_version></app>".to_vec()), ("code/cos.xml", b"<cos/>".to_vec()), ("meta/meta.xml", b"<menu><title_id>0005000010102000</title_id><title_version>0</title_version></menu>".to_vec()), ("content/data.bin", vec![42; 128 * 1024])] {
            let path = dir.path().join(name); fs::create_dir_all(path.parent().unwrap()).unwrap(); fs::write(path, &data).unwrap(); inputs.push(InputFile { path: name.into(), bytes: data.len() as u64 });
        }
        let inv = inspect(dir.path(), &inputs).unwrap();
        let output = dir.path().join("out.wua");
        let total = inv.bytes;
        let updates = std::sync::Mutex::new(Vec::new());
        let phases = std::sync::Mutex::new(Vec::new());
        let bytes = pack(
            inv,
            &output,
            1024 * 1024,
            0,
            Arc::new(AtomicBool::new(false)),
            |current, total| updates.lock().unwrap().push((current, total)),
            |phase| phases.lock().unwrap().push(phase),
        )
        .unwrap();
        assert!(bytes < 128 * 1024);
        let updates = updates.into_inner().unwrap();
        assert!(updates.len() > 1);
        assert_eq!(updates.last(), Some(&(total, total)));
        assert!(
            updates
                .windows(2)
                .all(|pair| pair[0].0 <= pair[1].0 && pair[0].1 == pair[1].1)
        );
        assert_eq!(
            phases.into_inner().unwrap(),
            ["finalizing_wua", "verifying"]
        );
        let inv = inspect(dir.path(), &inputs).unwrap();
        assert!(
            pack(
                inv,
                &dir.path().join("cancel.wua"),
                1024 * 1024,
                0,
                Arc::new(AtomicBool::new(true)),
                |_, _| {},
                |_| {}
            )
            .is_err()
        );
        let flag = Arc::new(AtomicBool::new(false));
        let progress_flag = flag.clone();
        let inv = inspect(dir.path(), &inputs).unwrap();
        assert!(
            pack(
                inv,
                &dir.path().join("mid-cancel.wua"),
                1024 * 1024,
                0,
                flag,
                |current, _| {
                    if current >= 64 * 1024 {
                        progress_flag.store(true, Ordering::Relaxed);
                    }
                },
                |_| {}
            )
            .is_err()
        );
        let inv = inspect(dir.path(), &inputs).unwrap();
        assert!(
            pack(
                inv,
                &dir.path().join("limited.wua"),
                1,
                0,
                Arc::new(AtomicBool::new(false)),
                |_, _| {},
                |_| {}
            )
            .is_err()
        );
        for invalid in ["<cos> ", "<a/><b/>", "      "] {
            fs::write(dir.path().join("code/cos.xml"), invalid).unwrap();
            inputs
                .iter_mut()
                .find(|file| file.path == "code/cos.xml")
                .unwrap()
                .bytes = invalid.len() as u64;
            assert!(
                inspect(dir.path(), &inputs).is_err(),
                "Reject incomplete or multiple-root XML"
            );
        }
        inputs
            .iter_mut()
            .find(|file| file.path == "code/cos.xml")
            .unwrap()
            .bytes = 6;
        fs::write(dir.path().join("code/cos.xml"), b"<cos/>").unwrap();
        inputs.push(InputFile {
            path: "readme.txt".into(),
            bytes: 0,
        });
        assert!(
            inspect(dir.path(), &inputs).is_err(),
            "Never silently discard root siblings"
        );
    }
}
