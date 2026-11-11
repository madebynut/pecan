//! GFM cells contain inline events only. Recover block Markdown from real HTML
//! line breaks/list tags, then let the regular serializer handle all formatting.
use super::document::options;
use pulldown_cmark::{BrokenLink, Event, Options, Parser, RefDefs, Tag, TagEnd};
use std::{ops::Range, sync::LazyLock};

static LIST_PREFIX: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"^(?:[-+*]\s|[0-9]{1,9}[.)]\s)").expect("valid list prefix")
});
static BLOCK_TAG: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^<(/?)(br|ul|ol|li)(?:\s+[^>]*)?\s*/?>$").expect("valid cell block tag")
});
static LIST_START: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)\bstart\s*=\s*["']?([0-9]+)"#).expect("valid list start")
});

/// Ordinary cells retain their borrowed events. Only rich cells are buffered,
/// and only one cell at a time; images and math still use the main export path.
pub(super) fn events(source: &str) -> impl Iterator<Item = Event<'_>> {
    let mut parser = Parser::new_ext(source, options()).into_offset_iter();
    let mut pending = Vec::new().into_iter();
    std::iter::from_fn(move || {
        if let Some(event) = pending.next() {
            return Some(event);
        }
        let (event, range) = parser.next()?;
        if matches!(event, Event::Start(Tag::TableCell)) {
            let cell = &source[range.clone()];
            if cell.contains('<') || LIST_PREFIX.is_match(cell.trim_start()) {
                let mut original = Vec::new();
                for (event, offset) in parser.by_ref() {
                    if matches!(event, Event::End(TagEnd::TableCell)) {
                        break;
                    }
                    original.push((event, offset));
                }
                pending = cell_events(source, range, original, parser.reference_definitions())
                    .into_iter();
            }
        }
        Some(event)
    })
}

fn cell_events<'src>(
    source: &'src str,
    range: Range<usize>,
    original: Vec<(Event<'src>, Range<usize>)>,
    references: &RefDefs<'_>,
) -> Vec<Event<'src>> {
    let mut edits = Vec::new();
    let mut lists: Vec<Option<u64>> = Vec::new();
    for (event, offset) in &original {
        let Event::InlineHtml(tag) = event else {
            continue;
        };
        let Some(parts) = BLOCK_TAG.captures(tag.trim()) else {
            continue;
        };
        let closing = !parts[1].is_empty();
        let replacement = match parts[2].to_ascii_lowercase().as_str() {
            "br" => "  \n".to_owned(),
            "ul" | "ol" if closing => {
                lists.pop();
                "\n".to_owned()
            }
            "ul" | "ol" => {
                let number = (parts[2].eq_ignore_ascii_case("ol")).then(|| {
                    LIST_START
                        .captures(tag)
                        .and_then(|c| c[1].parse().ok())
                        .unwrap_or(1)
                });
                lists.push(number);
                "\n".to_owned()
            }
            "li" if closing => "\n".to_owned(),
            "li" => {
                // Match the reader's supported note depth and bound malformed HTML.
                let indent = "    ".repeat(lists.len().saturating_sub(1).min(32));
                if let Some(Some(number)) = lists.last_mut() {
                    let marker = format!("\n{indent}{number}. ");
                    *number = number.saturating_add(1);
                    marker
                } else {
                    format!("\n{indent}- ")
                }
            }
            _ => continue,
        };
        edits.push((
            offset.start - range.start..offset.end - range.start,
            replacement,
        ));
    }
    let cell = &source[range];
    let mut events = if edits.is_empty() && !LIST_PREFIX.is_match(cell.trim_start()) {
        original.into_iter().map(|(event, _)| event).collect()
    } else {
        let mut markdown = String::with_capacity(cell.len());
        let mut cursor = 0;
        for (offset, replacement) in edits {
            markdown.push_str(&cell[cursor..offset.start]);
            markdown.push_str(&replacement);
            cursor = offset.end;
        }
        markdown.push_str(&cell[cursor..]);
        // Reuse document references without copying its definition map. A cell
        // is already inside a table, so its pipes must not create another table.
        let resolve = |link: BrokenLink<'_>| {
            references.get(&link.reference).map(|definition| {
                (
                    definition.dest.clone().into_static(),
                    definition
                        .title
                        .clone()
                        .unwrap_or_else(|| "".into())
                        .into_static(),
                )
            })
        };
        Parser::new_with_broken_link_callback(
            markdown.trim(),
            options() - Options::ENABLE_TABLES,
            Some(resolve),
        )
        .map(Event::into_static)
        .collect::<Vec<_>>()
    };
    events.push(Event::End(TagEnd::TableCell));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rich_cells_use_block_markdown_without_reinterpreting_code_or_escaped_html() {
        let source = "| 内容 |\n| --- |\n| 前言<br><br>- **bold**<br>- *italic*与~~removed~~、`<br>`<br><br>结尾 |\n| <ol start=\"3\"><li>第一项</li><li>第二项</li></ol> |\n| `* literal<br>`和\\<br> |\n| a\\|b |\n";
        let events: Vec<_> = events(source).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Start(Tag::Item)))
                .count(),
            4
        );
        assert!(events.contains(&Event::Start(Tag::List(Some(3)))));
        assert!(events.contains(&Event::Code("<br>".into())));
        assert!(events.contains(&Event::Code("* literal<br>".into())));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Text(text) if text.contains("结尾")))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Strong)))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Emphasis)))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Strikethrough)))
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Start(Tag::TableCell)))
                .count(),
            5
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::End(TagEnd::TableCell)))
                .count(),
            5
        );
    }

    #[test]
    fn plain_cells_keep_the_original_borrowed_events() {
        let source = "| **标题** | `code` |\n| --- | --- |\n| *斜体* | ~~删除~~ [链接](https://example.com) |\n";
        assert_eq!(
            events(source).collect::<Vec<_>>(),
            Parser::new_ext(source, options()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rich_cells_preserve_document_reference_links_and_images() {
        let source = "| 内容 |\n| --- |\n| - [**文档**][guide]<br>- ![图][image] |\n\n[guide]: https://example.com/docs \"说明\"\n[image]: image.png \"插图\"\n";
        let events: Vec<_> = events(source).collect();
        assert!(events.iter().any(|event| matches!(event,
            Event::Start(Tag::Link { dest_url, title, .. })
                if dest_url.as_ref() == "https://example.com/docs" && title.as_ref() == "说明"
        )));
        assert!(events.iter().any(|event| matches!(event,
            Event::Start(Tag::Image { dest_url, title, .. })
                if dest_url.as_ref() == "image.png" && title.as_ref() == "插图"
        )));
    }

    #[test]
    fn nested_html_lists_single_items_and_tasks_use_the_regular_parser() {
        let source = "| 内容 |\n| --- |\n| <ul><li>父项<ul><li>子项</li></ul></li><li>同级项</li></ul> |\n| - 单个项目 |\n| - [x] **完成**<br>- [ ] 待办 |\n| 第一行<BR />第二行 |\n";
        let events: Vec<_> = events(source).collect();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::Start(Tag::Item)))
                .count(),
            6
        );
        assert!(events.contains(&Event::TaskListMarker(true)));
        assert!(events.contains(&Event::TaskListMarker(false)));
        assert!(events.contains(&Event::HardBreak));
        let mut depth = 0;
        let mut maximum = 0;
        for event in events {
            match event {
                Event::Start(Tag::List(_)) => {
                    depth += 1;
                    maximum = maximum.max(depth);
                }
                Event::End(TagEnd::List(_)) => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(maximum, 2);
        assert_eq!(depth, 0);
    }
}
