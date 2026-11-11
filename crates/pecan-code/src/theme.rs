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
