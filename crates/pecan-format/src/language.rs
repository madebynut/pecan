//! Language detection and dispatch for bounded local formatting.
use crate::{FormatError, FormattedCode, config, json, web};
/// A supported grammar and its local formatting backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Format {
    /// JSON, JSONC and JSON-LD; preserves comments and literal numeric spelling.
    Json,
    /// YAML and YML; preserves decoded values, comments and anchors.
    Yaml,
    /// TOML; preserves literal spelling, comments and field order.
    Toml,
    /// JavaScript and ECMAScript modules.
    JavaScript,
    /// TypeScript and TypeScript modules.
    TypeScript,
    /// JavaScript with JSX expressions.
    Jsx,
    /// TypeScript with JSX expressions.
    Tsx,
    /// CSS stylesheets.
    Css,
    /// SCSS stylesheets.
    Scss,
    /// Indentation-based Sass stylesheets.
    Sass,
    /// Less stylesheets.
    Less,
    /// HTML fragments and documents.
    Html,
    /// XML and SVG documents.
    Xml,
    /// Vue single-file components.
    Vue,
    /// Svelte components.
    Svelte,
    /// Astro components.
    Astro,
    /// SQL with generic quoting and parameter syntax.
    Sql,
    /// PostgreSQL SQL, including array operators.
    PostgreSql,
    /// SQL Server SQL, including bracketed names and variables.
    SqlServer,
}
impl Format {
    /// Resolve a Markdown fence label, including common aliases and mixed case.
    pub fn for_language(language: &str) -> Option<Self> {
        match language
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_lowercase()
            .as_str()
        {
            "json" | "jsonc" | "json-ld" | "jsonld" => Some(Self::Json),
            "yaml" | "yml" => Some(Self::Yaml),
            "toml" => Some(Self::Toml),
            "js" | "javascript" | "mjs" | "cjs" => Some(Self::JavaScript),
            "ts" | "typescript" | "mts" | "cts" => Some(Self::TypeScript),
            "jsx" => Some(Self::Jsx),
            "tsx" => Some(Self::Tsx),
            "css" => Some(Self::Css),
            "scss" => Some(Self::Scss),
            "sass" => Some(Self::Sass),
            "less" => Some(Self::Less),
            "html" | "htm" => Some(Self::Html),
            "xml" | "svg" | "xsd" | "xslt" | "xsl" | "wsdl" => Some(Self::Xml),
            "vue" => Some(Self::Vue),
            "svelte" => Some(Self::Svelte),
            "astro" => Some(Self::Astro),
            "sql" | "mysql" | "sqlite" | "sqlite3" | "mariadb" | "oracle" => Some(Self::Sql),
            "postgres" | "postgresql" | "pg" | "pgsql" | "psql" => Some(Self::PostgreSql),
            "mssql" | "sqlserver" | "tsql" | "t-sql" => Some(Self::SqlServer),
            _ => None,
        }
    }
    /// Format with one to eight spaces per indentation level.
    ///
    /// # Errors
    /// Rejects invalid syntax, unsafe source mappings and input exceeding bounded resources.
    pub fn format(self, source: &str, indent: usize) -> Result<FormattedCode, FormatError> {
        let result = match self {
            Self::Json => json::json(source, indent),
            Self::Yaml => config::yaml(source, indent),
            Self::Toml => config::toml(source, indent),
            _ => web::format(self, source, indent),
        };
        result.map_err(FormatError::new)
    }
    /// Maximum accepted input bytes for this backend.
    pub fn input_limit(self) -> usize {
        match self {
            Self::Json => json::MAX_INPUT,
            Self::Yaml | Self::Toml => config::MAX_INPUT,
            _ => web::MAX_INPUT,
        }
    }
    /// Canonical syntax label for a code editor; independent of any UI framework.
    pub fn syntax_language(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::JavaScript | Self::Jsx => "javascript",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::Css | Self::Scss | Self::Sass | Self::Less => "css",
            Self::Html | Self::Xml | Self::Vue | Self::Svelte | Self::Astro => "html",
            Self::Sql | Self::PostgreSql | Self::SqlServer => "sql",
        }
    }
}
/// Detect common indentation once, with bounded sampling; literal tabs use the fallback.
pub fn tab_width(source: &str, preference: &str) -> usize {
    if let Ok(width @ (2 | 4 | 8)) = preference.parse::<usize>() {
        return width;
    }
    let mut votes = [0usize; 9];
    let mut previous = 0;
    let mut sampled = 0;
    for line in source.lines().take(512) {
        sampled += line.len();
        if sampled > 64 * 1024 {
            break;
        }
        let indent = line.bytes().take(65).take_while(|b| *b == b' ').count();
        let content = &line[indent..];
        if indent > 64 || content.is_empty() || content.starts_with(['\t', '/', '#']) {
            continue;
        }
        let delta = indent.saturating_sub(previous);
        if (2..=8).contains(&delta) {
            votes[delta] += 1;
        }
        previous = indent;
    }
    (2..=8)
        .filter(|&n| votes[n] > 0)
        .max_by_key(|&n| (votes[n], std::cmp::Reverse(n)))
        .unwrap_or(4)
}
