//! Compact, consistent controls for the image and diagram viewers.
use gpui_kit::{
    Div, ElementId, FontFallbacks, ParentElement, StyleRefinement, Styled, TextStyleRefinement,
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
        .small()
        .ghost()
        .w(px(28.))
        .h(px(28.))
        .rounded(px(4.))
        .cursor_pointer()
        .child(Icon::new(icon).with_size(Size::Size(px(ICON_SIZE))))
        .accessibility_label(tooltip)
        .tooltip(tooltip)
}

/// Both navigation panes use one full-width, vertically centered row geometry.
pub(crate) fn sidebar_row() -> Div {
    h_flex().w_full().h(px(32.)).items_center().cursor_pointer()
}
