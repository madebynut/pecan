//! Local document conversion with retained embedded assets.

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anydoc::{
    Format,
    model::{Block, CellSlot, ImageSource, Inline},
};
use pecan_document::{Document, MAX_DOCUMENT_BYTES};
use tempfile::TempDir;
use thiserror::Error;

/// Total extracted image bytes per imported document, independent of compressed input size.
const MAX_IMAGE_BYTES: usize = 32 * 1024 * 1024;
/// Longest wait for one conversion. Legitimate 32 MiB inputs finish well within it.
const CONVERSION_DEADLINE: Duration = Duration::from_secs(120);
/// Conversions still running after their deadline; new ones wait until these finish.
const MAX_OVERDUE: usize = 2;
static OVERDUE: AtomicUsize = AtomicUsize::new(0);

/// The reading presentation available for the source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceKind {
    /// Markdown is displayed directly.
    Markdown,
    /// An office document is reflowed through the common reader.
    Office,
    /// PDF supports reflow and an independent original-page viewer.
    Pdf,
}

/// Temporary images and source metadata retained for the lifetime of an open tab.
#[derive(Debug)]
pub struct Resources {
    directory: PathBuf,
    _images: Option<TempDir>,
    kind: SourceKind,
    notice: Option<String>,
}

impl Resources {
    /// Base directory for reflowed image references.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Reading modes supported by this source.
    pub fn kind(&self) -> SourceKind {
        self.kind
    }
    /// A limitation requiring original-page viewing, when reflow is unavailable.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }
}

/// An indexed reading document with its local image resources.
#[derive(Debug)]
#[non_exhaustive]
pub struct LoadedDocument {
    /// Immutable content ready for the virtual Markdown reader.
    pub document: Document,
    /// Retain this while displaying the document; dropping it removes extracted images.
    pub resources: Resources,
}

/// A document could not be read or converted within the reader's resource budget.
#[derive(Debug, Error)]
#[error(transparent)]
pub struct ImportError(#[from] ErrorKind);

#[derive(Debug, Error)]
enum ErrorKind {
    #[error("读取文件失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("文件超过 32 MiB 阅读预算")]
    InputBudget,
    #[error("内嵌图片超过 32 MiB 阅读预算")]
    ImageBudget,
    #[error(
        "不支持此文档格式，请打开 Markdown、Word、PDF、OpenDocument、RTF、EPUB、演示文稿或表格文件"
    )]
    Unsupported,
    #[error("Markdown 必须使用 UTF-8 编码")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("文档转换失败：{0}")]
    Conversion(#[from] anydoc::ConvertError),
    #[error("重排内容超过阅读预算：{0}")]
    Index(#[from] pecan_document::ParseDocumentError),
    #[error("文档转换超过 2 分钟，已停止等待；文件可能过于复杂或已损坏")]
    Timeout,
    #[error("仍有超时的文档在后台转换，请稍后再试")]
    Busy,
    #[error("文档转换程序异常")]
    Crashed,
}

/// Whether a path names one of the formats supported by local reading.
pub fn supports(path: &Path) -> bool {
    is_markdown(path) || Format::from_path(path).is_some()
}

/// Whether a path names a Markdown source, rather than an imported document.
pub fn is_markdown(path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| {
            ["md", "markdown", "mdown"]
                .iter()
                .any(|name| ext.eq_ignore_ascii_case(name))
        })
}

/// Read, convert and index a local document; call this on a background executor.
///
/// Embedded images are extracted into a private temporary directory, never beside the source.
/// This function does not perform OCR, execute macros, or make network requests.
///
/// # Errors
/// Returns an error for unsupported, malformed, encrypted or oversized inputs and I/O failures.
pub fn load(path: &Path) -> Result<LoadedDocument, ImportError> {
    load_inner(path).map_err(ImportError::from)
}

fn load_inner(path: &Path) -> Result<LoadedDocument, ErrorKind> {
    let file = fs::File::open(path)?;
    if file.metadata()?.len() > MAX_DOCUMENT_BYTES as u64 {
        return Err(ErrorKind::InputBudget);
    }
    let mut bytes = Vec::new();
    file.take(MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_DOCUMENT_BYTES {
        return Err(ErrorKind::InputBudget);
    }
    let original = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    if is_markdown(path) {
        return Ok(LoadedDocument {
            document: Document::parse(String::from_utf8(bytes)?)?,
            resources: Resources {
                directory: original,
                _images: None,
                kind: SourceKind::Markdown,
                notice: None,
            },
        });
    }
    let format = Format::from_bytes(&bytes)
        .or_else(|| Format::from_path(path))
        .ok_or(ErrorKind::Unsupported)?;
    // Converters parse untrusted archives and PDFs without cancellation points, so a
    // pathological file runs on its own thread and the caller stops waiting at the deadline.
    run_with_deadline(CONVERSION_DEADLINE, move || {
        convert(bytes, format, original)
    })
}

fn convert(bytes: Vec<u8>, format: Format, original: PathBuf) -> Result<LoadedDocument, ErrorKind> {
    // Release each intermediate as soon as its successor exists; conversions run
    // two at a time, so source bytes and extracted assets would otherwise overlap.
    if format == Format::Pdf {
        let converted = anydoc::to_markdown_bytes(&bytes, format);
        drop(bytes);
        let (markdown, notice) = match converted {
            Ok(markdown) => (markdown, None),
            Err(anydoc::ConvertError::NeedsOcr { .. }) => {
                let notice =
                    "这份 PDF 包含扫描或图像页面，无法完整重排；请使用顶部的「原版查看」阅读。"
                        .to_owned();
                (format!("# PDF 原版阅读\n\n{notice}\n"), Some(notice))
            }
            Err(error) => return Err(error.into()),
        };
        return Ok(LoadedDocument {
            document: Document::parse(markdown)?,
            resources: Resources {
                directory: original,
                _images: None,
                kind: SourceKind::Pdf,
                notice,
            },
        });
    }
    let mut model = anydoc::to_document(&bytes, format)?;
    drop(bytes);
    // An office package is parsed once. Reuse its serializer after preserving embedded images.
    let images = tempfile::Builder::new().prefix("pecan-images-").tempdir()?;
    let mut image_bytes = 0usize;
    let mut targets = vec![None; model.assets.len()];
    // The Markdown serializer renders assets as alt text, so each payload is freed once written.
    for asset in std::mem::take(&mut model.assets) {
        let Some(extension) = image_extension(&asset.media_type) else {
            continue;
        };
        image_bytes = image_bytes.saturating_add(asset.bytes.len());
        if image_bytes > MAX_IMAGE_BYTES {
            return Err(ErrorKind::ImageBudget);
        }
        let target = images
            .path()
            .join(format!("asset-{}.{}", asset.id.0, extension));
        fs::write(&target, &asset.bytes)?;
        if let Some(slot) = targets.get_mut(asset.id.0) {
            *slot = Some(target.to_string_lossy().into_owned());
        }
    }
    adapt_images(&mut model.blocks, &targets, &original);
    for note in &mut model.notes {
        adapt_images(&mut note.blocks, &targets, &original);
    }
    let source = anydoc::model_to_markdown(&model);
    drop(model);
    let directory = images.path().to_path_buf();
    Ok(LoadedDocument {
        document: Document::parse(source)?,
        resources: Resources {
            directory,
            _images: Some(images),
            kind: SourceKind::Office,
            notice: None,
        },
    })
}

enum Slot<T> {
    Pending,
    Done(Result<T, ErrorKind>),
    Abandoned,
}

/// Run `job` on a dedicated thread and give up waiting after `deadline`.
///
/// An abandoned job keeps running until it returns, then drops its result. At most
/// [`MAX_OVERDUE`] abandoned jobs may exist, which bounds their CPU and memory use.
fn run_with_deadline<T: Send + 'static>(
    deadline: Duration,
    job: impl FnOnce() -> Result<T, ErrorKind> + Send + 'static,
) -> Result<T, ErrorKind> {
    if OVERDUE.load(Ordering::SeqCst) >= MAX_OVERDUE {
        return Err(ErrorKind::Busy);
    }
    let shared = Arc::new((Mutex::new(Slot::Pending), Condvar::new()));
    let worker = shared.clone();
    std::thread::Builder::new()
        .name("pecan-import".into())
        // Nested document structures recurse in the converters.
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))
                .unwrap_or(Err(ErrorKind::Crashed));
            let (slot, ready) = &*worker;
            let mut slot = slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if matches!(*slot, Slot::Abandoned) {
                OVERDUE.fetch_sub(1, Ordering::SeqCst);
            } else {
                *slot = Slot::Done(result);
                ready.notify_one();
            }
        })?;
    let (slot, ready) = &*shared;
    let slot = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut slot, _) = ready
        .wait_timeout_while(slot, deadline, |slot| matches!(slot, Slot::Pending))
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match std::mem::replace(&mut *slot, Slot::Abandoned) {
        Slot::Done(result) => result,
        _ => {
            OVERDUE.fetch_add(1, Ordering::SeqCst);
            Err(ErrorKind::Timeout)
        }
    }
}

fn image_extension(media_type: &str) -> Option<&'static str> {
    Some(match media_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "image/tiff" => "tiff",
        _ => return None,
    })
}

fn adapt_images(blocks: &mut [Block], targets: &[Option<String>], original: &Path) {
    for block in blocks {
        match block {
            Block::Heading { content, .. } | Block::Paragraph(content) => {
                adapt_inlines(content, targets, original)
            }
            Block::BlockQuote(blocks) => adapt_images(blocks, targets, original),
            Block::List(list) => {
                for item in &mut list.items {
                    adapt_images(&mut item.blocks, targets, original);
                }
            }
            Block::Table(table) => {
                for row in &mut table.grid {
                    for slot in row {
                        if let CellSlot::Origin(cell) = slot {
                            adapt_images(&mut cell.blocks, targets, original);
                        }
                    }
                }
            }
            Block::CodeBlock { .. } | Block::Rule | Block::Math(_) => {}
        }
    }
}

fn adapt_inlines(inlines: &mut [Inline], targets: &[Option<String>], original: &Path) {
    for inline in inlines {
        match inline {
            Inline::Image { source, .. } => match source {
                ImageSource::Asset(id) => {
                    if let Some(Some(target)) = targets.get(id.0) {
                        *source = ImageSource::External(target.clone());
                    }
                }
                ImageSource::External(target)
                    if !target.contains(':') && !Path::new(target).is_absolute() =>
                {
                    *target = original.join(&*target).to_string_lossy().into_owned();
                }
                _ => {}
            },
            Inline::Link { content, .. } => adapt_inlines(content, targets, original),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_conversions_time_out_and_release_their_overdue_slot() {
        assert_eq!(
            run_with_deadline(Duration::from_secs(5), || Ok(7)).unwrap(),
            7
        );
        let started = std::time::Instant::now();
        let result = run_with_deadline(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        });
        assert!(matches!(result, Err(ErrorKind::Timeout)));
        assert!(started.elapsed() < Duration::from_millis(250));
        assert!(OVERDUE.load(Ordering::SeqCst) >= 1);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while OVERDUE.load(Ordering::SeqCst) > 0 && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            OVERDUE.load(Ordering::SeqCst),
            0,
            "a finished job frees its slot"
        );
    }
}
