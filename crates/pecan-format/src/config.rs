//! Configuration formatters validate values, comments and source mappings before publication.
use crate::{
    FormattedCode,
    mapping::{Token, map_changes},
};
use anyhow::{Context as _, Result, ensure};
use logos::Logos as _;
use taplo::syntax::SyntaxKind;
use yaml_rust2::{parser::Event, scanner::TScalarStyle};

/// Bound the CST and character diff allocations independently of the JSON fast path.
pub(crate) const MAX_INPUT: usize = 256 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_EVENTS: usize = 200_000;

fn check_input(source: &str, indent: usize) -> Result<()> {
    ensure!(
        source.len() <= MAX_INPUT,
        "配置代码超过 256 KiB，保留原始显示"
    );
    ensure!((1..=8).contains(&indent), "缩进宽度必须在 1–8 之间");
    Ok(())
}

/// Compare parser events without expanding aliases; preserve values, tags, types and ordering.
fn yaml_events(source: &str) -> Result<Vec<Event>> {
    // Pretty YAML's CST parser represents indentation in a 64-bit bitset.
    ensure!(
        source
            .lines()
            .all(|line| line.bytes().take_while(|b| *b == b' ').count() < 64),
        "YAML 缩进超过 63 列，保留原始显示"
    );
    let mut parser = yaml_rust2::parser::Parser::new_from_str(source);
    let mut events = Vec::new();
    let mut depth = 0usize;
    loop {
        let (mut event, _) = parser.next_token().context("YAML 语法无效")?;
        match &mut event {
            Event::SequenceStart(..) | Event::MappingStart(..) => {
                depth += 1;
                ensure!(depth <= MAX_DEPTH, "配置嵌套超过 32 层，保留原始显示");
            }
            Event::SequenceEnd | Event::MappingEnd => depth = depth.saturating_sub(1),
            Event::Scalar(_, style, _, _) if *style != TScalarStyle::Plain => {
                // Quoting or block style may change, but the decoded string must remain identical.
                *style = TScalarStyle::DoubleQuoted;
            }
            _ => {}
        }
        ensure!(events.len() < MAX_EVENTS, "配置过于复杂，保留原始显示");
        let end = event == Event::StreamEnd;
        events.push(event);
        if end {
            return Ok(events);
        }
    }
}

pub(crate) fn yaml(source: &str, indent: usize) -> Result<FormattedCode> {
    check_input(source, indent)?;
    let before = yaml_events(source)?;
    let mut estimate = source.len() * 2;
    let mut depth = 0usize;
    for event in &before {
        match event {
            Event::SequenceStart(..) | Event::MappingStart(..) => depth += 1,
            Event::SequenceEnd | Event::MappingEnd => depth = depth.saturating_sub(1),
            _ => {}
        }
        estimate += depth * indent + 4;
    }
    ensure!(
        estimate <= MAX_OUTPUT,
        "配置展开后可能超过 1 MiB，保留原始显示"
    );
    let options = pretty_yaml::config::FormatOptions {
        layout: pretty_yaml::config::LayoutOptions {
            indent_width: indent,
            print_width: 96,
            ..Default::default()
        },
        language: pretty_yaml::config::LanguageOptions {
            format_comments: false,
            trim_trailing_whitespaces: false,
            trim_trailing_zero: false,
            ..Default::default()
        },
    };
    let mut text = pretty_yaml::format_text(source, &options).context("YAML 格式化失败")?;
    ensure!(
        text.len() <= MAX_OUTPUT,
        "格式化结果超过 1 MiB，保留原始显示"
    );
    if before != yaml_events(&text)? {
        // Some printers collapse terminal blank lines belonging to a |+/>+ scalar.
        // Restore that original suffix, then revalidate rather than accepting changed values.
        let trailing_lines = source
            .bytes()
            .rev()
            .take_while(|b| matches!(b, b'\n' | b'\r'))
            .filter(|b| *b == b'\n')
            .count();
        text.truncate(text.trim_end_matches('\n').len());
        ensure!(
            text.len() + trailing_lines <= MAX_OUTPUT,
            "格式化结果超过 1 MiB，保留原始显示"
        );
        text.extend(std::iter::repeat_n('\n', trailing_lines));
        ensure!(
            before == yaml_events(&text)?,
            "格式化改变了 YAML 值或结构，保留原始显示"
        );
    }
    // The formatter's final layout newline is not an extra code row.
    if text.ends_with('\n') {
        let candidate = &text[..text.len() - 1];
        // A trailing newline can belong to a |+ block scalar rather than the layout.
        if yaml_events(candidate).is_ok_and(|events| events == before) {
            text.pop();
        }
    }
    map_changes(source, text)
}

fn toml_tokens(source: &str) -> Result<Vec<(SyntaxKind, std::ops::Range<usize>)>> {
    let mut lexer = SyntaxKind::lexer(source);
    let mut depth = 0usize;
    let mut tokens = Vec::new();
    while let Some(kind) = lexer.next() {
        match kind {
            SyntaxKind::BRACKET_START | SyntaxKind::BRACE_START => {
                depth += 1;
                ensure!(depth <= MAX_DEPTH, "配置嵌套超过 32 层，保留原始显示");
            }
            SyntaxKind::BRACKET_END | SyntaxKind::BRACE_END => depth = depth.saturating_sub(1),
            SyntaxKind::ERROR => anyhow::bail!("TOML 词法无效，保留原始显示"),
            _ => {}
        }
        if !matches!(
            kind,
            SyntaxKind::WHITESPACE | SyntaxKind::NEWLINE | SyntaxKind::COMMA
        ) {
            ensure!(tokens.len() < MAX_EVENTS, "配置过于复杂，保留原始显示");
            tokens.push((kind, lexer.span()));
        }
    }
    Ok(tokens)
}

pub(crate) fn toml(source: &str, indent: usize) -> Result<FormattedCode> {
    check_input(source, indent)?;
    // Lex before the recursive CST parser so excessive nesting cannot exhaust its stack.
    let before = toml_tokens(source)?;
    let mut estimate = source.len() * 2;
    let mut depth = 0usize;
    for (kind, _) in &before {
        match kind {
            SyntaxKind::BRACKET_START | SyntaxKind::BRACE_START => depth += 1,
            SyntaxKind::BRACKET_END | SyntaxKind::BRACE_END => depth = depth.saturating_sub(1),
            _ => {}
        }
        estimate += depth * indent + 4;
    }
    ensure!(
        estimate <= MAX_OUTPUT,
        "配置展开后可能超过 1 MiB，保留原始显示"
    );
    let parsed = taplo::parser::parse(source);
    ensure!(
        parsed.errors.is_empty(),
        "TOML 语法无效：{:?}",
        parsed.errors.first()
    );
    let text = taplo::formatter::format_syntax(
        parsed.into_syntax(),
        taplo::formatter::Options {
            indent_string: " ".repeat(indent),
            column_width: 96,
            array_auto_collapse: false,
            align_comments: false,
            trailing_newline: false,
            ..Default::default()
        },
    );
    ensure!(
        text.len() <= MAX_OUTPUT,
        "格式化结果超过 1 MiB，保留原始显示"
    );
    let after = toml_tokens(&text)?;
    ensure!(
        before.len() == after.len()
            && before
                .iter()
                .zip(&after)
                .all(|((kind, a), (new_kind, b))| kind == new_kind
                    && source[a.clone()] == text[b.clone()]),
        "格式化改变了 TOML 值、注释或顺序，保留原始显示"
    );
    ensure!(
        taplo::parser::parse(&text).errors.is_empty(),
        "格式化产生了无效 TOML，保留原始显示"
    );
    let tokens = before
        .into_iter()
        .zip(after)
        .map(|((_, source), (_, display))| Token {
            source,
            display: display.start,
        })
        .collect();
    Ok(FormattedCode { text, tokens })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_width_values_comments_anchors_and_search_survive() {
        let source = "# 配置\ndefaults: &defaults\n  port: 8080 # 端口\n  label: '设备'\n  text: |\n    第一行\n      第二行\nservers:\n  - name: \"alpha\"\n    options: *defaults\n---\nflow: {a: 1.2300, b: [true, false]}\n";
        for width in [2, 4, 8] {
            let formatted = yaml(source, width).unwrap();
            assert_eq!(
                yaml(&formatted.text, width).unwrap().text,
                formatted.text,
                "width {width}"
            );
            assert_eq!(
                yaml_events(source).unwrap(),
                yaml_events(&formatted.text).unwrap()
            );
            assert!(
                formatted
                    .text
                    .contains(&format!("\n{}port:", " ".repeat(width)))
            );
            for needle in ["配置", "端口", "设备", "第一行", "alpha", "1.2300"] {
                let at = source.find(needle).unwrap();
                assert_eq!(
                    &formatted.text[formatted.range(at..at + needle.len())],
                    needle,
                    "width {width}: {needle}"
                );
            }
            assert!(formatted.text.contains("# 端口"));
            assert!(formatted.text.contains("&defaults"));
            assert!(formatted.text.contains("*defaults"));
            assert!(!formatted.text.ends_with('\n'));
        }
    }

    #[test]
    fn toml_width_literals_comments_order_and_search_survive() {
        let source = "# 配置\n[reader]\nname=\"设备\" # 标签\nnumber=1.2300e+04\nports=[\n  8080, # 保留\n  8081,\n]\ntext='''第一行\n    第二行'''\n";
        for width in [2, 4, 8] {
            let formatted = toml(source, width).unwrap();
            assert_eq!(toml(&formatted.text, width).unwrap().text, formatted.text);
            assert!(
                formatted
                    .text
                    .contains(&format!("\n{}8080,", " ".repeat(width)))
            );
            assert!(formatted.text.contains("'''第一行\n    第二行'''"));
            for needle in ["设备", "标签", "1.2300e+04", "8080", "保留"] {
                let at = source.find(needle).unwrap();
                assert_eq!(
                    &formatted.text[formatted.range(at..at + needle.len())],
                    needle,
                    "width {width}: {needle}"
                );
            }
        }
    }

    #[test]
    fn invalid_or_over_budget_configuration_never_produces_partial_results() {
        for bad in ["a: [", "a:\n\tb: 1", "a: \"unterminated"] {
            assert!(yaml(bad, 2).is_err(), "{bad:?}");
        }
        for bad in ["a = [", "a = \"unterminated", "a = 1\n[bad"] {
            assert!(toml(bad, 2).is_err(), "{bad:?}");
        }
        let deep = format!("{}0{}", "[".repeat(5000), "]".repeat(5000));
        assert!(yaml(&deep, 2).is_err());
        assert!(toml(&format!("a = {deep}"), 2).is_err());
        for format in [super::super::Format::Yaml, super::super::Format::Toml] {
            assert!(format.format(&" ".repeat(MAX_INPUT + 1), 2).is_err());
            assert!(format.format("a: 1", 0).is_err());
        }
    }

    #[test]
    fn yaml_trailing_block_scalars_keep_chomping_semantics() {
        for suffix in ["|", "|-", "|+", ">", ">-", ">+"] {
            for tail in ["\n", "\n\n", "\n\n\n"] {
                let source = format!("message: {suffix}\n  value{tail}");
                let formatted = yaml(&source, 4).unwrap();
                assert_eq!(
                    yaml_events(&source).unwrap(),
                    yaml_events(&formatted.text).unwrap(),
                    "{source:?}"
                );
                assert_eq!(
                    yaml(&formatted.text, 4).unwrap().text,
                    formatted.text,
                    "{source:?}"
                );
            }
        }
    }
}
