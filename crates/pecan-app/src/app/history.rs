//! The recent-path picker reuses the searchable, keyboard-accessible virtualized list.
use super::Reader;
use crate::recent::{Entry, Kind};
use gpui_kit::{
    App, Context, IntoElement, Task, WeakEntity, Window,
    component::{
        ActiveTheme, Disableable, IconName, IndexPath, Sizable, StyledExt as _, WindowExt,
        button::{Button, ButtonVariants},
        h_flex,
        list::{List, ListDelegate, ListItem, ListState},
        v_flex,
    },
    div,
    prelude::*,
    px,
};

impl Reader {
    pub(super) fn record_recent(
        &mut self,
        path: std::path::PathBuf,
        kind: Kind,
        cx: &mut Context<Self>,
    ) {
        self.recent
            .update(cx, |recent, cx| recent.record(Entry { path, kind }, cx));
    }
    pub(super) fn open_recent(&mut self, entry: Entry, cx: &mut Context<Self>) {
        match entry.kind {
            Kind::File => self.open_file(entry.path, cx),
            Kind::Folder => self.open_folder(entry.path, cx),
        }
    }
    pub(super) fn clear_recent(&mut self, cx: &mut Context<Self>) {
        self.recent.update(cx, |recent, cx| recent.clear(cx));
        self.status = "已清空最近记录".into();
        cx.notify();
    }
    pub(super) fn more_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let list = self.history_list(window, cx);
        let focus_list = list.clone();
        let weak = cx.weak_entity();
        let count = self.recent.read(cx).entries().len();
        let style = crate::ui::font_style(&self.presented);
        let font_size = self.presented.ui_size as f32;
        let height = (f32::from(window.viewport_size().height) - 240.).clamp(180., 400.);
        let width = (f32::from(window.viewport_size().width) - 80.).clamp(320., 700.);
        window.open_dialog(cx, move |dialog, _, cx| {
            let weak = weak.clone();
            dialog
                .title(div().text_size(px(font_size)).child("打开最近文件"))
                .w(px(width))
                .refine_style(&style)
                .text_size(px(font_size))
                .child(
                    List::new(&list)
                        .search_placeholder("搜索文件名或路径…")
                        .h(px(height)),
                )
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{count} 条记录 · ↑↓ 选择 · Enter 打开 · Esc 关闭")),
                        )
                        .child(
                            Button::new("recent-clear")
                                .debug_selector(|| "recent-clear".into())
                                .small()
                                .ghost()
                                .label("清空记录")
                                .disabled(count == 0)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = weak.update(cx, |reader, cx| reader.clear_recent(cx));
                                }),
                        ),
                )
        });
        focus_list.update(cx, |list, cx| list.focus(window, cx));
    }
    fn history_list(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Entity<ListState<Choices>> {
        let entries = self.recent.read(cx).entries().to_vec();
        let keys = entries
            .iter()
            .map(|entry| entry.path.to_string_lossy().to_lowercase())
            .collect();
        let filtered = (0..entries.len()).collect();
        let reader = cx.weak_entity();
        let list = cx.new(|cx| {
            ListState::new(
                Choices {
                    entries,
                    keys,
                    filtered,
                    selected: None,
                    reader,
                },
                window,
                cx,
            )
            .searchable(true)
        });
        // The picker owns only paths and a weak reader; closing it releases its search data.
        list.update(cx, |list, cx| {
            if !list.delegate().entries.is_empty() {
                list.set_selected_index(Some(IndexPath::new(0)), window, cx);
            }
        });
        list
    }
}

struct Choices {
    entries: Vec<Entry>,
    keys: Vec<String>,
    filtered: Vec<usize>,
    selected: Option<IndexPath>,
    reader: WeakEntity<Reader>,
}
impl ListDelegate for Choices {
    type Item = ListItem;
    fn items_count(&self, _: usize, _: &App) -> usize {
        self.filtered.len()
    }
    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Task<()> {
        let query = query.to_lowercase();
        self.filtered = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, key)| query.split_whitespace().all(|part| key.contains(part)))
            .map(|(index, _)| index)
            .collect();
        cx.notify();
        Task::ready(())
    }
    fn set_selected_index(
        &mut self,
        selected: Option<IndexPath>,
        _: &mut Window,
        _: &mut Context<ListState<Self>>,
    ) {
        self.selected = selected;
    }
    fn render_item(
        &mut self,
        index: IndexPath,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let entry = &self.entries[*self.filtered.get(index.row)?];
        let title = entry
            .path
            .file_name()
            .unwrap_or(entry.path.as_os_str())
            .to_string_lossy()
            .into_owned();
        let path = crate::recent::display_path(&entry.path);
        Some(
            ListItem::new(("recent-entry", index.row))
                .debug_selector(move || format!("recent-entry-{}", index.row))
                .h(px(52.))
                .px(px(8.))
                .cursor_pointer()
                .accessibility_label(format!("{title} · {path}"))
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap(px(10.))
                        .child(crate::ui::icon(if entry.kind == Kind::Folder {
                            IconName::Folder
                        } else {
                            IconName::File
                        }))
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(div().truncate().child(title))
                                .child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(cx.theme().muted_foreground)
                                        .truncate()
                                        .child(path),
                                ),
                        ),
                ),
        )
    }
    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .p(px(24.))
            .text_color(cx.theme().muted_foreground)
            .child(if self.entries.is_empty() {
                "暂无最近记录，打开文档后会自动保存。"
            } else {
                "没有匹配的文件或文件夹。"
            })
    }
    fn confirm(&mut self, _: bool, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        let entry = self
            .selected
            .and_then(|index| self.filtered.get(index.row))
            .and_then(|index| self.entries.get(*index))
            .cloned();
        if let Some(entry) = entry {
            window.close_dialog(cx);
            let _ = self
                .reader
                .update(cx, |reader, cx| reader.open_recent(entry, cx));
        }
    }
    fn cancel(&mut self, window: &mut Window, cx: &mut Context<ListState<Self>>) {
        window.close_dialog(cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recent::Recent;
    use gpui_kit::{TestAppContext, VisualTestContext};
    use std::time::Duration;

    fn settle(cx: &mut TestAppContext) {
        for _ in 0..10_000 {
            if !cx.executor().tick() {
                return;
            }
        }
        panic!("recent-file test did not settle within 10000 executor ticks");
    }
    #[gpui_kit::test]
    fn successful_opens_only_record_explicit_paths_and_activate_existing_tabs(
        cx: &mut TestAppContext,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let a = directory.path().join("first.md");
        let b = directory.path().join("second.md");
        std::fs::write(&a, "# First\n\nBody").unwrap();
        std::fs::write(&b, "# Second").unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            crate::menus::install(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    Reader::new(
                        Some(a.clone()),
                        pecan_config::startup::Launch::First,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        settle(cx);
        let identity = pecan_document::files::canonicalize(&a).unwrap();
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            let state = reader.read(cx);
            assert_eq!(
                state.recent.read(cx).entries(),
                [Entry {
                    path: identity.clone(),
                    kind: Kind::File
                }],
                "automatic parent folder is not a recent folder"
            );
        })
        .unwrap();
        cx.update(|cx| reader.update(cx, |reader, cx| reader.open_file(b, cx)));
        settle(cx);
        let view = cx.update(|cx| reader.read(cx).tabs[1].view.entity_id());
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            window.dispatch_action(
                Box::new(crate::recent::OpenRecent {
                    entry: Entry {
                        path: identity.clone(),
                        kind: Kind::File,
                    },
                }),
                cx,
            );
        })
        .unwrap();
        settle(cx);
        cx.update(|cx| {
            let state = reader.read(cx);
            assert_eq!(state.tabs.len(), 3, "recent open reuses the existing tab");
            assert_eq!(state.active, 1);
            assert_eq!(
                state.tabs[state.active].view.entity_id(),
                view,
                "the reading entity is retained"
            );
            assert_eq!(state.recent.read(cx).entries()[0].path, identity);
        });
        #[cfg(unix)]
        {
            let alias = directory.path().join("alias.md");
            std::os::unix::fs::symlink(&a, &alias).unwrap();
            cx.update(|cx| reader.update(cx, |reader, cx| reader.open_file(alias, cx)));
            settle(cx);
            cx.update(|cx| {
                assert_eq!(
                    reader.read(cx).tabs.len(),
                    3,
                    "symlink aliases do not duplicate tabs"
                )
            });
        }
        let before = cx.update(|cx| reader.read(cx).recent.read(cx).entries().to_vec());
        cx.update(|cx| {
            reader.update(cx, |reader, cx| {
                reader.open_file(directory.path().join("missing.md"), cx)
            })
        });
        settle(cx);
        cx.update(|cx| {
            let state = reader.read(cx);
            assert_eq!(state.recent.read(cx).entries(), before);
            assert!(state.status.contains("失败"));
        });
        cx.update(|cx| {
            reader.update(cx, |reader, cx| {
                reader.open_folder(directory.path().join("missing-folder"), cx)
            })
        });
        settle(cx);
        cx.update(|cx| {
            let state = reader.read(cx);
            assert_eq!(state.recent.read(cx).entries(), before);
            assert_eq!(
                state.root.as_deref(),
                Some(directory.path()),
                "failed folder open keeps the previous tree"
            );
        });
        cx.update(|cx| {
            reader.update(cx, |reader, cx| {
                reader.open_folder(directory.path().to_path_buf(), cx)
            })
        });
        settle(cx);
        cx.update(|cx| {
            assert_eq!(
                reader.read(cx).recent.read(cx).entries()[0].kind,
                Kind::Folder
            )
        });
    }

    fn draw(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
    }
    fn show(visual: &mut VisualTestContext, reader: &gpui_kit::Entity<Reader>) {
        visual.update(|window, cx| {
            reader.read(cx).focus.clone().focus(window, cx);
            window.draw(cx).clear(cx);
            window.dispatch_action(Box::new(crate::MoreRecent), cx);
        });
        draw(visual);
        visual.update(|window, cx| {
            assert!(window.has_active_dialog(cx));
            assert!(
                window.focused_input(cx).is_some(),
                "search has keyboard focus"
            );
        });
    }
    #[gpui_kit::test]
    fn recent_dialog_search_keyboard_click_and_clear_work_in_the_reader(cx: &mut TestAppContext) {
        let directory = tempfile::tempdir().unwrap();
        let a = directory.path().join("alpha.md");
        let b = directory.path().join("中文 beta.md");
        std::fs::write(&a, "# Alpha").unwrap();
        std::fs::write(&b, "# Beta").unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            let recent = Recent::install(None, cx);
            recent.update(cx, |recent, cx| {
                for path in [&a, &b] {
                    recent.record(
                        Entry {
                            path: pecan_document::files::canonicalize(path).unwrap(),
                            kind: Kind::File,
                        },
                        cx,
                    );
                }
            });
            crate::menus::install(cx);
            gpui_kit::open_window(
                crate::native_window_options(
                    "Recent test",
                    cx,
                    gpui_kit::size(px(1000.), px(760.)),
                ),
                cx,
                |window, cx| {
                    cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        show(&mut visual, &reader);
        let row = visual.debug_bounds("recent-entry-0").unwrap();
        assert!(
            f32::from(row.size.height) >= 50.,
            "recent rows remain comfortable click targets"
        );
        assert!(visual.debug_bounds("recent-entry-1").is_some());
        visual.simulate_input("ALPHA");
        draw(&mut visual);
        assert!(visual.debug_bounds("recent-entry-0").is_some());
        assert!(visual.debug_bounds("recent-entry-1").is_none());
        visual.simulate_keystrokes("enter");
        draw(&mut visual);
        visual.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            let state = reader.read(cx);
            assert_eq!(
                state.tabs[state.active].identity,
                Some(pecan_document::files::canonicalize(&a).unwrap()),
                "uppercase filename search selects alpha"
            );
        });
        show(&mut visual, &reader);
        visual.simulate_keystrokes("down enter");
        draw(&mut visual);
        visual.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            let state = reader.read(cx);
            assert_eq!(state.tabs.len(), 3);
            assert_eq!(
                state.tabs[state.active].identity,
                Some(pecan_document::files::canonicalize(&b).unwrap()),
                "arrow keys select beta"
            );
        });
        show(&mut visual, &reader);
        let point = visual.debug_bounds("recent-entry-1").unwrap().center();
        visual.simulate_click(point, Default::default());
        draw(&mut visual);
        visual.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert_eq!(
                reader.read(cx).tabs.len(),
                3,
                "clicking recent alpha reuses its tab"
            );
        });
        show(&mut visual, &reader);
        visual.simulate_keystrokes("escape");
        draw(&mut visual);
        visual.update(|window, cx| assert!(!window.has_active_dialog(cx)));
        show(&mut visual, &reader);
        let point = visual.debug_bounds("recent-clear").unwrap().center();
        visual.simulate_click(point, Default::default());
        draw(&mut visual);
        visual.update(|window, cx| {
            assert!(!window.has_active_dialog(cx));
            assert!(reader.read(cx).recent.read(cx).entries().is_empty());
        });
        show(&mut visual, &reader);
        assert!(visual.debug_bounds("recent-entry-0").is_none());
        visual.simulate_keystrokes("enter");
        visual.update(|window, cx| {
            assert!(
                window.has_active_dialog(cx),
                "Enter on an empty list must not open anything"
            )
        });
        visual.executor().advance_clock(Duration::from_millis(150));
        visual.simulate_keystrokes("escape");
    }
}
