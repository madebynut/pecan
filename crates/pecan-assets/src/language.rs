//! Small, offline Devicon and Lucide SVG subset shared by reading and document export.

/// Attribution applies to embedded language marks, never to the user's document text.
pub fn license() -> String {
    format!(
        "Devicon v2.18.0 (language marks)\n{}\nLucide (generic code mark)\n{}",
        include_str!("../../../licenses/MIT-Devicon.txt"),
        include_str!("../../../licenses/LUCIDE-Feather-LICENSE.txt")
    )
}

/// A static path and SVG; code cards resolve their mark once at construction.
#[derive(Debug)]
pub struct LanguageIcon {
    pub path: &'static str,
    pub svg: &'static [u8],
    aliases: &'static [&'static str],
}

macro_rules! icon {
    ($name:literal, [$($alias:literal),+]) => {
        LanguageIcon {
            path: concat!("language-icons/", $name, ".svg"),
            svg: include_bytes!(concat!("../../../assets/devicon/", $name, ".svg")),
            aliases: &[$($alias),+],
        }
    };
}

pub static ICONS: &[LanguageIcon] = &[
    icon!("rust", ["rust", "rs"]),
    icon!("javascript", ["javascript", "js", "jsx", "node", "nodejs"]),
    icon!("typescript", ["typescript", "ts", "tsx"]),
    icon!("python", ["python", "py", "python3"]),
    icon!("bash", ["bash", "sh", "shell", "zsh"]),
    icon!("yaml", ["yaml", "yml"]),
    icon!("html5", ["html", "html5"]),
    icon!("css3", ["css", "css3"]),
    icon!("json", ["json", "jsonc", "json-ld", "jsonld"]),
    icon!("c", ["c"]),
    icon!("cplusplus", ["cpp", "c++", "cplusplus", "cxx", "cc"]),
    icon!("csharp", ["csharp", "c#", "cs"]),
    icon!("go", ["go", "golang"]),
    icon!("java", ["java"]),
    icon!("kotlin", ["kotlin", "kt", "kts"]),
    icon!("swift", ["swift"]),
    icon!("php", ["php"]),
    icon!("ruby", ["ruby", "rb"]),
    icon!("markdown", ["markdown", "md"]),
    icon!("xml", ["xml", "xhtml"]),
    icon!("toml", ["toml"]),
    icon!("docker", ["docker", "dockerfile"]),
    icon!("powershell", ["powershell", "ps1", "pwsh"]),
    icon!("dart", ["dart"]),
    icon!("r", ["r"]),
    icon!("scala", ["scala"]),
    icon!("lua", ["lua"]),
    icon!("objectivec", ["objectivec", "objective-c", "objc"]),
    icon!("perl", ["perl", "pl"]),
    icon!(
        "azuresqldatabase",
        ["sql", "azuresql", "azure-sql", "azuresqldatabase"]
    ),
    icon!("mysql", ["mysql"]),
    icon!(
        "postgresql",
        ["postgresql", "postgres", "pg", "pgsql", "psql"]
    ),
    icon!("sqlite", ["sqlite", "sqlite3"]),
    icon!(
        "microsoftsqlserver",
        [
            "mssql",
            "sqlserver",
            "sql-server",
            "tsql",
            "t-sql",
            "microsoftsqlserver"
        ]
    ),
    icon!("oracle", ["oracle", "plsql", "pl/sql"]),
    icon!("mariadb", ["mariadb"]),
    icon!("mongodb", ["mongodb", "mongo"]),
    icon!("redis", ["redis"]),
];

/// Shared by plain text and languages without a dedicated Devicon mark.
pub static GENERIC: LanguageIcon = LanguageIcon {
    path: "language-icons/code-xml.svg",
    svg: include_bytes!("../../../assets/icons/code-xml.svg"),
    aliases: &[],
};

/// Language names and fence aliases are case insensitive; missing marks use CodeXml.
pub fn for_language(language: &str) -> &'static LanguageIcon {
    let language = language.split_whitespace().next().unwrap_or("");
    ICONS
        .iter()
        .find(|icon| {
            icon.aliases
                .iter()
                .any(|alias| language.eq_ignore_ascii_case(alias))
        })
        .unwrap_or(&GENERIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_languages_use_the_generic_code_mark() {
        for language in ["text", "unknown-language", ""] {
            assert_eq!(
                for_language(language).path,
                "language-icons/code-xml.svg",
                "{language:?}"
            );
        }
    }

    #[test]
    fn database_aliases_use_the_requested_database_mark() {
        for (name, database) in [
            ("SQL", "azuresqldatabase"),
            ("azure-sql", "azuresqldatabase"),
            ("MySQL", "mysql"),
            ("postgres", "postgresql"),
            ("PG", "postgresql"),
            ("PgSql", "postgresql"),
            ("psql", "postgresql"),
            ("SQLite3", "sqlite"),
            ("T-SQL", "microsoftsqlserver"),
            ("pl/sql", "oracle"),
            ("MariaDB", "mariadb"),
            ("mongo", "mongodb"),
            ("Redis", "redis"),
        ] {
            assert_eq!(
                for_language(name).path,
                format!("language-icons/{database}.svg"),
                "{name}"
            );
            assert!(
                std::ptr::eq(for_language(name), for_language(database)),
                "{name} shares its canonical resource"
            );
        }
        assert!(!std::ptr::eq(for_language("sql"), for_language("mysql")));
        assert!(!std::ptr::eq(for_language("sql"), &GENERIC));
    }

    #[test]
    fn aliases_are_case_insensitive_and_unknown_languages_share_the_generic_mark() {
        for (alias, canonical) in [
            ("RS", "rust"),
            ("Js", "javascript"),
            ("TSX", "typescript"),
            ("JSONC", "json"),
            ("json-ld", "json"),
            ("c++", "cpp"),
            ("C#", "csharp"),
            ("YML", "yaml"),
            ("sh", "bash"),
            ("  python linenums", "py"),
        ] {
            assert!(
                std::ptr::eq(for_language(alias), for_language(canonical)),
                "{alias}"
            );
        }
        for language in [
            "",
            "text",
            "unknown",
            "mermaid",
            "../../rust",
            "javascript:evil",
        ] {
            assert!(
                std::ptr::eq(for_language(language), &GENERIC),
                "{language} uses the generic icon"
            );
        }
    }
}
