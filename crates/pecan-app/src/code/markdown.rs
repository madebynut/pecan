//! Reuse the native code card for fences inside notes, quotes and lists.
use super::{CodeCard, CodeFormats};
use gpui_kit::{
    App, Entity, IntoElement, SharedString, Window,
    component::text::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, markdown_ast,
    },
    div,
};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_CODE_ID: AtomicUsize = AtomicUsize::new(1);

pub(crate) fn extensions(
    base: MarkdownExtensions,
    formats: Option<Entity<CodeFormats>>,
) -> MarkdownExtensions {
    base.plugin(CodeBlocks { formats })
}
struct CodeBlocks {
    formats: Option<Entity<CodeFormats>>,
}
struct CodeBlock {
    id: usize,
    body: SharedString,
    language: String,
}
impl MarkdownPlugin for CodeBlocks {
    fn is_block(&self) -> bool {
        true
    }
    fn name(&self) -> &str {
        "pecan-code-card"
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Code(code) = node else {
            return None;
        };
        let body: SharedString = code.value.clone().into();
        Some(
            MarkdownNode::new(
                self.name(),
                CodeBlock {
                    id: NEXT_CODE_ID.fetch_add(1, Ordering::Relaxed),
                    body: body.clone(),
                    language: code.lang.clone().unwrap_or_default(),
                },
            )
            .text(body)
            .markdown(cx.node_source(node)?.to_owned()),
        )
    }
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let Some(code) = node.data::<CodeBlock>() else {
            return div().into_any_element();
        };
        // Scoped to the Markdown view and released when this element stops rendering.
        // A parsed node has its own identity: equal fences cannot share fold/Tab state,
        // and reparsing changed source cannot revive an editor with obsolete content.
        window
            .use_keyed_state(("markdown-code", code.id), cx, |window, cx| {
                CodeCard::from_fragment(
                    code.body.clone(),
                    code.language.clone(),
                    self.formats.clone(),
                    window,
                    cx,
                )
            })
            .into_any_element()
    }
}
