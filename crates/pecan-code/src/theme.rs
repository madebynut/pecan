//! Cached native syntax themes shared with the read-only editor.
use crate::Appearance;
use gpui_kit::component::highlighter::HighlightTheme;
use std::sync::{Arc, OnceLock};
/// Immutable native theme shared between the reader and export highlighters.
/// The process parses each embedded theme once; cloning shares its storage.
pub fn native_theme(appearance: Appearance) -> Arc<HighlightTheme> {
    // JSON-LD uses JSON's grammar while preserving its original fence label and copied source.
    static ALIASES: std::sync::Once = std::sync::Once::new();
    ALIASES.call_once(|| {
        let registry = gpui_kit::component::highlighter::LanguageRegistry::singleton();
        if let Some(json) = registry.language("json") {
            registry.register("json-ld", &json);
            registry.register("jsonld", &json);
        }
    });
    static LIGHT: OnceLock<Arc<HighlightTheme>> = OnceLock::new();
    static DARK: OnceLock<Arc<HighlightTheme>> = OnceLock::new();
    let (cache, source) = if appearance == Appearance::Dark {
        (&DARK, include_str!("../../../assets/themes/dark.json"))
    } else {
        (&LIGHT, include_str!("../../../assets/themes/light.json"))
    };
    cache
        .get_or_init(|| {
            Arc::new(
                serde_json::from_str(source).expect("embedded syntax theme has a valid schema"),
            )
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{
        FontStyle, FontWeight, base::input::Rope, component::highlighter::SyntaxHighlighter,
    };

    #[test]
    fn markdown_styles_cover_markup_and_preserve_fenced_language_injections() {
        let source = concat!(
            "# 标题 Heading\n\n普通正文 plain **加粗 bold** *斜体 italic* ~~删除 strike~~ ",
            "`inline code` [链接](https://example.com)\n\n",
            "> 引用\n\n- [x] 已完成\n\n",
            "[参考]: https://example.org \"说明\"\n\n",
            "| 内容 |\n| --- |\n| **表格** *Cell* |\n\n",
            "**重点 *nested* ~~obsolete~~** 和 `**literal**`\n\n",
            "```rust\nlet name = \"Pecan\";\n```\n",
        );
        for appearance in [Appearance::Light, Appearance::Dark] {
            let theme = native_theme(appearance);
            let mut highlighter = SyntaxHighlighter::new("markdown");
            highlighter.update(None, &Rope::from_str(source), None);
            let styles = highlighter.styles(&(0..source.len()), theme.as_ref());
            let at = |text: &str| {
                let offset = source.find(text).expect("fixture includes token");
                styles
                    .iter()
                    .find(|(range, _)| range.contains(&offset))
                    .map(|(_, style)| *style)
                    .unwrap_or_default()
            };
            let plain = at("普通正文");
            for token in [
                "标题",
                "inline code",
                "链接",
                "https://example.com",
                ">",
                "-",
                "[x]",
                "https://example.org",
            ] {
                assert!(
                    at(token).color.is_some() && at(token).color != plain.color,
                    "{appearance:?}: {token:?} must be visibly highlighted"
                );
            }
            assert_eq!(at("标题").font_weight, Some(FontWeight::BOLD));
            assert_eq!(at("加粗").font_weight, Some(FontWeight::BOLD));
            assert_eq!(at("斜体").font_style, Some(FontStyle::Italic));
            assert!(at("删除").strikethrough.is_some());
            assert_eq!(at("表格").font_weight, Some(FontWeight::BOLD));
            assert_eq!(at("Cell").font_style, Some(FontStyle::Italic));
            assert_eq!(at("nested").font_weight, Some(FontWeight::BOLD));
            assert_eq!(at("nested").font_style, Some(FontStyle::Italic));
            assert!(at("obsolete").strikethrough.is_some());
            assert_eq!(at("literal").font_weight, None);
            assert_ne!(at("let").color, plain.color);
            assert_ne!(at("Pecan").color, at("let").color);
        }
    }
}
