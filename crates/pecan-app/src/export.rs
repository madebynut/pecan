//! Native save-dialog and notification adapter for pecan-export.
mod save;
pub(crate) use pecan_export::{DiagramFormat, DiagramInput, DocumentFormat, Snapshot};
pub(crate) use save::{save_diagram, save_document};
