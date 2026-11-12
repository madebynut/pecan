//! Source sessions, asynchronous preview/save, and one shared unsaved-changes guard.
mod recovery;
mod reload;
mod save;
#[cfg(test)]
mod tests;
mod view;
use super::{Reader, Tab};
use crate::reading::DocumentView;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, EntityId, Focusable as _, Global,
    PromptLevel, Subscription, Task, WeakEntity, Window,
    component::input::{EditorState, InputEvent, Rope, TabSize},
};
use pecan_document::Document;
use std::{sync::Arc, time::Duration};

pub(super) use recovery::Drafts;
pub(crate) use recovery::{install as install_recovery, offer as offer_recovery};

#[derive(Clone, Copy)]
pub(super) enum SaveMode {
    Existing,
    As,
}

pub(super) enum PreviewRequest {
    Reading,
    Export(pecan_export::DocumentFormat),
    Heading { title: String, occurrence: usize },
}

pub(super) struct Session {
    pub(super) editor: Entity<EditorState>,
    pub(super) show_source: bool,
    pub(super) dirty: bool,
    busy: bool,
    revision: u64,
    preview_revision: u64,
    saved_revision: Option<u64>,
    saved: Arc<Document>,
    saved_text: Rope,
    _subscription: Subscription,
    comparison: Option<Task<()>>,
}
impl Session {
    fn save_confirmed(&self) -> bool {
        !self.dirty && self.revision > 0 && self.saved_revision == Some(self.revision)
    }

    pub(super) fn preview_pending(&self) -> bool {
        self.preview_revision != self.revision
    }

    fn set_busy(&mut self, busy: bool, cx: &mut App) {
        self.busy = busy;
        self.editor
            .update(cx, |editor, cx| editor.set_readonly(busy, cx));
    }
}

impl Tab {
    pub(super) fn source_bytes(&self, cx: &App) -> usize {
        self.edit.as_ref().map_or_else(
            || self.view.read(cx).document.source().len(),
            |edit| {
                edit.editor
                    .read(cx)
                    .text()
                    .len()
                    .max(self.view.read(cx).document.source().len())
            },
        )
    }
    pub(super) fn source_mode(&self) -> bool {
        self.edit.as_ref().is_some_and(|edit| edit.show_source)
    }
    pub(super) fn protected(&self) -> bool {
        self.edit
            .as_ref()
            .is_some_and(|edit| edit.dirty || edit.busy)
    }
}

#[derive(Default)]
struct Readers(Vec<(AnyWindowHandle, WeakEntity<Reader>)>);
impl Global for Readers {}

/// Drafts the user chose not to save during the quit in progress, by tab and revision.
///
/// The approvals stay out of each session's dirty flag: cancelling a later window's
/// prompt must leave earlier windows' drafts protected.
#[derive(Default)]
struct QuitApprovals(Vec<(EntityId, u64)>);
impl Global for QuitApprovals {}

fn quit_approved(tab: &Tab, cx: &App) -> bool {
    tab.edit.as_ref().is_some_and(|edit| {
        !edit.busy
            && cx.try_global::<QuitApprovals>().is_some_and(|approvals| {
                approvals.0.contains(&(tab.view.entity_id(), edit.revision))
            })
    })
}

pub(super) fn register(window: &mut Window, cx: &mut Context<Reader>) {
    if !cx.has_global::<Readers>() {
        cx.set_global(Readers::default());
    }
    let weak = cx.weak_entity();
    cx.global_mut::<Readers>()
        .0
        .retain(|(_, reader)| reader.upgrade().is_some());
    cx.global_mut::<Readers>()
        .0
        .push((window.window_handle(), weak.clone()));
    window.on_window_should_close(cx, move |window, cx| {
        weak.update(cx, |reader, cx| {
            if reader.tabs.iter().any(Tab::protected) {
                reader.close_window(window, cx);
                false
            } else {
                // Every draft was saved; drop copies the ticker has not removed yet.
                reader.discard_all_drafts(cx);
                true
            }
        })
        .unwrap_or(true)
    });
}

pub(crate) fn request_close_window(handle: AnyWindowHandle, cx: &mut App) -> bool {
    let reader = cx.try_global::<Readers>().and_then(|readers| {
        readers
            .0
            .iter()
            .find(|(window, _)| *window == handle)
            .map(|(_, reader)| reader.clone())
    });
    let Some(reader) = reader.and_then(|reader| reader.upgrade()) else {
        return false;
    };
    let _ = handle.update(cx, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.close_window(window, cx))
    });
    true
}

/// Open files the system hands to a running app in the frontmost reader window.
///
/// Returns `false` when no reader window is open, so the caller can create one.
pub(crate) fn open_in_front(paths: Vec<std::path::PathBuf>, cx: &mut App) -> bool {
    let active = cx.active_window();
    let target = cx.try_global::<Readers>().and_then(|readers| {
        let live = || {
            readers
                .0
                .iter()
                .filter_map(|(window, reader)| Some((*window, reader.upgrade()?)))
        };
        live()
            .find(|(window, _)| Some(*window) == active)
            .or_else(|| live().next_back())
    });
    let Some((handle, reader)) = target else {
        return false;
    };
    handle
        .update(cx, |_, window, cx| {
            window.activate_window();
            reader.update(cx, |reader, cx| reader.open_paths(paths, cx));
        })
        .is_ok()
}

/// Start quitting: every window with unsaved drafts asks before the app exits.
pub(crate) fn request_quit(cx: &mut App) {
    cx.set_global(QuitApprovals::default());
    continue_quit(cx);
}

fn continue_quit(cx: &mut App) {
    let pending = cx.try_global::<Readers>().and_then(|readers| {
        readers.0.iter().find_map(|(window, reader)| {
            let reader = reader.upgrade()?;
            reader
                .read(cx)
                .tabs
                .iter()
                .any(|tab| tab.protected() && !quit_approved(tab, cx))
                .then_some((*window, reader))
        })
    });
    if let Some((handle, reader)) = pending {
        let _ = handle.update(cx, |_, window, cx| {
            reader.update(cx, |reader, cx| {
                let mut plan =
                    ClosePlan::tabs(reader.tabs.iter().map(|tab| tab.view.entity_id()).collect());
                plan.finish = Finish::Quit;
                reader.request_close(plan, window, cx);
            })
        });
    } else {
        // Every window agreed: only now do the declined drafts stop guarding their windows.
        let readers: Vec<_> = cx.try_global::<Readers>().map_or_else(Vec::new, |readers| {
            readers
                .0
                .iter()
                .filter_map(|(_, reader)| reader.upgrade())
                .collect()
        });
        for reader in readers {
            reader.update(cx, |reader, cx| {
                // Each draft was saved or declined, so none should be offered at next launch.
                reader.discard_all_drafts(cx);
                for index in 0..reader.tabs.len() {
                    if quit_approved(&reader.tabs[index], cx)
                        && let Some(edit) = &mut reader.tabs[index].edit
                    {
                        edit.dirty = false;
                    }
                }
            });
        }
        cx.quit();
    }
}

#[derive(Clone, Copy, Debug)]
enum Finish {
    Tabs,
    Window,
    Quit,
    Reload,
    Reading,
}

pub(super) struct ClosePlan {
    ids: Vec<EntityId>,
    approved: Vec<(EntityId, u64)>,
    finish: Finish,
}
impl ClosePlan {
    pub(super) fn tabs(ids: Vec<EntityId>) -> Self {
        Self {
            ids,
            approved: Vec::new(),
            finish: Finish::Tabs,
        }
    }
}

impl Reader {
    pub(super) fn focus_document(&self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.edit.as_ref())
            .filter(|edit| edit.show_source)
            .map_or_else(|| self.focus.clone(), |edit| edit.editor.focus_handle(cx));
        focus.focus(window, cx);
    }

    fn ensure_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<EntityId> {
        let tab = self.tabs.get_mut(self.active)?;
        let id = tab.view.entity_id();
        if tab.edit.is_none() {
            let document = tab.view.read(cx).document.clone();
            let tab_size = self.presented.code_tab_size.parse().unwrap_or(4);
            let editor = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("markdown")
                    .soft_wrap(false)
                    .line_number(true)
                    .tab_size(TabSize {
                        tab_size,
                        hard_tabs: false,
                    })
                    .default_value(document.source().to_owned())
            });
            let saved_text = editor.read(cx).text().clone();
            let subscription =
                cx.subscribe(&editor, move |reader, editor, event: &InputEvent, cx| {
                    if !matches!(event, InputEvent::Change) {
                        return;
                    }
                    let Some(tab) = reader
                        .tabs
                        .iter_mut()
                        .find(|tab| tab.view.entity_id() == id)
                    else {
                        return;
                    };
                    let Some(edit) = tab.edit.as_mut() else {
                        return;
                    };
                    edit.revision += 1;
                    edit.dirty = true;
                    let revision = edit.revision;
                    let current = editor.read(cx).text().clone();
                    let saved = edit.saved_text.clone();
                    let executor = cx.background_executor().clone();
                    // Rope snapshots share storage; compare off-thread after typing settles.
                    edit.comparison = Some(cx.spawn(async move |weak, cx| {
                        executor.timer(Duration::from_millis(250)).await;
                        let equal = cx.background_spawn(async move { current == saved }).await;
                        let _ = weak.update(cx, |reader, cx| {
                            if let Some(edit) = reader
                                .tabs
                                .iter_mut()
                                .find(|tab| tab.view.entity_id() == id)
                                .and_then(|tab| tab.edit.as_mut())
                                && edit.revision == revision
                            {
                                edit.dirty = !equal;
                                cx.notify();
                            }
                        });
                    }));
                    cx.notify();
                });
            tab.edit = Some(Session {
                editor,
                show_source: false,
                dirty: false,
                busy: false,
                revision: 0,
                preview_revision: 0,
                saved_revision: None,
                saved: document,
                saved_text,
                _subscription: subscription,
                comparison: None,
            });
        }
        Some(id)
    }

    pub(super) fn toggle_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.ensure_editor(window, cx) else {
            return;
        };
        if self.tabs[self.active].source_mode() {
            let dirty = self.tabs[self.active]
                .edit
                .as_ref()
                .is_some_and(|edit| edit.dirty);
            if dirty {
                self.confirm_reading(id, window, cx);
            } else {
                self.prepare_reading(id, PreviewRequest::Reading, window, cx);
            }
        } else {
            self.search_open = false;
            self.search_revision += 1;
            self.search_job = None;
            let tab = &mut self.tabs[self.active];
            tab.view.update(cx, |view, cx| view.suspend(cx));
            let edit = tab.edit.as_mut().expect("editor was initialized");
            edit.show_source = true;
            edit.editor.focus_handle(cx).focus(window, cx);
            cx.notify();
        }
    }

    /// Leaving a dirty source asks with the same choices as closing a tab.
    fn confirm_reading(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_prompt {
            return;
        }
        if self.edit_job.is_some() {
            self.status = "正在处理源码，请稍候".into();
            cx.notify();
            return;
        }
        let Some(title) = self
            .tabs
            .iter()
            .find(|tab| tab.view.entity_id() == id)
            .map(|tab| tab.title.clone())
        else {
            return;
        };
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("要保存对“{title}”的修改吗？"),
            Some("未保存的修改不会写入文件。选择“不保存”仍会进入阅读模式，源码草稿会保留。"),
            &["保存", "不保存", "取消"],
            cx,
        );
        self.close_prompt = true;
        cx.spawn_in(window, async move |weak, cx| {
            let answer = prompt.await.ok();
            let _ = weak.update_in(cx, |reader, window, cx| {
                reader.close_prompt = false;
                match answer {
                    Some(0) => {
                        let mut plan = ClosePlan::tabs(vec![id]);
                        plan.finish = Finish::Reading;
                        reader.save_source(id, SaveMode::Existing, Some(plan), window, cx);
                    }
                    Some(1) => {
                        reader.prepare_reading(id, PreviewRequest::Reading, window, cx);
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn prepare_reading(
        &mut self,
        id: EntityId,
        request: PreviewRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.edit_job.is_some() {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.view.entity_id() == id) else {
            return;
        };
        let Some(edit) = tab.edit.as_mut() else {
            return;
        };
        if edit.preview_revision == edit.revision {
            self.finish_preview(id, request, window, cx);
            return;
        }
        let source = edit.editor.read(cx).text().clone();
        let revision = edit.revision;
        edit.set_busy(true, cx);
        self.status = "正在准备阅读视图…".into();
        let background =
            cx.background_spawn(async move { Document::parse(source.to_string()).map(Arc::new) });
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
                let tab = &mut reader.tabs[index];
                let Some(edit) = tab.edit.as_mut() else {
                    return;
                };
                edit.set_busy(false, cx);
                match result {
                    Ok(document) if edit.revision == revision => {
                        edit.preview_revision = revision;
                        let directory = tab.view.read(cx).directory.clone();
                        tab.view.update(cx, |view, cx| {
                            *view = DocumentView::new(
                                document,
                                directory.to_path_buf(),
                                reader.queue.clone(),
                                reader.math.clone(),
                                cx,
                            )
                        });
                        if index != reader.active || edit.show_source {
                            tab.view.update(cx, |view, cx| view.suspend(cx));
                        }
                        reader.status = if edit.dirty {
                            "阅读预览 · 有未保存修改"
                        } else {
                            "就绪"
                        }
                        .into();
                        reader.finish_preview(id, request, window, cx);
                    }
                    Ok(_) => reader.status = "源码已变化，请重新切换阅读模式".into(),
                    Err(error) => reader.status = format!("无法预览：{error}；源码已保留"),
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn finish_preview(
        &mut self,
        id: EntityId,
        request: PreviewRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self
            .tabs
            .get_mut(self.active)
            .filter(|tab| tab.view.entity_id() == id)
        else {
            return;
        };
        let Some(edit) = tab.edit.as_mut() else {
            return;
        };
        match request {
            PreviewRequest::Reading => {
                edit.show_source = false;
                // The green check is only a confirmation of the save just made.
                edit.saved_revision = None;
                self.focus.focus(window, cx);
            }
            PreviewRequest::Export(format) => self.export_document(format, window, cx),
            PreviewRequest::Heading { title, occurrence } => {
                let heading = tab
                    .view
                    .read(cx)
                    .document
                    .headings()
                    .iter()
                    .enumerate()
                    .filter(|(_, heading)| heading.title() == title)
                    .nth(occurrence)
                    .map(|(index, heading)| (index, heading.source_range()));
                if let Some((index, range)) = heading {
                    edit.editor.update(cx, |editor, cx| {
                        editor.set_selected_range(range.start..range.start, cx);
                        editor.focus(window, cx);
                    });
                    tab.view.update(cx, |view, cx| {
                        view.selected_heading = Some(index);
                        cx.notify();
                    });
                } else {
                    self.status = "标题已被修改或删除，大纲已更新，请重新选择".into();
                }
            }
        }
        cx.notify();
    }

    pub(super) fn navigate_heading(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.source_mode() {
            let document = &tab.view.read(cx).document;
            let Some(heading) = document.headings().get(index) else {
                return;
            };
            let title = heading.title().to_owned();
            let occurrence = document.headings()[..index]
                .iter()
                .filter(|heading| heading.title() == title)
                .count();
            self.prepare_reading(
                tab.view.entity_id(),
                PreviewRequest::Heading { title, occurrence },
                window,
                cx,
            );
        } else {
            tab.view
                .update(cx, |view, cx| view.reveal_heading(index, cx));
        }
        cx.notify();
    }

    pub(super) fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut plan = ClosePlan::tabs(self.tabs.iter().map(|tab| tab.view.entity_id()).collect());
        plan.finish = Finish::Window;
        self.request_close(plan, window, cx);
    }

    pub(super) fn reload_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active).filter(|tab| tab.edit.is_some()) {
            let mut plan = ClosePlan::tabs(vec![tab.view.entity_id()]);
            plan.finish = Finish::Reload;
            self.request_close(plan, window, cx);
        } else {
            self.reload(cx);
        }
    }

    pub(super) fn request_close(
        &mut self,
        mut plan: ClosePlan,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.close_prompt {
            return;
        }
        if self.tabs.iter().any(|tab| {
            plan.ids.contains(&tab.view.entity_id())
                && tab.edit.as_ref().is_some_and(|edit| edit.busy)
        }) {
            self.status = "正在处理源码，请完成后再关闭".into();
            cx.notify();
            return;
        }
        if let Some(tab) = self.tabs.iter().find(|tab| {
            plan.ids.contains(&tab.view.entity_id())
                && tab.edit.as_ref().is_some_and(|edit| {
                    edit.dirty
                        && !plan
                            .approved
                            .contains(&(tab.view.entity_id(), edit.revision))
                })
        }) {
            let id = tab.view.entity_id();
            let revision = tab.edit.as_ref().expect("dirty tab has an editor").revision;
            let prompt = window.prompt(
                PromptLevel::Warning,
                &format!("要保存对“{}”的修改吗？", tab.title),
                Some("未保存的修改会在关闭或重新加载后丢失。"),
                &["保存", "不保存", "取消"],
                cx,
            );
            self.close_prompt = true;
            cx.spawn_in(window, async move |weak, cx| {
                let answer = prompt.await.ok();
                let _ = weak.update_in(cx, |reader, window, cx| {
                    reader.close_prompt = false;
                    match answer {
                        Some(0) => {
                            reader.save_source(id, SaveMode::Existing, Some(plan), window, cx)
                        }
                        Some(1) => {
                            plan.approved.push((id, revision));
                            reader.request_close(plan, window, cx);
                        }
                        _ => {
                            if matches!(plan.finish, Finish::Quit) {
                                cx.set_global(QuitApprovals::default());
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        match plan.finish {
            Finish::Tabs => {
                self.discard_drafts(plan.ids.iter().copied(), cx);
                if !self.tabs.is_empty() {
                    self.remove_tabs(&plan.ids, window, cx);
                }
            }
            Finish::Window => {
                self.discard_all_drafts(cx);
                window.remove_window();
            }
            Finish::Quit => {
                // Approved drafts are discarded only when the app actually exits; until then
                // they stay dirty, so a cancelled prompt in another window loses nothing.
                let approved: Vec<_> = self
                    .tabs
                    .iter()
                    .filter_map(|tab| {
                        let edit = tab.edit.as_ref()?;
                        edit.dirty.then_some((tab.view.entity_id(), edit.revision))
                    })
                    .collect();
                cx.default_global::<QuitApprovals>().0.extend(approved);
                cx.defer(continue_quit);
            }
            Finish::Reload => {
                if let Some(id) = plan.ids.first() {
                    self.discard_drafts([*id], cx);
                    self.reload_source(*id, window, cx);
                }
            }
            Finish::Reading => {
                if let Some(id) = plan.ids.first() {
                    self.prepare_reading(*id, PreviewRequest::Reading, window, cx);
                }
            }
        }
    }
}
