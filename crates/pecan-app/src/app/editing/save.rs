//! Save operations keep source snapshots and imported resources alive until commit.
use super::{ClosePlan, Reader, SaveMode};
use crate::reading::DocumentView;
use gpui_kit::{AppContext as _, Context, EntityId, Window, component::input::Rope};
use pecan_document::Document;
use std::{path::PathBuf, sync::Arc};

struct Saved {
    path: PathBuf,
    identity: PathBuf,
    document: Arc<Document>,
    resources: Option<Arc<pecan_import::Resources>>,
}

impl Reader {
    pub(in crate::app) fn save_active(
        &mut self,
        mode: SaveMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.ensure_editor(window, cx) else {
            return;
        };
        self.save_source(id, mode, None, window, cx);
    }

    pub(super) fn save_source(
        &mut self,
        id: EntityId,
        mode: SaveMode,
        continuation: Option<ClosePlan>,
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
        let Some(edit) = tab.edit.as_mut() else {
            return;
        };
        let save_as = matches!(mode, SaveMode::As)
            || tab
                .path
                .as_ref()
                .is_none_or(|path| !pecan_import::is_markdown(path));
        let original_path = tab.path.clone();
        let previous = edit.saved.clone();
        let source = edit.editor.read(cx).text().clone();
        let revision = edit.revision;
        let mut snapshot = pecan_export::Snapshot {
            document: previous.clone(),
            directory: tab.view.read(cx).directory.clone(),
            title: tab.title.clone(),
            preferences: self.presented.clone(),
            codes: Default::default(),
            diagrams: self.queue.clone(),
            math: self.math.rendering.clone(),
            resources: tab.resources.clone(),
        };
        let directory = original_path
            .as_ref()
            .and_then(|path| path.parent())
            .map_or_else(
                || std::env::home_dir().unwrap_or_default(),
                std::path::Path::to_path_buf,
            );
        let name = format!(
            "{}.md",
            std::path::Path::new(&tab.title)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        );
        let selection = save_as.then(|| cx.prompt_for_new_path(&directory, Some(&name)));
        edit.set_busy(true, cx);
        edit.comparison = None;
        let client = cx.http_client();
        self.status = "正在保存源码…".into();
        self.edit_job = Some(cx.spawn_in(window, async move |weak, cx| {
            let result: anyhow::Result<Option<Saved>> = async {
                let path = if let Some(selection) = selection {
                    let Some(mut path) = selection.await?? else {
                        return Ok(None);
                    };
                    if path.extension().is_none() {
                        path.set_extension("md");
                    }
                    anyhow::ensure!(
                        pecan_import::is_markdown(&path),
                        "源码请保存为 .md、.markdown 或 .mdown 文件"
                    );
                    let checked = path.clone();
                    let identity = cx
                        .background_spawn(async move { std::fs::canonicalize(checked).ok() })
                        .await;
                    let duplicate = weak.update(cx, |reader, _| {
                        reader.tabs.iter().any(|tab| {
                            tab.view.entity_id() != id
                                && (tab.path.as_ref() == Some(&path)
                                    || identity.as_ref().is_some_and(|identity| {
                                        tab.identity.as_ref() == Some(identity)
                                    }))
                        })
                    })?;
                    anyhow::ensure!(!duplicate, "此文件已在其他标签打开，请选择其他名称");
                    path
                } else {
                    original_path.expect("existing Markdown has a path")
                };
                cx.background_spawn(async move {
                    let document = Arc::new(Document::parse(source.to_string())?);
                    if save_as {
                        snapshot.document = document;
                        pecan_export::write_document(
                            snapshot,
                            pecan_export::DocumentFormat::Markdown,
                            &path,
                            client,
                        )
                        .await?;
                        let loaded = pecan_import::load(&path)?;
                        Ok(Some(Saved {
                            identity: std::fs::canonicalize(&path)?,
                            path,
                            document: Arc::new(loaded.document),
                            resources: Some(Arc::new(loaded.resources)),
                        }))
                    } else {
                        let identity =
                            pecan_edit::save(&path, previous.source(), document.source())?;
                        Ok(Some(Saved {
                            path,
                            identity,
                            document,
                            resources: None,
                        }))
                    }
                })
                .await
            }
            .await;
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
                let mut completed = false;
                match result {
                    Ok(Some(Saved {
                        path,
                        identity,
                        document,
                        resources,
                    })) => {
                        // The editor is read-only during save, with a revision guard for queued edits.
                        if revision == edit.revision {
                            if save_as {
                                edit.editor.update(cx, |editor, cx| {
                                    editor.set_value(document.source().to_owned(), window, cx)
                                });
                            }
                            edit.dirty = false;
                            edit.saved_revision = Some(revision);
                            edit.preview_revision = revision;
                            completed = true;
                        }
                        edit.saved_text = Rope::from(document.source());
                        edit.saved = document.clone();
                        let directory = resources.as_ref().map_or_else(
                            || tab.view.read(cx).directory.to_path_buf(),
                            |resources| resources.directory().to_path_buf(),
                        );
                        tab.path = Some(path.clone());
                        tab.identity = Some(identity.clone());
                        tab.title = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        if let Some(resources) = resources {
                            tab.resources = Some(resources);
                        }
                        tab.view.update(cx, |view, cx| {
                            *view = DocumentView::new(
                                document,
                                directory,
                                reader.queue.clone(),
                                reader.math.clone(),
                                cx,
                            )
                        });
                        if index != reader.active || edit.show_source {
                            tab.view.update(cx, |view, cx| view.suspend(cx));
                        }
                        reader.status = format!("已保存 · {}", path.display());
                        if index == reader.active {
                            reader.watch(&path);
                        }
                        reader.record_recent(identity, crate::recent::Kind::File, cx);
                    }
                    Ok(None) => reader.status = "已取消保存，草稿已保留".into(),
                    Err(error) => reader.status = format!("保存失败：{error:#}；草稿已保留"),
                }
                cx.notify();
                if completed && let Some(plan) = continuation {
                    reader.request_close(plan, window, cx);
                }
            });
        }));
        cx.notify();
    }
}
