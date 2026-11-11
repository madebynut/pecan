//! Compact, consistent controls for the image and diagram viewers.
use gpui_kit::{
    Div, ElementId, FontFallbacks, StyleRefinement, Styled, TextStyleRefinement,
    component::{
        Icon, IconName, Sizable, Size,
        button::{Button, ButtonVariants},
        h_flex,
    },
    px,
};

/// Icons have a fixed size independent of text or button sizing.
pub(crate) const ICON_SIZE: f32 = 16.;
/// Status text remains quiet even when the main UI font size changes.
pub(crate) const STATUS_SIZE: f32 = 11.;

pub(crate) fn icon(name: IconName) -> Icon {
    Icon::new(name).with_size(Size::Size(px(ICON_SIZE)))
}

pub(crate) fn font_style(prefs: &crate::preferences::PreferencesData) -> StyleRefinement {
    StyleRefinement {
        text: TextStyleRefinement {
            font_family: Some(crate::fonts::family(&prefs.ui_font).to_owned().into()),
            font_fallbacks: Some(FontFallbacks::from_fonts(vec![
                prefs.ui_chinese_font.clone(),
            ])),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// A 16px icon inside a comfortable 28px hit target.
pub(crate) fn tool_button(
    id: impl Into<ElementId>,
    icon: impl Into<Icon>,
    tooltip: &'static str,
) -> Button {
    Button::new(id)
        .with_size(Size::Medium)
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .rounded(px(4.))
        .cursor_pointer()
        // Native icon mode leaves the full hit target available to the glyph.
        .icon(Icon::new(icon))
        .accessibility_label(tooltip)
        .tooltip(tooltip)
}

/// Both navigation panes use one full-width, vertically centered row geometry.
pub(crate) fn sidebar_row() -> Div {
    h_flex().w_full().h(px(32.)).items_center().cursor_pointer()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use gpui_kit::{
        AppContext, Context, HeadlessAppContext, IntoElement, ParentElement, Render, Window, div,
        rgb, size,
    };
    use std::sync::Arc;

    #[test]
    #[ignore = "Requires a Metal graphical session"]
    fn tool_button_icons_match_unclipped_native_icons() {
        struct Fixture;
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                let mut root = div().relative().size_full().bg(rgb(0xffffff));
                for (index, name) in [
                    gpui_kit::assets::IconName::CodeXml,
                    gpui_kit::assets::IconName::Copy,
                    gpui_kit::assets::IconName::SaveCheck,
                ]
                .into_iter()
                .enumerate()
                {
                    let icon = Icon::new(name).text_color(rgb(0xff00ff));
                    let top = px(24. + index as f32 * 40.);
                    root = root
                        .child(
                            tool_button(("test-icon", index), icon.clone(), "test")
                                .absolute()
                                .left(px(24.))
                                .top(top),
                        )
                        .child(
                            div()
                                .absolute()
                                .left(px(80.))
                                .top(top)
                                .size(px(28.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon.with_size(Size::Size(px(ICON_SIZE)))),
                        );
                }
                root
            }
        }
        let mut headless = HeadlessAppContext::with_platform(
            Arc::new(gpui_apple::AppleTextSystem::new()),
            Arc::new(crate::assets::ReaderAssets),
            gpui_platform::current_headless_renderer,
        );
        headless.update(gpui_kit::init);
        let window = headless
            .open_window(size(px(140.), px(160.)), |_, cx| cx.new(|_| Fixture))
            .unwrap();
        headless
            .update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        let image = headless.capture_screenshot(window.into()).unwrap();
        let scale = image.width() / 140;
        let mask = |left, top| {
            image::imageops::crop_imm(&image, left * scale, top * scale, 28 * scale, 28 * scale)
                .to_image()
                .pixels()
                .map(|pixel| pixel[0] > 200 && pixel[1] < 128 && pixel[2] > 200)
                .collect::<Vec<_>>()
        };
        if let Some(path) = std::env::var_os("PECAN_METAL_OUTPUT") {
            image
                .save(std::path::PathBuf::from(path).join("tool-icons.png"))
                .unwrap();
        }
        for top in [24, 64, 104] {
            let reference = mask(80, top);
            assert!(reference.iter().filter(|pixel| **pixel).count() > 30);
            assert!(
                mask(24, top) == reference,
                "button at y={top} must preserve the complete standalone icon"
            );
        }
    }
}
