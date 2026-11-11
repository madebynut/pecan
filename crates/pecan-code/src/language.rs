//! Shared fence aliases for native Tree-sitter editors and portable exports.
//! Grammars and queries are supplied by gpui-kit; formatting support is independent.

/// Preserve the original fence label in toolbars, but normalize it for parsing.
pub fn language(label: &str) -> String {
    let label = label
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let canonical = match label.as_str() {
        "jsonc" | "jsonld" | "json-ld" => "json",
        "js" | "mjs" | "cjs" | "jsx" | "javascriptreact" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "typescriptreact" => "tsx",
        "sh" | "shell" | "zsh" | "shellscript" => "bash",
        "yml" => "yaml",
        "py" | "pyi" | "python3" => "python",
        "rs" => "rust",
        "h" => "c",
        "c++" | "cc" | "cxx" | "hpp" | "h++" | "hxx" => "cpp",
        "cs" | "c#" => "csharp",
        "golang" => "go",
        "rb" | "rake" | "gemspec" => "ruby",
        "kt" | "kts" | "ktm" => "kotlin",
        "ex" | "exs" | "iex" => "elixir",
        "gql" => "graphql",
        "protobuf" | "proto3" => "proto",
        "makefile" | "gnumake" => "make",
        "cmakelists" => "cmake",
        "patch" => "diff",
        "md" | "mdx" => "markdown",
        "php3" | "php4" | "php5" | "phtml" => "php",
        "mysql" | "sqlite" | "sqlite3" | "mariadb" | "oracle" | "postgres" | "postgresql"
        | "pg" | "pgsql" | "psql" | "mssql" | "sqlserver" | "tsql" | "t-sql" | "plsql" => "sql",
        // These use host grammars; dedicated dialect syntax may remain uncolored.
        "scss" | "sass" | "less" => "css",
        "htm" | "xml" | "svg" | "xsd" | "xslt" | "xsl" | "wsdl" | "vue" => "html",
        "plain" | "plaintext" | "txt" | "" => "text",
        _ => return label,
    };
    canonical.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::component::highlighter::{LanguageRegistry, SyntaxHighlighter};

    #[test]
    fn every_bundled_grammar_has_queries_and_initializes() {
        let registry = LanguageRegistry::singleton();
        for name in registry.languages() {
            if name == "text" {
                continue;
            }
            let config = registry.language(&name).unwrap();
            assert!(
                !config.highlights.is_empty(),
                "{name}: missing highlight query"
            );
            let highlighter = SyntaxHighlighter::new(&name);
            assert_eq!(
                highlighter.language(),
                &config.name,
                "{name}: query failed to compile"
            );
        }
    }

    #[test]
    fn fence_aliases_ignore_case_and_additional_options() {
        for (alias, expected) in [
            ("SQL", "sql"),
            ("MySQL", "sql"),
            ("PG", "sql"),
            ("pgsql", "sql"),
            ("TSX title=Component.tsx", "tsx"),
            ("JSX", "javascript"),
            ("TS", "typescript"),
            ("C#", "csharp"),
            ("C++", "cpp"),
            ("PYI", "python"),
            ("ZSH", "bash"),
            ("JSON-LD", "json"),
            ("Vue", "html"),
            ("Svelte", "svelte"),
            ("Astro", "astro"),
        ] {
            assert_eq!(language(alias), expected);
            assert!(
                LanguageRegistry::singleton().language(expected).is_some(),
                "{alias}"
            );
        }
        assert_eq!(language("Unknown-Language"), "unknown-language");
    }
}
