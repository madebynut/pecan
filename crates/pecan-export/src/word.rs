//! Word (.docx) export from the same Markdown events, renderers and image budget as HTML.
//! Structure maps to native Word styles, numbering and tables, so the result stays editable.
use super::{
    Snapshot, code,
    document::{MAX_NOTE_DEPTH, MAX_OUTPUT, highlight_edge, note_text},
    images::Images,
    note_icons::{Icons, SIZE as NOTE_SIZE},
};
use anyhow::{Context as _, Result, ensure};
use http_client::HttpClient;
use pecan_document::AlertKind;
use pecan_math::{MathFont, MathStyle, Presentation};
use pecan_mermaid::{Appearance, DiagramFonts, Region, Renderer, SYSTEM_MONO_FONT, SYSTEM_UI_FONT};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Tag, TagEnd};
use std::{
    collections::HashMap,
    fmt::Write as _,
    io::{Cursor, Write as _},
    ops::Range,
    sync::{Arc, LazyLock},
};

const EMU_PER_PX: f64 = 9525.;
/// A4 with 2 cm margins.
const PAGE: (u32, u32) = (11906, 16838);
const MARGIN: u32 = 1134;
const CONTENT_TWIPS: u32 = PAGE.0 - 2 * MARGIN;
const NOTE_PAD_X: u32 = 240;
const NOTE_PAD_Y: u32 = 180;
/// A 19 pt gutter including the 12 pt mark, independent of font space widths.
const NOTE_GUTTER: u32 = 380;
const CANT_SPLIT: &str = "<w:cantSplit/>";
const CONTENT_EMU: f64 = CONTENT_TWIPS as f64 * 635.;
/// Keeps a tall figure on one page together with a line of text.
const MAX_FIGURE_EMU: f64 = 8_100_000.;
/// Nesting indent for lists and quotes, in twips.
const INDENT: u32 = 420;
/// Body and code sizes match the PDF print style (11 pt and 9 pt).
const BODY_HALF_POINTS: u32 = 22;
const CODE_HALF_POINTS: u32 = 18;
/// Allow the inline mark and baseline to fit without clipping in Word, in twips.
const LANGUAGE_LINE: u32 = 240;
/// A 24 pt minimum band; the row may grow for wrapped or enlarged labels.
const LANGUAGE_ROW_MIN: u32 = 480;
/// The compact header uses its line leading; code body padding stays independent.
const LANGUAGE_PAD_Y: u32 = 0;
/// One 0.5 pt frame and divider in every container, owned by the table: its
/// cells add no borders of their own.
const CODE_BORDERS: &str = "<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:insideV w:val=\"nil\"/></w:tblBorders>";
/// Word gives no height to an empty paragraph that ends a cell directly after
/// a nested table, and the shaded cell then covers that table's bottom border
/// at some zoom levels. A nonbreaking space keeps the paragraph. Its 3 pt line
/// is the room the border needs: shorter lines round away below 60% zoom.
const CELL_END: &str = "<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"60\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"6\"/><w:szCs w:val=\"6\"/></w:rPr></w:pPr><w:r><w:rPr><w:sz w:val=\"6\"/><w:szCs w:val=\"6\"/></w:rPr><w:t>\u{a0}</w:t></w:r></w:p>";
/// The row rhythm: 24 pt for content between the cell margins, without
/// enlarging paragraph leading. A minimum, never an exact height: wrapped
/// cells and lists still grow naturally.
const TABLE_ROW_MIN: u32 = 480;
/// Raise the 8 pt label alongside a 12 pt inline mark; image-run offsets vary by reader.
const LANGUAGE_LABEL_POSITION: &str = "<w:rPr><w:position w:val=\"7\"/></w:rPr>";
/// Character shading stops at the run bounds. Nonbreaking edge runs in the code
/// font at 40% character width extend it into a horizontal inset with the same
/// gray as editable code.
const INLINE_CODE_EDGE: &str = "\u{a0}";
/// Shading alone draws the capsule. Word paints run shading over the whole
/// line box but a character border around the bordered runs' own font box, so
/// a capsule drawn by both shows gray outside its frame whenever taller text
/// shares the line, as CJK text does beside the code font. The gray stays
/// visible on header and quote fills without an outline. Every run repeats
/// the style's shading as direct formatting, as tables do with their borders.
const INLINE_CODE_SHADING: &str = "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"E8E8E8\"/>";

pub(super) async fn docx(snapshot: &Snapshot, client: Arc<dyn HttpClient>) -> Result<Vec<u8>> {
    let mut images = Images::new(&snapshot.directory, client);
    let mut icons = Icons::default();
    let mut language_icons = super::language_icons::Icons::default();
    let prefs = &snapshot.preferences;
    // Formulas are typeset at reading sizes; keep their ratio to the 11 pt body.
    let math_scale = f64::from(BODY_HALF_POINTS) / 2. / (prefs.font_size * 0.75);
    let renderer = &snapshot.diagrams.renderer;
    let fonts = word_fonts(&prefs.english_font, &prefs.chinese_font);
    let mut out = Docx {
        // PingFang has no Bold face; Word leaves `w:b` at regular weight unless the
        // run names the Semibold face directly.
        note_line: print_line(prefs.line_height) as f32 / 20.,
        cjk_bold: cjk_emphasis_face(fonts.chinese_family()).map(str::to_owned),
        metrics: Some(super::typography::Fonts::new(
            fonts.english_family(),
            fonts.chinese_family(),
            &prefs.code_font,
        )),
        ..Docx::default()
    };
    let mut formats = super::code::Formats::new(
        snapshot.codes.clone(),
        snapshot.preferences.code_tab_size.clone(),
    );
    let source = super::document::export_markdown(snapshot.document.source());
    let mut parser = super::table::events(source.as_ref());
    while let Some(event) = parser.next() {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind {
                    CodeBlockKind::Fenced(info) => {
                        info.split_whitespace().next().unwrap_or("").to_owned()
                    }
                    CodeBlockKind::Indented => String::new(),
                };
                let mut code = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::CodeBlock) => break,
                        Event::Text(text) => code.push_str(&text),
                        _ => {}
                    }
                }
                let code = code.strip_suffix('\n').unwrap_or(&code);
                if language.eq_ignore_ascii_case("mermaid") {
                    let _permit = snapshot.diagrams.slots.acquire().await;
                    let diagram = renderer
                        .render_with_fonts(code, Appearance::Light, &prefs.diagram_fonts())
                        .context("导出 Mermaid")?;
                    // Print resolution, within the whole-raster budget.
                    let scale = diagram.max_scale().min(3.);
                    let png = diagram.png_region(Region::whole(&diagram, scale), None)?;
                    let (width, height) = diagram.dimensions();
                    let picture = out.media(png, "png", (f64::from(width), f64::from(height)));
                    out.figure(&picture, "Mermaid 图表");
                } else {
                    let icon = pecan_assets::language::for_language(&language);
                    let logo = if let Some(picture) = out.language_pictures.get(icon.path) {
                        picture.clone()
                    } else {
                        let png = language_icons.png(&language, renderer)?;
                        let size = f64::from(super::language_icons::SIZE);
                        let picture = out.media(png.to_vec(), "png", (size, size));
                        out.language_pictures.insert(icon.path, picture.clone());
                        picture
                    };
                    let formatted = formats.resolve(&language, code).await;
                    out.code_block(
                        formatted.as_ref().map_or(code, |output| output.text()),
                        &language,
                        Some(&logo),
                    );
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                let mut alt = String::new();
                for event in parser.by_ref() {
                    match event {
                        Event::End(TagEnd::Image) => break,
                        Event::Text(text) | Event::Code(text) => alt.push_str(&text),
                        _ => {}
                    }
                }
                let picture = out.picture(&mut images, renderer, &dest_url, None).await?;
                out.drawing(&picture, fit(picture.size, 1.), &alt, None);
            }
            Event::InlineMath(ref source) | Event::DisplayMath(ref source) => {
                let display = matches!(event, Event::DisplayMath(_));
                let _permit = snapshot.math.slots.acquire().await;
                let formula = snapshot
                    .math
                    .renderer
                    .render(
                        source,
                        Presentation {
                            font: if prefs.math_font == "modern" {
                                MathFont::LatinModern
                            } else {
                                MathFont::StixTwo
                            },
                            style: if display {
                                MathStyle::Display
                            } else {
                                MathStyle::Inline
                            },
                            font_size: prefs.math_size.round() as u8,
                            color: 0x242424,
                        },
                    )
                    .context("导出 LaTeX")?;
                let (width, height) = formula.dimensions();
                let size = (f64::from(width), f64::from(height));
                let picture = out.media(formula.png().to_vec(), "png", size);
                if display {
                    out.figure(&picture, formula.speech());
                } else {
                    // Lower the image so the formula baseline sits on the text baseline.
                    let depth = f64::from(height - formula.baseline()) * math_scale;
                    let lowered = -(depth * 1.5).round() as i32;
                    out.drawing(
                        &picture,
                        fit(size, math_scale),
                        formula.speech(),
                        Some(lowered),
                    );
                }
            }
            Event::Html(raw) | Event::InlineHtml(raw) => {
                if let Some(marker) = highlight_edge(&raw) {
                    match marker {
                        super::document::NoteMarker::Open { kind, titled } => {
                            ensure!(
                                out.frames.len() < MAX_NOTE_DEPTH,
                                "Word 提示块嵌套超过 32 层"
                            );
                            let png = icons.png(kind, renderer)?.to_vec();
                            let size = f64::from(NOTE_SIZE);
                            let icon = out.media(png, "png", (size, size));
                            out.note_start(icon, kind, titled);
                        }
                        super::document::NoteMarker::Close => {
                            out.note_end();
                        }
                    }
                } else {
                    out.html(&raw, &mut images, renderer).await?;
                }
            }
            Event::Start(Tag::BlockQuote(kind)) => {
                ensure!(
                    out.frames.len() < MAX_NOTE_DEPTH,
                    "Word 引用块嵌套超过 32 层"
                );
                out.event(Event::Start(Tag::BlockQuote(kind)));
            }
            event => out.event(event),
        }
        ensure!(out.size() <= MAX_OUTPUT, "Word 文档超过 128 MiB 导出预算");
    }
    out.finish(snapshot)
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Body,
    Heading(u8),
    Code,
    Center,
    Cell(Alignment),
}

enum Block {
    Quote,
    List { num: u32, level: u32 },
    Item { numbered: bool },
    Footnote { label: String, pending: bool },
}

#[derive(Default)]
struct Inline {
    bold: u32,
    italic: u32,
    strike: u32,
    code: u32,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum TextEdge {
    #[default]
    Other,
    Italic,
    Code,
}

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Text,
    CodePadding,
    Superscript,
}

struct NoteState {
    icon: Option<Picture>,
    prefix: Option<AlertKind>,
    led: bool,
    titled: bool,
}

struct Frame {
    table: Option<Table>,
    width: u32,
    cant_split_at: usize,
    /// A lower bound from native paragraph minimum heights, in twips. Word
    /// still owns wrapping; no second pagination engine guesses its page breaks.
    minimum_height: u32,
}

enum FrameStyle {
    Quote,
    Note(Option<AlertKind>),
}

struct Table {
    aligns: Vec<Alignment>,
    column: usize,
    head: bool,
    filled: bool,
}

#[derive(Clone)]
struct Picture {
    relationship: String,
    /// Logical (96 dpi) pixels.
    size: (f64, f64),
}

struct SymbolLayout {
    offset: usize,
    picture: Picture,
    ink: Option<super::typography::Ink>,
    first_row: bool,
}

#[derive(Default)]
struct Docx {
    body: String,
    relationships: String,
    next_relationship: u32,
    media: Vec<(String, Vec<u8>)>,
    media_relationships: Vec<String>,
    media_index: HashMap<u64, Vec<usize>>,
    media_bytes: usize,
    pictures: HashMap<String, Picture>,
    language_pictures: HashMap<&'static str, Picture>,
    /// Start value and level of each ordered list; numbering id is index + 2.
    ordered: Vec<(u32, u32)>,
    drawings: u32,
    /// East-Asian face that actually has a heavier weight, when the chosen family does not.
    cjk_bold: Option<String>,
    metrics: Option<super::typography::Fonts>,
    symbol: Option<SymbolLayout>,
    note_line: f32,
    highlight: bool,
    highlight_icon: Option<Picture>,
    /// Emoji emitted by alert expansion, consumed only in the generated title.
    highlight_prefix: Option<AlertKind>,
    /// The open note already wrote its leading mark.
    highlight_led: bool,
    highlight_titled: bool,
    notes: Vec<NoteState>,
    frames: Vec<Frame>,
    /// The paragraph written after the latest table. A cell that ends while
    /// `body` still ends here replaces it with [`CELL_END`].
    separator: Option<Range<usize>>,
    blocks: Vec<Block>,
    open: bool,
    /// Kind of the open paragraph.
    kind: Option<Kind>,
    link: bool,
    inline: Inline,
    text_edge: TextEdge,
    table: Option<Table>,
}

impl Docx {
    fn content_width(&self) -> u32 {
        let width = self.frames.last().map_or(CONTENT_TWIPS, |frame| {
            frame.width.saturating_sub(2 * NOTE_PAD_X)
        });
        self.table
            .as_ref()
            .map_or(width, |table| {
                (width / table.aligns.len().max(1) as u32).saturating_sub(270)
            })
            .max(240)
    }

    fn frame_start(&mut self, style: FrameStyle) {
        self.end();
        let width = self.content_width();
        let mut table = self.table.take();
        if let Some(table) = &mut table {
            table.filled = true;
        }
        let (caption, fill, edge, align, left_only) = match style {
            FrameStyle::Quote => ("引用块", [0xf7; 3], [0xd4; 3], "center", true),
            FrameStyle::Note(kind) => {
                let (fill, edge) = pecan_assets::notes::export_colors(kind);
                ("提示块", fill, edge, "top", false)
            }
        };
        let hex = |[r, g, b]: [u8; 3]| format!("{r:02X}{g:02X}{b:02X}");
        let fill = hex(fill);
        let edge = hex(edge);
        let _ = write!(
            self.body,
            "<w:tbl><w:tblPr><w:tblW w:w=\"{width}\" w:type=\"dxa\"/><w:tblBorders>"
        );
        for side in ["top", "left", "bottom", "right"] {
            let (val, size) = if left_only {
                if side == "left" {
                    ("single", 18)
                } else {
                    ("nil", 0)
                }
            } else {
                ("single", 4)
            };
            let _ = write!(
                self.body,
                "<w:{side} w:val=\"{val}\" w:sz=\"{size}\" w:space=\"0\" w:color=\"{edge}\"/>"
            );
        }
        let _ = write!(
            self.body,
            "</w:tblBorders><w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"{NOTE_PAD_Y}\" w:type=\"dxa\"/><w:left w:w=\"{NOTE_PAD_X}\" w:type=\"dxa\"/><w:bottom w:w=\"{NOTE_PAD_Y}\" w:type=\"dxa\"/><w:right w:w=\"{NOTE_PAD_X}\" w:type=\"dxa\"/></w:tblCellMar><w:tblCaption w:val=\"{caption}\"/></w:tblPr><w:tblGrid><w:gridCol w:w=\"{width}\"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/></w:trPr><w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/><w:shd w:val=\"clear\" w:fill=\"{fill}\"/><w:vAlign w:val=\"{align}\"/></w:tcPr>"
        );
        self.frames.push(Frame {
            table,
            width,
            cant_split_at: self.body.rfind(CANT_SPLIT).expect("frame row was written"),
            minimum_height: 2 * NOTE_PAD_Y,
        });
    }
    fn frame_end(&mut self) {
        self.end();
        let Some(frame) = self.frames.pop() else {
            return;
        };
        // Before the row edit below, which shifts the recorded offsets.
        self.cell_final();
        // A row that cannot possibly fit a page must be splittable. Otherwise
        // compatible readers may put the preceding heading on an empty page
        // before eventually ignoring cantSplit. Short rows keep native borders.
        if frame.minimum_height >= PAGE.1 - 2 * MARGIN {
            self.body.replace_range(
                frame.cant_split_at..frame.cant_split_at + CANT_SPLIT.len(),
                "",
            );
        }
        self.body.push_str("</w:tc></w:tr></w:tbl>");
        self.table_gap();
        self.table = frame.table;
    }
    /// Native tables need a trailing paragraph. Its mark must fit its 1 pt
    /// line, or Word paints the inherited body font over the thin frame; the
    /// 4 pt after it separate the table from the content that follows.
    fn table_gap(&mut self) {
        self.separate("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"80\" w:line=\"20\" w:lineRule=\"exact\"/><w:rPr><w:sz w:val=\"2\"/><w:szCs w:val=\"2\"/></w:rPr></w:pPr></w:p>");
    }
    /// Write the paragraph Word requires after a table.
    fn separate(&mut self, paragraph: &str) {
        let start = self.body.len();
        self.body.push_str(paragraph);
        self.separator = Some(start..self.body.len());
    }
    /// Every cell ends with a paragraph that Word lays out; see [`CELL_END`].
    fn cell_final(&mut self) {
        match self.separator.take() {
            Some(gap) if gap.end == self.body.len() => self.body.truncate(gap.start),
            _ if self.body.ends_with("</w:p>") => return,
            _ => {}
        }
        self.body.push_str(CELL_END);
    }
    fn note_start(&mut self, icon: Picture, kind: Option<AlertKind>, titled: bool) {
        self.frame_start(FrameStyle::Note(kind));
        self.notes.push(NoteState {
            icon: self.highlight_icon.take(),
            prefix: self.highlight_prefix.take(),
            led: self.highlight_led,
            titled: self.highlight_titled,
        });
        self.highlight = true;
        self.highlight_icon = Some(icon);
        self.highlight_prefix = kind;
        self.highlight_led = false;
        self.highlight_titled = titled;
    }
    fn note_end(&mut self) {
        let Some(previous) = self.notes.pop() else {
            self.end();
            return;
        };
        self.frame_end();
        self.highlight = !self.notes.is_empty();
        self.highlight_icon = previous.icon;
        self.highlight_prefix = previous.prefix;
        self.highlight_led = previous.led;
        self.highlight_titled = previous.titled;
    }

    fn size(&self) -> usize {
        self.body.len() + self.media_bytes
    }
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.begin(self.default_kind()),
                Tag::Heading { level, .. } => self.begin(Kind::Heading(level as u8)),
                Tag::BlockQuote(_) => {
                    self.push(Block::Quote);
                    self.frame_start(FrameStyle::Quote);
                }
                Tag::List(start) => {
                    let level = self.list_depth();
                    let num = match start {
                        Some(start) => {
                            self.ordered
                                .push((start.min(u64::from(u32::MAX)) as u32, level));
                            self.ordered.len() as u32 + 1
                        }
                        None => 1,
                    };
                    self.push(Block::List { num, level });
                }
                Tag::Item => self.push(Block::Item { numbered: false }),
                Tag::FootnoteDefinition(label) => self.push(Block::Footnote {
                    label: label.to_string(),
                    pending: true,
                }),
                Tag::Table(aligns) => self.table_start(aligns),
                Tag::TableHead => self.row_start(true),
                Tag::TableRow => self.row_start(false),
                Tag::TableCell => self.cell_start(),
                Tag::Emphasis => self.inline.italic += 1,
                Tag::Strong => self.inline.bold += 1,
                Tag::Strikethrough => self.inline.strike += 1,
                Tag::Link { dest_url, .. } => self.link_start(&dest_url),
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock => self.end(),
                TagEnd::BlockQuote(_) => {
                    self.pop();
                    self.frame_end();
                }
                TagEnd::List(_) | TagEnd::Item | TagEnd::FootnoteDefinition => self.pop(),
                TagEnd::Table => self.table_end(),
                TagEnd::TableHead | TagEnd::TableRow => self.row_end(),
                TagEnd::TableCell => self.cell_end(),
                TagEnd::Emphasis => self.inline.italic = self.inline.italic.saturating_sub(1),
                TagEnd::Strong => self.inline.bold = self.inline.bold.saturating_sub(1),
                TagEnd::Strikethrough => {
                    self.inline.strike = self.inline.strike.saturating_sub(1);
                }
                TagEnd::Link => self.link_end(),
                _ => {}
            },
            Event::Text(text) => {
                let text = note_text(&text, &mut self.highlight_prefix);
                self.text(text, Style::Text);
            }
            Event::Code(text) => self.code_span(&text),
            Event::SoftBreak => self.text(" ", Style::Text),
            Event::HardBreak => self.line_break(),
            Event::Rule => self.rule(),
            Event::TaskListMarker(done) => {
                self.text(if done { "☑ " } else { "☐ " }, Style::Text);
            }
            Event::FootnoteReference(label) => {
                self.text(&format!("[{label}]"), Style::Superscript);
            }
            _ => {}
        }
    }

    // Blocks and paragraphs.

    fn push(&mut self, block: Block) {
        self.end();
        self.blocks.push(block);
    }
    fn pop(&mut self) {
        self.end();
        self.blocks.pop();
    }
    fn list_depth(&self) -> u32 {
        self.blocks
            .iter()
            .filter(|block| matches!(block, Block::List { .. }))
            .count() as u32
    }
    fn default_kind(&self) -> Kind {
        match &self.table {
            Some(table) => Kind::Cell(
                table
                    .aligns
                    .get(table.column)
                    .copied()
                    .unwrap_or(Alignment::None),
            ),
            None => Kind::Body,
        }
    }
    fn begin(&mut self, kind: Kind) {
        self.end();
        self.text_edge = TextEdge::Other;
        self.open = true;
        self.kind = Some(kind);
        if let Some(table) = &mut self.table {
            table.filled = true;
        }
        let quotes = self
            .blocks
            .iter()
            .filter(|block| matches!(block, Block::Quote))
            .count() as u32;
        // The innermost item's first paragraph carries the bullet or number; later
        // paragraphs of the item align with its text.
        let mut number = None;
        let mut level = None;
        if let Some(index) = self
            .blocks
            .iter()
            .rposition(|block| matches!(block, Block::Item { .. }))
        {
            let list = self.blocks[..index]
                .iter()
                .rev()
                .find_map(|block| match block {
                    Block::List { num, level } => Some((*num, *level)),
                    _ => None,
                });
            if let (Some((num, depth)), Block::Item { numbered }) = (list, &mut self.blocks[index])
            {
                level = Some(depth);
                if !*numbered && kind != Kind::Code {
                    *numbered = true;
                    number = Some((num, depth));
                }
            }
        }
        self.body.push_str("<w:p><w:pPr>");
        let note_title = self.highlight && self.highlight_titled && !self.highlight_led;
        let note_lead = self.highlight
            && !self.highlight_led
            && matches!(kind, Kind::Body)
            && self.highlight_icon.is_some();
        if self.table.is_none() {
            // Table columns share row height and must not be added together.
            // Only sequential paragraphs contribute to this conservative bound.
            let minimum = match kind {
                Kind::Body if self.highlight => {
                    (if note_title {
                        300
                    } else {
                        (self.note_line * 20.) as u32
                    }) + 80
                }
                Kind::Body => BODY_HALF_POINTS * 10,
                Kind::Code => CODE_HALF_POINTS * 10,
                _ => 0,
            };
            for frame in &mut self.frames {
                frame.minimum_height = frame.minimum_height.saturating_add(minimum);
            }
        }
        match kind {
            Kind::Heading(level) => {
                let _ = write!(
                    self.body,
                    "<w:pStyle w:val=\"Heading{}\"/>",
                    level.clamp(1, 6)
                );
            }
            Kind::Code => self.body.push_str("<w:pStyle w:val=\"Code\"/>"),
            Kind::Cell(_) => self.body.push_str("<w:pStyle w:val=\"TableCell\"/>"),
            Kind::Body if self.highlight => self.body.push_str(if note_title {
                "<w:pStyle w:val=\"NoteTitle\"/>"
            } else {
                "<w:pStyle w:val=\"Highlight\"/>"
            }),
            Kind::Body if quotes > 0 => self.body.push_str("<w:pStyle w:val=\"Quote\"/>"),
            _ => {}
        }
        if let Some((num, depth)) = number {
            let _ = write!(
                self.body,
                "<w:numPr><w:ilvl w:val=\"{depth}\"/><w:numId w:val=\"{num}\"/></w:numPr>"
            );
        }
        if note_lead {
            let _ = write!(
                self.body,
                "<w:tabs><w:tab w:val=\"left\" w:pos=\"{NOTE_GUTTER}\"/></w:tabs>"
            );
        }
        let gutter = if note_lead { NOTE_GUTTER } else { 0 };
        let left = level.map_or(0, |depth| (depth + 1) * INDENT) + gutter;
        if left > 0 {
            let hanging = if number.is_some() { INDENT } else { 0 } + gutter;
            let _ = write!(
                self.body,
                "<w:ind w:left=\"{left}\" w:hanging=\"{hanging}\"/>"
            );
        }
        let justify = match kind {
            Kind::Center | Kind::Cell(Alignment::Center) => Some("center"),
            Kind::Cell(Alignment::Right) => Some("right"),
            Kind::Body if quotes == 0 && level.is_none() && !self.highlight => Some("both"),
            _ => None,
        };
        if let Some(justify) = justify {
            let _ = write!(self.body, "<w:jc w:val=\"{justify}\"/>");
        }
        if self.highlight && !self.highlight_led {
            self.body.push_str("<w:textAlignment w:val=\"baseline\"/>");
        }
        self.body.push_str("</w:pPr>");
        let label = self.blocks.iter_mut().rev().find_map(|block| match block {
            Block::Footnote { label, pending } if *pending => {
                *pending = false;
                Some(format!("[{label}] "))
            }
            _ => None,
        });
        if let Some(label) = label {
            self.run(&label, Style::Superscript, None);
        }
        if self.highlight
            && !self.highlight_led
            && matches!(kind, Kind::Body)
            && let Some(icon) = self.highlight_icon.clone()
        {
            self.highlight_led = true;
            // Measure the first line's visible text. Word owns the line baseline;
            // the inline symbol moves relative to it, never to a guessed paragraph top.
            self.symbol = Some(SymbolLayout {
                offset: self.body.len(),
                picture: icon,
                ink: None,
                first_row: true,
            });
        }
    }
    fn end(&mut self) {
        if self.link {
            self.body.push_str("</w:hyperlink>");
            self.link = false;
        }
        if self.open {
            if let Some(symbol) = self.symbol.take() {
                let tail = self.body.split_off(symbol.offset);
                let height = symbol.picture.size.1 * 0.75;
                let lowered = symbol
                    .ink
                    .map(|ink| -(f64::from(ink.object_drop(height as f32)) * 2.).round() as i32);
                self.drawing_run(&symbol.picture, fit(symbol.picture.size, 1.), "", lowered);
                // The explicit tab stop and hanging indent keep both the first
                // and wrapped lines outside the symbol's gutter.
                self.body.push_str("<w:r><w:tab/></w:r>");
                self.body.push_str(&tail);
            }
            self.body.push_str("</w:p>");
            self.open = false;
        }
    }
    fn ensure(&mut self) {
        if !self.open {
            self.begin(self.default_kind());
        }
    }

    // Inline content.

    fn code_span(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.code_start();
        for event in super::document::code_events(text) {
            self.event(event);
        }
        self.code_end();
    }

    fn code_start(&mut self) {
        self.ensure();
        if self.inline.code == 0 {
            if self.text_edge == TextEdge::Italic {
                self.code_gap();
            }
            self.text(INLINE_CODE_EDGE, Style::CodePadding);
        }
        self.inline.code += 1;
    }

    fn code_end(&mut self) {
        if self.inline.code == 0 {
            return;
        }
        self.inline.code -= 1;
        if self.inline.code == 0 {
            self.text(INLINE_CODE_EDGE, Style::CodePadding);
            self.text_edge = TextEdge::Code;
        }
    }

    /// Italic ink can extend beyond its advance width. Only adjacent italic
    /// text without a source space needs this small, unshaded clearance.
    fn code_gap(&mut self) {
        self.body
            .push_str("<w:r><w:rPr><w:rStyle w:val=\"CodeGap\"/>");
        if !matches!(self.kind, Some(Kind::Heading(_))) {
            self.body
                .push_str("<w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/>");
        }
        self.body
            .push_str("</w:rPr><w:t xml:space=\"preserve\">\u{a0}</w:t></w:r>");
    }

    fn text(&mut self, text: &str, style: Style) {
        if text.is_empty() {
            return;
        }
        self.ensure();
        self.run(text, style, None);
    }
    fn run(&mut self, text: &str, style: Style, ink: Option<code::Ink>) {
        let inline_code = style == Style::CodePadding || self.inline.code > 0;
        if !inline_code
            && self.text_edge == TextEdge::Code
            && self.inline.italic > 0
            && text.starts_with(|c: char| !c.is_whitespace())
        {
            self.code_gap();
        }
        self.body.push_str("<w:r>");
        let mut properties = String::new();
        if inline_code {
            properties.push_str(if style == Style::CodePadding {
                "<w:rStyle w:val=\"CodePadding\"/>"
            } else {
                "<w:rStyle w:val=\"CodeChar\"/>"
            });
        } else if self.link {
            properties.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
        }
        let head = self.table.as_ref().is_some_and(|table| table.head);
        let emphasize = self.inline.bold > 0
            || head
            || matches!(self.kind, Some(Kind::Heading(_)))
            || ink.is_some_and(|ink| ink.bold);
        if emphasize
            && style != Style::CodePadding
            && let Some(face) = &self.cjk_bold
        {
            let _ = write!(
                properties,
                "<w:rFonts w:eastAsia=\"{face}\" w:hint=\"eastAsia\"/>"
            );
        }
        if emphasize {
            properties.push_str("<w:b/><w:bCs/>");
        }
        if self.symbol.as_ref().is_some_and(|symbol| symbol.first_row)
            && style != Style::CodePadding
        {
            use super::typography::{Fonts, Role, Style as TextStyle};
            let mut text_style = TextStyle::new(
                if inline_code { Role::Code } else { Role::Body },
                if inline_code { 10. } else { 11. },
                [0x24; 3],
            );
            text_style.bold = emphasize;
            text_style.italic = self.inline.italic > 0;
            let measured = self
                .metrics
                .get_or_insert_with(|| Fonts::new("", "", ""))
                .measure_text(text, &text_style);
            if let Some(symbol) = &mut self.symbol
                && let Some(ink) = measured.ink
            {
                symbol.ink = Some(symbol.ink.map_or(ink, |old| old.union(ink)));
            }
        }
        if self.inline.italic > 0 || ink.is_some_and(|ink| ink.italic) {
            properties.push_str("<w:i/><w:iCs/>");
        }
        if self.inline.strike > 0 {
            properties.push_str("<w:strike/>");
        }
        if let Some([red, green, blue]) = ink.and_then(|ink| ink.rgb) {
            let _ = write!(
                properties,
                "<w:color w:val=\"{red:02X}{green:02X}{blue:02X}\"/>"
            );
        }
        // Like the HTML `.88em`: smaller than body text, but headings keep their size.
        if inline_code && !matches!(self.kind, Some(Kind::Heading(_))) {
            properties.push_str("<w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/>");
        }
        if inline_code {
            properties.push_str(INLINE_CODE_SHADING);
        }
        if style == Style::Superscript {
            properties.push_str("<w:vertAlign w:val=\"superscript\"/>");
        }
        if !properties.is_empty() {
            let _ = write!(self.body, "<w:rPr>{properties}</w:rPr>");
        }
        for (index, piece) in text.split('\t').enumerate() {
            if index > 0 {
                self.body.push_str("<w:tab/>");
            }
            if !piece.is_empty() {
                self.body.push_str("<w:t xml:space=\"preserve\">");
                escape_into(&mut self.body, piece);
                self.body.push_str("</w:t>");
            }
        }
        self.body.push_str("</w:r>");
        if !inline_code {
            self.text_edge =
                if self.inline.italic > 0 && text.ends_with(|c: char| !c.is_whitespace()) {
                    TextEdge::Italic
                } else {
                    TextEdge::Other
                };
        }
    }
    fn line_break(&mut self) {
        self.ensure();
        self.text_edge = TextEdge::Other;
        if let Some(symbol) = &mut self.symbol {
            symbol.first_row = false;
        }
        self.body.push_str("<w:r><w:br/></w:r>");
    }
    fn rule(&mut self) {
        self.end();
        self.body.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"E1E1E1\"/></w:pBdr></w:pPr></w:p>");
    }
    fn link_start(&mut self, destination: &str) {
        self.ensure();
        if self.link {
            return;
        }
        let external = ["https://", "http://", "mailto:"]
            .iter()
            .any(|scheme| destination.starts_with(scheme));
        if external {
            let id = self.relationship("hyperlink", destination, true);
            let _ = write!(self.body, "<w:hyperlink r:id=\"{id}\" w:history=\"1\">");
            self.link = true;
        }
    }
    fn link_end(&mut self) {
        if self.link {
            self.body.push_str("</w:hyperlink>");
            self.link = false;
        }
    }

    // Code, tables and figures.

    fn code_block(&mut self, source: &str, language: &str, logo: Option<&Picture>) {
        self.end();
        let width = self.content_width();
        let label_properties = if logo.is_some() {
            LANGUAGE_LABEL_POSITION
        } else {
            ""
        };
        let parent = self.table.take();
        let body_fill = format!("{:06X}", pecan_assets::LIGHT_CODE_BODY);
        let header_fill = format!("{:06X}", pecan_assets::LIGHT_CODE_HEADER);
        let mut label = String::new();
        escape_into(
            &mut label,
            &if language.is_empty() {
                "text"
            } else {
                language
            }
            .to_uppercase(),
        );
        let _ = write!(
            self.body,
            "<w:tbl><w:tblPr><w:tblW w:w=\"{width}\" w:type=\"dxa\"/>{CODE_BORDERS}<w:tblLayout w:type=\"fixed\"/><w:tblCellMar><w:top w:w=\"100\" w:type=\"dxa\"/><w:left w:w=\"180\" w:type=\"dxa\"/><w:bottom w:w=\"100\" w:type=\"dxa\"/><w:right w:w=\"180\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr><w:tblGrid><w:gridCol w:w=\"{width}\"/></w:tblGrid><w:tr><w:trPr><w:cantSplit/><w:trHeight w:val=\"{LANGUAGE_ROW_MIN}\" w:hRule=\"atLeast\"/><w:tblHeader/></w:trPr><w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/><w:shd w:val=\"clear\" w:fill=\"{header_fill}\"/><w:tcMar><w:top w:w=\"{LANGUAGE_PAD_Y}\" w:type=\"dxa\"/><w:bottom w:w=\"{LANGUAGE_PAD_Y}\" w:type=\"dxa\"/></w:tcMar><w:vAlign w:val=\"center\"/></w:tcPr><w:p><w:pPr><w:pStyle w:val=\"CodeLanguage\"/><w:keepNext/><w:jc w:val=\"right\"/></w:pPr>"
        );
        if let Some(logo) = logo {
            self.drawing_run(logo, fit(logo.size, 1.), "", None);
            self.body
                .push_str("<w:r><w:t xml:space=\"preserve\">\u{2002}</w:t></w:r>");
        }
        let _ = write!(
            self.body,
            "<w:r>{label_properties}<w:t>{}</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/><w:shd w:val=\"clear\" w:fill=\"{body_fill}\"/></w:tcPr>",
            label
        );
        self.begin(Kind::Code);
        for (range, ink) in code::spans(source, language) {
            for (index, line) in source[range].split('\n').enumerate() {
                if index > 0 {
                    self.end();
                    self.begin(Kind::Code);
                }
                if !line.is_empty() {
                    let ink = (ink != code::Ink::default()).then_some(ink);
                    self.run(line, Style::Text, ink);
                }
            }
        }
        self.end();
        self.body.push_str("</w:tc></w:tr></w:tbl>");
        self.table_gap();
        self.table = parent;
        if let Some(table) = &mut self.table {
            table.filled = true;
        }
    }
    fn table_start(&mut self, aligns: Vec<Alignment>) {
        self.end();
        let columns = aligns.len().max(1) as u32;
        let width = self.content_width();
        // Borders and width are repeated as direct formatting: some readers ignore
        // table-style borders and percentage widths.
        let _ = write!(
            self.body,
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"PecanTable\"/><w:tblW w:w=\"{width}\" w:type=\"dxa\"/>{TABLE_BORDERS}<w:tblLayout w:type=\"autofit\"/>{TABLE_MARGINS}<w:tblLook w:val=\"04A0\" w:firstRow=\"1\" w:lastRow=\"0\" w:firstColumn=\"0\" w:lastColumn=\"0\" w:noHBand=\"1\" w:noVBand=\"1\"/></w:tblPr><w:tblGrid>"
        );
        for _ in 0..columns {
            let _ = write!(self.body, "<w:gridCol w:w=\"{}\"/>", width / columns);
        }
        self.body.push_str("</w:tblGrid>");
        self.table = Some(Table {
            aligns,
            column: 0,
            head: false,
            filled: false,
        });
    }
    fn row_start(&mut self, head: bool) {
        if let Some(table) = &mut self.table {
            table.head = head;
            table.column = 0;
        }
        let _ = write!(
            self.body,
            "<w:tr><w:trPr><w:cantSplit/><w:trHeight w:val=\"{TABLE_ROW_MIN}\" w:hRule=\"atLeast\"/>{}</w:trPr>",
            if head { "<w:tblHeader/>" } else { "" }
        );
    }
    fn row_end(&mut self) {
        self.end();
        self.body.push_str("</w:tr>");
        if let Some(table) = &mut self.table {
            table.head = false;
        }
    }
    fn cell_start(&mut self) {
        self.end();
        let head = self.table.as_ref().is_some_and(|table| table.head);
        // Explicit cell fill keeps nested tables independent of their container.
        let fill = if head { "F5F5F5" } else { "FFFFFF" };
        let _ = write!(
            self.body,
            "<w:tc><w:tcPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{fill}\"/><w:vAlign w:val=\"center\"/></w:tcPr>"
        );
        if let Some(table) = &mut self.table {
            table.filled = false;
        }
    }
    fn cell_end(&mut self) {
        // Every cell needs at least one paragraph.
        if !self.table.as_ref().is_some_and(|table| table.filled) {
            self.begin(self.default_kind());
        }
        self.end();
        self.cell_final();
        self.body.push_str("</w:tc>");
        if let Some(table) = &mut self.table {
            table.column += 1;
        }
    }
    fn table_end(&mut self) {
        self.end();
        self.body.push_str("</w:tbl>");
        self.table = None;
        // Separates adjacent tables, which Word would otherwise merge.
        self.separate("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr></w:p>");
    }
    fn figure(&mut self, picture: &Picture, alt: &str) {
        self.end();
        self.begin(Kind::Center);
        let (width, height) = fit(picture.size, 1.);
        let shrink = (f64::from(self.content_width()) * 635. / width as f64).min(1.);
        self.drawing(
            picture,
            (
                (width as f64 * shrink).round() as u64,
                (height as f64 * shrink).round() as u64,
            ),
            alt,
            None,
        );
        self.end();
    }
    /// Identical bytes, such as one file referenced two ways or a repeated formula,
    /// share a single part.
    fn media(&mut self, bytes: Vec<u8>, extension: &str, size: (f64, f64)) -> Picture {
        use std::hash::{BuildHasher as _, RandomState};
        static HASHER: LazyLock<RandomState> = LazyLock::new(RandomState::new);
        let hash = HASHER.hash_one(&bytes);
        let existing = self
            .media_index
            .get(&hash)
            .and_then(|indices| indices.iter().find(|&&index| self.media[index].1 == bytes));
        if let Some(&index) = existing {
            return Picture {
                relationship: self.media_relationships[index].clone(),
                size,
            };
        }
        let name = format!("image{}.{extension}", self.media.len() + 1);
        let relationship = self.relationship("image", &format!("media/{name}"), false);
        self.media_index
            .entry(hash)
            .or_default()
            .push(self.media.len());
        self.media_relationships.push(relationship.clone());
        self.media_bytes += bytes.len();
        self.media.push((name, bytes));
        Picture { relationship, size }
    }
    async fn picture(
        &mut self,
        images: &mut Images,
        renderer: &Renderer,
        uri: &str,
        width: Option<f64>,
    ) -> Result<Picture> {
        let picture = match self.pictures.get(uri) {
            Some(picture) => picture.clone(),
            None => {
                let (bytes, extension) = images.fetch(uri).await?;
                let (bytes, extension, size) = if extension == "svg" {
                    let image = images.svg_image(renderer, uri, &bytes)?;
                    let scale = image.max_scale().min(2.);
                    let png = image.png_region(Region::whole(&image, scale), None)?;
                    let (width, height) = image.dimensions();
                    (png, "png", (f64::from(width), f64::from(height)))
                } else {
                    if extension == "webp" {
                        images.check_decode(uri, &bytes)?;
                    }
                    word_image(bytes, extension)?
                };
                let picture = self.media(bytes, extension, size);
                self.pictures.insert(uri.to_owned(), picture.clone());
                picture
            }
        };
        // An HTML width keeps its proportions.
        Ok(match width {
            Some(width) if width > 0. => Picture {
                size: (width, picture.size.1 * width / picture.size.0.max(1.)),
                ..picture
            },
            _ => picture,
        })
    }
    fn drawing(
        &mut self,
        picture: &Picture,
        (cx, cy): (u64, u64),
        alt: &str,
        lowered: Option<i32>,
    ) {
        self.ensure();
        self.text_edge = TextEdge::Other;
        self.drawing_run(picture, (cx, cy), alt, lowered);
    }

    /// Emit an inline picture inside an already-open native paragraph.
    fn drawing_run(
        &mut self,
        picture: &Picture,
        (cx, cy): (u64, u64),
        alt: &str,
        lowered: Option<i32>,
    ) {
        self.drawings += 1;
        let id = self.drawings;
        self.body.push_str("<w:r>");
        if let Some(position) = lowered
            && position != 0
        {
            let _ = write!(
                self.body,
                "<w:rPr><w:position w:val=\"{position}\"/></w:rPr>"
            );
        }
        self.body
            .push_str("<w:drawing><wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">");
        let _ = write!(self.body, "<wp:extent cx=\"{cx}\" cy=\"{cy}\"/>");
        let _ = write!(
            self.body,
            "<wp:docPr id=\"{id}\" name=\"图片 {id}\" descr=\""
        );
        escape_into(&mut self.body, alt);
        let _ = write!(
            self.body,
            "\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{id}\" name=\"图片 {id}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"{}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{cx}\" cy=\"{cy}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>",
            picture.relationship
        );
    }

    /// Raw HTML has no Word equivalent; keep its text, images, breaks and basic emphasis.
    async fn html(&mut self, raw: &str, images: &mut Images, renderer: &Renderer) -> Result<()> {
        static TAG: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(
                r#"(?is)<!--.*?-->|<(/?)([a-z][a-z0-9]*)\b(?:[^>"']|"[^"]*"|'[^']*')*>"#,
            )
            .expect("valid pattern")
        });
        let mut end = 0;
        for tag in TAG.captures_iter(raw) {
            let whole = tag.get(0).expect("regex has a full match");
            self.html_text(&raw[end..whole.start()]);
            end = whole.end();
            let (Some(slash), Some(name)) = (tag.get(1), tag.get(2)) else {
                continue;
            };
            let closing = !slash.as_str().is_empty();
            let delta = |count: &mut u32| {
                *count = if closing {
                    count.saturating_sub(1)
                } else {
                    *count + 1
                };
            };
            match name.as_str().to_ascii_lowercase().as_str() {
                "img" => {
                    if let Some(image) = pecan_document::html_image(whole.as_str()) {
                        let width = image.width.map(f64::from);
                        let picture = self.picture(images, renderer, &image.uri, width).await?;
                        self.drawing(&picture, fit(picture.size, 1.), &image.alt, None);
                    }
                }
                "br" => self.line_break(),
                "hr" => self.rule(),
                "b" | "strong" => delta(&mut self.inline.bold),
                "i" | "em" => delta(&mut self.inline.italic),
                "s" | "del" | "strike" => delta(&mut self.inline.strike),
                "code" => {
                    if closing {
                        self.code_end();
                    } else {
                        self.code_start();
                    }
                }
                // Block tags end the paragraph, except inside a Markdown table cell.
                "p" | "div" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                | "blockquote" | "ul" | "ol" | "table" | "pre"
                    if self.table.is_none() =>
                {
                    self.end();
                }
                _ => {}
            }
        }
        self.html_text(&raw[end..]);
        Ok(())
    }
    fn html_text(&mut self, text: &str) {
        let text = decode_entities(text);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            self.text(&text, Style::Text);
        }
    }

    fn relationship(&mut self, kind: &str, target: &str, external: bool) -> String {
        // rId1 and rId2 are styles and numbering.
        self.next_relationship += 1;
        let id = format!("rId{}", self.next_relationship + 2);
        let _ = write!(
            self.relationships,
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\""
        );
        escape_into(&mut self.relationships, target);
        self.relationships.push('"');
        if external {
            self.relationships.push_str(" TargetMode=\"External\"");
        }
        self.relationships.push_str("/>");
        id
    }

    fn finish(mut self, snapshot: &Snapshot) -> Result<Vec<u8>> {
        self.end();
        while !self.notes.is_empty() {
            self.note_end();
        }
        if !self.body.ends_with("</w:p>") {
            self.body.push_str("<w:p/>");
        }
        let prefs = &snapshot.preferences;
        let fonts = word_fonts(&prefs.english_font, &prefs.chinese_font);
        let code_fonts = word_fonts(&prefs.code_font, SYSTEM_UI_FONT);
        let english = word_font(fonts.english_family());
        let chinese = word_font(fonts.chinese_family());
        let code_font = word_font(code_fonts.english_family());
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let deflate = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        // Images are already compressed.
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut part = |name: &str, contents: &[u8], options| -> Result<()> {
            zip.start_file(name, options)?;
            zip.write_all(contents)?;
            Ok(())
        };
        part("[Content_Types].xml", CONTENT_TYPES.as_bytes(), deflate)?;
        part("_rels/.rels", PACKAGE_RELATIONSHIPS.as_bytes(), deflate)?;
        let mut title = String::new();
        escape_into(
            &mut title,
            std::path::Path::new(&snapshot.title)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(&snapshot.title),
        );
        let mut icon_license = String::new();
        if !self.language_pictures.is_empty() {
            escape_into(&mut icon_license, &pecan_assets::language::license());
            icon_license = format!(
                "<dc:description>Embedded language marks: Devicon v2.18.0 and Lucide. Licenses apply only to these marks.\n{icon_license}</dc:description>"
            );
        }
        part(
            "docProps/core.xml",
            format!("{XML}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{title}</dc:title><dc:creator>Pecan</dc:creator>{icon_license}</cp:coreProperties>").as_bytes(),
            deflate,
        )?;
        part(
            "word/_rels/document.xml.rels",
            format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/><Relationship Id=\"rIdSettings\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/settings\" Target=\"settings.xml\"/>{}</Relationships>", self.relationships).as_bytes(),
            deflate,
        )?;
        let emphasis = self.cjk_bold.as_deref().unwrap_or(&chinese);
        part(
            "word/styles.xml",
            styles(&english, &chinese, emphasis, &code_font, prefs.line_height).as_bytes(),
            deflate,
        )?;
        part(
            "word/numbering.xml",
            numbering(&self.ordered).as_bytes(),
            deflate,
        )?;
        part("word/settings.xml", SETTINGS.as_bytes(), deflate)?;
        part(
            "word/document.xml",
            format!("{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\"><w:body>{}<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/><w:pgMar w:top=\"{MARGIN}\" w:right=\"{MARGIN}\" w:bottom=\"{MARGIN}\" w:left=\"{MARGIN}\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>", self.body, PAGE.0, PAGE.1).as_bytes(),
            deflate,
        )?;
        for (name, bytes) in &self.media {
            part(&format!("word/media/{name}"), bytes, stored)?;
        }
        Ok(zip.finish()?.into_inner())
    }
}

/// Fit logical pixels into the text column and one page height.
fn fit((width, height): (f64, f64), scale: f64) -> (u64, u64) {
    let (mut cx, mut cy) = (width * scale * EMU_PER_PX, height * scale * EMU_PER_PX);
    let shrink = (CONTENT_EMU / cx.max(1.))
        .min(MAX_FIGURE_EMU / cy.max(1.))
        .min(1.);
    cx *= shrink;
    cy *= shrink;
    (cx.round().max(1.) as u64, cy.round().max(1.) as u64)
}

/// Word reliably shows PNG, JPEG, GIF and BMP; WebP becomes PNG.
/// SVG is parsed with its resource budget and rasterized by the caller.
fn word_image(
    bytes: Vec<u8>,
    extension: &'static str,
) -> Result<(Vec<u8>, &'static str, (f64, f64))> {
    match extension {
        "webp" => {
            let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::WebP)?;
            let size = (f64::from(image.width()), f64::from(image.height()));
            let mut png = Vec::new();
            image.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
            Ok((png, "png", size))
        }
        _ => {
            let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
                .with_guessed_format()?
                .into_dimensions()
                .context("读取图片尺寸")?;
            Ok((bytes, extension, (f64::from(width), f64::from(height))))
        }
    }
}

fn word_fonts(english: &str, chinese: &str) -> DiagramFonts {
    DiagramFonts::new(
        if english.is_empty() {
            SYSTEM_MONO_FONT
        } else {
            english
        },
        // Word does not enumerate macOS MobileAsset UI fonts such as PingFang.
        // Resolve the abstract system choice to its installed Office-compatible
        // sans fallback; concrete user font choices remain unchanged.
        if cfg!(target_os = "macos") && (chinese.is_empty() || chinese == SYSTEM_UI_FONT) {
            "Hiragino Sans GB"
        } else if chinese.is_empty() {
            SYSTEM_UI_FONT
        } else {
            chinese
        },
    )
}

/// Families whose "bold" is a separate face. Word will not synthesize one from `w:b`.
fn cjk_emphasis_face(font: &str) -> Option<&'static str> {
    match font {
        "PingFang SC" => Some("PingFang SC Semibold"),
        "PingFang TC" => Some("PingFang TC Semibold"),
        "PingFang HK" => Some("PingFang HK Semibold"),
        _ => None,
    }
}

fn word_font(font: &str) -> String {
    let mut escaped = String::new();
    escape_into(&mut escaped, font);
    escaped
}

/// XML text and attribute escaping that also drops characters XML 1.0 forbids.
fn escape_into(output: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\t' | '\n' | '\r' => output.push(character),
            c if c < ' ' || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => output.push(c),
        }
    }
}

pub(super) fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        // Search ASCII bytes so the lookahead cannot split a CJK or emoji codepoint.
        let Some(end) = rest
            .as_bytes()
            .iter()
            .take(12)
            .position(|&byte| byte == b';')
        else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match decoded {
            Some(character) => {
                output.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                output.push('&');
                rest = &rest[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

const TABLE_BORDERS: &str = "<w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"DEDEDE\"/></w:tblBorders>";
const TABLE_MARGINS: &str = "<w:tblCellMar><w:top w:w=\"120\" w:type=\"dxa\"/><w:left w:w=\"140\" w:type=\"dxa\"/><w:bottom w:w=\"120\" w:type=\"dxa\"/><w:right w:w=\"140\" w:type=\"dxa\"/></w:tblCellMar>";

const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"png\" ContentType=\"image/png\"/><Default Extension=\"jpg\" ContentType=\"image/jpeg\"/><Default Extension=\"gif\" ContentType=\"image/gif\"/><Default Extension=\"bmp\" ContentType=\"image/bmp\"/><Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/><Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/><Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/><Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/><Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>";

/// Select the current Word layout engine instead of its implicit legacy mode.
/// This is a standard OOXML document setting on both macOS and Windows.
const SETTINGS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:settings xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>";

const PACKAGE_RELATIONSHIPS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/><Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>";

fn print_line(line_height: f64) -> u32 {
    (line_height.clamp(1.2, 1.5) * f64::from(BODY_HALF_POINTS) * 10.).round() as u32
}

fn styles(english: &str, chinese: &str, emphasis: &str, code: &str, line_height: f64) -> String {
    // Automatic line spacing multiplies CJK font metrics; use a print-sized minimum.
    let line = print_line(line_height);
    let note_line = line;
    // All cells share native leading and vertical centering.
    // Short note rows keep their complete native frame. Chaining keepNext
    // inside that cell duplicates the constraint and can delay an oversized
    // table by an entire blank page in compatible document readers.
    let mut styles = format!(
        "{XML}<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"{english}\" w:hAnsi=\"{english}\" w:eastAsia=\"{chinese}\" w:cs=\"{english}\"/><w:color w:val=\"242424\"/><w:sz w:val=\"{BODY_HALF_POINTS}\"/><w:szCs w:val=\"{BODY_HALF_POINTS}\"/><w:lang w:val=\"en-US\" w:eastAsia=\"zh-CN\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:snapToGrid w:val=\"0\"/><w:spacing w:after=\"120\" w:line=\"{line}\" w:lineRule=\"atLeast\"/><w:textAlignment w:val=\"baseline\"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/><w:rPr><w:rFonts w:ascii=\"{english}\" w:hAnsi=\"{english}\" w:eastAsia=\"{chinese}\" w:cs=\"{english}\"/></w:rPr></w:style>"
    );
    for (level, scale) in [2., 1.6, 1.3, 1.15, 1.05, 1.].into_iter().enumerate() {
        let size = (f64::from(BODY_HALF_POINTS) * scale).round() as u32;
        let before = if level == 0 { 240 } else { 360 };
        let _ = write!(
            styles,
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{before}\" w:after=\"140\" w:line=\"300\" w:lineRule=\"auto\"/><w:outlineLvl w:val=\"{level}\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"{english}\" w:hAnsi=\"{english}\" w:eastAsia=\"{emphasis}\" w:cs=\"{english}\" w:hint=\"eastAsia\"/><w:b/><w:bCs/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>",
            n = level + 1
        );
    }
    let _ = write!(
        styles,
        "<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:snapToGrid w:val=\"0\"/><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr><w:rPr><w:color w:val=\"747474\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"TableCell\"><w:name w:val=\"Table Cell\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:snapToGrid w:val=\"0\"/><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Highlight\"><w:name w:val=\"Highlight\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:keepLines/><w:spacing w:before=\"0\" w:after=\"80\" w:line=\"{note_line}\" w:lineRule=\"atLeast\"/></w:pPr><w:rPr><w:color w:val=\"242424\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"NoteTitle\"><w:name w:val=\"Note Title\"/><w:basedOn w:val=\"Highlight\"/><w:pPr><w:keepLines/><w:spacing w:before=\"0\" w:after=\"80\" w:line=\"300\" w:lineRule=\"atLeast\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepLines/><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"276\" w:lineRule=\"auto\"/><w:jc w:val=\"left\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"{code}\" w:hAnsi=\"{code}\" w:eastAsia=\"{chinese}\" w:cs=\"{code}\"/><w:sz w:val=\"{CODE_HALF_POINTS}\"/><w:szCs w:val=\"{CODE_HALF_POINTS}\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"CodeLanguage\"><w:name w:val=\"Code Language\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:keepNext/><w:keepLines/><w:snapToGrid w:val=\"0\"/><w:spacing w:before=\"0\" w:after=\"0\" w:line=\"{LANGUAGE_LINE}\" w:lineRule=\"atLeast\"/></w:pPr><w:rPr><w:color w:val=\"777777\"/><w:sz w:val=\"16\"/><w:szCs w:val=\"16\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"CodeChar\"><w:name w:val=\"Inline Code\"/><w:rPr><w:rFonts w:ascii=\"{code}\" w:hAnsi=\"{code}\" w:cs=\"{code}\"/>{INLINE_CODE_SHADING}</w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"CodePadding\"><w:name w:val=\"Inline Code Padding\"/><w:basedOn w:val=\"CodeChar\"/><w:rPr><w:rFonts w:ascii=\"{code}\" w:hAnsi=\"{code}\" w:eastAsia=\"{code}\" w:cs=\"{code}\" w:hint=\"default\"/><w:w w:val=\"40\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"CodeGap\"><w:name w:val=\"Italic Code Clearance\"/><w:rPr><w:rFonts w:ascii=\"{code}\" w:hAnsi=\"{code}\" w:eastAsia=\"{code}\" w:cs=\"{code}\" w:hint=\"default\"/><w:w w:val=\"30\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"365D8B\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"PecanTable\"><w:name w:val=\"Pecan Table\"/><w:tblPr>{TABLE_BORDERS}{TABLE_MARGINS}</w:tblPr><w:rPr><w:sz w:val=\"20\"/><w:szCs w:val=\"20\"/></w:rPr></w:style></w:styles>",
    );
    styles
}

fn numbering(ordered: &[(u32, u32)]) -> String {
    let mut numbering = format!(
        "{XML}<w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">"
    );
    for (id, decimal) in [(0, false), (1, true)] {
        let _ = write!(
            numbering,
            "<w:abstractNum w:abstractNumId=\"{id}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"
        );
        for level in 0..9u32 {
            let left = (level + 1) * INDENT;
            if decimal {
                let _ = write!(
                    numbering,
                    "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%{}.\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"{INDENT}\"/></w:pPr></w:lvl>",
                    level + 1
                );
            } else {
                let bullet = ["•", "◦", "▪"][level as usize % 3];
                let _ = write!(
                    numbering,
                    "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"{bullet}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"{INDENT}\"/></w:pPr></w:lvl>"
                );
            }
        }
        numbering.push_str("</w:abstractNum>");
    }
    numbering.push_str("<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>");
    for (index, (start, level)) in ordered.iter().enumerate() {
        let _ = write!(
            numbering,
            "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/><w:lvlOverride w:ilvl=\"{level}\"><w:startOverride w:val=\"{start}\"/></w:lvlOverride></w:num>",
            index + 2
        );
    }
    numbering.push_str("</w:numbering>");
    numbering
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::options;
    use pecan_config::PreferencesData;
    use pecan_document::Document;
    use pecan_math::RenderQueue as MathQueue;
    use pecan_mermaid::RenderQueue;
    use pulldown_cmark::Parser;
    use std::io::Read as _;

    type Package = zip::ZipArchive<Cursor<Vec<u8>>>;

    /// `source` with default preferences, resolving relative paths against
    /// the system temporary directory.
    fn fixture(source: &str) -> Snapshot {
        Snapshot {
            document: Arc::new(Document::parse(source.to_owned()).unwrap()),
            directory: std::env::temp_dir().into(),
            title: "fixture.md".into(),
            preferences: Arc::new(PreferencesData::default()),
            diagrams: RenderQueue::new(),
            math: MathQueue::new(),
            codes: pecan_format::CodeSnapshot::default(),
            resources: None,
        }
    }

    fn package(bytes: Vec<u8>) -> Package {
        zip::ZipArchive::new(Cursor::new(bytes)).unwrap()
    }

    /// A complete export of `snapshot`.
    fn export(snapshot: &Snapshot) -> Package {
        package(futures_lite::future::block_on(docx(snapshot, crate::network::client())).unwrap())
    }

    fn read_part<R: std::io::Read + std::io::Seek>(
        package: &mut zip::ZipArchive<R>,
        name: &str,
    ) -> String {
        let mut xml = String::new();
        package
            .by_name(name)
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        xml
    }

    /// `word/document.xml` of a complete export with default preferences.
    fn document_xml(source: &str) -> String {
        read_part(&mut export(&fixture(source)), "word/document.xml")
    }

    #[test]
    fn oversized_note_rows_allow_native_pagination_without_splitting_short_notes() {
        let xml = document_xml(include_str!("../../../samples/callout-pagination.md"));
        let document = roxmltree::Document::parse(&xml).unwrap();
        for (label, keep) in [("短块应保持完整", true), ("长块允许分页", false)] {
            let row = document
                .descendants()
                .find(|n| n.is_text() && n.text() == Some(label))
                .unwrap()
                .ancestors()
                .find(|n| n.has_tag_name("tr"))
                .unwrap();
            assert_eq!(
                row.children()
                    .filter(|n| n.has_tag_name("trPr"))
                    .any(|n| n.children().any(|n| n.has_tag_name("cantSplit"))),
                keep,
                "native pagination policy for {label}",
            );
        }
        assert!(xml.contains("跨页末尾仍可见"));
        assert!(xml.contains("TOML"));
    }

    #[test]
    fn native_properties_follow_ooxml_order_in_all_containers() {
        futures_lite::future::block_on(async {
            let source = include_str!("../../../samples/callout-alignment.md");
            let snapshot = fixture(source);
            let bytes = docx(&snapshot, crate::network::client()).await.unwrap();
            let mut archive = package(bytes);
            let orders = [
                (
                    "rPr",
                    "rStyle rFonts b bCs i iCs caps smallCaps strike dstrike outline shadow emboss imprint noProof snapToGrid vanish webHidden color spacing w kern position sz szCs highlight u effect bdr shd fitText vertAlign rtl cs em lang eastAsianLayout specVanish oMath rPrChange",
                ),
                (
                    "pPr",
                    "pStyle keepNext keepLines pageBreakBefore framePr widowControl numPr suppressLineNumbers pBdr shd tabs suppressAutoHyphens kinsoku wordWrap overflowPunct topLinePunct autoSpaceDE autoSpaceDN bidi adjustRightInd snapToGrid spacing ind contextualSpacing mirrorInd suppressOverlap jc textDirection textAlignment textboxTightWrap outlineLvl divId cnfStyle rPr sectPr pPrChange",
                ),
                (
                    "tcPr",
                    "cnfStyle tcW gridSpan hMerge vMerge tcBorders shd noWrap tcMar textDirection tcFitText vAlign hideMark headers cellIns cellDel cellMerge tcPrChange",
                ),
                (
                    "tblPr",
                    "tblStyle tblpPr tblOverlap bidiVisual tblStyleRowBandSize tblStyleColBandSize tblW jc tblCellSpacing tblInd tblBorders shd tblLayout tblCellMar tblLook tblCaption tblDescription tblPrChange",
                ),
                (
                    "trPr",
                    "cnfStyle divId gridBefore gridAfter wBefore wAfter cantSplit trHeight tblHeader tblCellSpacing jc hidden ins del trPrChange",
                ),
            ];
            for part in ["word/document.xml", "word/styles.xml", "word/numbering.xml"] {
                let xml = read_part(&mut archive, part);
                let document = roxmltree::Document::parse(&xml).unwrap();
                if part == "word/document.xml" {
                    let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
                    let leads: Vec<_> = document
                        .descendants()
                        .filter(|n| {
                            n.has_tag_name("p")
                                && n.descendants().any(|c| c.has_tag_name("inline"))
                                && n.descendants().any(|c| {
                                    c.has_tag_name("tab") && c.attribute((w, "pos")).is_some()
                                })
                        })
                        .collect();
                    assert_eq!(
                        leads.len(),
                        10,
                        "every generated note has one gutter symbol"
                    );
                    for lead in leads {
                        assert!(!lead.descendants().any(|n| n.has_tag_name("anchor")));
                        for position in lead.descendants().filter(|n| n.has_tag_name("position")) {
                            let run = position.ancestors().find(|n| n.has_tag_name("r")).unwrap();
                            assert!(run.descendants().any(|n| n.has_tag_name("drawing")));
                            assert!(
                                !run.descendants().any(|n| n.has_tag_name("t")),
                                "only the inline icon shifts; text retains Word's baseline"
                            );
                        }
                        let stop = lead
                            .descendants()
                            .find(|n| n.has_tag_name("tab") && n.attribute((w, "pos")).is_some())
                            .unwrap();
                        let indent = lead.descendants().find(|n| n.has_tag_name("ind")).unwrap();
                        assert_eq!(stop.attribute((w, "pos")), indent.attribute((w, "left")));
                        assert_eq!(
                            indent.attribute((w, "left")),
                            indent.attribute((w, "hanging"))
                        );
                        let row = lead.ancestors().find(|n| n.has_tag_name("tr")).unwrap();
                        assert!(
                            row.children()
                                .filter(|n| n.has_tag_name("trPr"))
                                .any(|n| n.children().any(|n| n.has_tag_name("cantSplit")))
                        );
                    }
                }
                for (name, order) in orders {
                    let order: Vec<_> = order.split_whitespace().collect();
                    for node in document.descendants().filter(|n| n.has_tag_name(name)) {
                        let names: Vec<_> = node
                            .children()
                            .filter(|n| n.is_element())
                            .map(|n| n.tag_name().name())
                            .collect();
                        let indices: Vec<_> = names
                            .iter()
                            .map(|name| {
                                order
                                    .iter()
                                    .position(|key| key == name)
                                    .unwrap_or_else(|| panic!("unknown {name} in {part}"))
                            })
                            .collect();
                        assert!(
                            indices.windows(2).all(|pair| pair[0] < pair[1]),
                            "non-canonical {name} in {part}: {names:?}"
                        );
                    }
                }
            }
            let settings = read_part(&mut archive, "word/settings.xml");
            let settings = roxmltree::Document::parse(&settings).unwrap();
            let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
            assert!(
                settings
                    .descendants()
                    .any(|n| n.attribute((w, "name")) == Some("compatibilityMode")
                        && n.attribute((w, "val")) == Some("15"))
            );
            for (part, target, attribute) in [
                ("word/_rels/document.xml.rels", "settings.xml", "Target"),
                ("[Content_Types].xml", "/word/settings.xml", "PartName"),
            ] {
                let xml = read_part(&mut archive, part);
                let document = roxmltree::Document::parse(&xml).unwrap();
                assert!(
                    document
                        .descendants()
                        .any(|n| n.attribute(attribute) == Some(target)),
                    "modern layout settings must be discoverable through OPC parts"
                );
            }
        });
    }

    #[test]
    fn code_shading_is_explicit_on_each_native_run_in_notes() {
        let mut out = Docx::default();
        out.frame_start(FrameStyle::Note(Some(AlertKind::Tip)));
        out.highlight = true;
        out.code_span("needAddress=false");
        out.frame_end();
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let cell = document
            .descendants()
            .find(|n| n.has_tag_name("tc"))
            .unwrap();
        assert!(cell.descendants().any(|n| n.has_tag_name("pStyle")
            && n.attribute((
                "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
                "val"
            )) == Some("Highlight")));
        let runs: Vec<_> = cell.descendants().filter(|n| n.has_tag_name("r")).collect();
        assert_eq!(runs.len(), 3, "two padding runs surround editable code");
        for run in runs {
            assert!(
                run.descendants().any(|n| n.has_tag_name("shd")),
                "shading must include horizontal padding"
            );
            assert!(
                !run.descendants().any(|n| n.has_tag_name("bdr")),
                "shading alone draws the capsule in shaded cells too"
            );
        }
    }

    #[test]
    fn inline_code_is_drawn_by_shading_alone() {
        // Word paints run shading over the whole line box but draws a character
        // border around the bordered runs' own font box. The two only coincide
        // when the code font is the tallest on its line, so a capsule drawn by
        // both shows gray outside its frame beside CJK text, larger text, or a
        // formula. One painter cannot disagree with itself.
        let source = concat!(
            include_str!("../../../samples/word-inline-borders.md"),
            "\n### 标题 `code`\n\n*斜体*`code`，公式 $x^2$ 与 `代码` 同行。\n",
        );
        let mut package = export(&fixture(source));
        let document = read_part(&mut package, "word/document.xml");
        assert!(document.matches("<w:rStyle w:val=\"CodeChar\"/>").count() >= 30);
        for part in [document, read_part(&mut package, "word/styles.xml")] {
            assert!(
                !part.contains("<w:bdr"),
                "no run may carry a character border"
            );
        }
    }

    #[test]
    fn formatted_code_and_highlighting_are_written_as_editable_word_paragraphs() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            for width in [2, 4, 8] {
                let snapshot = Snapshot {
                    directory: directory.path().into(),
                    preferences: Arc::new(PreferencesData {
                        code_tab_size: width.to_string(),
                        ..Default::default()
                    }),
                    ..fixture(super::super::FORMAT_FIXTURE)
                };
                let bytes = docx(&snapshot, crate::network::client()).await.unwrap();
                let xml = read_part(&mut package(bytes), "word/document.xml");
                let doc = roxmltree::Document::parse(&xml).unwrap();
                let ns = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
                let lines: Vec<_> = doc
                    .descendants()
                    .filter(|n| {
                        n.has_tag_name((ns, "p"))
                            && n.descendants().any(|n| {
                                n.has_tag_name((ns, "pStyle"))
                                    && n.attribute((ns, "val")) == Some("Code")
                            })
                    })
                    .map(|p| {
                        p.descendants()
                            .filter(|n| n.has_tag_name((ns, "t")))
                            .filter_map(|n| n.text())
                            .collect::<String>()
                    })
                    .collect();
                assert_eq!(
                    lines.join("\n"),
                    super::super::expected_code(width).join("\n")
                );
                assert!(xml.contains("w:val=\"8250DF\""), "TSX keywords are colored");
                assert!(xml.contains("w:val=\"0A6440\""), "strings are colored");
                assert_eq!(snapshot.document.source(), super::super::FORMAT_FIXTURE);
            }
        });
    }

    #[test]
    fn language_logos_precede_native_labels_and_repeated_aliases_share_media() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = Snapshot {
            directory: directory.path().into(),
            ..fixture(
                "```javascript\nlet a = 1;\n```\n\n```JS\nlet b = 2;\n```\n\n```unknown\nkeep me\n```",
            )
        };
        let mut archive = export(&snapshot);
        let media = (0..archive.len())
            .filter(|&index| {
                archive
                    .by_index(index)
                    .unwrap()
                    .name()
                    .unwrap()
                    .starts_with("word/media/")
            })
            .count();
        assert_eq!(
            media, 2,
            "aliases and unknown languages share their respective media"
        );
        let style_xml = read_part(&mut archive, "word/styles.xml");
        let styles = roxmltree::Document::parse(&style_xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let label_style = styles
            .descendants()
            .find(|n| {
                n.has_tag_name((w, "style")) && n.attribute((w, "styleId")) == Some("CodeLanguage")
            })
            .unwrap();
        let spacing = label_style
            .descendants()
            .find(|n| n.has_tag_name((w, "spacing")))
            .unwrap();
        assert_eq!(spacing.attribute((w, "lineRule")), Some("atLeast"));
        assert!(
            spacing
                .attribute((w, "line"))
                .unwrap()
                .parse::<u32>()
                .unwrap()
                == 240,
            "Word must reserve a compact 12 pt line for its 12 pt inline logo and baseline"
        );
        let xml = read_part(&mut archive, "word/document.xml");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let headers: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name((w, "p"))
                    && node.children().any(|pr| {
                        pr.has_tag_name((w, "pPr"))
                            && pr
                                .children()
                                .any(|style| style.attribute((w, "val")) == Some("CodeLanguage"))
                    })
            })
            .collect();
        assert_eq!(headers.len(), 3);
        for header in &headers {
            let cell = header
                .ancestors()
                .find(|n| n.has_tag_name((w, "tc")))
                .unwrap();
            let row = cell.parent().unwrap();
            let height = row
                .descendants()
                .find(|n| n.has_tag_name((w, "trHeight")))
                .unwrap();
            assert_eq!(
                height.attribute((w, "hRule")),
                Some("atLeast"),
                "header rows must grow rather than crop their inline picture"
            );
            assert!(
                height
                    .attribute((w, "val"))
                    .unwrap()
                    .parse::<u32>()
                    .unwrap()
                    == 480
            );
            let margins = cell
                .descendants()
                .find(|n| n.has_tag_name((w, "tcMar")))
                .unwrap();
            for side in ["top", "bottom"] {
                let margin = margins
                    .children()
                    .find(|n| n.has_tag_name((w, side)))
                    .unwrap();
                assert!(margin.attribute((w, "w")).unwrap().parse::<u32>().unwrap() == 0);
            }
            let drawing = header
                .descendants()
                .find(|node| node.has_tag_name((w, "drawing")));
            let text = header
                .descendants()
                .rfind(|node| node.has_tag_name((w, "t")))
                .unwrap();
            assert!(
                drawing.unwrap().range().start < text.range().start,
                "dedicated or generic logo precedes the label"
            );
            assert!(
                !header.children().any(|node| node.has_tag_name((w, "p"))),
                "inline marks do not create nested paragraphs"
            );
        }
        assert!(xml.contains("keep me"), "source remains native text");
    }

    #[test]
    fn table_cells_keep_native_lists_and_inline_markdown() {
        let source = "| 项目 | 内容 |\n| --- | --- |\n| 无序 | * **加粗**<br><br>* *斜体*与~~删除~~ |\n| 有序 | 3. `code`<br>4. [链接](https://example.com) |\n";
        let mut out = Docx::default();
        for event in crate::table::events(source) {
            out.event(event);
        }
        assert_eq!(out.body.matches("<w:numPr>").count(), 4);
        assert_eq!(out.ordered, vec![(3, 0)]);
        assert!(out.body.contains("<w:b/>"));
        assert!(out.body.contains("<w:i/>"));
        assert!(out.body.contains("<w:strike/>"));
        assert!(out.body.contains("<w:rStyle w:val=\"CodeChar\"/>"));
        assert!(out.body.contains("<w:hyperlink"));
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        for paragraph in document.descendants().filter(|n| {
            n.has_tag_name((w, "p")) && n.ancestors().any(|a| a.has_tag_name((w, "tc")))
        }) {
            let properties = paragraph
                .children()
                .find(|n| n.has_tag_name((w, "pPr")))
                .unwrap();
            assert!(properties.children().any(|n| {
                n.has_tag_name((w, "pStyle")) && n.attribute((w, "val")) == Some("TableCell")
            }));
            assert!(
                !properties
                    .children()
                    .any(|n| n.has_tag_name((w, "spacing"))),
                "plain and decorated cells share native leading without capsule margins"
            );
        }
    }

    #[test]
    fn tables_keep_opaque_cells_inside_notes_and_quotes() {
        let source = "| 标题 | 内容 |\n| --- | --- |\n| 列表 | - **加粗**<br>- *斜体* |\n";
        let mut out = Docx::default();
        for event in crate::table::events(source) {
            out.event(event);
        }
        for style in [
            FrameStyle::Quote,
            FrameStyle::Note(None),
            FrameStyle::Note(Some(AlertKind::Note)),
        ] {
            out.frame_start(style);
            for event in crate::table::events(source) {
                out.event(event);
            }
            out.frame_end();
        }
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let tables: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name((w, "tbl"))
                    && node.children().any(|pr| {
                        pr.has_tag_name((w, "tblPr"))
                            && pr.children().any(|style| {
                                style.has_tag_name((w, "tblStyle"))
                                    && style.attribute((w, "val")) == Some("PecanTable")
                            })
                    })
            })
            .collect();
        assert_eq!(tables.len(), 4);
        for table in tables {
            for (index, row) in table
                .children()
                .filter(|node| node.has_tag_name((w, "tr")))
                .enumerate()
            {
                for cell in row.children().filter(|node| node.has_tag_name((w, "tc"))) {
                    let fill = cell
                        .children()
                        .find(|node| node.has_tag_name((w, "tcPr")))
                        .unwrap()
                        .children()
                        .find(|node| node.has_tag_name((w, "shd")))
                        .expect("table cells must not inherit the note or quote background");
                    assert_eq!(fill.attribute((w, "val")), Some("clear"));
                    assert_eq!(
                        fill.attribute((w, "fill")),
                        Some(if index == 0 { "F5F5F5" } else { "FFFFFF" })
                    );
                }
            }
        }
        assert_eq!(out.body.matches("<w:numPr>").count(), 8);
        assert!(out.frames.is_empty() && out.table.is_none());
    }

    #[test]
    fn cells_ending_in_a_nested_table_keep_a_final_paragraph_word_will_lay_out() {
        // Each container ends with a different nested table: code card, data
        // table, note, and a quote inside a data cell.
        let xml = document_xml(
            ":::\n正文\n\n```json\n{\"a\": 1}\n```\n:::\n\n\
             :::\n正文\n\n| 甲 | 乙 |\n| --- | --- |\n| 1 | 2 |\n:::\n\n\
             :::\n外层\n\n> [!TIP] 内层\n> 正文\n:::\n\n\
             > 引用\n>\n> ```toml\n> a = 1\n> ```\n\n\
             | 内容 |\n| --- |\n| 说明<br><br>> 单元格内引用 |\n\n\
             :::\n```text\nmiddle\n```\n\n代码之后的正文。\n:::\n",
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let mut ends = 0;
        for cell in document.descendants().filter(|n| n.has_tag_name((w, "tc"))) {
            let last = cell.last_element_child().unwrap();
            assert!(last.has_tag_name((w, "p")), "a cell ends with a paragraph");
            if !last
                .prev_sibling_element()
                .is_some_and(|previous| previous.has_tag_name((w, "tbl")))
            {
                continue;
            }
            ends += 1;
            // Word drops an empty paragraph here and loses the table's bottom border.
            assert_eq!(
                last.descendants()
                    .find(|n| n.has_tag_name((w, "t")))
                    .and_then(|text| text.text()),
                Some("\u{a0}"),
                "the paragraph after a cell's final nested table must not be empty"
            );
            let spacing = last
                .descendants()
                .find(|n| n.has_tag_name((w, "spacing")))
                .unwrap();
            for (name, value) in [("line", "60"), ("lineRule", "exact"), ("after", "0")] {
                assert_eq!(spacing.attribute((w, name)), Some(value), "{name}");
            }
            assert!(
                last.descendants()
                    .filter(|n| n.has_tag_name((w, "sz")))
                    .all(|size| size.attribute((w, "val")) == Some("6")),
                "the mark and its anchor fit the 3 pt line"
            );
        }
        assert_eq!(ends, 5, "every container above ends with a nested table");
        // A table followed by more content keeps its ordinary empty separator.
        let middle = document
            .descendants()
            .find(|n| n.has_tag_name((w, "t")) && n.text() == Some("middle"))
            .unwrap()
            .ancestors()
            .find(|n| n.has_tag_name((w, "tbl")))
            .unwrap()
            .next_sibling_element()
            .unwrap();
        assert!(middle.has_tag_name((w, "p")));
        assert!(!middle.descendants().any(|n| n.has_tag_name((w, "t"))));
    }

    #[test]
    fn code_cards_share_one_native_frame_in_every_container() {
        let mut out = Docx::default();
        let icon = Picture {
            relationship: "rId1".into(),
            size: (16., 16.),
        };
        out.code_block("before", "text", None);
        out.frame_start(FrameStyle::Quote);
        out.code_block("quoted", "text", None);
        out.frame_end();
        out.note_start(icon.clone(), None, false);
        out.code_block("highlight", "text", None);
        out.note_start(icon, Some(AlertKind::Tip), true);
        out.code_block("nested", "text", None);
        out.note_end();
        out.code_block("restored", "text", None);
        out.frame_start(FrameStyle::Quote);
        out.code_block("quoted note", "text", None);
        out.frame_end();
        out.note_end();
        out.code_block("after", "text", None);
        assert!(!out.highlight && out.frames.is_empty() && out.notes.is_empty());

        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let cards: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name((w, "tbl"))
                    && node
                        .children()
                        .filter(|row| row.has_tag_name((w, "tr")))
                        .count()
                        == 2
            })
            .collect();
        assert_eq!(cards.len(), 7);
        for (card, text) in cards.iter().zip([
            "before",
            "quoted",
            "highlight",
            "nested",
            "restored",
            "quoted note",
            "after",
        ]) {
            assert!(
                card.descendants()
                    .any(|node| node.has_tag_name((w, "t")) && node.text() == Some(text)),
                "code source remains editable in {text}"
            );
            // Cell spacing would switch Word to separate borders: the table and
            // both cells each draw a frame, and the doubled lines differ by zoom.
            assert!(
                !card
                    .descendants()
                    .any(|node| node.has_tag_name((w, "tblCellSpacing"))),
                "{text}: one shared frame"
            );
            let borders = card
                .children()
                .find(|node| node.has_tag_name((w, "tblPr")))
                .unwrap()
                .children()
                .find(|node| node.has_tag_name((w, "tblBorders")))
                .unwrap();
            assert_eq!(
                &xml[borders.range()],
                CODE_BORDERS,
                "{text}: the frame does not depend on the container"
            );
            let after = card.next_sibling_element().unwrap();
            assert!(
                after.has_tag_name((w, "p")),
                "{text}: Word requires a paragraph"
            );
        }
    }

    #[test]
    fn code_cards_have_two_padded_shaded_rows_and_preserve_source() {
        let mut out = Docx::default();
        for (source, language) in [
            ("fn main() {\n    let value = 42;\n}", "rust"),
            ("plain\ntext", ""),
        ] {
            out.code_block(source, language, None);
        }
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let tables: Vec<_> = document
            .descendants()
            .filter(|n| n.has_tag_name((w, "tbl")))
            .collect();
        assert_eq!(tables.len(), 2, "each code block is a native Word card");
        for table in tables {
            let borders = table
                .children()
                .find(|n| n.has_tag_name((w, "tblPr")))
                .unwrap()
                .children()
                .find(|n| n.has_tag_name((w, "tblBorders")))
                .unwrap();
            for side in ["top", "left", "bottom", "right", "insideH", "insideV"] {
                let border = borders
                    .children()
                    .find(|n| n.has_tag_name((w, side)))
                    .unwrap();
                assert_eq!(
                    border.attribute((w, "val")),
                    Some(if side == "insideV" { "nil" } else { "single" })
                );
                if side != "insideV" {
                    assert_eq!(border.attribute((w, "sz")), Some("4"));
                }
            }
            let spacing = table
                .descendants()
                .find(|n| n.has_tag_name((w, "tblCellSpacing")));
            assert!(spacing.is_none(), "collapsed borders draw each edge once");
            let rows: Vec<_> = table
                .children()
                .filter(|n| n.has_tag_name((w, "tr")))
                .collect();
            assert_eq!(rows.len(), 2);
            assert!(
                !table
                    .descendants()
                    .any(|n| n.has_tag_name((w, "tcBorders"))),
                "cell edges must not compete with the shared table frame"
            );
            for (row, fill) in rows.iter().zip(["F4F4F4", "FCFCFC"]) {
                let shading = row
                    .descendants()
                    .find(|n| n.has_tag_name((w, "shd")))
                    .unwrap();
                assert_eq!(shading.attribute((w, "fill")), Some(fill));
            }
            assert!(
                rows[0]
                    .descendants()
                    .any(|n| n.has_tag_name((w, "tblHeader")))
            );
            let alignment = rows[0]
                .descendants()
                .find(|n| n.has_tag_name((w, "jc")))
                .unwrap();
            assert_eq!(alignment.attribute((w, "val")), Some("right"));

            assert!(
                rows[0]
                    .descendants()
                    .any(|n| n.has_tag_name((w, "keepNext")))
            );
            assert!(
                !rows[1]
                    .descendants()
                    .any(|n| n.has_tag_name((w, "cantSplit")))
            );
            for side in ["top", "left", "bottom", "right"] {
                let margin = table
                    .descendants()
                    .find(|n| n.has_tag_name((w, "tblCellMar")))
                    .unwrap()
                    .children()
                    .find(|n| n.has_tag_name((w, side)))
                    .unwrap();
                assert!(margin.attribute((w, "w")).unwrap().parse::<u32>().unwrap() > 0);
            }
        }
        assert!(out.body.contains("value"));
        assert!(out.body.contains("RUST") && out.body.contains("TEXT"));
    }

    #[test]
    fn paragraphs_after_tables_fit_their_exact_line_and_body_cells_stay_centered() {
        let xml = document_xml(include_str!("../../../samples/word-inline-borders.md"));
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let mut exact = 0;
        for gap in document.descendants().filter(|n| {
            n.has_tag_name((w, "p"))
                && n.prev_sibling_element()
                    .is_some_and(|table| table.has_tag_name((w, "tbl")))
        }) {
            let spacing = gap
                .descendants()
                .find(|n| n.has_tag_name((w, "spacing")))
                .unwrap();
            // The separator between a data table and more content keeps the body line.
            let Some(line) = spacing.attribute((w, "line")) else {
                continue;
            };
            assert_eq!(spacing.attribute((w, "lineRule")), Some("exact"));
            let line: u32 = line.parse().unwrap();
            let sizes: Vec<u32> = gap
                .descendants()
                .filter(|n| n.has_tag_name((w, "sz")) || n.has_tag_name((w, "szCs")))
                .map(|n| n.attribute((w, "val")).unwrap().parse().unwrap())
                .collect();
            assert!(sizes.len() >= 2, "the paragraph mark is sized explicitly");
            // Half-points against twentieths of a point: a larger mark would
            // be painted over the thin frame above it.
            assert!(sizes.iter().all(|size| size * 10 <= line));
            exact += 1;
        }
        // Three frames, and the code card or table that ends each of them.
        assert_eq!(exact, 6);
        for cell in document.descendants().filter(|n| {
            n.has_tag_name((w, "tcPr"))
                && n.children().any(|s| {
                    s.has_tag_name((w, "shd")) && s.attribute((w, "fill")) == Some("FFFFFF")
                })
        }) {
            assert_eq!(
                cell.children()
                    .find(|n| n.has_tag_name((w, "vAlign")))
                    .unwrap()
                    .attribute((w, "val")),
                Some("center")
            );
        }
    }

    #[test]
    fn inline_code_has_only_shaded_horizontal_edges_and_preserves_source_runs() {
        let source = "# `⌘O`\n\n按 `⌘⇧O` 打开；`设备状态` 和 `i`。\n\n> `request_id`\n\n| 字段 |\n| --- |\n| `GET /api` |\n\n<code>left<strong>middle</strong>right</code>\n";
        let directory = tempfile::tempdir().unwrap();
        let snapshot = Snapshot {
            directory: directory.path().into(),
            ..fixture(source)
        };
        let xml = read_part(&mut export(&snapshot), "word/document.xml");
        assert_eq!(snapshot.document.source(), source);
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let code_runs: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name("r")
                    && node.descendants().any(|style| {
                        style.has_tag_name("rStyle")
                            && matches!(
                                style.attribute((w, "val")),
                                Some("CodeChar" | "CodePadding")
                            )
                    })
            })
            .collect();
        let text: Vec<_> = code_runs
            .iter()
            .filter_map(|run| {
                run.descendants()
                    .find(|node| node.has_tag_name("t"))
                    .and_then(|node| node.text())
            })
            .collect();
        assert_eq!(
            text.iter()
                .filter(|value| **value == INLINE_CODE_EDGE)
                .count(),
            14
        );
        for code in ["⌘O", "⌘⇧O", "设备状态", "i", "request_id", "GET /api"] {
            let at = text
                .iter()
                .position(|value| *value == code)
                .expect("unmodified editable code run");
            assert_eq!(text[at - 1], INLINE_CODE_EDGE);
            assert_eq!(text[at + 1], INLINE_CODE_EDGE);
        }
        assert!(
            text.windows(5).any(|runs| runs
                == [
                    INLINE_CODE_EDGE,
                    "left",
                    "middle",
                    "right",
                    INLINE_CODE_EDGE
                ]),
            "HTML emphasis must not insert extra padding inside a code span"
        );
        assert!(
            !code_runs
                .iter()
                .any(|run| run.descendants().any(|node| node.has_tag_name("drawing"))),
            "capsules remain native editable text rather than images"
        );
        for run in &code_runs {
            assert!(
                !run.descendants().any(|node| node.has_tag_name((w, "bdr"))),
                "a character border follows other bounds than the shading"
            );
            let fill = run
                .descendants()
                .find(|node| node.has_tag_name((w, "shd")))
                .expect("text and both horizontal insets retain explicit gray shading");
            assert_eq!(fill.attribute((w, "fill")), Some("E8E8E8"));
        }
    }

    #[test]
    fn italic_clearance_only_separates_adjacent_ink_from_a_capsule() {
        for (source, expected) in [
            ("plain`code`plain", 0),
            ("*italic*`code`*italic*", 2),
            ("*italic* `code` *italic*", 0),
            ("*italic*\n\n`code`", 0),
            ("*italic*  \n`code`", 0),
            ("`*italic*``code`", 0),
            ("`code`**bold**", 0),
        ] {
            let mut out = Docx::default();
            for event in Parser::new_ext(source, options()) {
                out.event(event);
            }
            assert_eq!(
                out.body.matches("<w:rStyle w:val=\"CodeGap\"/>").count(),
                expected,
                "clearance for {source:?}",
            );
        }
        let mut out = Docx::default();
        out.begin(Kind::Body);
        out.inline.italic = 1;
        out.text("italic", Style::Text);
        out.inline.italic = 0;
        out.code_start(); // The same entry/exit used for HTML <code>.
        out.text("left", Style::Text);
        out.code_start();
        out.text("nested", Style::Text);
        out.code_end();
        out.code_end();
        out.inline.italic = 1;
        out.text("italic", Style::Text);
        out.end();
        assert_eq!(out.body.matches("<w:rStyle w:val=\"CodeGap\"/>").count(), 2);
        assert_eq!(
            out.body
                .matches("<w:rStyle w:val=\"CodePadding\"/>")
                .count(),
            2
        );
        assert!(out.body.contains("nested"));
        let xml = styles("Menlo", "PingFang SC", "PingFang SC Semibold", "Menlo", 1.6);
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let gap = document
            .descendants()
            .find(|n| n.attribute((w, "styleId")) == Some("CodeGap"))
            .unwrap();
        assert!(
            !gap.descendants()
                .any(|n| matches!(n.tag_name().name(), "bdr" | "shd" | "basedOn"))
        );
    }

    #[test]
    fn data_tables_reserve_cell_space_without_paragraph_margins() {
        let mut out = Docx::default();
        for event in crate::table::events(
            "| Header | `code` |\n| - | - |\n| multiline<br>text | *italic*`code` |",
        ) {
            out.event(event);
        }
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let rows: Vec<_> = document
            .descendants()
            .filter(|n| n.has_tag_name((w, "tr")))
            .collect();
        assert_eq!(rows.len(), 2);
        for row in rows {
            let height = row
                .descendants()
                .find(|n| n.has_tag_name((w, "trHeight")))
                .unwrap();
            assert_eq!(height.attribute((w, "hRule")), Some("atLeast"));
            assert_eq!(height.attribute((w, "val")), Some("480"));
        }
        for cell in document
            .descendants()
            .filter(|n| n.has_tag_name((w, "tcPr")))
        {
            let align = cell
                .children()
                .find(|n| n.has_tag_name((w, "vAlign")))
                .unwrap();
            assert_eq!(align.attribute((w, "val")), Some("center"));
        }
        let margins = document
            .descendants()
            .find(|n| n.has_tag_name((w, "tblCellMar")))
            .unwrap();
        for side in ["top", "bottom"] {
            assert_eq!(
                margins
                    .children()
                    .find(|n| n.has_tag_name((w, side)))
                    .unwrap()
                    .attribute((w, "w")),
                Some("120")
            );
        }
        assert!(document.descendants().any(|n| n.has_tag_name((w, "br"))));
        assert!(
            !document
                .descendants()
                .any(|n| n.has_tag_name((w, "spacing"))
                    && n.ancestors().any(|parent| parent.has_tag_name((w, "tc"))))
        );
    }

    #[test]
    fn word_svg_images_render_normally_and_check_their_embedded_raster_budget() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let mut webp = Vec::new();
            image::DynamicImage::new_rgb8(3, 2)
                .write_to(&mut Cursor::new(&mut webp), image::ImageFormat::WebP)
                .unwrap();
            let svg = |bytes: &[u8]| {
                format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"24\"><rect width=\"32\" height=\"24\" fill=\"white\"/><image href=\"data:image/webp;base64,{}\" width=\"32\" height=\"24\"/></svg>",
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
                )
            };
            std::fs::write(directory.path().join("small.svg"), svg(&webp)).unwrap();
            let renderer = Renderer::default();
            let mut images = Images::new(directory.path(), crate::network::client());
            let mut out = Docx::default();
            let picture = out
                .picture(&mut images, &renderer, "small.svg", None)
                .await
                .unwrap();
            assert_eq!(picture.size, (32., 24.));
            let dimensions = image::ImageReader::new(Cursor::new(&out.media[0].1))
                .with_guessed_format()
                .unwrap()
                .into_dimensions()
                .unwrap();
            assert_eq!(
                dimensions,
                (64, 48),
                "normal SVG retains print-resolution rendering"
            );

            let lossless = webp.windows(4).position(|chunk| chunk == b"VP8L").unwrap();
            let dimensions = (8193_u32 - 1) | ((4096_u32 - 1) << 14);
            webp[lossless + 9..lossless + 13].copy_from_slice(&dimensions.to_le_bytes());
            std::fs::write(directory.path().join("oversized.svg"), svg(&webp)).unwrap();
            let error = out
                .picture(&mut images, &renderer, "oversized.svg", None)
                .await
                .err()
                .expect("embedded raster must be rejected before SVG rendering");
            assert!(format!("{error:#}").contains("128 MiB"), "{error:#}");
            assert_eq!(out.media.len(), 1, "failed SVG adds no media part");
        });
    }

    #[test]
    fn word_webp_conversion_retains_resolution_and_checks_the_decode_budget() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let mut bytes = Vec::new();
            image::DynamicImage::new_rgb8(3, 2)
                .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::WebP)
                .unwrap();
            std::fs::write(directory.path().join("small.webp"), &bytes).unwrap();
            let renderer = Renderer::default();
            let mut images = Images::new(directory.path(), crate::network::client());
            let mut out = Docx::default();
            let picture = out
                .picture(&mut images, &renderer, "small.webp", None)
                .await
                .unwrap();
            assert_eq!(picture.size, (3., 2.));
            assert_eq!(out.media.len(), 1);
            assert!(out.media[0].1.starts_with(b"\x89PNG"));

            // Mutate only the lossless header, so an oversized compressed fixture
            // remains tiny and must be rejected before any pixel allocation.
            let lossless = bytes.windows(4).position(|chunk| chunk == b"VP8L").unwrap();
            assert_eq!(bytes[lossless + 8], 0x2f);
            let dimensions = (8193_u32 - 1) | ((4096_u32 - 1) << 14);
            bytes[lossless + 9..lossless + 13].copy_from_slice(&dimensions.to_le_bytes());
            std::fs::write(directory.path().join("oversized.webp"), &bytes).unwrap();
            let error = out
                .picture(&mut images, &renderer, "oversized.webp", None)
                .await
                .err()
                .expect("oversized WebP must be rejected before conversion");
            assert!(error.to_string().contains("128 MiB"), "{error:#}");
            assert_eq!(out.media.len(), 1, "a rejected image adds no media part");
        });
    }

    #[test]
    fn word_styles_resolve_portable_fonts_and_preserve_explicit_families() {
        let directory = tempfile::tempdir().unwrap();
        let mut snapshot = Snapshot {
            directory: directory.path().into(),
            ..fixture("")
        };
        let system_fonts = pecan_mermaid::DiagramFonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
        );
        let mono_fonts = pecan_mermaid::DiagramFonts::default();
        let word_chinese = if cfg!(target_os = "macos") {
            "Hiragino Sans GB"
        } else {
            system_fonts.chinese_family()
        };
        for (english, chinese, code, expected) in [
            (
                pecan_mermaid::SYSTEM_UI_FONT,
                pecan_mermaid::SYSTEM_UI_FONT,
                pecan_mermaid::SYSTEM_MONO_FONT,
                (
                    system_fonts.english_family(),
                    word_chinese,
                    mono_fonts.english_family(),
                ),
            ),
            (
                "",
                "",
                "",
                (
                    mono_fonts.english_family(),
                    word_chinese,
                    mono_fonts.english_family(),
                ),
            ),
            (
                "Latin & UI",
                "中文字体",
                "Mono<Code>",
                ("Latin & UI", "中文字体", "Mono<Code>"),
            ),
        ] {
            let prefs = Arc::make_mut(&mut snapshot.preferences);
            prefs.english_font = english.into();
            prefs.chinese_font = chinese.into();
            prefs.code_font = code.into();
            let bytes = Docx::default().finish(&snapshot).unwrap();
            let xml = read_part(&mut package(bytes), "word/styles.xml");
            let document = roxmltree::Document::parse(&xml).unwrap();
            let namespace = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
            let defaults = document
                .descendants()
                .find(|node| node.has_tag_name("docDefaults"))
                .unwrap();
            let body_font = defaults
                .descendants()
                .find(|node| node.has_tag_name("rFonts"))
                .unwrap();
            assert_eq!(body_font.attribute((namespace, "ascii")), Some(expected.0));
            assert_eq!(
                body_font.attribute((namespace, "eastAsia")),
                Some(expected.1)
            );
            let code_style = document
                .descendants()
                .find(|node| node.attribute((namespace, "styleId")) == Some("Code"))
                .unwrap();
            let code_font = code_style
                .descendants()
                .find(|node| node.has_tag_name("rFonts"))
                .unwrap();
            assert_eq!(code_font.attribute((namespace, "ascii")), Some(expected.2));
            let inline_code = document
                .descendants()
                .find(|node| node.attribute((namespace, "styleId")) == Some("CodeChar"))
                .unwrap();
            assert!(
                !inline_code
                    .descendants()
                    .any(|node| node.has_tag_name("bdr")),
                "editable inline code is drawn by shading alone"
            );
            let fill = inline_code
                .descendants()
                .find(|node| node.has_tag_name("shd"))
                .unwrap();
            assert_eq!(
                fill.attribute((namespace, "fill")),
                Some("E8E8E8"),
                "gray shading covers code and its horizontal edge runs"
            );
            let padding = document
                .descendants()
                .find(|node| node.attribute((namespace, "styleId")) == Some("CodePadding"))
                .unwrap();
            assert!(padding.descendants().any(|node| {
                node.has_tag_name("basedOn")
                    && node.attribute((namespace, "val")) == Some("CodeChar")
            }));
            let padding_font = padding
                .descendants()
                .find(|node| node.has_tag_name("rFonts"))
                .unwrap();
            for script in ["ascii", "hAnsi", "eastAsia", "cs"] {
                assert_eq!(
                    padding_font.attribute((namespace, script)),
                    Some(expected.2),
                    "both edges use the same font regardless of neighboring script"
                );
            }
            assert!(padding.descendants().any(|node| {
                node.has_tag_name("w") && node.attribute((namespace, "val")) == Some("40")
            }));
            for (name, keep_next) in [("Highlight", false), ("NoteTitle", false)] {
                let note = document
                    .descendants()
                    .find(|node| node.attribute((namespace, "styleId")) == Some(name))
                    .unwrap();
                assert!(
                    note.descendants()
                        .any(|node| node.has_tag_name("keepLines"))
                );
                assert_eq!(
                    note.descendants().any(|node| node.has_tag_name("keepNext")),
                    keep_next,
                    "only the title is tied to the next paragraph"
                );
            }
            assert!(!xml.contains(pecan_mermaid::SYSTEM_UI_FONT));
            assert!(!xml.contains(pecan_mermaid::SYSTEM_MONO_FONT));
        }
    }

    #[test]
    fn print_styles_keep_compact_body_spacing_and_native_table_line_extents() {
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        for line_height in [1.2, 1.8, 2.6] {
            let xml = styles(
                "Menlo",
                "PingFang SC",
                "PingFang SC Semibold",
                "Menlo",
                line_height,
            );
            let document = roxmltree::Document::parse(&xml).unwrap();
            let defaults = document
                .descendants()
                .find(|n| n.has_tag_name("pPrDefault"))
                .unwrap();
            let spacing = defaults
                .descendants()
                .find(|n| n.has_tag_name("spacing"))
                .unwrap();
            assert_eq!(spacing.attribute((w, "lineRule")), Some("atLeast"));
            assert!(
                spacing
                    .attribute((w, "line"))
                    .unwrap()
                    .parse::<u32>()
                    .unwrap()
                    <= 330
            );
            let cell = document
                .descendants()
                .find(|n| n.attribute((w, "styleId")) == Some("TableCell"))
                .unwrap();
            assert!(
                cell.descendants()
                    .any(|n| n.has_tag_name("snapToGrid") && n.attribute((w, "val")) == Some("0"))
            );
            let spacing = cell
                .descendants()
                .find(|n| n.has_tag_name("spacing"))
                .unwrap();
            assert_eq!(spacing.attribute((w, "lineRule")), Some("auto"));
            assert_eq!(spacing.attribute((w, "line")), Some("276"));
            assert_eq!(spacing.attribute((w, "before")), Some("0"));
            assert_eq!(spacing.attribute((w, "after")), Some("0"));
            assert!(
                !xml.contains("TableCode"),
                "plain text and capsules must share the same cell paragraph style"
            );
        }
    }

    #[test]
    fn quotes_use_padded_native_cells_and_restore_nested_table_context() {
        let source = "> 第一段 `code`。\n>\n> 第二段。\n>\n> > 嵌套引用。\n>\n> | a | b |\n> | - | - |\n> | 多行 | 甲<br>乙 |\n\n末尾正文。";
        let mut out = Docx::default();
        for event in Parser::new_ext(source, options()) {
            out.event(event);
        }
        assert!(out.frames.is_empty());
        assert!(out.table.is_none());
        let xml = format!(
            "<root xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">{}</root>",
            out.body
        );
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let frames: Vec<_> = document
            .descendants()
            .filter(|n| {
                n.has_tag_name("tbl")
                    && n.children().any(|p| {
                        p.has_tag_name("tblPr")
                            && p.children().any(|c| {
                                c.has_tag_name("tblCaption")
                                    && c.attribute((w, "val")) == Some("引用块")
                            })
                    })
            })
            .collect();
        assert_eq!(frames.len(), 2);
        for frame in frames {
            let padding = frame
                .descendants()
                .find(|n| n.has_tag_name("tblCellMar"))
                .unwrap();
            let value = |side| {
                padding
                    .children()
                    .find(|n| n.has_tag_name(side))
                    .unwrap()
                    .attribute((w, "w"))
                    .unwrap()
                    .parse::<u32>()
                    .unwrap()
            };
            assert_eq!(value("top"), value("bottom"));
            assert_eq!(value("left"), value("right"));
            assert!(value("top") >= 120);
            let cell = frame
                .children()
                .find(|n| n.has_tag_name("tr"))
                .unwrap()
                .children()
                .find(|n| n.has_tag_name("tc"))
                .unwrap();
            assert!(
                cell.children()
                    .find(|n| n.has_tag_name("tcPr"))
                    .unwrap()
                    .children()
                    .any(|n| n.has_tag_name("vAlign") && n.attribute((w, "val")) == Some("center"))
            );
        }
        assert!(out.body.contains("<w:pStyle w:val=\"TableCell\"/>"));
        assert!(!out.body.contains("<w:ind w:left=\"420\""));
        for text in [
            "第一段 ",
            "code",
            "第二段。",
            "嵌套引用。",
            "甲",
            "乙",
            "末尾正文。",
        ] {
            assert!(out.body.contains(text), "{text}");
        }
        out.event(Event::Start(Tag::BlockQuote(None)));
        out.note_end();
        assert_eq!(
            out.frames.len(),
            1,
            "an unmatched note close cannot close a quote"
        );
        out.event(Event::End(TagEnd::BlockQuote(None)));
        assert!(out.frames.is_empty());
    }

    #[test]
    fn native_note_cells_use_semantic_colors() {
        for (kind, fill) in [
            (AlertKind::Note, "E8F2FF"),
            (AlertKind::Tip, "E7F6EE"),
            (AlertKind::Important, "F4EAFF"),
            (AlertKind::Warning, "FFF6E8"),
            (AlertKind::Error, "FDECEC"),
        ] {
            let mut out = Docx::default();
            out.frame_start(FrameStyle::Note(Some(kind)));
            out.run("Editable", Style::Text, None);
            out.frame_end();
            assert!(out.body.contains(&format!("w:fill=\"{fill}\"")), "{kind:?}");
        }
    }

    #[test]
    fn typed_and_generic_notes_keep_editable_text_and_share_icon_media() {
        let directory = tempfile::tempdir().unwrap();
        let snapshot = Snapshot {
            directory: directory.path().into(),
            ..fixture(super::super::document::NOTE_FIXTURE)
        };
        let mut archive = export(&snapshot);
        let media = (0..archive.len())
            .filter(|&index| {
                archive
                    .by_index(index)
                    .unwrap()
                    .name()
                    .unwrap()
                    .starts_with("word/media/")
            })
            .count();
        assert_eq!(
            media, 3,
            "typed notes, generic notes and unlabeled code share their respective icon media"
        );
        let xml = read_part(&mut archive, "word/document.xml");
        let document = roxmltree::Document::parse(&xml).unwrap();
        let w = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
        let notes: Vec<_> = document
            .descendants()
            .filter(|node| {
                node.has_tag_name((w, "tbl"))
                    && node.children().any(|properties| {
                        properties.has_tag_name((w, "tblPr"))
                            && properties.children().any(|caption| {
                                caption.has_tag_name((w, "tblCaption"))
                                    && caption.attribute((w, "val")) == Some("提示块")
                            })
                    })
            })
            .collect();
        assert_eq!(
            notes.len(),
            3,
            "each note has one editable padded container"
        );
        for note in notes {
            let margins = note
                .descendants()
                .find(|node| node.has_tag_name((w, "tblCellMar")))
                .expect("note padding");
            for side in ["top", "bottom", "left", "right"] {
                let padding = margins
                    .children()
                    .find(|node| node.has_tag_name((w, side)))
                    .expect("padding side");
                assert!(
                    padding.attribute((w, "w")).unwrap().parse::<u32>().unwrap() >= 120,
                    "note {side} padding"
                );
            }
        }
        assert_eq!(
            xml.matches("<w:drawing>").count(),
            4,
            "three note marks and one generic code mark"
        );
        assert_eq!(xml.matches("<w:pStyle w:val=\"Highlight\"/>").count(), 3);
        assert_eq!(xml.matches("<w:pStyle w:val=\"NoteTitle\"/>").count(), 2);
        for expected in [
            "💡 自定义",
            "保留 ℹ️ 正文。",
            "💡 提示正文",
            "⚠️ 💡 ℹ️ ❗ 🚫 ❌",
        ] {
            assert!(
                xml.contains(expected),
                "Word text lacks original content {expected}"
            );
        }
        assert!(
            !xml.contains(">ℹ️ </w:t>"),
            "generated title emoji is replaced by an icon"
        );
    }
    #[test]
    fn docx_is_well_formed_editable_and_round_trips_through_the_importer() {
        let directory = tempfile::tempdir().unwrap();
        let png = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9ZcXcAAAAASUVORK5CYII=",
        )
        .unwrap();
        std::fs::write(directory.path().join("图 片.png"), &png).unwrap();
        let source = "# 导出 & 标题\n\n正文 **粗体** *斜体* ~~删除~~ `code<>` 与 [链接](https://example.com/?a=1&b=2) 和公式 $y_i^2$。\n\n> 引用段落\n\n1. 第一项\n   - 嵌套项目\n2. 第二项\n\n间隔段落\n\n3. 新列表\n\n- [x] 已完成\n\n| 左 | 中 | 右 |\n|:--|:-:|--:|\n| a | **b** | 第一行<br>第二行 |\n\n```rust\nfn main() {\n\tprintln!(\"你好\");\n}\n```\n\n$$\n\\frac{a}{b}\n$$\n\n![本地图](<图 片.png>)\n\n<img src=\"图%20片.png\" width=\"40\" alt=\"HTML 图\">\n\n注释[^1]\n\n[^1]: 脚注内容\n\n---\n\n:::\n提示正文\n:::\n\n```mermaid\nflowchart LR\n  A[开始] --> B[结束]\n```\n";
        // A raw opening marker without its closing marker must still produce valid Word XML.
        let source =
            format!("{source}\n<div class=\"pecan-highlight\"></div>\n\n未闭合提示正文。\n");
        let snapshot = Snapshot {
            directory: directory.path().into(),
            title: "验证 & 文档.md".into(),
            ..fixture(&source)
        };
        let bytes =
            futures_lite::future::block_on(docx(&snapshot, crate::network::client())).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        let mut media = 0;
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().unwrap().into_owned();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            if name.ends_with(".xml") || name.ends_with(".rels") {
                let text = String::from_utf8(contents).unwrap();
                roxmltree::Document::parse(&text)
                    .unwrap_or_else(|error| panic!("{name} is not well-formed XML: {error}"));
            } else if name.starts_with("word/media/") {
                assert!(contents.starts_with(b"\x89PNG"), "{name} is a PNG");
                media += 1;
            }
        }
        assert_eq!(
            media, 6,
            "one shared photo, inline and display formulas, the note icon, language logo and diagram"
        );
        let document = read_part(&mut archive, "word/document.xml");
        for expected in [
            "<w:pStyle w:val=\"Heading1\"/>",
            "<w:numId w:val=\"2\"/>",
            "<w:numId w:val=\"3\"/>",
            "<w:ilvl w:val=\"1\"/><w:numId w:val=\"1\"/>",
            "<w:pStyle w:val=\"Quote\"/>",
            "<w:pStyle w:val=\"Code\"/>",
            "<w:rStyle w:val=\"CodeChar\"/>",
            "<w:tblHeader/>",
            "<w:jc w:val=\"right\"/>",
            "<w:tab/>",
            "<w:position w:val=",
            "<w:vertAlign w:val=\"superscript\"/>",
            "code&lt;&gt;",
            "☑ ",
            "提示正文",
            "未闭合提示正文。",
            "<w:pStyle w:val=\"Highlight\"/>",
        ] {
            assert!(document.contains(expected), "document.xml lacks {expected}");
        }
        assert!(
            !document.contains(":::"),
            "callout fences must not be stored as text"
        );
        assert!(
            !document.contains("💡"),
            "the note uses an embedded flat icon"
        );
        let fonts = word_fonts(
            &snapshot.preferences.english_font,
            &snapshot.preferences.chinese_font,
        );
        if let Some(emphasis) = cjk_emphasis_face(fonts.chinese_family()) {
            assert!(
                document.contains(&format!(
                    "<w:rFonts w:eastAsia=\"{emphasis}\" w:hint=\"eastAsia\"/>"
                )),
                "runs name the selected CJK family's separate emphasis face"
            );
        }
        assert!(
            document.contains("<w:color w:val="),
            "code keeps syntax colors"
        );
        let numbering = read_part(&mut archive, "word/numbering.xml");
        assert!(
            numbering.contains("<w:startOverride w:val=\"3\"/>"),
            "a separate ordered list keeps its own start number"
        );
        let styles = read_part(&mut archive, "word/styles.xml");
        let emphasis = cjk_emphasis_face(fonts.chinese_family()).unwrap_or(fonts.chinese_family());
        assert!(
            styles.contains(&format!("w:eastAsia=\"{emphasis}\"")),
            "heading styles keep the platform CJK font and its emphasis face"
        );
        // The project's own Word importer reads the structure back.
        let path = directory.path().join("导出.docx");
        std::fs::write(&path, &bytes).unwrap();
        let imported = pecan_import::load(&path).unwrap();
        let markdown = imported.document.source();
        for expected in [
            "导出 & 标题",
            "**粗体**",
            "第一项",
            "嵌套项目",
            "println!",
            "第一行",
            "脚注内容",
            "提示正文",
        ] {
            assert!(
                markdown.contains(expected),
                "re-imported Word lacks {expected}:\n{markdown}"
            );
        }
        assert!(markdown.contains('|'), "the table survives as a table");
    }

    #[test]
    fn entities_decode_and_xml_drops_forbidden_characters() {
        assert_eq!(
            decode_entities("a&amp;b &lt;c&gt; &#20013;&#x6587; &bogus; &"),
            "a&b <c> 中文 &bogus; &"
        );
        let mut output = String::new();
        escape_into(&mut output, "x\u{1}<\"y\"\u{FFFF}");
        assert_eq!(output, "x&lt;&quot;y&quot;");
    }

    #[test]
    fn entities_preserve_unicode_and_malformed_input_without_panicking() {
        for text in [
            "&中文中文中文中文",
            "AT&T中文中文中文",
            "&😀😀😀",
            "&中文; &unknown; &#not-a-number; &#xD800; &#1114112;",
            "plain 中文 😀 & trailing",
            "&&&&&",
        ] {
            assert_eq!(decode_entities(text), text);
        }
        assert_eq!(
            decode_entities("&amp;中文 &lt;😀&gt; &#x1F600; &#128512; &nbsp;&quot;&apos;"),
            "&中文 <😀> 😀 😀 \u{a0}\"'"
        );
    }
}
