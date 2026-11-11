//! The source editor uses the same document column and typography as reading.
use super::{Reader, SaveMode};
use gpui_kit::{
    AnyElement, Context, IntoElement,
    component::{ActiveTheme, Disableable, Icon, Selectable, input::Editor, v_flex},
    div,
    prelude::*,
    px,
};

impl Reader {
    pub(in crate::app) fn source_tools(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(tab) = self.tabs.get(self.active) else {
            return Vec::new();
        };
        let mut tools = vec![
            crate::ui::tool_button(
                "toggle-source",
                // The open chevrons need a little more optical weight than closed icons.
                Icon::new(gpui_kit::assets::IconName::CodeXml).transform(
                    gpui_kit::Transformation::scale(gpui_kit::size(1.125, 1.125)),
                ),
                crate::platform::choose("查看源码／阅读 · ⌘E", "查看源码／阅读 · Ctrl+E"),
            )
            .selected(tab.source_mode())
            .disabled(self.edit_job.is_some())
            .debug_selector(|| "toggle-source".into())
            .on_click(cx.listener(|reader, _, window, cx| reader.toggle_source(window, cx)))
            .into_any_element(),
        ];
        if let Some(edit) = tab
            .edit
            .as_ref()
            .filter(|edit| edit.dirty || edit.save_confirmed())
        {
            let saved = edit.save_confirmed();
            let icon = Icon::new(if saved {
                gpui_kit::assets::IconName::SaveCheck
            } else {
                gpui_kit::assets::IconName::Save
            })
            .text_color(if saved {
                cx.theme().success
            } else {
                cx.theme().foreground
            });
            let tooltip = if saved {
                "已保存"
            } else {
                crate::platform::choose("保存 · ⌘S", "保存 · Ctrl+S")
            };
            tools.push(
                crate::ui::tool_button("source-save", icon, tooltip)
                    .disabled(self.edit_job.is_some())
                    .debug_selector(|| "source-save".into())
                    .when(saved, |button| {
                        button
                            .debug_selector(|| "source-saved".into())
                            .cursor_default()
                    })
                    .when(!saved, |button| {
                        button.on_click(cx.listener(|reader, _, window, cx| {
                            reader.save_active(SaveMode::Existing, window, cx);
                        }))
                    })
                    .into_any_element(),
            );
        }
        tools
    }

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
