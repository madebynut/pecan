//! Bounded native LaTeX, mhchem and chemfig rendering with embedded OpenType math fonts.

use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use lru::LruCache;
use resvg::{tiny_skia, usvg};
use thiserror::Error;

const MAX_SOURCE: usize = 4096;
const MAX_DEPTH: usize = 32;
const MAX_SVG: usize = 1024 * 1024;
const MAX_PIXELS: u32 = 262_144;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;

/// A genuine OpenType math font, independent of body and interface fonts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MathFont {
    /// STIX Two Math, bundled under the SIL Open Font License.
    #[default]
    StixTwo,
    /// Latin Modern Math, bundled under the GUST Font License.
    LatinModern,
}

/// Inline or centered display math layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MathStyle {
    /// Compact math within prose.
    #[default]
    Inline,
    /// Display equations with larger operators.
    Display,
}

/// Presentation options included in the formula cache identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Presentation {
    /// Math font used for glyph geometry and layout metrics.
    pub font: MathFont,
    /// Inline or display layout.
    pub style: MathStyle,
    /// Logical font size; accepted range is 10–64 pixels.
    pub font_size: u8,
    /// RGB ink color, without alpha.
    pub color: u32,
}

impl Default for Presentation {
    fn default() -> Self {
        Self {
            font: MathFont::default(),
            style: MathStyle::Inline,
            font_size: 18,
            color: 0x222222,
        }
    }
}

/// A recoverable formula syntax or resource-budget error.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderMathError {
    /// Formula input or output exceeds the bounded reader budget.
    #[error("formula exceeds resource budget: {0}")]
    Budget(&'static str),
    /// Unsupported commands or malformed LaTeX.
    #[error("invalid LaTeX: {0}")]
    Syntax(#[from] mathra::Error),
    /// Invalid generated SVG.
    #[error("invalid formula SVG: {0}")]
    Svg(#[from] usvg::Error),
}

/// Prepared PNG and metrics; clones share immutable buffers.
#[derive(Debug, Clone)]
pub struct Formula {
    png: Arc<[u8]>,
    svg: Arc<str>,
    width: f32,
    height: f32,
    baseline: f32,
    speech: Arc<str>,
    raster_bytes: usize,
}

impl Formula {
    /// Retina-resolution PNG with transparent background.
    pub fn png(&self) -> &[u8] {
        &self.png
    }
    /// Vector formula with glyph outlines for resolution-independent document exports.
    pub fn svg(&self) -> &str {
        &self.svg
    }
    /// Logical width and height, capped to a bounded preview.
    pub fn dimensions(&self) -> (f32, f32) {
        (self.width, self.height)
    }
    /// Distance from the top edge to the math baseline in logical pixels.
    pub fn baseline(&self) -> f32 {
        self.baseline
    }
    /// Decoded RGBA pixel cost, used to bound retained UI previews.
    pub fn raster_bytes(&self) -> usize {
        self.raster_bytes
    }
    /// Accessible spoken form produced from the formula syntax tree.
    pub fn speech(&self) -> &str {
        &self.speech
    }
}

struct Cache {
    entries: LruCache<[u8; 32], Formula>,
    bytes: usize,
}

/// Shared bounded raster cache; callers execute CPU work on their own background executor.
#[derive(Clone)]
pub struct Renderer {
    cache: Arc<Mutex<Cache>>,
}

impl fmt::Debug for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Renderer")
            .field("resident_bytes", &self.resident_bytes())
            .finish()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Create an empty cache without loading fonts or doing I/O.
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(Cache {
                entries: LruCache::new(
                    NonZeroUsize::new(CACHE_ENTRIES).expect("math cache capacity is positive"),
                ),
                bytes: 0,
            })),
        }
    }
    /// Total cached vector, PNG and accessible-label bytes.
    pub fn resident_bytes(&self) -> usize {
        self.cache.lock().expect("math cache lock poisoned").bytes
    }

    /// Typeset LaTeX and chemical notation into a bounded, transparent PNG.
    ///
    /// # Errors
    /// Rejects excessive source, nesting, syntax, font sizes and generated geometry.
    pub fn render(
        &self,
        source: &str,
        presentation: Presentation,
    ) -> Result<Formula, RenderMathError> {
        validate(source)?;
        if !(10..=64).contains(&presentation.font_size) {
            return Err(RenderMathError::Budget("font size"));
        }
        let mut hash = blake3::Hasher::new();
        hash.update(source.as_bytes());
        hash.update(&[
            presentation.font as u8,
            presentation.style as u8,
            presentation.font_size,
        ]);
        hash.update(&presentation.color.to_le_bytes());
        let key = *hash.finalize().as_bytes();
        if let Some(formula) = self
            .cache
            .lock()
            .expect("math cache lock poisoned")
            .entries
            .get(&key)
        {
            return Ok(formula.clone());
        }
        let font = match presentation.font {
            MathFont::StixTwo => mathra::font::MathFont::StixTwo,
            MathFont::LatinModern => mathra::font::MathFont::LatinModern,
        };
        let output = mathra::convert_in_font(
            source,
            &mathra::Options {
                strict: true,
                display: presentation.style == MathStyle::Display,
                ..Default::default()
            },
            font,
        )?;
        if output.svg.len() > MAX_SVG {
            return Err(RenderMathError::Budget("SVG bytes"));
        }
        // usvg does not resolve mathra's `svg:root` CSS selector. Resolve the
        // inherited ink explicitly; user-specified LaTeX colors remain intact.
        let svg = output.svg.replace(
            "currentColor",
            &format!("#{:06x}", presentation.color & 0xffffff),
        );
        let options = usvg::Options {
            font_size: f32::from(presentation.font_size),
            ..Default::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options)?;
        let size = tree.size();
        let (w, h) = (size.width(), size.height());
        if !w.is_finite() || !h.is_finite() || w <= 0. || h <= 0. || w > 16384. || h > 16384. {
            return Err(RenderMathError::Budget("formula dimensions"));
        }
        let scale = 2_f32
            .min(2048. / w.max(h))
            .min((MAX_PIXELS as f32 / (w * h)).sqrt());
        let pw = (w * scale).ceil().max(1.) as u32;
        let ph = (h * scale).ceil().max(1.) as u32;
        if pw.saturating_mul(ph) > MAX_PIXELS + 4096 {
            return Err(RenderMathError::Budget("raster pixels"));
        }
        let mut pixmap =
            tiny_skia::Pixmap::new(pw, ph).ok_or(RenderMathError::Budget("raster allocation"))?;
        resvg::render(
            &tree,
            tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        let png = pixmap
            .encode_png()
            .map_err(|_| RenderMathError::Budget("PNG encoding"))?;
        let depth = svg
            .split("vertical-align: ")
            .nth(1)
            .and_then(|s| s.split("em").next())
            .and_then(|s| s.parse::<f32>().ok())
            .map_or(0., |s| -s * f32::from(presentation.font_size));
        let display_scale = (780. / w).min(300. / h).min(1.);
        let formula = Formula {
            png: png.into(),
            svg: svg.into(),
            width: w * display_scale,
            height: h * display_scale,
            baseline: (h - depth).clamp(0., h) * display_scale,
            speech: output.speech.into(),
            raster_bytes: pw as usize * ph as usize * 4,
        };
        let mut cache = self.cache.lock().expect("math cache lock poisoned");
        // Concurrent misses may finish in either order; avoid accounting the same key twice.
        if let Some(existing) = cache.entries.get(&key) {
            return Ok(existing.clone());
        }
        let cost = formula.png.len() + formula.svg.len() + formula.speech.len();
        while cache.bytes + cost > CACHE_BYTES || cache.entries.len() == CACHE_ENTRIES {
            if let Some((_, evicted)) = cache.entries.pop_lru() {
                cache.bytes -= evicted.png.len() + evicted.svg.len() + evicted.speech.len();
            } else {
                break;
            }
        }
        cache.bytes += cost;
        cache.entries.put(key, formula.clone());
        Ok(formula)
    }
}

fn validate(source: &str) -> Result<(), RenderMathError> {
    if source.len() > MAX_SOURCE {
        return Err(RenderMathError::Budget("source exceeds 4096 bytes"));
    }
    let mut depth = 0_usize;
    let mut commands = 0_usize;
    let mut escaped = false;
    for ch in source.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                commands += 1;
            }
            '{' | '(' | '[' => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return Err(RenderMathError::Budget("nesting exceeds 32 levels"));
                }
            }
            '}' | ')' | ']' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    if commands > 512 {
        return Err(RenderMathError::Budget("too many commands"));
    }
    for command in [
        "\\def",
        "\\newcommand",
        "\\renewcommand",
        "\\loop",
        "\\input",
        "\\include",
    ] {
        if source.contains(command) {
            return Err(RenderMathError::Budget(
                "macro definitions and file inclusion",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ink_reaches_glyphs_rules_and_chemical_bonds() {
        let renderer = Renderer::new();
        for source in [r"x^2+\alpha", r"\frac{a}{b}", r"\chemfig{H-C-H}"] {
            for color in [0x242424, 0xeeeeee] {
                let formula = renderer
                    .render(
                        source,
                        Presentation {
                            color,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                let pixels = tiny_skia::Pixmap::decode_png(formula.png()).unwrap();
                let opaque: Vec<_> = pixels
                    .pixels()
                    .iter()
                    .filter(|pixel| pixel.alpha() > 240)
                    .collect();
                assert!(
                    !opaque.is_empty(),
                    "formula must contain visible ink: {source}"
                );
                let channel = (color & 0xff) as u8;
                assert!(
                    opaque.iter().all(|pixel| pixel.red().abs_diff(channel) < 16
                        && pixel.green().abs_diff(channel) < 16
                        && pixel.blue().abs_diff(channel) < 16),
                    "theme must color glyphs and rules: {source}, {color:x}"
                );
            }
        }
    }

    #[test]
    fn renders_math_chemistry_and_both_fonts() {
        let renderer = Renderer::new();
        for source in [
            r"\frac{-b\pm\sqrt{b^2-4ac}}{2a}",
            r"\sum_{n=1}^{\infty}\frac{1}{n^2}",
            r"\begin{pmatrix}a&b\\c&d\end{pmatrix}",
            r"\ce{N2 + 3H2 <=> 2NH3}",
            r"\chemfig{H_3C-*6(=-=-=-)}",
        ] {
            let formula = renderer
                .render(
                    source,
                    Presentation {
                        style: MathStyle::Display,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(&formula.png()[..8], b"\x89PNG\r\n\x1a\n", "{source}");
            assert!(
                formula.dimensions().0 > 0. && formula.baseline() >= 0.,
                "{source}"
            );
        }
        let stix = renderer
            .render(r"x^2+\alpha", Presentation::default())
            .unwrap();
        let modern = renderer
            .render(
                r"x^2+\alpha",
                Presentation {
                    font: MathFont::LatinModern,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(
            stix.png(),
            modern.png(),
            "font selection must change actual glyphs"
        );
        assert!(Arc::ptr_eq(
            &stix.png,
            &renderer
                .render(r"x^2+\alpha", Presentation::default())
                .unwrap()
                .png
        ));
    }

    #[test]
    fn malformed_and_adversarial_math_are_recoverable() {
        let renderer = Renderer::new();
        for source in [
            "{".repeat(10000),
            format!("{}x{}", "{".repeat(40), "}".repeat(40)),
            r"\unsupportedcommand{x}".into(),
        ] {
            assert!(
                renderer.render(&source, Presentation::default()).is_err(),
                "{source}"
            );
        }
        for n in 0..160 {
            renderer
                .render(&format!("x_{{{n}}}"), Presentation::default())
                .unwrap();
        }
        assert!(renderer.resident_bytes() <= CACHE_BYTES);
    }
}

mod queue;
pub use queue::RenderQueue;
