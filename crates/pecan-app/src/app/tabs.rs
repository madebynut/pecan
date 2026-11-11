//! Tab interaction stays local to the reader; documents and imported assets move together.
use super::Reader;
use gpui_kit::{
    AnyElement, ClipboardItem, Context, EntityId, IntoElement, Render, Window,
    component::{
        ActiveTheme, IconName, Sizable,
        button::{Button, ButtonVariants},
        h_flex,
        menu::{ContextMenuExt as _, PopupMenuItem},
    },
    div,
    prelude::*,
    px,
};

#[derive(Clone)]
struct TabDrag {
    owner: EntityId,
    view: EntityId,
    title: String,
}
impl Render for TabDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_2()
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .text_color(cx.theme().foreground)
            .text_size(px(14.))
            .max_w(px(240.))
            .truncate()
            .child(self.title.clone())
    }
}
#[derive(Clone, Copy)]
pub(super) enum CloseScope {
    One,
    Others,
    Right,
    All,
}

impl Reader {
    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs(index, CloseScope::One, window, cx);
    }
    fn close_tabs(
        &mut self,
        index: usize,
        scope: CloseScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.tabs.len() {
            return;
        }
        let ids = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(i, _)| match scope {
                CloseScope::One => *i == index,
                CloseScope::Others => *i != index,
                CloseScope::Right => *i > index,
                CloseScope::All => true,
            })
            .map(|(_, tab)| tab.view.entity_id())
            .collect();
        self.request_close(super::editing::ClosePlan::tabs(ids), window, cx);
    }
    pub(super) fn remove_tabs(
        &mut self,
        ids: &[EntityId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.tabs[self.active].view.entity_id();
        let old_index = self.active;
        self.tabs.retain(|tab| {
            let retained = !ids.contains(&tab.view.entity_id());
            if !retained {
                tab.view.update(cx, |view, cx| view.suspend(cx));
            }
            retained
        });
        if self.tabs.is_empty() {
            self.active = 0;
            self.load_revision += 1;
            self.load_job = None;
            self.search_revision += 1;
            self.search_job = None;
            self.matches = Default::default();
            self.match_index = 0;
            self.search_open = false;
            self.watcher = None;
            self.changed
                .store(false, std::sync::atomic::Ordering::Relaxed);
            self.pane_drag = None;
            self.focus_mode = false;
            self.status = "就绪".into();
            self.focus.focus(window, cx);
            if self.prefs.read(cx).data.quit_on_last_tab {
                window.remove_window();
                cx.defer(super::editing::request_quit);
            }
            cx.notify();
            return;
        }
        if let Some(index) = self
            .tabs
            .iter()
            .position(|tab| tab.view.entity_id() == active)
        {
            self.active = index;
            cx.notify();
        } else {
            self.active = old_index.min(self.tabs.len() - 1);
            self.activate(self.active, cx);
        }
        self.focus_document(window, cx);
    }
    fn move_tab(&mut self, view: EntityId, target: usize, cx: &mut Context<Self>) {
        let Some(from) = self
            .tabs
            .iter()
            .position(|tab| tab.view.entity_id() == view)
        else {
            return;
        };
        if target >= self.tabs.len() || from == target {
            return;
        }
        let active = self.tabs[self.active].view.entity_id();
        let tab = self.tabs.remove(from);
        self.tabs.insert(target, tab);
        self.active = self
            .tabs
            .iter()
            .position(|tab| tab.view.entity_id() == active)
            .expect("moving a tab retains the active document");
        cx.notify();
    }
    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let weak = cx.weak_entity();
                let view = tab.view.entity_id();
                let owner = cx.entity_id();
                h_flex()
                    .id(("tab", view))
                    .debug_selector(move || format!("tab-{index}"))
                    .flex_shrink_0()
                    .h(px(34.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(self.presented.ui_size as f32))
                    .when(index == self.active, |tab| {
                        tab.bg(cx.theme().accent).text_color(cx.theme().foreground)
                    })
                    .drag_over::<TabDrag>(|style, _, _, cx| {
                        style
                            .bg(cx.theme().muted)
                            .border_b_2()
                            .border_color(cx.theme().foreground)
                    })
                    .child(
                        div()
                            .cursor_pointer()
                            .max_w(px(200.))
                            .truncate()
                            .child(tab.title.clone()),
                    )
                    .when(tab.protected(), |row| {
                        row.child(div().text_size(px(11.)).child("●"))
                    })
                    .on_click(cx.listener(move |reader, _, window, cx| {
                        reader.activate(index, cx);
                        reader.focus_document(window, cx);
                    }))
                    .on_drag(
                        TabDrag {
                            owner,
                            view,
                            title: tab.title.clone(),
                        },
                        |drag, _, _, cx| cx.new(|_| drag.clone()),
                    )
                    .on_drop(cx.listener(move |reader, drag: &TabDrag, _, cx| {
                        if drag.owner == cx.entity_id() {
                            reader.move_tab(drag.view, index, cx);
                        }
                    }))
                    .child(
                        Button::new(("close", view))
                            .small()
                            .ghost()
                            .cursor_pointer()
                            .debug_selector(move || format!("tab-close-{index}"))
                            .child(crate::ui::icon(IconName::Close))
                            .on_click(cx.listener(move |reader, _, window, cx| {
                                cx.stop_propagation();
                                if let Some(index) = reader
                                    .tabs
                                    .iter()
                                    .position(|tab| tab.view.entity_id() == view)
                                {
                                    reader.close_tab(index, window, cx);
                                }
                            })),
                    )
                    .context_menu(move |mut menu, _, cx| {
                        let Some(reader) = weak.upgrade() else {
                            return menu;
                        };
                        let state = reader.read(cx);
                        let Some(index) = state
                            .tabs
                            .iter()
                            .position(|tab| tab.view.entity_id() == view)
                        else {
                            return menu;
                        };
                        let count = state.tabs.len();
                        let path = state.tabs[index].path.clone();
                        let relative = path
                            .as_ref()
                            .and_then(|path| {
                                state
                                    .root
                                    .as_ref()
                                    .and_then(|root| path.strip_prefix(root).ok())
                            })
                            .map(|path| path.to_path_buf());
                        for (label, scope, disabled) in [
                            ("关闭", CloseScope::One, false),
                            ("关闭其他标签页", CloseScope::Others, count <= 1),
                            ("关闭右侧标签页", CloseScope::Right, index + 1 == count),
                            ("全部关闭", CloseScope::All, false),
                        ] {
                            let weak = weak.clone();
                            menu =
                                menu.item(PopupMenuItem::new(label).disabled(disabled).on_click(
                                    move |_, window, cx| {
                                        let _ = weak.update(cx, |reader, cx| {
                                            if let Some(index) = reader
                                                .tabs
                                                .iter()
                                                .position(|tab| tab.view.entity_id() == view)
                                            {
                                                reader.close_tabs(index, scope, window, cx);
                                            }
                                        });
                                    },
                                ));
                        }
                        menu = menu.separator();
                        for (label, copy) in
                            [("复制路径", path.clone()), ("复制相对路径", relative)]
                        {
                            menu = menu.item(
                                PopupMenuItem::new(label).disabled(copy.is_none()).on_click(
                                    move |_, _, cx| {
                                        if let Some(path) = &copy {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                path.to_string_lossy().into_owned(),
                                            ));
                                        }
                                    },
                                ),
                            );
                        }
                        menu.item(
                            PopupMenuItem::new(crate::platform::REVEAL_FILE)
                                .disabled(path.is_none())
                                .on_click(move |_, _, cx| {
                                    if let Some(path) = &path {
                                        cx.reveal_path(path);
                                    }
                                }),
                        )
                    })
                    .into_any_element()
            })
            .collect()
    }
    pub(super) fn activate(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.tabs.len() {
            return;
        }
        if index != self.active
            && let Some(tab) = self.tabs.get(self.active)
        {
            tab.view.update(cx, |view, cx| view.suspend(cx));
        }
        self.active = index;
        if let Some(path) = self.tabs[self.active].path.clone() {
            self.watch(&path);
        } else {
            self.watcher = None;
            self.changed
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        self.search_revision += 1;
        self.search_job = None;
        self.matches = Default::default();
        self.match_index = 0;
        if self.search_open {
            self.start_search(cx);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Tab;
    use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};

    #[gpui_kit::test]
    fn tabs_drag_and_context_menu_keep_documents_and_close_the_requested_scope(
        cx: &mut TestAppContext,
    ) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        Default::default(),
                        size(px(1280.), px(860.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
                },
            )
            .unwrap()
        });
        cx.update(|cx| {
            reader.update(cx, |reader, cx| {
                reader.tabs.clear();
                for title in ["A", "B", "C", "D"] {
                    let document = std::sync::Arc::new(
                        pecan_document::Document::parse(format!("# {title}\n\n阅读正文")).unwrap(),
                    );
                    let view = cx.new(|cx| {
                        crate::reading::DocumentView::new(
                            document,
                            std::path::PathBuf::from("."),
                            reader.queue.clone(),
                            reader.math.clone(),
                            cx,
                        )
                    });
                    reader.tabs.push(Tab {
                        path: Some(format!("/tmp/{title}.md").into()),
                        identity: None,
                        title: title.into(),
                        view,
                        resources: None,
                        edit: None,
                    });
                }
                reader.active = 1;
                reader.root = Some("/tmp".into());
                cx.notify();
            })
        });
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        let draw = |window: &mut Window, cx: &mut gpui_kit::App| {
            window.refresh();
            window.draw(cx).clear(cx);
        };
        for _ in 0..10 {
            visual.run_until_parked();
            visual.update(draw);
        }
        let from = visual.debug_bounds("tab-1").unwrap();
        let to = visual.debug_bounds("tab-3").unwrap();
        let view = visual.update(|_, cx| {
            let r = reader.read(cx);
            r.tabs[r.active].view.entity_id()
        });
        visual.simulate_mouse_down(
            from.center(),
            gpui_kit::MouseButton::Left,
            Default::default(),
        );
        for step in 1..=8 {
            let position = from.center() + (to.center() - from.center()) * (step as f32 / 8.);
            visual.simulate_event(gpui_kit::MouseMoveEvent {
                position,
                pressed_button: Some(gpui_kit::MouseButton::Left),
                modifiers: Default::default(),
            });
            visual.update(draw);
        }
        visual.simulate_mouse_up(to.center(), gpui_kit::MouseButton::Left, Default::default());
        visual.update(draw);
        visual.update(|_, cx| {
            let r = reader.read(cx);
            assert_eq!(
                r.tabs
                    .iter()
                    .map(|tab| tab.title.as_str())
                    .collect::<Vec<_>>(),
                ["A", "C", "D", "B"]
            );
            assert_eq!(
                r.tabs[r.active].view.entity_id(),
                view,
                "drag preserves the active reader entity"
            );
        });
        // Moving inactive D must not activate it or rebuild the current document.
        let from = visual.debug_bounds("tab-2").unwrap().center();
        let to = visual.debug_bounds("tab-0").unwrap().center();
        visual.simulate_mouse_down(from, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(to, Some(gpui_kit::MouseButton::Left), Default::default());
        visual.update(draw);
        visual.simulate_mouse_up(to, gpui_kit::MouseButton::Left, Default::default());
        visual.update(draw);
        visual.update(|_, cx| {
            let r = reader.read(cx);
            assert_eq!(
                r.tabs
                    .iter()
                    .map(|tab| tab.title.as_str())
                    .collect::<Vec<_>>(),
                ["D", "A", "C", "B"]
            );
            assert_eq!(r.tabs[r.active].view.entity_id(), view);
        });
        let tab = visual.debug_bounds("tab-2").unwrap();
        // The inactive C tab's menu must act on C rather than the current B tab.
        visual.simulate_mouse_down(
            tab.center(),
            gpui_kit::MouseButton::Right,
            Default::default(),
        );
        visual.simulate_mouse_up(
            tab.center(),
            gpui_kit::MouseButton::Right,
            Default::default(),
        );
        visual.update(draw);
        // Menus select the first enabled entry; copy-relative is the sixth action.
        visual.simulate_keystrokes("down down down down down down enter");
        visual
            .update(|_, cx| assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(), "C.md"));
        visual.update(draw);
        visual.simulate_mouse_down(
            tab.center(),
            gpui_kit::MouseButton::Right,
            Default::default(),
        );
        visual.simulate_mouse_up(
            tab.center(),
            gpui_kit::MouseButton::Right,
            Default::default(),
        );
        visual.update(draw);
        visual.simulate_keystrokes("down down enter");
        visual.update(|_, cx| {
            let r = reader.read(cx);
            assert_eq!(r.tabs.len(), 1);
            assert_eq!(r.tabs[r.active].title, "C");
        });
        visual.update(|window, cx| {
            reader.update(cx, |reader, cx| {
                reader.close_tabs(0, CloseScope::All, window, cx)
            })
        });
        assert!(
            visual.update(|_, cx| !cx.windows().is_empty()),
            "closing all tabs retains the window by default"
        );
        visual.update(draw);
        assert!(visual.debug_bounds("empty-reader").is_some());
        let background = visual.debug_bounds("empty-reader").unwrap();
        let logo = visual.debug_bounds("empty-logo").unwrap();
        assert_eq!(logo.size, gpui_kit::size(px(100.), px(100.)));
        assert!((logo.center().x - background.center().x).abs() < px(1.));
        assert!((logo.center().y - background.center().y).abs() < px(1.));
        visual.update(|_, cx| {
            reader.update(cx, |reader, cx| {
                reader.help(cx);
                reader.prefs.update(cx, |prefs, cx| {
                    prefs.change(|data| data.quit_on_last_tab = true, cx);
                });
            });
        });
        visual.update(draw);
        let close = visual.debug_bounds("tab-close-0").unwrap().center();
        visual.simulate_mouse_down(close, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_up(close, gpui_kit::MouseButton::Left, Default::default());
        drop(visual);
        assert!(
            cx.update(|cx| cx.windows().is_empty()),
            "the enabled setting closes the last reader window"
        );
    }

    #[gpui_kit::test]
    fn empty_reader_actions_reopening_and_background_cancellation(cx: &mut TestAppContext) {
        use crate::{
            CloseTab, CopyDocument, CopyFilePath, ExportHtml, ExportMarkdown, ExportPdf,
            ExportWord, Find, Help, NextTab, PreviousTab, Reload, RevealFile,
        };
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("reopen.md");
        std::fs::write(&path, "# Reopened\n\nBody").unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::Returning, window, cx))
            })
            .unwrap()
        });
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        let draw = |window: &mut Window, cx: &mut gpui_kit::App| {
            window.refresh();
            window.draw(cx).clear(cx);
        };
        visual.update(draw);
        assert!(visual.debug_bounds("empty-reader").is_some());
        visual.update(|window, cx| {
            let focus = reader.read(cx).focus.clone();
            focus.focus(window, cx);
            cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
        });
        for action in [
            Box::new(CloseTab) as Box<dyn gpui_kit::Action>,
            Box::new(CopyDocument),
            Box::new(CopyFilePath),
            Box::new(RevealFile),
            Box::new(Find),
            Box::new(Reload),
            Box::new(NextTab),
            Box::new(PreviousTab),
            Box::new(ExportPdf),
            Box::new(ExportWord),
            Box::new(ExportHtml),
            Box::new(ExportMarkdown),
        ] {
            visual.update(|window, cx| window.dispatch_action(action, cx));
            visual.run_until_parked();
        }
        visual.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "unchanged"
            );
            assert!(reader.read(cx).tabs.is_empty());
            assert!(!reader.read(cx).search_open);
        });
        assert!(!visual.cx.did_prompt_for_new_path());
        visual.update(|window, cx| window.dispatch_action(Box::new(Help), cx));
        visual.update(draw);
        assert!(visual.debug_bounds("reading-context").is_some());
        // Close with a pending load and pending search: neither can resurrect a closed tab.
        visual.update(|window, cx| {
            reader.update(cx, |reader, cx| {
                reader.search_open = true;
                reader.start_search(cx);
                reader.open_file(path.clone(), cx);
                reader.close_tab(0, window, cx);
                assert!(reader.search_job.is_none());
                assert!(reader.load_job.is_none());
                assert!(reader.watcher.is_none());
                reader.activate(99, cx);
            });
        });
        visual.run_until_parked();
        visual.update(draw);
        assert!(visual.debug_bounds("empty-reader").is_some());
        visual.update(|_, cx| reader.update(cx, |reader, cx| reader.open_file(path.clone(), cx)));
        visual.run_until_parked();
        visual.update(draw);
        visual.update(|window, cx| {
            assert_eq!(
                reader.read(cx).tabs.len(),
                1,
                "reopening never inserts a welcome tab"
            );
            assert_eq!(reader.read(cx).tabs[0].path.as_ref(), Some(&path));
            reader.update(cx, |reader, cx| reader.close_tab(0, window, cx));
        });
        visual.update(draw);
        assert!(visual.debug_bounds("empty-reader").is_some());
    }
}
