//! Local code formatting with bounded work, immutable output and source mappings.
//!
//! No UI, runtime, subprocess or network is required. Unsupported fence labels resolve to
//! `None`; a formatting error leaves callers free to display their original source.
//!
//! ```
//! use pecan_format::Format;
//! let source = r#"{"width":960}"#;
//! let formatted = Format::Json.format(source, 4)?;
//! assert!(formatted.text().contains("\n    \"width\""));
//! # Ok::<(), pecan_format::FormatError>(())
//! ```
mod export;
mod prepared;
pub use export::Formats;
pub use prepared::{CodeSnapshot, SLOTS};
mod config;
mod json;
mod language;
mod mapping;
mod web;
pub use language::{Format, tab_width};
pub use mapping::FormattedCode;

/// A formatting, validation or resource-limit failure; callers retain their original input.
#[derive(Debug)]
pub struct FormatError {
    source: anyhow::Error,
}
impl FormatError {
    fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}
impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "code formatting failed: {:#}", self.source)
    }
}
impl std::error::Error for FormatError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.as_ref())
    }
}
