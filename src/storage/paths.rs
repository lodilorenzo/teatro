use std::{
    io,
    path::{Component, Path, PathBuf},
};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PathSafetyError {
    #[error("path cannot be empty")]
    Empty,

    #[error("absolute paths are not allowed")]
    Absolute,

    #[error("parent-directory traversal is not allowed")]
    ParentTraversal,

    #[error("path escapes configured root")]
    EscapesRoot,

    #[error("symlinks are not allowed in managed paths")]
    Symlink,

    #[error("failed to access path: {0}")]
    Io(#[from] io::Error),
}

pub fn resolve_existing(root: &Path, relative_path: &str) -> Result<PathBuf, PathSafetyError> {
    let root = root.canonicalize()?;
    let relative = clean_relative_path(relative_path)?;
    reject_symlink_components(&root, &relative)?;
    let candidate = root.join(relative);
    let candidate = candidate.canonicalize()?;

    if !candidate.starts_with(&root) {
        return Err(PathSafetyError::EscapesRoot);
    }

    Ok(candidate)
}

pub fn resolve_new(root: &Path, relative_path: &str) -> Result<PathBuf, PathSafetyError> {
    let root = root.canonicalize()?;
    let relative = clean_relative_path(relative_path)?;
    reject_symlink_components(&root, &relative)?;
    let candidate = root.join(relative);
    let parent = candidate.parent().ok_or(PathSafetyError::Empty)?;
    let parent = parent.canonicalize()?;

    if !parent.starts_with(&root) {
        return Err(PathSafetyError::EscapesRoot);
    }

    let file_name = candidate.file_name().ok_or(PathSafetyError::Empty)?;
    let candidate = parent.join(file_name);

    if !candidate.starts_with(&root) {
        return Err(PathSafetyError::EscapesRoot);
    }

    Ok(candidate)
}

fn reject_symlink_components(root: &Path, relative: &Path) -> Result<(), PathSafetyError> {
    let mut candidate = root.to_path_buf();
    for component in relative.components() {
        candidate.push(component.as_os_str());
        match std::fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(PathSafetyError::Symlink);
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }

    Ok(())
}

pub fn safe_content_disposition_filename(file_name: &str) -> String {
    let sanitized: String = file_name
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | ' ' | '.' | '_' | '-' | '(' | ')' | '[' | ']' => {
                character
            }
            _ => '_',
        })
        .collect();

    let sanitized = sanitized.trim_matches([' ', '.']);
    if sanitized.is_empty() {
        "download.bin".to_string()
    } else {
        sanitized.to_string()
    }
}

pub fn clean_relative_path(relative_path: &str) -> Result<PathBuf, PathSafetyError> {
    if relative_path.is_empty() {
        return Err(PathSafetyError::Empty);
    }

    let path = Path::new(relative_path);
    if path.is_absolute() {
        return Err(PathSafetyError::Absolute);
    }

    let mut cleaned = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(segment) => cleaned.push(segment),
            Component::CurDir => {}
            Component::ParentDir => return Err(PathSafetyError::ParentTraversal),
            Component::RootDir | Component::Prefix(_) => return Err(PathSafetyError::Absolute),
        }
    }

    if cleaned.as_os_str().is_empty() {
        return Err(PathSafetyError::Empty);
    }

    Ok(cleaned)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{PathSafetyError, resolve_existing, resolve_new};

    #[test]
    fn resolves_existing_paths_inside_root() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path();
        fs::create_dir_all(root.join("nes")).unwrap();
        fs::write(root.join("nes/game.nes"), b"rom").unwrap();

        let resolved = resolve_existing(root, "nes/game.nes").unwrap();

        assert_eq!(resolved, root.join("nes/game.nes").canonicalize().unwrap());
    }

    #[test]
    fn rejects_parent_traversal() {
        let temp_dir = TempDir::new().unwrap();

        let error = resolve_existing(temp_dir.path(), "../secret").unwrap_err();

        assert!(matches!(error, PathSafetyError::ParentTraversal));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_and_dangling_final_symlinks_for_writes() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("inside"), b"inside").unwrap();
        std::os::unix::fs::symlink(root.join("inside"), root.join("existing")).unwrap();
        std::os::unix::fs::symlink(root.join("missing"), root.join("dangling")).unwrap();

        assert!(matches!(
            resolve_new(&root, "existing").unwrap_err(),
            PathSafetyError::Symlink
        ));
        assert!(matches!(
            resolve_new(&root, "dangling").unwrap_err(),
            PathSafetyError::Symlink
        ));
    }

    #[test]
    fn rejects_symlink_escape() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("root");
        let outside = temp_dir.path().join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.txt"), b"secret").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("secret.txt"))
                .unwrap();
            let error = resolve_existing(&root, "secret.txt").unwrap_err();
            assert!(matches!(error, PathSafetyError::Symlink));
        }
    }
}
