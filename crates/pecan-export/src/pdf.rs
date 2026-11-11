//! Portable PDF export in pure Rust: Markdown is laid out by our own engine and written
//! with krilla for portable output without a browser.
mod layout;
pub(super) mod text;

use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::{Context as _, Result, ensure};
use http_client::HttpClient;
use pecan_document::AlertKind;
use pecan_math::{MathFont, MathStyle, Presentation};
use pecan_mermaid::{Appearance, Diagram, Renderer};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Tag, TagEnd};

use super::{
    Snapshot, code,
    document::{MAX_NOTE_DEPTH, MAX_OUTPUT, highlight_edge, note_text},
    images::Images,
    note_icons::{Icons, SIZE as NOTE_SIZE},
};
use layout::{Align, BODY_SIZE, CODE_SIZE, INK, LINK, LIST_STEP, MUTED, QUOTE_STEP};
use text::{Inline, Role, SOFT_BREAK, Style};

/// Logical pixels (96 dpi) to points.
const PX: f32 = 0.75;

#[derive(Clone)]
pub(crate) enum Object {
    Raster(krilla::image::Image),
    Vector(Vector),
}

#[derive(Clone)]
pub(crate) enum Vector {
    Diagram(Diagram),
    Tree(Arc<usvg::Tree>),
}

impl Vector {
    fn tree(&self) -> &usvg::Tree {
        match self {
            Self::Diagram(diagram) => diagram.tree(),
            Self::Tree(tree) => tree,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ParaKind {
    Body,
    Heading(u8),
    Item,
    Footnote,
}

pub(super) struct Para {
    inlines: Vec<Inline>,
    kind: ParaKind,
    indent: f32,
    marker: Option<Inline>,
    quote: u8,
    align: Align,
    /// Keep a note title with the beginning of its body.
    keep_next: bool,
}

pub(super) struct Row {
    head: bool,
    /// A cell is a short column of paragraphs, indented relative to the cell.
    cells: Vec<Vec<Para>>,
}

enum Block {
    Paragraph(Para),
    Note {
        blocks: Vec<Block>,
        indent: f32,
        kind: Option<AlertKind>,
    },
    Code {
        inlines: Vec<Inline>,
        label: String,
        icon: Option<Object>,
        indent: f32,
    },
    Table {
        aligns: Vec<Alignment>,
        rows: Vec<Row>,
        indent: f32,
    },
    Figure {
        object: Object,
        width: f32,
        height: f32,
        indent: f32,
        framed: bool,
    },
    Rule {
        indent: f32,
    },
}

pub(super) async fn write(
    snapshot: &Snapshot,
    path: &Path,
    client: Arc<dyn HttpClient>,
) -> Result<()> {
    let bytes = pdf(snapshot, client).await?;
    ensure!(bytes.len() <= MAX_OUTPUT, "PDF 超过 128 MiB 导出预算");
    super::atomic_write_inner(path, &bytes)
}

pub(super) async fn pdf(snapshot: &Snapshot, client: Arc<dyn HttpClient>) -> Result<Vec<u8>> {
    let blocks = Builder::default().build(snapshot, client).await?;
    let prefs = &snapshot.preferences;
    let mut fonts = text::Fonts::new(&prefs.english_font, &prefs.chinese_font, &prefs.code_font);
    let title = Path::new(&snapshot.title)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(&snapshot.title);
    layout::render(
        &blocks,
        &mut fonts,
        prefs.line_height.clamp(1.2, 2.2) as f32,
        title,
    )
}

enum Container {
    Quote,
    List { next: Option<u64> },
    Item { marker: Option<Inline> },
    Footnote { label: Option<String> },
}

#[derive(Default)]
struct Inlines {
    bold: u32,
    italic: u32,
    strike: u32,
    code: u32,
    capsule: Option<usize>,
    link: Option<Arc<str>>,
}

struct Table {
    aligns: Vec<Alignment>,
    rows: Vec<Row>,
    head: bool,
    /// Containers opened around the table. Cell content is shaped only by
    /// the containers opened inside its cell.
    outer: usize,
}

struct Note {
    start: usize,
    indent: f32,
    icon: Object,
    kind: Option<AlertKind>,
    prefix: Option<AlertKind>,
    led: bool,
    titled: bool,
}

#[derive(Default)]
struct Builder {
    blocks: Vec<Block>,
    containers: Vec<Container>,
    current: Vec<Inline>,
    kind: Option<ParaKind>,
    inline: Inlines,
    table: Option<Table>,
    bytes: usize,
    next_capsule: usize,
    notes: Vec<Note>,
    pictures: HashMap<String, (Object, f32, f32)>,
}

impl Builder {
    async fn build(
        mut self,
        snapshot: &Snapshot,
        client: Arc<dyn HttpClient>,
    ) -> Result<Vec<Block>> {
        let mut images = Images::new(&snapshot.directory, client);
        let mut icons = Icons::default();
        let mut language_icons = super::language_icons::Icons::default();
        let prefs = &snapshot.preferences;
        // Formulas are typeset at reading sizes; keep their ratio to the 11 pt body.
        let math_scale = BODY_SIZE / (prefs.font_size as f32 * PX);
        let renderer = &snapshot.diagrams.renderer;
        let mut formats = super::code::Formats::new(
            snapshot.codes.clone(),
            snapshot.preferences.code_tab_size.clone(),
        );
        let source = super::document::export_markdown(snapshot.document.source());
        // Cells with lists, tasks or breaks arrive as block events, as in Word and HTML.
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
                    let mut source = String::new();
                    for event in parser.by_ref() {
                        match event {
                            Event::End(TagEnd::CodeBlock) => break,
                            Event::Text(text) => source.push_str(&text),
                            _ => {}
                        }
                    }
                    let source = source.strip_suffix('\n').unwrap_or(&source);
                    self.close();
                    if language.eq_ignore_ascii_case("mermaid") {
                        let _permit = snapshot.diagrams.slots.acquire().await;
                        let diagram = renderer
                            .render_with_fonts(source, Appearance::Light, &prefs.diagram_fonts())
                            .context("导出 Mermaid")?;
                        let (width, height) = diagram.dimensions();
                        self.block(Block::Figure {
                            object: Object::Vector(Vector::Diagram(diagram)),
                            width: width * PX,
                            height: height * PX,
                            indent: self.indent(),
                            framed: true,
                        });
                    } else {
                        let formatted = formats.resolve(&language, source).await;
                        self.code(
                            formatted.as_ref().map_or(source, |output| output.text()),
                            &language,
                            &mut language_icons,
                        )?;
                    }
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    for event in parser.by_ref() {
                        if matches!(event, Event::End(TagEnd::Image)) {
                            break;
                        }
                    }
                    let (object, width, height) =
                        self.picture(&mut images, renderer, &dest_url).await?;
                    self.object(object, width, height, 0.);
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
                    let tree = usvg::Tree::from_str(formula.svg(), &usvg::Options::default())
                        .context("解析公式 SVG")?;
                    let (width, height) = formula.dimensions();
                    let scale = PX * math_scale;
                    let object = Object::Vector(Vector::Tree(Arc::new(tree)));
                    if display {
                        self.close();
                        self.block(Block::Figure {
                            object,
                            width: width * scale,
                            height: height * scale,
                            indent: self.indent(),
                            framed: false,
                        });
                    } else {
                        let drop = (height - formula.baseline()) * scale;
                        self.object(object, width * scale, height * scale, drop);
                    }
                }
                Event::Html(raw) | Event::InlineHtml(raw) => {
                    if let Some(marker) = highlight_edge(&raw) {
                        match marker {
                            super::document::NoteMarker::Open { kind, titled } => {
                                ensure!(
                                    self.notes.len() < MAX_NOTE_DEPTH,
                                    "PDF 提示块嵌套超过 32 层"
                                );
                                self.close();
                                self.notes.push(Note {
                                    start: self.blocks.len(),
                                    indent: self.indent(),
                                    icon: Object::Vector(Vector::Tree(icons.tree(kind)?)),
                                    kind,
                                    prefix: kind,
                                    led: false,
                                    titled,
                                });
                            }
                            super::document::NoteMarker::Close => {
                                self.close_note();
                            }
                        }
                    } else {
                        self.html(&raw, &mut images, renderer).await?;
                    }
                }
                event => self.event(event),
            }
            ensure!(self.bytes <= MAX_OUTPUT, "PDF 超过 128 MiB 导出预算");
        }
        self.close();
        while !self.notes.is_empty() {
            self.close_note();
        }
        Ok(self.blocks)
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.open(self.default_kind()),
                Tag::Heading { level, .. } => self.open(ParaKind::Heading(level as u8)),
                Tag::BlockQuote(_) => self.push(Container::Quote),
                Tag::List(start) => self.push(Container::List { next: start }),
                Tag::Item => {
                    let depth = self.list_depth();
                    let marker = match self.containers.iter_mut().rev().find_map(|c| match c {
                        Container::List { next } => Some(next),
                        _ => None,
                    }) {
                        Some(Some(number)) => {
                            let text = format!("{number}.");
                            *number += 1;
                            text
                        }
                        _ => ["•", "◦", "▪"][depth.saturating_sub(1) % 3].to_owned(),
                    };
                    let size = if self.table.is_some() {
                        layout::CELL_SIZE
                    } else {
                        BODY_SIZE
                    };
                    let marker = Inline::Text(marker, Style::new(Role::Body, size, INK));
                    self.push(Container::Item {
                        marker: Some(marker),
                    });
                }
                Tag::FootnoteDefinition(label) => self.push(Container::Footnote {
                    label: Some(label.to_string()),
                }),
                Tag::Table(aligns) => {
                    self.close();
                    self.table = Some(Table {
                        aligns,
                        rows: Vec::new(),
                        head: false,
                        outer: self.containers.len(),
                    });
                }
                Tag::TableHead => self.row(true),
                Tag::TableRow => self.row(false),
                Tag::TableCell => {
                    if let Some(row) = self.table.as_mut().and_then(|table| table.rows.last_mut()) {
                        row.cells.push(Vec::new());
                    }
                }
                Tag::Emphasis => self.inline.italic += 1,
                Tag::Strong => self.inline.bold += 1,
                Tag::Strikethrough => self.inline.strike += 1,
                Tag::Link { dest_url, .. } => {
                    let external = ["https://", "http://", "mailto:"]
                        .iter()
                        .any(|scheme| dest_url.starts_with(scheme));
                    self.inline.link = external.then(|| Arc::from(dest_url.as_ref()));
                }
                _ => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::HtmlBlock | TagEnd::TableCell => {
                    self.close()
                }
                TagEnd::BlockQuote(_)
                | TagEnd::List(_)
                | TagEnd::Item
                | TagEnd::FootnoteDefinition => {
                    self.close();
                    self.containers.pop();
                }
                TagEnd::Table => {
                    if let Some(table) = self.table.take() {
                        self.blocks.push(Block::Table {
                            aligns: table.aligns,
                            rows: table.rows,
                            indent: self.indent(),
                        });
                    }
                }
                TagEnd::TableHead => {
                    if let Some(table) = &mut self.table {
                        table.head = false;
                    }
                }
                TagEnd::Emphasis => self.inline.italic = self.inline.italic.saturating_sub(1),
                TagEnd::Strong => self.inline.bold = self.inline.bold.saturating_sub(1),
                TagEnd::Strikethrough => self.inline.strike = self.inline.strike.saturating_sub(1),
                TagEnd::Link => self.inline.link = None,
                _ => {}
            },
            Event::Text(text) => {
                let text = match self.notes.last_mut() {
                    Some(note) => note_text(&text, &mut note.prefix),
                    None => text.as_ref(),
                };
                self.text(text, false);
            }
            Event::Code(text) => self.code_span(&text),
            Event::SoftBreak => self.text(&SOFT_BREAK.to_string(), false),
            Event::HardBreak => self.inline_push(Inline::Break),
            Event::Rule => {
                self.close();
                self.block(Block::Rule {
                    indent: self.indent(),
                });
            }
            Event::TaskListMarker(done) => self.text(if done { "☑ " } else { "☐ " }, false),
            Event::FootnoteReference(label) => {
                let mut style = self.style(false);
                style.superscript = true;
                self.inline_push(Inline::Text(format!("[{label}]"), style));
            }
            _ => {}
        }
    }

    fn push(&mut self, container: Container) {
        self.close();
        self.containers.push(container);
    }
    /// The containers that shape the open paragraph: those inside the open
    /// table cell, or all of them outside a table.
    fn scope(&self) -> &[Container] {
        &self.containers[self.table.as_ref().map_or(0, |table| table.outer)..]
    }
    fn list_depth(&self) -> usize {
        self.scope()
            .iter()
            .filter(|c| matches!(c, Container::List { .. }))
            .count()
    }
    /// Every enclosing quote, also around a table: quoted text is muted in cells too.
    fn quotes(&self) -> u8 {
        self.containers
            .iter()
            .filter(|c| matches!(c, Container::Quote))
            .count() as u8
    }
    /// From the page margin, or from the cell's edge inside a table.
    fn indent(&self) -> f32 {
        let quotes = self
            .scope()
            .iter()
            .filter(|c| matches!(c, Container::Quote))
            .count();
        self.list_depth() as f32 * LIST_STEP + quotes as f32 * QUOTE_STEP
    }
    fn default_kind(&self) -> ParaKind {
        let in_item = self
            .scope()
            .iter()
            .any(|c| matches!(c, Container::Item { .. }));
        let in_footnote = self
            .scope()
            .iter()
            .any(|c| matches!(c, Container::Footnote { .. }));
        if in_footnote {
            ParaKind::Footnote
        } else if in_item {
            ParaKind::Item
        } else {
            ParaKind::Body
        }
    }
    fn open(&mut self, kind: ParaKind) {
        self.close();
        self.kind = Some(kind);
    }

    fn style(&self, code: bool) -> Style {
        let heading = match self.kind {
            Some(ParaKind::Heading(level)) => Some(level),
            _ => None,
        };
        let base = match (heading, self.kind) {
            (Some(level), _) => BODY_SIZE * layout::heading_scale(level),
            (_, Some(ParaKind::Footnote)) => BODY_SIZE * 0.88,
            _ if self.table.is_some() => layout::CELL_SIZE,
            _ => BODY_SIZE,
        };
        let color = if self.inline.link.is_some() {
            LINK
        } else if heading == Some(6) || matches!(self.kind, Some(ParaKind::Footnote)) {
            [0x66; 3]
        } else if self.quotes() > 0 {
            MUTED
        } else {
            INK
        };
        let mut style = Style::new(
            if code { Role::Code } else { Role::Body },
            if code { base * 0.88 } else { base },
            color,
        );
        let head = self.table.as_ref().is_some_and(|table| table.head);
        style.bold = self.inline.bold > 0 || heading.is_some() || head;
        style.italic = self.inline.italic > 0;
        style.strike = self.inline.strike > 0;
        style.capsule = if code { self.inline.capsule } else { None };
        style.link = self.inline.link.clone();
        style
    }

    fn code_span(&mut self, text: &str) {
        let previous = self.inline.capsule;
        if previous.is_none() {
            self.inline.capsule = Some(self.next_capsule);
            self.next_capsule += 1;
        }
        self.inline.code += 1;
        for event in super::document::code_events(text) {
            self.event(event);
        }
        self.inline.code -= 1;
        self.inline.capsule = previous;
    }

    fn text(&mut self, text: &str, code: bool) {
        if text.is_empty() {
            return;
        }
        let style = self.style(code || self.inline.code > 0);
        self.inline_push(Inline::Text(text.to_owned(), style));
    }
    fn inline_push(&mut self, inline: Inline) {
        if let Inline::Text(text, _) = &inline {
            self.bytes += text.len();
        }
        if self.kind.is_none() {
            self.kind = Some(self.default_kind());
        }
        self.current.push(inline);
    }
    fn object(&mut self, object: Object, width: f32, height: f32, drop: f32) {
        let link = self.inline.link.clone();
        self.inline_push(Inline::Object {
            object,
            width,
            height,
            align: text::ObjectAlign::Baseline(drop),
            link,
        });
    }
    fn row(&mut self, head: bool) {
        if let Some(table) = &mut self.table {
            table.head = head;
            table.rows.push(Row {
                head,
                cells: Vec::new(),
            });
        }
    }

    /// Finish the open paragraph, attaching a pending list marker or footnote label.
    fn close(&mut self) {
        let Some(kind) = self.kind.take() else {
            return;
        };
        let mut inlines = std::mem::take(&mut self.current);
        if inlines.is_empty() {
            return;
        }
        let mut marker = None;
        let outer = self.table.as_ref().map_or(0, |table| table.outer);
        for container in self.containers[outer..].iter_mut().rev() {
            match container {
                Container::Item { marker: pending } => {
                    marker = pending.take();
                    break;
                }
                Container::Footnote { label } => {
                    if let Some(label) = label.take() {
                        let mut style = Style::new(Role::Body, BODY_SIZE * 0.88, [0x66; 3]);
                        style.superscript = true;
                        inlines.insert(0, Inline::Text(format!("[{label}] "), style));
                    }
                    break;
                }
                _ => {}
            }
        }
        // A note's symbol and title lead its own paragraphs, never a cell's.
        let cell = self.table.is_some();
        let highlight = !cell && !self.notes.is_empty() && matches!(kind, ParaKind::Body);
        let keep_next = highlight
            && self
                .notes
                .last()
                .is_some_and(|note| note.titled && !note.led);
        if highlight
            && let Some(note) = self.notes.last_mut()
            && !note.led
        {
            let size = NOTE_SIZE * PX;
            inlines.insert(
                0,
                Inline::Object {
                    object: note.icon.clone(),
                    width: size,
                    height: size,
                    // Resolve against the actual glyphs after line breaking.
                    align: text::ObjectAlign::TextCenter,
                    link: None,
                },
            );
            inlines.insert(
                1,
                Inline::Text(" ".into(), Style::new(Role::Body, BODY_SIZE, INK)),
            );
            note.led = true;
        }
        let quote = if cell { 0 } else { self.quotes() };
        let align = if kind == ParaKind::Body && quote == 0 && !highlight && !cell {
            Align::Justify
        } else {
            Align::Left
        };
        self.block(Block::Paragraph(Para {
            inlines,
            kind,
            indent: self.indent(),
            marker,
            quote,
            align,
            keep_next,
        }));
    }

    /// Add a finished block to the page, or to the open table cell. A cell
    /// holds paragraphs only: code and figures join it as text and inline
    /// objects, and a rule has no place in it.
    fn block(&mut self, block: Block) {
        let Some(cell) = self
            .table
            .as_mut()
            .and_then(|table| table.rows.last_mut())
            .and_then(|row| row.cells.last_mut())
        else {
            self.blocks.push(block);
            return;
        };
        let (inlines, indent) = match block {
            Block::Paragraph(para) => {
                cell.push(para);
                return;
            }
            Block::Code {
                inlines, indent, ..
            } => (inlines, indent),
            Block::Figure {
                object,
                width,
                height,
                indent,
                ..
            } => (
                vec![Inline::Object {
                    object,
                    width,
                    height,
                    align: text::ObjectAlign::Baseline(0.),
                    link: None,
                }],
                indent,
            ),
            Block::Rule { .. } => return,
            // Neither starts inside a cell: its Markdown is parsed without tables or notes.
            Block::Table { .. } | Block::Note { .. } => {
                self.blocks.push(block);
                return;
            }
        };
        cell.push(Para {
            inlines,
            kind: ParaKind::Body,
            indent,
            marker: None,
            quote: 0,
            align: Align::Left,
            keep_next: false,
        });
    }

    fn close_note(&mut self) {
        self.close();
        if let Some(note) = self.notes.pop() {
            let blocks = self.blocks.split_off(note.start);
            if !blocks.is_empty() {
                self.blocks.push(Block::Note {
                    blocks,
                    indent: note.indent,
                    kind: note.kind,
                });
            }
        }
    }

    fn code(
        &mut self,
        source: &str,
        language: &str,
        icons: &mut super::language_icons::Icons,
    ) -> Result<()> {
        let mut inlines = Vec::new();
        for (range, ink) in code::spans(source, language) {
            let mut style = Style::new(Role::Code, CODE_SIZE, ink.rgb.unwrap_or(INK));
            style.bold = ink.bold;
            style.italic = ink.italic;
            for (index, line) in source[range].split('\n').enumerate() {
                if index > 0 {
                    inlines.push(Inline::Break);
                }
                if !line.is_empty() {
                    inlines.push(Inline::Text(line.to_owned(), style.clone()));
                }
            }
        }
        self.bytes += source.len();
        self.block(Block::Code {
            inlines,
            label: language.to_owned(),
            icon: Some(Object::Vector(Vector::Tree(icons.tree(language)?))),
            indent: self.indent(),
        });
        Ok(())
    }

    /// Raw HTML keeps its text, images, breaks and basic emphasis.
    async fn html(&mut self, raw: &str, images: &mut Images, renderer: &Renderer) -> Result<()> {
        static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
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
            let step = |count: &mut u32| {
                *count = if closing {
                    count.saturating_sub(1)
                } else {
                    *count + 1
                };
            };
            match name.as_str().to_ascii_lowercase().as_str() {
                "img" => {
                    if let Some(image) = pecan_document::html_image(whole.as_str()) {
                        let (object, mut width, mut height) =
                            self.picture(images, renderer, &image.uri).await?;
                        if let Some(wanted) = image.width.filter(|w| *w > 0.) {
                            height *= wanted * PX / width.max(f32::EPSILON);
                            width = wanted * PX;
                        }
                        self.object(object, width, height, 0.);
                    }
                }
                "br" => self.inline_push(Inline::Break),
                "hr" => self.event(Event::Rule),
                "b" | "strong" => step(&mut self.inline.bold),
                "i" | "em" => step(&mut self.inline.italic),
                "s" | "del" | "strike" => step(&mut self.inline.strike),
                "code" => {
                    if !closing && self.inline.code == 0 {
                        self.inline.capsule = Some(self.next_capsule);
                        self.next_capsule += 1;
                    }
                    step(&mut self.inline.code);
                    if self.inline.code == 0 {
                        self.inline.capsule = None;
                    }
                }
                "p" | "div" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote"
                | "ul" | "ol" | "pre"
                    if self.table.is_none() =>
                {
                    self.close();
                }
                _ => {}
            }
        }
        self.html_text(&raw[end..]);
        Ok(())
    }
    fn html_text(&mut self, text: &str) {
        let text = super::word::decode_entities(text);
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if !text.is_empty() {
            self.text(&text, false);
        }
    }
    /// Keep vectors and native raster bytes; repeated references share the same image handle.
    async fn picture(
        &mut self,
        images: &mut Images,
        renderer: &Renderer,
        uri: &str,
    ) -> Result<(Object, f32, f32)> {
        if let Some(picture) = self.pictures.get(uri) {
            return Ok(picture.clone());
        }
        let (bytes, extension) = images.fetch(uri).await?;
        let picture = if extension == "svg" {
            let diagram = images.svg_image(renderer, uri, &bytes)?;
            let (width, height) = diagram.dimensions();
            (
                Object::Vector(Vector::Diagram(diagram)),
                width * PX,
                height * PX,
            )
        } else {
            images.check_decode(uri, &bytes)?;
            let image = match extension {
                "png" => krilla::image::Image::from_png(bytes.into(), true),
                "jpg" => krilla::image::Image::from_jpeg(bytes.into(), true),
                "gif" => krilla::image::Image::from_gif(bytes.into(), true),
                "webp" => krilla::image::Image::from_webp(bytes.into(), true),
                _ => {
                    let decoded = image::load_from_memory(&bytes)?.to_rgba8();
                    let (width, height) = decoded.dimensions();
                    Ok(krilla::image::Image::from_rgba8(
                        decoded.into_raw(),
                        width,
                        height,
                    ))
                }
            }
            .map_err(|error| anyhow::anyhow!("图片无法写入 PDF：{error}"))?;
            let (width, height) = image.size();
            (Object::Raster(image), width as f32 * PX, height as f32 * PX)
        };
        self.pictures.insert(uri.to_owned(), picture.clone());
        Ok(picture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pecan_config::PreferencesData;
    use pecan_document::Document;
    use pecan_math::RenderQueue as MathQueue;
    use pecan_mermaid::RenderQueue;

    fn snapshot(source: &str) -> Snapshot {
        Snapshot {
            document: Arc::new(Document::parse(source.to_owned()).unwrap()),
            directory: std::env::temp_dir().into(),
            title: "cells.md".into(),
            preferences: Arc::new(PreferencesData::default()),
            diagrams: RenderQueue::new(),
            math: MathQueue::new(),
            codes: pecan_format::CodeSnapshot::default(),
            resources: None,
        }
    }

    fn plain(inlines: &[Inline]) -> String {
        inlines
            .iter()
            .map(|inline| match inline {
                Inline::Text(text, _) => text.as_str(),
                Inline::Break => "\n",
                Inline::Object { .. } => "\u{fffc}",
            })
            .collect()
    }

    /// The paragraphs of every body cell in the last column: marker and text.
    fn last_cells(blocks: &[Block]) -> Vec<Vec<(Option<String>, String)>> {
        let [Block::Table { rows, .. }] = blocks else {
            panic!(
                "cell content stays inside its table: {} blocks",
                blocks.len()
            )
        };
        rows.iter()
            .filter(|row| !row.head)
            .map(|row| {
                row.cells
                    .last()
                    .expect("row has cells")
                    .iter()
                    .map(|para| {
                        let marker = para.marker.as_ref().map(std::slice::from_ref);
                        (marker.map(plain), plain(&para.inlines))
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn table_cells_hold_lists_tasks_and_paragraphs_as_their_own_paragraphs() {
        futures_lite::future::block_on(async {
            let source = "| 场景 | 内容 |\n| --- | --- |\n| 任务 | - [x] **已完成**<br>- [ ] 待办 |\n| 有序 | 3. 第三项<br>4. 第四项 |\n| 单项 | - 单个项目 |\n| 混合 | 说明<br><br>- 第一项<br>- 第二项<br><br>结尾 |\n| 嵌套 | <ul><li>父项<ul><li>子项</li></ul></li></ul> |\n| 普通 | **加粗**与`code` |\n| 换行 | 第一行<br>第二行 |\n| 代码 | 前言<br><br>    let x = 1; |\n| 分隔 | 上<br><br>---<br><br>下 |\n";
            let blocks = Builder::default()
                .build(&snapshot(source), crate::network::client())
                .await
                .unwrap();
            let item = |marker: &str, text: &str| (Some(marker.to_owned()), text.to_owned());
            let body = |text: &str| (None, text.to_owned());
            assert_eq!(
                last_cells(&blocks),
                [
                    vec![item("•", "☑ 已完成"), item("•", "☐ 待办")],
                    vec![item("3.", "第三项"), item("4.", "第四项")],
                    vec![item("•", "单个项目")],
                    vec![
                        body("说明"),
                        item("•", "第一项"),
                        item("•", "第二项"),
                        body("结尾")
                    ],
                    vec![item("•", "父项"), item("◦", "子项")],
                    vec![body("加粗与code")],
                    vec![body("第一行\n第二行")],
                    vec![body("前言"), body("let x = 1;")],
                    vec![body("上"), body("下")],
                ]
            );
        });
    }

    #[test]
    fn a_note_that_begins_with_a_table_keeps_its_symbol_out_of_the_cells() {
        futures_lite::future::block_on(async {
            let source = ":::\n| 内容 |\n| --- |\n| - 第一项<br>- 第二项 |\n\n正文。\n:::\n";
            let blocks = Builder::default()
                .build(&snapshot(source), crate::network::client())
                .await
                .unwrap();
            let [Block::Note { blocks, .. }] = blocks.as_slice() else {
                panic!("one note expected")
            };
            let item = |text: &str| (Some("•".to_owned()), text.to_owned());
            assert_eq!(
                last_cells(&blocks[..1]),
                [vec![item("第一项"), item("第二项")]]
            );
            let Block::Paragraph(lead) = &blocks[1] else {
                panic!("the body paragraph follows the table")
            };
            assert!(
                matches!(lead.inlines.first(), Some(Inline::Object { .. })),
                "the first body paragraph carries the note symbol"
            );
        });
    }

    #[test]
    fn code_capsules_preserve_nested_emphasis() {
        let mut builder = Builder::default();
        builder.event(Event::Code("**version**".into()));
        builder.event(Event::Text("旁边".into()));
        builder.event(Event::Code("~~deprecated~~".into()));
        builder.close();
        let Block::Paragraph(para) = &builder.blocks[0] else {
            panic!("paragraph expected")
        };
        assert!(para.inlines.iter().any(|inline| matches!(inline, Inline::Text(text, style) if text == "version" && style.bold)), "bold delimiters must become styling inside code");
        assert!(para.inlines.iter().any(|inline| matches!(inline, Inline::Text(text, style) if text == "deprecated" && style.strike)), "strike delimiters must become styling inside code");
    }

    #[test]
    fn pdf_layout_receives_formatted_and_highlighted_code_in_nested_containers() {
        fn code_text(blocks: &[Block], output: &mut Vec<String>) {
            for block in blocks {
                match block {
                    Block::Code { inlines, .. } => {
                        let mut text = String::new();
                        for inline in inlines {
                            match inline {
                                Inline::Text(value, _) => text.push_str(value),
                                Inline::Break => text.push('\n'),
                                _ => panic!("code should remain selectable text"),
                            }
                        }
                        output.push(text);
                    }
                    Block::Note { blocks, .. } => code_text(blocks, output),
                    _ => {}
                }
            }
        }
        futures_lite::future::block_on(async {
            for width in [2, 4, 8] {
                let snapshot = Snapshot {
                    document: Arc::new(
                        Document::parse(super::super::FORMAT_FIXTURE.to_owned()).unwrap(),
                    ),
                    directory: std::path::Path::new("/").into(),
                    title: "format.md".into(),
                    preferences: Arc::new(PreferencesData {
                        code_tab_size: width.to_string(),
                        ..Default::default()
                    }),
                    diagrams: RenderQueue::new(),
                    math: MathQueue::new(),
                    codes: pecan_format::CodeSnapshot::default(),
                    resources: None,
                };
                let blocks = Builder::default()
                    .build(&snapshot, crate::network::client())
                    .await
                    .unwrap();
                let mut actual = Vec::new();
                code_text(&blocks, &mut actual);
                assert_eq!(actual, super::super::expected_code(width));
                assert_eq!(snapshot.document.source(), super::super::FORMAT_FIXTURE);
            }
        });
    }

    #[test]
    fn code_aliases_share_their_vector_mark_without_changing_source() {
        let mut builder = Builder::default();
        let mut icons = super::super::language_icons::Icons::default();
        for language in ["json", "JSONC", "text"] {
            builder
                .code("{\"value\": 1}", language, &mut icons)
                .unwrap();
        }
        let trees: Vec<_> = builder
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Code {
                    icon: Some(Object::Vector(Vector::Tree(tree))),
                    ..
                } => Some(tree),
                _ => None,
            })
            .collect();
        assert_eq!(trees.len(), 3, "plain text uses the generic code mark");
        assert!(
            !Arc::ptr_eq(trees[0], trees[2]),
            "generic mark differs from JSON"
        );
        assert!(
            Arc::ptr_eq(trees[0], trees[1]),
            "code aliases reuse vector geometry"
        );
    }

    #[test]
    fn note_type_remains_after_title_prefix_is_consumed() {
        futures_lite::future::block_on(async {
            let source = "> [!TIP] 建议\n> 正文。\n\n> [!IMPORTANT] 重点\n> 另一段。";
            let snapshot = Snapshot {
                document: Arc::new(Document::parse(source.to_owned()).unwrap()),
                directory: std::env::temp_dir().into(),
                title: "colors.md".into(),
                preferences: Arc::new(PreferencesData::default()),
                diagrams: RenderQueue::new(),
                math: MathQueue::new(),
                codes: pecan_format::CodeSnapshot::default(),
                resources: None,
            };
            let blocks = Builder::default()
                .build(&snapshot, crate::network::client())
                .await
                .unwrap();
            let kinds: Vec<_> = blocks
                .iter()
                .filter_map(|b| {
                    if let Block::Note { kind, .. } = b {
                        Some(*kind)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(kinds, [Some(AlertKind::Tip), Some(AlertKind::Important)]);
        });
    }

    #[test]
    fn typed_and_generic_notes_keep_editable_text_and_share_vector_icons() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let snapshot = Snapshot {
                document: Arc::new(
                    Document::parse(super::super::document::NOTE_FIXTURE.to_owned()).unwrap(),
                ),
                directory: directory.path().into(),
                title: "notes.md".into(),
                preferences: Arc::new(PreferencesData::default()),
                diagrams: RenderQueue::new(),
                math: MathQueue::new(),
                codes: pecan_format::CodeSnapshot::default(),
                resources: None,
            };
            let blocks = Builder::default()
                .build(&snapshot, crate::network::client())
                .await
                .unwrap();
            let notes: Vec<_> = blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Note { blocks, .. } => Some(blocks),
                    _ => None,
                })
                .collect();
            assert_eq!(notes.len(), 3, "each note has one container");
            let notes: Vec<_> = notes
                .iter()
                .flat_map(|blocks| blocks.iter())
                .filter_map(|block| match block {
                    Block::Paragraph(para) => Some(para),
                    _ => None,
                })
                .collect();
            assert_eq!(
                notes.len(),
                5,
                "typed titles and body paragraphs remain highlighted"
            );
            assert_eq!(
                notes.iter().map(|para| para.keep_next).collect::<Vec<_>>(),
                [true, false, true, false, false],
                "only semantic titles stay with the following content"
            );
            let icons: Vec<_> = notes
                .iter()
                .flat_map(|para| &para.inlines)
                .filter_map(|inline| match inline {
                    Inline::Object {
                        object: Object::Vector(Vector::Tree(tree)),
                        width,
                        height,
                        ..
                    } => {
                        assert_eq!((*width, *height), (12., 12.));
                        Some(tree)
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(icons.len(), 3, "each note gets one vector mark");
            assert!(
                Arc::ptr_eq(icons[0], icons[1]),
                "repeated typed notes share parsed geometry"
            );
            let leaves: Vec<_> = blocks
                .iter()
                .flat_map(|block| match block {
                    Block::Note { blocks, .. } => blocks.iter().collect::<Vec<_>>(),
                    _ => vec![block],
                })
                .collect();
            let text: String = leaves
                .iter()
                .flat_map(|block| match block {
                    Block::Paragraph(para) => para.inlines.as_slice(),
                    Block::Code { inlines, .. } => inlines.as_slice(),
                    _ => &[],
                })
                .filter_map(|inline| match inline {
                    Inline::Text(text, _) => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            for expected in [
                "💡 自定义",
                "保留 ℹ️ 正文。",
                "💡 提示正文",
                "⚠️ 💡 ℹ️ ❗ 🚫 ❌",
            ] {
                assert!(
                    text.contains(expected),
                    "PDF text lacks original content {expected}"
                );
            }
            assert!(
                !text.contains("ℹ️ 💡 自定义"),
                "generated prefix was replaced by vector icon"
            );
        });
    }
    #[test]
    fn cached_picture_survives_without_refetching_its_source() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("pixel.png");
            image::DynamicImage::new_rgb8(3, 2).save(&path).unwrap();
            let mut images = Images::new(directory.path(), crate::network::client());
            let mut builder = Builder::default();
            let renderer = Renderer::new();
            let (first, width, height) = builder
                .picture(&mut images, &renderer, "pixel.png")
                .await
                .unwrap();
            std::fs::remove_file(path).unwrap();
            let (cached, cached_width, cached_height) = builder
                .picture(&mut images, &renderer, "pixel.png")
                .await
                .unwrap();
            let (Object::Raster(first), Object::Raster(cached)) = (first, cached) else {
                panic!("PNG must remain raster images");
            };
            assert_eq!(first, cached);
            assert_eq!((width, height), (cached_width, cached_height));
            assert_eq!(
                builder.pictures.len(),
                1,
                "repeated references share one object"
            );
        });
    }

    #[test]
    fn repeated_markdown_and_html_images_share_the_resource_budget() {
        futures_lite::future::block_on(async {
            let directory = tempfile::tempdir().unwrap();
            let image = image::DynamicImage::new_rgb8(1, 1);
            image.save(directory.path().join("pixel.png")).unwrap();
            let source = (0..2050)
                .map(|index| {
                    if index % 2 == 0 {
                        "![pixel](pixel.png)\n\n"
                    } else {
                        "<img src=\"pixel.png\">\n\n"
                    }
                })
                .collect::<String>();
            let snapshot = Snapshot {
                document: Arc::new(Document::parse(source).unwrap()),
                directory: directory.path().into(),
                title: "repeated.md".into(),
                preferences: Arc::new(PreferencesData::default()),
                diagrams: RenderQueue::new(),
                math: MathQueue::new(),
                codes: pecan_format::CodeSnapshot::default(),
                resources: None,
            };
            let blocks = Builder::default()
                .build(&snapshot, crate::network::client())
                .await
                .unwrap();
            assert_eq!(
                blocks
                    .iter()
                    .filter_map(|block| match block {
                        Block::Paragraph(para) => Some(&para.inlines),
                        _ => None,
                    })
                    .flatten()
                    .filter(|inline| matches!(inline, Inline::Object { .. }))
                    .count(),
                2050,
                "every reference must survive without spending the unique-image budget again"
            );
        });
    }
}
