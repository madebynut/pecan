//! Save operations keep source snapshots and imported resources alive until commit.
use super::{ClosePlan, Reader, SaveMode};
use crate::reading::DocumentView;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EntityId, EntityInputHandler as _, Window,
    component::input::{EditorState, Rope},
};
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
        let editor = self
            .tabs
            .iter()
            .find(|tab| tab.view.entity_id() == id)
            .and_then(|tab| tab.edit.as_ref())
            .map(|edit| edit.editor.clone());
        // The trim emits a change. Defer the write until that revision is recorded,
        // or the save would miss it and leave the cleaned draft looking unsaved.
        if let Some(editor) = editor
            && trim_editor_trailing_whitespace(&editor, window, cx)
        {
            let weak = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                let _ = weak.update(cx, |reader, cx| {
                    reader.persist_source(id, mode, continuation, window, cx);
                });
            });
            return;
        }
        self.persist_source(id, mode, continuation, window, cx);
    }

    fn persist_source(
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
                        .background_spawn(async move {
                            pecan_document::files::canonicalize(&checked).ok()
                        })
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
                            identity: pecan_document::files::canonicalize(&path)?,
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

/// Drop trailing spaces and tabs on save, the way Cursor's `trimTrailingWhitespace` does.
///
/// Whitespace the caret is still inside is kept, so saving does not eat the end of the line
/// being edited. A line of text ending in two or more spaces is a Markdown hard line break,
/// so its trailing whitespace is kept as written. `caret` is a UTF-8 byte offset. Returns the cleaned text and the deleted
/// byte ranges in the original.
fn trim_trailing_whitespace(text: &str, caret: usize) -> Option<Trimmed> {
    let mut cleaned = String::with_capacity(text.len());
    let mut deletions = Vec::new();
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let line_start = index;
        while index < bytes.len() && bytes[index] != b'\n' {
            index += 1;
        }
        let mut body_end = index;
        if body_end > line_start && bytes[body_end - 1] == b'\r' {
            body_end -= 1;
        }
        let body = &text[line_start..body_end];
        let trimmed = body.trim_end_matches([' ', '\t']);
        // Two or more trailing spaces after text are a Markdown hard line break.
        let content_len = if !trimmed.is_empty() && body.ends_with("  ") {
            body.len()
        } else {
            trimmed.len()
        };
        let caret_on_line = caret >= line_start && caret <= body_end;
        let keep = if caret_on_line && caret > line_start + content_len {
            caret - line_start
        } else {
            content_len
        };
        cleaned.push_str(&body[..keep]);
        if keep < body.len() {
            deletions.push((line_start + keep, body_end));
        }
        cleaned.push_str(&text[body_end..index]);
        if index < bytes.len() {
            cleaned.push('\n');
            index += 1;
        }
    }
    (!deletions.is_empty()).then_some(Trimmed { cleaned, deletions })
}

struct Trimmed {
    cleaned: String,
    deletions: Vec<(usize, usize)>,
}

impl Trimmed {
    fn map_offset(&self, offset: usize) -> usize {
        let mut shift = 0usize;
        for &(start, end) in &self.deletions {
            if offset <= start {
                break;
            }
            if offset < end {
                return start - shift;
            }
            shift += end - start;
        }
        offset.saturating_sub(shift)
    }
}

fn trim_editor_trailing_whitespace(
    editor: &Entity<EditorState>,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    editor.update(cx, |editor, cx| {
        let current = editor.text().to_string();
        let selection = editor.selected_range();
        let reversed = editor.is_selection_reversed();
        let caret = if reversed {
            selection.start
        } else {
            selection.end
        };
        let Some(trimmed) = trim_trailing_whitespace(&current, caret) else {
            return false;
        };
        let start = trimmed.map_offset(selection.start);
        let end = trimmed.map_offset(selection.end);
        let utf16_len = current.encode_utf16().count();
        editor.replace_text_in_range(Some(0..utf16_len), &trimmed.cleaned, window, cx);
        editor.set_selected_range_with_direction(start..end, reversed, cx);
        true
    })
}

#[cfg(test)]
mod tests {
    use super::trim_trailing_whitespace;

    #[test]
    fn trailing_spaces_and_tabs_are_removed_except_around_the_caret() {
        let trimmed = trim_trailing_whitespace("alpha \nbeta\t \n\t \nend ", 0).unwrap();
        assert_eq!(trimmed.cleaned, "alpha\nbeta\n\nend");
        assert_eq!(trimmed.map_offset(0), 0);
        assert_eq!(trimmed.map_offset(6), 5);

        let kept = trim_trailing_whitespace("alpha \nbeta\t \n", 6).unwrap();
        assert_eq!(kept.cleaned, "alpha \nbeta\n");
        assert!(trim_trailing_whitespace("alpha \nbeta\n", 6).is_none());

        let partial = trim_trailing_whitespace("alpha \t \r\n", 6).unwrap();
        assert_eq!(partial.cleaned, "alpha \r\n");
        assert_eq!(partial.map_offset(6), 6);
        assert_eq!(partial.map_offset(8), 6);
    }

    #[test]
    fn markdown_hard_breaks_survive_the_save_trim() {
        assert!(trim_trailing_whitespace("first  \nsecond   \r\nthird\t  \nlast", 0).is_none());
        let trimmed = trim_trailing_whitespace("first  \n   \nsecond \n", 0).unwrap();
        assert_eq!(trimmed.cleaned, "first  \n\nsecond\n");
    }
}
