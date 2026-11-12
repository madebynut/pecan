//! Immutable Markdown indexing keeps document work outside the desktop rendering loop.

mod document;
pub mod files;
mod search;

pub use document::{
    AlertKind, Block, Document, Heading, MAX_DOCUMENT_BYTES, ParseDocumentError, alert_marker,
    expand_alerts, expand_callouts,
};
pub use search::{SearchMatch, SearchQuery};

mod html_image;
pub use html_image::{HtmlImage, html_image};
