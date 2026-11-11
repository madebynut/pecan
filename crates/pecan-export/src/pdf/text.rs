//! Portable text layout for PDF: system fonts with per-character fallback, rustybuzz
//! shaping, UAX #14 line breaking (which includes CJK punctuation rules) and justification.
use std::{
    collections::{BTreeMap, VecDeque},
    ops::Range,
    sync::Arc,
};

use super::super::typography::Glyph;
pub(crate) use super::super::typography::{Fonts, Ink, Role, Run, Style};

use super::Object;

/// Horizontal inset belongs to the line measure, including the contained border.
pub(crate) const CAPSULE_PAD: f32 = 2.5;

/// Objects take part in line breaking like ideographs.
const OBJECT: char = '\u{FFFC}';
/// A Markdown soft break: a space, except between CJK characters.
pub(crate) const SOFT_BREAK: char = '\u{2029}';

/// How an inline graphic shares the text baseline. Formulas keep their own
/// baseline; a symbol is centered on the actual shaped ink of its line.
#[derive(Clone, Copy)]
pub(crate) enum ObjectAlign {
    Baseline(f32),
    TextCenter,
}

pub(crate) enum Inline {
    Text(String, Style),
    Object {
        object: Object,
        width: f32,
        height: f32,
        align: ObjectAlign,
        link: Option<Arc<str>>,
    },
    Break,
}

#[derive(Clone)]
pub(crate) enum Item {
    Run(Run),
    /// Index of an object inline.
    Object(usize),
}

#[derive(Clone)]
struct Piece {
    items: Vec<Item>,
    width: f32,
    /// Trailing whitespace, dropped at the end of a line.
    trailing: f32,
    /// Ends at a break opportunity (not merely a style change).
    breakable: bool,
    mandatory: bool,
    /// One source capsule, shared across font fallback and emphasis runs.
    capsule: Option<usize>,
}

pub(crate) struct Placed {
    pub(crate) x: f32,
    pub(crate) item: Item,
}

pub(crate) struct Line {
    pub(crate) items: Vec<Placed>,
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
    pub(crate) width: f32,
    pub(crate) ink: Option<Ink>,
}

impl Line {
    pub(crate) fn object_drop(&self, paragraph: &Paragraph<'_>, index: usize) -> f32 {
        let (_, height, drop) = paragraph.object_size(index);
        match paragraph.inlines.get(index) {
            Some(Inline::Object {
                align: ObjectAlign::TextCenter,
                ..
            }) => self.ink.map_or(height / 2., |ink| ink.object_drop(height)),
            _ => drop,
        }
    }
    pub(crate) fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Paragraph<'a> {
    pub(crate) inlines: &'a [Inline],
    /// The paragraph's own role and size set the minimum line box.
    pub(crate) role: Role,
    pub(crate) size: f32,
    pub(crate) width: f32,
    /// A complete line must fit this height, including its text line box.
    pub(crate) max_height: f32,
    pub(crate) line_height: f32,
    pub(crate) justify: bool,
    /// Code keeps every space and wraps long lines anywhere.
    pub(crate) preserve: bool,
}

impl Paragraph<'_> {
    pub(crate) fn object_size(&self, inline: usize) -> (f32, f32, f32) {
        match self.inlines.get(inline) {
            Some(Inline::Object {
                width,
                height,
                align,
                ..
            }) => {
                // Reserve the text line box: an image on the baseline still has
                // the paragraph's descent and leading below it.
                let available = (self.max_height - self.size * self.line_height).max(0.);
                let scale = (self.width / width.max(f32::EPSILON))
                    .min(available / height.max(f32::EPSILON))
                    .min(1.);
                (
                    width * scale,
                    height * scale,
                    match align {
                        ObjectAlign::Baseline(drop) => drop * scale,
                        ObjectAlign::TextCenter => height * scale / 2.,
                    },
                )
            }
            _ => (0., 0., 0.),
        }
    }
}

pub(crate) fn layout(fonts: &mut Fonts, paragraph: &Paragraph<'_>) -> Vec<Line> {
    let pieces = pieces(fonts, paragraph);
    fill(fonts, paragraph, pieces)
}

/// Unbreakable width and single-line width, for table column sizing.
pub(crate) fn measure(fonts: &mut Fonts, paragraph: &Paragraph<'_>) -> (f32, f32) {
    let pieces = pieces(fonts, paragraph);
    let minimum = pieces
        .iter()
        .map(|piece| piece.width - piece.trailing)
        .fold(0., f32::max);
    let mut natural: f32 = 0.;
    let mut line: f32 = 0.;
    for piece in &pieces {
        line += piece.width;
        if piece.mandatory {
            natural = natural.max(line - piece.trailing);
            line = 0.;
        }
    }
    natural = natural.max(line - pieces.last().map_or(0., |piece| piece.trailing));
    (minimum, natural.max(minimum))
}

fn pieces(fonts: &mut Fonts, paragraph: &Paragraph<'_>) -> Vec<Piece> {
    let (text, spans) = flatten(paragraph);
    if text.is_empty() {
        return Vec::new();
    }
    let breaking: String = text
        .chars()
        .map(|c| if c == OBJECT { '\u{4E00}' } else { c })
        .collect();
    let mut capsules: Vec<(Range<usize>, usize)> = Vec::new();
    for (range, index) in &spans {
        if let Inline::Text(_, style) = &paragraph.inlines[*index]
            && let Some(id) = style.capsule
        {
            if let Some((previous, previous_id)) = capsules.last_mut()
                && *previous_id == id
                && previous.end == range.start
            {
                previous.end = range.end;
            } else {
                capsules.push((range.clone(), id));
            }
        }
    }
    if capsules.is_empty() {
        let mut pieces = Vec::new();
        let mut start = 0;
        for (end, kind) in unicode_linebreak::linebreaks(&breaking) {
            let mandatory = kind == unicode_linebreak::BreakOpportunity::Mandatory;
            if paragraph.preserve && !mandatory && end < text.len() && !text[..end].ends_with(' ') {
                continue;
            }
            if end <= start {
                continue;
            }
            let mut piece = build(fonts, paragraph, &text, &spans, start..end);
            piece.breakable = true;
            piece.mandatory = mandatory && end < text.len();
            pieces.push(piece);
            start = end;
        }
        return pieces;
    }
    let mut breaks = BTreeMap::new();
    for (end, kind) in unicode_linebreak::linebreaks(&breaking) {
        let mandatory = kind == unicode_linebreak::BreakOpportunity::Mandatory;
        let index = capsules.partition_point(|(range, _)| range.end <= end);
        if !mandatory
            && capsules
                .get(index)
                .is_some_and(|(range, _)| range.start < end && end < range.end)
        {
            continue;
        }
        if paragraph.preserve && !mandatory && end < text.len() && !text[..end].ends_with(' ') {
            continue;
        }
        breaks.insert(end, mandatory);
    }
    for (range, _) in capsules {
        if range.start > 0 {
            breaks.entry(range.start).or_insert(false);
        }
        breaks.entry(range.end).or_insert(false);
    }
    let mut pieces = Vec::new();
    let mut start = 0;
    for (end, mandatory) in breaks {
        if end <= start {
            continue;
        }
        let mut piece = build(fonts, paragraph, &text, &spans, start..end);
        piece.breakable = true;
        piece.mandatory = mandatory && end < text.len();
        pieces.push(piece);
        start = end;
    }
    pieces
}

/// Collapse whitespace (unless preserved) and record which inline each byte came from.
fn flatten(paragraph: &Paragraph<'_>) -> (String, Vec<(Range<usize>, usize)>) {
    let mut raw = String::new();
    let mut origin = Vec::new();
    for (index, inline) in paragraph.inlines.iter().enumerate() {
        let start = raw.len();
        match inline {
            Inline::Text(content, style) => {
                for character in content.chars() {
                    if paragraph.preserve {
                        match character {
                            '\t' => raw.push_str("    "),
                            '\n' | '\r' => {}
                            c => raw.push(c),
                        }
                    } else if style.capsule.is_some() {
                        raw.push(character);
                    } else if character == SOFT_BREAK {
                        raw.push(SOFT_BREAK);
                    } else if character.is_whitespace() && character != '\u{a0}' {
                        if !raw.is_empty() && !raw.ends_with([' ', SOFT_BREAK, '\n']) {
                            raw.push(' ');
                        }
                    } else {
                        raw.push(character);
                    }
                }
            }
            Inline::Object { .. } => raw.push(OBJECT),
            Inline::Break => raw.push('\n'),
        }
        origin.push((start..raw.len(), index));
    }
    // Resolve soft breaks against their neighbours, tracking byte offsets.
    let characters: Vec<(usize, char)> = raw.char_indices().collect();
    let mut text = String::with_capacity(raw.len());
    let mut offsets = vec![0; raw.len() + 1];
    for (position, &(byte, character)) in characters.iter().enumerate() {
        offsets[byte] = text.len();
        if character == SOFT_BREAK {
            let before = position.checked_sub(1).map(|i| characters[i].1);
            let after = characters.get(position + 1).map(|&(_, c)| c);
            let space = before.is_some_and(|c| c != ' ' && c != '\n')
                && after.is_some_and(|c| c != ' ')
                && !(before.is_some_and(wide) && after.is_some_and(wide));
            if space {
                text.push(' ');
            }
        } else {
            text.push(character);
        }
        for step in 1..character.len_utf8() {
            offsets[byte + step] = text.len();
        }
    }
    offsets[raw.len()] = text.len();
    let spans = origin
        .into_iter()
        .map(|(range, inline)| (offsets[range.start]..offsets[range.end], inline))
        .filter(|(range, _)| !range.is_empty())
        .collect();
    (text, spans)
}

fn wide(character: char) -> bool {
    matches!(character as u32,
        0x1100..=0x11FF | 0x2E80..=0x303E | 0x3040..=0x33FF | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF | 0xA960..=0xA97F | 0xAC00..=0xD7FF | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x20000..=0x3FFFD)
}

fn build(
    fonts: &mut Fonts,
    paragraph: &Paragraph<'_>,
    text: &str,
    spans: &[(Range<usize>, usize)],
    range: Range<usize>,
) -> Piece {
    let mut piece = Piece {
        items: Vec::new(),
        width: 0.,
        trailing: 0.,
        breakable: false,
        mandatory: false,
        capsule: None,
    };
    for (span, inline) in spans {
        let (start, end) = (span.start.max(range.start), span.end.min(range.end));
        if start >= end {
            continue;
        }
        let style = match &paragraph.inlines[*inline] {
            Inline::Text(_, style) => style,
            Inline::Object { .. } => {
                piece.width += paragraph.object_size(*inline).0;
                piece.items.push(Item::Object(*inline));
                continue;
            }
            Inline::Break => continue,
        };
        let size = style.glyph_size();
        let segment = &text[start..end];
        let mut run_start = 0;
        let mut current = None;
        for (offset, character) in segment.char_indices() {
            // Joiners and variation selectors stay with the preceding face.
            let face = if matches!(character, '\u{200D}' | '\u{FE00}'..='\u{FE0F}') {
                current
            } else {
                fonts.choose(style, character)
            };
            if face != current && offset > run_start {
                if let Some(face) = current {
                    push(
                        fonts,
                        &mut piece,
                        face,
                        *inline,
                        style,
                        &segment[run_start..offset],
                    );
                }
                run_start = offset;
            }
            if face.is_some() {
                current = face;
            }
        }
        if let Some(face) = current {
            push(
                fonts,
                &mut piece,
                face,
                *inline,
                style,
                &segment[run_start..],
            );
        }
        let _ = size;
    }
    piece.capsule = piece.items.first().and_then(|item| match item {
        Item::Run(run) => match &paragraph.inlines[run.inline] {
            Inline::Text(_, style) => style.capsule,
            _ => None,
        },
        Item::Object(_) => None,
    });
    if piece.capsule.is_some() {
        piece.width += 2. * CAPSULE_PAD;
    }
    if !paragraph.preserve
        && piece.capsule.is_none()
        && let Some(Item::Run(run)) = piece.items.last()
    {
        let kept = run.text.trim_end_matches(' ').len();
        piece.trailing = run
            .glyphs
            .iter()
            .filter(|glyph| glyph.cluster.start >= kept)
            .map(|glyph| glyph.advance * run.size)
            .sum();
    }
    piece
}

fn push(fonts: &Fonts, piece: &mut Piece, face: usize, inline: usize, style: &Style, text: &str) {
    let size = style.glyph_size();
    let glyphs = fonts.shape(face, text, size);
    let width = glyphs.iter().map(|glyph| glyph.advance).sum::<f32>() * size;
    let chosen = &fonts.faces[face];
    piece.width += width;
    piece.items.push(Item::Run(Run {
        face,
        inline,
        size,
        text: text.to_owned(),
        glyphs,
        width,
        fake_bold: style.bold && !chosen.bold,
        fake_italic: style.italic && !chosen.italic,
    }));
}

/// Split an over-wide piece at glyph clusters so it can wrap anywhere.
fn split(piece: Piece, paragraph: &Paragraph<'_>) -> Vec<Piece> {
    let mut output = Vec::new();
    for item in piece.items {
        match item {
            Item::Object(inline) => output.push(Piece {
                width: paragraph.object_size(inline).0,
                items: vec![item],
                trailing: 0.,
                breakable: true,
                mandatory: false,
                capsule: None,
            }),
            Item::Run(run) => {
                let mut index = 0;
                while index < run.glyphs.len() {
                    let cluster = run.glyphs[index].cluster.clone();
                    let mut end = index + 1;
                    while end < run.glyphs.len() && run.glyphs[end].cluster.start == cluster.start {
                        end += 1;
                    }
                    let glyphs: Vec<Glyph> = run.glyphs[index..end]
                        .iter()
                        .map(|glyph| Glyph {
                            cluster: 0..cluster.len(),
                            ..glyph.clone()
                        })
                        .collect();
                    let width = glyphs.iter().map(|glyph| glyph.advance).sum::<f32>() * run.size;
                    output.push(Piece {
                        items: vec![Item::Run(Run {
                            face: run.face,
                            inline: run.inline,
                            size: run.size,
                            text: run.text.get(cluster).unwrap_or_default().to_owned(),
                            glyphs,
                            width,
                            fake_bold: run.fake_bold,
                            fake_italic: run.fake_italic,
                        })],
                        width: width
                            + if piece.capsule.is_some() {
                                2. * CAPSULE_PAD
                            } else {
                                0.
                            },
                        trailing: 0.,
                        breakable: true,
                        mandatory: false,
                        capsule: piece.capsule,
                    });
                    index = end;
                }
            }
        }
    }
    if let Some(last) = output.last_mut() {
        last.trailing = piece.trailing.min(last.width);
        last.breakable = piece.breakable;
        last.mandatory = piece.mandatory;
    }
    output
}

fn shared_inset(left: &Piece, right: &Piece) -> f32 {
    if left.capsule.is_some() && left.capsule == right.capsule {
        2. * CAPSULE_PAD
    } else {
        0.
    }
}

fn fill(fonts: &mut Fonts, paragraph: &Paragraph<'_>, pieces: Vec<Piece>) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    let mut width = 0.;
    let mut queue: VecDeque<Piece> = pieces.into();
    while let Some(piece) = queue.pop_front() {
        let shared = current
            .last()
            .map_or(0., |previous| shared_inset(previous, &piece));
        let visible = piece.width - piece.trailing;
        if !current.is_empty() && width + visible - shared > paragraph.width + 0.01 {
            lines.push(finish(
                fonts,
                paragraph,
                std::mem::take(&mut current),
                false,
            ));
            width = 0.;
        }
        let parts: usize = piece
            .items
            .iter()
            .map(|item| match item {
                Item::Run(run) => run.glyphs.len(),
                Item::Object(_) => 1,
            })
            .sum();
        if current.is_empty() && visible > paragraph.width + 0.01 && parts > 1 {
            for part in split(piece, paragraph).into_iter().rev() {
                queue.push_front(part);
            }
            continue;
        }
        width += piece.width
            - current
                .last()
                .map_or(0., |previous| shared_inset(previous, &piece));
        let mandatory = piece.mandatory;
        current.push(piece);
        if mandatory {
            lines.push(finish(fonts, paragraph, std::mem::take(&mut current), true));
            width = 0.;
        }
    }
    if !current.is_empty() {
        lines.push(finish(fonts, paragraph, current, true));
    }
    lines
}

fn finish(fonts: &mut Fonts, paragraph: &Paragraph<'_>, pieces: Vec<Piece>, last: bool) -> Line {
    let shared: f32 = pieces
        .windows(2)
        .map(|pair| shared_inset(&pair[0], &pair[1]))
        .sum();
    let natural: f32 = pieces.iter().map(|piece| piece.width).sum::<f32>()
        - shared
        - pieces.last().map_or(0., |piece| piece.trailing);
    let joints = pieces
        .windows(2)
        .filter(|pair| pair[0].breakable && shared_inset(&pair[0], &pair[1]) == 0.)
        .count();
    let gap = if paragraph.justify && !last && joints > 0 {
        // Never stretch a sparse line into visibly torn spacing.
        ((paragraph.width - natural) / joints as f32).clamp(0., paragraph.size * 0.8)
    } else {
        0.
    };
    // Half-leading around the paragraph's own line box (CSS `line-height`).
    let line_box = |fonts: &mut Fonts, role, face, size: f32, font_size: f32| {
        let (a, d) = fonts.metrics(role, face, font_size);
        let leading = (size * paragraph.line_height - (a + d)).max(0.);
        (a + leading / 2., d + leading / 2.)
    };
    let (mut ascent, mut descent) =
        line_box(fonts, paragraph.role, None, paragraph.size, paragraph.size);
    let mut items = Vec::new();
    let mut x = 0.;
    let mut previous = None;
    let mut pieces = pieces.into_iter().peekable();
    while let Some(piece) = pieces.next() {
        let capsule = piece.capsule;
        if capsule.is_some() && capsule != previous {
            x += CAPSULE_PAD;
        }
        for item in piece.items {
            let advance = match &item {
                Item::Run(run) => {
                    let Inline::Text(_, style) = &paragraph.inlines[run.inline] else {
                        unreachable!("runs come from text inlines");
                    };
                    let (a, d) = line_box(fonts, style.role, Some(run.face), style.size, run.size);
                    ascent = ascent.max(a + style.raise());
                    descent = descent.max(d - style.raise());
                    run.width
                }
                Item::Object(inline) => {
                    let (width, height, drop) = paragraph.object_size(*inline);
                    if matches!(
                        paragraph.inlines[*inline],
                        Inline::Object {
                            align: ObjectAlign::Baseline(_),
                            ..
                        }
                    ) {
                        ascent = ascent.max(height - drop);
                        descent = descent.max(drop);
                    }
                    width
                }
            };
            items.push(Placed { x, item });
            x += advance;
        }
        let next = pieces.peek().and_then(|piece| piece.capsule);
        if capsule.is_some() && capsule != next {
            x += CAPSULE_PAD;
        }
        if pieces.peek().is_some() && piece.breakable && !(capsule.is_some() && capsule == next) {
            x += gap;
        }
        previous = capsule;
    }
    let ink = items
        .iter()
        .filter_map(|placed| match &placed.item {
            Item::Run(run) => fonts.run_ink(run).map(|ink| {
                let raise = match &paragraph.inlines[run.inline] {
                    Inline::Text(_, style) => style.raise(),
                    _ => 0.,
                };
                Ink {
                    top: ink.top - raise,
                    bottom: ink.bottom - raise,
                }
            }),
            _ => None,
        })
        .reduce(Ink::union);
    for placed in &items {
        if let Item::Object(index) = placed.item
            && let Inline::Object {
                align: ObjectAlign::TextCenter,
                ..
            } = paragraph.inlines[index]
        {
            let (_, height, _) = paragraph.object_size(index);
            let drop = ink.map_or(height / 2., |ink| ink.object_drop(height));
            ascent = ascent.max(height - drop);
            descent = descent.max(drop);
        }
    }
    Line {
        ink,
        items,
        ascent,
        descent,
        width: natural + gap * joints as f32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_symbols_follow_their_wrapped_line_and_preserve_formula_baselines() {
        let mut fonts = Fonts::new("Menlo", "Hiragino Sans GB", "Menlo");
        let raster = || Object::Raster(krilla::image::Image::from_rgba8(vec![255; 4], 1, 1));
        for text in ["中文提示 English", "English with descenders gjpq"] {
            let body = Style::new(Role::Body, 11., [0; 3]);
            let inlines = [
                Inline::Object {
                    object: raster(),
                    width: 11.,
                    height: 11.,
                    align: ObjectAlign::TextCenter,
                    link: None,
                },
                Inline::Text(text.into(), body),
                Inline::Break,
                Inline::Text(
                    "Later line has a much larger font".into(),
                    Style::new(Role::Body, 32., [0; 3]),
                ),
                Inline::Object {
                    object: raster(),
                    width: 14.,
                    height: 14.,
                    align: ObjectAlign::Baseline(3.),
                    link: None,
                },
            ];
            let paragraph = paragraph(&inlines, 160., false);
            let lines = layout(&mut fonts, &paragraph);
            let first = &lines[0];
            let glyph_ink = first
                .items
                .iter()
                .filter_map(|p| match &p.item {
                    Item::Run(run) => fonts.run_ink(run),
                    _ => None,
                })
                .reduce(Ink::union)
                .unwrap();
            let symbol_center = first.object_drop(&paragraph, 0) - 11. / 2.;
            assert!((symbol_center - glyph_ink.center()).abs() < 0.001);
            let formula_line = lines
                .iter()
                .find(|line| {
                    line.items
                        .iter()
                        .any(|item| matches!(item.item, Item::Object(4)))
                })
                .unwrap();
            assert_eq!(formula_line.object_drop(&paragraph, 4), 3.);
        }
    }

    fn paragraph<'a>(inlines: &'a [Inline], width: f32, justify: bool) -> Paragraph<'a> {
        Paragraph {
            inlines,
            role: Role::Body,
            size: 11.,
            width,
            max_height: 700.,
            line_height: 1.6,
            justify,
            preserve: false,
        }
    }

    #[test]
    fn code_capsules_reserve_padding_and_keep_cjk_together() {
        let mut fonts = Fonts::new("Menlo", "PingFang SC", "Menlo");
        let body = Style::new(Role::Body, 11., [0; 3]);
        let mut code = Style::new(Role::Code, 9.68, [0; 3]);
        code.capsule = Some(0);
        let inlines = [
            Inline::Text("左".into(), body.clone()),
            Inline::Text("API接口文档".into(), code),
            Inline::Text("右".into(), body),
        ];
        let lines = layout(&mut fonts, &paragraph(&inlines, 250., true));
        let line = &lines[0];
        let left = &line.items[0];
        let Item::Run(left_run) = &left.item else {
            panic!("text expected")
        };
        let code_start = line
            .items
            .iter()
            .find(|p| matches!(&p.item, Item::Run(run) if run.inline == 1))
            .unwrap();
        assert!(
            code_start.x - left.x - left_run.width >= 2.0,
            "capsule inset must be included in layout, not painted over adjacent text"
        );
        let natural = layout(&mut fonts, &paragraph(&inlines, 70., false));
        assert!(
            natural
                .iter()
                .any(|line| line_text(line).contains("API接口文档")),
            "a fitting mixed-script capsule must wrap as one unit"
        );
        let text = "very_long_API_接口_".repeat(8);
        let inlines = [Inline::Text(text.clone(), code_style())];
        for width in [24., 70., 250.] {
            let lines = layout(&mut fonts, &paragraph(&inlines, width, true));
            assert!(lines.iter().all(|line| line.width <= width + 0.01));
            assert_eq!(lines.iter().map(line_text).collect::<String>(), text);
        }
    }

    fn code_style() -> Style {
        let mut style = Style::new(Role::Code, 9.68, [0; 3]);
        style.capsule = Some(0);
        style
    }

    #[test]
    fn mixed_text_wraps_within_width_with_cjk_punctuation_rules() {
        let mut fonts = Fonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_MONO_FONT,
        );
        let text = "终端侧已将 DataService（DS）、RemoteControl（RC）等多套定位实现收敛为统一的定位 SDK，并约定：配置整包替换。".repeat(3);
        let inlines = [Inline::Text(
            text.clone(),
            Style::new(Role::Body, 11., [0; 3]),
        )];
        let lines = layout(&mut fonts, &paragraph(&inlines, 200., true));
        assert!(lines.len() > 3, "long prose wraps: {}", lines.len());
        for (index, line) in lines.iter().enumerate() {
            assert!(line.width <= 200.5, "line {index} fits: {}", line.width);
            let first = line.items.iter().find_map(|placed| match &placed.item {
                Item::Run(run) => run.text.chars().next(),
                Item::Object(_) => None,
            });
            assert!(
                !matches!(first, Some('，' | '。' | '）' | '、' | '：')),
                "line {index} must not start with closing punctuation"
            );
        }
        let text: String = lines
            .iter()
            .flat_map(|line| &line.items)
            .filter_map(|placed| match &placed.item {
                Item::Run(run) => Some(run.text.as_str()),
                Item::Object(_) => None,
            })
            .collect();
        assert_eq!(
            text.replace(' ', ""),
            inlines_text(&inlines).replace(' ', ""),
            "no text is lost"
        );
        let natural = layout(&mut fonts, &paragraph(&inlines, 200., false));
        let mut reaches_measure = false;
        for (index, (line, natural)) in lines.iter().zip(&natural).enumerate() {
            assert_eq!(
                line.items.len(),
                natural.items.len(),
                "line {index} contents"
            );
            assert!(
                line.width + 0.01 >= natural.width,
                "line {index} never shrinks"
            );
            let mut previous = 0.;
            for (item, original) in line.items.iter().zip(&natural.items) {
                let extra = item.x - original.x;
                assert!(
                    (previous - 0.01..=previous + 11. * 0.8 + 0.01).contains(&extra),
                    "line {index} has bounded spacing: {previous} -> {extra}"
                );
                previous = extra;
            }
            reaches_measure |= index + 1 < lines.len() && (line.width - 200.).abs() < 0.5;
            let contents = line_text(line);
            for word in ["DataService", "RemoteControl"] {
                assert!(
                    !contents.contains(&word[..4]) || contents.contains(word),
                    "line {index} keeps {word} together: {contents}"
                );
            }
        }
        assert!(reaches_measure, "dense justified lines reach the measure");
        assert_eq!(
            lines.last().unwrap().width,
            natural.last().unwrap().width,
            "the final line keeps natural spacing"
        );
    }

    fn line_text(line: &Line) -> String {
        line.items
            .iter()
            .filter_map(|placed| match &placed.item {
                Item::Run(run) => Some(run.text.as_str()),
                Item::Object(_) => None,
            })
            .collect()
    }

    fn inlines_text(inlines: &[Inline]) -> String {
        inlines
            .iter()
            .filter_map(|inline| match inline {
                Inline::Text(text, _) => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn soft_breaks_vanish_between_cjk_and_over_wide_words_split() {
        let mut fonts = Fonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_MONO_FONT,
        );
        let style = Style::new(Role::Body, 11., [0; 3]);
        let inlines = [Inline::Text(
            format!("中文{SOFT_BREAK}继续 English{SOFT_BREAK}words"),
            style.clone(),
        )];
        let (text, _) = flatten(&paragraph(&inlines, 500., false));
        assert_eq!(text, "中文继续 English words");
        let long = [Inline::Text("https://example.com/".repeat(10), style)];
        let lines = layout(&mut fonts, &paragraph(&long, 120., false));
        assert!(lines.len() > 1 && lines.iter().all(|line| line.width <= 120.5));
        let (minimum, natural) = measure(&mut fonts, &paragraph(&inlines, 500., false));
        assert!(minimum > 0. && natural > minimum);
    }

    #[test]
    fn split_keeps_multibyte_clusters_glyphs_and_style() {
        let glyph = |id, cluster| Glyph {
            id,
            cluster,
            advance: 0.5,
            x_offset: 0.1,
            y_offset: 0.2,
        };
        let run = Run {
            face: 3,
            inline: 7,
            size: 11.,
            text: "e\u{301}中ff".into(),
            glyphs: vec![
                glyph(1, 0..3),
                glyph(2, 0..3),
                glyph(3, 3..6),
                glyph(4, 6..8),
            ],
            width: 22.,
            fake_bold: true,
            fake_italic: true,
        };
        let piece = Piece {
            items: vec![Item::Run(run)],
            width: 22.,
            trailing: 0.,
            breakable: true,
            mandatory: true,
            capsule: None,
        };
        let pieces = split(piece, &paragraph(&[], 20., false));
        assert_eq!(
            pieces.len(),
            3,
            "multiple glyphs in one cluster stay together"
        );
        let runs: Vec<&Run> = pieces
            .iter()
            .flat_map(|piece| &piece.items)
            .filter_map(|item| match item {
                Item::Run(run) => Some(run),
                Item::Object(_) => None,
            })
            .collect();
        assert_eq!(
            runs.iter().map(|run| run.text.as_str()).collect::<String>(),
            "e\u{301}中ff"
        );
        assert_eq!(runs.iter().map(|run| run.glyphs.len()).sum::<usize>(), 4);
        assert_eq!(runs.iter().map(|run| run.width).sum::<f32>(), 22.);
        for run in runs {
            assert_eq!((run.face, run.inline, run.size), (3, 7, 11.));
            assert!(run.fake_bold && run.fake_italic);
            assert!(
                run.glyphs
                    .iter()
                    .all(|glyph| glyph.cluster == (0..run.text.len()))
            );
        }
        assert!(
            pieces.last().unwrap().mandatory,
            "the hard break survives splitting"
        );
    }

    #[test]
    fn long_single_token_wraps_without_losing_text() {
        let mut fonts = Fonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_MONO_FONT,
        );
        let contents = "abcdefghijklmnop".repeat(1000);
        let inlines = [Inline::Text(
            contents.clone(),
            Style::new(Role::Code, 11., [0; 3]),
        )];
        let mut paragraph = paragraph(&inlines, 180., false);
        paragraph.role = Role::Code;
        paragraph.preserve = true;
        let lines = layout(&mut fonts, &paragraph);
        assert!(lines.len() > 100, "the entire long token is wrapped");
        assert!(
            lines.iter().all(|line| line.width <= 180.5),
            "wrapped lines fit"
        );
        assert_eq!(lines.iter().map(line_text).collect::<String>(), contents);
    }

    #[test]
    fn inline_object_fits_page_height_without_changing_aspect_ratio() {
        let inlines = [Inline::Object {
            object: Object::Raster(krilla::image::Image::from_rgba8(vec![255; 4], 1, 1)),
            width: 5.,
            height: 1500.,
            align: ObjectAlign::Baseline(0.),
            link: None,
        }];
        let paragraph = paragraph(&inlines, 200., false);
        let (width, height, _) = paragraph.object_size(0);
        assert!(height < 700. && height > 600.);
        assert!((width / height - 5. / 1500.).abs() < 0.0001);
        let mut fonts = Fonts::new(
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_UI_FONT,
            pecan_mermaid::SYSTEM_MONO_FONT,
        );
        let lines = layout(&mut fonts, &paragraph);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].height() <= paragraph.max_height,
            "the complete line box fits"
        );
    }
}
