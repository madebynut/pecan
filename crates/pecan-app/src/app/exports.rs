//! Shared export actions and useful reading context-menu commands.
use super::Reader;
use crate::{
    CopyFilePath, ExportHtml, ExportMarkdown, ExportPdf, ExportWord, Find, FocusReading, Reload,
    RevealFile,
    export::{self, DocumentFormat, Snapshot},
    reading::DocumentView,
};
use gpui_kit::{
    AnyElement, ClipboardItem, Context, Entity, IntoElement, Window,
    component::menu::{ContextMenuExt as _, PopupMenuItem},
    div,
    prelude::*,
};

impl Reader {
    pub(super) fn export_document(
        &mut self,
        format: DocumentFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.edit.as_ref().is_some_and(|edit| edit.preview_pending()) {
            self.prepare_reading(
                tab.view.entity_id(),
                super::editing::PreviewRequest::Export(format),
                window,
                cx,
            );
            return;
        }
        let view = tab.view.read(cx);
        let snapshot = Snapshot {
            document: view.document.clone(),
            directory: view.directory.clone(),
            title: tab.title.clone(),
            preferences: self.prefs.read(cx).data.clone(),
            codes: view.code_snapshot(cx),
            diagrams: self.queue.clone(),
            math: self.math.rendering.clone(),
            resources: tab.resources.clone(),
        };
        self.status = format!("正在导出 {}…", format.extension().to_uppercase());
        cx.notify();
        let weak = cx.weak_entity();
        export::save_document(snapshot, format, cx, move |result, cx| {
            let _ = weak.update(cx, |reader, cx| {
                reader.status = match result {
                    Ok(Some(path)) => format!("已导出 · {}", path.display()),
                    Ok(None) => "已取消导出".into(),
                    Err(error) => format!("导出失败：{error:#}"),
                };
                cx.notify();
            });
        });
    }

    /// Save a converted document's Markdown; Markdown files already are their own source.
    pub(super) fn export_markdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.markdown_exportable() {
            self.export_document(DocumentFormat::Markdown, window, cx);
            return;
        }
        // The native menu stays enabled on every platform, so explain why nothing opens.
        self.status = if tab.has_markdown_source() {
            "当前文档已是 Markdown，无需导出；如需副本请使用「另存为…」".into()
        } else {
            "这份文档无法完整转换为 Markdown，请使用原版查看".into()
        };
        cx.notify();
    }

    pub(super) fn document_body(
        &self,
        view: Entity<DocumentView>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let weak = cx.weak_entity();
        let focus = self.focus.clone();
        div()
            .id("reading-context")
            .debug_selector(|| "reading-context".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .child(view)
            .context_menu(move |menu, window, cx| {
                let selected = gpui_kit::base::TextSelection::selected_text(window, cx);
                let (path, markdown_source, exportable) = weak
                    .upgrade()
                    .and_then(|reader| {
                        let reader = reader.read(cx);
                        let tab = reader.tabs.get(reader.active)?;
                        Some((
                            tab.path.clone(),
                            tab.has_markdown_source(),
                            tab.markdown_exportable(),
                        ))
                    })
                    .unwrap_or((None, false, false));
                menu.action_context(focus.clone())
                    .item(
                        PopupMenuItem::new("复制选中内容")
                            .disabled(selected.is_empty())
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(selected.clone()));
                            }),
                    )
                    .when(markdown_source, |menu| {
                        menu.menu("查看源码", Box::new(crate::ToggleSource))
                    })
                    .menu("查找…", Box::new(Find))
                    .separator()
                    .submenu("导出文档", window, cx, move |menu, _, _| {
                        menu.menu("PDF…", Box::new(ExportPdf))
                            .menu("Word…", Box::new(ExportWord))
                            .menu("HTML…", Box::new(ExportHtml))
                            .when(exportable, |menu| {
                                menu.menu("Markdown…", Box::new(ExportMarkdown))
                            })
                    })
                    .separator()
                    .menu("专注阅读", Box::new(FocusReading))
                    .menu("重新加载", Box::new(Reload))
                    .separator()
                    .item(
                        PopupMenuItem::new("复制文件路径")
                            .action(Box::new(CopyFilePath))
                            .disabled(path.is_none()),
                    )
                    .item(
                        PopupMenuItem::new(crate::platform::REVEAL_FILE)
                            .action(Box::new(RevealFile))
                            .disabled(path.is_none()),
                    )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext, VisualTestContext, component::TitleBar};

    #[gpui_kit::test]
    fn system_and_reading_menu_actions_open_the_same_native_save_panel(cx: &mut TestAppContext) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menus::install(cx);
            gpui_kit::open_window(TitleBar::window_options(), cx, |window, cx| {
                cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        visual.update(|window, cx| {
            let focus = reader.read(cx).focus.clone();
            focus.focus(window, cx);
            window.activate_window();
            window.draw(cx).clear(cx);
        });
        // These are the very actions shared by the native and context menus.
        for action in [
            Box::new(ExportPdf) as Box<dyn gpui_kit::Action>,
            Box::new(ExportWord),
            Box::new(ExportHtml),
        ] {
            visual.update(|window, cx| window.dispatch_action(action, cx));
            visual.run_until_parked();
            assert!(visual.cx.did_prompt_for_new_path());
            visual.cx.simulate_new_path_selection(|_| None);
            visual.run_until_parked();
        }
        visual.update(|window, cx| {
            let focus = reader.read(cx).focus.clone();
            focus.focus(window, cx);
            window.draw(cx).clear(cx);
        });
        let bounds = visual.debug_bounds("reading-context").unwrap();
        let position = bounds.origin + gpui_kit::point(gpui_kit::px(8.), gpui_kit::px(8.));
        visual.simulate_mouse_down(position, gpui_kit::MouseButton::Right, Default::default());
        visual.simulate_mouse_up(position, gpui_kit::MouseButton::Right, Default::default());
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        // GPUI Kit 0.9 skips the disabled selection item on the first Down.
        visual.simulate_keystrokes("down enter");
        visual.run_until_parked();
        reader.read_with(&visual.cx, |reader, _| {
            assert!(
                reader.tabs[reader.active].source_mode(),
                "查看源码 opens the Markdown source"
            );
        });
    }
    #[gpui_kit::test]
    fn converted_document_menu_omits_source_and_offers_markdown_export(cx: &mut TestAppContext) {
        let source = std::path::absolute(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../pecan-import/tests/fixtures/rich.docx"),
        )
        .unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menus::install(cx);
            gpui_kit::open_window(TitleBar::window_options(), cx, |window, cx| {
                cx.new(|cx| {
                    Reader::new(
                        Some(source),
                        pecan_config::startup::Launch::Returning,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        crate::app::tests::settle(&visual.cx);
        let open_menu = |visual: &mut VisualTestContext| {
            visual.update(|window, cx| {
                let focus = reader.read(cx).focus.clone();
                focus.focus(window, cx);
                window.activate_window();
                window.draw(cx).clear(cx);
            });
            let bounds = visual.debug_bounds("reading-context").unwrap();
            let position = bounds.origin + gpui_kit::point(gpui_kit::px(8.), gpui_kit::px(8.));
            visual.simulate_mouse_down(position, gpui_kit::MouseButton::Right, Default::default());
            visual.simulate_mouse_up(position, gpui_kit::MouseButton::Right, Default::default());
            visual.update(|window, cx| window.draw(cx).clear(cx));
            visual.run_until_parked();
            visual.update(|window, cx| window.draw(cx).clear(cx));
        };
        open_menu(&mut visual);
        // Without 查看源码, the first enabled item is 查找….
        visual.simulate_keystrokes("down enter");
        visual.run_until_parked();
        reader.read_with(&visual.cx, |reader, _| {
            assert!(!reader.tabs[reader.active].source_mode());
            assert!(reader.search_open, "查找… follows 复制选中内容 directly");
        });
        visual.simulate_keystrokes("escape");
        open_menu(&mut visual);
        // 查找…, then 导出文档; its last entry is Markdown….
        visual.simulate_keystrokes("down down right up enter");
        visual.run_until_parked();
        assert!(visual.cx.did_prompt_for_new_path());
        reader.read_with(&visual.cx, |reader, _| {
            assert_eq!(reader.status, "正在导出 MD…");
        });
        visual.cx.simulate_new_path_selection(|_| None);
        visual.run_until_parked();
        // The File menu item dispatches the same action from the window.
        let listed = visual.cx.update(|cx| {
            cx.get_menus().unwrap().iter().any(|menu| {
                menu.name == "文件"
                    && menu.items.iter().any(|item| {
                        matches!(item, gpui_kit::OwnedMenuItem::Action { name, action, .. }
                            if name == "导出 Markdown…" && action.partial_eq(&ExportMarkdown))
                    })
            })
        });
        assert!(listed, "文件 lists 导出 Markdown…");
        visual.update(|window, cx| window.dispatch_action(Box::new(ExportMarkdown), cx));
        visual.run_until_parked();
        assert!(visual.cx.did_prompt_for_new_path());
        visual.cx.simulate_new_path_selection(|_| None);
        visual.run_until_parked();
    }
}
