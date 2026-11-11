mod exports;
mod history;
mod tabs;

use gpui_kit::component::StyledExt as _;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use gpui_kit::{
    AnyElement, Context, Entity, FocusHandle, Focusable, IntoElement, PathPromptOptions, Render,
    Subscription, Task, UniformListScrollHandle, Window,
    component::{
        ActiveTheme, IconName, Selectable, Sizable, TitleBar,
        button::{Button, ButtonVariants},
        h_flex,
        input::{Input, InputEvent, InputState},
        v_flex,
    },
    div,
    prelude::*,
    px, uniform_list,
};
use notify::{RecursiveMode, Watcher as _};
use pecan_document::{Document, SearchMatch, SearchQuery};

use crate::{
    CloseTab, CopyDocument, CopyFilePath, DecreaseText, ExportHtml, ExportMarkdown, ExportPdf,
    ExportWord, Find, FocusReading, Help, IncreaseText, NextTab, Open, OpenFolder, OriginalPdf,
    PreviousTab, Reload, ResetText, RevealFile, ShowSettings, ToggleFullscreen, ToggleOutline,
    ToggleSidebar, ToggleTheme,
    diagram::RenderQueue,
    math::MathQueue,
    preferences::{Preferences, PreferencesData},
    reading::DocumentView,
    workspace::{self, Entry},
};

/// Limits independent source buffers, in addition to bounded per-document UI caches.
const MAX_TABS: usize = 8;
const MAX_OPEN_BYTES: usize = 64 * 1024 * 1024;

struct Tab {
    path: Option<PathBuf>,
    identity: Option<PathBuf>,
    title: String,
    view: Entity<DocumentView>,
    resources: Option<Arc<pecan_import::Resources>>,
}

pub(crate) struct Reader {
    focus: FocusHandle,
    tabs: Vec<Tab>,
    active: usize,
    queue: RenderQueue,
    math: MathQueue,
    root: Option<PathBuf>,
    tree: Vec<Entry>,
    expanded: HashSet<PathBuf>,
    tree_scroll: UniformListScrollHandle,
    sidebar: bool,
    outline: bool,
    sidebar_width: f32,
    outline_width: f32,
    pane_drag: Option<crate::panes::Drag>,
    focus_mode: bool,
    prefs: Entity<Preferences>,
    recent: Entity<crate::recent::Recent>,
    presented: Arc<PreferencesData>,
    presented_dark: bool,
    search_open: bool,
    search: Entity<InputState>,
    matches: Arc<[SearchMatch]>,
    match_index: usize,
    status: String,
    load_revision: u64,
    search_revision: u64,
    folder_revision: u64,
    _subscriptions: Vec<Subscription>,
    load_job: Option<Task<()>>,
    search_job: Option<Task<()>>,
    tree_job: Option<Task<()>>,
    folder_job: Option<Task<()>>,
    watcher: Option<(PathBuf, notify::RecommendedWatcher)>,
    changed: Arc<AtomicBool>,
    _watch_task: Task<()>,
}

/// The bundled guide is immutable and references no local files; reopened Help tabs share it.
fn welcome_view(
    queue: &RenderQueue,
    math: &MathQueue,
    cx: &mut Context<Reader>,
) -> Entity<DocumentView> {
    static WELCOME: std::sync::OnceLock<Arc<Document>> = std::sync::OnceLock::new();
    let document = WELCOME
        .get_or_init(|| {
            Arc::new(
                Document::parse(include_str!("../../../samples/welcome.md").into())
                    .expect("bundled welcome document fits the document budget"),
            )
        })
        .clone();
    cx.new(|cx| DocumentView::new(document, PathBuf::new(), queue.clone(), math.clone(), cx))
}

impl Reader {
    pub(crate) fn new(
        initial: Option<PathBuf>,
        launch: pecan_config::startup::Launch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prefs = Preferences::shared(cx);
        let presented = prefs.read(cx).data.clone();
        let pref_subscription = cx.observe(&prefs, |reader, prefs, cx| {
            let data = prefs.read(cx).data.clone();
            let rebuild = !reader.presented.same_presentation(&data);
            reader.presented = data;
            if rebuild {
                for tab in &reader.tabs {
                    tab.view.update(cx, |view, cx| view.reset_presentation(cx));
                }
            }
            cx.notify();
        });
        let theme_subscription = cx.observe_global::<gpui_kit::component::Theme>(|reader, cx| {
            let dark = cx.theme().is_dark();
            if reader.presented_dark != dark {
                reader.presented_dark = dark;
                for tab in &reader.tabs {
                    tab.view.update(cx, |view, cx| view.reset_presentation(cx));
                }
                cx.notify();
            }
        });
        let bounds_subscription = cx.observe_window_bounds(window, |_, _, cx| cx.notify());
        let appearance_subscription = cx.observe_window_appearance(window, |reader, _, cx| {
            if reader.prefs.read(cx).data.theme == "system" {
                let data = reader.prefs.read(cx).data.clone();
                crate::preferences::apply_theme(&data, cx);
            }
        });
        let queue = RenderQueue::new();
        let math = MathQueue::new();
        let tabs = if launch == pecan_config::startup::Launch::First {
            vec![Tab {
                path: None,
                identity: None,
                title: "欢迎使用碧果".into(),
                view: welcome_view(&queue, &math, cx),
                resources: None,
            }]
        } else {
            Vec::new()
        };
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索正文，Enter 下一个结果"));
        let subscription = cx.subscribe(&search, |reader, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                reader.start_search(cx);
            } else if let InputEvent::PressEnter { shift, .. } = event {
                reader.step_match(*shift, cx);
            }
        });
        let changed = Arc::new(AtomicBool::new(false));
        let poll_flag = changed.clone();
        let watch_task = cx.spawn(async move |weak, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(400))
                    .await;
                if poll_flag.swap(false, Ordering::Relaxed) {
                    let _ = weak.update(cx, |reader, cx| {
                        if reader.prefs.read(cx).data.auto_reload {
                            reader.reload(cx);
                        }
                    });
                }
                if weak.upgrade().is_none() {
                    break;
                }
            }
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut reader = Self {
            focus,
            tabs,
            active: 0,
            queue,
            math,
            root: None,
            tree: Vec::new(),
            expanded: HashSet::new(),
            tree_scroll: UniformListScrollHandle::new(),
            sidebar_width: crate::panes::DEFAULT_WIDTH,
            outline_width: crate::panes::DEFAULT_WIDTH,
            pane_drag: None,
            sidebar: true,
            outline: true,
            focus_mode: false,
            prefs,
            recent: crate::recent::Recent::shared(cx),
            presented,
            presented_dark: cx.theme().is_dark(),
            search_open: false,
            search,
            matches: Arc::default(),
            match_index: 0,
            status: "就绪 · 本地文件 · 原生图表".into(),
            load_revision: 0,
            search_revision: 0,
            folder_revision: 0,
            _subscriptions: vec![
                subscription,
                pref_subscription,
                theme_subscription,
                appearance_subscription,
                bounds_subscription,
            ],
            load_job: None,
            search_job: None,
            tree_job: None,
            folder_job: None,
            watcher: None,
            changed,
            _watch_task: watch_task,
        };
        if let Some(path) = initial {
            if path.is_dir() {
                reader.open_folder(path, cx);
            } else {
                reader.open_file(path, cx);
            }
        }
        reader
    }
    fn dialog(&mut self, target: PathKind, cx: &mut Context<Self>) {
        let selection = cx.prompt_for_paths(PathPromptOptions {
            files: target == PathKind::File,
            directories: target == PathKind::Folder,
            multiple: false,
            prompt: Some("打开".into()),
        });
        cx.spawn(async move |weak, cx| match selection.await {
            Ok(Ok(Some(paths))) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = weak.update(cx, |reader, cx| match target {
                        PathKind::File => reader.open_file(path, cx),
                        PathKind::Folder => reader.open_folder(path, cx),
                    });
                }
            }
            Ok(Err(error)) => {
                let _ = weak.update(cx, |reader, cx| {
                    reader.status = format!("打开失败：{error}");
                    cx.notify();
                });
            }
            _ => {}
        })
        .detach();
    }
    fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        if let Some(index) = self.tabs.iter().position(|tab| {
            tab.path.as_ref() == Some(&path) || tab.identity.as_ref() == Some(&path)
        }) {
            self.load_revision += 1;
            self.load_job = None;
            let identity = self.tabs[index].identity.clone().unwrap_or(path);
            self.record_recent(identity, crate::recent::Kind::File, cx);
            self.status = format!("已打开 {}", self.tabs[index].title);
            self.activate(index, cx);
            return;
        }
        self.load(path, LoadMode::NewTab, cx);
    }
    fn load(&mut self, path: PathBuf, mode: LoadMode, cx: &mut Context<Self>) {
        self.load_revision += 1;
        let revision = self.load_revision;
        self.status = format!(
            "正在读取 {}…",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        cx.notify();
        let worker_path = path.clone();
        let background = cx.background_spawn(async move {
            let _permit = workspace::CONVERSION_SLOTS.acquire().await;
            let identity = std::fs::canonicalize(&worker_path)?;
            workspace::load(&worker_path).map(|loaded| (identity, loaded))
        });
        self.load_job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |reader, cx| {
                if reader.load_revision != revision {
                    return;
                }
                match result {
                    Ok((identity, loaded)) => {
                        if mode == LoadMode::NewTab {
                            reader.record_recent(identity.clone(), crate::recent::Kind::File, cx);
                            if let Some(index) = reader
                                .tabs
                                .iter()
                                .position(|tab| tab.identity.as_ref() == Some(&identity))
                            {
                                reader.status = format!("已打开 {}", reader.tabs[index].title);
                                reader.activate(index, cx);
                                return;
                            }
                        }
                        let resources = Arc::new(loaded.resources);
                        let notice = resources.notice().map(str::to_owned);
                        let document = Arc::new(loaded.document);
                        let bytes = document.source().len();
                        let directory = resources.directory().to_path_buf();
                        let view = cx.new(|cx| {
                            DocumentView::new(
                                document.clone(),
                                directory.clone(),
                                reader.queue.clone(),
                                reader.math.clone(),
                                cx,
                            )
                        });
                        let title = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        if mode == LoadMode::Replace {
                            if let Some(tab) = reader
                                .tabs
                                .iter_mut()
                                .find(|tab| tab.path.as_ref() == Some(&path))
                            {
                                let offset = tab.view.read(cx).scroll.logical_scroll_top();
                                view.read(cx).scroll.scroll_to(offset);
                                tab.view = view;
                                tab.resources = Some(resources);
                            }
                        } else {
                            if let Some(tab) = reader.tabs.get(reader.active) {
                                tab.view.update(cx, |view, cx| view.suspend(cx));
                            }
                            while reader.tabs.len() >= MAX_TABS
                                || reader
                                    .tabs
                                    .iter()
                                    .map(|tab| tab.view.read(cx).document.source().len())
                                    .sum::<usize>()
                                    + bytes
                                    > MAX_OPEN_BYTES
                            {
                                if reader.tabs.is_empty() {
                                    break;
                                }
                                reader.tabs.remove(0);
                                reader.active = reader.active.saturating_sub(1);
                            }
                            reader.tabs.push(Tab {
                                path: Some(path.clone()),
                                identity: Some(identity),
                                title,
                                view,
                                resources: Some(resources),
                            });
                            reader.active = reader.tabs.len() - 1;
                        }
                        reader.status = notice.unwrap_or_else(|| {
                            format!(
                                "{} KiB · {} 阅读块 · {} 个标题",
                                bytes / 1024,
                                document.blocks().len(),
                                document.headings().len()
                            )
                        });
                        reader.watch(&path);
                        if reader.root.is_none() && reader.folder_job.is_none() {
                            reader.set_folder(
                                path.parent()
                                    .unwrap_or_else(|| std::path::Path::new("."))
                                    .to_path_buf(),
                                FolderOpen::DocumentParent,
                                cx,
                            );
                        }
                        if reader.search_open {
                            reader.start_search(cx);
                        }
                    }
                    Err(error) => reader.status = format!("读取失败：{error:#}"),
                }
                cx.notify();
            });
        }));
    }
    fn watch(&mut self, path: &std::path::Path) {
        // Each watcher owns an FSEvents stream and thread; reuse it across tab switches and reloads.
        if self
            .watcher
            .as_ref()
            .is_some_and(|(watched, _)| watched == path)
        {
            return;
        }
        let changed = self.changed.clone();
        let watched = path.to_path_buf();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if let Ok(event) = result
                && (event.kind.is_modify() || event.kind.is_create() || event.kind.is_remove())
            {
                // Editors often save through a temporary file and atomic rename.
                if event.paths.iter().any(|p| p == &watched) {
                    changed.store(true, Ordering::Relaxed);
                }
            }
        });
        match watcher {
            Ok(mut watcher) => {
                if let Some(parent) = path.parent()
                    && watcher.watch(parent, RecursiveMode::NonRecursive).is_ok()
                {
                    self.watcher = Some((path.to_path_buf(), watcher));
                }
            }
            Err(error) => tracing::warn!(%error, "file watcher unavailable"),
        }
    }
    fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.tabs.get(self.active).and_then(|tab| tab.path.clone()) {
            self.load(path, LoadMode::Replace, cx);
        }
    }
    fn open_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.set_folder(path, FolderOpen::Explicit, cx);
    }
    fn set_folder(&mut self, path: PathBuf, source: FolderOpen, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        self.folder_revision += 1;
        let revision = self.folder_revision;
        let worker_path = path.clone();
        let background = cx.background_spawn(async move {
            let identity = std::fs::canonicalize(&worker_path)?;
            workspace::entries(&worker_path, 0).map(|entries| (identity, entries))
        });
        self.folder_job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |reader, cx| {
                if reader.folder_revision != revision {
                    return;
                }
                reader.folder_job = None;
                match result {
                    Ok((identity, entries)) => {
                        reader.root = Some(path);
                        reader.tree = entries;
                        reader.expanded.clear();
                        reader.tree_job = None;
                        reader
                            .tree_scroll
                            .scroll_to_item(0, gpui_kit::ScrollStrategy::Top);
                        if source == FolderOpen::Explicit {
                            reader.sidebar = true;
                            reader.focus_mode = false;
                            reader.record_recent(identity, crate::recent::Kind::Folder, cx);
                        }
                    }
                    Err(error) => reader.status = format!("读取目录失败：{error:#}"),
                }
                cx.notify();
            });
        }));
    }
    fn select_entry(&mut self, index: usize, cx: &mut Context<Self>) {
        let entry = self.tree[index].clone();
        if !entry.directory {
            self.open_file(entry.path, cx);
            return;
        }
        if self.expanded.remove(&entry.path) {
            // Expanded siblings follow the subtree, so depth is not partitioned beyond it.
            let end = self.tree[index + 1..]
                .iter()
                .position(|child| child.depth <= entry.depth)
                .map_or(self.tree.len(), |offset| index + 1 + offset);
            self.tree.drain(index + 1..end);
            self.expanded.retain(|path| !path.starts_with(&entry.path));
            cx.notify();
            return;
        }
        self.expanded.insert(entry.path.clone());
        let parent = entry.path.clone();
        let root = self.root.clone();
        let background =
            cx.background_spawn(async move { workspace::entries(&entry.path, entry.depth + 1) });
        self.tree_job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |reader, cx| {
                if reader.root != root || !reader.expanded.contains(&parent) {
                    return;
                }
                if let Some(index) = reader.tree.iter().position(|entry| entry.path == parent) {
                    match result {
                        Ok(entries) => {
                            reader.tree.splice(index + 1..index + 1, entries);
                        }
                        Err(error) => {
                            reader.expanded.remove(&parent);
                            reader.status = format!("读取目录失败：{error:#}");
                        }
                    }
                }
                cx.notify();
            });
        }));
    }
    fn start_search(&mut self, cx: &mut Context<Self>) {
        self.search_revision += 1;
        let revision = self.search_revision;
        let query = self.search.read(cx).value().to_string();
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        let document = tab.view.read(cx).document.clone();
        let view = tab.view.clone();
        let executor = cx.background_executor().clone();
        let background = cx.background_spawn(async move {
            executor.timer(Duration::from_millis(180)).await;
            SearchQuery::new(&query).map_or_else(Vec::new, |query| query.find(&document))
        });
        self.search_job = Some(cx.spawn(async move |weak, cx| {
            let matches: Arc<[SearchMatch]> = background.await.into();
            let _ = weak.update(cx, |reader, cx| {
                if reader.search_revision != revision {
                    return;
                }
                view.update(cx, |view, cx| view.search(matches.clone(), cx));
                reader.matches = matches;
                reader.match_index = 0;
                if !reader.matches.is_empty() {
                    reader.reveal_match(cx);
                }
                cx.notify();
            });
        }));
    }
    fn reveal_match(&mut self, cx: &mut Context<Self>) {
        if let Some(found) = self.matches.get(self.match_index)
            && let Some(tab) = self.tabs.get(self.active)
        {
            tab.view.update(cx, |view, cx| {
                view.reveal(found.block_index(), found.source_range(), cx)
            });
        }
    }
    fn step_match(&mut self, backward: bool, cx: &mut Context<Self>) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.match_index = (self.match_index + if backward { count - 1 } else { 1 }) % count;
        self.reveal_match(cx);
        cx.notify();
    }
    fn render_search_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .debug_selector(|| "document-search".into())
            .w_full()
            .flex_shrink_0()
            .px_4()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Input::new(&self.search)
                    .cleanable(true)
                    .flex_1()
                    .min_w_0()
                    .max_w(px(400.)),
            )
            .child(
                div()
                    .text_size(px(self.presented.ui_size as f32))
                    .child(format!(
                        "{} / {}",
                        if self.matches.is_empty() {
                            0
                        } else {
                            self.match_index + 1
                        },
                        self.matches.len()
                    )),
            )
            .child(
                Button::new("previous-match")
                    .small()
                    .ghost()
                    .label("上一处")
                    .on_click(cx.listener(|reader, _, _, cx| reader.step_match(true, cx))),
            )
            .child(
                Button::new("next-match")
                    .small()
                    .ghost()
                    .label("下一处")
                    .on_click(cx.listener(|reader, _, _, cx| reader.step_match(false, cx))),
            )
            .child(div().flex_1())
            .child(
                Button::new("close-search")
                    .small()
                    .ghost()
                    .label("关闭搜索")
                    .on_click(cx.listener(|reader, _, _, cx| {
                        reader.search_open = false;
                        reader.matches = Arc::default();
                        reader.search_revision += 1;
                        reader.search_job = None;
                        if let Some(tab) = reader.tabs.get(reader.active) {
                            tab.view
                                .update(cx, |view, cx| view.search(Arc::default(), cx));
                        }
                        cx.notify();
                    })),
            )
    }
    fn search_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.search_open = true;
        self.search.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }
    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar = self.focus_mode || !self.sidebar;
        self.focus_mode = false;
        cx.notify();
    }
    fn toggle_outline(&mut self, cx: &mut Context<Self>) {
        self.outline = self.focus_mode || !self.outline;
        self.focus_mode = false;
        cx.notify();
    }
    fn focus_reading(&mut self, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        cx.notify();
    }
    fn pane_maximum(&self, side: crate::panes::Side, window: &Window) -> f32 {
        let opposite = match side {
            crate::panes::Side::Left if self.outline => self.outline_width,
            crate::panes::Side::Right if self.sidebar => self.sidebar_width,
            _ => 0.,
        };
        crate::panes::maximum(f32::from(window.viewport_size().width), opposite)
    }
    fn pane_handle(
        &self,
        side: crate::panes::Side,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::panes::Side;
        let width = match side {
            Side::Left => self.sidebar.then_some(self.sidebar_width),
            Side::Right => self.outline.then_some(self.outline_width),
        };
        let cursor = side.cursor(width, self.pane_maximum(side, window));
        div()
            .id(match side {
                Side::Left => "left-pane-handle",
                Side::Right => "right-pane-handle",
            })
            .debug_selector(move || match side {
                Side::Left => "left-pane-handle".into(),
                Side::Right => "right-pane-handle".into(),
            })
            .absolute()
            .top_0()
            .h_full()
            .w(px(8.))
            .map(|handle| match side {
                Side::Left => handle.left(px(width.map_or(0., |w| w - 4.))),
                Side::Right => handle.right(px(width.map_or(0., |w| w - 4.))),
            })
            .cursor(cursor)
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(move |reader, event: &gpui_kit::MouseDownEvent, _, cx| {
                    reader.pane_drag = Some(crate::panes::Drag::new(
                        side,
                        event.position.x.into(),
                        width.unwrap_or(0.),
                    ));
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .into_any_element()
    }
    fn drag_pane(
        &mut self,
        event: &gpui_kit::MouseMoveEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let Some(side) = self.pane_drag.as_ref().map(|drag| drag.side) else {
            return;
        };
        if event.pressed_button != Some(gpui_kit::MouseButton::Left) {
            self.pane_drag = None;
            cx.notify();
            return;
        }
        let max = self.pane_maximum(side, window);
        let width = self
            .pane_drag
            .as_mut()
            .expect("active drag has a side")
            .advance(event.position.x.into(), max);
        let (visible, size) = match side {
            crate::panes::Side::Left => (&mut self.sidebar, &mut self.sidebar_width),
            crate::panes::Side::Right => (&mut self.outline, &mut self.outline_width),
        };
        let changed =
            *visible != width.is_some() || width.is_some_and(|width| (*size - width).abs() > 0.1);
        *visible = width.is_some();
        if let Some(width) = width {
            *size = width;
        }
        if changed {
            cx.notify();
        }
    }
    fn resize_text(&mut self, delta: f64, cx: &mut Context<Self>) {
        self.prefs.update(cx, |prefs, cx| {
            prefs.change(|data| data.font_size += delta, cx)
        });
    }
    fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        let theme = if cx.theme().is_dark() {
            "light"
        } else {
            "dark"
        };
        self.prefs.update(cx, |prefs, cx| {
            prefs.change(|data| data.theme = theme.into(), cx)
        });
    }
    fn help(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.path.is_none()) {
            self.activate(index, cx);
            return;
        }
        let view = welcome_view(&self.queue, &self.math, cx);
        if self.tabs.len() >= MAX_TABS {
            self.tabs.remove(0);
            self.active = self.active.saturating_sub(1);
        }
        self.tabs.push(Tab {
            path: None,
            identity: None,
            title: "碧果使用指南".into(),
            view,
            resources: None,
        });
        self.activate(self.tabs.len() - 1, cx);
    }
    fn show_original_pdf(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(self.active).and_then(|tab| {
            tab.resources
                .as_ref()
                .filter(|resources| resources.kind() == pecan_import::SourceKind::Pdf)?;
            tab.path.clone()
        }) else {
            return;
        };
        let weak = cx.entity().downgrade();
        cx.defer(move |cx| {
            if let Err(error) = crate::pdf::open(&path, cx) {
                let _ = weak.update(cx, |reader, cx| {
                    reader.status = format!("原版查看失败：{error:#}");
                    cx.notify();
                });
            }
        });
    }
    fn render_titlebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        // Center against the window, independently of asymmetric native controls and tools.
        let title_width =
            (window.viewport_size().width - px(crate::platform::choose(640., 844.))).max(px(0.));
        div()
            .relative()
            .w_full()
            .flex_shrink_0()
            .child(
                TitleBar::new()
                    .pl(crate::platform::titlebar_inset(window.is_fullscreen()))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .pr_2()
                            .items_center()
                            .child(
                                div()
                                    .debug_selector(|| "titlebar-sidebar-toggle".into())
                                    .child(
                                        Button::new("toggle-sidebar")
                                            .small()
                                            .ghost()
                                            .child(crate::ui::icon(
                                                if self.sidebar && !self.focus_mode {
                                                    IconName::PanelLeftClose
                                                } else {
                                                    IconName::PanelLeftOpen
                                                },
                                            ))
                                            .tooltip(crate::platform::choose(
                                                "展开／收起文件栏 · ⌘B",
                                                "展开／收起文件栏 · Ctrl+B",
                                            ))
                                            .on_click(cx.listener(|reader, _, _, cx| {
                                                reader.toggle_sidebar(cx);
                                            })),
                                    ),
                            )
                            .child(div().flex_1())
                            .child(
                                Button::new("open-file")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::FileText))
                                    .tooltip(crate::platform::choose(
                                        "打开文档 · ⌘O",
                                        "打开文档 · Ctrl+O",
                                    ))
                                    .on_click(cx.listener(|reader, _, _, cx| {
                                        reader.dialog(PathKind::File, cx)
                                    })),
                            )
                            .child(
                                Button::new("open-folder")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::FolderOpen))
                                    .tooltip(crate::platform::choose(
                                        "打开文件夹 · ⇧⌘O",
                                        "打开文件夹 · Ctrl+Shift+O",
                                    ))
                                    .on_click(cx.listener(|reader, _, _, cx| {
                                        reader.dialog(PathKind::Folder, cx)
                                    })),
                            )
                            .child(
                                Button::new("find")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::Search))
                                    .tooltip(crate::platform::choose("搜索 · ⌘F", "搜索 · Ctrl+F"))
                                    .on_click(cx.listener(|reader, _, window, cx| {
                                        reader.search_focus(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("reload")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::RefreshCw))
                                    .tooltip(crate::platform::choose(
                                        "重新加载 · ⌘R",
                                        "重新加载 · Ctrl+R",
                                    ))
                                    .on_click(cx.listener(|reader, _, _, cx| reader.reload(cx))),
                            )
                            .child(
                                Button::new("reading-focus")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::BookOpen))
                                    .tooltip(crate::platform::choose(
                                        "专注阅读 · ⇧⌘F",
                                        "专注阅读 · Ctrl+Shift+F",
                                    ))
                                    .selected(self.focus_mode)
                                    .on_click(
                                        cx.listener(|reader, _, _, cx| reader.focus_reading(cx)),
                                    ),
                            )
                            .child(
                                Button::new("text-smaller")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::Minus))
                                    .tooltip(crate::platform::choose(
                                        "减小正文字号 · ⌘−",
                                        "减小正文字号 · Ctrl+−",
                                    ))
                                    .on_click(
                                        cx.listener(|reader, _, _, cx| reader.resize_text(-1., cx)),
                                    ),
                            )
                            .child(
                                Button::new("text-larger")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::Plus))
                                    .tooltip(crate::platform::choose(
                                        "增大正文字号 · ⌘+",
                                        "增大正文字号 · Ctrl++",
                                    ))
                                    .on_click(
                                        cx.listener(|reader, _, _, cx| reader.resize_text(1., cx)),
                                    ),
                            )
                            .child(
                                Button::new("theme")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(if cx.theme().is_dark() {
                                        IconName::Sun
                                    } else {
                                        IconName::Moon
                                    }))
                                    .tooltip("切换主题")
                                    .on_click(
                                        cx.listener(|reader, _, _, cx| reader.toggle_theme(cx)),
                                    ),
                            )
                            .child(
                                Button::new("fullscreen")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::Maximize))
                                    .tooltip(crate::platform::choose("全屏 · ⌃⌘F", "全屏 · F11"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(ToggleFullscreen), cx)
                                    }),
                            )
                            .child(
                                Button::new("settings")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(IconName::Settings))
                                    .tooltip(crate::platform::choose("设置 · ⌘,", "设置 · Ctrl+,"))
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(ShowSettings), cx)
                                    }),
                            )
                            .child(
                                Button::new("toggle-outline")
                                    .small()
                                    .ghost()
                                    .child(crate::ui::icon(if self.outline && !self.focus_mode {
                                        IconName::PanelRightClose
                                    } else {
                                        IconName::PanelRightOpen
                                    }))
                                    .tooltip(crate::platform::choose(
                                        "展开／收起大纲 · ⌘L",
                                        "展开／收起大纲 · Ctrl+L",
                                    ))
                                    .on_click(cx.listener(|reader, _, _, cx| {
                                        reader.toggle_outline(cx);
                                    })),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(34.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .debug_selector(|| "window-title".into())
                            .w(title_width)
                            .text_center()
                            .text_size(px(self.presented.ui_size as f32))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .truncate()
                            .child(
                                self.tabs
                                    .get(self.active)
                                    .map_or("碧果", |tab| &tab.title)
                                    .to_string(),
                            ),
                    ),
            )
            .into_any_element()
    }
    fn render_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let root_name = self
            .root
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or("工作区".into(), |name| {
                name.to_string_lossy().into_owned()
            });
        v_flex()
            .id("file-sidebar")
            .debug_selector(|| "file-sidebar".into())
            .w(px(self.sidebar_width))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .child(
                        div()
                            .text_size(px((self.presented.ui_size - 1.) as f32))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(root_name),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("open-workspace")
                            .small()
                            .ghost()
                            .child(crate::ui::icon(IconName::Plus))
                            .on_click(
                                cx.listener(|reader, _, _, cx| reader.dialog(PathKind::Folder, cx)),
                            ),
                    ),
            )
            .child(
                uniform_list(
                    "workspace-list",
                    self.tree.len(),
                    cx.processor(|reader, range: std::ops::Range<usize>, _, cx| {
                        range
                            .map(|index| {
                                let entry = &reader.tree[index];
                                let active = reader
                                    .tabs
                                    .get(reader.active)
                                    .is_some_and(|tab| tab.path.as_ref() == Some(&entry.path));
                                let icon = if entry.directory {
                                    if reader.expanded.contains(&entry.path) {
                                        IconName::FolderOpen
                                    } else {
                                        IconName::Folder
                                    }
                                } else {
                                    IconName::FileText
                                };
                                crate::ui::sidebar_row()
                                    .id(index)
                                    .debug_selector(move || format!("tree-row-{index}"))
                                    .pl(px(14. + entry.depth as f32 * 14.))
                                    .pr_2()
                                    .gap_2()
                                    .items_center()
                                    .text_size(px(reader.presented.ui_size as f32))
                                    .cursor_pointer()
                                    .when(active, |row| {
                                        row.bg(cx.theme().accent)
                                            .text_color(cx.theme().accent_foreground)
                                    })
                                    .hover(|style| style.bg(cx.theme().muted))
                                    .child(crate::ui::icon(icon))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .child(entry.name.clone()),
                                    )
                                    .on_click(cx.listener(move |reader, _, _, cx| {
                                        reader.select_entry(index, cx)
                                    }))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .track_scroll(&self.tree_scroll)
                .flex_1(),
            )
            .into_any_element()
    }
    fn render_outline(&self, cx: &mut Context<Self>) -> AnyElement {
        let document = self
            .tabs
            .get(self.active)
            .map(|tab| tab.view.read(cx).document.clone());
        v_flex()
            .id("outline-sidebar")
            .debug_selector(|| "outline-sidebar".into())
            .w(px(self.outline_width))
            .h_full()
            .flex_shrink_0()
            .bg(cx.theme().sidebar)
            .border_l_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .px_4()
                    .py_3()
                    .text_size(px((self.presented.ui_size - 1.) as f32))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(cx.theme().muted_foreground)
                    .child("文档大纲"),
            )
            .child(
                uniform_list(
                    "outline-list",
                    document
                        .as_ref()
                        .map_or(0, |document| document.headings().len()),
                    cx.processor(move |reader, range: std::ops::Range<usize>, _, cx| {
                        let Some(document) = document.as_ref() else {
                            return Vec::new();
                        };
                        range
                            .map(|index| {
                                let heading = &document.headings()[index];
                                let selected = reader.tabs.get(reader.active).is_some_and(|tab| {
                                    tab.view.read(cx).selected_heading == Some(index)
                                });
                                crate::ui::sidebar_row()
                                    .id(index)
                                    .debug_selector(move || format!("outline-row-{index}"))
                                    .pl(
                                        px(12.
                                            + f32::from(heading.level().saturating_sub(1)) * 12.),
                                    )
                                    .pr_3()
                                    .text_size(px(reader.presented.ui_size as f32))
                                    .cursor_pointer()
                                    .when(selected, |row| {
                                        row.bg(cx.theme().accent)
                                            .text_color(cx.theme().accent_foreground)
                                    })
                                    .hover(|style| style.bg(cx.theme().muted))
                                    .child(
                                        div()
                                            .debug_selector(move || {
                                                format!("outline-label-{index}")
                                            })
                                            .min_w_0()
                                            .truncate()
                                            .child(heading.title().to_string()),
                                    )
                                    .on_click(cx.listener(move |reader, _, _, cx| {
                                        if let Some(tab) = reader.tabs.get(reader.active) {
                                            tab.view.update(cx, |view, cx| {
                                                view.reveal_heading(index, cx)
                                            });
                                        }
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            })
                            .collect()
                    }),
                )
                .flex_1(),
            )
            .into_any_element()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PathKind {
    File,
    Folder,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadMode {
    NewTab,
    Replace,
}

impl Render for Reader {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sidebar_width = self
            .sidebar_width
            .min(self.pane_maximum(crate::panes::Side::Left, window));
        self.outline_width = self
            .outline_width
            .min(self.pane_maximum(crate::panes::Side::Right, window));
        let active_tab = self.tabs.get(self.active);
        let active_view = active_tab.map(|tab| tab.view.clone());
        let kind = active_tab
            .and_then(|tab| tab.resources.as_ref())
            .map(|resources| resources.kind());
        let is_pdf = kind == Some(pecan_import::SourceKind::Pdf);
        let reading_mode = match kind {
            Some(pecan_import::SourceKind::Pdf) => "PDF · 重排",
            Some(pecan_import::SourceKind::Office) => "文档 · 重排",
            _ if active_tab.is_some() => "UTF-8 · Markdown",
            _ => "",
        };
        let document_body = if let Some(view) = active_view {
            self.document_body(view, cx)
        } else {
            div()
                .debug_selector(|| "empty-reader".into())
                .flex_1()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .debug_selector(|| "empty-logo".into())
                        .opacity(0.2)
                        .child(gpui_kit::img(crate::brand::icon(cx)).size(px(100.))),
                )
                .into_any_element()
        };
        let center = v_flex()
            .id("document-column")
            .debug_selector(|| "document-column".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .when(!self.focus_mode, |column| {
                column.child(
                    h_flex()
                        .id("document-tabs")
                        .debug_selector(|| "document-tabs".into())
                        .w_full()
                        .h(px(34.))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            h_flex()
                                .id("tab-scroll")
                                .flex_1()
                                .min_w_0()
                                .overflow_x_scroll()
                                .children(self.render_tabs(cx)),
                        )
                        .when(is_pdf, |bar| {
                            bar.child(
                                div()
                                    .flex_shrink_0()
                                    .px_2()
                                    .border_l_1()
                                    .border_color(cx.theme().border)
                                    .child(
                                        Button::new("pdf-original")
                                            .small()
                                            .ghost()
                                            .label("原版查看")
                                            .tooltip(crate::platform::choose(
                                                "查看 PDF 原始页面 · ⌥⌘P",
                                                "用默认 PDF 阅读器打开原版 · Ctrl+Alt+P",
                                            ))
                                            .debug_selector(|| "pdf-original".into())
                                            .on_click(cx.listener(|reader, _, _, cx| {
                                                reader.show_original_pdf(cx)
                                            })),
                                    ),
                            )
                        }),
                )
            })
            .when(self.search_open, |column| {
                column.child(self.render_search_bar(cx))
            })
            .child(document_body);
        let body = h_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .when(self.sidebar && !self.focus_mode, |row| {
                row.child(self.render_sidebar(cx))
            })
            .child(center)
            .when(self.outline && !self.focus_mode, |row| {
                row.child(self.render_outline(cx))
            })
            .when(!self.focus_mode, |row| {
                row.child(self.pane_handle(crate::panes::Side::Left, window, cx))
                    .child(self.pane_handle(crate::panes::Side::Right, window, cx))
            });
        let prefs = self.prefs.read(cx).data.clone();
        let interface = crate::presentation::interface_style(cx);
        let drag_cursor = self.pane_drag.as_ref().map(|drag| {
            let width = match drag.side {
                crate::panes::Side::Left => self.sidebar.then_some(self.sidebar_width),
                crate::panes::Side::Right => self.outline.then_some(self.outline_width),
            };
            drag.side
                .cursor(width, self.pane_maximum(drag.side, window))
        });
        v_flex()
            .size_full()
            .relative()
            .refine_style(&interface)
            .text_size(px(prefs.ui_size as f32))
            .key_context("Pecan")
            .track_focus(&self.focus)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_mouse_move(
                cx.listener(|reader, event, window, cx| reader.drag_pane(event, window, cx)),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|reader, _, _, cx| {
                    if reader.pane_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|reader, _, _, cx| {
                    if reader.pane_drag.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .on_action(cx.listener(|reader, _: &Open, _, cx| reader.dialog(PathKind::File, cx)))
            .on_action(cx.listener(|reader, _: &OriginalPdf, _, cx| reader.show_original_pdf(cx)))
            .on_action(
                cx.listener(|reader, _: &OpenFolder, _, cx| reader.dialog(PathKind::Folder, cx)),
            )
            .on_action(
                cx.listener(|reader, action: &crate::recent::OpenRecent, _, cx| {
                    reader.open_recent(action.entry.clone(), cx);
                }),
            )
            .on_action(cx.listener(|reader, _: &crate::MoreRecent, window, cx| {
                reader.more_recent(window, cx)
            }))
            .on_action(cx.listener(|reader, _: &crate::ClearRecent, _, cx| reader.clear_recent(cx)))
            .on_action(cx.listener(|reader, _: &Find, window, cx| reader.search_focus(window, cx)))
            .on_action(cx.listener(|reader, _: &ToggleSidebar, _, cx| {
                reader.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|reader, _: &ToggleOutline, _, cx| {
                reader.toggle_outline(cx);
            }))
            .on_action(cx.listener(|reader, _: &Reload, _, cx| reader.reload(cx)))
            .on_action(cx.listener(|reader, _: &CloseTab, window, cx| {
                if crate::pdf::is_active(cx) {
                    cx.defer(|cx| {
                        crate::pdf::perform(crate::pdf::WindowAction::Close, cx);
                    });
                } else {
                    reader.close_tab(reader.active, window, cx);
                }
            }))
            .on_action(cx.listener(|reader, _: &ExportPdf, _, cx| {
                reader.export_document(crate::export::DocumentFormat::Pdf, cx)
            }))
            .on_action(cx.listener(|reader, _: &ExportWord, _, cx| {
                reader.export_document(crate::export::DocumentFormat::Word, cx)
            }))
            .on_action(cx.listener(|reader, _: &ExportHtml, _, cx| {
                reader.export_document(crate::export::DocumentFormat::Html, cx)
            }))
            .on_action(cx.listener(|reader, _: &ExportMarkdown, _, cx| {
                reader.export_document(crate::export::DocumentFormat::Markdown, cx)
            }))
            .on_action(cx.listener(|reader, _: &CopyFilePath, _, cx| {
                if let Some(path) = reader
                    .tabs
                    .get(reader.active)
                    .and_then(|tab| tab.path.as_ref())
                {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                        path.to_string_lossy().into_owned(),
                    ));
                }
            }))
            .on_action(cx.listener(|reader, _: &CopyDocument, _, cx| {
                if let Some(tab) = reader.tabs.get(reader.active) {
                    cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(
                        tab.view.read(cx).document.source().to_string(),
                    ));
                }
            }))
            .on_action(cx.listener(|reader, _: &RevealFile, _, cx| {
                if let Some(path) = reader
                    .tabs
                    .get(reader.active)
                    .and_then(|tab| tab.path.as_ref())
                {
                    cx.reveal_path(path);
                }
            }))
            .on_action(cx.listener(|reader, _: &FocusReading, _, cx| reader.focus_reading(cx)))
            .on_action(cx.listener(|reader, _: &IncreaseText, _, cx| reader.resize_text(1., cx)))
            .on_action(cx.listener(|reader, _: &DecreaseText, _, cx| reader.resize_text(-1., cx)))
            .on_action(cx.listener(|reader, _: &ResetText, _, cx| {
                reader.prefs.update(cx, |prefs, cx| {
                    prefs.change(
                        |data| data.font_size = PreferencesData::default().font_size,
                        cx,
                    )
                })
            }))
            .on_action(cx.listener(|reader, _: &ToggleTheme, _, cx| reader.toggle_theme(cx)))
            .on_action(cx.listener(|reader, _: &Help, _, cx| reader.help(cx)))
            .on_action(cx.listener(|reader, _: &NextTab, _, cx| {
                if !reader.tabs.is_empty() {
                    reader.activate((reader.active + 1) % reader.tabs.len(), cx);
                }
            }))
            .on_action(cx.listener(|reader, _: &PreviousTab, _, cx| {
                if !reader.tabs.is_empty() {
                    reader.activate(
                        (reader.active + reader.tabs.len() - 1) % reader.tabs.len(),
                        cx,
                    );
                }
            }))
            .child(self.render_titlebar(window, cx))
            .children(crate::menus::bar(cx))
            .child(body)
            .child(
                h_flex()
                    .h(px(24.))
                    .px_4()
                    .items_center()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .debug_selector(|| "reader-status".into())
                    .text_size(px(crate::ui::STATUS_SIZE))
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.status.clone()),
                    )
                    .when(!self.tabs.is_empty(), |bar| {
                        bar.child(
                            div()
                                .flex_shrink_0()
                                .pl_3()
                                .child(format!("{} px · {reading_mode}", prefs.font_size)),
                        )
                    }),
            )
            .when_some(drag_cursor, |root, cursor| {
                root.child(div().absolute().inset_0().cursor(cursor))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Requires a Metal graphical session"]
    fn empty_reader_logo_is_visible_in_the_native_frame() {
        use gpui_kit::{AppContext, HeadlessAppContext, WindowOptions};
        let mut headless = HeadlessAppContext::with_platform(
            Arc::new(gpui_apple::AppleTextSystem::new()),
            Arc::new(crate::assets::ReaderAssets),
            gpui_platform::current_headless_renderer,
        );
        headless.allow_parking();
        let (handle, _) = headless.update(|cx| {
            gpui_kit::init(cx);
            crate::fonts::install(cx);
            Preferences::shared(cx).update(cx, |prefs, cx| {
                prefs.change(|data| data.theme = "light".into(), cx)
            });
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(gpui_kit::WindowBounds::Windowed(gpui_kit::Bounds::new(
                        Default::default(),
                        gpui_kit::size(px(1280.), px(860.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Reader::new(None, pecan_config::startup::Launch::Returning, window, cx)
                    })
                },
            )
            .unwrap()
        });
        headless
            .update_window(handle, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
        let image = headless
            .update_window(handle, |_, window, _| window.render_to_image().unwrap())
            .unwrap();
        if let Ok(output) = std::env::var("PECAN_METAL_OUTPUT") {
            image
                .save(PathBuf::from(output).join("empty-reader.png"))
                .unwrap();
        }
        let visible = (image.height() / 2 - 120..image.height() / 2 + 120)
            .flat_map(|y| (image.width() / 2 - 120..image.width() / 2 + 120).map(move |x| (x, y)))
            .filter(|&(x, y)| image.get_pixel(x, y).0[..3].iter().any(|&v| v < 235))
            .count();
        assert!(
            visible > 500,
            "logo must be painted inside the empty reading area; visible={visible}"
        );
    }
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "Requires a Metal graphical session and the external regression document"]
    fn full_reader_wheel_preserves_code_bottom_with_glyphs_and_adjacent_table() {
        use gpui_kit::{
            AppContext, HeadlessAppContext, InputEvent, ListOffset, MouseButton, MouseDownEvent,
            MouseMoveEvent, MouseUpEvent, ScrollDelta, ScrollEvent, TouchPhase, point,
        };
        use pecan_document::Block;

        let path = std::env::var_os("PECAN_VERIFY_DOCUMENT")
            .map(PathBuf::from)
            .expect("set PECAN_VERIFY_DOCUMENT to a local regression Markdown document");
        let source = std::fs::read_to_string(&path).expect("read supplied regression document");
        let document =
            Arc::new(Document::parse(source).expect("parse supplied regression document"));
        let marker = std::env::var("PECAN_VERIFY_MARKER").unwrap_or_else(|_| "bridgeUrl".into());
        let theme_override = std::env::var("PECAN_VERIFY_THEME").ok();
        let dimension = |name, default| {
            let value = std::env::var(name).map_or(default, |value| {
                value
                    .parse::<f32>()
                    .unwrap_or_else(|error| panic!("{name} must be a number: {error}"))
            });
            assert!(
                value.is_finite() && value > 0.,
                "{name} must be finite and positive"
            );
            value
        };
        let width = dimension("PECAN_VERIFY_WIDTH", 2560.);
        let height = dimension("PECAN_VERIFY_HEIGHT", 1320.);
        // The production loader may migrate and save its input. Isolate the supplied
        // settings so neither migration nor test theme changes can touch that file.
        let preferences_fixture = std::env::var_os("PECAN_VERIFY_PREFERENCES").map(|path| {
            let fixture = tempfile::tempdir().expect("isolate supplied regression preferences");
            std::fs::copy(PathBuf::from(path), fixture.path().join("preferences.json"))
                .expect("copy supplied regression preferences");
            fixture
        });
        let block_ix = document
            .blocks()
            .iter()
            .position(|block| match block {
                Block::CodePage { code, content, .. } if code.start == content.start => {
                    let source = &document.source()[code.clone()];
                    source.contains(&marker)
                }
                _ => false,
            })
            .expect("the requested native code block is present");
        let item_ix = document.blocks()[..block_ix].iter().filter(|block| {
        !matches!(block, Block::CodePage { code, content, .. } if code.start != content.start)
    }).count();
        let directory = path.parent().expect("document parent").to_path_buf();
        let mut headless = HeadlessAppContext::with_platform(
            Arc::new(gpui_apple::AppleTextSystem::new()),
            Arc::new(crate::assets::ReaderAssets),
            gpui_platform::current_headless_renderer,
        );
        headless.allow_parking();
        let (handle, reader, theme) = headless.update(|cx| {
            gpui_kit::init(cx);
            crate::fonts::install(cx);
            // Use production font metrics, scrollbar hover transitions and Reader shell.
            let loaded = preferences_fixture.as_ref().map(|fixture| {
                let loaded =
                    Preferences::install(Some(fixture.path().join("preferences.json")), cx);
                let loaded = loaded.read(cx);
                assert!(
                    loaded.error.is_none(),
                    "supplied preferences failed to load: {:?}",
                    loaded.error
                );
                loaded.data.clone()
            });
            let theme = theme_override.clone().unwrap_or_else(|| {
                loaded
                    .as_ref()
                    .map_or_else(|| "light".into(), |data| data.theme.clone())
            });
            let prefs = Preferences::install(None, cx);
            prefs.update(cx, |prefs, cx| {
                prefs.change(
                    |data| {
                        if let Some(loaded) = loaded {
                            *data = PreferencesData::clone(&loaded);
                        }
                        data.theme = theme.clone();
                    },
                    cx,
                );
            });
            let (handle, reader) = gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(width), px(height)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut reader =
                            Reader::new(None, pecan_config::startup::Launch::First, window, cx);
                        reader.tabs[0].title = "设备定位SDK服务端技术方案.md".into();
                        reader.tabs[0].view = cx.new(|cx| {
                            DocumentView::new(
                                document,
                                directory,
                                reader.queue.clone(),
                                reader.math.clone(),
                                cx,
                            )
                        });
                        reader
                    })
                },
            )
            .expect("open the production Reader shell");
            (handle, reader, theme)
        });
        for _ in 0..4 {
            headless.run_until_parked();
            headless
                .update_window(handle, |_, window, cx| {
                    window.draw(cx).clear(cx);
                })
                .unwrap();
        }
        let code_height = headless
            .update_window(handle, |_, window, cx| {
                let view = reader.read(cx).tabs[0].view.clone();
                view.update(cx, |view, cx| {
                    view.scroll.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: px(0.),
                    });
                    cx.notify();
                });
                window.draw(cx).clear(cx);
                window.draw(cx).clear(cx);
                view.read(cx)
                    .scroll
                    .bounds_for_item(item_ix)
                    .expect("code item is measured")
                    .size
                    .height
            })
            .unwrap();
        let position = headless
            .update_window(handle, |_, window, cx| {
                let view = reader.read(cx).tabs[0].view.clone();
                let viewport = view.read(cx).scroll.viewport_bounds();
                view.update(cx, |view, cx| {
                    view.scroll.scroll_to(ListOffset {
                        item_ix,
                        offset_in_item: code_height - viewport.size.height / 2.,
                    });
                    cx.notify();
                });
                window.draw(cx).clear(cx);
                point(viewport.center().x, viewport.top() + px(80.))
            })
            .unwrap();
        // Publish hover and focus through the same hitboxes used by the native app.
        headless
            .update_window(handle, |_, window, cx| {
                window.dispatch_event(
                    MouseMoveEvent {
                        position,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseDownEvent {
                        position,
                        button: MouseButton::Left,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    MouseUpEvent {
                        position,
                        button: MouseButton::Left,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.dispatch_event(
                    ScrollEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(px(-180.), px(0.))),
                        touch_phase: TouchPhase::Moved,
                        ..Default::default()
                    }
                    .to_platform_input(),
                    cx,
                );
                window.draw(cx).clear(cx);
            })
            .unwrap();

        for (direction, delta_y) in [("down", -7.25), ("up", 7.25)] {
            for tick in 0..24 {
                headless.advance_clock(Duration::from_millis(16));
                // No reveal, scroll_by, cx.notify, or window.refresh from this point onward.
                let (frame, mask, palette, scale, viewport, offset, image) = headless
                    .update_window(handle, |_, window, cx| {
                        window.dispatch_event(
                            ScrollEvent {
                                position,
                                delta: ScrollDelta::Pixels(point(px(0.), px(delta_y))),
                                touch_phase: if tick == 0 {
                                    TouchPhase::Started
                                } else {
                                    TouchPhase::Moved
                                },
                                ..Default::default()
                            }
                            .to_platform_input(),
                            cx,
                        );
                        window.draw(cx).clear(cx);
                        let palette = crate::presentation::ReadingPalette::for_app(cx);
                        let scale = window.scale_factor();
                        let code_frame = window
                            .painted_quads()
                            .into_iter()
                            .find(|quad| {
                                quad.border_color == palette.border
                                    && quad.border_widths.top.0 > 0.
                                    && quad.border_widths.bottom.0 > 0.
                                    && quad.border_widths.left.0 > 0.
                                    && quad.border_widths.right.0 > 0.
                                    && (quad.bounds.size.height.0 - code_height.as_f32() * scale)
                                        .abs()
                                        < 1.
                            })
                            .expect("the native frame is present in the original full scene");
                        let view = reader.read(cx).tabs[0].view.read(cx);
                        (
                            code_frame.bounds,
                            code_frame.content_mask.bounds,
                            palette.border.to_rgb(),
                            scale,
                            view.scroll.viewport_bounds(),
                            view.scroll.logical_scroll_top(),
                            window
                                .render_to_image()
                                .expect("capture complete frame with glyph atlas"),
                        )
                    })
                    .unwrap();
                // The image includes the original Scene and its glyph atlas from this exact frame.
                let bottom = frame.bottom().0;
                assert!(
                    bottom > viewport.top().as_f32() * scale + 16. * scale
                        && bottom < viewport.bottom().as_f32() * scale - 16. * scale,
                    "fixture keeps the actual code bottom inside the document: {frame:?} {viewport:?}"
                );
                let expected =
                    [palette.r, palette.g, palette.b].map(|channel| (channel * 255.).round() as u8);
                let y_start = (bottom.floor() as u32).saturating_sub(3);
                let y_end = (bottom.ceil() as u32 + 2).min(image.height() - 1);
                let mut samples = 0;
                let mut visible = 0;
                for x in ((frame.left().0 + 12. * scale) as u32
                    ..(frame.right().0 - 12. * scale) as u32)
                    .step_by(scale.ceil() as usize)
                {
                    samples += 1;
                    visible += usize::from((y_start..=y_end).any(|y| {
                        image.get_pixel(x, y).0[..3]
                            .iter()
                            .zip(expected)
                            .all(|(&actual, target)| actual.abs_diff(target) <= 8)
                    }));
                }
                let complete = samples > 100 && visible * 100 >= samples * 98;
                if let Ok(output) = std::env::var("PECAN_METAL_OUTPUT")
                    && (tick == 0 || tick == 23 || !complete)
                {
                    image
                        .save(PathBuf::from(output).join(format!(
                            "full-reader-{theme}-{marker}-{direction}-{tick}.png"
                        )))
                        .unwrap();
                }
                assert!(
                    complete,
                    "{direction}/{tick}: border={visible}/{samples}; native frame={frame:?}; mask={mask:?}; offset={offset:?}"
                );
            }
            headless
                .update_window(handle, |_, window, cx| {
                    window.dispatch_event(
                        ScrollEvent {
                            position,
                            touch_phase: TouchPhase::Ended,
                            delta: ScrollDelta::Pixels(point(px(0.), px(0.))),
                            ..Default::default()
                        }
                        .to_platform_input(),
                        cx,
                    );
                    window.draw(cx).clear(cx);
                })
                .unwrap();
        }
    }

    use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};

    #[gpui_kit::test]
    fn outline_labels_center_and_clicks_align_sections_near_top(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("headings.md");
        let source = (0..6)
            .map(|level| {
                format!(
                    "{} Section {level}\n\n{}\n\n",
                    "#".repeat(level + 1),
                    "Paragraph with words.\n\n".repeat(6)
                )
            })
            .collect::<String>();
        std::fs::write(&path, source).unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
                        size(px(1280.), px(860.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Reader::new(Some(path), pecan_config::startup::Launch::First, window, cx)
                    })
                },
            )
            .unwrap()
        });
        settle(cx);
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        let draw = |window: &mut Window, cx: &mut gpui_kit::App| {
            window.refresh();
            window.draw(cx).clear(cx);
        };
        visual.update(draw);
        for (index, row_selector, label_selector) in [
            (0, "outline-row-0", "outline-label-0"),
            (4, "outline-row-4", "outline-label-4"),
            (1, "outline-row-1", "outline-label-1"),
            (5, "outline-row-5", "outline-label-5"),
        ] {
            let row = visual.debug_bounds(row_selector).unwrap();
            let label = visual.debug_bounds(label_selector).unwrap();
            assert!(
                (f32::from(row.center().y - label.center().y)).abs() <= 0.5,
                "outline label {index} centers within the same 32px row as the file sidebar"
            );
            visual.simulate_click(row.center(), Default::default());
            for _ in 0..5 {
                visual.run_until_parked();
                visual.update(draw);
            }
            visual.update(|_, cx| {
                let view = reader.read(cx).tabs[reader.read(cx).active].view.read(cx);
                assert_eq!(view.selected_heading, Some(index));
                let heading = &view.document.headings()[index];
                let viewport = view.scroll.viewport_bounds();
                let item = view.scroll.bounds_for_item(heading.block_index()).unwrap();
                assert!(item.top() >= viewport.top() - px(1.) && item.top() < viewport.top() + px(64.),
                    "heading {index} should be near the top, including the last section: {item:?}, {viewport:?}");
                assert!(view.resident_rows() <= 32, "navigation retains the bounded row cache");
            });
        }
    }

    #[gpui_kit::test]
    fn collapsing_a_folder_removes_only_its_subtree_and_nested_expansion(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let root = fixture.path();
        for dir in ["a/b", "c"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in ["a/b/x.md", "c/y.md"] {
            std::fs::write(root.join(file), "# x\n").unwrap();
        }
        let (_, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Reader::new(None, pecan_config::startup::Launch::First, window, cx))
            })
            .unwrap()
        });
        let names = |cx: &mut TestAppContext| {
            reader.read_with(cx, |reader, _| {
                reader
                    .tree
                    .iter()
                    .map(|entry| format!("{}{}", "  ".repeat(entry.depth), entry.name))
                    .collect::<Vec<_>>()
            })
        };
        let select = |cx: &mut TestAppContext, name: &str| {
            reader.update(cx, |reader, cx| {
                let index = reader.tree.iter().position(|e| e.name == name).unwrap();
                reader.select_entry(index, cx);
            });
            settle(cx);
        };
        reader.update(cx, |reader, cx| reader.open_folder(root.to_path_buf(), cx));
        settle(cx);
        for name in ["a", "b", "c"] {
            select(cx, name);
        }
        assert_eq!(names(cx), ["a", "  b", "    x.md", "c", "  y.md"]);
        select(cx, "a");
        assert_eq!(
            names(cx),
            ["a", "c", "  y.md"],
            "the expanded sibling survives"
        );
        select(cx, "a");
        assert_eq!(
            names(cx),
            ["a", "  b", "c", "  y.md"],
            "nested folders reopen collapsed"
        );
        select(cx, "b");
        assert_eq!(names(cx), ["a", "  b", "    x.md", "c", "  y.md"]);
    }

    #[gpui_kit::test]
    fn short_folder_rows_fill_the_sidebar_after_resizing(cx: &mut TestAppContext) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
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
                reader.tree = vec![
                    Entry {
                        path: "/tmp/a".into(),
                        name: "a".into(),
                        directory: true,
                        depth: 0,
                    },
                    Entry {
                        path: "/tmp/a/b".into(),
                        name: "b".into(),
                        directory: true,
                        depth: 1,
                    },
                ];
                cx.notify();
            })
        });
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        for width in [230., 370., 180.] {
            visual.update(|window, cx| {
                reader.update(cx, |reader, cx| {
                    reader.sidebar_width = width;
                    cx.notify();
                });
                window.refresh();
                window.draw(cx).clear(cx);
            });
            let pane = visual.debug_bounds("file-sidebar").unwrap();
            for selector in ["tree-row-0", "tree-row-1"] {
                let row = visual.debug_bounds(selector).unwrap();
                assert!(
                    (f32::from(row.left() - pane.left())).abs() <= 1.
                        && (f32::from(row.right() - pane.right())).abs() <= 1.,
                    "short nested directory rows fill the pane at width {width}: {row:?}, {pane:?}"
                );
            }
        }
    }

    #[gpui_kit::test]
    fn reader_loads_large_file_and_reloads_saved_content(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("long.md");
        let paragraph = "## 阅读章节\n\n中文长文档 **强调** 与 `code`。\n\n";
        let source = paragraph.repeat(10 * 1024 * 1024 / paragraph.len());
        std::fs::write(&path, &source).unwrap();
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
                        size(px(1280.), px(860.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Reader::new(
                            Some(path.clone()),
                            pecan_config::startup::Launch::First,
                            window,
                            cx,
                        )
                    })
                },
            )
            .unwrap()
        });
        settle(cx);
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            let reader = reader.read(cx);
            let view = reader.tabs[reader.active].view.read(cx);
            assert_eq!(reader.tabs[reader.active].path.as_ref(), Some(&path));
            assert_eq!(view.document.source(), source);
            assert!(view.document.headings().len() > 100_000);
            assert!(view.resident_rows() > 0 && view.resident_rows() <= 32);
        })
        .unwrap();
        settle(cx);

        std::fs::write(&path, "# 已保存的新内容\n\n外部保存后重新加载。\n").unwrap();
        cx.update(|cx| reader.update(cx, |reader, cx| reader.reload(cx)));
        settle(cx);
        cx.update_window(handle, |_, window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
            let reader = reader.read(cx);
            let view = reader.tabs[reader.active].view.read(cx);
            assert_eq!(reader.tabs.len(), 2, "reload must replace the existing tab");
            assert_eq!(view.document.headings()[0].title(), "已保存的新内容");
            assert_eq!(view.document.headings().len(), 1);
            assert!(!reader.status.contains("失败"));
        })
        .unwrap();
        settle(cx);
    }

    #[gpui_kit::test]
    fn imported_tabs_retain_assets_and_keep_pdf_modes_within_the_document_column(
        cx: &mut TestAppContext,
    ) {
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../pecan-import/tests/fixtures");
        let office = fixtures.join("rich.docx");
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
                        size(px(1280.), px(860.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        Reader::new(
                            Some(office.clone()),
                            pecan_config::startup::Launch::First,
                            window,
                            cx,
                        )
                    })
                },
            )
            .unwrap()
        });
        settle(cx);
        let first_assets = cx.update(|cx| {
            let reader = reader.read(cx);
            assert_eq!(
                reader.root.as_deref(),
                office.parent(),
                "workspace uses the source folder"
            );
            let tab = &reader.tabs[reader.active];
            let resources = tab.resources.as_ref().unwrap();
            assert_eq!(resources.kind(), pecan_import::SourceKind::Office);
            assert!(
                resources.directory().exists(),
                "an open tab retains embedded images"
            );
            assert!(tab.view.read(cx).document.source().contains("!["));
            resources.directory().to_path_buf()
        });
        cx.update(|cx| reader.update(cx, |reader, cx| reader.reload(cx)));
        settle(cx);
        assert!(
            !first_assets.exists(),
            "reload releases replaced image resources"
        );
        let current_assets = cx.update(|cx| {
            let reader = reader.read(cx);
            reader.tabs[reader.active]
                .resources
                .as_ref()
                .unwrap()
                .directory()
                .to_path_buf()
        });
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        for name in ["text.pdf", "scan.pdf"] {
            cx.update(|cx| {
                reader.update(cx, |reader, cx| reader.open_file(fixtures.join(name), cx))
            });
            settle(cx);
            for width in [1280., 760.] {
                visual.simulate_resize(size(px(width), px(600.)));
                visual.update(|window, cx| window.draw(cx).clear(cx));
                let button = visual.debug_bounds("pdf-original").unwrap();
                let tabs = visual.debug_bounds("document-tabs").unwrap();
                assert!(
                    button.left() >= tabs.left() && button.right() <= tabs.right(),
                    "{name}: original-page control stays within the center column at {width}px"
                );
            }
            cx.update(|cx| {
                let reader = reader.read(cx);
                let tab = &reader.tabs[reader.active];
                assert_eq!(
                    tab.resources.as_ref().unwrap().kind(),
                    pecan_import::SourceKind::Pdf
                );
                let source = tab.view.read(cx).document.source();
                assert!(source.contains(if name == "text.pdf" {
                    "Fixture Document"
                } else {
                    "原版查看"
                }));
            });
        }
        cx.update_window(handle, |_, window, cx| {
            reader.update(cx, |reader, cx| {
                let index = reader
                    .tabs
                    .iter()
                    .position(|tab| tab.path.as_ref() == Some(&office))
                    .unwrap();
                reader.close_tab(index, window, cx);
            })
        })
        .unwrap();
        settle(cx);
        assert!(
            !current_assets.exists(),
            "closing the office tab cleans up its image resources"
        );
    }

    #[gpui_kit::test]
    fn sidebar_handles_drag_collapse_reopen_and_limit_actual_layout(cx: &mut TestAppContext) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
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
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        settle(cx);
        let draw = |window: &mut Window, cx: &mut gpui_kit::App| {
            window.refresh();
            window.draw(cx).clear(cx);
        };
        visual.update(draw);
        for side in [crate::panes::Side::Left, crate::panes::Side::Right] {
            let (selector, pane, direction) = match side {
                crate::panes::Side::Left => ("left-pane-handle", "file-sidebar", 1.),
                crate::panes::Side::Right => ("right-pane-handle", "outline-sidebar", -1.),
            };
            let start = visual.debug_bounds(selector).unwrap().center();
            visual.simulate_mouse_down(start, gpui_kit::MouseButton::Left, Default::default());
            let edge = gpui_kit::point(if direction > 0. { px(2.) } else { px(1278.) }, start.y);
            visual.simulate_mouse_move(edge, gpui_kit::MouseButton::Left, Default::default());
            visual.update(draw);
            assert!(
                visual.debug_bounds(pane).is_none(),
                "dragging {pane} to its outer edge collapses it"
            );
            let reopened = gpui_kit::point(edge.x + px(200. * direction), start.y);
            visual.simulate_mouse_move(reopened, gpui_kit::MouseButton::Left, Default::default());
            visual.update(draw);
            assert!(
                visual.debug_bounds(pane).is_some(),
                "continuing the same drag reopens {pane}"
            );
            let max = gpui_kit::point(reopened.x + px(400. * direction), start.y);
            visual.simulate_mouse_move(max, gpui_kit::MouseButton::Left, Default::default());
            visual.update(draw);
            assert!(visual.debug_bounds(pane).unwrap().size.width <= px(400.));
            visual.simulate_mouse_up(max, gpui_kit::MouseButton::Left, Default::default());
            visual.update(draw);
            cx.update(|cx| assert!(reader.read(cx).pane_drag.is_none()));
        }
    }

    #[gpui_kit::test]
    fn three_columns_keep_tabs_in_center_and_toggle_independently(cx: &mut TestAppContext) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
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
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        settle(cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let left = visual.debug_bounds("file-sidebar").unwrap();
        let center = visual.debug_bounds("document-column").unwrap();
        let tabs = visual.debug_bounds("document-tabs").unwrap();
        let right = visual.debug_bounds("outline-sidebar").unwrap();
        assert_eq!(left.right(), center.left());
        assert_eq!(center.right(), right.left());
        assert_eq!(tabs.left(), center.left());
        assert_eq!(tabs.right(), center.right());
        assert_eq!(tabs.top(), left.top());
        visual.dispatch_action(Find);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let search = visual.debug_bounds("document-search").unwrap();
        assert_eq!(search.left(), center.left());
        assert_eq!(search.right(), center.right());
        visual.dispatch_action(ToggleSidebar);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("file-sidebar").is_none());
        assert!(visual.debug_bounds("outline-sidebar").is_some());
        assert_eq!(
            visual.debug_bounds("document-column").unwrap().left(),
            left.left()
        );
        visual.dispatch_action(ToggleOutline);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("outline-sidebar").is_none());
        visual.dispatch_action(ToggleSidebar);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("file-sidebar").is_some());
        assert!(visual.debug_bounds("outline-sidebar").is_none());
        visual.dispatch_action(FocusReading);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("document-tabs").is_none());
        visual.dispatch_action(ToggleSidebar);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("file-sidebar").is_some());
        assert!(visual.debug_bounds("document-tabs").is_some());
        cx.update(|cx| assert!(!reader.read(cx).focus_mode));
        settle(cx);
    }

    #[gpui_kit::test]
    fn title_stays_centered_and_fullscreen_reclaims_window_controls_space(cx: &mut TestAppContext) {
        let (handle, reader) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        gpui_kit::Point::default(),
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
        let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
        settle(cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            visual.debug_bounds("window-title").unwrap().center().x,
            px(640.),
            "title is centered against the entire window"
        );
        let toggle = visual.debug_bounds("titlebar-sidebar-toggle").unwrap();
        visual.simulate_click(toggle.center(), gpui_kit::Modifiers::default());
        cx.update(|cx| {
            assert!(
                !reader.read(cx).sidebar,
                "centered overlay must leave toolbar clicks functional"
            )
        });
        let windowed_left = visual
            .debug_bounds("titlebar-sidebar-toggle")
            .unwrap()
            .left();
        visual.update(|window, cx| {
            window.toggle_fullscreen();
            window.refresh();
            window.draw(cx).clear(cx);
        });
        settle(cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let fullscreen_left = visual
            .debug_bounds("titlebar-sidebar-toggle")
            .unwrap()
            .left();
        assert!(
            fullscreen_left <= px(16.)
                && if cfg!(target_os = "macos") {
                    windowed_left - fullscreen_left >= px(60.)
                } else {
                    windowed_left == fullscreen_left
                },
            "fullscreen must reclaim traffic-light space: {windowed_left:?} -> {fullscreen_left:?}"
        );
        assert_eq!(
            visual.debug_bounds("window-title").unwrap().center().x,
            px(640.)
        );
        visual.simulate_resize(size(px(900.), px(700.)));
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            visual.debug_bounds("window-title").unwrap().center().x,
            px(450.)
        );
        visual.update(|window, cx| {
            window.toggle_fullscreen();
            window.refresh();
            window.draw(cx).clear(cx);
        });
        settle(cx);
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            visual
                .debug_bounds("titlebar-sidebar-toggle")
                .unwrap()
                .left(),
            windowed_left,
            "windowed mode restores native-control spacing"
        );
    }

    fn settle(cx: &TestAppContext) {
        for _ in 0..10_000 {
            if !cx.executor().tick() {
                return;
            }
        }
        panic!("the complete reader window did not settle within 10000 executor ticks");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FolderOpen {
    Explicit,
    DocumentParent,
}
