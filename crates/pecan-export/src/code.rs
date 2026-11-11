//! Shared formatting and highlighter adapters; no duplicate lexer or caches.
pub(super) use pecan_code::Ink;
pub(super) use pecan_format::Formats;
pub(super) fn spans(code: &str, language: &str) -> Vec<(std::ops::Range<usize>, Ink)> {
    pecan_code::spans(code, language, pecan_code::Appearance::Light)
}
