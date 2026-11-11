use super::*;
use gpui_kit::{EntityInputHandler as _, TestAppContext, VisualTestContext, px, size};
use std::path::PathBuf;

fn open(cx: &mut TestAppContext, path: Option<PathBuf>) -> (AnyWindowHandle, Entity<Reader>) {
    let (handle, reader) = cx.update(|cx| {
        gpui_kit::init(cx);
        crate::menus::install(cx);
        cx.set_reduce_motion(true);
        gpui_kit::open_window(
            crate::native_window_options("源码验收", cx, size(px(1280.), px(860.))),
            cx,
            |window, cx| {
                cx.new(|cx| Reader::new(path, pecan_config::startup::Launch::First, window, cx))
            },
        )
        .unwrap()
    });
    super::super::tests::settle(cx);
    (handle, reader)
}

fn replace(reader: &Entity<Reader>, text: &str, window: &mut Window, cx: &mut App) {
    let editor = reader.read(cx).tabs[reader.read(cx).active]
        .edit
        .as_ref()
        .unwrap()
        .editor
        .clone();
    editor.update(cx, |editor, cx| {
        // UTF-16 ranges are supplied by the native input handler.
        let end = editor.text().to_string().encode_utf16().count();
        editor.replace_text_in_range(Some(0..end), text, window, cx);
    });
}

fn text(reader: &Entity<Reader>, cx: &App) -> String {
    reader.read(cx).tabs[reader.read(cx).active]
        .edit
        .as_ref()
        .unwrap()
        .editor
        .read(cx)
        .text()
        .to_string()
}

#[gpui_kit::test]
fn editing_preview_saving_and_reopening_preserve_exact_markdown(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.md");
    std::fs::write(&path, "# Before\n").unwrap();
    let source = "# 新标题\n\n**粗体**、~~删除~~ 与 `inline`。\n\n```json\n{\"keep\":  2}\n```\n";
    let (handle, reader) = open(cx, Some(path.clone()));
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.update(|window, cx| reader.update(cx, |reader, cx| reader.toggle_source(window, cx)));
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("source-editor-pane").is_some());
    visual.update(|window, cx| replace(&reader, source, window, cx));
    visual.run_until_parked();
    visual.update(|window, cx| {
        reader.update(cx, |reader, cx| {
            assert!(reader.tabs[reader.active].protected());
            reader.toggle_source(window, cx);
        })
    });
    super::super::tests::settle(&visual.cx);
    visual.update(|window, cx| {
        let state = reader.read(cx);
        let tab = &state.tabs[state.active];
        assert!(!tab.source_mode());
        assert_eq!(tab.view.read(cx).document.headings()[0].title(), "新标题");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# Before\n",
            "preview does not save"
        );
        reader.update(cx, |reader, cx| {
            reader.save_active(SaveMode::Existing, window, cx)
        });
    });
    super::super::tests::settle(&visual.cx);
    visual.update(|window, cx| {
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        let state = reader.read(cx);
        assert!(!state.tabs[state.active].protected());
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        assert_eq!(text(&reader, cx), source);
        window.draw(cx).clear(cx);
    });
    assert!(visual.debug_bounds("source-editor-pane").is_some());
}

#[gpui_kit::test]
fn top_toolbar_tracks_real_save_success_and_reload_keeps_the_unsaved_guard(
    cx: &mut TestAppContext,
) {
    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let point = visual.debug_bounds(selector).expect(selector).center();
        visual.simulate_mouse_down(point, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_up(point, gpui_kit::MouseButton::Left, Default::default());
        visual.run_until_parked();
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("toolbar.md");
    std::fs::write(&path, "# Original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    let toolbar = visual.debug_bounds("reader-titlebar").unwrap();
    let toggle = visual.debug_bounds("toggle-source").unwrap();
    assert!(toolbar.contains(&toggle.center()));
    assert!(visual.debug_bounds("source-save").is_none());
    assert!(visual.debug_bounds("source-saved").is_none());
    click(&mut visual, "toggle-source");
    assert!(reader.read_with(&visual.cx, |r, _| r.tabs[r.active].source_mode()));
    visual.update(|window, cx| replace(&reader, "# Edited", window, cx));
    super::super::tests::settle(&visual.cx);
    click(&mut visual, "source-save");
    super::super::tests::settle(&visual.cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Edited");
    let saved = visual.debug_bounds("source-saved").unwrap();
    assert!(toolbar.contains(&saved.center()));
    assert!(visual.debug_bounds("source-save").is_none());
    click(&mut visual, "toggle-source");
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        visual.debug_bounds("source-saved").is_some(),
        "saved status follows the tab into reading"
    );
    click(&mut visual, "toggle-source");
    visual.update(|window, cx| replace(&reader, "# Another edit", window, cx));
    super::super::tests::settle(&visual.cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(visual.debug_bounds("source-saved").is_none());
    assert!(visual.debug_bounds("source-save").is_some());
    std::fs::write(&path, "# External").unwrap();
    click(&mut visual, "source-save");
    super::super::tests::settle(&visual.cx);
    visual.update(|window, cx| window.draw(cx).clear(cx));
    assert!(
        visual.debug_bounds("source-saved").is_none(),
        "a failed save cannot show success"
    );
    assert!(visual.debug_bounds("source-save").is_some());
    click(&mut visual, "titlebar-reload");
    assert!(visual.cx.has_pending_prompt());
    visual.cx.simulate_prompt_answer("取消");
    super::super::tests::settle(&visual.cx);
    reader.read_with(&visual.cx, |_, cx| {
        assert_eq!(text(&reader, cx), "# Another edit")
    });
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# External");
}

#[gpui_kit::test]
fn source_reload_preserves_editor_mode_and_refreshes_the_save_baseline(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reload.md");
    std::fs::write(&path, "# Original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    let (id, editor) = cx
        .update_window(handle, |_, window, cx| {
            reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
            let state = reader.read(cx);
            let tab = &state.tabs[state.active];
            (
                tab.view.entity_id(),
                tab.edit.as_ref().unwrap().editor.clone(),
            )
        })
        .unwrap();
    // Reload must clamp the old caret to a UTF-8 boundary in a shorter document.
    editor.update(cx, |editor, cx| editor.set_selected_range(4..4, cx));
    std::fs::write(&path, "中文").unwrap();
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.reload_action(window, cx));
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        let tab = &state.tabs[state.active];
        assert!(tab.source_mode(), "reload must stay in the source editor");
        assert_eq!(tab.view.entity_id(), id);
        let edit = tab.edit.as_ref().unwrap();
        assert_eq!(edit.editor, editor);
        assert_eq!(edit.editor.read(cx).selected_range(), 3..3);
        assert_eq!(text(&reader, cx), "中文");
        assert_eq!(tab.view.read(cx).document.source(), "中文");
        assert!(!tab.protected());
        assert!(!edit.preview_pending());
        assert!(!edit.save_confirmed());
    });
    cx.update_window(handle, |_, window, cx| {
        replace(&reader, "# After reload", window, cx)
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.save_active(SaveMode::Existing, window, cx)
        });
    })
    .unwrap();
    super::super::tests::settle(cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# After reload");
    reader.read_with(cx, |state, _| {
        assert!(
            state.tabs[state.active]
                .edit
                .as_ref()
                .unwrap()
                .save_confirmed()
        );
    });
}

#[gpui_kit::test]
fn source_reload_discard_failure_preserves_draft_and_success_resets_it(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reload.md");
    std::fs::write(&path, "# Original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Draft", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    for invalid in [true, false] {
        if invalid {
            std::fs::write(&path, [0xff, 0xfe]).unwrap();
        } else {
            std::fs::write(&path, "# External").unwrap();
        }
        cx.update_window(handle, |_, window, cx| {
            reader.update(cx, |reader, cx| reader.reload_action(window, cx));
        })
        .unwrap();
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("不保存");
        super::super::tests::settle(cx);
        reader.read_with(cx, |state, cx| {
            let tab = &state.tabs[state.active];
            assert!(tab.source_mode());
            let edit = tab.edit.as_ref().unwrap();
            assert!(!edit.busy);
            assert_eq!(edit.dirty, invalid);
            assert_eq!(
                text(&reader, cx),
                if invalid { "# Draft" } else { "# External" }
            );
            assert_eq!(
                edit.saved.source(),
                if invalid { "# Original" } else { "# External" }
            );
            if invalid {
                assert!(state.status.contains("读取失败"), "{}", state.status);
            }
        });
    }
}

#[gpui_kit::test]
fn source_reload_save_choice_and_reading_mode_are_preserved(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("reload.md");
    std::fs::write(&path, "# Original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Saved draft", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.reload_action(window, cx));
    })
    .unwrap();
    cx.simulate_prompt_answer("保存");
    super::super::tests::settle(cx);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Saved draft");
    reader.read_with(cx, |state, cx| {
        assert!(state.tabs[state.active].source_mode());
        assert!(!state.tabs[state.active].protected());
        assert_eq!(text(&reader, cx), "# Saved draft");
    });
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
    })
    .unwrap();
    std::fs::write(&path, "# Reading refresh").unwrap();
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.reload_action(window, cx));
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        let tab = &state.tabs[state.active];
        assert!(!tab.source_mode(), "reading mode must also stay unchanged");
        assert_eq!(tab.view.read(cx).document.source(), "# Reading refresh");
        assert_eq!(text(&reader, cx), "# Reading refresh");
    });
}

#[gpui_kit::test]
fn source_reload_completion_does_not_activate_a_background_tab(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.md");
    let b = directory.path().join("b.md");
    std::fs::write(&a, "# Original A").unwrap();
    std::fs::write(&b, "# B").unwrap();
    let (handle, reader) = open(cx, Some(b.clone()));
    reader.update(cx, |reader, cx| reader.open_file(a.clone(), cx));
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
    })
    .unwrap();
    std::fs::write(&a, "# Updated A").unwrap();
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.reload_action(window, cx);
            assert!(reader.tabs[reader.active].edit.as_ref().unwrap().busy);
            let index = reader
                .tabs
                .iter()
                .position(|tab| tab.path.as_ref() == Some(&b))
                .unwrap();
            reader.activate(index, cx);
            reader.focus_document(window, cx);
        });
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        let state = reader.read(cx);
        assert_eq!(state.tabs[state.active].path.as_ref(), Some(&b));
        assert_eq!(state.watcher.as_ref().map(|w| &w.0), Some(&b));
        assert!(state.focus.is_focused(window));
        let tab = state
            .tabs
            .iter()
            .find(|tab| tab.path.as_ref() == Some(&a))
            .unwrap();
        assert!(tab.source_mode());
        assert!(!tab.protected());
        assert_eq!(
            tab.edit
                .as_ref()
                .unwrap()
                .editor
                .read(cx)
                .text()
                .to_string(),
            "# Updated A"
        );
        assert_eq!(tab.view.read(cx).document.source(), "# Updated A");
    })
    .unwrap();
}

#[gpui_kit::test]
fn source_outline_uses_current_offsets_and_reports_deleted_headings(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("outline.md");
    std::fs::write(&path, "# 开头\n\n## 同名\n\n## 同名\n\n## 删除\n").unwrap();
    let (handle, reader) = open(cx, Some(path));
    let source = "# 开头\n\n新插入的中文内容\n\n## 同名\n\n更多内容\n\n## 同名\n";
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, source, window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.navigate_heading(3, window, cx));
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, _| {
        assert!(state.tabs[state.active].source_mode());
        assert!(state.status.contains("标题已被修改或删除"));
    });
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.navigate_heading(2, window, cx));
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        let tab = &state.tabs[state.active];
        let position = source.rfind("## 同名").unwrap();
        assert_eq!(
            tab.edit.as_ref().unwrap().editor.read(cx).selected_range(),
            position..position
        );
        assert_eq!(tab.view.read(cx).selected_heading, Some(2));
        assert!(tab.source_mode());
    });
}

#[gpui_kit::test]
fn failed_save_and_reload_cancellation_preserve_the_draft(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("conflict.md");
    std::fs::write(&path, "# Original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Draft", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    std::fs::write(&path, "# External").unwrap();
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.close_tab(reader.active, window, cx));
    })
    .unwrap();
    cx.simulate_prompt_answer("保存");
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(text(&reader, cx), "# Draft");
        assert!(state.tabs[state.active].protected());
    });
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.reload_action(window, cx));
    })
    .unwrap();
    cx.simulate_prompt_answer("取消");
    super::super::tests::settle(cx);
    reader.read_with(cx, |_, cx| assert_eq!(text(&reader, cx), "# Draft"));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "# External");
}

#[gpui_kit::test]
fn external_changes_cannot_replace_a_draft_or_be_silently_overwritten(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.md");
    std::fs::write(&path, "# Before").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Draft", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    std::fs::write(&path, "# External").unwrap();
    reader.update(cx, |reader, cx| reader.reload(cx));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.save_active(SaveMode::Existing, window, cx)
        })
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        assert!(state.status.contains("其他程序修改"), "{}", state.status);
        assert!(state.tabs[state.active].protected());
        assert_eq!(text(&reader, cx), "# Draft");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "# External");
    });
}

#[gpui_kit::test]
fn close_cancel_preserves_draft_and_save_closes_only_after_success(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.md");
    std::fs::write(&path, "original").unwrap();
    let (handle, reader) = open(cx, Some(path.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# 保存后关闭", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.close_tab(reader.active, window, cx))
    })
    .unwrap();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        assert_eq!(state.tabs.len(), 2);
        assert_eq!(text(&reader, cx), "# 保存后关闭");
    });
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.close_tab(reader.active, window, cx))
    })
    .unwrap();
    cx.simulate_prompt_answer("保存");
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, _| assert_eq!(state.tabs.len(), 1));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# 保存后关闭");
}

#[gpui_kit::test]
fn imported_source_saves_as_portable_markdown_and_cancellation_keeps_the_original(
    cx: &mut TestAppContext,
) {
    let source = std::path::absolute(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pecan-import/tests/fixtures/rich.docx"),
    )
    .unwrap();
    let original = std::fs::read(&source).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("converted.md");
    let (handle, reader) = open(cx, Some(source.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.toggle_source(window, cx);
            reader.save_active(SaveMode::Existing, window, cx);
        })
    })
    .unwrap();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, _| {
        assert_eq!(state.tabs[state.active].path.as_ref(), Some(&source))
    });
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.save_active(SaveMode::Existing, window, cx)
        })
    })
    .unwrap();
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        let tab = &state.tabs[state.active];
        assert_eq!(tab.path.as_ref(), Some(&destination), "{}", state.status);
        assert!(!tab.protected());
        let saved = std::fs::read_to_string(&destination).unwrap();
        assert_eq!(text(&reader, cx), saved);
        assert!(saved.contains("pecan-assets-"));
        assert_eq!(
            tab.resources.as_ref().unwrap().kind(),
            pecan_import::SourceKind::Markdown
        );
    });
    assert_eq!(std::fs::read(source).unwrap(), original);
}

#[gpui_kit::test]
fn source_sessions_follow_tabs_and_undo_can_return_to_the_saved_state(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("a.md");
    let b = directory.path().join("b.md");
    std::fs::write(&a, "# A").unwrap();
    std::fs::write(&b, "# B").unwrap();
    let (handle, reader) = open(cx, Some(a.clone()));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Edited", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    reader.update(cx, |reader, cx| reader.open_file(b, cx));
    super::super::tests::settle(cx);
    reader.update(cx, |reader, cx| reader.open_file(a, cx));
    super::super::tests::settle(cx);
    let mut visual = VisualTestContext::from_window(handle, cx);
    visual.update(|window, cx| {
        assert_eq!(text(&reader, cx), "# Edited");
        let state = reader.read(cx);
        let editor = state.tabs[state.active]
            .edit
            .as_ref()
            .unwrap()
            .editor
            .clone();
        editor.focus_handle(cx).focus(window, cx);
        window.draw(cx).clear(cx);
        window.dispatch_action(Box::new(gpui_kit::component::input::Undo), cx);
    });
    super::super::tests::settle(&visual.cx);
    visual.executor().advance_clock(Duration::from_millis(300));
    super::super::tests::settle(&visual.cx);
    visual.update(|_, cx| {
        assert_eq!(text(&reader, cx), "# A");
        assert!(!reader.read(cx).tabs[reader.read(cx).active].protected());
    });
}

#[gpui_kit::test]
fn window_close_and_quit_share_the_unsaved_guard(cx: &mut TestAppContext) {
    let (handle, reader) = open(cx, None);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Unsaved", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    assert!(cx.update(|cx| request_close_window(handle, cx)));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    super::super::tests::settle(cx);
    cx.update(request_quit);
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("取消");
    super::super::tests::settle(cx);
    reader.read_with(cx, |state, cx| {
        assert_eq!(text(&reader, cx), "# Unsaved");
        assert!(state.tabs[state.active].protected());
    });
}

#[gpui_kit::test]
fn oversized_paste_is_rejected_before_it_enters_the_buffer(cx: &mut TestAppContext) {
    let (handle, reader) = open(cx, None);
    let mut visual = VisualTestContext::from_window(handle, cx);
    let original = visual.update(|window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        window.draw(cx).clear(cx);
        text(&reader, cx)
    });
    visual.update(|window, cx| {
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
            "x".repeat(pecan_document::MAX_DOCUMENT_BYTES + 1),
        ));
        window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
    });
    visual.run_until_parked();
    visual.update(|_, cx| {
        assert_eq!(text(&reader, cx), original);
        assert!(reader.read(cx).status.contains("粘贴超过"));
        cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new()));
    });
}

#[gpui_kit::test]
fn large_source_input_reuses_the_reading_snapshot_and_editor(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.md");
    let line = "中英文 source text **emphasis** and `inline code`.\n\n";
    let source = format!("# Large source\n\n{}", line.repeat(20_000));
    std::fs::write(&path, &source).unwrap();
    let (handle, reader) = open(cx, Some(path));
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        let tab = &reader.read(cx).tabs[reader.read(cx).active];
        let snapshot = tab.view.read(cx).document.clone();
        let editor = tab.edit.as_ref().unwrap().editor.clone();
        let first_layout = std::time::Instant::now();
        window.draw(cx).clear(cx);
        eprintln!("source first layout: {:?}", first_layout.elapsed());
        let started = std::time::Instant::now();
        editor.update(cx, |editor, cx| {
            editor.replace_text_in_range(Some(0..0), "追加内容\n\n", window, cx);
        });
        window.draw(cx).clear(cx);
        eprintln!(
            "source: {} bytes; single edit and native layout: {:?}",
            source.len(),
            started.elapsed()
        );
        let tab = &reader.read(cx).tabs[reader.read(cx).active];
        assert!(Arc::ptr_eq(&snapshot, &tab.view.read(cx).document));
        assert_eq!(tab.edit.as_ref().unwrap().editor, editor);
        assert_eq!(
            editor.read(cx).text().len(),
            source.len() + "追加内容\n\n".len()
        );
        assert_eq!(snapshot.source(), source);
    })
    .unwrap();
}

#[gpui_kit::test]
fn dirty_export_uses_the_latest_editor_text(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("latest.html");
    let (handle, reader) = open(cx, None);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
        replace(&reader, "# Latest editor text\n\n**当前草稿**", window, cx);
    })
    .unwrap();
    super::super::tests::settle(cx);
    cx.update_window(handle, |_, window, cx| {
        reader.update(cx, |reader, cx| {
            reader.export_document(pecan_export::DocumentFormat::Html, window, cx);
        })
    })
    .unwrap();
    super::super::tests::settle(cx);
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| Some(destination.clone()));
    super::super::tests::settle(cx);
    let html = std::fs::read_to_string(destination).unwrap();
    assert!(html.contains("Latest editor text") && html.contains("<strong>当前草稿</strong>"));
    reader.read_with(cx, |state, _| {
        assert!(state.tabs[state.active].source_mode());
        assert!(
            state.tabs[state.active].protected(),
            "export does not mark the draft saved"
        );
    });
}

#[test]
#[cfg(target_os = "macos")]
#[ignore = "Requires a Metal graphical session"]
fn source_editor_native_frames_have_readable_highlights_in_both_themes() {
    use gpui_kit::HeadlessAppContext;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("源码编辑验收.md");
    // No fenced code: its colors must not conceal missing Markdown highlights.
    let source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../samples/markdown-typography.md"
    ));
    std::fs::write(&path, source).unwrap();
    let mut headless = HeadlessAppContext::with_platform(
        Arc::new(gpui_apple::AppleTextSystem::new()),
        Arc::new(crate::assets::ReaderAssets),
        gpui_platform::current_headless_renderer,
    );
    headless.allow_parking();
    let (handle, reader) = headless.update(|cx| {
        gpui_kit::init(cx);
        crate::fonts::install(cx);
        gpui_kit::open_window(
            crate::native_window_options("源码编辑", cx, size(px(1280.), px(860.))),
            cx,
            |window, cx| {
                cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
            },
        )
        .unwrap()
    });
    headless
        .update_window(handle, |_, window, cx| {
            reader.update(cx, |reader, cx| {
                let tab = &mut reader.tabs[reader.active];
                tab.title = "源码编辑验收.md".into();
                tab.path = Some(path.clone());
                tab.view.update(cx, |view, _| {
                    view.document = Arc::new(Document::parse(source.to_owned()).unwrap());
                });
                reader.toggle_source(window, cx);
            });
            replace(&reader, &format!("{source}\n"), window, cx);
        })
        .unwrap();
    headless.run_until_parked();
    headless
        .update_window(handle, |_, window, cx| {
            reader.update(cx, |reader, cx| {
                reader.save_active(SaveMode::Existing, window, cx);
                let edit = reader.tabs[reader.active].edit.as_ref().unwrap();
                edit.editor
                    .update(cx, |editor, cx| editor.set_selected_range(0..0, cx));
            });
        })
        .unwrap();
    for theme in ["light", "dark"] {
        headless.update(|cx| {
            let prefs = crate::preferences::Preferences::shared(cx);
            prefs.update(cx, |prefs, cx| {
                prefs.change(|data| data.theme = theme.into(), cx)
            });
        });
        for _ in 0..6 {
            headless.run_until_parked();
            headless.advance_clock(Duration::from_millis(200));
            headless
                .update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
        let image = headless
            .update_window(handle, |_, window, _| window.render_to_image().unwrap())
            .unwrap();
        headless.update(|cx| {
            let state = reader.read(cx);
            assert!(
                state.tabs[state.active]
                    .edit
                    .as_ref()
                    .unwrap()
                    .save_confirmed(),
                "{}",
                state.status
            );
        });
        let green = image
            .rows()
            .take(68)
            .flatten()
            .filter(|pixel| {
                let [r, g, b, _] = pixel.0;
                g > r.saturating_add(25) && g > b.saturating_add(15)
            })
            .count();
        assert!(
            green > 20,
            "{theme} must show the green saved icon in the top toolbar: {green}"
        );
        let colors = image
            .rows()
            .skip(68)
            .flatten()
            .filter(|pixel| {
                let [r, g, b, _] = pixel.0;
                r.abs_diff(g).max(g.abs_diff(b)).max(r.abs_diff(b)) > 45
            })
            .count();
        assert!(
            colors > 300,
            "{theme} source frame must highlight Markdown without fenced code: {colors}"
        );
        if let Some(directory) = std::env::var_os("PECAN_METAL_OUTPUT") {
            image
                .save(PathBuf::from(directory).join(format!("source-{theme}.png")))
                .unwrap();
        }
        headless
            .update_window(handle, |_, window, cx| {
                reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
            })
            .unwrap();
        for _ in 0..4 {
            headless.run_until_parked();
            headless
                .update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
                .unwrap();
        }
        if let Some(directory) = std::env::var_os("PECAN_METAL_OUTPUT") {
            headless
                .capture_screenshot(handle)
                .unwrap()
                .save(PathBuf::from(directory).join(format!("reading-{theme}.png")))
                .unwrap();
        }
        headless
            .update_window(handle, |_, window, cx| {
                reader.update(cx, |reader, cx| reader.toggle_source(window, cx));
            })
            .unwrap();
    }
}
