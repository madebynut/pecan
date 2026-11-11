//! Shared font fallback, shaping and visible glyph metrics for HTML, Word and PDF.
//! Measurements happen during export, never during desktop scrolling.
use krilla::text::{Font, GlyphId, KrillaGlyph};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
    sync::Arc,
};
use usvg::fontdb;

/// Font families tried after the user's choices, covering macOS, Windows and Linux.
const SANS: &[&str] = &[
    "PingFang SC",
    "Hiragino Sans GB",
    "Microsoft YaHei",
    "Microsoft YaHei UI",
    "Noto Sans CJK SC",
    "Source Han Sans SC",
    "Helvetica Neue",
    "Segoe UI",
    "Arial",
    "Arial Unicode MS",
    "Apple Symbols",
    "Segoe UI Symbol",
    "Apple Color Emoji",
    "Segoe UI Emoji",
    "Noto Color Emoji",
];
const MONO: &[&str] = &[
    "SF Mono",
    "Menlo",
    "Consolas",
    "Cascadia Mono",
    "DejaVu Sans Mono",
    "Courier New",
];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Role {
    Body,
    Code,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Style {
    pub(crate) role: Role,
    pub(crate) size: f32,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) color: [u8; 3],
    pub(crate) strike: bool,
    /// Inline code capsule behind the text.
    pub(crate) capsule: Option<usize>,
    pub(crate) link: Option<Arc<str>>,
    pub(crate) superscript: bool,
}

impl Style {
    pub(crate) fn new(role: Role, size: f32, color: [u8; 3]) -> Self {
        Self {
            role,
            size,
            bold: false,
            italic: false,
            color,
            strike: false,
            capsule: None,
            link: None,
            superscript: false,
        }
    }
    pub(crate) fn glyph_size(&self) -> f32 {
        if self.superscript {
            self.size * 0.7
        } else {
            self.size
        }
    }
    pub(crate) fn raise(&self) -> f32 {
        if self.superscript {
            self.size * 0.35
        } else {
            0.
        }
    }
}

/// Visible vertical extent in y-down coordinates, relative to the baseline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ink {
    pub(crate) top: f32,
    pub(crate) bottom: f32,
}
impl Ink {
    pub(crate) fn union(self, other: Self) -> Self {
        Self {
            top: self.top.min(other.top),
            bottom: self.bottom.max(other.bottom),
        }
    }
    pub(crate) fn center(self) -> f32 {
        (self.top + self.bottom) / 2.
    }
    pub(crate) fn object_drop(self, height: f32) -> f32 {
        self.center() + height / 2.
    }
}

#[derive(Default)]
pub(crate) struct TextMetrics {
    pub(crate) ink: Option<Ink>,
    pub(crate) ascent: f32,
    pub(crate) descent: f32,
}

pub(crate) struct Face {
    pub(crate) font: Font,
    data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    index: u32,
    units_per_em: f32,
    ascender: f32,
    descender: f32,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
}

pub(crate) struct Fonts {
    database: fontdb::Database,
    pub(crate) faces: Vec<Face>,
    by_id: HashMap<fontdb::ID, usize>,
    chains: HashMap<(Role, bool, bool), Vec<usize>>,
    choices: HashMap<(Role, bool, bool, char), usize>,
    families: HashMap<Role, Vec<String>>,
}

impl Fonts {
    pub(crate) fn new(english: &str, chinese: &str, code: &str) -> Self {
        let body_fonts = families(english, chinese);
        let code_fonts = families(code, chinese);
        let mut body = vec![
            body_fonts.english_family().to_owned(),
            body_fonts.chinese_family().to_owned(),
        ];
        body.extend(SANS.iter().map(|family| (*family).to_owned()));
        let mut code_families = vec![code_fonts.english_family().to_owned()];
        code_families.extend(MONO.iter().map(|family| (*family).to_owned()));
        code_families.push(code_fonts.chinese_family().to_owned());
        code_families.extend(SANS.iter().map(|family| (*family).to_owned()));
        let unique = |list: Vec<String>| {
            let mut seen = HashSet::new();
            list.into_iter()
                .filter(|family| seen.insert(family.clone()))
                .collect::<Vec<_>>()
        };
        Self {
            // A private copy: sharing mapped files rewrites face sources.
            database: (*pecan_mermaid::system_fonts()).clone(),
            faces: Vec::new(),
            by_id: HashMap::new(),
            chains: HashMap::new(),
            choices: HashMap::new(),
            families: HashMap::from([
                (Role::Body, unique(body)),
                (Role::Code, unique(code_families)),
            ]),
        }
    }

    pub(crate) fn font(&self, face: usize) -> &Font {
        &self.faces[face].font
    }

    /// Ascent and descent of the role's primary face at `size`, before leading.
    pub(crate) fn metrics(&mut self, role: Role, face: Option<usize>, size: f32) -> (f32, f32) {
        let face = face.or_else(|| self.chain(role, false, false).first().copied());
        face.map_or((size * 0.8, size * 0.2), |face| {
            let face = &self.faces[face];
            (face.ascender * size, -face.descender * size)
        })
    }

    fn chain(&mut self, role: Role, bold: bool, italic: bool) -> Vec<usize> {
        if let Some(chain) = self.chains.get(&(role, bold, italic)) {
            return chain.clone();
        }
        let families = self.families[&role].clone();
        let mut chain = Vec::new();
        for family in &families {
            let query = fontdb::Query {
                families: &[fontdb::Family::Name(family)],
                weight: if bold {
                    fontdb::Weight::BOLD
                } else {
                    fontdb::Weight::NORMAL
                },
                stretch: fontdb::Stretch::Normal,
                style: if italic {
                    fontdb::Style::Italic
                } else {
                    fontdb::Style::Normal
                },
            };
            if let Some(id) = self.database.query(&query)
                && let Some(face) = self.load(id)
                && !chain.contains(&face)
            {
                chain.push(face);
            }
        }
        self.chains.insert((role, bold, italic), chain.clone());
        chain
    }

    fn load(&mut self, id: fontdb::ID) -> Option<usize> {
        if let Some(&face) = self.by_id.get(&id) {
            return Some(face);
        }
        let info = self.database.face(id)?;
        let (bold, italic) = (info.weight.0 >= 600, info.style != fontdb::Style::Normal);
        // SAFETY: system font files are installed read-only by the OS and are not
        // modified while an export holds the mapping; the database copy is private.
        let (data, index) = unsafe { self.database.make_shared_face_data(id)? };
        let parsed = ttf_parser::Face::parse((*data).as_ref(), index).ok()?;
        let units_per_em = f32::from(parsed.units_per_em());
        let face = Face {
            font: Font::new(data.clone().into(), index)?,
            ascender: f32::from(parsed.ascender()) / units_per_em,
            descender: f32::from(parsed.descender()) / units_per_em,
            units_per_em,
            data,
            index,
            bold,
            italic,
        };
        self.faces.push(face);
        self.by_id.insert(id, self.faces.len() - 1);
        Some(self.faces.len() - 1)
    }

    /// The first face in the chain with a glyph for `character`.
    pub(crate) fn choose(&mut self, style: &Style, character: char) -> Option<usize> {
        let key = (style.role, style.bold, style.italic, character);
        if let Some(&face) = self.choices.get(&key) {
            return Some(face);
        }
        let chain = self.chain(style.role, style.bold, style.italic);
        let face = chain
            .iter()
            .copied()
            .find(|&face| {
                let face = &self.faces[face];
                ttf_parser::Face::parse((*face.data).as_ref(), face.index)
                    .ok()
                    .and_then(|parsed| parsed.glyph_index(character))
                    .is_some()
            })
            .or_else(|| chain.first().copied())?;
        self.choices.insert(key, face);
        Some(face)
    }

    fn glyph_ink(&self, face: usize, glyphs: &[Glyph], size: f32) -> Option<Ink> {
        let face = &self.faces[face];
        let parsed = ttf_parser::Face::parse((*face.data).as_ref(), face.index).ok()?;
        glyphs
            .iter()
            .filter_map(|glyph| {
                let id = u16::try_from(glyph.id).ok()?;
                let bounds = parsed.glyph_bounding_box(ttf_parser::GlyphId(id))?;
                let scale = size / face.units_per_em;
                Some(Ink {
                    top: -f32::from(bounds.y_max) * scale - glyph.y_offset * size,
                    bottom: -f32::from(bounds.y_min) * scale - glyph.y_offset * size,
                })
            })
            .reduce(Ink::union)
    }

    pub(crate) fn run_ink(&self, run: &Run) -> Option<Ink> {
        self.glyph_ink(run.face, &run.glyphs, run.size)
    }

    /// Font fallback and shaping match PDF. Measurements are export-time only.
    pub(crate) fn text_ink(&mut self, text: &str, style: &Style) -> Option<Ink> {
        self.measure_text(text, style).ink
    }

    pub(crate) fn measure_text(&mut self, text: &str, style: &Style) -> TextMetrics {
        let mut metrics = TextMetrics::default();
        let mut rest = text;
        while let Some(character) = rest.chars().next() {
            let Some(face) = self.choose(style, character) else {
                break;
            };
            let mut end = character.len_utf8();
            for (index, next) in rest.char_indices().skip(1) {
                if self.choose(style, next) != Some(face) {
                    break;
                }
                end = index + next.len_utf8();
            }
            let size = style.glyph_size();
            let (ascent, descent) = self.metrics(style.role, Some(face), size);
            metrics.ascent = metrics.ascent.max(ascent);
            metrics.descent = metrics.descent.max(descent);
            let glyphs = self.shape(face, &rest[..end], size);
            if let Some(ink) = self.glyph_ink(face, &glyphs, size) {
                metrics.ink = Some(metrics.ink.map_or(ink, |old| old.union(ink)));
            }
            rest = &rest[end..];
        }
        metrics
    }

    pub(crate) fn shape(&self, face: usize, text: &str, size: f32) -> Vec<Glyph> {
        let face = &self.faces[face];
        let Some(shaper) = rustybuzz::Face::from_slice((*face.data).as_ref(), face.index) else {
            return Vec::new();
        };
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let output = rustybuzz::shape(&shaper, &[], buffer);
        let units = face.units_per_em;
        let mut glyphs: Vec<Glyph> = output
            .glyph_infos()
            .iter()
            .zip(output.glyph_positions())
            .map(|(info, position)| Glyph {
                id: info.glyph_id,
                cluster: info.cluster as usize..text.len(),
                advance: position.x_advance as f32 / units,
                x_offset: position.x_offset as f32 / units,
                y_offset: position.y_offset as f32 / units,
            })
            .collect();
        // A cluster ends where the next distinct cluster starts (left-to-right text).
        let mut end = text.len();
        for index in (0..glyphs.len()).rev() {
            let start = glyphs[index].cluster.start;
            glyphs[index].cluster.end = end.max(start);
            if index > 0 && glyphs[index - 1].cluster.start != start {
                end = start;
            }
        }
        let _ = size;
        glyphs
    }
}

/// Resolve aliases once in the same order used by the native reader.
pub(crate) fn families(english: &str, chinese: &str) -> pecan_mermaid::DiagramFonts {
    pecan_mermaid::DiagramFonts::new(
        if english.is_empty() {
            pecan_mermaid::SYSTEM_MONO_FONT
        } else {
            english
        },
        if chinese.is_empty() {
            pecan_mermaid::SYSTEM_UI_FONT
        } else {
            chinese
        },
    )
}

#[derive(Clone, Debug)]
pub(crate) struct Glyph {
    pub(crate) id: u32,
    pub(crate) cluster: Range<usize>,
    pub(crate) advance: f32,
    pub(crate) x_offset: f32,
    pub(crate) y_offset: f32,
}

/// Glyphs sharing one face and one inline style.
#[derive(Clone)]
pub(crate) struct Run {
    pub(crate) face: usize,
    /// Index of the text inline whose style applies.
    pub(crate) inline: usize,
    pub(crate) size: f32,
    pub(crate) text: String,
    pub(crate) glyphs: Vec<Glyph>,
    pub(crate) width: f32,
    /// Synthesized when the chosen face lacks the requested weight or slant.
    pub(crate) fake_bold: bool,
    pub(crate) fake_italic: bool,
}

impl Run {
    pub(crate) fn krilla_glyphs(&self) -> Vec<KrillaGlyph> {
        self.glyphs
            .iter()
            .map(|glyph| {
                KrillaGlyph::new(
                    GlyphId::new(glyph.id),
                    glyph.advance,
                    glyph.x_offset,
                    -glyph.y_offset,
                    0.,
                    glyph.cluster.clone(),
                    None,
                )
            })
            .collect()
    }
}
