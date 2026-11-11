use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

use crate::fonts::DiagramFonts;
use lru::LruCache;
use mermaid_rs_renderer::{RenderOptions, Theme};
use resvg::{tiny_skia, usvg};
use thiserror::Error;

/// Diagram source and output caps prevent a single fence exhausting reader memory.
const MAX_SOURCE_BYTES: usize = 128 * 1024;
const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
const CACHE_BYTES: usize = 32 * 1024 * 1024;
const CACHE_ENTRIES: usize = 64;
/// Whole-diagram rasters stop here; viewport regions may reach [`Region::MAX_EDGE`].
const MAX_RASTER_EDGE: u32 = 4096;

/// Color palette, included in diagram cache identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Appearance {
    /// Light reading canvas.
    #[default]
    Light,
    /// Dark reading canvas.
    Dark,
}

/// A native diagram parsing, rendering, or resource-budget failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RenderDiagramError {
    /// Source or rendered output exceeds the reader's budget.
    #[error("diagram exceeds resource budget: {0}")]
    Budget(&'static str),
    /// Native Mermaid rejected this source.
    #[error("unable to render Mermaid: {0}")]
    Syntax(String),
    /// The renderer produced SVG that the rasterizer could not read.
    #[error("unable to decode diagram SVG: {0}")]
    Svg(#[from] usvg::Error),
    /// Generated SVG text markup could not be read.
    #[error("unable to read diagram text: {0}")]
    TextMarkup(#[from] roxmltree::Error),
    /// The requested preview dimensions are invalid.
    #[error("invalid diagram preview size")]
    InvalidSize,
}

/// An SVG with parsed vector geometry ready for repeated zoom rasterization.
#[derive(Clone)]
pub struct Diagram {
    inner: Arc<DiagramData>,
}

struct DiagramData {
    svg: String,
    tree: usvg::Tree,
    render_time: Duration,
}

impl fmt::Debug for Diagram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Diagram")
            .field("svg_bytes", &self.svg().len())
            .field("size", &self.dimensions())
            .field("render_time", &self.render_time())
            .finish()
    }
}

impl Diagram {
    /// SVG output suitable for vector export.
    pub fn svg(&self) -> &str {
        &self.inner.svg
    }
    /// Parsed vector geometry, for vector exports such as PDF.
    pub fn tree(&self) -> &usvg::Tree {
        &self.inner.tree
    }
    /// Intrinsic diagram width and height in logical pixels.
    pub fn dimensions(&self) -> (f32, f32) {
        let size = self.inner.tree.size();
        (size.width(), size.height())
    }
    /// Time spent generating and decoding this SVG, excluding cache lookup.
    pub fn render_time(&self) -> Duration {
        self.inner.render_time
    }
    /// Rasterize vector geometry to PNG at a requested width, with bounded allocation.
    ///
    /// # Errors
    /// Rejects non-finite or non-positive widths, allocation and PNG encoding failures.
    pub fn png(&self, requested_width: f32) -> Result<Vec<u8>, RenderDiagramError> {
        if !requested_width.is_finite() || requested_width <= 0.0 {
            return Err(RenderDiagramError::InvalidSize);
        }
        let scale = (requested_width / self.dimensions().0).min(self.max_scale());
        self.png_region(Region::whole(self, scale), None)
    }
    /// Largest scale whose whole-diagram raster, after rounding to whole pixels,
    /// fits the pixel budget.
    pub fn max_scale(&self) -> f32 {
        let (width, height) = self.dimensions();
        let extent = width.max(height);
        let mut scale = (MAX_RASTER_EDGE as f32 / extent)
            .min((Region::MAX_PIXELS as f32 / (width * height)).sqrt());
        // Rounding up can add a row and a column; shed one long-edge pixel at a time.
        // Terminates because a 1×1 raster always fits.
        while !Region::whole(self, scale).within(MAX_RASTER_EDGE) {
            scale -= 1. / extent;
        }
        scale
    }
    /// Rasterize a region to PNG. An opaque background keeps antialiased edges from
    /// blending with transparent black when the GPU filters the texture.
    ///
    /// # Errors
    /// Rejects invalid or over-budget regions and PNG encoding failures.
    pub fn png_region(
        &self,
        region: Region,
        background: Option<[u8; 3]>,
    ) -> Result<Vec<u8>, RenderDiagramError> {
        encode_png(self.pixmap(region, background)?)
            .map_err(|_| RenderDiagramError::Budget("PNG encoding"))
    }
    /// Rasterize a region onto an opaque background as BGRA rows for direct GPU upload.
    ///
    /// # Errors
    /// Rejects invalid or over-budget regions.
    pub fn bgra_region(
        &self,
        region: Region,
        background: [u8; 3],
    ) -> Result<Vec<u8>, RenderDiagramError> {
        let mut pixels = self.pixmap(region, Some(background))?.take();
        // Opaque pixels are identical premultiplied and straight.
        for [red, _, blue, _] in pixels.as_chunks_mut::<4>().0 {
            std::mem::swap(red, blue);
        }
        Ok(pixels)
    }
    fn pixmap(
        &self,
        region: Region,
        background: Option<[u8; 3]>,
    ) -> Result<tiny_skia::Pixmap, RenderDiagramError> {
        if !region.scale.is_finite()
            || region.scale <= 0.0
            || region.width == 0
            || region.height == 0
        {
            return Err(RenderDiagramError::InvalidSize);
        }
        if !region.within(Region::MAX_EDGE) {
            return Err(RenderDiagramError::Budget("raster region"));
        }
        let mut pixmap = tiny_skia::Pixmap::new(region.width, region.height)
            .ok_or(RenderDiagramError::Budget("pixel allocation"))?;
        if let Some([red, green, blue]) = background {
            pixmap.fill(tiny_skia::Color::from_rgba8(red, green, blue, 255));
        }
        resvg::render(
            &self.inner.tree,
            tiny_skia::Transform::from_row(
                region.scale,
                0.,
                0.,
                region.scale,
                -(region.x as f32),
                -(region.y as f32),
            ),
            &mut pixmap.as_mut(),
        );
        Ok(pixmap)
    }
}

/// Output pixels `x..x + width`, `y..y + height` of a diagram drawn at `scale`
/// pixels per SVG unit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    /// Output pixels per SVG unit.
    pub scale: f32,
    /// Left edge in output pixels.
    pub x: u32,
    /// Top edge in output pixels.
    pub y: u32,
    /// Width in output pixels.
    pub width: u32,
    /// Height in output pixels.
    pub height: u32,
}

impl Region {
    /// Pixel budget for every raster, whole or viewport region.
    pub const MAX_PIXELS: u64 = 8_000_000;
    /// Longest edge of a viewport region.
    pub const MAX_EDGE: u32 = 8192;

    /// Whether this region's whole pixels fit the budget with edges up to `max_edge`.
    pub fn within(&self, max_edge: u32) -> bool {
        self.width.max(self.height) <= max_edge
            && u64::from(self.width) * u64::from(self.height) <= Self::MAX_PIXELS
    }
    /// The whole diagram at `scale`, rounded up to whole pixels. The tolerance keeps
    /// `scale = pixels / width` from gaining a pixel through float error.
    pub fn whole(diagram: &Diagram, scale: f32) -> Self {
        let (width, height) = diagram.dimensions();
        let pixels = |extent: f32| (extent * scale - 1e-3).ceil().max(1.0) as u32;
        Self {
            scale,
            x: 0,
            y: 0,
            width: pixels(width),
            height: pixels(height),
        }
    }
}

/// Encodes an owned premultiplied pixmap in place, avoiding tiny-skia's full-size
/// demultiply copy; flat diagram colors compress well with the fast PNG mode.
fn encode_png(mut pixmap: tiny_skia::Pixmap) -> Result<Vec<u8>, png::EncodingError> {
    for [red, green, blue, alpha] in pixmap.data_mut().as_chunks_mut::<4>().0 {
        let alpha = u16::from(*alpha);
        if alpha != 0 && alpha != 255 {
            for channel in [red, green, blue] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let mut data = Vec::new();
    let mut encoder = png::Encoder::new(&mut data, pixmap.width(), pixmap.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    encoder.write_header()?.write_image_data(pixmap.data())?;
    Ok(data)
}

/// Observable cache counters used by tests and performance reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheStats {
    /// Reused native diagrams.
    pub hits: u64,
    /// Generated native diagrams.
    pub misses: u64,
    /// Currently resident diagrams.
    pub entries: usize,
    /// Approximate source, SVG and geometry byte accounting.
    pub bytes: usize,
}

struct Cache {
    diagrams: LruCache<[u8; 32], Diagram>,
    stats: CacheStats,
}

/// A cheap shared handle to a bounded diagram cache and lazily loaded system fonts.
#[derive(Clone)]
pub struct Renderer {
    cache: Arc<Mutex<Cache>>,
    fonts: Arc<OnceLock<Arc<usvg::fontdb::Database>>>,
}

impl fmt::Debug for Renderer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Renderer")
            .field("cache", &self.stats())
            .finish()
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    /// Construct an empty cache; font loading occurs on the first background render.
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(Cache {
                diagrams: LruCache::new(
                    NonZeroUsize::new(CACHE_ENTRIES).expect("diagram cache capacity is positive"),
                ),
                stats: CacheStats::default(),
            })),
            fonts: Arc::default(),
        }
    }
    /// Generate or reuse native Mermaid SVG; call from a bounded background executor.
    ///
    /// # Errors
    /// Returns syntax, SVG decoding and resource-budget failures.
    pub fn render(
        &self,
        source: &str,
        appearance: Appearance,
    ) -> Result<Diagram, RenderDiagramError> {
        self.render_with_fonts(source, appearance, &DiagramFonts::default())
    }
    /// Generate or reuse native SVG with independent Latin and CJK font families.
    ///
    /// # Errors
    /// Returns syntax, SVG decoding and resource-budget failures.
    pub fn render_with_fonts(
        &self,
        source: &str,
        appearance: Appearance,
        fonts: &DiagramFonts,
    ) -> Result<Diagram, RenderDiagramError> {
        if source.len() > MAX_SOURCE_BYTES {
            return Err(RenderDiagramError::Budget("source exceeds 128 KiB"));
        }
        let mut hash = blake3::Hasher::new();
        hash.update(source.as_bytes());
        hash.update(&[appearance as u8]);
        for family in [fonts.english(), fonts.chinese()] {
            hash.update(&(family.len() as u64).to_le_bytes());
            hash.update(family.as_bytes());
        }
        let key = *hash.finalize().as_bytes();
        {
            let mut cache = self.cache.lock().expect("diagram cache lock poisoned");
            if let Some(diagram) = cache.diagrams.get(&key).cloned() {
                cache.stats.hits += 1;
                return Ok(diagram);
            }
        }
        let start = Instant::now();
        let mut theme = match appearance {
            Appearance::Light => Theme::modern(),
            Appearance::Dark => Theme::dark(),
        };
        let (paper, ink, line, wash) = match appearance {
            Appearance::Light => ("#ececff", "#333333", "#333333", "#fff5ad"),
            Appearance::Dark => ("#2a2a2a", "#eeeeee", "#a8a8a8", "#383838"),
        };
        theme.background = "transparent".into();
        theme.primary_color = paper.into();
        theme.primary_text_color = ink.into();
        theme.primary_border_color = line.into();
        theme.line_color = line.into();
        theme.secondary_color = wash.into();
        theme.tertiary_color = paper.into();
        theme.text_color = ink.into();
        theme.edge_label_background = paper.into();
        theme.cluster_background = paper.into();
        theme.cluster_border = line.into();
        theme.sequence_actor_fill = paper.into();
        theme.sequence_actor_border = line.into();
        theme.sequence_actor_line = line.into();
        theme.sequence_note_fill = wash.into();
        theme.sequence_note_border = line.into();
        theme.sequence_activation_fill = wash.into();
        theme.sequence_activation_border = line.into();
        let gray = |index: usize| {
            let level = if appearance == Appearance::Dark {
                32 + index * 5
            } else {
                160 + index * 7
            };
            format!("#{level:02x}{level:02x}{level:02x}")
        };
        theme.pie_colors = std::array::from_fn(gray);
        theme.pie_title_text_color = ink.into();
        theme.pie_section_text_color = ink.into();
        theme.pie_legend_text_color = ink.into();
        theme.pie_stroke_color = line.into();
        theme.pie_outer_stroke_color = line.into();
        theme.git_colors = std::array::from_fn(gray);
        theme.git_inv_colors = std::array::from_fn(|_| ink.into());
        theme.git_branch_label_colors = std::array::from_fn(|_| ink.into());
        theme.git_commit_label_color = ink.into();
        theme.git_commit_label_background = paper.into();
        theme.git_tag_label_color = ink.into();
        theme.git_tag_label_background = paper.into();
        theme.git_tag_label_border = line.into();
        if appearance == Appearance::Light {
            // Separate quiet structural strokes from high-contrast message arrows.
            let border = "#c1ade5";
            let lifeline = "#b4a0d6";
            theme.primary_border_color = border.into();
            theme.tertiary_color = "#f5f2ff".into();
            theme.edge_label_background = "#ffffff".into();
            theme.cluster_background = "#f8f6ff".into();
            theme.cluster_border = border.into();
            theme.sequence_actor_border = border.into();
            theme.sequence_actor_line = lifeline.into();
            theme.sequence_note_border = "#b8ac53".into();
            theme.sequence_activation_fill = "#f3efff".into();
            theme.sequence_activation_border = lifeline.into();
            theme.git_tag_label_border = border.into();
        }
        // Latin metrics own the measurement key; CJK uses the existing full-width
        // cluster metrics. Changing only CJK must not duplicate Latin font data.
        theme.font_family = fonts.english_family().into();
        let parsed = mermaid_rs_renderer::parse_mermaid_strict(source)
            .map_err(|error| RenderDiagramError::Syntax(error.to_string()))?;
        if parsed.graph.nodes.len() > 512 || parsed.graph.edges.len() > 2048 {
            return Err(RenderDiagramError::Budget(
                "graph exceeds 512 nodes or 2048 edges",
            ));
        }
        let options = RenderOptions {
            theme,
            ..RenderOptions::default()
        };
        let layout =
            mermaid_rs_renderer::compute_layout(&parsed.graph, &options.theme, &options.layout);
        let svg = mermaid_rs_renderer::render_svg(&layout, &options.theme, &options.layout);
        if svg.len() > MAX_SVG_BYTES {
            return Err(RenderDiagramError::Budget("SVG exceeds 4 MiB"));
        }
        let svg = crate::fonts::apply_to_svg(&svg, fonts)?;
        if svg.len() > MAX_SVG_BYTES {
            return Err(RenderDiagramError::Budget("SVG exceeds 4 MiB"));
        }
        let database = self.fonts.get_or_init(crate::fonts::database);
        let options = usvg::Options {
            font_family: fonts.english_family().into(),
            fontdb: database.clone(),
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(&svg, &options)?;
        let diagram = Diagram {
            inner: Arc::new(DiagramData {
                svg,
                tree,
                render_time: start.elapsed(),
            }),
        };
        // SVG plus decoded path geometry is estimated conservatively at four times SVG size.
        let cost = diagram.svg().len().saturating_mul(4);
        let mut cache = self.cache.lock().expect("diagram cache lock poisoned");
        cache.stats.misses += 1;
        if cost <= CACHE_BYTES {
            while cache.stats.bytes + cost > CACHE_BYTES || cache.diagrams.len() == CACHE_ENTRIES {
                if let Some((_, removed)) = cache.diagrams.pop_lru() {
                    cache.stats.bytes = cache.stats.bytes.saturating_sub(removed.svg().len() * 4);
                } else {
                    break;
                }
            }
            if let Some(old) = cache.diagrams.put(key, diagram.clone()) {
                cache.stats.bytes = cache.stats.bytes.saturating_sub(old.svg().len() * 4);
            }
            cache.stats.bytes += cost;
        }
        cache.stats.entries = cache.diagrams.len();
        Ok(diagram)
    }
    /// Parse a standalone SVG with the shared system fonts and usvg's default image resolver.
    /// The SVG text is bounded; callers budgeting embedded resources use `svg_image_with_resolver`.
    /// The result is not cached.
    ///
    /// # Errors
    /// Rejects oversized or malformed SVG.
    pub fn svg_image(&self, svg: &str) -> Result<Diagram, RenderDiagramError> {
        self.svg_image_with_resolver(svg, usvg::ImageHrefResolver::default())
    }
    /// Parse a standalone SVG with a caller-provided resolver for embedded images.
    ///
    /// # Errors
    /// Rejects oversized or malformed SVG. The caller reports its resolver's resource errors.
    pub fn svg_image_with_resolver(
        &self,
        svg: &str,
        image_href_resolver: usvg::ImageHrefResolver<'_>,
    ) -> Result<Diagram, RenderDiagramError> {
        if svg.len() > MAX_SVG_BYTES {
            return Err(RenderDiagramError::Budget("SVG exceeds 4 MiB"));
        }
        let options = usvg::Options {
            fontdb: self.fonts.get_or_init(crate::fonts::database).clone(),
            image_href_resolver,
            ..usvg::Options::default()
        };
        let tree = usvg::Tree::from_str(svg, &options)?;
        Ok(Diagram {
            inner: Arc::new(DiagramData {
                svg: svg.to_owned(),
                tree,
                render_time: Duration::ZERO,
            }),
        })
    }
    /// Snapshot resident cache accounting and hit/miss counters.
    pub fn stats(&self) -> CacheStats {
        self.cache
            .lock()
            .expect("diagram cache lock poisoned")
            .stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_place_png_matches_tiny_skia_demultiplied_pixels() {
        let mut pixmap = tiny_skia::Pixmap::new(4, 1).unwrap();
        let pixels = [
            [0, 0, 0, 0],
            [40, 80, 120, 128],
            [3, 1, 2, 7],
            [10, 200, 30, 255],
        ];
        for (pixel, rgba) in pixmap
            .data_mut()
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(pixels)
        {
            *pixel = rgba;
        }
        let decode = |png: Vec<u8>| {
            let mut reader = png::Decoder::new(std::io::Cursor::new(png))
                .read_info()
                .unwrap();
            let mut data = vec![0; reader.output_buffer_size().unwrap()];
            reader.next_frame(&mut data).unwrap();
            data
        };
        let expected = decode(pixmap.encode_png().unwrap());
        let actual = decode(encode_png(pixmap).unwrap());
        for (actual, expected) in actual.iter().zip(&expected) {
            assert!(
                actual.abs_diff(*expected) <= 1,
                "{actual:?} vs {expected:?}"
            );
        }
    }

    #[test]
    fn largest_whole_raster_fits_the_budget_after_pixel_rounding() {
        let renderer = Renderer::new();
        for source in [
            // Its continuous maximum rounds up to 2192×3650 = 8,000,800 pixels.
            "flowchart TB\nA-->B\nA-->C\nB-->D\nC-->E\nD-->F\nE-->F",
            "flowchart LR\n A --> B --> C --> D --> E --> F --> G --> H",
            "flowchart TB\n A --> B --> C --> D --> E --> F --> G --> H",
            "graph LR; A-->B",
        ] {
            let diagram = renderer.render(source, Appearance::Light).unwrap();
            let region = Region::whole(&diagram, diagram.max_scale());
            assert!(
                region.width.max(region.height) <= MAX_RASTER_EDGE
                    && u64::from(region.width) * u64::from(region.height) <= Region::MAX_PIXELS,
                "{source:?}: {region:?} exceeds the whole-raster budget"
            );
        }
        let diagram = renderer
            .render(
                "flowchart TB\nA-->B\nA-->C\nB-->D\nC-->E\nD-->F\nE-->F",
                Appearance::Light,
            )
            .unwrap();
        let region = Region::whole(&diagram, diagram.max_scale());
        assert!(diagram.png_region(region, Some([255; 3])).is_ok());
    }

    #[test]
    fn regions_tile_the_whole_raster_on_an_opaque_background() {
        let diagram = Renderer::new()
            .render("flowchart LR\n A[开始] --> B[完成]", Appearance::Light)
            .unwrap();
        let whole = Region::whole(&diagram, 1.75);
        let background = [0xfa, 0xf8, 0xf2];
        let full = diagram.bgra_region(whole, background).unwrap();
        assert_eq!(full.len(), (whole.width * whole.height * 4) as usize);
        let pixels = full.as_chunks::<4>().0;
        assert!(
            pixels.iter().all(|pixel| pixel[3] == 255),
            "no transparent pixel remains for the GPU to blend with black"
        );
        assert!(
            pixels.contains(&[background[2], background[1], background[0], 255]),
            "the background is stored in BGRA order"
        );
        let (x, y) = (whole.width / 3, whole.height / 4);
        let tile = Region {
            x,
            y,
            width: whole.width / 2,
            height: whole.height / 2,
            ..whole
        };
        let part = diagram.bgra_region(tile, background).unwrap();
        let stride = (tile.width * 4) as usize;
        for row in 0..tile.height {
            let start = (((y + row) * whole.width + x) * 4) as usize;
            let expected = &full[start..start + stride];
            let actual = &part[row as usize * stride..(row as usize + 1) * stride];
            assert!(
                actual
                    .iter()
                    .zip(expected)
                    .all(|(a, e)| a.abs_diff(*e) <= 2),
                "tile row {row} matches the whole raster"
            );
        }
        let oversized = Region {
            width: 9000,
            ..whole
        };
        assert!(matches!(
            diagram.bgra_region(oversized, background),
            Err(RenderDiagramError::Budget(_))
        ));
    }
}
