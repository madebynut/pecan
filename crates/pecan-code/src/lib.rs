//! Shared code languages, cached themes and bounded Tree-sitter syntax colors.
mod highlight;
mod language;
mod theme;
pub use highlight::{Ink, spans};
pub use language::language;
pub use theme::native_theme;

/// The syntax palette used by reading or a portable document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Neutral light paper, also used for exports.
    #[default]
    Light,
    /// High-contrast tokens on a dark reading surface.
    Dark,
}
