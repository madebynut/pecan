//! Portable document layout and output; native save dialogs stay in pecan-app.
mod code;
mod document;
mod images;
mod language_icons;
mod note_icons;
mod pdf;
mod table;
mod typography;
mod word;
use anyhow::{Context as _, Result};
use http_client::HttpClient;
use pecan_config::PreferencesData;
use pecan_document::Document;
use pecan_math::RenderQueue as MathQueue;
use pecan_mermaid::{Diagram, Region, RenderQueue};
use std::{io::Write as _, path::Path, sync::Arc};
/// The source and imported resources stay alive if the originating tab closes.
#[derive(Clone, Debug)]
pub struct Snapshot {
    /// Immutable source shared with the reader, without a text copy.
    pub document: Arc<Document>,
    /// Base path used to resolve relative document resources.
    pub directory: Arc<Path>,
    /// Source filename used for output titles and metadata.
    pub title: String,
    /// Reading configuration captured at the start of the export.
    pub preferences: Arc<PreferencesData>,
    /// Immutable prepared code cache shared with the reader.
    pub codes: pecan_format::CodeSnapshot,
    /// Diagram renderer cache and concurrency gate shared with reading.
    pub diagrams: RenderQueue,
    /// Math renderer cache and concurrency gate shared with reading.
    pub math: MathQueue,
    /// Keeps imported temporary resources alive through atomic commit.
    pub resources: Option<Arc<pecan_import::Resources>>,
}

/// File formats supported by complete document exports.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentFormat {
    /// Vector PDF with selectable text and embedded fonts.
    Pdf,
    /// Editable DOCX with native paragraphs and tables.
    Word,
    /// Self-contained HTML with embedded resources.
    Html,
    /// Source Markdown with portable image files.
    Markdown,
}
impl DocumentFormat {
    /// Filename extension associated with this output format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Word => "docx",
            Self::Html => "html",
            Self::Markdown => "md",
        }
    }
}
/// Vector or raster output for one independent Mermaid diagram.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagramFormat {
    /// Original vector SVG, preserving sharp geometry.
    Svg,
    /// Full-resolution PNG, rasterized from vector geometry.
    Png,
}
impl DiagramFormat {
    /// Filename extension associated with this output format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
        }
    }
}

/// A diagram and its shared CPU gate captured before opening a save dialog.
#[derive(Debug)]
pub struct DiagramInput {
    /// Rendered vector geometry; PNG output uses this full-resolution source.
    pub diagram: Diagram,
    /// Opaque RGB canvas color for raster export.
    pub background: [u8; 3],
    /// The originating renderer gate, preserving the shared CPU limit.
    pub slots: Arc<async_lock::Semaphore>,
}

fn diagram_bytes_inner(
    diagram: &Diagram,
    format: DiagramFormat,
    background: [u8; 3],
) -> Result<Vec<u8>> {
    match format {
        DiagramFormat::Svg => Ok(diagram.svg().as_bytes().to_vec()),
        DiagramFormat::Png => {
            // Re-rasterize the vector rather than saving a reading thumbnail.
            let scale = 2_f32.min(diagram.max_scale());
            Ok(diagram.png_region(Region::whole(diagram, scale), Some(background))?)
        }
    }
}

fn atomic_write_inner(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("导出路径没有父目录")?;
    let mut file = tempfile::NamedTempFile::new_in(parent).context("创建导出临时文件")?;
    file.write_all(bytes).context("写入导出文件")?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|error| error.error)
        .context("保存导出文件")?;
    Ok(())
}

async fn write_document_inner(
    snapshot: Snapshot,
    format: DocumentFormat,
    path: &Path,
    client: Arc<dyn HttpClient>,
) -> Result<()> {
    // Keep the temporary Office image directory until every byte is committed.
    let _resources = snapshot.resources.as_ref();
    match format {
        DocumentFormat::Markdown => document::markdown(&snapshot, path, client).await,
        DocumentFormat::Word => atomic_write_inner(path, &word::docx(&snapshot, client).await?),
        DocumentFormat::Html => {
            atomic_write_inner(path, document::html(&snapshot, client).await?.as_bytes())
        }
        DocumentFormat::Pdf => pdf::write(&snapshot, path, client).await,
    }
}

/// Encode a diagram as vector SVG or a full-resolution 2x PNG.
/// # Errors
/// Returns the renderer's resource-limit or rasterization error.
pub fn diagram_bytes(
    diagram: &Diagram,
    format: DiagramFormat,
    background: [u8; 3],
) -> Result<Vec<u8>, ExportError> {
    diagram_bytes_inner(diagram, format, background).map_err(ExportError::new)
}
/// Commit bytes atomically, preserving the target on write failure.
/// # Errors
/// Returns an I/O error if writing or atomic replacement fails.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    atomic_write_inner(path, bytes).map_err(ExportError::new)
}

/// Write a complete portable export; failed generation leaves the target untouched.
/// # Errors
/// Returns an error for unavailable resources, resource limits, invalid output or I/O.
pub async fn write_document(
    snapshot: Snapshot,
    format: DocumentFormat,
    path: &Path,
    client: Arc<dyn HttpClient>,
) -> Result<(), ExportError> {
    write_document_inner(snapshot, format, path, client)
        .await
        .map_err(ExportError::new)
}
/// A portable export failure retaining the original cause and context.
#[derive(Debug, thiserror::Error)]
#[error("export failed: {source:#}")]
pub struct ExportError {
    #[source]
    source: anyhow::Error,
}
impl ExportError {
    fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}

#[cfg(test)]
const FORMAT_FIXTURE: &str = "```json\n{\"outer\":{\"value\":1}}\n```\n\n:::info\n```yaml\nreader:\n  port: 8080\n```\n:::\n\n> ```TSX\n> const V=()=> <div title=\"Pecan\"/>;\n> ```\n\n```unknown\nkeep   original\n```\n";

#[cfg(test)]
fn expected_code(width: usize) -> Vec<String> {
    use pecan_format::Format;
    [
        (Some(Format::Json), "{\"outer\":{\"value\":1}}"),
        (Some(Format::Yaml), "reader:\n  port: 8080"),
        (Some(Format::Tsx), "const V=()=> <div title=\"Pecan\"/>;"),
        (None, "keep   original"),
    ]
    .into_iter()
    .map(|(format, source)| {
        format.map_or_else(
            || source.to_owned(),
            |format| format.format(source, width).unwrap().text().to_owned(),
        )
    })
    .collect()
}

#[cfg(test)]
mod network {
    pub(crate) fn client() -> std::sync::Arc<dyn http_client::HttpClient> {
        std::sync::Arc::new(pecan_network::Client::new().unwrap())
    }
}
