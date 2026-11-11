//! Document-scoped formatting runs before code rows are materialized and is reused on scroll.
use crate::{Format, FormattedCode, tab_width};
use pecan_document::{Block, Document};
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Share the bounded queue with per-block Tab changes instead of starting unbounded CPU jobs.
pub static SLOTS: async_lock::Semaphore = async_lock::Semaphore::new(2);
const MAX_RETAINED: usize = 8 * 1024 * 1024;
const MAX_BLOCKS: usize = 4096;
const MAX_WORK: Duration = Duration::from_secs(2);

type Key = (Format, [u8; 32]);
pub(crate) type Prepared = HashMap<Key, Option<Arc<FormattedCode>>>;

fn key(language: &str, source: &str) -> Option<Key> {
    Some((
        Format::for_language(language)?,
        *blake3::hash(source.as_bytes()).as_bytes(),
    ))
}

/// An immutable, cheap export snapshot; closing a tab cannot invalidate its results.
#[derive(Clone, Debug, Default)]
pub struct CodeSnapshot {
    preference: String,
    prepared: Option<Arc<Prepared>>,
}
impl CodeSnapshot {
    /// Prepare an immutable document batch on the caller's background executor.
    /// Cancellation and budgets retain the original source as a safe fallback.
    pub fn prepare(document: &Document, preference: String, cancel: &AtomicBool) -> Self {
        Self {
            prepared: Some(Arc::new(prepare(document, &preference, cancel))),
            preference,
        }
    }
    /// Share one prepared result without copying formatted text or source mappings.
    pub fn get(&self, language: &str, source: &str) -> Option<Arc<FormattedCode>> {
        self.prepared
            .as_ref()?
            .get(&key(language, source)?)?
            .clone()
    }
}
impl CodeSnapshot {
    /// Outer None means not prepared; inner None is a deliberate safe fallback.
    pub fn lookup(
        &self,
        language: &str,
        source: &str,
        preference: &str,
    ) -> Option<Option<Arc<FormattedCode>>> {
        if preference != self.preference {
            return None;
        }
        self.prepared
            .as_ref()?
            .get(&key(language, source)?)
            .cloned()
    }
}

fn prepare(document: &Document, preference: &str, cancel: &AtomicBool) -> Prepared {
    let source = document.source();
    let started = Instant::now();
    let mut prepared = HashMap::new();
    let mut retained = 0usize;
    let mut insert = |prepared: &mut Prepared, format: Format, source: &str| {
        if source.len() > format.input_limit() || prepared.len() >= MAX_BLOCKS {
            return;
        }
        let key = (format, *blake3::hash(source.as_bytes()).as_bytes());
        if prepared.contains_key(&key) {
            return;
        }
        let output = if retained < MAX_RETAINED && started.elapsed() < MAX_WORK {
            format
                .format(source, tab_width(source, preference))
                .ok()
                .filter(|code| code.estimated_size() <= MAX_RETAINED - retained)
        } else {
            None
        };
        let output = output.map(|code| {
            retained += code.estimated_size();
            Arc::new(code)
        });
        prepared.insert(key, output);
    };
    // Top-level cards borrow these exact bytes. CommonMark normalizes CRLF and
    // removes fence indentation, so parser text alone cannot key their source maps.
    for block in document.blocks() {
        if cancel.load(Ordering::Relaxed)
            || prepared.len() >= MAX_BLOCKS
            || started.elapsed() >= MAX_WORK
        {
            return prepared;
        }
        if let Block::CodePage {
            code,
            content,
            language,
            ..
        } = block
            && content.start == code.start
            && let Some(format) = Format::for_language(language)
        {
            insert(&mut prepared, format, fence_body(&source[code.clone()]));
        }
    }
    let mut body = String::new();
    let mut format = None;
    let mut oversized = false;
    for event in Parser::new_ext(
        source,
        Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_MATH,
    ) {
        if cancel.load(Ordering::Relaxed)
            || prepared.len() >= MAX_BLOCKS
            || started.elapsed() >= MAX_WORK
        {
            break;
        }
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) => {
                format = Format::for_language(&info);
                body.clear();
                oversized = false;
            }
            Event::Text(text) if format.is_some() && !oversized => {
                if body.len() + text.len()
                    <= format.expect("guarded active code format").input_limit()
                {
                    body.push_str(&text);
                } else {
                    oversized = true;
                    body.clear();
                }
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(format) = format.take().filter(|_| !oversized) {
                    insert(&mut prepared, format, fence_body(&body));
                }
            }
            _ => {}
        }
    }
    prepared
}

fn fence_body(source: &str) -> &str {
    source
        .strip_suffix("\r\n")
        .or_else(|| source.strip_suffix('\n'))
        .unwrap_or(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn export_snapshots_reuse_results_and_reject_stale_preferences() {
        let raw = "{\"a\":{\"b\":1}}";
        let document = Document::parse(format!("```json\n{raw}\n```\n")).unwrap();
        let results = Arc::new(prepare(&document, "4", &AtomicBool::new(false)));
        let snapshot = CodeSnapshot {
            preference: "4".into(),
            prepared: Some(results.clone()),
        };
        let exported = snapshot.lookup("json", raw, "4").unwrap().unwrap();
        assert!(Arc::ptr_eq(
            &exported,
            results
                .get(&key("json", raw).unwrap())
                .unwrap()
                .as_ref()
                .unwrap()
        ));
        assert!(snapshot.lookup("json", raw, "2").is_none());
        let frozen = snapshot.clone();
        drop(snapshot);
        drop(results);
        assert!(
            frozen
                .lookup("json", raw, "4")
                .unwrap()
                .unwrap()
                .text()
                .contains("\n    \"a\"")
        );
    }

    #[test]
    fn preformats_hidden_nested_and_crlf_blocks_without_changing_markdown() {
        let source = "```yaml\nreader:\n  port: 8080\n```\n\n  ```json\n  {\"indent\":1}\n  ```\n\n```json\n{\"a\":{\"b\":1}}\n```\n\n> ```yaml\n> reader:\n>   port: 8080\n> ```\n\n:::info\n```TOML\nports=[\n  8080,\n]\n```\n:::\n";
        for source in [source.to_owned(), source.replace('\n', "\r\n")] {
            let original = source.clone();
            let document = Document::parse(source.clone()).unwrap();
            let prepared = prepare(&document, "4", &AtomicBool::new(false));
            let json = prepared
                .get(&key("json", "{\"a\":{\"b\":1}}").unwrap())
                .unwrap()
                .as_ref()
                .unwrap();
            assert!(json.text().contains("\n    \"a\""));
            let yaml = prepared
                .get(&key("yaml", "reader:\n  port: 8080").unwrap())
                .unwrap()
                .as_ref()
                .unwrap();
            assert!(yaml.text().contains("\n    port:"));
            let toml = prepared
                .get(&key("toml", "ports=[\n  8080,\n]").unwrap())
                .unwrap()
                .as_ref()
                .unwrap();
            assert!(toml.text().contains("\n    8080,"));
            for block in document.blocks() {
                if let pecan_document::Block::CodePage {
                    code,
                    content,
                    language,
                    ..
                } = block
                    && content.start == code.start
                {
                    let raw = &document.source()[code.clone()];
                    let raw = raw
                        .strip_suffix("\r\n")
                        .or_else(|| raw.strip_suffix('\n'))
                        .unwrap_or(raw);
                    assert!(
                        prepared
                            .get(&key(language, raw).unwrap())
                            .is_some_and(Option::is_some),
                        "raw fence must hit cache: {raw:?}"
                    );
                }
            }
            assert_eq!(source, original);
        }
    }
    #[test]
    fn repeated_fences_share_output_and_cancellation_stops_document_work() {
        let source = "```json\n{\"a\":1}\n```\n".repeat(200);
        let document = Document::parse(source).unwrap();
        let prepared = prepare(&document, "2", &AtomicBool::new(false));
        assert_eq!(prepared.len(), 1);
        assert!(prepared.values().all(Option::is_some));
        assert!(prepare(&document, "2", &AtomicBool::new(true)).is_empty());
    }
}
