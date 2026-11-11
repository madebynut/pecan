//! Reading colors and typography shared by native Markdown and code cards.
use std::sync::Arc;

use gpui_kit::{
    App, Background, FontFallbacks, FontWeight, Global, Hsla, Overflow, StyleRefinement, Styled,
    TextStyleRefinement, base::TextViewStyle, component::ActiveTheme, px, relative, rems, rgb,
};

use crate::preferences::{Preferences, PreferencesData};

/// Code card fills shared with the portable exporters, which use light paper.
pub(crate) use pecan_assets::LIGHT_CODE_BODY;

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReadingPalette {
    pub(crate) canvas: Background,
    pub(crate) paper: Background,
    pub(crate) ink: Hsla,
    pub(crate) muted: Hsla,
    pub(crate) accent: Hsla,
    pub(crate) wash: Hsla,
    pub(crate) border: Hsla,
    pub(crate) code: Hsla,
    /// Warm note fill, distinct from the gray quote wash.
    pub(crate) highlight: Hsla,
    pub(crate) highlight_edge: Hsla,
    pub(crate) highlight_mark: Hsla,
}

impl ReadingPalette {
    pub(crate) fn for_app(cx: &App) -> Self {
        let theme = cx.theme();
        Self {
            canvas: theme.muted.into(),
            paper: theme.background.into(),
            ink: theme.foreground,
            muted: theme.muted_foreground,
            accent: theme.primary,
            wash: theme.muted,
            border: theme.border,
            code: rgb(if theme.is_dark() {
                0x222222
            } else {
                LIGHT_CODE_BODY
            })
            .into(),
            highlight: rgb(if theme.is_dark() { 0x3a3226 } else { 0xfff6e8 }).into(),
            highlight_edge: rgb(if theme.is_dark() { 0x5c4b34 } else { 0xf0e0c0 }).into(),
            highlight_mark: rgb(if theme.is_dark() { 0xf0c060 } else { 0xc4841a }).into(),
        }
    }
}

/// Styles derived only from preferences and appearance, prepared once per change.
struct StyleCache {
    // Holding the allocation keeps its address unique; `Arc::make_mut` then
    // gives every preference change a new address.
    prefs: Arc<PreferencesData>,
    dark: bool,
    styles: Styles,
}
impl Global for StyleCache {}

/// Prepared reading styles; clones share font strings and the heading closure, so
/// `TextViewStyle` equality takes its pointer fast path between frames.
#[derive(Clone)]
pub(crate) struct Styles {
    pub(crate) reading: TextViewStyle,
    /// Natural paragraph spacing for compact notes and their rich content.
    pub(crate) note: TextViewStyle,
    /// Interface font family and CJK fallback.
    pub(crate) interface: StyleRefinement,
    /// Body font family, CJK fallback, size and line height.
    pub(crate) body: StyleRefinement,
}

pub(crate) fn styles(cx: &mut App) -> Styles {
    cached(cx).clone()
}

pub(crate) fn interface_style(cx: &mut App) -> StyleRefinement {
    cached(cx).interface.clone()
}

fn cached(cx: &mut App) -> &Styles {
    let prefs = Preferences::shared(cx).read(cx).data.clone();
    let dark = cx.theme().is_dark();
    let fresh = cx
        .try_global::<StyleCache>()
        .is_some_and(|cache| cache.dark == dark && Arc::ptr_eq(&cache.prefs, &prefs));
    if !fresh {
        let styles = build(&prefs, cx);
        cx.set_global(StyleCache {
            prefs,
            dark,
            styles,
        });
    }
    &cx.global::<StyleCache>().styles
}

fn build(prefs: &PreferencesData, cx: &App) -> Styles {
    let size = px(prefs.font_size as f32);
    let note = natural_reading_style(prefs, ReadingPalette::for_app(cx), cx);
    Styles {
        reading: note.clone().with_paragraph_justification(),
        note,
        interface: crate::ui::font_style(prefs),
        body: StyleRefinement {
            text: TextStyleRefinement {
                font_family: Some(crate::fonts::family(&prefs.english_font).to_owned().into()),
                font_fallbacks: Some(FontFallbacks::from_fonts(vec![prefs.chinese_font.clone()])),
                ..Default::default()
            },
            ..Default::default()
        }
        .text_size(size)
        .line_height(size * prefs.line_height as f32),
    }
}

#[cfg(test)]
pub(crate) fn reading_style(
    prefs: &PreferencesData,
    palette: ReadingPalette,
    cx: &App,
) -> TextViewStyle {
    natural_reading_style(prefs, palette, cx).with_paragraph_justification()
}

fn natural_reading_style(
    prefs: &PreferencesData,
    palette: ReadingPalette,
    cx: &App,
) -> TextViewStyle {
    let size = px(prefs.font_size as f32);
    let mut table = StyleRefinement::default()
        .rounded(px(3.))
        .border_color(palette.border);
    table.overflow.x = Some(Overflow::Scroll);
    TextViewStyle::default()
        .with_foreground(palette.ink)
        .with_muted_foreground(palette.muted)
        .with_link(palette.accent)
        .with_selection(cx.theme().selection)
        .with_border(palette.border)
        .with_code_background(palette.code)
        .with_dark(cx.theme().is_dark())
        .with_paragraph_gap(rems(0.85))
        .with_heading(move |level| {
            let scale = match level {
                1 => 2.,
                2 => 1.75,
                3 => 1.5,
                4 => 1.3125,
                5 => 1.1875,
                _ => 1.0625,
            };
            // TextView owns spacing between blocks, including consecutive headings.
            StyleRefinement::default()
                .text_size(size * scale)
                .line_height(relative(1.35))
                .font_weight(if level == 1 {
                    FontWeight::BOLD
                } else {
                    FontWeight::SEMIBOLD
                })
        })
        .with_blockquote_style(
            StyleRefinement::default()
                .border_l(px(2.))
                .border_color(palette.border)
                .rounded_r(px(4.))
                .bg(palette.wash)
                .text_color(palette.muted)
                .px_4()
                .py_3(),
        )
        .with_list_style(StyleRefinement::default())
        .with_inline_code_style({
            let mut code = StyleRefinement::default()
                .font_family(crate::fonts::family(&prefs.code_font).to_owned())
                .text_size(size * 0.86)
                .text_color(palette.ink)
                .bg(palette.wash)
                .border_1()
                .border_color(palette.border)
                .rounded(px(3.))
                .self_center()
                // Same inset on both sides. A 1px border sits inside this padding.
                .pl(px(5.))
                .pr(px(5.))
                .py(px(1.));
            // Keep the configured Chinese face for glyphs missing in the code font.
            code.text.font_fallbacks =
                Some(FontFallbacks::from_fonts(vec![prefs.chinese_font.clone()]));
            code
        })
        .with_table(table)
        .with_table_head(
            StyleRefinement::default()
                .bg(palette.wash)
                .text_color(palette.ink)
                .font_weight(FontWeight::SEMIBOLD),
        )
        .with_table_cell(
            StyleRefinement::default()
                .flex()
                .flex_col()
                .justify_center()
                .whitespace_nowrap()
                .px_4()
                .py_3()
                .border_color(palette.border),
        )
        .with_code_block(code_style(prefs, palette))
}

fn code_style(prefs: &PreferencesData, palette: ReadingPalette) -> StyleRefinement {
    let size = px(prefs.code_size as f32);
    let mut style = StyleRefinement::default()
        .font_family(crate::fonts::family(&prefs.code_font).to_owned())
        .text_size(size)
        .line_height(size * 1.65)
        .text_color(palette.ink)
        .bg(palette.code)
        .border_color(palette.border)
        .rounded(px(3.));
    style.text.white_space = Some(gpui_kit::WhiteSpace::Nowrap);
    style.overflow.x = Some(Overflow::Scroll);
    style.overflow.y = Some(Overflow::Visible);
    style.pt(px(44.)).pb(px(12.))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui_kit::test]
    fn rich_code_emphasis_keeps_selection_and_literal_fences(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::base::{TextView, TextViewState};
        use gpui_kit::component::text::MarkdownExtensions;
        use gpui_kit::{
            AppContext, Context, Entity, IntoElement, ParentElement, Render, Window, div,
        };
        struct Fixture {
            text: Entity<TextViewState>,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().w(px(640.)).child(
                    TextView::new(&self.text)
                        .style(styles(cx).note)
                        .markdown_extensions(
                            crate::notes::extensions(MarkdownExtensions::default()),
                        )
                        .selectable(true),
                )
            }
        }
        cx.update(gpui_kit::init);
        for source in [
            "正文 `**加粗** *italic* ~~旧版~~` 结尾",
            "# 标题 `**加粗** *italic* ~~旧版~~` 结尾",
            "> 引用 `**加粗** *italic* ~~旧版~~` 结尾",
            "- 列表 `**加粗** *italic* ~~旧版~~` 结尾",
            "| 标题 | 内容 |\n| --- | --- |\n| 表格 | `**加粗** *italic* ~~旧版~~` |",
        ] {
            let (root, visual) = cx.add_window_view(|_, cx| Fixture {
                text: cx.new(|cx| TextViewState::markdown(source, cx)),
            });
            for _ in 0..3 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            visual.update(|_, cx| {
                root.read(cx).text.clone().update(cx, |text, cx| {
                    let rendered = text.rendered_text();
                    assert!(
                        rendered.as_str().contains("加粗 italic 旧版"),
                        "emphasis must render: {source:?}: {:?}",
                        rendered.as_str()
                    );
                    assert!(
                        !rendered.as_str().contains("**"),
                        "delimiters are not visible"
                    );
                    for word in ["加粗", "italic", "旧版"] {
                        let start = source.find(word).expect("fixture word");
                        let range = rendered
                            .range_for_source(start..start + word.len())
                            .expect("source mapping remains selectable");
                        assert_eq!(&rendered.as_str()[range], word, "mapping for {word}");
                    }
                    text.select_all(cx);
                    assert!(
                        text.selected_text().contains("加粗 italic 旧版"),
                        "copy rendered text"
                    );
                })
            });
        }
    }

    #[gpui_kit::test]
    fn prepared_styles_are_reused_until_preferences_change(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let first = styles(cx);
            let cached = Arc::as_ptr(&cx.global::<StyleCache>().prefs);
            assert!(first.reading == styles(cx).reading);
            assert_eq!(Arc::as_ptr(&cx.global::<StyleCache>().prefs), cached);
            Preferences::shared(cx).update(cx, |prefs, cx| {
                prefs.change(|data| data.font_size = 22., cx)
            });
            let changed = styles(cx);
            assert_ne!(Arc::as_ptr(&cx.global::<StyleCache>().prefs), cached);
            assert!(
                changed.reading != first.reading,
                "heading sizes follow the body"
            );
            assert_eq!(changed.body.text.font_size, Some(px(22.).into()));
        });
    }

    #[gpui_kit::test]
    fn justified_paragraphs_keep_final_lines_and_copy_source(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::base::{TextView, TextViewState};
        use gpui_kit::component::text::RangeHighlight;
        use gpui_kit::{
            AppContext, Context, Entity, IntoElement, ParentElement, Render, Window, div,
        };
        struct Fixture {
            text: Entity<TextViewState>,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(420.))
                    .text_size(px(16.))
                    .line_height(px(28.8))
                    .child(
                        TextView::new(&self.text)
                            .selectable(true)
                            .style(reading_style(
                                &PreferencesData::default(),
                                ReadingPalette::for_app(cx),
                                cx,
                            )),
                    )
            }
        }
        cx.update(gpui_kit::init);
        for code in ["", " `model setting` "] {
            let prose = format!("这是测试正文两端对齐和选择复制的段落 mixed English words{code}，文字排版需要保持清晰与稳定。").repeat(5);
            let source = format!("{prose}  \n短行\n");
            let (root, visual) = cx.add_window_view(|_, cx| Fixture {
                text: cx.new(|cx| TextViewState::markdown(&source, cx)),
            });
            visual.update(|_, cx| {
                root.read(cx).text.clone().update(cx, |text, cx| {
                    text.set_range_highlights(
                        [RangeHighlight::new(
                            0..text.rendered_text().len(),
                            rgb(0xff00ff),
                        )],
                        cx,
                    )
                    .unwrap();
                });
            });
            for _ in 0..4 {
                visual.run_until_parked();
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            visual.update(|window, cx| {
                let scale = window.scale_factor();
                let rows = window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| quad.background.as_solid() == Some(rgb(0xff00ff).into()))
                    .collect::<Vec<_>>();
                assert!(
                    rows.len() >= 5,
                    "fixture must wrap into many lines: {}",
                    rows.len()
                );
                let first = rows[0];
                let last = rows.last().unwrap();
                assert!(
                    first.bounds.size.width.0 > 410. * scale,
                    "soft-wrapped prose fills the column: {:?}",
                    first.bounds
                );
                assert!(
                    last.bounds.size.width.0 < 120. * scale,
                    "hard-break final line keeps natural width: {:?}",
                    last.bounds
                );
                let text = root.read(cx).text.clone();
                text.update(cx, |text, cx| {
                    text.select_all(cx);
                    let selected = text.selected_text();
                    assert!(selected.contains("mixed English words"));
                    assert!(selected.contains("短行"));
                    assert_eq!(
                        selected.matches("这是测试").count(),
                        5,
                        "justification must not duplicate or drop source"
                    );
                });
            });
        }
    }

    #[gpui_kit::test]
    fn inline_code_centers_in_body_and_all_heading_lines(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::base::{TextView, TextViewState};
        use gpui_kit::component::text::RangeHighlight;
        use gpui_kit::{
            AppContext, Context, Entity, IntoElement, ParentElement, Render, Window, div,
        };

        struct Preview {
            text: Entity<TextViewState>,
            prefs: PreferencesData,
        }
        impl Render for Preview {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .font_family(crate::fonts::family(&self.prefs.english_font).to_owned())
                    .text_size(px(self.prefs.font_size as f32))
                    .line_height(relative(self.prefs.line_height as f32))
                    .child(
                        TextView::new(&self.text)
                            .style(reading_style(&self.prefs, ReadingPalette::for_app(cx), cx))
                            .selectable(true),
                    )
            }
        }
        cx.update(gpui_kit::init);
        for body in [16., 22.] {
            for level in 0_u8..=6 {
                let source = format!(
                    "{}远控 `**model setting**` 配置",
                    if level == 0 {
                        String::new()
                    } else {
                        format!("{} ", "#".repeat(usize::from(level)))
                    }
                );
                let prefs = PreferencesData {
                    font_size: body,
                    ..Default::default()
                };
                let (root, visual) = cx.add_window_view(|_, cx| Preview {
                    text: cx.new(|cx| TextViewState::markdown(&source, cx)),
                    prefs,
                });
                visual.update(|window, cx| {
                    let text_entity = root.read(cx).text.clone();
                    text_entity.update(cx, |text, cx| {
                        text.set_range_highlights(
                            [RangeHighlight::new(0.."远控".len(), rgb(0xff00ff))],
                            cx,
                        )
                        .unwrap();
                    });
                    window.draw(cx).clear(cx);
                    window.draw(cx).clear(cx);
                    let quads = window.painted_quads();
                    let code = quads
                        .iter()
                        .find(|quad| {
                            quad.border_widths.left.0 > 0. && quad.bounds.size.height.0 < 50.
                        })
                        .expect("code capsule is painted");
                    let prose = quads
                        .iter()
                        .find(|quad| quad.background.as_solid() == Some(rgb(0xff00ff).into()))
                        .expect("prose reference is painted");
                    let delta = (code.bounds.center().y.0 - prose.bounds.center().y.0).abs();
                    assert!(
                        delta <= window.scale_factor(),
                        "body {body}, H{level}: capsule must center on the text line, delta {delta}"
                    );
                    text_entity.update(cx, |text, cx| {
                        text.select_all(cx);
                        assert_eq!(
                            text.selected_text().trim(),
                            "远控 **model setting** 配置",
                            "inline code remains native selectable text"
                        );
                    });
                });
            }
        }
    }

    #[gpui_kit::test]
    fn consecutive_headings_share_compact_native_spacing(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::base::{TextView, TextViewState};
        use gpui_kit::component::text::RangeHighlight;
        use gpui_kit::{
            AppContext, Context, Entity, IntoElement, ParentElement, Render, Window, div,
        };
        struct Fixture {
            text: Entity<TextViewState>,
            prefs: PreferencesData,
        }
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .w(px(900.))
                    .text_size(px(self.prefs.font_size as f32))
                    .child(TextView::new(&self.text).style(reading_style(
                        &self.prefs,
                        ReadingPalette::for_app(cx),
                        cx,
                    )))
            }
        }
        cx.update(gpui_kit::init);
        for body in [16., 22.] {
            let source = "# 设备定位 SDK\n\n## 背景与目标\n\n### 背景\n\n#### 四级\n\n##### 五级\n\n###### 六级\n\n正文";
            let (_, visual) = cx.add_window_view(|_, cx| Fixture {
                text: cx.new(|cx| {
                    let mut text = TextViewState::markdown(source, cx);
                    text.set_range_highlights(
                        [RangeHighlight::new(
                            0..text.rendered_text().len(),
                            rgb(0xff00ff),
                        )],
                        cx,
                    )
                    .unwrap();
                    text
                }),
                prefs: PreferencesData {
                    font_size: body,
                    ..Default::default()
                },
            });
            for _ in 0..4 {
                visual.run_until_parked();
                visual.update(|window, cx| window.draw(cx).clear(cx));
            }
            visual.update(|window, _| {
                let rows = window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| quad.background.as_solid() == Some(rgb(0xff00ff).into()))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), 7, "six headings followed by one paragraph");
                for pair in rows.windows(2) {
                    let gap = (pair[1].bounds.top().0 - pair[0].bounds.bottom().0)
                        / window.scale_factor();
                    assert!(
                        (0. ..=body as f32 * 1.25).contains(&gap),
                        "body {body}: adjacent heading lines must stay together, gap {gap}"
                    );
                }
            });
        }
    }

    #[gpui_kit::test]
    fn all_heading_levels_have_distinct_sizes_that_follow_body_settings(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            for body in [16., 22.] {
                let prefs = PreferencesData {
                    font_size: body,
                    ..Default::default()
                };
                let style = reading_style(&prefs, ReadingPalette::for_app(cx), cx);
                let sizes = (1..=6)
                    .map(|level| {
                        style
                            .heading(level)
                            .text
                            .font_size
                            .unwrap()
                            .to_pixels(px(16.))
                    })
                    .collect::<Vec<_>>();
                assert!(
                    sizes.windows(2).all(|pair| pair[0] > pair[1]),
                    "six heading sizes must descend independently: {sizes:?}"
                );
                assert!(sizes[5] > px(body as f32), "even H6 remains a heading");
                assert_eq!(
                    sizes[0],
                    px(body as f32 * 2.),
                    "heading sizes follow body preferences"
                );
            }
        });
    }
}
