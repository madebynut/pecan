//! Bounded preparation of code that has not yet entered the reading cache.
use crate::{CodeSnapshot, Format, FormattedCode, SLOTS, tab_width};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// Reuse reading results; prepare missing export blocks once, within the same safety budgets.
#[derive(Debug)]
pub struct Formats {
    reading: CodeSnapshot,
    preference: String,
    cache: HashMap<(Format, [u8; 32]), Option<Arc<FormattedCode>>>,
    retained: usize,
    work: Duration,
}
impl Formats {
    /// Reuse an immutable reading snapshot under a captured Tab preference.
    pub fn new(reading: CodeSnapshot, preference: String) -> Self {
        Self {
            reading,
            preference,
            cache: HashMap::new(),
            retained: 0,
            work: Duration::ZERO,
        }
    }
    /// Format missing code once under the shared task and retained-memory budgets.
    /// Invalid, unsupported or over-budget code returns None for source-preserving fallback.
    pub async fn resolve(&mut self, language: &str, source: &str) -> Option<Arc<FormattedCode>> {
        let format = Format::for_language(language)?;
        if let Some(result) = self.reading.lookup(language, source, &self.preference) {
            return result;
        }
        if source.len() > format.input_limit() {
            return None;
        }
        let key = (format, *blake3::hash(source.as_bytes()).as_bytes());
        if let Some(result) = self.cache.get(&key) {
            return result.clone();
        }
        const MAX_RETAINED: usize = 8 * 1024 * 1024;
        if self.retained >= MAX_RETAINED
            || self.cache.len() >= 4096
            || self.work >= Duration::from_secs(2)
        {
            return None;
        }
        let _permit = SLOTS.acquire().await;
        let start = Instant::now();
        let output = format
            .format(source, tab_width(source, &self.preference))
            .ok()
            .filter(|code| code.estimated_size() <= MAX_RETAINED - self.retained)
            .map(Arc::new);
        self.work += start.elapsed();
        if let Some(output) = &output {
            self.retained += output.estimated_size();
        }
        self.cache.insert(key, output.clone());
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_export_code_reuses_results_and_preserves_safe_fallbacks() {
        futures_lite::future::block_on(async {
            let mut formats = Formats::new(CodeSnapshot::default(), "4".into());
            let raw = "{\"outer\":{\"value\":1}}";
            let formatted = formats.resolve("JSON", raw).await.unwrap();
            assert!(formatted.text().contains("\n    \"outer\""));
            assert!(Arc::ptr_eq(
                &formatted,
                &formats.resolve("jsonc", raw).await.unwrap()
            ));
            assert!(formats.resolve("json", "{invalid}").await.is_none());
            assert!(formats.resolve("unknown", raw).await.is_none());
            let oversized = "x".repeat(Format::JavaScript.input_limit() + 1);
            assert!(formats.resolve("js", &oversized).await.is_none());
            assert!(
                formats
                    .resolve("mermaid", "graph LR; A-->B")
                    .await
                    .is_none()
            );
        });
    }
}
