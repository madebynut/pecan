use std::{borrow::Cow, ops::Range, sync::Arc};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use thiserror::Error;

/// Maximum accepted file size; bounds indexing, searching and resident source memory.
pub const MAX_DOCUMENT_BYTES: usize = 32 * 1024 * 1024;
/// Chunks end at Markdown block boundaries so lists and tables stay intact.
const CHUNK_TARGET_BYTES: usize = 8 * 1024;
const MAX_RICH_BLOCK_BYTES: usize = 64 * 1024;
const SOURCE_PAGE_BYTES: usize = 16 * 1024;
/// Outline labels are repeatedly shaped on the UI thread; source remains unmodified.
const MAX_HEADING_LABEL_BYTES: usize = 512;

/// A document read or indexing failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ParseDocumentError {
    /// The document exceeds the supported in-memory reading budget.
    #[error("document is {size} bytes; limit is {MAX_DOCUMENT_BYTES} bytes")]
    TooLarge {
        /// Actual UTF-8 length.
        size: usize,
    },
}

/// A native Markdown chunk or a separately rendered Mermaid diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Block {
    /// Contiguous source ending at complete Markdown block boundaries.
    Markdown {
        /// Source byte range.
        source: Range<usize>,
    },
    /// A paired `:::` container (DingTalk / VitePress). The fences stay in the source
    /// but are not part of the rendered body.
    Callout {
        /// Full range, including the opening and closing fence lines.
        source: Range<usize>,
        /// Inner Markdown between the fences.
        body: Range<usize>,
    },
    /// Mermaid source without its opening and closing fences.
    Mermaid {
        /// Full fenced source range.
        source: Range<usize>,
        /// Diagram source retained for exact copying, shared with rendered diagram rows.
        code: Arc<str>,
    },
    /// A bounded portion of a code block, flowing through the outer document list.
    CodePage {
        /// Source range, including the opening/closing fence on the first/last page.
        source: Range<usize>,
        /// Code bytes displayed in this page.
        content: Range<usize>,
        /// Entire original code body for one-click copying.
        code: Range<usize>,
        /// Fence language label (bounded to 512 bytes).
        language: String,
    },
    /// Oversized semantic content displayed as bounded, lossless source pages.
    Source {
        /// Full source byte range.
        source: Range<usize>,
        /// UTF-8-safe source page ranges, prepared during background indexing.
        pages: Vec<Range<usize>>,
    },
}

impl Block {
    /// Byte range occupied by this block in the original document.
    pub fn source_range(&self) -> Range<usize> {
        match self {
            Self::Markdown { source }
            | Self::Callout { source, .. }
            | Self::Mermaid { source, .. }
            | Self::Source { source, .. }
            | Self::CodePage { source, .. } => source.clone(),
        }
    }
}

/// A heading and its location in the virtual document list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub(crate) title: String,
    pub(crate) level: u8,
    pub(crate) source: Range<usize>,
    pub(crate) block: usize,
}

impl Heading {
    /// Plain outline text without emphasis delimiters, truncated for oversized headings.
    pub fn title(&self) -> &str {
        &self.title
    }
    /// Heading depth, between one and six.
    pub fn level(&self) -> u8 {
        self.level
    }
    /// Range used to reveal this heading after its chunk is materialized.
    pub fn source_range(&self) -> Range<usize> {
        self.source.clone()
    }
    /// Row index in the virtual document list.
    pub fn block_index(&self) -> usize {
        self.block
    }
}

/// An immutable source buffer and compact semantic block index.
#[derive(Debug, Clone)]
pub struct Document {
    source: Arc<str>,
    blocks: Arc<[Block]>,
    headings: Arc<[Heading]>,
    reference_suffix: Arc<str>,
}

impl Document {
    /// Index a UTF-8 document without creating any UI objects.
    ///
    /// # Errors
    /// Rejects source larger than [`MAX_DOCUMENT_BYTES`].
    pub fn parse(source: String) -> Result<Self, ParseDocumentError> {
        if source.len() > MAX_DOCUMENT_BYTES {
            return Err(ParseDocumentError::TooLarge { size: source.len() });
        }
        let options = Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_MATH;
        let parser = Parser::new_ext(&source, options);
        let mut references = String::new();
        use std::fmt::Write as _;
        for (label, definition) in parser.reference_definitions().iter() {
            let _ = writeln!(
                references,
                "\n[{label}]: <{}> \"{}\"",
                definition.dest,
                definition
                    .title
                    .as_deref()
                    .unwrap_or_default()
                    .replace('"', "\\\"")
            );
        }
        let callouts = callout_spans(&source);
        let mut blocks = Vec::new();
        let mut headings = Vec::new();
        let mut chunk_start = 0;
        let mut depth = 0usize;
        let mut block_start = 0;
        let mut diagram: Option<String> = None;
        let mut heading: Option<(u8, String)> = None;
        let mut oversized = None;
        let mut fenced_code = None;
        for (event, range) in parser.into_offset_iter() {
            match event {
                Event::Start(tag) => {
                    if depth == 0 {
                        block_start = range.start;
                        // A container must stay one row. Splitting at a heading or fence
                        // inside it would leave the closing `:::` in another chunk.
                        let enclosed = callouts.iter().any(|span| {
                            span.source.start < range.start && range.start < span.source.end
                        });
                        // A top-level heading is an exact navigation anchor. Keep its
                        // following paragraphs grouped and all nested blocks intact.
                        if !enclosed
                            && matches!(tag, Tag::Heading { .. })
                            && chunk_start < range.start
                        {
                            if !source[chunk_start..range.start].trim().is_empty() {
                                blocks.push(Block::Markdown {
                                    source: chunk_start..range.start,
                                });
                            }
                            chunk_start = range.start;
                        }
                        if !enclosed {
                            if let Tag::CodeBlock(CodeBlockKind::Fenced(language)) = &tag
                                && range.len() <= 128 * 1024
                                && language
                                    .split_whitespace()
                                    .next()
                                    .is_some_and(|s| s.eq_ignore_ascii_case("mermaid"))
                            {
                                if chunk_start < range.start
                                    && !source[chunk_start..range.start].trim().is_empty()
                                {
                                    blocks.push(Block::Markdown {
                                        source: chunk_start..range.start,
                                    });
                                }
                                diagram = Some(String::new());
                            } else if let Tag::CodeBlock(CodeBlockKind::Fenced(language)) = &tag {
                                if chunk_start < range.start
                                    && !source[chunk_start..range.start].trim().is_empty()
                                {
                                    blocks.push(Block::Markdown {
                                        source: chunk_start..range.start,
                                    });
                                }
                                let language = language.split_whitespace().next().unwrap_or("text");
                                let mut end = language.len().min(MAX_HEADING_LABEL_BYTES);
                                while !language.is_char_boundary(end) {
                                    end -= 1;
                                }
                                fenced_code = Some(language[..end].to_string());
                            } else if range.len() > MAX_RICH_BLOCK_BYTES {
                                if chunk_start < range.start
                                    && !source[chunk_start..range.start].trim().is_empty()
                                {
                                    blocks.push(Block::Markdown {
                                        source: chunk_start..range.start,
                                    });
                                }
                                chunk_start = range.start;
                                oversized = Some(());
                            }
                        }
                    }
                    if let Tag::Heading { level, .. } = tag {
                        heading = Some((level as u8, String::new()));
                    }
                    depth += 1;
                }
                Event::Text(text) | Event::Code(text) => {
                    if let Some(code) = diagram.as_mut() {
                        code.push_str(&text);
                    }
                    if let Some((_, title)) = heading.as_mut() {
                        title.push_str(&text);
                    }
                }
                Event::SoftBreak | Event::HardBreak => {
                    if let Some((_, title)) = heading.as_mut() {
                        title.push(' ');
                    }
                }
                Event::End(tag) => {
                    depth = depth.saturating_sub(1);
                    if matches!(tag, TagEnd::Heading(_))
                        && let Some((level, mut title)) = heading.take()
                    {
                        if title.len() > MAX_HEADING_LABEL_BYTES {
                            let mut end = MAX_HEADING_LABEL_BYTES - '…'.len_utf8();
                            while !title.is_char_boundary(end) {
                                end -= 1;
                            }
                            title.truncate(end);
                            title.push('…');
                        }
                        headings.push(Heading {
                            title,
                            level,
                            source: block_start..range.end,
                            block: 0,
                        });
                    }
                    if depth == 0 {
                        if let Some(code) = diagram.take() {
                            blocks.push(Block::Mermaid {
                                source: block_start..range.end,
                                code: code.into(),
                            });
                            chunk_start = range.end;
                        } else if let Some(language) = fenced_code.take() {
                            let full = block_start..range.end;
                            let code = fenced_code_range(&source, full.clone());
                            let pages = code_pages(&source, code.clone());
                            let count = pages.len();
                            for (index, content) in pages.into_iter().enumerate() {
                                let page_source = if index == 0 {
                                    full.start
                                } else {
                                    content.start
                                }
                                    ..if index + 1 == count {
                                        full.end
                                    } else {
                                        content.end
                                    };
                                blocks.push(Block::CodePage {
                                    source: page_source,
                                    content,
                                    code: code.clone(),
                                    language: language.clone(),
                                });
                            }
                            chunk_start = range.end;
                        } else if oversized.take().is_some() {
                            let full = block_start..range.end;
                            blocks.push(Block::Source {
                                pages: source_pages(&source, full.clone()),
                                source: full,
                            });
                            chunk_start = range.end;
                        } else if !callouts.iter().any(|span| {
                            span.source.start < range.end && range.end < span.source.end
                        }) && range.end - chunk_start >= CHUNK_TARGET_BYTES
                        {
                            blocks.push(Block::Markdown {
                                source: chunk_start..range.end,
                            });
                            chunk_start = range.end;
                        }
                    }
                }
                _ => {}
            }
        }
        if chunk_start < source.len() && !source[chunk_start..].trim().is_empty() {
            blocks.push(Block::Markdown {
                source: chunk_start..source.len(),
            });
        }
        let mut blocks = extract_callouts(&source, blocks, &callouts, references.len());
        // Raw HTML and reference-only tails have no paired top-level Start/End events.
        // Included reference definitions also contribute to each rich row's parsing cost.
        for block in &mut blocks {
            if let Block::Markdown { source: range } = block
                && range.len().saturating_add(references.len()) > MAX_RICH_BLOCK_BYTES
            {
                *block = Block::Source {
                    pages: source_pages(&source, range.clone()),
                    source: range.clone(),
                };
            }
        }
        for heading in &mut headings {
            heading.block = blocks
                .partition_point(|block| block.source_range().end <= heading.source.start)
                .min(blocks.len().saturating_sub(1));
        }
        Ok(Self {
            source: source.into(),
            blocks: blocks.into(),
            headings: headings.into(),
            reference_suffix: references.into(),
        })
    }

    /// Original Markdown source, shared across tabs and background tasks.
    pub fn source(&self) -> &str {
        &self.source
    }
    /// Indexed rows in document order.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }
    /// All document headings in source order.
    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }
    /// Reference definitions appended to each chunk to preserve reference links.
    pub fn reference_suffix(&self) -> &str {
        &self.reference_suffix
    }
    /// Find the virtual row containing a source byte offset.
    pub fn block_at(&self, offset: usize) -> Option<usize> {
        let index = self
            .blocks
            .partition_point(|block| block.source_range().end <= offset);
        (index < self.blocks.len()).then_some(index)
    }
}

struct CalloutSpan {
    source: Range<usize>,
    body: Range<usize>,
}

/// Replace paired `:::` containers with markers exporters can style as a highlight,
/// leaving the inner Markdown intact. Unclosed fences and fences inside code blocks
/// stay literal. The original document bytes are untouched.
pub fn expand_callouts(source: &str) -> Cow<'_, str> {
    let spans = callout_spans(source);
    if spans.is_empty() {
        return Cow::Borrowed(source);
    }
    let mut out = String::with_capacity(source.len());
    let mut cursor = 0;
    for span in &spans {
        out.push_str(&source[cursor..span.source.start]);
        let label = callout_label(&source[span.source.start..span.body.start]);
        write_highlight(&mut out, label, &source[span.body.clone()]);
        cursor = span.source.end;
    }
    out.push_str(&source[cursor..]);
    Cow::Owned(out)
}

/// GitHub alert types. The marker is case-insensitive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertKind {
    Note,
    /// Same blue as [`Self::Note`].
    Info,
    Tip,
    Important,
    Warning,
    Caution,
    /// Same red as [`Self::Caution`].
    Error,
}

impl AlertKind {
    /// CSS class paired with `pecan-highlight`.
    pub fn class(self) -> &'static str {
        match self {
            Self::Note => "pecan-note",
            Self::Info => "pecan-info",
            Self::Tip => "pecan-tip",
            Self::Important => "pecan-important",
            Self::Warning => "pecan-warning",
            Self::Caution => "pecan-caution",
            Self::Error => "pecan-error",
        }
    }
    pub fn icon(self) -> &'static str {
        match self {
            Self::Note | Self::Info => "ℹ️",
            Self::Tip => "💡",
            Self::Important => "❗",
            Self::Warning => "⚠️",
            Self::Caution => "🚫",
            Self::Error => "❌",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Info => "Info",
            Self::Tip => "Tip",
            Self::Important => "Important",
            Self::Warning => "Warning",
            Self::Caution => "Caution",
            Self::Error => "Error",
        }
    }
}

/// `[!NOTE]` or `[!NOTE] title` at the start of a blockquote paragraph.
pub fn alert_marker(text: &str) -> Option<(AlertKind, &str)> {
    let text = text.trim();
    let rest = text.strip_prefix("[!")?;
    let (name, title) = rest.split_once(']')?;
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return None;
    }
    let kind = match name {
        name if name.eq_ignore_ascii_case("NOTE") => AlertKind::Note,
        name if name.eq_ignore_ascii_case("INFO") => AlertKind::Info,
        name if name.eq_ignore_ascii_case("TIP") => AlertKind::Tip,
        name if name.eq_ignore_ascii_case("IMPORTANT") => AlertKind::Important,
        name if name.eq_ignore_ascii_case("WARNING") => AlertKind::Warning,
        name if name.eq_ignore_ascii_case("CAUTION") => AlertKind::Caution,
        name if name.eq_ignore_ascii_case("ERROR") => AlertKind::Error,
        _ => return None,
    };
    Some((kind, title.trim()))
}

/// Turn GitHub alerts into the same highlight markers as `:::` containers, keeping
/// the inner Markdown (including lists) outside the quote. Ordinary quotes stay.
pub fn expand_alerts(source: &str) -> Cow<'_, str> {
    if !source.contains("[!") {
        return Cow::Borrowed(source);
    }
    let mut out = String::with_capacity(source.len());
    let mut offset = 0;
    let mut code: Option<(char, usize)> = None;
    let mut changed = false;
    while offset < source.len() {
        let end = source[offset..]
            .find('\n')
            .map_or(source.len(), |index| offset + index + 1);
        let line = source[offset..end].trim_end_matches(['\n', '\r']);
        if let Some((marker, width)) = code {
            if closing_code_fence(line, marker, width) {
                code = None;
            }
            out.push_str(&source[offset..end]);
            offset = end;
            continue;
        }
        if let Some(fence) = opening_code_fence(line) {
            code = Some(fence);
            out.push_str(&source[offset..end]);
            offset = end;
            continue;
        }
        if let Some((kind, title)) = quote_content(line).and_then(alert_marker) {
            changed = true;
            offset = end;
            let mut body = String::new();
            while offset < source.len() {
                let next = source[offset..]
                    .find('\n')
                    .map_or(source.len(), |index| offset + index + 1);
                let quoted = source[offset..next].trim_end_matches(['\n', '\r']);
                let Some(content) = quote_content(quoted) else {
                    break;
                };
                body.push_str(content);
                body.push('\n');
                offset = next;
            }
            write_alert(&mut out, kind, title, &body);
            continue;
        }
        out.push_str(&source[offset..end]);
        offset = end;
    }
    if changed {
        Cow::Owned(out)
    } else {
        Cow::Borrowed(source)
    }
}

fn quote_content(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let rest = trimmed.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn write_alert(out: &mut String, kind: AlertKind, title: &str, body: &str) {
    use std::fmt::Write as _;
    let title = if title.is_empty() {
        kind.label()
    } else {
        title
    };
    let _ = write!(
        out,
        "<div class=\"pecan-highlight {}\"></div>\n\n{} ",
        kind.class(),
        kind.icon()
    );
    if !title.contains(['*', '[', ']', '<', '>', '`', '\n', '\r']) {
        out.push_str("**");
        out.push_str(title);
        out.push_str("**");
    } else {
        out.push_str(title);
    }
    out.push_str("\n\n");
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n<div class=\"pecan-highlight-end\"></div>\n");
}

fn callout_label(opening: &str) -> &str {
    opening.trim().strip_prefix(":::").unwrap_or("").trim()
}

fn write_highlight(out: &mut String, label: &str, body: &str) {
    // Empty divs bracket the note. A blank line keeps each one a separate HTML block,
    // so the inner Markdown is still parsed instead of being swallowed as HTML.
    if !label.is_empty() {
        out.push_str("<div class=\"pecan-highlight pecan-highlight-titled\"></div>\n\n");
        let plain = !label.contains(['*', '_', '~', '[', ']', '<', '>', '`', '\\']);
        if plain {
            out.push_str("**");
        }
        out.push_str(label);
        if plain {
            out.push_str("**");
        }
        out.push_str("\n\n");
    } else {
        out.push_str("<div class=\"pecan-highlight\"></div>\n\n");
    }
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("\n<div class=\"pecan-highlight-end\"></div>\n");
}

/// Paired container fences at column zero. A code fence wins, so `:::` inside it
/// is diagram or source text rather than a note.
fn callout_spans(source: &str) -> Vec<CalloutSpan> {
    if !source.contains(":::") {
        return Vec::new();
    }
    let mut spans = Vec::new();
    let mut offset = 0;
    let mut code: Option<(char, usize)> = None;
    let mut open: Option<usize> = None;
    let mut body_start = 0;
    while offset < source.len() {
        let end = source[offset..]
            .find('\n')
            .map_or(source.len(), |index| offset + index + 1);
        let line = source[offset..end].trim_end_matches(['\n', '\r']);
        if let Some((marker, width)) = code {
            if closing_code_fence(line, marker, width) {
                code = None;
            }
        } else if let Some(fence) = opening_code_fence(line) {
            code = Some(fence);
        } else if callout_line(line) {
            if let Some(start) = open.take() {
                spans.push(CalloutSpan {
                    source: start..end,
                    body: body_start..offset,
                });
            } else {
                open = Some(offset);
                body_start = end;
            }
        }
        offset = end;
    }
    spans
}

fn callout_line(line: &str) -> bool {
    let Some(rest) = line.trim_end().strip_prefix(":::") else {
        return false;
    };
    !rest.starts_with(':')
}

fn opening_code_fence(line: &str) -> Option<(char, usize)> {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return None;
    }
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let width = trimmed.chars().take_while(|&ch| ch == marker).count();
    if width < 3 {
        return None;
    }
    let rest = &trimmed[width..];
    if marker == '`' && rest.contains('`') {
        return None;
    }
    Some((marker, width))
}

fn closing_code_fence(line: &str, marker: char, width: usize) -> bool {
    let trimmed = line.trim_start_matches(' ');
    if line.len() - trimmed.len() > 3 {
        return false;
    }
    let count = trimmed.chars().take_while(|&ch| ch == marker).count();
    count >= width && trimmed[count..].trim().is_empty()
}

fn extract_callouts(
    source: &str,
    blocks: Vec<Block>,
    spans: &[CalloutSpan],
    references_len: usize,
) -> Vec<Block> {
    if spans.is_empty() {
        return blocks;
    }
    let mut output = Vec::with_capacity(blocks.len());
    for block in blocks {
        let Block::Markdown { source: range } = block else {
            output.push(block);
            continue;
        };
        let mut cursor = range.start;
        let mut split = false;
        for span in spans
            .iter()
            .filter(|span| span.source.start >= range.start && span.body.end <= range.end)
        {
            split = true;
            if cursor < span.source.start && !source[cursor..span.source.start].trim().is_empty() {
                output.push(Block::Markdown {
                    source: cursor..span.source.start,
                });
            }
            if !source[span.body.clone()].trim().is_empty() {
                if span.body.len().saturating_add(references_len) > MAX_RICH_BLOCK_BYTES {
                    output.push(Block::Source {
                        pages: source_pages(source, span.body.clone()),
                        source: span.body.clone(),
                    });
                } else {
                    output.push(Block::Callout {
                        source: span.source.clone(),
                        body: span.body.clone(),
                    });
                }
            }
            cursor = span.source.end;
        }
        if !split {
            output.push(Block::Markdown { source: range });
        } else if cursor < range.end && !source[cursor..range.end].trim().is_empty() {
            output.push(Block::Markdown {
                source: cursor..range.end,
            });
        }
    }
    output
}

fn source_pages(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut pages = Vec::new();
    let mut start = range.start;
    while start < range.end {
        let mut end = (start + SOURCE_PAGE_BYTES).min(range.end);
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        if end < range.end
            && let Some(newline) = source[start..end].rfind('\n')
            && newline > SOURCE_PAGE_BYTES / 2
        {
            end = start + newline + 1;
        }
        pages.push(start..end);
        start = end;
    }
    pages
}

fn fenced_code_range(source: &str, full: Range<usize>) -> Range<usize> {
    let raw = &source[full.clone()];
    let start = raw.find('\n').map_or(raw.len(), |offset| offset + 1);
    let opening = raw[..start].trim_start();
    let marker = opening.chars().next().unwrap_or('`');
    let width = opening.chars().take_while(|&ch| ch == marker).count();
    let last_line = raw.rfind('\n').map_or(start, |offset| offset + 1);
    let last = raw[last_line..].trim();
    let closing_width = last.chars().take_while(|&ch| ch == marker).count();
    let end = if closing_width >= width && last.chars().all(|ch| ch == marker) {
        last_line
    } else {
        raw.len()
    };
    full.start + start..full.start + end.max(start)
}

// Bound both shaping bytes and line count. Splits are presentation-only: copying
// always uses the full code range, and adjacent rows have no visual gaps.
fn code_pages(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut pages = Vec::new();
    let mut start = range.start;
    while start < range.end {
        let mut end = (start + SOURCE_PAGE_BYTES).min(range.end);
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        let candidate = &source[start..end];
        if let Some((offset, _)) = candidate.match_indices('\n').nth(127) {
            end = start + offset + 1;
        } else if end < range.end
            && let Some(offset) = candidate.rfind('\n')
        {
            end = start + offset + 1;
        }
        pages.push(start..end);
        start = end;
    }
    if pages.is_empty() {
        pages.push(range);
    }
    pages
}
