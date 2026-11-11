use std::{ops::Range, sync::Arc};

use gpui_kit::{
    ClipboardItem, Context, Entity, IntoElement, Render, Window,
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Editor, EditorState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};
use pecan_document::Document;

pub(crate) struct SourceContent {
    pub(crate) document: Arc<Document>,
    pub(crate) pages: Vec<Range<usize>>,
    pub(crate) copy_range: Range<usize>,
}

pub(crate) struct SourceView {
    document: Arc<Document>,
    pages: Vec<Range<usize>>,
    copy_range: Range<usize>,
    page: usize,
    editor: Entity<EditorState>,
    copied: bool,
}

impl SourceView {
    pub(crate) fn new(content: SourceContent, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let SourceContent {
            document,
            pages,
            copy_range,
        } = content;
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(document.source()[pages[0].clone()].to_string())
        });
        Self {
            document,
            pages,
            copy_range,
            page: 0,
            editor,
            copied: false,
        }
    }
    fn page(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.page = index.min(self.pages.len() - 1);
        self.editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(self.document.source()[self.pages[self.page].clone()].to_string())
        });
        cx.notify();
    }
    pub(crate) fn reveal(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let index = self
            .pages
            .partition_point(|page| page.end <= range.start)
            .min(self.pages.len() - 1);
        self.page(index, window, cx);
        let base = self.pages[index].start;
        self.editor.update(cx, |editor, cx| {
            editor.set_selected_range(
                range.start.saturating_sub(base)
                    ..range.end.min(self.pages[index].end).saturating_sub(base),
                cx,
            )
        });
    }
}

impl Render for SourceView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_xs().child("此段内容较大，按页查看完整源码"))
                    .child(div().flex_1())
                    .child(div().text_xs().child(format!(
                        "{} / {}",
                        self.page + 1,
                        self.pages.len()
                    )))
                    .child(
                        Button::new("source-previous")
                            .small()
                            .ghost()
                            .label("上一页")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.page(view.page.saturating_sub(1), window, cx)
                            })),
                    )
                    .child(
                        Button::new("source-next")
                            .small()
                            .ghost()
                            .label("下一页")
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.page(view.page + 1, window, cx)
                            })),
                    )
                    .child(
                        Button::new("source-copy")
                            .small()
                            .ghost()
                            .label(if self.copied {
                                "已复制"
                            } else {
                                "复制完整源码"
                            })
                            .on_click(cx.listener(|view, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    view.document.source()[view.copy_range.clone()].to_string(),
                                ));
                                view.copied = true;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .h(px(440.))
                    .child(Editor::new(&self.editor).readonly(true).h_full()),
            )
    }
}
