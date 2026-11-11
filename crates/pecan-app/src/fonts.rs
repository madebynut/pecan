//! Resolve portable font preferences once, using GPUI's installed platform defaults.
use gpui_kit::{App, SharedString, component::ActiveTheme};
use pecan_mermaid::SYSTEM_MONO_FONT;
use std::sync::OnceLock;

static MONO: OnceLock<SharedString> = OnceLock::new();

pub(crate) fn install(cx: &App) {
    MONO.get_or_init(|| cx.theme().mono_font_family.clone());
}

pub(crate) fn family(name: &str) -> &str {
    if name == SYSTEM_MONO_FONT {
        MONO.get_or_init(|| gpui_kit::base::TypographyTokens::default().mono)
            .as_ref()
    } else {
        name
    }
}
