//! Asynchronous reader invariants: every request remains scoped to its target.
use super::*;
use gpui_kit::{TestAppContext, WindowOptions};

fn reader(cx: &mut TestAppContext) -> Entity<Reader> {
    cx.update(|cx| {
        gpui_kit::init(cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
        })
        .unwrap()
        .1
    })
}

#[gpui_kit::test]
fn concurrent_folder_expansion_keeps_both_results(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
        std::fs::write(dir.path().join(name).join("child.md"), "# Child").unwrap();
    }
    let reader = reader(cx);
    reader.update(cx, |reader, cx| {
        reader.open_folder(dir.path().to_path_buf(), cx)
    });
    super::tests::settle(cx);
    // Both requests are started before either completion can update the UI.
    reader.update(cx, |reader, cx| {
        reader.select_entry(0, cx);
        reader.select_entry(1, cx);
    });
    super::tests::settle(cx);
    reader.read_with(cx, |reader, _| {
        for name in ["a", "b"] {
            assert!(
                reader
                    .tree
                    .iter()
                    .any(|e| e.path == dir.path().join(name).join("child.md")),
                "expanding a sibling must not cancel {name}"
            );
        }
    });
}

#[gpui_kit::test]
fn background_reload_does_not_retarget_active_watcher(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    std::fs::write(&a, "# Old A").unwrap();
    std::fs::write(&b, "# B").unwrap();
    let reader = reader(cx);
    for path in [&a, &b] {
        reader.update(cx, |reader, cx| reader.open_file(path.clone(), cx));
        super::tests::settle(cx);
    }
    std::fs::write(&a, "# New A").unwrap();
    reader.update(cx, |reader, cx| {
        let index = reader
            .tabs
            .iter()
            .position(|t| t.path.as_ref() == Some(&a))
            .unwrap();
        reader.activate(index, cx);
        reader.reload(cx);
        let index = reader
            .tabs
            .iter()
            .position(|t| t.path.as_ref() == Some(&b))
            .unwrap();
        reader.activate(index, cx);
    });
    super::tests::settle(cx);
    reader.read_with(cx, |reader, cx| {
        assert_eq!(reader.tabs[reader.active].path.as_ref(), Some(&b));
        assert_eq!(
            reader.watcher.as_ref().map(|w| &w.0),
            Some(&b),
            "completion of A must leave B's watcher active"
        );
        let a = reader
            .tabs
            .iter()
            .find(|t| t.path.as_ref() == Some(&a))
            .unwrap();
        assert_eq!(a.view.read(cx).document.source(), "# New A");
    });
}

#[gpui_kit::test]
fn watcher_poll_uses_the_current_tabs_signal(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let reader = reader(cx);
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    std::fs::write(&a, "# A").unwrap();
    std::fs::write(&b, "# B").unwrap();
    reader.update(cx, |r, cx| r.open_file(a.clone(), cx));
    super::tests::settle(cx);
    let old_signal = reader.read_with(cx, |r, _| r.changed.clone());
    reader.update(cx, |r, cx| r.open_file(b.clone(), cx));
    super::tests::settle(cx);
    // Deliver a queued notification from the old watcher after the tab switch.
    reader.read_with(cx, |r, _| r.changed.store(false, Ordering::Relaxed));
    old_signal.store(true, Ordering::Relaxed);
    let revision = reader.read_with(cx, |r, _| r.load_revision);
    cx.executor().advance_clock(Duration::from_millis(450));
    super::tests::settle(cx);
    assert_eq!(reader.read_with(cx, |r, _| r.load_revision), revision);
    std::fs::write(&b, "# Updated B").unwrap();
    reader.read_with(cx, |r, _| r.changed.store(true, Ordering::Relaxed));
    cx.executor().advance_clock(Duration::from_millis(450));
    super::tests::settle(cx);
    reader.read_with(cx, |r, cx| {
        assert_eq!(
            r.tabs[r.active].view.read(cx).document.source(),
            "# Updated B"
        )
    });
}

#[gpui_kit::test]
fn workspace_budget_rejects_expansion_without_losing_existing_rows(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("folder")).unwrap();
    for name in ["a.md", "b.md"] {
        std::fs::write(dir.path().join("folder").join(name), "text").unwrap();
    }
    let reader = reader(cx);
    reader.update(cx, |r, cx| r.open_folder(dir.path().to_path_buf(), cx));
    super::tests::settle(cx);
    reader.update(cx, |r, cx| {
        for i in 1..workspace::MAX_TREE_ENTRIES {
            r.tree.push(Entry {
                path: dir.path().join(format!("{i}.md")),
                name: format!("{i}.md"),
                directory: false,
                depth: 0,
            });
        }
        r.select_entry(0, cx);
    });
    super::tests::settle(cx);
    reader.update(cx, |r, cx| {
        assert_eq!(r.tree.len(), workspace::MAX_TREE_ENTRIES);
        assert!(!r.expanded.contains(&dir.path().join("folder")));
        assert!(r.tree_jobs.is_empty());
        r.tree.truncate(1);
        r.select_entry(0, cx);
    });
    super::tests::settle(cx);
    reader.read_with(cx, |r, _| {
        assert_eq!(r.tree.len(), 3, "retry after freeing room must work")
    });
}

#[gpui_kit::test]
fn oversized_reload_keeps_original_document_and_other_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.md");
    std::fs::write(&path, "# Original").unwrap();
    let reader = reader(cx);
    reader.update(cx, |r, cx| r.open_file(path.clone(), cx));
    super::tests::settle(cx);
    let original = reader.read_with(cx, |r, _| r.tabs[r.active].view.clone());
    // Two independently addressable tabs share immutable fixture bytes to keep the
    // regression itself lightweight. Admission accounts for each open tab's source.
    let large = Arc::new(
        Document::parse(format!("~~~text\n{}\n~~~", "x".repeat(31 * 1024 * 1024))).unwrap(),
    );
    reader.update(cx, |r, cx| {
        for name in ["other-a", "other-b"] {
            let view = cx.new(|cx| {
                DocumentView::new(
                    large.clone(),
                    dir.path().to_path_buf(),
                    r.queue.clone(),
                    r.math.clone(),
                    cx,
                )
            });
            r.tabs.push(Tab {
                path: Some(dir.path().join(name)),
                identity: None,
                title: name.into(),
                view,
                resources: None,
                edit: None,
            });
        }
    });
    std::fs::write(
        &path,
        format!("~~~text\n{}\n~~~", "x".repeat(3 * 1024 * 1024)),
    )
    .unwrap();
    reader.update(cx, |r, cx| r.reload(cx));
    super::tests::settle(cx);
    reader.read_with(cx, |r, cx| {
        assert_eq!(r.tabs.len(), 4, "reload must not evict unrelated tabs");
        assert_eq!(
            r.tabs[r.active].view, original,
            "failed admission keeps the original view"
        );
        assert_eq!(original.read(cx).document.source(), "# Original");
        assert!(r.status.contains("64 MiB"));
    });
}
