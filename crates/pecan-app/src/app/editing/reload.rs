//! Explicit reload commits a new disk snapshot without replacing the editing session.
use super::Reader;
use crate::{reading::DocumentView, workspace};
use gpui_kit::{AppContext as _, Context, EntityId, Window};
use std::sync::Arc;

impl Reader {
    pub(super) fn reload_source(
        &mut self,
        id: EntityId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edit_job.is_some() {
            self.status = "正在处理源码，请稍候".into();
            cx.notify();
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.view.entity_id() == id) else {
            return;
        };
        let (Some(path), Some(edit)) = (tab.path.clone(), tab.edit.as_mut()) else {
            return;
        };
        let revision = edit.revision;
        edit.set_busy(true, cx);
        edit.comparison = None;
        self.status = "正在重新加载源码…".into();
        let worker_path = path.clone();
        let background = cx.background_spawn(async move {
            let _permit = workspace::CONVERSION_SLOTS.acquire().await;
            let identity = pecan_document::files::canonicalize(&worker_path)?;
            workspace::load(&worker_path).map(|loaded| (identity, loaded))
        });
        self.edit_job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result = background.await;
            let _ = weak.update_in(cx, |reader, window, cx| {
                reader.edit_job = None;
                let Some(index) = reader
                    .tabs
                    .iter()
                    .position(|tab| tab.view.entity_id() == id)
                else {
                    return;
                };
                let other_bytes: usize = reader
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| *i != index)
                    .map(|(_, tab)| tab.source_bytes(cx))
                    .sum();
                let tab = &mut reader.tabs[index];
                let Some(edit) = tab.edit.as_mut() else {
                    return;
                };
                edit.set_busy(false, cx);
                // Read-only input prevents new edits; this also guards queued input events.
                if edit.revision != revision || tab.path.as_ref() != Some(&path) {
                    reader.status = "源码已变化，重新加载已取消；当前内容已保留".into();
                    cx.notify();
                    return;
                }
                match result {
                    Ok((_, loaded))
                        if other_bytes.saturating_add(loaded.document.source().len())
                            > super::super::MAX_OPEN_BYTES =>
                    {
                        reader.status =
                            "重新加载后文档总量超过 64 MiB，请关闭部分标签后重试；当前内容已保留"
                                .into();
                    }
                    Ok((identity, loaded)) => {
                        let resources = Arc::new(loaded.resources);
                        let document = Arc::new(loaded.document);
                        let directory = resources.directory().to_path_buf();
                        edit.saved_text = edit.editor.update(cx, |editor, cx| {
                            let selection = editor.selected_range();
                            let reversed = editor.is_selection_reversed();
                            // set_value clears old undo history without emitting a user edit.
                            editor.set_value(document.source().to_owned(), window, cx);
                            editor.set_selected_range_with_direction(selection, reversed, cx);
                            editor.text().clone()
                        });
                        edit.revision += 1;
                        edit.preview_revision = edit.revision;
                        edit.saved_revision = None;
                        edit.saved = document.clone();
                        edit.dirty = false;
                        let offset = tab.view.read(cx).scroll.logical_scroll_top();
                        // Keep the entity ID used by the editor's change subscription.
                        tab.view.update(cx, |view, cx| {
                            *view = DocumentView::new(
                                document,
                                directory,
                                reader.queue.clone(),
                                reader.math.clone(),
                                cx,
                            );
                            view.scroll.scroll_to(offset);
                            if index != reader.active || edit.show_source {
                                view.suspend(cx);
                            }
                        });
                        reader.status = resources.notice().unwrap_or("已重新加载").into();
                        tab.identity = Some(identity);
                        tab.resources = Some(resources);
                        if index == reader.active {
                            reader.watch(&path);
                            reader.focus_document(window, cx);
                            if reader.search_open {
                                reader.start_search(cx);
                            }
                        }
                    }
                    Err(error) => reader.status = format!("读取失败：{error:#}；当前内容已保留"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
