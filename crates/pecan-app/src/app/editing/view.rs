//! The source editor uses the same document column and typography as reading.
use super::{Reader, SaveMode};
use gpui_kit::{
    AnyElement, Context, IntoElement,
    component::{
        ActiveTheme, Disableable, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        input::Editor,
        v_flex,
    },
    div,
    prelude::*,
    px,
};

impl Reader {
    pub(in crate::app) fn source_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[self.active];
        let edit = tab.edit.as_ref().expect("source mode has an editor");
        let prefs = &self.presented;
        let weak = cx.weak_entity();
        let editor = edit.editor.clone();
        let on_paste = move |clipboard: &gpui_kit::ClipboardItem,
                             _: &mut gpui_kit::Window,
                             cx: &mut gpui_kit::App| {
            let Some(text) = clipboard.text() else {
                return false;
            };
            let state = editor.read(cx);
            let size = state
                .text()
                .len()
                .saturating_sub(state.selected_text().len())
                .saturating_add(text.len());
            let allowed = weak
                .update(cx, |reader, cx| {
                    let other: usize = reader
                        .tabs
                        .iter()
                        .filter(|tab| tab.edit.as_ref().is_none_or(|edit| edit.editor != editor))
                        .map(|tab| tab.source_bytes(cx))
                        .sum();
                    if size > pecan_document::MAX_DOCUMENT_BYTES
                        || size.saturating_add(other) > super::super::MAX_OPEN_BYTES
                    {
                        reader.status =
                            "粘贴超过文档或总源文本预算，请缩小内容或先关闭部分标签".into();
                        cx.notify();
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(false);
            !allowed
        };
        v_flex()
            .debug_selector(|| "source-editor-pane".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .h(px(38.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .flex_shrink_0()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(cx.theme().muted_foreground)
                            .child(if edit.dirty {
                                "Markdown · 未保存"
                            } else {
                                "Markdown"
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("source-reload")
                            .small()
                            .ghost()
                            .label("重新加载")
                            .cursor_pointer()
                            .disabled(edit.busy || tab.path.is_none())
                            .on_click(cx.listener(|reader, _, window, cx| {
                                reader.reload_action(window, cx);
                            })),
                    )
                    .child(
                        Button::new("source-save")
                            .small()
                            .ghost()
                            .label("保存")
                            .disabled(edit.busy)
                            .cursor_pointer()
                            .debug_selector(|| "source-save".into())
                            .on_click(cx.listener(|reader, _, window, cx| {
                                reader.save_active(SaveMode::Existing, window, cx)
                            })),
                    )
                    .child(
                        Button::new("source-read")
                            .small()
                            .ghost()
                            .label("阅读模式")
                            .disabled(edit.busy)
                            .cursor_pointer()
                            .on_click(cx.listener(|reader, _, window, cx| {
                                reader.toggle_source(window, cx)
                            })),
                    ),
            )
            .child(
                div().flex_1().min_h_0().w_full().child(
                    Editor::new(&edit.editor)
                        .h_full()
                        .w_full()
                        .appearance(false)
                        .bordered(false)
                        .readonly(edit.busy)
                        .font_family(crate::fonts::family(&prefs.code_font).to_owned())
                        .text_size(px(prefs.code_size as f32))
                        .on_paste(on_paste)
                        .aria_label("Markdown 源码编辑器"),
                ),
            )
            .into_any_element()
    }
}
