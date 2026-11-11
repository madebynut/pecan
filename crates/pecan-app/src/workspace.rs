use std::{
    fs,
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context as _, Result, bail};
use pecan_import::LoadedDocument;

/// Ignore build and dependency directories; never traverse symbolic links.
const IGNORED: &[&str] = &["target", "node_modules", ".git", ".idea", ".vscode"];

/// Share two conversion slots across reader windows without blocking the UI thread.
pub(crate) static CONVERSION_SLOTS: async_lock::Semaphore = async_lock::Semaphore::new(2);

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) path: PathBuf,
    pub(crate) name: String,
    pub(crate) directory: bool,
    pub(crate) depth: usize,
}

pub(crate) fn entries(directory: &Path, depth: usize) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for entry in
        fs::read_dir(directory).with_context(|| format!("读取目录 {}", directory.display()))?
    {
        let entry = entry?;
        let kind = entry.file_type()?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if kind.is_symlink() || name.starts_with('.') || IGNORED.contains(&name.as_str()) {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() || pecan_import::supports(&path) {
            entries.push(Entry {
                path,
                name,
                directory: kind.is_dir(),
                depth,
            });
        }
        if entries.len() == 10_000 {
            bail!("目录中的条目超过 10,000 个，请打开更小的工作区");
        }
    }
    entries.sort_unstable_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(entries)
}

pub(crate) fn load(path: &Path) -> Result<LoadedDocument> {
    let started = Instant::now();
    let document = pecan_import::load(path)?;
    tracing::info!(path = %path.display(), bytes = document.document.source().len(), chunks = document.document.blocks().len(), elapsed_ms = started.elapsed().as_millis(), "document indexed");
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("pecan-{name}-{}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn workspace_filters_build_files_and_orders_folders_before_markdown() {
        let root = Fixture::new("tree");
        fs::create_dir_all(root.0.join("folder")).unwrap();
        fs::create_dir_all(root.0.join("target")).unwrap();
        fs::write(root.0.join("z.MD"), "# Title").unwrap();
        fs::write(root.0.join("a.txt"), "not Markdown").unwrap();
        let visible = entries(&root.0, 0).unwrap();
        assert_eq!(
            visible
                .iter()
                .map(|entry| entry.name.as_str())
                .collect::<Vec<_>>(),
            vec!["folder", "z.MD"]
        );
    }

    #[test]
    fn loader_checks_utf8_and_size_before_creating_document_ui() {
        let root = Fixture::new("load");
        let path = root.0.join("document.md");
        fs::write(&path, "# 中文\n\n正文").unwrap();
        assert_eq!(load(&path).unwrap().document.headings()[0].title(), "中文");
        fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(load(&path).is_err());
        fs::File::create(&path)
            .unwrap()
            .set_len(pecan_document::MAX_DOCUMENT_BYTES as u64 + 1)
            .unwrap();
        assert!(load(&path).unwrap_err().to_string().contains("32 MiB"));
    }
}
