//! Bounded semantic ranges for portable exports.
use crate::Appearance;
use gpui_kit::{
    FontStyle, FontWeight, base::input::Rope, component::highlighter::SyntaxHighlighter,
};
use std::ops::Range;
// Extremely long code is still exported in full, without syntax parsing.
const MAX_HIGHLIGHT: usize = 256 * 1024;
const MAX_HIGHLIGHT_LINE: usize = 16 * 1024;

/// Styling of one source range; `None` color inherits the document ink.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Ink {
    pub rgb: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
}

/// Contiguous, ordered ranges covering all of `code`.
pub fn spans(code: &str, language: &str, appearance: Appearance) -> Vec<(Range<usize>, Ink)> {
    let plain = vec![(0..code.len(), Ink::default())];
    if code.is_empty()
        || language.is_empty()
        || code.len() > MAX_HIGHLIGHT
        || code.lines().any(|line| line.len() > MAX_HIGHLIGHT_LINE)
    {
        return plain;
    }
    // Also registers the reader's grammar aliases, such as `json-ld`.
    let theme = crate::native_theme(appearance);
    let language = crate::language(language);
    let mut highlighter = SyntaxHighlighter::new(&language);
    highlighter.update(None, &Rope::from_str(code), None);
    let mut output = Vec::new();
    let mut end = 0;
    for (range, style) in highlighter.styles(&(0..code.len()), theme.as_ref()) {
        if range.start < end || range.is_empty() {
            continue;
        }
        if range.start > end {
            output.push((end..range.start, Ink::default()));
        }
        end = range.end;
        let rgb = style.color.map(|color| {
            let color = color.to_rgb();
            [color.r, color.g, color.b].map(|channel| (channel * 255.).round() as u8)
        });
        output.push((
            range,
            Ink {
                rgb,
                bold: style
                    .font_weight
                    .is_some_and(|weight| weight >= FontWeight::SEMIBOLD),
                italic: style.font_style == Some(FontStyle::Italic),
            },
        ));
    }
    if end < code.len() {
        output.push((end..code.len(), Ink::default()));
    }
    if output.is_empty() { plain } else { output }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spans(code: &str, language: &str) -> Vec<(std::ops::Range<usize>, Ink)> {
        super::spans(code, language, Appearance::Light)
    }

    fn ink_at(code: &str, language: &str, needle: &str) -> Ink {
        let offset = code.find(needle).expect("token exists in fixture");
        spans(code, language)
            .into_iter()
            .find(|(range, _)| range.contains(&offset))
            .unwrap()
            .1
    }

    #[test]
    fn common_languages_and_database_aliases_color_semantic_tokens() {
        let mut errors = Vec::new();
        for (language, code, keyword, value) in [
            (
                "SQL",
                "-- comment\nSELECT name FROM devices WHERE name = 'Pecan' AND id = 42;",
                "SELECT",
                "'Pecan'",
            ),
            ("MySQL", "SELECT 'Pecan' FROM devices;", "SELECT", "'Pecan'"),
            ("pgsql", "SELECT 'Pecan' FROM devices;", "SELECT", "'Pecan'"),
            (
                "TSX",
                "// comment\ninterface Props { name: string }\nconst View = () => <div title=\"Pecan\">Hello</div>;",
                "const",
                "Pecan",
            ),
            (
                "jsx",
                "// comment\nconst View = () => <div title=\"Pecan\">Hello</div>;",
                "const",
                "Pecan",
            ),
            (
                "TS",
                "// comment\nconst name: string = \"Pecan\";",
                "const",
                "Pecan",
            ),
            (
                "mjs",
                "// comment\nconst name = \"Pecan\";",
                "const",
                "Pecan",
            ),
            (
                "C",
                "// comment\nint main() { return puts(\"Pecan\"); }",
                "return",
                "Pecan",
            ),
            (
                "C++",
                "// comment\nint main() { return puts(\"Pecan\"); }",
                "return",
                "Pecan",
            ),
            (
                "C#",
                "// comment\nclass View { string name = \"Pecan\"; }",
                "class",
                "Pecan",
            ),
            (
                "Java",
                "// comment\nclass View { String name = \"Pecan\"; }",
                "class",
                "Pecan",
            ),
            (
                "Go",
                "package main\nfunc main() { println(\"Pecan\") }",
                "func",
                "Pecan",
            ),
            (
                "Rust",
                "// comment\nfn main() { let name = \"Pecan\"; }",
                "fn",
                "Pecan",
            ),
            (
                "Python",
                "# comment\ndef main():\n    return \"Pecan\"",
                "return",
                "Pecan",
            ),
            (
                "Ruby",
                "# comment\ndef main\n  return \"Pecan\"\nend",
                "return",
                "Pecan",
            ),
            (
                "Swift",
                "// comment\nfunc main() -> String { return \"Pecan\" }",
                "return",
                "Pecan",
            ),
            (
                "Kotlin",
                "// comment\nfun main() { val name = \"Pecan\" }",
                "fun",
                "Pecan",
            ),
            (
                "Scala",
                "// comment\nobject Main { val name = \"Pecan\" }",
                "object",
                "Pecan",
            ),
            (
                "Lua",
                "-- comment\nlocal name = \"Pecan\"",
                "local",
                "Pecan",
            ),
            (
                "Elixir",
                "# comment\ndef name do\n  \"Pecan\"\nend",
                "do",
                "Pecan",
            ),
            (
                "GraphQL",
                "# comment\nquery Reader { device(name: \"Pecan\") { id } }",
                "query",
                "Pecan",
            ),
            (
                "Proto",
                "syntax = \"proto3\";\nmessage View { string name = 1; }",
                "message",
                "proto3",
            ),
            (
                "Bash",
                "# comment\nif true; then echo \"Pecan\"; fi",
                "if",
                "Pecan",
            ),
            (
                "CMake",
                "# comment\nif(TRUE)\nmessage(\"Pecan\")\nendif()",
                "if",
                "Pecan",
            ),
        ] {
            let key = ink_at(code, language, keyword);
            let string = ink_at(code, language, value);
            if key.rgb != Some([0x82, 0x50, 0xdf]) {
                errors.push(format!("{language}: keyword {keyword}: {:?}", key.rgb));
            }
            if string.rgb != Some([0x0a, 0x64, 0x40]) {
                errors.push(format!("{language}: string {value}: {:?}", string.rgb));
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
        for language in ["tsx", "jsx"] {
            assert_eq!(
                ink_at("const V = () => <div />;", language, "div").rgb,
                Some([0x11, 0x6b, 0x80]),
                "{language}: JSX tag"
            );
        }
        assert_eq!(
            ink_at("SELECT 42;", "sql", "42").rgb,
            Some([0x05, 0x50, 0xae]),
            "SQL numeric literals must not look like strings"
        );
    }

    #[test]
    fn embedded_scripts_styles_and_template_tokens_are_highlighted() {
        for (language, source, token, expected) in [
            (
                "html",
                "<script>const name = \"Pecan\";</script>",
                "const",
                [0x82, 0x50, 0xdf],
            ),
            (
                "vue",
                "<script lang=\"ts\">interface Props { name: string }</script>",
                "interface",
                [0x82, 0x50, 0xdf],
            ),
            (
                "svelte",
                "<script lang=\"ts\">const name: string = \"Pecan\";</script><h1>{name}</h1>",
                "const",
                [0x82, 0x50, 0xdf],
            ),
            (
                "astro",
                "---\nconst name: string = \"Pecan\";\n---\n<h1>{name}</h1>",
                "const",
                [0x82, 0x50, 0xdf],
            ),
            (
                "tsx",
                "const query = sql`SELECT name FROM devices`;",
                "SELECT",
                [0x82, 0x50, 0xdf],
            ),
            (
                "ejs",
                "<p><% if (ok) { %><%= \"Pecan\" %><% } %></p>",
                "if",
                [0x82, 0x50, 0xdf],
            ),
            (
                "erb",
                "<p><% if ok %><%= \"Pecan\" %><% end %></p>",
                "if",
                [0x82, 0x50, 0xdf],
            ),
        ] {
            assert_eq!(
                ink_at(source, language, token).rgb,
                Some(expected),
                "{language}: embedded {token}"
            );
        }
    }

    #[test]
    fn spans_cover_the_source_in_order_with_semantic_colors() {
        for (language, code) in [
            (
                "json-ld",
                "{\"@type\": \"Device\", \"version\": 42, \"on\": true}\n",
            ),
            ("rust", "fn main() {\n    let 名称 = \"值\"; // 注释\n}\n"),
            ("TOML", "# 配置\n[reader]\nname = \"设备\"\nport = 8080\n"),
            ("unknown-language", "plain text\n"),
        ] {
            let spans = spans(code, language);
            let mut end = 0;
            for (range, _) in &spans {
                assert_eq!(range.start, end, "{language}: ranges are contiguous");
                assert!(code.is_char_boundary(range.end));
                end = range.end;
            }
            assert_eq!(end, code.len(), "{language}: the whole source is covered");
        }
        let colors: std::collections::HashSet<_> = spans("{\"a\": 1, \"b\": true}", "json")
            .into_iter()
            .filter_map(|(_, ink)| ink.rgb)
            .collect();
        assert!(
            colors.len() >= 2,
            "JSON keys and values have distinct colors"
        );
        let colors: std::collections::HashSet<_> =
            spans("# 配置\n[reader]\nname = \"设备\"\nport = 8080\n", "TOML")
                .into_iter()
                .filter_map(|(_, ink)| ink.rgb)
                .collect();
        assert!(colors.len() >= 2, "TOML exports need semantic colors");
    }
}
