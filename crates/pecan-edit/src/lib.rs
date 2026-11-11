//! Conflict-aware Markdown persistence without a UI or platform-specific dependencies.
use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use pecan_document::MAX_DOCUMENT_BYTES;

/// A source save failed without deliberately replacing a newer external edit.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SaveError {
    /// The file has changed since its last load or successful save.
    #[error("文件已被其他程序修改，请先另存为或重新加载，当前草稿已保留")]
    Conflict,
    /// The source exceeds the reader's document budget.
    #[error("源码超过 32 MiB，请缩小文档后保存")]
    TooLarge,
    /// An underlying file operation failed.
    #[error("保存文件失败：{0}")]
    Io(#[from] std::io::Error),
}

/// Atomically replace an existing Markdown file after checking its previous contents.
///
/// Run on a background executor. Follows symlinks, preserves permissions, and compares
/// bytes immediately before replacement. This is optimistic conflict detection, not an
/// interprocess file lock; another writer can still race the final rename.
///
/// # Errors
/// Returns an error for external changes, oversized source, or filesystem failures.
pub fn save(path: &Path, previous: &str, source: &str) -> Result<PathBuf, SaveError> {
    if source.len() > MAX_DOCUMENT_BYTES {
        return Err(SaveError::TooLarge);
    }
    let target = fs::canonicalize(path)?;
    let metadata = fs::metadata(&target)?;
    if metadata.permissions().readonly() {
        return Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into());
    }
    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "file has no parent directory",
        )
    })?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(source.as_bytes())?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.as_file().sync_all()?;
    let mut current = blake3::Hasher::new();
    let mut input = File::open(&target)?.take(MAX_DOCUMENT_BYTES as u64 + 1);
    let mut buffer = [0; 64 * 1024];
    let mut length = 0;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        length += count;
        current.update(&buffer[..count]);
    }
    if length != previous.len() || current.finalize() != blake3::hash(previous.as_bytes()) {
        return Err(SaveError::Conflict);
    }
    temporary.persist(&target).map_err(|error| error.error)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_bytes_and_line_endings_survive_save_and_conflicts_keep_both_versions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("中文.md");
        let original = "\u{feff}# 中文\r\n\r\n`raw`\r\n";
        let edited = "\u{feff}# 中文\r\n\r\n**edited**\r\n";
        fs::write(&path, original).unwrap();
        save(&path, original, edited).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), edited);
        fs::write(&path, "external").unwrap();
        assert!(matches!(
            save(&path, edited, "draft"),
            Err(SaveError::Conflict)
        ));
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn saving_through_a_link_preserves_the_link_and_target_permissions() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.md");
        let link = directory.path().join("link.md");
        fs::write(&target, "before").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
        symlink(&target, &link).unwrap();
        save(&link, "before", "after").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "after");
        assert_eq!(
            fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
