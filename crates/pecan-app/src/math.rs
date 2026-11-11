use std::{collections::HashMap, sync::Arc};

use gpui_kit::{
    App, ClipboardItem, Image, ImageFormat, IntoElement, Window,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        text::{
            InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode,
            MarkdownParseContext, MarkdownPlugin, markdown_ast,
        },
    },
    div, img,
    prelude::*,
    px,
};
use pecan_math::{MathFont, MathStyle, Presentation};
use pulldown_cmark::{Event, Options, Parser};

use crate::preferences::PreferencesData;

#[derive(Clone)]
pub(crate) struct MathQueue {
    pub(crate) rendering: pecan_math::RenderQueue,
}
impl MathQueue {
    pub(crate) fn new() -> Self {
        Self {
            rendering: pecan_math::RenderQueue::new(),
        }
    }
    /// Called only from a bounded background job; plugin parsing itself only looks up assets.
    pub(crate) fn prepare(
        &self,
        source: &str,
        prefs: &PreferencesData,
        dark: bool,
    ) -> MarkdownExtensions {
        let assets = self.prepare_assets(source, prefs, dark);
        let assets = Arc::new(assets);
        MarkdownExtensions::default()
            .plugin(MathPlugin {
                assets: assets.clone(),
                style: MathStyle::Inline,
            })
            .plugin(MathPlugin {
                assets,
                style: MathStyle::Display,
            })
    }
    fn prepare_assets(
        &self,
        source: &str,
        prefs: &PreferencesData,
        dark: bool,
    ) -> HashMap<(String, MathStyle), Asset> {
        let mut assets = HashMap::new();
        let mut bytes = 0;
        let mut count = 0;
        for event in Parser::new_ext(source, Options::ENABLE_MATH) {
            let (source, style) = match event {
                Event::InlineMath(source) => (source, MathStyle::Inline),
                Event::DisplayMath(source) => (source, MathStyle::Display),
                _ => continue,
            };
            let key = formula_key(&source, style);
            if assets.contains_key(&key) {
                continue;
            }
            count += 1;
            let asset = if count > 128 || bytes >= 2 * 1024 * 1024 {
                Asset::Error("本阅读块公式超过预览预算，请拆分段落".into())
            } else {
                match self.rendering.renderer.render(
                    &source,
                    Presentation {
                        font: if prefs.math_font == "modern" {
                            MathFont::LatinModern
                        } else {
                            MathFont::StixTwo
                        },
                        style,
                        font_size: prefs.math_size.round() as u8,
                        color: if dark { 0xeeeeee } else { 0x242424 },
                    },
                ) {
                    Ok(formula) if bytes + formula.raster_bytes() > 2 * 1024 * 1024 => {
                        Asset::Error("本阅读块公式像素超过预览预算，请拆分段落".into())
                    }
                    Ok(formula) => {
                        bytes += formula.raster_bytes();
                        let (width, height) = formula.dimensions();
                        Asset::Image {
                            image: Arc::new(Image::from_bytes(
                                ImageFormat::Png,
                                formula.png().to_vec(),
                            )),
                            width,
                            height,
                            baseline: formula.baseline(),
                            speech: formula.speech().into(),
                        }
                    }
                    Err(error) => Asset::Error(error.to_string()),
                }
            };
            assets.insert(key, asset);
        }
        assets
    }
}

#[derive(Clone)]
enum Asset {
    Image {
        image: Arc<Image>,
        width: f32,
        height: f32,
        baseline: f32,
        speech: String,
    },
    Error(String),
}

struct MathPlugin {
    assets: Arc<HashMap<(String, MathStyle), Asset>>,
    style: MathStyle,
}

impl MarkdownPlugin for MathPlugin {
    fn name(&self) -> &str {
        if self.is_block() {
            "pecan-display-math"
        } else {
            "pecan-inline-math"
        }
    }
    fn is_block(&self) -> bool {
        self.style == MathStyle::Display
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        context: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let source = match (node, self.style) {
            (markdown_ast::Node::InlineMath(node), MathStyle::Inline) => &node.value,
            (markdown_ast::Node::Math(node), MathStyle::Display) => &node.value,
            _ => return None,
        };
        let asset = self
            .assets
            .get(&formula_key(source, self.style))
            .cloned()
            .unwrap_or_else(|| {
                Asset::Error("此公式语法未被索引，请使用 $…$ 或独占行的 $$…$$".into())
            });
        let speech = match &asset {
            Asset::Image { speech, .. } => speech.clone(),
            Asset::Error(error) => error.clone(),
        };
        Some(
            MarkdownNode::new(self.name(), asset)
                .text(source.clone())
                .markdown(context.node_source(node).unwrap_or(source).to_string())
                .accessibility_label(speech),
        )
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let source = node.as_text().to_string();
        let body = asset_element(node, window, cx);
        div()
            .w_full()
            .py_3()
            .child(h_flex().w_full().justify_center().child(body))
            .child(
                h_flex().justify_end().child(
                    Button::new("copy-latex")
                        .small()
                        .ghost()
                        .label("复制 LaTeX")
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(source.clone()))
                        }),
                ),
            )
    }
    fn render_inline(
        &self,
        node: &MarkdownNode,
        _: &InlineRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        let baseline = match node.data::<Asset>() {
            Some(Asset::Image { baseline, .. }) => *baseline,
            _ => 16.,
        };
        Some(InlineElement::new(asset_element(node, window, cx)).with_baseline(px(baseline)))
    }
}

fn asset_element(node: &MarkdownNode, window: &mut Window, cx: &mut App) -> gpui_kit::AnyElement {
    match node.data::<Asset>() {
        Some(Asset::Image {
            image,
            width,
            height,
            ..
        }) => div()
            .w(px(*width))
            .h(px(*height))
            .when_some(crate::raster::visible(image, window, cx), |this, pixels| {
                this.child(img(pixels).size_full())
            })
            .into_any_element(),
        Some(Asset::Error(error)) => div()
            .text_size(px(13.))
            .child(format!("{} · {error}", node.as_text()))
            .into_any_element(),
        None => div().child(node.as_text().to_string()).into_any_element(),
    }
}

// pulldown-cmark retains a display fence's trailing newline; markdown-rs omits it.
// TeX ignores outer whitespace, while the node retains original text for copying.
fn formula_key(source: &str, style: MathStyle) -> (String, MathStyle) {
    (source.trim().to_string(), style)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_equations_and_chemistry_match_prepared_assets() {
        let source = "$$\n x = \\frac{1}{2} \n$$\n\n$$\n\\ce{N2 + 3H2 <=> 2NH3}\n$$\n\n$$\n\\chemfig{H_3C-*6(=-=-=-)}\n$$\n";
        let assets = MathQueue::new().prepare_assets(source, &PreferencesData::default(), false);
        for source in [
            r"x = \frac{1}{2}",
            r"\ce{N2 + 3H2 <=> 2NH3}",
            r"\chemfig{H_3C-*6(=-=-=-)}",
        ] {
            assert!(
                matches!(
                    assets.get(&formula_key(source, MathStyle::Display)),
                    Some(Asset::Image { .. })
                ),
                "display AST must find a rendered image for {source}"
            );
        }
        assert_eq!(
            formula_key("\n x^2 \n", MathStyle::Display),
            formula_key("x^2", MathStyle::Display)
        );
    }
}
