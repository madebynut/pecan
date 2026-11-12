//! Resolve portable font preferences once, using GPUI's installed platform defaults.
use gpui_kit::{App, FontFallbacks, SharedString, component::ActiveTheme};
use pecan_mermaid::SYSTEM_MONO_FONT;
use std::sync::OnceLock;

static MONO: OnceLock<SharedString> = OnceLock::new();

pub(crate) fn install(cx: &App) {
    MONO.get_or_init(|| cx.theme().mono_font_family.clone());
}

/// Family GPUI should load for Latin and monospace text.
///
/// On Linux, `.SystemUIFont` is IBM Plex Sans, which most desktops do not ship.
/// Naming an installed family avoids that missing-font lookup. macOS and Windows
/// keep the virtual UI name so the platform cascade still covers the system font.
pub(crate) fn family(name: &str) -> &str {
    if cfg!(target_os = "linux") {
        pecan_mermaid::resolve_installed(name, false)
    } else if name == SYSTEM_MONO_FONT {
        MONO.get_or_init(|| gpui_kit::base::TypographyTokens::default().mono)
            .as_ref()
    } else {
        name
    }
}

/// Family used when the primary face has no glyph.
///
/// Linux cannot use the virtual `.SystemUIFont` here: GPUI drops the fallback
/// when IBM Plex Sans is missing, and Chinese then never reaches a CJK face.
/// macOS and Windows keep the virtual name so emoji and rare scripts stay on
/// the platform cascade.
pub(crate) fn cjk(name: &str) -> &str {
    if cfg!(target_os = "linux") {
        pecan_mermaid::resolve_installed(name, true)
    } else {
        name
    }
}

pub(crate) fn text_fallbacks(name: &str) -> FontFallbacks {
    FontFallbacks::from_fonts(vec![cjk(name).to_owned()])
}

#[cfg(test)]
mod tests {
    use super::*;
    use pecan_mermaid::{SYSTEM_MONO_FONT, SYSTEM_UI_FONT};

    #[test]
    fn explicit_font_names_pass_through() {
        assert_eq!(family("Songti SC"), "Songti SC");
        assert_eq!(cjk("Songti SC"), "Songti SC");
    }

    #[test]
    fn system_aliases_follow_the_platform_text_engine() {
        if cfg!(target_os = "linux") {
            let installed = pecan_mermaid::system_fonts().faces().next().is_some();
            for (alias, resolved) in [
                (SYSTEM_MONO_FONT, family(SYSTEM_MONO_FONT)),
                (SYSTEM_UI_FONT, family(SYSTEM_UI_FONT)),
                (SYSTEM_UI_FONT, cjk(SYSTEM_UI_FONT)),
            ] {
                if installed {
                    assert_ne!(resolved, alias);
                    assert!(!resolved.starts_with('.'));
                } else {
                    assert_eq!(resolved, alias);
                }
            }
        } else {
            assert_eq!(family(SYSTEM_UI_FONT), SYSTEM_UI_FONT);
            assert_eq!(cjk(SYSTEM_UI_FONT), SYSTEM_UI_FONT);
            assert_eq!(cjk("Songti SC"), "Songti SC");
            assert_ne!(family(SYSTEM_MONO_FONT), SYSTEM_MONO_FONT);
        }
    }
}
