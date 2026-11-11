use std::fmt::Write as _;

use pecan_document::{
    AlertKind, Block, Document, SearchQuery, alert_marker, expand_alerts, expand_callouts,
};

#[test]
fn outline_headings_start_sections_without_splitting_lists() {
    let source = "# Title\n\nintro\n\n## Second\n\n- one\n  - nested\n\n### Third\n\nbody\n";
    let document = Document::parse(source.into()).unwrap();
    for heading in document.headings() {
        let range = document.blocks()[heading.block_index()].source_range();
        assert_eq!(
            range.start,
            heading.source_range().start,
            "outline navigation must target the section top: {}",
            heading.title()
        );
    }
    assert_eq!(
        document.source(),
        source,
        "section indexing preserves source"
    );
    assert!(
        document
            .blocks()
            .iter()
            .any(|block| document.source()[block.source_range()].contains("- one\n  - nested")),
        "a nested list remains one semantic block"
    );
}

#[test]
fn diagrams_keep_exact_source_and_headings_reveal_the_correct_row() {
    let source = "# 开始\n\n正文\n\n```mermaid\nflowchart LR\n A[中文] --> B\n```\n\n## **结果**\n";
    let document = Document::parse(source.into()).unwrap();
    assert_eq!(document.headings().len(), 2);
    assert_eq!(document.headings()[1].title(), "结果");
    assert_eq!(document.headings()[1].block_index(), 2);
    let Block::Mermaid { code, .. } = &document.blocks()[1] else {
        panic!("fence must become a diagram row")
    };
    assert_eq!(
        &**code, "flowchart LR\n A[中文] --> B\n",
        "copy preserves whitespace and trailing newline"
    );
}

#[test]
fn fences_in_code_do_not_create_false_headings() {
    let document = Document::parse("```rust\n# fake\n```\n\n# Real\n".into()).unwrap();
    assert_eq!(document.headings().len(), 1);
    assert_eq!(document.headings()[0].title(), "Real");
}

#[test]
fn long_documents_keep_semantic_blocks_and_small_indices() {
    let mut source = String::new();
    for index in 0..12_000 {
        write!(
            source,
            "## 第 {index} 节\n\n包含中文以及 **emphasis** 的阅读段落。\n\n"
        )
        .unwrap();
    }
    let document = Document::parse(source).unwrap();
    assert_eq!(document.headings().len(), 12_000);
    assert!(
        document.blocks().len() <= document.headings().len(),
        "paragraphs remain grouped under each heading, without a row per paragraph"
    );
    for heading in document.headings() {
        assert!(
            document.blocks()[heading.block_index()]
                .source_range()
                .contains(&heading.source_range().start)
        );
    }
}

#[test]
fn unicode_literal_search_is_case_insensitive_and_bounded() {
    let document =
        Document::parse("# 中文 Rust\n\nRust rust RUST\n\n中文 **文字** [link](x)\n".repeat(2000))
            .unwrap();
    let results = SearchQuery::new("rUsT").unwrap().find(&document);
    assert_eq!(results.len(), 4096, "search allocations are bounded");
    let chinese = SearchQuery::new("中文").unwrap().find(&document);
    assert_eq!(&document.source()[chinese[0].source_range()], "中文");
    assert!(SearchQuery::new("  ").is_none());
}

#[test]
fn reference_definitions_survive_chunking() {
    let document =
        Document::parse("# 文档\n\n[引用][ref]\n\n[ref]: https://example.com \"标题\"\n".into())
            .unwrap();
    assert!(document.reference_suffix().contains("https://example.com"));
}

#[test]
fn oversized_code_flows_losslessly_and_copy_excludes_fences() {
    let code = "let value = \"中文\";\n".repeat(10_000);
    let source = format!("# Large\n\n```rust\n{code}```\n\n## After\n");
    let document = Document::parse(source).unwrap();
    let mut displayed = String::new();
    let mut previous_end = None;
    let mut count = 0;
    for block in document.blocks() {
        if let Block::CodePage {
            source,
            content,
            code: copy,
            language,
        } = block
        {
            assert_eq!(&document.source()[copy.clone()], code);
            assert_eq!(language, "rust");
            assert!(content.len() <= 16 * 1024);
            assert!(document.source()[content.clone()].matches('\n').count() <= 128);
            if let Some(end) = previous_end {
                assert_eq!(source.start, end);
            }
            previous_end = Some(source.end);
            displayed.push_str(&document.source()[content.clone()]);
            count += 1;
        }
    }
    assert!(
        count > 1,
        "large code is divided only for outer-list rendering"
    );
    assert_eq!(displayed, code);
    let heading = &document.headings()[1];
    assert_eq!(heading.title(), "After");
    assert!(
        document.blocks()[heading.block_index()]
            .source_range()
            .contains(&heading.source_range().start)
    );
    let last_code = document.source().rfind("let value").unwrap();
    assert!(matches!(
        document.blocks()[document.block_at(last_code).unwrap()],
        Block::CodePage { .. }
    ));
}

#[test]
fn code_fences_keep_empty_unclosed_crlf_and_literal_fences() {
    for (source, expected) in [
        ("```\n```", ""),
        ("```rust\nlet a = 1;", "let a = 1;"),
        ("~~~rust\r\nlet a = 1;\r\n~~~\r\n", "let a = 1;\r\n"),
        ("````text\n```\n~~~\n````\n", "```\n~~~\n"),
    ] {
        let document = Document::parse(source.into()).unwrap();
        let Block::CodePage { content, code, .. } = &document.blocks()[0] else {
            panic!("code must be a flowing code row")
        };
        assert_eq!(&document.source()[code.clone()], expected);
        assert_eq!(&document.source()[content.clone()], expected);
    }
    let code = "中文".repeat(20_000);
    let document = Document::parse(format!("```text\n{code}\n```\n")).unwrap();
    let displayed = document
        .blocks()
        .iter()
        .filter_map(|block| {
            if let Block::CodePage { content, .. } = block {
                Some(&document.source()[content.clone()])
            } else {
                None
            }
        })
        .collect::<String>();
    assert_eq!(displayed, format!("{code}\n"));
}

#[test]
fn oversized_single_line_pages_never_split_utf8() {
    let document = Document::parse("中文阅读器".repeat(20_000)).unwrap();
    let Block::Source { pages, .. } = &document.blocks()[0] else {
        panic!("giant paragraphs must avoid unbounded rich layout")
    };
    for page in pages {
        assert!(document.source().is_char_boundary(page.start));
        assert!(document.source().is_char_boundary(page.end));
    }
}

#[test]
fn large_reference_tables_and_raw_html_cannot_bypass_rich_layout_budget() {
    let mut references = String::from("# References\n\n[link][ref0]\n\n");
    for index in 0..10_000 {
        writeln!(references, "[ref{index}]: https://example.com/{index}").unwrap();
    }
    let html = format!("<div>\n{}\n</div>\n", "中文内容\n".repeat(20_000));
    for source in [references, html] {
        let document = Document::parse(source.clone()).unwrap();
        assert!(
            document.blocks().iter().all(|block| match block {
                Block::Markdown { source } => {
                    source.len() + document.reference_suffix().len() <= 64 * 1024
                }
                Block::Source { pages, .. } => pages.iter().all(|page| page.len() <= 16 * 1024),
                _ => true,
            }),
            "oversized reference definitions and raw HTML must use bounded source pages"
        );
        assert_eq!(
            document.source(),
            source,
            "fallback retains the original document"
        );
    }
}

#[test]
fn callout_fences_become_a_row_and_stay_literal_inside_code() {
    let source = "# 节\n\n前文\n\n::: tip\n提示 [链接](https://example.com)\n\n第二段\n:::\n\n后文\n\n```\n:::\n不是提示\n:::\n```\n\n::: 未闭合\n仍然是正文\n";
    let document = Document::parse(source.into()).unwrap();
    let callout = document
        .blocks()
        .iter()
        .find_map(|block| match block {
            Block::Callout { source, body } => Some((source.clone(), body.clone())),
            _ => None,
        })
        .expect("a paired container is its own row");
    assert_eq!(
        &document.source()[callout.1],
        "提示 [链接](https://example.com)\n\n第二段\n"
    );
    assert!(document.source()[callout.0].starts_with("::: tip\n"));
    assert!(
        document.blocks().iter().any(|block| {
            let text = &document.source()[block.source_range()];
            text.contains("不是提示") && text.contains(":::")
        }),
        "fences inside a code block stay with that block"
    );
    assert!(
        document.blocks().iter().any(|block| {
            matches!(block, Block::Markdown { .. })
                && document.source()[block.source_range()].contains("::: 未闭合")
        }),
        "an opening fence without a partner stays ordinary Markdown"
    );
    let found = SearchQuery::new("提示").unwrap().find(&document);
    assert!(
        found
            .iter()
            .any(|hit| { matches!(document.blocks()[hit.block_index()], Block::Callout { .. }) }),
        "search lands on the note, not the fence line"
    );
    assert!(
        SearchQuery::new(":::")
            .unwrap()
            .find(&document)
            .iter()
            .all(|hit| !matches!(document.blocks()[hit.block_index()], Block::Callout { .. })),
        "the hidden fence is not a search result"
    );
    let expanded = expand_callouts(source);
    assert!(expanded.contains("<div class=\"pecan-highlight pecan-highlight-titled\"></div>"));
    assert!(expanded.contains("**tip**"));
    assert!(expanded.contains("提示 [链接](https://example.com)"));
    assert!(expanded.contains("第二段"));
    assert!(expanded.contains("<div class=\"pecan-highlight-end\"></div>"));
    assert!(expanded.contains("```\n:::\n不是提示\n:::\n```"));
    assert!(expanded.contains("::: 未闭合"));
    assert_eq!(
        expanded.lines().filter(|line| line.trim() == ":::").count(),
        2,
        "only the fences inside the code block remain"
    );
    let listed = Document::parse(":::\n说明\n\n- 甲\n- 乙\n\n1. 有序\n:::\n".into()).unwrap();
    let Block::Callout { body, .. } = &listed.blocks()[0] else {
        panic!("a list inside a container stays in that container");
    };
    let body = &listed.source()[body.clone()];
    assert!(body.contains("- 甲\n- 乙"), "{body}");
    assert!(body.contains("1. 有序"), "{body}");

    assert_eq!(document.headings()[0].title(), "节");
    assert_eq!(
        document.blocks()[document.headings()[0].block_index()]
            .source_range()
            .start,
        document.headings()[0].source_range().start
    );
}

#[test]
fn github_alerts_keep_inner_lists_and_leave_ordinary_quotes() {
    assert_eq!(
        alert_marker("[!note] 标题").unwrap(),
        (AlertKind::Note, "标题")
    );
    assert_eq!(alert_marker("[!INFO]").unwrap().0, AlertKind::Info);
    assert_eq!(alert_marker("[!error]").unwrap().0, AlertKind::Error);
    assert!(alert_marker("[!ASIDE]").is_none());
    let source = "前文\n\n> [!WARNING]\n> 注意\n>\n> - 甲\n> - 乙\n\n后文\n\n> 普通引用\n\n```\n> [!NOTE]\n```\n";
    let expanded = expand_alerts(source);
    assert!(expanded.contains("class=\"pecan-highlight pecan-warning\""));
    assert!(expanded.contains("⚠️ **Warning**"));
    assert!(expanded.contains("- 甲\n- 乙"));
    assert!(expanded.contains("后文"));
    assert!(expanded.contains("> 普通引用"));
    assert!(expanded.contains("```\n> [!NOTE]\n```"));
    assert!(!expanded.contains("> [!WARNING]"));
}

#[test]
fn giant_heading_preserves_source_without_an_unbounded_outline_label() {
    let title = "中文标题".repeat(20_000);
    let source = format!("# {title}\n");
    let document = Document::parse(source.clone()).unwrap();
    assert!(
        document.headings()[0].title().len() <= 512,
        "outline labels have a bounded display cost"
    );
    assert!(title.starts_with(document.headings()[0].title().trim_end_matches('…')));
    assert_eq!(
        document.source(),
        source,
        "display truncation never changes source"
    );
}

#[test]
fn rich_callout_labels_keep_inline_markup() {
    for label in [
        "**标题** *italic* ~~旧版~~",
        "`**code 标题**`",
        "_italic_",
        "标题",
    ] {
        let source = format!("::: {label}\n正文\n:::\n");
        let expanded = expand_callouts(&source);
        assert!(
            expanded.contains(label),
            "the title must never be dropped: {label:?}"
        );
        assert!(
            expanded.contains("pecan-highlight-titled"),
            "titled container: {label:?}"
        );
        assert_eq!(Document::parse(source.clone()).unwrap().source(), source);
    }
}

#[test]
fn dense_structure_is_rejected_but_long_fenced_source_is_preserved() {
    let dense = "# tiny\n".repeat(500_001);
    assert!(
        Document::parse(dense).is_err(),
        "small bytes must not permit an unbounded semantic index"
    );
    let source = format!("```text\n{}```\n", "line\n".repeat(500_001));
    let document = Document::parse(source.clone()).unwrap();
    assert_eq!(document.source(), source);
    assert!(
        document
            .blocks()
            .iter()
            .all(|b| matches!(b, Block::CodePage { .. }))
    );
}

#[test]
fn many_callouts_keep_nested_headings_and_fenced_markers_intact() {
    for newline in ["\n", "\r\n"] {
        let part = "# Outside\n\n::: title\n\n## Inside\n\n~~~text\n::: literal\n~~~\n\n[text][target]\n\n:::\n\n".replace('\n', newline);
        let source = format!("{}[target]: https://example.com\n", part.repeat(2000));
        let document = Document::parse(source.clone()).unwrap();
        assert_eq!(document.headings().len(), 4000);
        assert_eq!(
            document
                .blocks()
                .iter()
                .filter(|b| matches!(b, Block::Callout { .. }))
                .count(),
            2000
        );
        for heading in document.headings().iter().filter(|h| h.title() == "Inside") {
            assert!(matches!(
                document.blocks()[heading.block_index()],
                Block::Callout { .. }
            ));
        }
        assert_eq!(document.source(), source);
        assert!(document.reference_suffix().contains("https://example.com"));
    }
}
