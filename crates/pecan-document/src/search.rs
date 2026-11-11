use std::ops::Range;

use regex::{Regex, RegexBuilder};

use crate::{Document, document::Block};

/// Maximum highlighted results, bounding search allocations and UI work.
const MAX_MATCHES: usize = 4096;

/// A compiled, Unicode-aware literal search query.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    pattern: Regex,
}

/// A literal occurrence in the original source and its virtual row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchMatch {
    range: Range<usize>,
    block: usize,
}

impl SearchMatch {
    /// Source byte range, always at valid UTF-8 boundaries.
    pub fn source_range(&self) -> Range<usize> {
        self.range.clone()
    }
    /// Virtual row containing the beginning of this match.
    pub fn block_index(&self) -> usize {
        self.block
    }
}

impl SearchQuery {
    /// Compile a case-insensitive literal query; empty text clears search.
    pub fn new(query: &str) -> Option<Self> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        RegexBuilder::new(&regex::escape(query))
            .case_insensitive(true)
            .build()
            .ok()
            .map(|pattern| Self { pattern })
    }
    /// Search up to 4096 occurrences without copying or lowercasing the document.
    pub fn find(&self, document: &Document) -> Vec<SearchMatch> {
        self.pattern
            .find_iter(document.source())
            .take(MAX_MATCHES)
            .filter_map(|found| {
                let block = document.block_at(found.start())?;
                let range = found.range();
                // Fence lines are not drawn, so a query for `:::` must not land on them.
                if let Block::Callout { body, .. } = &document.blocks()[block]
                    && (range.start < body.start || range.end > body.end)
                {
                    return None;
                }
                Some(SearchMatch { range, block })
            })
            .collect()
    }
}
