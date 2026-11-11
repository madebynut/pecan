//! Cached lexical highlighting for the readonly Mermaid source pane.

use std::{ops::Range, rc::Rc, sync::OnceLock};

use gpui_kit::{
    Context, HighlightStyle, SharedString, Window,
    base::input::{
        EditorState, FoldRange, HighlightStyleResolver, InputEdit, InputHighlighter, Rope,
    },
};
use regex::Regex;

#[derive(Debug, Default)]
struct MermaidHighlighter {
    tokens: Vec<(Range<usize>, &'static str)>,
}

pub(crate) fn editor(
    source: &str,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) -> EditorState {
    let mut editor = EditorState::new(window, cx)
        .language("mermaid")
        .folding(false)
        .default_value(source.to_owned());
    editor.set_highlighter_factory(
        Rc::new(|language| {
            (language == "mermaid")
                .then(|| Box::new(MermaidHighlighter::default()) as Box<dyn InputHighlighter>)
        }),
        cx,
    );
    editor
}

impl MermaidHighlighter {
    fn scan(&mut self, source: &str) {
        // One compiled linear-time lexer, reused across cards. Labels and comments
        // are consumed before keywords so their prose cannot become syntax.
        static LEXER: OnceLock<Regex> = OnceLock::new();
        let lexer = LEXER.get_or_init(|| {
            Regex::new(r#"(?m)(?P<comment>%%[^\r\n]*)|(?P<string>"(?:\\.|[^"\\])*"|`(?:\\.|[^`\\])*`|\|[^|\r\n]*\||\[[^\]\r\n]*\])|(?P<number>\#[0-9a-fA-F]{3,8}\b|\b[0-9]+(?:\.[0-9]+)?\b)|(?P<operator>[-=.<ox]*[-=][-=.>ox]*|:::+)|(?P<word>[\p{L}_][\p{L}\p{N}_]*(?:-(?:beta|v2|pick))?)|(?P<punctuation>[{}();:,])"#)
                .expect("embedded Mermaid lexer is valid")
        });
        self.tokens.clear();
        // Oversized fences remain fully selectable; only colorization is capped
        // at the renderer's 128 KiB source budget.
        let mut end = source.len().min(128 * 1024);
        while !source.is_char_boundary(end) {
            end -= 1;
        }
        for capture in lexer.captures_iter(&source[..end]) {
            let (token, kind) = if let Some(token) = capture.name("word") {
                let kind = if keyword(token.as_str()) {
                    "keyword"
                } else {
                    "variable"
                };
                (token, kind)
            } else {
                ["comment", "string", "number", "operator", "punctuation"]
                    .into_iter()
                    .find_map(|kind| capture.name(kind).map(|token| (token, kind)))
                    .expect("each lexer match has a named token")
            };
            self.tokens.push((token.range(), kind));
        }
    }
}

fn keyword(word: &str) -> bool {
    matches!(
        word,
        "flowchart"
            | "graph"
            | "sequenceDiagram"
            | "classDiagram"
            | "stateDiagram"
            | "stateDiagram-v2"
            | "erDiagram"
            | "journey"
            | "gantt"
            | "pie"
            | "mindmap"
            | "timeline"
            | "gitGraph"
            | "quadrantChart"
            | "xychart"
            | "xychart-beta"
            | "sankey"
            | "sankey-beta"
            | "block"
            | "block-beta"
            | "packet"
            | "packet-beta"
            | "architecture"
            | "architecture-beta"
            | "requirementDiagram"
            | "zenuml"
            | "C4Context"
            | "C4Container"
            | "C4Component"
            | "C4Dynamic"
            | "C4Deployment"
            | "LR"
            | "RL"
            | "TD"
            | "TB"
            | "BT"
            | "subgraph"
            | "end"
            | "direction"
            | "participant"
            | "actor"
            | "as"
            | "autonumber"
            | "activate"
            | "deactivate"
            | "alt"
            | "else"
            | "opt"
            | "loop"
            | "par"
            | "and"
            | "critical"
            | "option"
            | "break"
            | "rect"
            | "Note"
            | "note"
            | "over"
            | "left"
            | "right"
            | "of"
            | "class"
            | "classDef"
            | "style"
            | "linkStyle"
            | "click"
            | "callback"
            | "href"
            | "state"
            | "fork"
            | "join"
            | "choice"
            | "title"
            | "section"
            | "dateFormat"
            | "axisFormat"
            | "excludes"
            | "includes"
            | "tickInterval"
            | "todayMarker"
            | "accTitle"
            | "accDescr"
            | "commit"
            | "branch"
            | "checkout"
            | "merge"
            | "cherry-pick"
            | "service"
            | "group"
            | "junction"
            | "requirement"
            | "element"
            | "id"
            | "type"
            | "text"
            | "risk"
            | "verifyMethod"
            | "satisfies"
            | "contains"
    )
}

impl InputHighlighter for MermaidHighlighter {
    fn language(&self) -> SharedString {
        "mermaid".into()
    }

    fn update(
        &mut self,
        _: Option<InputEdit>,
        text: &Rope,
        _: bool,
        _: &mut Window,
        _: &mut Context<EditorState>,
    ) {
        self.scan(&text.to_string());
    }

    fn styles(
        &self,
        range: &Range<usize>,
        resolver: &dyn HighlightStyleResolver,
    ) -> Vec<(Range<usize>, HighlightStyle)> {
        let mut styles = Vec::new();
        let mut offset = range.start;
        let first = self
            .tokens
            .partition_point(|(token, _)| token.end <= range.start);
        for (token, kind) in self.tokens[first..]
            .iter()
            .take_while(|(token, _)| token.start < range.end)
        {
            let start = token.start.max(range.start);
            let end = token.end.min(range.end);
            if start > offset {
                styles.push((offset..start, HighlightStyle::default()));
            }
            styles.push((start..end, resolver.style(kind).unwrap_or_default()));
            offset = end;
        }
        if offset < range.end {
            styles.push((offset..range.end, HighlightStyle::default()));
        }
        styles
    }

    fn fold_ranges(&self, _: &Rope) -> Vec<FoldRange> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_comments_preserve_utf8_and_do_not_become_keywords() {
        let source = "sequenceDiagram\n participant A as 阅读器\n alt ready\n A->>B: \"loop 中文\\\"标签\"\n %% end --> 中文\n end\n";
        let mut parser = MermaidHighlighter::default();
        parser.scan(source);
        for (range, _) in &parser.tokens {
            assert!(source.get(range.clone()).is_some());
        }
        assert!(
            parser
                .tokens
                .iter()
                .any(|(r, k)| *k == "operator" && &source[r.clone()] == "->>")
        );
        assert!(
            parser
                .tokens
                .iter()
                .any(|(r, k)| *k == "comment" && &source[r.clone()] == "%% end --> 中文")
        );
        assert!(
            parser
                .tokens
                .iter()
                .any(|(r, k)| *k == "string" && source[r.clone()].contains("loop 中文"))
        );
        assert!(
            parser
                .tokens
                .windows(2)
                .all(|tokens| tokens[0].0.end <= tokens[1].0.start)
        );
    }

    #[test]
    fn oversized_source_keeps_a_plain_tail_without_scanning_it() {
        let source = "中文 --> A\n".repeat(20_000);
        let mut parser = MermaidHighlighter::default();
        parser.scan(&source);
        assert!(parser.tokens.last().unwrap().0.end <= 128 * 1024);
        let json = include_str!("../../../assets/themes/light.json");
        let theme: gpui_kit::component::highlighter::HighlightTheme =
            serde_json::from_str(json).unwrap();
        let range = source.len() - 10..source.len();
        assert_eq!(
            parser.styles(&range, &theme),
            vec![(range, HighlightStyle::default())]
        );
    }

    #[test]
    fn visible_runs_cover_partial_ranges_and_follow_both_themes() {
        use gpui_kit::component::highlighter::HighlightTheme;
        let source = "flowchart LR\n A[中文标签] --> B\n style B fill:#fff\n %% note\n";
        let mut parser = MermaidHighlighter::default();
        parser.scan(source);
        for json in [
            include_str!("../../../assets/themes/light.json"),
            include_str!("../../../assets/themes/dark.json"),
        ] {
            let theme: HighlightTheme = serde_json::from_str(json).unwrap();
            for range in [
                0..source.len(),
                3..source.find("标签").unwrap(),
                source.len()..source.len(),
            ] {
                let styles = parser.styles(&range, &theme);
                let mut offset = range.start;
                for (part, _) in &styles {
                    assert_eq!(part.start, offset);
                    offset = part.end;
                }
                assert_eq!(offset, range.end);
                if range.start == 0 {
                    let colors: Vec<_> =
                        styles.iter().filter_map(|(_, style)| style.color).collect();
                    assert!(colors.iter().any(|c| colors.iter().any(|other| c != other)));
                }
                assert_eq!(
                    styles,
                    parser.styles(&range, &theme),
                    "redraw reuses tokens"
                );
            }
        }
    }
}
