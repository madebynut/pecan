//! Compact source mappings shared by all formatting backends.
use anyhow::{Result, ensure};
use similar::{Algorithm, ChangeTag, TextDiff};
use std::{ops::Range, time::Duration};
#[derive(Debug)]
pub(crate) struct Token {
    pub(crate) source: Range<usize>,
    pub(crate) display: usize,
}

/// Immutable formatted text and UTF-8 source ranges for search and selection.
#[derive(Debug)]
pub struct FormattedCode {
    pub(crate) text: String,
    pub(crate) tokens: Vec<Token>,
}
impl FormattedCode {
    /// Borrow the full display text; the original input is never modified.
    pub fn text(&self) -> &str {
        &self.text
    }
    /// Estimate retained heap bytes for a caller-owned cache budget.
    pub fn estimated_size(&self) -> usize {
        self.text.capacity() + self.tokens.capacity() * std::mem::size_of::<Token>()
    }

    /// Map surviving bytes, snapping removed whitespace toward the selected text.
    pub fn range(&self, range: Range<usize>) -> Range<usize> {
        let first = self.tokens.partition_point(|t| t.source.end <= range.start);
        let last = self.tokens.partition_point(|t| t.source.start < range.end);
        if first >= last {
            let at = self.tokens.get(first).map_or_else(
                || self.tokens.last().map_or(0, |t| t.display + t.source.len()),
                |t| t.display,
            );
            return at..at;
        }
        let start = &self.tokens[first];
        let end = &self.tokens[last - 1];
        start.display + range.start.saturating_sub(start.source.start)
            ..end.display + range.end.min(end.source.end) - end.source.start
    }
}

/// Keep UTF-8 byte mappings for surviving code, including literals whose quotes change.
pub(crate) fn map_changes(source: &str, text: String) -> Result<FormattedCode> {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .timeout(Duration::from_millis(100))
        .diff_chars(source, &text);
    let mut tokens: Vec<Token> = Vec::new();
    let (mut old, mut new) = (0, 0);
    for change in diff.iter_all_changes() {
        let len = change.value().len();
        match change.tag() {
            ChangeTag::Equal => {
                if let Some(last) = tokens.last_mut().filter(|last| {
                    last.source.end == old && last.display + last.source.len() == new
                }) {
                    last.source.end += len;
                } else {
                    tokens.push(Token {
                        source: old..old + len,
                        display: new,
                    });
                }
                old += len;
                new += len;
            }
            ChangeTag::Delete => old += len,
            ChangeTag::Insert => new += len,
        }
    }
    // A timed-out diff can omit a middle section. Reject that result instead of publishing
    // a source mapping which would send search or selection to unrelated text.
    let insignificant = |gap: &str| {
        gap.chars()
            .all(|c| c.is_whitespace() || matches!(c, '\'' | '"' | '\\' | ',' | ';'))
    };
    let mut covered = 0;
    for token in &tokens {
        ensure!(
            insignificant(&source[covered..token.source.start]),
            "code comparison cannot preserve source ranges"
        );
        covered = token.source.end;
    }
    ensure!(
        insignificant(&source[covered..]),
        "code comparison cannot preserve source ranges"
    );
    Ok(FormattedCode { text, tokens })
}
