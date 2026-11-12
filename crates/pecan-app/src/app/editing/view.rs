//! The source editor uses the same document column and typography as reading.
use super::{Reader, SaveMode};
use gpui_kit::{
    AnyElement, App, Bounds, Context, Element, ElementId, GlobalElementId, InspectorElementId,
    IntoElement, LayoutId, Pixels, Window,
    component::{
        ActiveTheme, Icon, Sizable, Size, StyledExt as _, h_flex, input::Editor, tooltip::Tooltip,
        v_flex,
    },
    div,
    prelude::*,
    px,
};

/// The eye is centered in the first source line; the save glyph is slightly smaller.
const SOURCE_GLYPH: f32 = 20.;
const SAVE_GLYPH: f32 = 16.;

impl Reader {
    /// Source-only corner: the eye returns to reading, and a save glyph tracks the draft.
    pub(in crate::app) fn reading_corner(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(edit) = self
            .tabs
            .get(self.active)
            .filter(|tab| tab.source_mode())
            .and_then(|tab| tab.edit.as_ref())
        else {
            return div().into_any_element();
        };
        let saved = edit.save_confirmed();
        let dirty = edit.dirty;
        let enabled = self.edit_job.is_none();
        // The active-line band starts below the editor's medium padding and is one line tall.
        // Center the glyphs in that band, and keep the same gap inside its right edge.
        let line_height = f32::from(edit.editor.read(cx).line_height().unwrap_or(px(
            self.presented.font_size as f32 * self.presented.line_height as f32,
        )));
        let gap = (line_height - SOURCE_GLYPH) * 0.5;
        let top = (f32::from(Size::Medium.input_py()) + gap).max(0.);
        let right = (f32::from(Size::Medium.input_px()) + gap).max(0.);
        above_editor_text(
            h_flex()
                .id("reading-corner")
                .debug_selector(|| "reading-corner".into())
                .absolute()
                .top(px(top))
                .right(px(right))
                .items_center()
                .gap(px(4.))
                .when(saved, |row| {
                    row.child(corner_glyph(
                        "source-saved",
                        Icon::new(gpui_kit::assets::IconName::SaveCheck)
                            .text_color(cx.theme().success),
                        SAVE_GLYPH,
                    ))
                })
                .when(dirty, |row| {
                    let tooltip = crate::platform::choose("保存 · ⌘S", "保存 · Ctrl+S");
                    row.child(
                        corner_glyph(
                            "source-save",
                            Icon::new(gpui_kit::assets::IconName::Save)
                                .text_color(cx.theme().foreground),
                            SAVE_GLYPH,
                        )
                        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                        .when(enabled, |button| {
                            button.cursor_pointer().on_click(cx.listener(
                                |reader, _, window, cx| {
                                    reader.save_active(SaveMode::Existing, window, cx);
                                },
                            ))
                        })
                        .when(!enabled, |button| button.opacity(0.45)),
                    )
                })
                .child({
                    let tooltip =
                        crate::platform::choose("进入阅读模式 · ⌘E", "进入阅读模式 · Ctrl+E");
                    corner_glyph(
                        "reading-eye",
                        Icon::new(gpui_kit::assets::IconName::Eye)
                            .text_color(cx.theme().foreground),
                        SOURCE_GLYPH,
                    )
                    .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
                    .when(enabled, |button| {
                        button.cursor_pointer().on_click(
                            cx.listener(|reader, _, window, cx| reader.toggle_source(window, cx)),
                        )
                    })
                    .when(!enabled, |button| button.opacity(0.45))
                }),
        )
        .into_any_element()
    }

    pub(in crate::app) fn source_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let tab = &self.tabs[self.active];
        let edit = tab.edit.as_ref().expect("source mode has an editor");
        let reading = crate::presentation::styles(cx).body;
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
                        .refine_style(&reading)
                        .on_paste(on_paste)
                        .aria_label("Markdown 源码编辑器"),
                ),
            )
            .into_any_element()
    }
}

fn corner_glyph(id: &'static str, icon: Icon, size: f32) -> gpui_kit::Stateful<gpui_kit::Div> {
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .child(icon.with_size(Size::Size(px(size))))
}

/// Paints the corner after each source line's own layer, so scrolling text stays underneath.
fn above_editor_text(child: impl IntoElement) -> AboveEditorText {
    AboveEditorText {
        child: child.into_any_element(),
    }
}

struct AboveEditorText {
    child: AnyElement,
}

impl IntoElement for AboveEditorText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for AboveEditorText {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.paint_layer(bounds, |window| self.child.paint(window, cx));
    }
}
