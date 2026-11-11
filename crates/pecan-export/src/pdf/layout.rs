//! Pagination into a display list, then painting with krilla. Mirrors the print style
//! of the HTML export: A4, 11 pt body, quiet gray rules and boxes.
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use krilla::{
    Document, SerializeSettings,
    action::LinkAction,
    annotation::{LinkAnnotation, Target},
    color::rgb,
    destination::XyzDestination,
    geom::{Path, PathBuilder, Point, Rect, Size, Transform},
    metadata::Metadata,
    num::NormalizedF32,
    outline::{Outline, OutlineNode},
    page::PageSettings,
    paint::{Fill, Stroke},
    surface::Surface,
};
use krilla_svg::{SurfaceExt as _, SvgSettings};
use pulldown_cmark::Alignment;

use super::{
    Block, Object, Para, ParaKind, Row,
    text::{self, Fonts, Inline, Item, Line, Paragraph, Role, Style},
};

pub(super) const PAGE: (f32, f32) = (595.28, 841.89);
const MARGIN_X: f32 = 36.;
const MARGIN_Y: f32 = 40.;
const TOP: f32 = MARGIN_Y;
const BOTTOM: f32 = PAGE.1 - MARGIN_Y;
pub(super) const WIDTH: f32 = PAGE.0 - 2. * MARGIN_X;
pub(super) const BODY_SIZE: f32 = 11.;
pub(super) const CODE_SIZE: f32 = 9.;
/// Table text, like the reader's `.93em`.
pub(super) const CELL_SIZE: f32 = BODY_SIZE * 0.93;
const MAX_PAGES: usize = 5000;

pub(super) const INK: [u8; 3] = [0x24; 3];
pub(super) const MUTED: [u8; 3] = [0x74; 3];
pub(super) const LINK: [u8; 3] = [0x36, 0x5d, 0x8b];
const BORDER: [u8; 3] = [0xde; 3];
const CODE_FILL: [u8; 3] = color_bytes(pecan_assets::LIGHT_CODE_BODY);
const CODE_HEADER_FILL: [u8; 3] = color_bytes(pecan_assets::LIGHT_CODE_HEADER);
const CAPSULE_FILL: [u8; 3] = [0xf3; 3];
const HEAD_FILL: [u8; 3] = [0xf5; 3];
const STRIPE_FILL: [u8; 3] = [0xfb; 3];
const RULE: [u8; 3] = [0xe1; 3];
const QUOTE_BAR: [u8; 3] = [0xd4; 3];
const QUOTE_FILL: [u8; 3] = [0xf7; 3];
const QUOTE_PAD_Y: f32 = 6.;
#[cfg(test)]
const HIGHLIGHT_FILL: [u8; 3] = [0xff, 0xf6, 0xe8];
const FIGURE_BORDER: [u8; 3] = [0xe5; 3];

const CODE_PAD: (f32, f32) = (12., 8.);
/// Compact PDF language band, in points; inline logos retain their original size.
const CODE_HEADER_HEIGHT: f32 = 26.;
/// Preserve vertical padding when a long language label wraps onto multiple lines.
const CODE_HEADER_PAD: f32 = 6.;
const CELL_PAD: (f32, f32) = (6.75, 4.5);
const FIGURE_PAD: f32 = 12.;
const NOTE_PAD: (f32, f32) = (12., 9.);

enum Draw {
    Run {
        x: f32,
        baseline: f32,
        run: text::Run,
        style: Style,
    },
    Rect {
        rect: (f32, f32, f32, f32),
        fill: Option<[u8; 3]>,
        stroke: Option<([u8; 3], f32)>,
        radius: f32,
    },
    Object {
        object: Object,
        rect: (f32, f32, f32, f32),
    },
}

struct Link {
    rect: (f32, f32, f32, f32),
    url: Arc<str>,
}

struct Page {
    draws: Vec<Draw>,
    links: Vec<Link>,
}

struct Heading {
    level: u8,
    title: String,
    page: usize,
    y: f32,
}

struct Pager<'a> {
    fonts: &'a mut Fonts,
    pages: Vec<Page>,
    y: f32,
    margin: f32,
    headings: Vec<Heading>,
    line_height: f32,
    inset: f32,
    reserved: f32,
    suppress_space: bool,
}

pub(super) fn render(
    blocks: &[Block],
    fonts: &mut Fonts,
    line_height: f32,
    title: &str,
) -> Result<Vec<u8>> {
    let mut pager = Pager {
        fonts,
        pages: vec![Page {
            draws: Vec::new(),
            links: Vec::new(),
        }],
        y: TOP,
        margin: 0.,
        headings: Vec::new(),
        line_height,
        inset: 0.,
        reserved: 0.,
        suppress_space: false,
    };
    for (index, block) in blocks.iter().enumerate() {
        pager.block(block, blocks.get(index + 1))?;
    }
    pager.paint(title)
}

impl Pager<'_> {
    fn left(&self) -> f32 {
        MARGIN_X + self.inset
    }
    fn width(&self) -> f32 {
        WIDTH - 2. * self.inset
    }
    fn top(&self) -> f32 {
        TOP + self.reserved
    }
    fn bottom(&self) -> f32 {
        BOTTOM - self.reserved
    }

    fn note(
        &mut self,
        blocks: &[Block],
        indent: f32,
        kind: Option<pecan_document::AlertKind>,
    ) -> Result<()> {
        if blocks.is_empty() {
            return Ok(());
        }
        let (fill, edge) = pecan_assets::notes::export_colors(kind);
        self.space(BODY_SIZE * 0.75);
        let first_page = self.pages.len() - 1;
        let first_draw = self.page().draws.len();
        let top = self.y;
        let left = self.left() + indent;
        let width = self.width() - indent;
        let outer_reserved = self.reserved;
        ensure!(width > NOTE_PAD.0 * 2. + 1., "PDF 提示块嵌套过深");
        self.inset += NOTE_PAD.0;
        self.reserved += NOTE_PAD.1;
        self.y += NOTE_PAD.1;
        self.suppress_space = true;
        for (index, block) in blocks.iter().enumerate() {
            self.block(block, blocks.get(index + 1))?;
        }
        self.y += NOTE_PAD.1;
        let last_page = self.pages.len() - 1;
        for page in first_page..=last_page {
            let draw = if page == first_page { first_draw } else { 0 };
            // A title moved to the next page must not leave an empty box behind.
            if self.pages[page].draws.len() == draw {
                continue;
            }
            let top = if page == first_page {
                top
            } else {
                TOP + outer_reserved
            };
            let bottom = if page == last_page {
                self.y
            } else {
                BOTTOM - outer_reserved
            };
            self.pages[page].draws.insert(
                draw,
                Draw::Rect {
                    rect: (left, top, width, bottom - top),
                    fill: Some(fill),
                    stroke: Some((edge, 0.6)),
                    radius: 4.5,
                },
            );
        }
        self.inset -= NOTE_PAD.0;
        self.reserved = outer_reserved;
        self.suppress_space = false;
        self.margin = BODY_SIZE * 0.75;
        Ok(())
    }

    fn page(&mut self) -> &mut Page {
        self.pages.last_mut().expect("there is always a page")
    }
    fn new_page(&mut self) -> Result<()> {
        ensure!(
            self.pages.len() < MAX_PAGES,
            "PDF 超过 {MAX_PAGES} 页导出预算"
        );
        self.pages.push(Page {
            draws: Vec::new(),
            links: Vec::new(),
        });
        self.y = self.top();
        self.margin = 0.;
        Ok(())
    }
    /// CSS margin collapsing: adjacent margins use the larger one, none at a page top.
    fn space(&mut self, before: f32) {
        if std::mem::take(&mut self.suppress_space) {
            self.margin = 0.;
            return;
        }
        if self.y > self.top() {
            self.y += self.margin.max(before);
        }
        self.margin = 0.;
    }
    fn fits(&self, height: f32) -> bool {
        self.y + height <= self.bottom() + 0.01
    }
    /// Move to a new page unless already at the top of one.
    fn need(&mut self, height: f32) -> Result<()> {
        if !self.fits(height) && self.y > self.top() + 0.01 {
            self.new_page()?;
        }
        Ok(())
    }

    fn block(&mut self, block: &Block, next: Option<&Block>) -> Result<()> {
        match block {
            Block::Paragraph(para) => self.paragraph(para, next),
            Block::Note {
                blocks,
                indent,
                kind,
            } => self.note(blocks, *indent, *kind),
            Block::Code {
                inlines,
                label,
                icon,
                indent,
            } => self.code(inlines, label, icon.as_ref(), *indent),
            Block::Table {
                aligns,
                rows,
                indent,
            } => self.table(aligns, rows, *indent),
            Block::Figure {
                object,
                width,
                height,
                indent,
                framed,
            } => self.figure(object, *width, *height, *indent, *framed),
            Block::Rule { indent } => {
                self.space(BODY_SIZE * 2.);
                self.need(1.)?;
                let y = self.y;
                let left = self.left() + indent;
                let width = self.width() - indent;
                self.page().draws.push(Draw::Rect {
                    rect: (left, y, width, 0.75),
                    fill: Some(RULE),
                    stroke: None,
                    radius: 0.,
                });
                self.y += 0.75;
                self.margin = BODY_SIZE * 2.;
                Ok(())
            }
        }
    }

    fn paragraph(&mut self, para: &Para, next: Option<&Block>) -> Result<()> {
        let (size, before, after) = match para.kind {
            ParaKind::Heading(level) => {
                let size = BODY_SIZE * heading_scale(level);
                (
                    size,
                    if level == 1 { size * 0.9 } else { size * 1.6 },
                    size * 0.65,
                )
            }
            ParaKind::Body => (BODY_SIZE, BODY_SIZE * 0.75, BODY_SIZE * 0.75),
            ParaKind::Item => (BODY_SIZE, BODY_SIZE * 0.25, BODY_SIZE * 0.25),
            ParaKind::Footnote => (BODY_SIZE * 0.88, BODY_SIZE * 0.4, BODY_SIZE * 0.4),
        };
        let (before, after) = if self.inset > 0. {
            (before.min(BODY_SIZE * 0.5), after.min(BODY_SIZE * 0.5))
        } else {
            (before, after)
        };
        let left = self.left() + para.indent;
        let width = self.width() - para.indent;
        let padding = if para.quote > 0 { QUOTE_PAD_Y } else { 0. };
        let line_height = if matches!(para.kind, ParaKind::Heading(_)) || para.keep_next {
            1.35
        } else {
            self.line_height
        };
        let paragraph = Paragraph {
            inlines: &para.inlines,
            role: Role::Body,
            size,
            width,
            max_height: self.bottom() - self.top() - 2. * padding,
            line_height,
            justify: para.align == Align::Justify,
            preserve: false,
        };
        let lines = text::layout(self.fonts, &paragraph);
        if lines.is_empty() {
            return Ok(());
        }
        self.space(before);
        if para.keep_next && next.is_some() {
            let following = match next {
                Some(Block::Paragraph(next)) => {
                    let following = Paragraph {
                        inlines: &next.inlines,
                        width: self.width() - next.indent,
                        size: BODY_SIZE,
                        line_height: self.line_height,
                        ..paragraph
                    };
                    text::layout(self.fonts, &following)
                        .iter()
                        .take(2)
                        .map(Line::height)
                        .sum()
                }
                _ => BODY_SIZE * self.line_height * 2.,
            };
            let height: f32 = lines.iter().map(Line::height).sum();
            self.need(height + 2. * padding + after + following)?;
        } else if self.inset > 0. {
            let first_lines: f32 = lines.iter().take(2).map(Line::height).sum();
            self.need(first_lines + 2. * padding)?;
        }
        if let ParaKind::Heading(level) = para.kind {
            // Keep a heading with the start of what follows it.
            let following = match next {
                Some(Block::Code {
                    inlines,
                    label,
                    icon,
                    indent,
                }) => {
                    after.max(BODY_SIZE * 1.1) - after
                        + self.code_intro_height(inlines, label, icon.as_ref(), *indent)
                }
                Some(Block::Figure { height, .. }) => height.min(self.bottom() - self.top()) * 0.5,
                Some(_) => BODY_SIZE * self.line_height * 3.,
                None => 0.,
            };
            let height: f32 = lines.iter().map(Line::height).sum();
            self.need(height + 2. * padding + after + following)?;
            self.headings.push(Heading {
                level,
                title: plain(&para.inlines),
                page: self.pages.len() - 1,
                y: self.y,
            });
        }
        if para.quote > 0 {
            self.need(lines[0].height() + 2. * padding)?;
        }
        // Quote bars and backgrounds include inner padding on every occupied page.
        let mut segment_top = self.y;
        let mut segment_draw = self.page().draws.len();
        let mut segments = Vec::new();
        self.y += padding;
        for (index, line) in lines.iter().enumerate() {
            let required = if self.inset > 0. && lines.len() - index == 2 {
                lines[index..].iter().map(Line::height).sum()
            } else {
                line.height()
            };
            let required = if required <= self.bottom() - self.top() - 2. * padding {
                required
            } else {
                line.height()
            };
            if !self.fits(required + padding) && self.y > self.top() + padding + 0.01 {
                segments.push((
                    self.pages.len() - 1,
                    segment_draw,
                    segment_top,
                    self.y + padding,
                ));
                self.new_page()?;
                segment_top = self.top();
                segment_draw = 0;
                self.y += padding;
            }
            ensure!(
                self.fits(line.height() + padding),
                "PDF 文本行高度超过单页可用高度"
            );
            if index == 0
                && let Some(marker) = &para.marker
            {
                self.marker(marker, left, line);
            }
            self.line(&paragraph, line, left);
        }
        self.y += padding;
        segments.push((self.pages.len() - 1, segment_draw, segment_top, self.y));
        if para.quote > 0 {
            let origin = self.left() + para.indent - f32::from(para.quote) * QUOTE_STEP;
            let width = self.width() - (origin - self.left());
            for &(page, draw, top, bottom) in &segments {
                self.pages[page].draws.insert(
                    draw,
                    Draw::Rect {
                        rect: (origin, top, width, (bottom - top).max(0.)),
                        fill: Some(QUOTE_FILL),
                        stroke: None,
                        radius: 0.,
                    },
                );
            }
        }
        for depth in 0..para.quote {
            let x = self.left() + para.indent - (para.quote - depth) as f32 * QUOTE_STEP;
            for &(page, _, top, bottom) in &segments {
                self.pages[page].draws.push(Draw::Rect {
                    rect: (x, top, 2.25, bottom - top),
                    fill: Some(QUOTE_BAR),
                    stroke: None,
                    radius: 0.,
                });
            }
        }
        self.margin = after;
        Ok(())
    }

    fn marker(&mut self, marker: &Inline, left: f32, line: &Line) {
        let inlines = std::slice::from_ref(marker);
        let paragraph = Paragraph {
            inlines,
            role: Role::Body,
            size: BODY_SIZE,
            width: LIST_STEP,
            max_height: self.bottom() - self.top(),
            line_height: self.line_height,
            justify: false,
            preserve: false,
        };
        if let Some(marker_line) = text::layout(self.fonts, &paragraph).into_iter().next() {
            let x = left - marker_line.width - 5.;
            let baseline = self.y + line.ascent;
            for placed in marker_line.items {
                if let Item::Run(run) = placed.item {
                    let style = text_style(inlines, run.inline);
                    self.page().draws.push(Draw::Run {
                        x: x + placed.x,
                        baseline,
                        run,
                        style,
                    });
                }
            }
        }
    }

    /// Paint one line at the current position and advance.
    fn line(&mut self, paragraph: &Paragraph<'_>, line: &Line, left: f32) {
        let baseline = self.y + line.ascent;
        let mut capsule: Option<(usize, f32, f32)> = None;
        let mut capsules = Vec::new();
        let mut runs = Vec::new();
        for placed in &line.items {
            match &placed.item {
                Item::Run(run) => {
                    let style = text_style(paragraph.inlines, run.inline);
                    let x = left + placed.x;
                    if let Some(id) = style.capsule {
                        match &mut capsule {
                            Some((previous, _, end)) if *previous == id => *end = x + run.width,
                            _ => {
                                if let Some(done) = capsule.take() {
                                    capsules.push(done);
                                }
                                capsule = Some((id, x, x + run.width));
                            }
                        }
                    } else if let Some(done) = capsule.take() {
                        capsules.push(done);
                    }
                    if let Some(link) = &style.link {
                        let top = baseline - style.size;
                        self.page().links.push(Link {
                            rect: (x, top, run.width, style.size * 1.3),
                            url: link.clone(),
                        });
                        runs.push(Draw::Rect {
                            rect: (x, baseline + style.size * 0.15, run.width, 0.6),
                            fill: Some(style.color),
                            stroke: None,
                            radius: 0.,
                        });
                    }
                    if style.strike {
                        runs.push(Draw::Rect {
                            rect: (x, baseline - style.size * 0.3, run.width, 0.6),
                            fill: Some(style.color),
                            stroke: None,
                            radius: 0.,
                        });
                    }
                    runs.push(Draw::Run {
                        x,
                        baseline: baseline - style.raise(),
                        run: run.clone(),
                        style,
                    });
                }
                Item::Object(index) => {
                    if let Some(done) = capsule.take() {
                        capsules.push(done);
                    }
                    if let Some(Inline::Object { object, link, .. }) = paragraph.inlines.get(*index)
                    {
                        let (width, height, _) = paragraph.object_size(*index);
                        let drop = line.object_drop(paragraph, *index);
                        let rect = (left + placed.x, baseline + drop - height, width, height);
                        if let Some(link) = link {
                            self.page().links.push(Link {
                                rect,
                                url: link.clone(),
                            });
                        }
                        runs.push(Draw::Object {
                            object: object.clone(),
                            rect,
                        });
                    }
                }
            }
        }
        if let Some(done) = capsule {
            capsules.push(done);
        }
        let size = paragraph.size;
        for (_, start, end) in capsules {
            let (ascent, descent) = (size * 0.9, size * 0.3);
            self.page().draws.push(Draw::Rect {
                rect: (
                    start - text::CAPSULE_PAD + 0.25,
                    baseline - ascent,
                    end - start + 2. * text::CAPSULE_PAD - 0.5,
                    ascent + descent,
                ),
                fill: Some(CAPSULE_FILL),
                stroke: Some((BORDER, 0.5)),
                radius: 2.,
            });
        }
        self.page().draws.extend(runs);
        self.y += line.height();
    }

    /// Reserve exactly the lead band the code renderer needs, without laying out a whole fence twice.
    fn code_intro_height(
        &mut self,
        inlines: &[Inline],
        label: &str,
        icon: Option<&Object>,
        indent: f32,
    ) -> f32 {
        let width = self.width() - indent - 2. * CODE_PAD.0;
        let first_break = inlines
            .iter()
            .position(|inline| matches!(inline, Inline::Break))
            .unwrap_or(inlines.len());
        let paragraph = Paragraph {
            inlines: &inlines[..first_break],
            role: Role::Code,
            size: CODE_SIZE,
            width,
            max_height: self.bottom() - self.top() - 2. * CODE_PAD.1,
            line_height: 1.6,
            justify: false,
            preserve: true,
        };
        let first = text::layout(self.fonts, &paragraph)
            .first()
            .map_or(CODE_SIZE * 1.6, Line::height);
        let label_inline = Self::code_label(label, icon);
        let label_height: f32 = text::layout(
            self.fonts,
            &Paragraph {
                inlines: &label_inline,
                role: Role::Body,
                size: 7.5,
                line_height: 1.4,
                preserve: false,
                ..paragraph
            },
        )
        .iter()
        .map(Line::height)
        .sum();
        CODE_PAD.1 * 2. + (label_height + 2. * CODE_HEADER_PAD).max(CODE_HEADER_HEIGHT) + first * 2.
    }

    fn code(
        &mut self,
        inlines: &[Inline],
        label: &str,
        icon: Option<&Object>,
        indent: f32,
    ) -> Result<()> {
        let left = self.left() + indent;
        let width = self.width() - indent;
        let paragraph = Paragraph {
            inlines,
            role: Role::Code,
            size: CODE_SIZE,
            width: width - 2. * CODE_PAD.0,
            max_height: self.bottom() - self.top() - 2. * CODE_PAD.1,
            line_height: 1.6,
            justify: false,
            preserve: true,
        };
        let lines = text::layout(self.fonts, &paragraph);
        let label_inline = Self::code_label(label, icon);
        let label_lines = text::layout(
            self.fonts,
            &Paragraph {
                inlines: &label_inline,
                role: Role::Body,
                size: 7.5,
                width: paragraph.width,
                max_height: self.bottom() - self.top() - 2. * CODE_PAD.1,
                line_height: 1.4,
                justify: false,
                preserve: false,
            },
        );
        self.space(BODY_SIZE * 1.1);
        let first = lines.first().map_or(CODE_SIZE * 1.6, Line::height);
        let label_height: f32 = label_lines.iter().map(Line::height).sum();
        let header_height = (label_height + 2. * CODE_HEADER_PAD).max(CODE_HEADER_HEIGHT);
        let header_padding = (header_height - label_height) / 2.;
        self.need(CODE_PAD.1 * 2. + header_height + first * 2.)?;
        let mut start = self.y;
        let mut draw_index = self.page().draws.len();
        let label_paragraph = Paragraph {
            inlines: &label_inline,
            ..paragraph
        };
        self.code_header(&label_paragraph, &label_lines, left, header_padding);
        self.y += CODE_PAD.1;
        for line in &lines {
            if !self.fits(line.height() + CODE_PAD.1) && self.y > self.top() + 0.01 {
                let bottom = self.y + CODE_PAD.1 * 0.5;
                self.code_box(
                    draw_index,
                    (left, start, width, bottom - start),
                    header_height,
                );
                self.new_page()?;
                start = self.y;
                draw_index = 0;
                self.code_header(&label_paragraph, &label_lines, left, header_padding);
                self.y += CODE_PAD.1;
            }
            ensure!(
                self.fits(line.height() + CODE_PAD.1),
                "PDF 代码行高度超过单页可用高度"
            );
            self.line(&paragraph, line, left + CODE_PAD.0);
        }
        self.y += CODE_PAD.1;
        let bottom = self.y;
        self.code_box(
            draw_index,
            (left, start, width, bottom - start),
            header_height,
        );
        self.margin = BODY_SIZE * 1.1;
        Ok(())
    }
    fn code_header(&mut self, paragraph: &Paragraph<'_>, lines: &[Line], left: f32, padding: f32) {
        self.y += padding;
        for line in lines {
            let x = left + CODE_PAD.0 + (paragraph.width - line.width).max(0.);
            self.line(paragraph, line, x);
        }
        self.y += padding;
    }
    fn code_box(&mut self, index: usize, rect: (f32, f32, f32, f32), header_height: f32) {
        self.page().draws.insert(
            index,
            Draw::Rect {
                rect,
                fill: Some(CODE_FILL),
                stroke: Some((BORDER, 0.75)),
                radius: 3.,
            },
        );
        // Keep the language fill inside the outer stroke, including on continuation pages.
        self.page().draws.insert(
            index + 1,
            Draw::Rect {
                rect: (
                    rect.0 + 0.75,
                    rect.1 + 0.75,
                    rect.2 - 1.5,
                    header_height - 0.75,
                ),
                fill: Some(CODE_HEADER_FILL),
                stroke: None,
                radius: 2.,
            },
        );
    }

    fn figure(
        &mut self,
        object: &Object,
        width: f32,
        height: f32,
        indent: f32,
        framed: bool,
    ) -> Result<()> {
        let pad = if framed { FIGURE_PAD } else { 0. };
        let available = self.width() - indent - 2. * pad;
        let scale = (available / width.max(f32::EPSILON))
            .min((self.bottom() - self.top() - 2. * pad) / height.max(f32::EPSILON))
            .min(1.);
        let (width, height) = (width * scale, height * scale);
        self.space(BODY_SIZE * 1.3);
        self.need(height + 2. * pad)?;
        let left = self.left() + indent;
        let top = self.y;
        if framed {
            let width = self.width() - indent;
            self.page().draws.push(Draw::Rect {
                rect: (left, top, width, height + 2. * pad),
                fill: None,
                stroke: Some((FIGURE_BORDER, 0.75)),
                radius: 3.,
            });
        }
        let x = left + pad + (available - width) / 2.;
        self.page().draws.push(Draw::Object {
            object: object.clone(),
            rect: (x, top + pad, width, height),
        });
        self.y = top + height + 2. * pad;
        self.margin = BODY_SIZE * 1.3;
        Ok(())
    }

    fn table(&mut self, aligns: &[Alignment], rows: &[Row], indent: f32) -> Result<()> {
        let columns = aligns
            .len()
            .max(rows.iter().map(|row| row.cells.len()).max().unwrap_or(1));
        let available = self.width() - indent;
        let cell_height = self.bottom() - self.top() - 2. * CELL_PAD.1;
        // Automatic layout: minimum (longest word) and natural (single line) widths.
        let mut minimum = vec![0f32; columns];
        let mut natural = vec![0f32; columns];
        for row in rows {
            for (column, paras) in row.cells.iter().enumerate() {
                for para in paras {
                    let (low, high) =
                        text::measure(self.fonts, &cell_paragraph(para, available, cell_height));
                    let edges = para.indent + 2. * CELL_PAD.0;
                    minimum[column] = minimum[column].max(low + edges);
                    natural[column] = natural[column].max(high + edges);
                }
            }
        }
        let (low, high): (f32, f32) = (minimum.iter().sum(), natural.iter().sum());
        let widths: Vec<f32> = if high <= available {
            // Like `width: 100%`, spread the remaining width by natural size.
            natural
                .iter()
                .map(|width| width + (available - high) * width / high.max(f32::EPSILON))
                .collect()
        } else if low >= available {
            minimum
                .iter()
                .map(|width| width * available / low)
                .collect()
        } else {
            minimum
                .iter()
                .zip(&natural)
                .map(|(min, max)| {
                    min + (max - min) * (available - low) / (high - low).max(f32::EPSILON)
                })
                .collect()
        };
        let mut laid: Vec<Vec<Vec<CellLine>>> = rows
            .iter()
            .map(|row| {
                if row.head {
                    self.cells(row, &widths, cell_height)
                } else {
                    Vec::new()
                }
            })
            .collect();
        let head: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.head)
            .map(|(index, _)| index)
            .collect();
        let header_height: f32 = head.iter().map(|&index| row_height(&laid[index])).sum();
        let body_height = self.bottom() - self.top() - header_height;
        let body_line_height = body_height - 2. * CELL_PAD.1;
        ensure!(
            header_height <= self.bottom() - self.top()
                && (rows.iter().all(|row| row.head) || body_line_height >= CELL_SIZE * 1.45),
            "PDF 表格表头过高，无法为正文保留一行空间"
        );
        // Repeat headers consume part of every page. Scale body objects against
        // that same remainder in layout and paint so other columns stay beside them.
        for (index, row) in rows.iter().enumerate().filter(|(_, row)| !row.head) {
            laid[index] = self.cells(row, &widths, body_line_height);
            ensure!(
                laid[index]
                    .iter()
                    .flatten()
                    .all(|line| line.line.height() <= body_line_height + 0.01),
                "PDF 表格行无法放入表头下方的单页可用空间"
            );
        }
        self.space(BODY_SIZE * 1.1);
        if !head.is_empty() {
            let first_body = rows.iter().position(|row| !row.head);
            let following =
                first_body.map_or(0., |index| row_height(&laid[index]).min(body_height));
            self.need(header_height + following)?;
        }
        let mut body_index = 0;
        for (index, row) in rows.iter().enumerate() {
            let shade = if row.head {
                Some(HEAD_FILL)
            } else {
                body_index += 1;
                (body_index % 2 == 0).then_some(STRIPE_FILL)
            };
            let height = row_height(&laid[index]);
            if !self.fits(height)
                && self.y > self.top() + 0.01
                && height <= self.bottom() - self.top()
            {
                self.new_page()?;
                if !row.head {
                    for &head in &head {
                        self.row(
                            &rows[head],
                            &laid[head],
                            &widths,
                            aligns,
                            indent,
                            Some(HEAD_FILL),
                            cell_height,
                        )?;
                    }
                }
            }
            let max_height = if row.head {
                cell_height
            } else {
                body_line_height
            };
            self.row(
                row,
                &laid[index],
                &widths,
                aligns,
                indent,
                shade,
                max_height,
            )?;
        }
        self.margin = BODY_SIZE * 1.1;
        Ok(())
    }

    /// Break every cell of a row into lines: its paragraphs one below another.
    fn cells(&mut self, row: &Row, widths: &[f32], max_height: f32) -> Vec<Vec<CellLine>> {
        let mut cells = Vec::with_capacity(row.cells.len());
        for (paras, width) in row.cells.iter().zip(widths) {
            let mut lines = Vec::new();
            let mut above = 0f32;
            for (index, para) in paras.iter().enumerate() {
                // List items sit closer together than paragraphs.
                let space = match para.kind {
                    ParaKind::Item => CELL_SIZE * 0.2,
                    _ => CELL_SIZE * 0.5,
                };
                let inner = width - 2. * CELL_PAD.0;
                let laid = text::layout(self.fonts, &cell_paragraph(para, inner, max_height));
                for (number, line) in laid.into_iter().enumerate() {
                    lines.push(CellLine {
                        gap: if number == 0 && index > 0 {
                            above.max(space)
                        } else {
                            0.
                        },
                        first: number == 0,
                        para: index,
                        line,
                    });
                }
                above = space;
            }
            cells.push(lines);
        }
        cells
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        &mut self,
        row: &Row,
        cells: &[Vec<CellLine>],
        widths: &[f32],
        aligns: &[Alignment],
        indent: f32,
        shade: Option<[u8; 3]>,
        max_height: f32,
    ) -> Result<()> {
        // Rows taller than a page continue line by line on following pages.
        let mut next = vec![0usize; cells.len()];
        loop {
            let room = self.bottom() - self.y - 2. * CELL_PAD.1;
            let mut slice = vec![0usize; cells.len()];
            let mut content: f32 = 0.;
            for (column, lines) in cells.iter().enumerate() {
                let start = next[column];
                let mut height = 0.;
                let mut end = start;
                while end < lines.len() {
                    // The part of a cell on this page starts at the padding.
                    let gap = if end > start { lines[end].gap } else { 0. };
                    let step = lines[end].line.height() + gap;
                    if height + step > room {
                        break;
                    }
                    height += step;
                    end += 1;
                }
                slice[column] = end;
                content = content.max(height);
            }
            let progress = slice.iter().zip(&next).any(|(end, start)| end > start);
            let unfinished = cells
                .iter()
                .zip(&next)
                .any(|(lines, start)| *start < lines.len());
            if !progress && unfinished {
                ensure!(
                    self.y > self.top() + 0.01,
                    "PDF 表格行高度超过单页可用高度，无法继续分页"
                );
                self.new_page()?;
                continue;
            }
            let height = content + 2. * CELL_PAD.1;
            let top = self.y;
            let mut x = self.left() + indent;
            for (column, width) in widths.iter().enumerate() {
                self.page().draws.push(Draw::Rect {
                    rect: (x, top, *width, height),
                    fill: shade,
                    stroke: Some((BORDER, 0.5)),
                    radius: 0.,
                });
                if let (Some(lines), Some(paras)) = (cells.get(column), row.cells.get(column)) {
                    let inner = width - 2. * CELL_PAD.0;
                    let lines = &lines[next[column]..slice[column]];
                    // Vertically centered, like the reader's tables.
                    self.y = top + CELL_PAD.1 + (content - lines_height(lines)) / 2.;
                    for (index, placed) in lines.iter().enumerate() {
                        let Some(para) = paras.get(placed.para) else {
                            continue;
                        };
                        if index > 0 {
                            self.y += placed.gap;
                        }
                        let room = inner - para.indent;
                        let offset = match aligns.get(column) {
                            Some(Alignment::Center) => (room - placed.line.width) / 2.,
                            Some(Alignment::Right) => room - placed.line.width,
                            _ => 0.,
                        };
                        let left = x + CELL_PAD.0 + para.indent + offset.max(0.);
                        if placed.first
                            && let Some(marker) = &para.marker
                        {
                            self.marker(marker, left, &placed.line);
                        }
                        self.line(&cell_paragraph(para, inner, max_height), &placed.line, left);
                    }
                }
                x += width;
            }
            self.y = top + height;
            next = slice;
            if !cells
                .iter()
                .zip(&next)
                .any(|(lines, start)| *start < lines.len())
            {
                break;
            }
            self.new_page()?;
        }
        Ok(())
    }

    fn code_label(label: &str, icon: Option<&Object>) -> Vec<Inline> {
        let mut label_inline = Vec::with_capacity(3);
        if let Some(icon) = icon {
            label_inline.push(Inline::Object {
                object: icon.clone(),
                width: 12.,
                height: 12.,
                align: text::ObjectAlign::Baseline(3.),
                link: None,
            });
            label_inline.push(Inline::Text(
                "\u{2002}".into(),
                Style::new(Role::Body, 7.5, [0x77; 3]),
            ));
        }
        label_inline.push(Inline::Text(
            if label.is_empty() {
                "TEXT".into()
            } else {
                label.to_uppercase()
            },
            Style::new(Role::Body, 7.5, [0x77; 3]),
        ));
        label_inline
    }

    fn paint(self, title: &str) -> Result<Vec<u8>> {
        let mut document = Document::new_with(SerializeSettings {
            enable_tagging: false,
            render_svg_glyph_fn: krilla_svg::render_svg_glyph,
            ..SerializeSettings::default()
        });
        let total = self.pages.len();
        let size = Size::from_wh(PAGE.0, PAGE.1).context("A4 page size")?;
        for (index, page) in self.pages.iter().enumerate() {
            let mut sheet = document.start_page_with(PageSettings::new(size));
            let mut surface = sheet.surface();
            for draw in &page.draws {
                paint(&mut surface, self.fonts, draw);
            }
            // Page number, quiet and centered in the bottom margin.
            let number = [Inline::Text(
                format!("{} / {total}", index + 1),
                Style::new(Role::Body, 8., [0x99; 3]),
            )];
            let paragraph = Paragraph {
                inlines: &number,
                role: Role::Body,
                size: 8.,
                width: self.width(),
                max_height: MARGIN_Y,
                line_height: 1.2,
                justify: false,
                preserve: false,
            };
            if let Some(line) = text::layout(self.fonts, &paragraph).into_iter().next() {
                for placed in line.items {
                    if let Item::Run(run) = placed.item {
                        let draw = Draw::Run {
                            x: (PAGE.0 - line.width) / 2. + placed.x,
                            baseline: PAGE.1 - MARGIN_Y / 2.,
                            run,
                            style: text_style(&number, 0),
                        };
                        paint(&mut surface, self.fonts, &draw);
                    }
                }
            }
            surface.finish();
            for link in &page.links {
                let (x, y, width, height) = link.rect;
                if let Some(rect) = Rect::from_xywh(x, y, width.max(0.1), height.max(0.1)) {
                    sheet.add_annotation(
                        LinkAnnotation::new(
                            rect,
                            Target::Action(LinkAction::new(link.url.to_string()).into()),
                        )
                        .into(),
                    );
                }
            }
            sheet.finish();
        }
        document.set_outline(outline(&self.headings));
        document.set_metadata(
            Metadata::new()
                .title(title.to_owned())
                .description(format!("Embedded language marks: Devicon v2.18.0 and Lucide. Licenses apply only to these marks.\n{}", pecan_assets::language::license()))
                .creator("Pecan".into())
                .language("zh-CN".into()),
        );
        document
            .finish()
            .map_err(|error| anyhow::anyhow!("PDF 写出失败：{error:?}"))
    }
}

pub(super) const QUOTE_STEP: f32 = 14.;
pub(super) const LIST_STEP: f32 = 20.;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Align {
    Justify,
    Left,
}

pub(super) fn heading_scale(level: u8) -> f32 {
    [2., 1.6, 1.3, 1.15, 1.05, 1.][usize::from(level.clamp(1, 6) - 1)]
}

/// A paragraph of a table cell. `width` is the cell's inner width; a list
/// item starts further in. Objects taller than `max_height` are scaled down.
fn cell_paragraph(para: &Para, width: f32, max_height: f32) -> Paragraph<'_> {
    Paragraph {
        inlines: &para.inlines,
        role: Role::Body,
        size: CELL_SIZE,
        width: width - para.indent,
        max_height,
        line_height: 1.45,
        justify: false,
        preserve: false,
    }
}

/// One line of a table cell.
struct CellLine {
    line: Line,
    /// Index of its paragraph in the cell.
    para: usize,
    /// Space above the first line of a later paragraph. A cell, or the part
    /// of it continued on another page, starts at the padding instead.
    gap: f32,
    /// The first line of its paragraph, beside the list marker.
    first: bool,
}

/// Height of consecutive lines of one cell, without its padding.
fn lines_height(lines: &[CellLine]) -> f32 {
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| line.line.height() + if index > 0 { line.gap } else { 0. })
        .sum()
}

fn row_height(cells: &[Vec<CellLine>]) -> f32 {
    cells
        .iter()
        .map(|lines| lines_height(lines))
        .fold(0., f32::max)
        + 2. * CELL_PAD.1
}

fn text_style(inlines: &[Inline], index: usize) -> Style {
    match &inlines[index] {
        Inline::Text(_, style) => style.clone(),
        _ => Style::new(Role::Body, BODY_SIZE, INK),
    }
}

fn plain(inlines: &[Inline]) -> String {
    inlines
        .iter()
        .filter_map(|inline| match inline {
            Inline::Text(text, _) => Some(text.replace(text::SOFT_BREAK, " ")),
            _ => None,
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn outline(headings: &[Heading]) -> Outline {
    // Each heading owns the deeper headings that follow it.
    fn build(headings: &[Heading], index: &mut usize) -> OutlineNode {
        let heading = &headings[*index];
        let mut node = OutlineNode::new(
            heading.title.clone(),
            XyzDestination::new(heading.page, Point::from_xy(MARGIN_X, heading.y)),
        );
        *index += 1;
        while *index < headings.len() && headings[*index].level > heading.level {
            node.push_child(build(headings, index));
        }
        node
    }
    let mut outline = Outline::new();
    let mut index = 0;
    while index < headings.len() {
        outline.push_child(build(headings, &mut index));
    }
    outline
}

fn color(rgb: [u8; 3]) -> rgb::Color {
    rgb::Color::new(rgb[0], rgb[1], rgb[2])
}

fn rounded(rect: (f32, f32, f32, f32), radius: f32) -> Option<Path> {
    let (x, y, w, h) = rect;
    let mut builder = PathBuilder::new();
    if radius <= 0. {
        builder.push_rect(Rect::from_xywh(x, y, w.max(0.01), h.max(0.01))?);
        return builder.finish();
    }
    let r = radius.min(w / 2.).min(h / 2.);
    // Cubic approximation of quarter circles.
    let k = r * 0.552_284_8;
    builder.move_to(x + r, y);
    builder.line_to(x + w - r, y);
    builder.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    builder.line_to(x + w, y + h - r);
    builder.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    builder.line_to(x + r, y + h);
    builder.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    builder.line_to(x, y + r);
    builder.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    builder.close();
    builder.finish()
}

fn paint(surface: &mut Surface<'_>, fonts: &Fonts, draw: &Draw) {
    match draw {
        Draw::Rect {
            rect,
            fill,
            stroke,
            radius,
        } => {
            let Some(path) = rounded(*rect, *radius) else {
                return;
            };
            surface.set_fill(fill.map(|rgb| Fill {
                paint: color(rgb).into(),
                ..Fill::default()
            }));
            surface.set_stroke(stroke.map(|(rgb, width)| Stroke {
                paint: color(rgb).into(),
                width,
                ..Stroke::default()
            }));
            surface.draw_path(&path);
        }
        Draw::Run {
            x,
            baseline,
            run,
            style,
        } => {
            surface.set_fill(Some(Fill {
                paint: color(style.color).into(),
                opacity: NormalizedF32::ONE,
                ..Fill::default()
            }));
            // Synthesized bold strokes the outline; synthesized italic slants it.
            surface.set_stroke(run.fake_bold.then(|| Stroke {
                paint: color(style.color).into(),
                width: run.size * 0.035,
                ..Stroke::default()
            }));
            let slanted = run.fake_italic;
            if slanted {
                surface.push_transform(&Transform::from_row(1., 0., -0.2, 1., 0.2 * baseline, 0.));
            }
            surface.draw_glyphs(
                Point::from_xy(*x, *baseline),
                &run.krilla_glyphs(),
                fonts.font(run.face).clone(),
                &run.text,
                run.size,
                false,
            );
            if slanted {
                surface.pop();
            }
            surface.set_stroke(None);
        }
        Draw::Object { object, rect } => {
            let (x, y, width, height) = *rect;
            let Some(size) = Size::from_wh(width.max(0.01), height.max(0.01)) else {
                return;
            };
            surface.push_transform(&Transform::from_translate(x, y));
            match object {
                Object::Raster(image) => surface.draw_image(image.clone(), size),
                Object::Vector(vector) => {
                    let _ = surface.draw_svg(vector.tree(), size, SvgSettings::default());
                }
            }
            surface.pop();
        }
    }
}

const fn color_bytes(color: u32) -> [u8; 3] {
    let [_, r, g, b] = color.to_be_bytes();
    [r, g, b]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fonts() -> Fonts {
        Fonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_MONO_FONT,
        )
    }

    fn pager(fonts: &mut Fonts) -> Pager<'_> {
        Pager {
            fonts,
            pages: vec![Page {
                draws: Vec::new(),
                links: Vec::new(),
            }],
            y: TOP,
            margin: 0.,
            headings: Vec::new(),
            line_height: 1.6,
            inset: 0.,
            reserved: 0.,
            suppress_space: false,
        }
    }

    fn note_fixture(lines: usize) -> Block {
        let style = Style::new(Role::Body, BODY_SIZE, INK);
        let paragraph = |inlines, keep_next| {
            Block::Paragraph(Para {
                inlines,
                kind: ParaKind::Body,
                indent: 0.,
                marker: None,
                quote: 0,
                align: Align::Left,
                keep_next,
            })
        };
        let mut title = style.clone();
        title.bold = true;
        let mut body = Vec::new();
        for index in 0..lines {
            if index > 0 {
                body.push(Inline::Break);
            }
            body.push(Inline::Text(format!("Body line {index}"), style.clone()));
        }
        Block::Note {
            blocks: vec![
                paragraph(vec![Inline::Text("Note title".into(), title)], true),
                paragraph(body, false),
            ],
            indent: 0.,
            kind: None,
        }
    }

    #[test]
    fn one_capsule_surrounds_all_faces_and_strike_is_painted_above_it() {
        let mut fonts = fonts();
        let mut code = Style::new(Role::Code, 9.68, INK);
        code.capsule = Some(0);
        code.strike = true;
        let mut bold = code.clone();
        bold.bold = true;
        let inlines = [
            Inline::Text("API接口".into(), code),
            Inline::Text("文档".into(), bold),
        ];
        let paragraph = Paragraph {
            inlines: &inlines,
            role: Role::Body,
            size: BODY_SIZE,
            width: WIDTH,
            max_height: 700.,
            line_height: 1.6,
            justify: true,
            preserve: false,
        };
        let lines = text::layout(&mut fonts, &paragraph);
        let mut pager = pager(&mut fonts);
        pager.line(&paragraph, &lines[0], MARGIN_X);
        let draws = &pager.pages[0].draws;
        let capsules: Vec<_> = draws
            .iter()
            .enumerate()
            .filter(|(_, draw)| {
                matches!(
                    draw,
                    Draw::Rect {
                        fill: Some(CAPSULE_FILL),
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(
            capsules.len(),
            1,
            "font fallback and emphasis must not create layered borders"
        );
        for (index, draw) in draws.iter().enumerate() {
            if matches!(
                draw,
                Draw::Rect {
                    fill: Some(INK),
                    ..
                }
            ) {
                assert!(
                    index > capsules[0].0,
                    "strike must not be hidden under the capsule fill"
                );
            }
        }
        let mut next = Style::new(Role::Code, 9.68, INK);
        next.capsule = Some(1);
        let separate = [
            Inline::Text("甲".into(), text_style(&inlines, 0)),
            Inline::Text("乙".into(), next),
        ];
        let paragraph = Paragraph {
            inlines: &separate,
            ..paragraph
        };
        let lines = text::layout(pager.fonts, &paragraph);
        pager.pages[0].draws.clear();
        pager.line(&paragraph, &lines[0], MARGIN_X);
        assert_eq!(
            pager.pages[0]
                .draws
                .iter()
                .filter(|draw| matches!(
                    draw,
                    Draw::Rect {
                        fill: Some(CAPSULE_FILL),
                        ..
                    }
                ))
                .count(),
            2,
            "neighboring source capsules retain separate borders"
        );
    }

    #[test]
    fn quotes_keep_symmetric_vertical_padding_on_every_occupied_page() {
        for (count, start, depth) in [
            (1, TOP, 1),
            (4, TOP, 2),
            (140, TOP, 1),
            (1, BOTTOM - 20., 1),
        ] {
            let mut fonts = fonts();
            let style = Style::new(Role::Body, BODY_SIZE, MUTED);
            let mut inlines = Vec::new();
            for index in 0..count {
                if index > 0 {
                    inlines.push(Inline::Break);
                }
                inlines.push(Inline::Text(format!("Quoted line {index}"), style.clone()));
            }
            let para = Para {
                inlines,
                kind: ParaKind::Body,
                indent: f32::from(depth) * QUOTE_STEP,
                marker: None,
                quote: depth,
                align: Align::Left,
                keep_next: false,
            };
            let line = text::layout(
                &mut fonts,
                &Paragraph {
                    inlines: &para.inlines,
                    role: Role::Body,
                    size: BODY_SIZE,
                    width: WIDTH - para.indent,
                    max_height: BOTTOM - TOP,
                    line_height: 1.6,
                    justify: false,
                    preserve: false,
                },
            )
            .remove(0);
            let mut pager = pager(&mut fonts);
            pager.y = start;
            pager.paragraph(&para, None).unwrap();
            let mut seen = 0;
            for page in &pager.pages {
                let baselines: Vec<_> = page
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Run { baseline, .. } => Some(*baseline),
                        _ => None,
                    })
                    .collect();
                let frames: Vec<_> = page
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Rect {
                            rect,
                            fill: Some(QUOTE_FILL),
                            ..
                        } => Some(*rect),
                        _ => None,
                    })
                    .collect();
                if baselines.is_empty() {
                    assert!(
                        frames.is_empty(),
                        "moving a quote must not leave an empty frame"
                    );
                    continue;
                }
                seen += 1;
                assert_eq!(frames.len(), 1);
                let (_, top, _, height) = frames[0];
                assert!(top >= TOP && top + height <= BOTTOM + 0.01);
                let first = baselines.iter().copied().fold(f32::INFINITY, f32::min);
                let last = baselines.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                assert!(
                    (first - line.ascent - top - 6.).abs() < 0.02,
                    "quote needs 6pt top padding"
                );
                assert!(
                    (top + height - last - line.descent - 6.).abs() < 0.02,
                    "quote needs 6pt bottom padding"
                );
                let bars = page
                    .draws
                    .iter()
                    .filter(|draw| {
                        matches!(draw, Draw::Rect { rect, fill: Some(QUOTE_BAR), .. }
                    if (rect.1 - top).abs() < 0.02 && (rect.3 - height).abs() < 0.02)
                    })
                    .count();
                assert_eq!(
                    bars,
                    usize::from(depth),
                    "quote bars include the padded band"
                );
            }
            assert_eq!(seen > 1, count == 140);
            if start > TOP {
                assert_eq!(
                    pager.pages.len(),
                    2,
                    "a complete padded line moves to the next page"
                );
            }
        }
    }

    #[test]
    fn semantic_note_colors_survive_continuation_pages() {
        use pecan_document::AlertKind;
        let mut fonts = fonts();
        for (kind, expected) in [
            (AlertKind::Note, [0xe8, 0xf2, 0xff]),
            (AlertKind::Tip, [0xe7, 0xf6, 0xee]),
            (AlertKind::Important, [0xf4, 0xea, 0xff]),
            (AlertKind::Warning, [0xff, 0xf6, 0xe8]),
            (AlertKind::Error, [0xfd, 0xec, 0xec]),
        ] {
            let mut note = note_fixture(140);
            if let Block::Note {
                kind: note_kind, ..
            } = &mut note
            {
                *note_kind = Some(kind);
            }
            let mut pager = pager(&mut fonts);
            pager.block(&note, None).unwrap();
            assert!(pager.pages.len() > 1);
            for page in &pager.pages {
                assert!(page.draws.iter().any(|draw| matches!(draw,Draw::Rect { fill: Some(fill), .. } if *fill == expected)),"{kind:?}");
            }
        }
    }

    #[test]
    fn note_title_and_body_share_one_padded_frame_on_each_page() {
        for lines in [2, 140] {
            let mut fonts = fonts();
            let mut pager = pager(&mut fonts);
            pager.block(&note_fixture(lines), None).unwrap();
            for page in &pager.pages {
                let frames: Vec<_> = page
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Rect {
                            rect,
                            fill: Some(HIGHLIGHT_FILL),
                            ..
                        } => Some(*rect),
                        _ => None,
                    })
                    .collect();
                assert_eq!(frames.len(), 1, "a note has one frame per occupied page");
                let (left, top, width, height) = frames[0];
                assert!(
                    top >= TOP && top + height <= BOTTOM + 0.01,
                    "note frame stays inside the page"
                );
                let runs: Vec<_> = page
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Run {
                            x, baseline, run, ..
                        } => Some((*x, *baseline, run)),
                        _ => None,
                    })
                    .collect();
                assert!(!runs.is_empty(), "no empty continuation box");
                for (x, baseline, run) in runs {
                    assert!(x >= left + NOTE_PAD.0 - 0.01, "left inner padding");
                    assert!(
                        x + run.width <= left + width - NOTE_PAD.0 + 0.01,
                        "right inner padding"
                    );
                    assert!(baseline > top + NOTE_PAD.1, "top inner padding");
                    assert!(
                        baseline + 2. <= top + height - NOTE_PAD.1 + 0.01,
                        "bottom inner padding"
                    );
                }
            }
            assert_eq!(
                pager.pages.len() > 1,
                lines == 140,
                "long notes split without losing frames"
            );
        }
    }

    #[test]
    fn a_note_title_moves_with_body_and_leaves_no_empty_frame() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        pager.y = BOTTOM - 34.;
        pager.page().draws.push(Draw::Rect {
            rect: (MARGIN_X, TOP, WIDTH, 1.),
            fill: None,
            stroke: None,
            radius: 0.,
        });
        pager.block(&note_fixture(2), None).unwrap();
        assert_eq!(
            pager.pages.len(),
            2,
            "title must move with the start of its body"
        );
        assert_eq!(
            pager.pages[0].draws.len(),
            1,
            "no empty box at the old page bottom"
        );
        let text: String = pager.pages[1]
            .draws
            .iter()
            .filter_map(|draw| match draw {
                Draw::Run { run, .. } => Some(run.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            text.contains("Note title") && text.contains("Body line 0"),
            "title and body share the next page"
        );
    }

    #[test]
    fn page_budget_is_checked_before_allocating() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        pager.pages = (0..MAX_PAGES)
            .map(|_| Page {
                draws: Vec::new(),
                links: Vec::new(),
            })
            .collect();
        pager.y = BOTTOM;
        let error = pager.new_page().unwrap_err();
        assert!(error.to_string().contains("导出预算"));
        assert_eq!(pager.pages.len(), MAX_PAGES, "no extra page is allocated");
        assert_eq!(
            pager.y, BOTTOM,
            "failed allocation does not alter pagination"
        );
    }

    /// A table cell with one plain paragraph.
    fn cell(inlines: Vec<Inline>) -> Vec<Para> {
        vec![cell_para(inlines, ParaKind::Body, 0., None)]
    }

    fn cell_para(inlines: Vec<Inline>, kind: ParaKind, indent: f32, marker: Option<&str>) -> Para {
        Para {
            inlines,
            kind,
            indent,
            marker: marker
                .map(|marker| Inline::Text(marker.into(), Style::new(Role::Body, CELL_SIZE, INK))),
            quote: 0,
            align: Align::Left,
            keep_next: false,
        }
    }

    fn cell_text(contents: &str) -> Vec<Inline> {
        vec![Inline::Text(
            contents.into(),
            Style::new(Role::Body, CELL_SIZE, INK),
        )]
    }

    /// Text runs of a page as `(x, baseline, text)`, in drawing order.
    fn runs(page: &Page) -> Vec<(f32, f32, &str)> {
        page.draws
            .iter()
            .filter_map(|draw| match draw {
                Draw::Run {
                    x, baseline, run, ..
                } => Some((*x, *baseline, run.text.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn table_cell_paragraphs_stack_with_markers_and_hanging_indents() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let long = "wrapped ".repeat(40);
        let rows = [Row {
            head: false,
            cells: vec![
                cell(cell_text("Label")),
                vec![
                    cell_para(cell_text("Intro"), ParaKind::Body, 0., None),
                    cell_para(cell_text(&long), ParaKind::Item, LIST_STEP, Some("•")),
                    cell_para(cell_text("Second"), ParaKind::Item, LIST_STEP, Some("•")),
                ],
            ],
        }];
        pager.table(&[Alignment::None; 2], &rows, 0.).unwrap();
        let runs = runs(&pager.pages[0]);
        let find = |text: &str| {
            *runs
                .iter()
                .find(|(_, _, run)| run.starts_with(text))
                .unwrap_or_else(|| panic!("{text} is drawn: {runs:?}"))
        };
        let intro = find("Intro");
        let second = find("Second");
        // The first run of every line of the long item.
        let mut wrapped: Vec<_> = runs
            .iter()
            .filter(|(_, _, run)| run.starts_with("wrapped"))
            .collect();
        wrapped.dedup_by(|next, line| (next.1 - line.1).abs() < 0.01);
        assert!(wrapped.len() > 1, "the long item wraps: {runs:?}");
        assert!(
            wrapped
                .iter()
                .all(|(x, _, _)| (x - (intro.0 + LIST_STEP)).abs() < 0.01),
            "every line of an item hangs at the list indent: {wrapped:?}"
        );
        let markers: Vec<_> = runs.iter().filter(|(_, _, run)| *run == "•").collect();
        assert_eq!(markers.len(), 2, "one marker per item: {runs:?}");
        assert!(
            (markers[0].1 - wrapped[0].1).abs() < 0.01 && (markers[1].1 - second.1).abs() < 0.01,
            "a marker shares the baseline of its item's first line"
        );
        assert!(
            markers
                .iter()
                .all(|(x, _, _)| *x > intro.0 && *x < intro.0 + LIST_STEP),
            "markers stay inside the cell, in the indent: {markers:?}"
        );
        let step = wrapped[1].1 - wrapped[0].1;
        let last = wrapped[wrapped.len() - 1].1;
        assert!(
            wrapped[0].1 - intro.1 > step + 0.01 && second.1 - last > step + 0.01,
            "paragraphs of a cell are set apart"
        );
        assert!(
            second.1 - last < wrapped[0].1 - intro.1,
            "list items sit closer than paragraphs"
        );
    }

    #[test]
    fn table_row_continued_on_the_next_page_starts_without_a_paragraph_gap() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let paras: Vec<Para> = (0..80)
            .map(|index| {
                cell_para(
                    cell_text(&format!("Item {index}")),
                    ParaKind::Item,
                    LIST_STEP,
                    Some("•"),
                )
            })
            .collect();
        let rows = [Row {
            head: false,
            cells: vec![paras],
        }];
        pager.table(&[Alignment::None], &rows, 0.).unwrap();
        assert_eq!(pager.pages.len(), 2, "the row continues on a second page");
        let tops: Vec<f32> = pager
            .pages
            .iter()
            .map(|page| {
                let top = page
                    .draws
                    .iter()
                    .find_map(|draw| match draw {
                        Draw::Rect { rect, .. } => Some(rect.1),
                        _ => None,
                    })
                    .expect("a cell frame");
                let first = runs(page)
                    .into_iter()
                    .find(|(_, _, run)| run.starts_with("Item"))
                    .expect("an item");
                first.1 - top
            })
            .collect();
        assert!(
            (tops[0] - tops[1]).abs() < 0.01,
            "both parts start at the cell padding: {tops:?}"
        );
        let items: usize = pager
            .pages
            .iter()
            .map(|page| {
                runs(page)
                    .iter()
                    .filter(|(_, _, run)| run.starts_with("Item"))
                    .count()
            })
            .sum();
        assert_eq!(items, 80, "no item is lost or repeated at the break");
    }

    #[test]
    fn impossible_table_line_fails_without_allocating_blank_pages() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let row = Row {
            head: false,
            cells: vec![Vec::new()],
        };
        let cells = vec![vec![CellLine {
            line: Line {
                ink: None,
                items: Vec::new(),
                ascent: PAGE.1,
                descent: 0.,
                width: 10.,
            },
            para: 0,
            gap: 0.,
            first: true,
        }]];
        let max_height = BOTTOM - TOP - 2. * CELL_PAD.1;
        let error = pager
            .row(&row, &cells, &[WIDTH], &[], 0., None, max_height)
            .unwrap_err();
        assert!(error.to_string().contains("无法继续分页"));
        assert_eq!(
            pager.pages.len(),
            1,
            "an impossible row cannot loop over blank pages"
        );
    }

    #[test]
    fn tall_table_object_is_scaled_and_drawn_within_a_page() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let rows = [Row {
            head: false,
            cells: vec![cell(vec![Inline::Object {
                object: Object::Raster(krilla::image::Image::from_rgba8(vec![255; 4], 1, 1)),
                width: 5.,
                height: 1500.,
                align: text::ObjectAlign::Baseline(0.),
                link: None,
            }])],
        }];
        pager.table(&[Alignment::None], &rows, 0.).unwrap();
        assert_eq!(
            pager.pages.len(),
            1,
            "a tall image needs no blank continuation pages"
        );
        let rect = pager.pages[0]
            .draws
            .iter()
            .find_map(|draw| match draw {
                Draw::Object { rect, .. } => Some(*rect),
                _ => None,
            })
            .expect("the image is preserved");
        let (_, y, width, height) = rect;
        assert!(
            y >= TOP && y + height <= BOTTOM,
            "image bounds {rect:?} fit the page"
        );
        assert!(
            (width / height - 5. / 1500.).abs() < 0.0001,
            "aspect ratio survives scaling"
        );
        assert!(pager.y <= BOTTOM, "cell padding and descent also fit");
        let pdf = pager.paint("tall image").unwrap();
        assert!(
            pdf.starts_with(b"%PDF"),
            "the scaled image can be serialized"
        );
    }

    #[test]
    fn tall_table_object_keeps_header_and_caption_on_the_same_page() {
        for near_bottom in [false, true] {
            let mut fonts = fonts();
            let mut pager = pager(&mut fonts);
            if near_bottom {
                pager.page().draws.push(Draw::Rect {
                    rect: (MARGIN_X, TOP, WIDTH, 1.),
                    fill: Some(RULE),
                    stroke: None,
                    radius: 0.,
                });
                pager.y = BOTTOM - 40.;
            }
            let text = |contents: &str| {
                cell(vec![Inline::Text(
                    contents.to_owned(),
                    Style::new(Role::Body, CELL_SIZE, INK),
                )])
            };
            let rows = [
                Row {
                    head: true,
                    cells: vec![text("Image header"), text("Caption header")],
                },
                Row {
                    head: false,
                    cells: vec![
                        cell(vec![Inline::Object {
                            object: Object::Raster(krilla::image::Image::from_rgba8(
                                vec![255; 4],
                                1,
                                1,
                            )),
                            width: 5.,
                            height: 1500.,
                            align: text::ObjectAlign::Baseline(0.),
                            link: None,
                        }]),
                        text("Tall image caption"),
                    ],
                },
            ];
            pager.table(&[Alignment::None; 2], &rows, 0.).unwrap();
            assert_eq!(pager.pages.len(), if near_bottom { 2 } else { 1 });
            let mut table_pages = 0;
            for page in &pager.pages {
                let contents: String = page
                    .draws
                    .iter()
                    .filter_map(|draw| match draw {
                        Draw::Run { run, .. } => Some(run.text.as_str()),
                        _ => None,
                    })
                    .collect();
                let image = page.draws.iter().find_map(|draw| match draw {
                    Draw::Object { rect, .. } => Some(*rect),
                    _ => None,
                });
                if image.is_some() || contents.contains("header") || contents.contains("caption") {
                    table_pages += 1;
                    let rect = image.expect("a table page also contains the image");
                    for expected in ["Image header", "Caption header", "Tall image caption"] {
                        assert!(
                            contents.contains(expected),
                            "page keeps {expected}: {contents}"
                        );
                    }
                    assert!(
                        rect.1 >= TOP && rect.1 + rect.3 <= BOTTOM,
                        "image fits below header: {rect:?}"
                    );
                }
            }
            assert_eq!(table_pages, 1, "no isolated table-header or caption page");
            assert!(pager.y <= BOTTOM, "the row fits with its repeated header");
        }
    }

    #[test]
    fn table_header_that_leaves_no_body_line_returns_an_error() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let mut header = Vec::new();
        for _ in 0..100 {
            header.push(Inline::Text(
                "Heading".into(),
                Style::new(Role::Body, BODY_SIZE, INK),
            ));
            header.push(Inline::Break);
        }
        let rows = [
            Row {
                head: true,
                cells: vec![cell(header)],
            },
            Row {
                head: false,
                cells: vec![cell(vec![Inline::Text(
                    "Body".into(),
                    Style::new(Role::Body, BODY_SIZE, INK),
                )])],
            },
        ];
        let error = pager.table(&[Alignment::None], &rows, 0.).unwrap_err();
        assert!(error.to_string().contains("表头过高"));
        assert_eq!(
            pager.pages.len(),
            1,
            "failure happens before allocating pages"
        );
        assert!(
            pager.pages[0].draws.is_empty(),
            "a rejected table leaves no orphan header"
        );
    }

    #[test]
    fn heading_stays_with_code_header_and_first_rows_at_a_page_boundary() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let heading = Para {
            inlines: vec![Inline::Text(
                "JSON".into(),
                Style::new(Role::Body, BODY_SIZE, INK),
            )],
            kind: ParaKind::Heading(2),
            indent: 0.,
            marker: None,
            quote: 0,
            align: super::super::Align::Left,
            keep_next: false,
        };
        let code = Block::Code {
            inlines: vec![Inline::Text(
                "{\n    \"name\": \"Pecan\"\n}".into(),
                Style::new(Role::Code, CODE_SIZE, INK),
            )],
            label: "json".into(),
            icon: None,
            indent: 0.,
        };
        // The old 3 body-line estimate fits here, but the native code lead band does not.
        pager.y = BOTTOM - 125.;
        pager.paragraph(&heading, Some(&code)).unwrap();
        pager.block(&code, None).unwrap();
        assert_eq!(pager.pages.len(), 2);
        assert_eq!(pager.headings[0].page, 1);
        assert!(
            pager.pages[0].draws.is_empty(),
            "no orphan title remains on the prior page"
        );
        assert!(pager.pages[1].draws.iter().any(|draw| matches!(
            draw,
            Draw::Rect {
                fill: Some(CODE_HEADER_FILL),
                ..
            }
        )));
    }

    #[test]
    fn code_language_has_a_distinct_fill_and_stays_with_the_body() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let mut inlines = Vec::new();
        for index in 0..120 {
            if index > 0 {
                inlines.push(Inline::Break);
            }
            inlines.push(Inline::Text(
                format!("let value_{index} = {index};"),
                Style::new(Role::Code, CODE_SIZE, INK),
            ));
        }
        let tree = crate::language_icons::Icons::default()
            .tree("rust")
            .unwrap();
        let icon = Object::Vector(crate::pdf::Vector::Tree(tree));
        pager.code(&inlines, "rust", Some(&icon), 0.).unwrap();
        for page in &pager.pages {
            let header = page
                .draws
                .iter()
                .find_map(|draw| match draw {
                    Draw::Rect {
                        rect,
                        fill: Some([0xf4, 0xf4, 0xf4]),
                        ..
                    } => Some(*rect),
                    _ => None,
                })
                .expect("every code page has a shaded language header");
            let body = page
                .draws
                .iter()
                .find_map(|draw| match draw {
                    Draw::Rect {
                        rect,
                        fill: Some([0xfc, 0xfc, 0xfc]),
                        stroke: Some(_),
                        ..
                    } => Some(*rect),
                    _ => None,
                })
                .expect("the body retains the native light fill and full border");
            assert!(header.1 >= body.1 && header.1 + header.3 < body.1 + body.3);
            assert!(
                (header.3 + 0.75 - 26.).abs() < 0.01,
                "the language band retains 26 pt height on every code page: {header:?}"
            );
            let right = page
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    Draw::Run { x, run, style, .. } if style.role == Role::Body => {
                        Some(x + run.width)
                    }
                    _ => None,
                })
                .reduce(f32::max)
                .expect("language label is painted");
            assert!(
                (right - (body.0 + body.2 - CODE_PAD.0)).abs() < 0.1,
                "language aligns with the right content inset"
            );
            let logo = page
                .draws
                .iter()
                .find_map(|draw| match draw {
                    Draw::Object { rect, .. } => Some(*rect),
                    _ => None,
                })
                .expect("every continuation header repeats its vector logo");
            assert_eq!((logo.2, logo.3), (12., 12.));
            assert!(logo.1 > body.1 && logo.1 + logo.3 < header.1 + header.3);
            let text_left = page
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    Draw::Run { x, run, style, .. }
                        if style.role == Role::Body && !run.text.trim().is_empty() =>
                    {
                        Some(*x)
                    }
                    _ => None,
                })
                .reduce(f32::min)
                .unwrap();
            assert!(
                text_left > logo.0 + logo.2,
                "logo stays left of the language with a gap"
            );

            let code_runs: Vec<_> = page
                .draws
                .iter()
                .filter_map(|draw| match draw {
                    Draw::Run {
                        baseline, style, ..
                    } if style.role == Role::Code => Some(*baseline),
                    _ => None,
                })
                .collect();
            assert!(!code_runs.is_empty(), "a header cannot be stranded");
            assert!(
                code_runs
                    .iter()
                    .all(|y| *y > header.1 + header.3 && *y < body.1 + body.3)
            );
        }
    }

    #[test]
    fn long_code_keeps_all_lines_and_a_box_on_each_page() {
        let mut fonts = fonts();
        let mut pager = pager(&mut fonts);
        let mut inlines = Vec::new();
        for index in 0..120 {
            inlines.push(Inline::Text(
                format!("let item_{index} = {index};"),
                Style::new(Role::Code, CODE_SIZE, INK),
            ));
            if index < 119 {
                inlines.push(Inline::Break);
            }
        }
        pager.code(&inlines, "rust", None, 0.).unwrap();
        assert!(pager.pages.len() > 2, "long code spans pages");
        let mut contents = String::new();
        for page in &pager.pages {
            assert!(
                page.draws.iter().any(|draw| matches!(draw, Draw::Rect {
                fill: Some(fill), stroke: Some(_), ..
            } if *fill == CODE_FILL)),
                "each continuation page has a code border"
            );
            for draw in &page.draws {
                if let Draw::Run { run, .. } = draw {
                    contents.push_str(&run.text);
                }
            }
        }
        for index in 0..120 {
            assert!(
                contents.contains(&format!("let item_{index} = {index};")),
                "line {index} is kept"
            );
        }
    }
}
