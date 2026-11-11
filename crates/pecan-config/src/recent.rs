//! Bounded recent-path history, including lossless native filenames.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
/// One hundred entries keep startup, search and menu updates bounded.
const MAX_ENTRIES: usize = 100;
const MAX_FILE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
/// Whether a history item opens one document or a workspace folder.
pub enum Kind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A lossless native path and the operation needed to reopen it.
pub struct Entry {
    #[serde(with = "native_path")]
    pub path: PathBuf,
    pub kind: Kind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
/// Bounded most-recent-first paths; source contents are never stored.
pub struct History {
    entries: Vec<Entry>,
}
impl History {
    /// Paths in most-recent-first order; reading this never accesses the filesystem.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    /// Clear history without modifying any source documents.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Move a path to the front and normalize count and encoded-byte budgets.
    pub fn record(&mut self, entry: Entry) -> bool {
        if self.entries.first() == Some(&entry) {
            return false;
        }
        self.entries.retain(|old| old.path != entry.path);
        self.entries.insert(0, entry);
        self.normalize();
        true
    }
    fn normalize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut budget = MAX_FILE_BYTES - 1024;
        self.entries.retain(|entry| {
            // JSON escaping or native byte arrays need at most six bytes per encoded byte.
            let cost = entry
                .path
                .as_os_str()
                .len()
                .saturating_mul(6)
                .saturating_add(128);
            if !entry.path.is_absolute() || cost > budget || !seen.insert(entry.path.clone()) {
                return false;
            }
            budget -= cost;
            true
        });
        self.entries.truncate(MAX_ENTRIES);
    }
}

/// Read history within its byte budget and prune duplicate or relative paths.
/// # Errors
/// Returns an I/O or invalid JSON error without scanning document contents.
pub fn read(path: &Path) -> std::io::Result<History> {
    use std::io::Read as _;
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "recent history exceeds 256 KiB",
        ));
    }
    let mut data: History = serde_json::from_slice(&bytes)?;
    data.normalize();
    Ok(data)
}
fn save_inner(path: &Path, data: &History) -> anyhow::Result<()> {
    use std::io::Write as _;
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("recent history path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    // Compact JSON keeps the same byte budget when paths use a native byte representation.
    let bytes = serde_json::to_vec(data)?;
    anyhow::ensure!(
        bytes.len() <= MAX_FILE_BYTES,
        "recent history exceeds its byte budget"
    );
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

/// Display a compact sanitized path without changing its native identity.
pub fn display_path(path: &Path) -> String {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    let text = home
        .and_then(|home| {
            path.strip_prefix(home)
                .ok()
                .map(|rest| format!("~/{}", rest.display()))
        })
        .unwrap_or_else(|| path.display().to_string());
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

// Preserve non-UTF-8 paths without unsafe conversions or lossy filenames.
mod native_path {
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};
    #[derive(Serialize, Deserialize)]
    #[serde(untagged)]
    enum Encoded {
        Text(String),
        Unix { unix: Vec<u8> },
        Windows { windows: Vec<u16> },
    }
    pub(super) fn serialize<S: serde::Serializer>(
        path: &Path,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if let Some(text) = path.to_str() {
            return Encoded::Text(text.into()).serialize(serializer);
        }
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            Encoded::Unix {
                unix: path.as_os_str().as_bytes().to_vec(),
            }
            .serialize(serializer)
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt as _;
            Encoded::Windows {
                windows: path.as_os_str().encode_wide().collect(),
            }
            .serialize(serializer)
        }
    }
    pub(super) fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<PathBuf, D::Error> {
        match Encoded::deserialize(deserializer)? {
            Encoded::Text(text) => Ok(text.into()),
            #[cfg(unix)]
            Encoded::Unix { unix } => {
                use std::os::unix::ffi::OsStringExt as _;
                Ok(std::ffi::OsString::from_vec(unix).into())
            }
            #[cfg(windows)]
            Encoded::Windows { windows } => {
                use std::os::windows::ffi::OsStringExt as _;
                Ok(std::ffi::OsString::from_wide(&windows).into())
            }
            _ => Err(serde::de::Error::custom(
                "native path belongs to another platform",
            )),
        }
    }
}

/// Persist recent paths atomically under their existing byte budget.
/// # Errors
/// Returns an error if storage or serialization fails.
pub fn save(path: &Path, data: &History) -> Result<(), crate::ConfigError> {
    save_inner(path, data).map_err(crate::ConfigError::new)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn entry(root: &Path, name: &str, kind: Kind) -> Entry {
        Entry {
            path: root.join(name),
            kind,
        }
    }

    #[test]
    fn mru_deduplicates_preserves_kinds_and_bounds_storage() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let mut data = History::default();
        let folder = entry(root, "folder", Kind::Folder);
        data.record(folder.clone());
        let file = entry(root, "中文 notes.md", Kind::File);
        data.record(file.clone());
        data.record(folder.clone());
        assert_eq!(data.entries, [folder.clone(), file]);
        assert!(
            !data.record(folder),
            "opening the latest path does not write again"
        );
        for index in 0..150 {
            data.record(entry(root, &format!("{index}.md"), Kind::File));
        }
        assert_eq!(data.entries.len(), 100);
        assert!(data.entries[0].path.ends_with("149.md"));
        assert!(data.entries[99].path.ends_with("50.md"));
        for index in 0..100 {
            data.record(entry(
                root,
                &format!("{index}-{}", "\n".repeat(4000)),
                Kind::File,
            ));
        }
        let path = root.join("recent.json");
        save(&path, &data).unwrap();
        assert!(std::fs::metadata(&path).unwrap().len() <= MAX_FILE_BYTES as u64);
        assert_eq!(
            read(&path).unwrap(),
            data,
            "escaped paths stay within the read budget"
        );
    }

    #[test]
    fn loading_rejects_corruption_and_prunes_duplicate_or_relative_paths() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("recent.json");
        std::fs::write(&path, "broken JSON").unwrap();
        assert!(read(&path).is_err());
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        assert_eq!(
            read(&path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        let file = entry(directory.path(), "missing.md", Kind::File);
        let data = History {
            entries: vec![
                file.clone(),
                Entry {
                    path: "relative.md".into(),
                    kind: Kind::File,
                },
                file.clone(),
            ],
        };
        std::fs::write(&path, serde_json::to_vec(&data).unwrap()).unwrap();
        assert_eq!(
            read(&path).unwrap().entries,
            [file],
            "do not scan or discard offline paths at startup"
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_native_paths_round_trip_without_loss() {
        use std::os::unix::ffi::OsStringExt as _;
        let directory = tempfile::tempdir().unwrap();
        let file = Entry {
            path: directory
                .path()
                .join(std::ffi::OsString::from_vec(b"file-\xff.md".to_vec())),
            kind: Kind::File,
        };
        let mut data = History::default();
        data.record(file);
        let path = directory.path().join("recent.json");
        save(&path, &data).unwrap();
        assert_eq!(read(&path).unwrap(), data);
    }
}
