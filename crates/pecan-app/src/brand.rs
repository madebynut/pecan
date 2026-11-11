//! Application identity and the shared neutral component palette.
use std::sync::{Arc, OnceLock};

use gpui_kit::component::Theme;
use gpui_kit::{App, Image, ImageFormat, RenderImage, px, rgb};

/// The small bundled icon is decoded once so branding appears in the first frame.
pub(crate) fn icon(cx: &App) -> Arc<RenderImage> {
    static ICON: OnceLock<Arc<RenderImage>> = OnceLock::new();
    ICON.get_or_init(|| {
        Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../../assets/brand/pecan-icon-256.png").to_vec(),
        )
        .to_image_data(cx.svg_renderer())
        .expect("bundled Pecan PNG is valid")
    })
    .clone()
}

/// Give AppKit the same multi-resolution icon used by the application bundle.
#[cfg(target_os = "macos")]
pub(crate) fn install_native_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let mtm = MainThreadMarker::new().expect("application initialization runs on the main thread");
    let data = NSData::with_bytes(include_bytes!("../../../assets/brand/pecan.icns"));
    if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
        // SAFETY: AppKit receives a non-null, retained NSImage on its main thread.
        unsafe { NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&image)) };
    } else {
        tracing::warn!("unable to decode the bundled application icon");
    }
}

/// Apply once on a theme change; reading and interaction never rebuild this palette.
pub(crate) fn apply(theme: &mut Theme) {
    let dark = theme.is_dark();
    let (background, foreground, sidebar, muted, muted_foreground, border, accent, primary) =
        if dark {
            (
                0x1c1c1c, 0xeeeeee, 0x181818, 0x2a2a2a, 0xa8a8a8, 0x3b3b3b, 0x383838, 0xd6d6d6,
            )
        } else {
            (
                0xffffff, 0x242424, 0xfafafa, 0xf4f4f4, 0x737373, 0xdedede, 0xe8e8e8, 0x333333,
            )
        };
    theme.background = rgb(background).into();
    theme.foreground = rgb(foreground).into();
    theme.sidebar = rgb(sidebar).into();
    theme.sidebar_foreground = theme.foreground;
    theme.muted = rgb(muted).into();
    theme.muted_foreground = rgb(muted_foreground).into();
    theme.border = rgb(border).into();
    theme.input = theme.border;
    theme.accent = rgb(accent).into();
    theme.accent_foreground = theme.foreground;
    theme.primary = rgb(primary).into();
    theme.primary_foreground = if dark {
        rgb(0x1c1c1c).into()
    } else {
        rgb(0xffffff).into()
    };
    theme.primary_hover = theme.primary;
    theme.primary_active = theme.primary;
    theme.button = theme.background;
    theme.button_foreground = theme.foreground;
    theme.button_hover = theme.muted;
    theme.button_active = theme.accent;
    theme.button_primary = theme.primary;
    theme.button_primary_foreground = theme.primary_foreground;
    theme.button_primary_hover = theme.primary;
    theme.button_primary_active = theme.primary;
    theme.secondary = theme.muted;
    theme.secondary_foreground = theme.foreground;
    theme.secondary_hover = theme.accent;
    theme.secondary_active = theme.accent;
    theme.button_secondary = theme.secondary;
    theme.button_secondary_foreground = theme.foreground;
    theme.button_secondary_hover = theme.accent;
    theme.button_secondary_active = theme.accent;
    theme.popover = theme.background;
    theme.popover_foreground = theme.foreground;
    theme.colors.list = theme.background;
    theme.list_hover = theme.muted;
    theme.list_active = theme.accent;
    theme.list_active_border = theme.border;
    theme.list_head = theme.sidebar;
    theme.table = theme.background;
    theme.table_head = theme.sidebar;
    theme.table_head_foreground = theme.foreground;
    theme.table_even = theme.sidebar;
    theme.link = theme.primary;
    theme.link_hover = theme.primary;
    theme.ring = theme.primary;
    theme.caret = theme.primary;
    theme.selection = theme.accent.opacity(0.8);
    theme.sidebar_accent = theme.accent;
    theme.sidebar_accent_foreground = theme.foreground;
    theme.sidebar_border = theme.border;
    theme.sidebar_primary = theme.primary;
    theme.sidebar_primary_foreground = theme.primary_foreground;
    theme.scrollbar_thumb = theme.muted_foreground.opacity(0.35);
    theme.scrollbar_thumb_hover = theme.muted_foreground.opacity(0.6);
    theme.title_bar_border = theme.border;
    theme.title_bar = theme.sidebar;
    theme.radius = px(3.);
    theme.radius_lg = px(4.);
    theme.highlight_theme = pecan_code::native_theme(if dark {
        pecan_code::Appearance::Dark
    } else {
        pecan_code::Appearance::Light
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{base::input::Rope, component::highlighter::SyntaxHighlighter};

    #[test]
    fn code_highlighting_uses_distinct_semantic_colors_in_both_themes() {
        for dark in [false, true] {
            let theme = pecan_code::native_theme(if dark {
                pecan_code::Appearance::Dark
            } else {
                pecan_code::Appearance::Light
            });
            for (language, code) in [
                (
                    "rust",
                    "fn main() { let value = 42; println!(\"Hello\"); } // comment",
                ),
                (
                    "json-ld",
                    r#"{"@type":"Device","version":42,"active":true}"#,
                ),
                (
                    "toml",
                    "# 阅读配置\n[reader]\nname = \"中文设备\"\nenabled = true\nports = [8080, 8081]\nmessage = '''第一行\n第二行'''\n",
                ),
            ] {
                let mut parser = SyntaxHighlighter::new(language);
                assert!(parser.update(None, &Rope::from_str(code), None));
                let styles = parser.styles(&(0..code.len()), theme.as_ref());
                let colors: Vec<_> = styles.iter().filter_map(|(_, style)| style.color).collect();
                assert!(
                    colors
                        .iter()
                        .any(|color| colors.iter().any(|other| color != other)),
                    "{language} tokens need actual semantic colors"
                );
            }
            assert!(
                Arc::ptr_eq(
                    &theme,
                    &pecan_code::native_theme(if dark {
                        pecan_code::Appearance::Dark
                    } else {
                        pecan_code::Appearance::Light
                    })
                ),
                "redraws reuse the same parsed theme"
            );
        }
    }
}
