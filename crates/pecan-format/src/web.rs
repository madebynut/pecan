//! Parser-backed JavaScript, stylesheets, markup and SQL formatting.
use crate::{Format, FormattedCode, mapping::map_changes};
use anyhow::{Context as _, Result, ensure};
use dprint_plugin_typescript::{
    FormatTextOptions,
    configuration::{Configuration, ConfigurationBuilder, QuoteStyle, UseBraces},
};
use std::{borrow::Cow, path::Path, sync::OnceLock};

/// Bound automatic AST work and source mapping before calling third-party parsers.
pub(crate) const MAX_INPUT: usize = 64 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const MAX_DEPTH: usize = 96;
const MAX_PUNCTUATION: usize = 2048;

fn check(source: &str, indent: usize) -> Result<()> {
    ensure!(
        source.len() <= MAX_INPUT,
        "code exceeds the 64 KiB formatting budget"
    );
    ensure!(
        (1..=8).contains(&indent),
        "indentation must be between one and eight spaces"
    );
    let mut depth = 0usize;
    let mut punctuation = 0usize;
    // Conservative preflight: brackets inside literals may cause a safe rejection too.
    // Parsing is never attempted on unbounded recursive or operator-heavy examples.
    for byte in source.bytes() {
        if matches!(byte, b'(' | b'[' | b'{') {
            depth += 1;
        } else if matches!(byte, b')' | b']' | b'}') {
            depth = depth.saturating_sub(1);
        }
        ensure!(
            depth <= MAX_DEPTH,
            "code nesting exceeds the formatting budget"
        );
        punctuation += usize::from(byte.is_ascii_punctuation());
        ensure!(
            punctuation <= MAX_PUNCTUATION,
            "code complexity exceeds the formatting budget"
        );
    }
    ensure!(
        source.lines().all(|line| line
            .bytes()
            .take_while(|b| matches!(b, b' ' | b'\t'))
            .count()
            <= 192),
        "code indentation exceeds the formatting budget"
    );
    Ok(())
}

pub(crate) fn format(language: Format, source: &str, indent: usize) -> Result<FormattedCode> {
    check(source, indent)?;
    let mut text = match language {
        Format::JavaScript | Format::TypeScript | Format::Jsx | Format::Tsx => {
            script(language, source, indent)?
        }
        Format::Css | Format::Scss | Format::Sass | Format::Less => {
            stylesheet(language, source, indent)?
        }
        Format::Html | Format::Xml | Format::Vue | Format::Svelte | Format::Astro => {
            markup(language, source, indent)?
        }
        Format::Sql | Format::PostgreSql | Format::SqlServer => {
            let dialect = match language {
                Format::PostgreSql => sqlformat::Dialect::PostgreSql,
                Format::SqlServer => sqlformat::Dialect::SQLServer,
                _ => sqlformat::Dialect::Generic,
            };
            sqlformat::format(
                source,
                &sqlformat::QueryParams::None,
                &sqlformat::FormatOptions {
                    indent: sqlformat::Indent::Spaces(indent as u8),
                    dialect,
                    joins_as_top_level: true,
                    ..Default::default()
                },
            )
        }
        _ => unreachable!("configuration formats dispatch before web backends"),
    };
    ensure!(text.len() <= MAX_OUTPUT, "formatted output exceeds 1 MiB");
    if text.ends_with('\n') {
        text.pop();
    }
    map_changes(source, text)
}

fn script(language: Format, source: &str, indent: usize) -> Result<String> {
    static CONFIGS: [[OnceLock<Configuration>; 2]; 8] =
        [const { [const { OnceLock::new() }; 2] }; 8];
    let single = source.bytes().filter(|&b| b == b'\'').count()
        > source.bytes().filter(|&b| b == b'"').count();
    let config = CONFIGS[indent - 1][usize::from(single)].get_or_init(|| {
        ConfigurationBuilder::new()
            .indent_width(indent as u8)
            .line_width(96)
            .use_braces(UseBraces::Maintain)
            .quote_style(if single {
                QuoteStyle::PreferSingle
            } else {
                QuoteStyle::PreferDouble
            })
            .build()
    });
    let extension = match language {
        Format::JavaScript => "js",
        Format::Jsx => "jsx",
        Format::Tsx => "tsx",
        _ => "ts",
    };
    let text = dprint_plugin_typescript::format_text(FormatTextOptions {
        path: Path::new("code"),
        extension: Some(extension),
        text: source.to_owned(),
        config,
        external_formatter: None,
    })
    .context("JavaScript or TypeScript syntax is invalid")?;
    Ok(text.unwrap_or_else(|| source.to_owned()))
}

fn stylesheet(language: Format, source: &str, indent: usize) -> Result<String> {
    let syntax = match language {
        Format::Scss => malva::Syntax::Scss,
        Format::Sass => malva::Syntax::Sass,
        Format::Less => malva::Syntax::Less,
        _ => malva::Syntax::Css,
    };
    let options = malva::config::FormatOptions {
        layout: malva::config::LayoutOptions {
            indent_width: indent,
            print_width: 96,
            ..Default::default()
        },
        language: malva::config::LanguageOptions {
            hex_case: malva::config::HexCase::Ignore,
            align_comments: false,
            ..Default::default()
        },
    };
    malva::format_text(source, syntax, &options).context("stylesheet syntax is invalid")
}

fn markup(language: Format, source: &str, indent: usize) -> Result<String> {
    // Bound recursive markup generation, including malformed or implicit nesting.
    let mut depth = 0usize;
    for tag in source.split('<').skip(1) {
        let Some(end) = tag.find('>') else { continue };
        let tag = &tag[..end];
        if tag.starts_with('/') {
            depth = depth.saturating_sub(1);
        } else if !tag.starts_with(['!', '?']) && !tag.ends_with('/') {
            let name = tag.split_ascii_whitespace().next().unwrap_or("");
            if !matches!(
                name.to_ascii_lowercase().as_str(),
                "area"
                    | "base"
                    | "br"
                    | "col"
                    | "embed"
                    | "hr"
                    | "img"
                    | "input"
                    | "link"
                    | "meta"
                    | "param"
                    | "source"
                    | "track"
                    | "wbr"
            ) {
                depth += 1;
            }
        }
        ensure!(
            depth <= MAX_DEPTH,
            "markup nesting exceeds the formatting budget"
        );
    }
    let syntax = match language {
        Format::Xml => markup_fmt::Language::Xml,
        Format::Vue => markup_fmt::Language::Vue,
        Format::Svelte => markup_fmt::Language::Svelte,
        Format::Astro => markup_fmt::Language::Astro,
        _ => markup_fmt::Language::Html,
    };
    let options = markup_fmt::config::FormatOptions {
        layout: markup_fmt::config::LayoutOptions {
            indent_width: indent,
            print_width: 96,
            ..Default::default()
        },
        language: markup_fmt::config::LanguageOptions {
            closing_bracket_same_line: true,
            script_indent: true,
            style_indent: true,
            whitespace_sensitivity: markup_fmt::config::WhitespaceSensitivity::Strict,
            doctype_keyword_case: markup_fmt::config::DoctypeKeywordCase::Ignore,
            ..Default::default()
        },
    };
    markup_fmt::format_text(source, syntax, &options, |code, hints| {
        if hints.attr {
            return Ok(Cow::Borrowed(code));
        }
        match Format::for_language(hints.ext) {
            Some(
                format @ (Format::JavaScript
                | Format::TypeScript
                | Format::Jsx
                | Format::Tsx
                | Format::Css
                | Format::Scss
                | Format::Sass
                | Format::Less
                | Format::Json),
            ) => format
                .format(code, indent)
                .map(|text| Cow::Owned(text.text().to_owned()))
                .map_err(anyhow::Error::new),
            _ => Ok(Cow::Borrowed(code)),
        }
    })
    .context("markup syntax is invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_delimiters_keep_search_mapping() {
        let source = "function f(){if(true){return 1;}}";
        let output = script(Format::JavaScript, source, 2).unwrap();
        let formatted = map_changes(source, output.clone())
            .unwrap_or_else(|error| panic!("mapping {output:?}: {error}"));
        let at = source.find("return").unwrap();
        assert_eq!(&formatted.text()[formatted.range(at..at + 6)], "return");
    }
}
