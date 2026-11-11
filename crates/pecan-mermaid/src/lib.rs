//! Native Mermaid layout and bounded caches avoid browser processes and repeated parsing.

mod fonts;
mod renderer;

pub use fonts::{DiagramFonts, SYSTEM_MONO_FONT, SYSTEM_UI_FONT, database as system_fonts};
pub use renderer::{Appearance, CacheStats, Diagram, Region, RenderDiagramError, Renderer};

mod queue;
pub use queue::RenderQueue;
