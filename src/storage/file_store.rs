//! Auditable boundary for every managed library and asset mutation.
//!
//! Callers acquire a cross-process root lock before multi-step operations. New writes reject
//! existing targets and symlinks; generated/replacement content uses a temporary sibling plus
//! rename; destructive workflows move files to same-filesystem trash before database changes.

use std::{
    fs::{File, OpenOptions as StdOpenOptions},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use fs2::FileExt;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::paths::{self, PathSafetyError};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Error)]
pub enum FileStoreError {
    #[error(transparent)]
    Path(#[from] PathSafetyError),

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("managed target already exists")]
    AlreadyExists,

    #[error("managed target is not a regular file")]
    NotRegularFile,

    #[error("managed target is not a directory")]
    NotDirectory,
}

#[derive(Debug, Clone)]
pub struct FileStore {
    #[cfg(test)]
    fail_after_mutations: std::sync::Arc<std::sync::atomic::AtomicIsize>,
}

/// Holds the advisory cross-process mutation lock for one managed root.
#[derive(Debug)]
pub struct RootMutationGuard {
    _file: File,
    root: PathBuf,
}

impl RootMutationGuard {
    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl Default for FileStore {
    fn default() -> Self {
        Self::new()
    }
}

impl FileStore {
    pub fn new() -> Self {
        Self {
            #[cfg(test)]
            fail_after_mutations: std::sync::Arc::new(std::sync::atomic::AtomicIsize::new(-1)),
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_after_mutations(&self, successful_mutations: isize) {
        self.fail_after_mutations
            .store(successful_mutations.max(0), Ordering::SeqCst);
    }

    #[cfg(test)]
    fn inject_mutation_failure(&self) -> Result<(), FileStoreError> {
        loop {
            let remaining = self.fail_after_mutations.load(Ordering::SeqCst);
            if remaining < 0 {
                return Ok(());
            }
            let next = if remaining == 0 { -1 } else { remaining - 1 };
            if self
                .fail_after_mutations
                .compare_exchange(remaining, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return if remaining == 0 {
                    Err(io::Error::other("injected managed filesystem mutation failure").into())
                } else {
                    Ok(())
                };
            }
        }
    }

    #[cfg(not(test))]
    fn inject_mutation_failure(&self) -> Result<(), FileStoreError> {
        Ok(())
    }

    pub async fn lock_root(&self, root: &Path) -> Result<RootMutationGuard, FileStoreError> {
        let root = root.canonicalize()?;
        let lock_path = root.join(".teatro.lock");
        match std::fs::symlink_metadata(&lock_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathSafetyError::Symlink.into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }

        let file = tokio::task::spawn_blocking(move || {
            let file = StdOpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(lock_path)?;
            file.lock_exclusive()?;
            Ok::<_, io::Error>(file)
        })
        .await
        .map_err(|error| io::Error::other(format!("root lock task failed: {error}")))??;

        Ok(RootMutationGuard { _file: file, root })
    }

    pub fn resolve_existing(
        &self,
        root: &Path,
        relative_path: &str,
    ) -> Result<PathBuf, FileStoreError> {
        Ok(paths::resolve_existing(root, relative_path)?)
    }

    pub fn resolve_new(&self, root: &Path, relative_path: &str) -> Result<PathBuf, FileStoreError> {
        Ok(paths::resolve_new(root, relative_path)?)
    }

    /// Validate and open an existing managed regular file.
    pub async fn open_existing(
        &self,
        root: &Path,
        relative_path: &str,
    ) -> Result<tokio::fs::File, FileStoreError> {
        let path = self.resolve_existing(root, relative_path)?;
        let file = tokio::fs::File::open(path).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            return Err(FileStoreError::NotRegularFile);
        }
        Ok(file)
    }

    /// Read a bounded managed regular file through the same validation boundary.
    pub async fn read_existing(
        &self,
        root: &Path,
        relative_path: &str,
        max_bytes: u64,
    ) -> Result<Vec<u8>, FileStoreError> {
        let file = self.open_existing(root, relative_path).await?;
        let metadata = file.metadata().await?;
        if metadata.len() > max_bytes {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "managed file is too large").into(),
            );
        }
        let capacity = usize::try_from(metadata.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "managed file size is unsupported",
            )
        })?;
        let mut bytes = Vec::with_capacity(capacity);
        file.take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .await?;
        if bytes.len() as u64 > max_bytes {
            return Err(
                io::Error::new(io::ErrorKind::InvalidData, "managed file is too large").into(),
            );
        }
        Ok(bytes)
    }

    /// Read a bounded managed file supplied as an absolute path under `root`.
    pub async fn read_managed_path(
        &self,
        root: &Path,
        path: &Path,
        max_bytes: u64,
    ) -> Result<Vec<u8>, FileStoreError> {
        let root = root.canonicalize()?;
        let relative = path
            .strip_prefix(&root)
            .map_err(|_| PathSafetyError::EscapesRoot)?;
        let relative = relative.to_string_lossy().replace('\\', "/");
        self.read_existing(&root, &relative, max_bytes).await
    }

    pub async fn create_dir_all(
        &self,
        root: &Path,
        relative_dir: &Path,
    ) -> Result<PathBuf, FileStoreError> {
        let root = root.canonicalize()?;
        let relative = paths::clean_relative_path(&relative_dir.to_string_lossy())?;
        let mut current = root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            match tokio::fs::symlink_metadata(&current).await {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(PathSafetyError::Symlink.into());
                }
                Ok(metadata) if !metadata.is_dir() => return Err(FileStoreError::NotRegularFile),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    match tokio::fs::create_dir(&current).await {
                        Ok(()) => {}
                        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                            let metadata = tokio::fs::symlink_metadata(&current).await?;
                            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                                return Err(PathSafetyError::Symlink.into());
                            }
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }

        Ok(current)
    }

    pub async fn move_managed(
        &self,
        root: &Path,
        source_relative_path: &str,
        target_relative_path: &str,
    ) -> Result<PathBuf, FileStoreError> {
        let source = self.resolve_existing(root, source_relative_path)?;
        let metadata = tokio::fs::symlink_metadata(&source).await?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(FileStoreError::NotRegularFile);
        }
        let target_relative = paths::clean_relative_path(target_relative_path)?;
        let parent = target_relative.parent().ok_or(PathSafetyError::Empty)?;
        self.create_dir_all(root, parent).await?;
        self.move_new(root, target_relative_path, &source).await
    }

    pub async fn move_managed_directory(
        &self,
        root: &Path,
        source_relative_path: &str,
        target_relative_path: &str,
    ) -> Result<PathBuf, FileStoreError> {
        let source = self.resolve_existing(root, source_relative_path)?;
        let metadata = tokio::fs::symlink_metadata(&source).await?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FileStoreError::NotDirectory);
        }
        let target_relative = paths::clean_relative_path(target_relative_path)?;
        let parent = target_relative.parent().ok_or(PathSafetyError::Empty)?;
        self.create_dir_all(root, parent).await?;
        self.move_new(root, target_relative_path, &source).await
    }

    pub async fn exists(&self, root: &Path, relative_path: &str) -> Result<bool, FileStoreError> {
        match self.resolve_existing(root, relative_path) {
            Ok(_) => Ok(true),
            Err(FileStoreError::Path(PathSafetyError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    pub async fn move_new(
        &self,
        root: &Path,
        relative_path: &str,
        source: &Path,
    ) -> Result<PathBuf, FileStoreError> {
        let target = self.resolve_new(root, relative_path)?;
        reject_existing_target(&target).await?;
        self.inject_mutation_failure()?;
        match tokio::fs::rename(source, &target).await {
            Ok(()) => Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(FileStoreError::AlreadyExists)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub async fn write_new_atomic(
        &self,
        root: &Path,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<PathBuf, FileStoreError> {
        let target = self.resolve_new(root, relative_path)?;
        reject_existing_target(&target).await?;
        self.write_temporary_then_rename(&target, contents, false)
            .await?;
        Ok(target)
    }

    pub async fn replace_atomic(
        &self,
        root: &Path,
        relative_path: &str,
        contents: &[u8],
    ) -> Result<PathBuf, FileStoreError> {
        let target = self.resolve_new(root, relative_path)?;
        if let Ok(metadata) = tokio::fs::symlink_metadata(&target).await
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err(FileStoreError::NotRegularFile);
        }
        self.write_temporary_then_rename(&target, contents, true)
            .await?;
        Ok(target)
    }

    pub async fn remove(&self, root: &Path, relative_path: &str) -> Result<bool, FileStoreError> {
        let target = match self.resolve_existing(root, relative_path) {
            Ok(target) => target,
            Err(FileStoreError::Path(PathSafetyError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let metadata = tokio::fs::symlink_metadata(&target).await?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(FileStoreError::NotRegularFile);
        }
        tokio::fs::remove_file(target).await?;
        Ok(true)
    }

    pub async fn remove_directory_all(
        &self,
        root: &Path,
        relative_path: &str,
    ) -> Result<bool, FileStoreError> {
        let target = match self.resolve_existing(root, relative_path) {
            Ok(target) => target,
            Err(FileStoreError::Path(PathSafetyError::Io(error)))
                if error.kind() == io::ErrorKind::NotFound =>
            {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        let metadata = tokio::fs::symlink_metadata(&target).await?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(FileStoreError::NotDirectory);
        }
        tokio::fs::remove_dir_all(target).await?;
        Ok(true)
    }

    pub async fn create_staging_file(
        &self,
        root: &Path,
        relative_path: &str,
    ) -> Result<(PathBuf, tokio::fs::File), FileStoreError> {
        let relative = paths::clean_relative_path(relative_path)?;
        if relative.parent() != Some(Path::new(".uploads"))
            || relative
                .extension()
                .and_then(|extension| extension.to_str())
                != Some("part")
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging files must be direct .uploads/*.part children",
            )
            .into());
        }

        self.create_dir_all(root, Path::new(".uploads")).await?;
        let path = self.resolve_new(root, relative_path)?;
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    FileStoreError::AlreadyExists
                } else {
                    error.into()
                }
            })?;
        Ok((path, file))
    }

    pub async fn cleanup_stale_uploads(
        &self,
        root: &Path,
        minimum_age_seconds: u64,
    ) -> Result<usize, FileStoreError> {
        let root = root.canonicalize()?;
        let _lock = self.lock_root(&root).await?;
        let staging_dir = root.join(".uploads");
        match tokio::fs::symlink_metadata(&staging_dir).await {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathSafetyError::Symlink.into());
            }
            Ok(metadata) if !metadata.is_dir() => return Err(FileStoreError::NotRegularFile),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        }
        let mut entries = tokio::fs::read_dir(&staging_dir).await?;
        let cutoff = std::time::SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(minimum_age_seconds))
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let mut removed = 0;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("part") {
                continue;
            }
            let metadata = tokio::fs::symlink_metadata(&path).await?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            let modified = metadata.modified()?;
            if modified <= cutoff {
                tokio::fs::remove_file(path).await?;
                removed += 1;
            }
        }
        if removed > 0 {
            tracing::info!(removed, "removed stale upload staging files");
        }
        Ok(removed)
    }

    pub async fn cleanup_path(&self, path: &Path) {
        if let Err(error) = tokio::fs::remove_file(path).await
            && error.kind() != io::ErrorKind::NotFound
        {
            tracing::warn!(?error, ?path, "failed to clean managed temporary file");
        }
    }

    async fn write_temporary_then_rename(
        &self,
        target: &Path,
        contents: &[u8],
        replace: bool,
    ) -> Result<(), FileStoreError> {
        let temp = temporary_sibling(target)?;
        let result = async {
            self.inject_mutation_failure()?;
            let mut file = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .await?;
            file.write_all(contents).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);

            if !replace {
                reject_existing_target(target).await?;
            }
            self.inject_mutation_failure()?;
            tokio::fs::rename(&temp, target).await?;
            Ok::<_, FileStoreError>(())
        }
        .await;

        if result.is_err() {
            self.cleanup_path(&temp).await;
        }
        result
    }
}

async fn reject_existing_target(target: &Path) -> Result<(), FileStoreError> {
    match tokio::fs::symlink_metadata(target).await {
        Ok(_) => Err(FileStoreError::AlreadyExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn temporary_sibling(target: &Path) -> Result<PathBuf, FileStoreError> {
    let parent = target.parent().ok_or(PathSafetyError::Empty)?;
    let file_name = target
        .file_name()
        .ok_or(PathSafetyError::Empty)?
        .to_string_lossy();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    )))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{FileStore, FileStoreError};
    use crate::storage::paths::PathSafetyError;

    #[tokio::test]
    async fn new_writes_never_replace_an_existing_target() {
        let temp = TempDir::new().unwrap();
        let store = FileStore::new();
        let _lock = store.lock_root(temp.path()).await.unwrap();
        assert!(temp.path().join(".teatro.lock").is_file());
        assert!(!temp.path().join(".beacon.lock").exists());
        store
            .write_new_atomic(temp.path(), "game.rom", b"first")
            .await
            .unwrap();
        let error = store
            .write_new_atomic(temp.path(), "game.rom", b"second")
            .await
            .unwrap_err();
        assert!(matches!(error, FileStoreError::AlreadyExists));
        assert_eq!(
            std::fs::read(temp.path().join("game.rom")).unwrap(),
            b"first"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stale_upload_cleanup_rejects_a_symlinked_staging_directory() {
        let temp = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let stale = outside.path().join("outside.part");
        std::fs::write(&stale, b"do not remove").unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join(".uploads")).unwrap();

        let error = FileStore::new()
            .cleanup_stale_uploads(temp.path(), 0)
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            FileStoreError::Path(PathSafetyError::Symlink)
        ));
        assert_eq!(std::fs::read(stale).unwrap(), b"do not remove");
    }

    #[tokio::test]
    async fn failed_atomic_write_removes_its_temporary_file() {
        let temp = TempDir::new().unwrap();
        let store = FileStore::new();
        let _lock = store.lock_root(temp.path()).await.unwrap();
        store.fail_after_mutations(1);

        let error = store
            .write_new_atomic(temp.path(), "game.rom", b"partial")
            .await
            .unwrap_err();

        assert!(error.to_string().contains("injected managed filesystem"));
        assert!(!temp.path().join("game.rom").exists());
        assert!(std::fs::read_dir(temp.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[tokio::test]
    async fn replacements_are_atomic_and_leave_no_temporary_file() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("cover.jpg"), b"old").unwrap();
        let store = FileStore::new();
        let _lock = store.lock_root(temp.path()).await.unwrap();
        store
            .replace_atomic(temp.path(), "cover.jpg", b"new")
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("cover.jpg")).unwrap(),
            b"new"
        );
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 2); // cover and lock
    }
}
