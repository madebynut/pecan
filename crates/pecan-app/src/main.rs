//! Pecan is a native, virtualized Markdown reader with background Mermaid rendering.
// Debug builds keep the console so `cargo run` shows logs on Windows.
#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod assets;
mod brand;
mod code;
mod diagnostics;
mod diagram;
mod export;
mod fonts;
#[cfg(test)]
mod image_budget;
use pecan_assets::language as language_icons;
mod math;
mod media;
mod menus;
mod mermaid_source;
mod network;
mod notes;
mod panes;
mod pdf;
mod platform;
mod preferences;
mod presentation;
#[cfg(target_os = "macos")]
mod quicklook;
mod raster;
mod reading;
mod recent;
mod settings;
mod source;
mod tables;
mod ui;
mod update;
mod viewer;
mod workspace;

use gpui_kit::{
    App, AppContext, Bounds, KeyBinding, Pixels, QuitMode, Size, WindowBounds, WindowDecorations,
    WindowKind, WindowOptions,
    component::{TitleBar, WindowExt},
    px, size,
};

gpui_kit::actions!(
    pecan,
    [
        Quit,
        Open,
        OpenFolder,
        MoreRecent,
        ClearRecent,
        OriginalPdf,
        Find,
        ToggleSidebar,
        ToggleOutline,
        Reload,
        ShowSettings,
        About,
        Hide,
        HideOthers,
        ShowAll,
        CloseTab,
        CopyDocument,
        CopyFilePath,
        ExportPdf,
        ExportWord,
        ExportHtml,
        ToggleSource,
        SaveDocument,
        SaveDocumentAs,
        RevealFile,
        FocusReading,
        IncreaseText,
        DecreaseText,
        ResetText,
        ToggleFullscreen,
        Minimize,
        ZoomWindow,
        CloseWindow,
        Help,
        NextTab,
        PreviousTab,
        ToggleTheme
    ]
);

fn main() {
    diagnostics::install();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        "starting Pecan"
    );
    // File managers pass every selected document; Windows Explorer and `.desktop` `%F`
    // both deliver plain paths, while unknown `-flags` are ignored.
    let initial: Vec<std::path::PathBuf> = std::env::args_os()
        .skip(1)
        .filter(|argument| !argument.to_string_lossy().starts_with('-'))
        .map(std::path::PathBuf::from)
        .collect();
    let application = gpui_kit::application()
        .with_http_client(network::client())
        .with_assets(assets::ReaderAssets)
        .with_quit_mode(QuitMode::LastWindowClosed);
    // macOS delivers Finder double-clicks, Open With and Dock drops as `file:` URLs, at
    // launch possibly before the first window exists, so they queue until the app runs.
    let (opened, opened_paths) = async_channel::unbounded::<Vec<std::path::PathBuf>>();
    application.on_open_urls(move |urls| {
        let paths = platform::opened_paths(&urls);
        if !paths.is_empty() {
            let _ = opened.try_send(paths);
        }
    });
    application.run(move |cx: &mut App| {
        gpui_kit::init(cx);
        cx.set_app_identity(platform::APP_ID, "Pecan");
        fonts::install(cx);
        #[cfg(target_os = "macos")]
        brand::install_native_icon();
        gpui_kit::component::set_locale("zh-CN");
        let config_directory = preferences::config_directory();
        let launch = config_directory
            .as_deref()
            .map(pecan_config::startup::record_launch)
            .transpose()
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "unable to record first launch");
                None
            })
            .unwrap_or(pecan_config::startup::Launch::Returning);
        let prefs = preferences::Preferences::install(
            config_directory
                .as_ref()
                .map(|directory| directory.join("preferences.json")),
            cx,
        );
        recent::Recent::install(
            config_directory.map(|directory| directory.join("recent.json")),
            cx,
        );
        if let Some(root) = pecan_config::recovery_directory() {
            app::install_recovery(root, cx);
        }
        preferences::apply_theme(&prefs.read(cx).data.clone(), cx);
        cx.on_action(|_: &ShowSettings, cx| cx.defer(settings::open));
        cx.on_action(|_: &About, cx| cx.defer(settings::about));
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        cx.on_action(|_: &Quit, cx| cx.defer(app::request_quit));
        cx.bind_keys([
            KeyBinding::new("escape", diagram::CloseDiagram, Some("DiagramViewer")),
            KeyBinding::new("+", diagram::ZoomIn, Some("DiagramViewer")),
            KeyBinding::new("-", diagram::ZoomOut, Some("DiagramViewer")),
            KeyBinding::new("escape", media::CloseImage, Some("ImageViewer")),
            KeyBinding::new("+", media::ZoomImageIn, Some("ImageViewer")),
            KeyBinding::new("=", media::ZoomImageIn, Some("ImageViewer")),
            KeyBinding::new("-", media::ZoomImageOut, Some("ImageViewer")),
            KeyBinding::new("0", media::FitImage, Some("ImageViewer")),
            KeyBinding::new(
                platform::choose("cmd-r", "ctrl-r"),
                media::ReloadImage,
                Some("ImageViewer"),
            ),
            KeyBinding::new(platform::choose("cmd-q", "ctrl-q"), Quit, None),
            KeyBinding::new(platform::choose("cmd-,", "ctrl-,"), ShowSettings, None),
            KeyBinding::new(platform::choose("cmd-w", "ctrl-w"), CloseWindow, None),
            KeyBinding::new(platform::choose("cmd-w", "ctrl-w"), CloseTab, Some("Pecan")),
            KeyBinding::new(platform::choose("cmd-m", "ctrl-m"), Minimize, None),
            KeyBinding::new(
                platform::choose("ctrl-cmd-f", "f11"),
                ToggleFullscreen,
                None,
            ),
            KeyBinding::new(
                platform::choose("cmd-+", "ctrl-+"),
                IncreaseText,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-=", "ctrl-="),
                IncreaseText,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd--", "ctrl--"),
                DecreaseText,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-0", "ctrl-0"),
                ResetText,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-shift-f", "ctrl-shift-f"),
                FocusReading,
                Some("Pecan"),
            ),
            KeyBinding::new("ctrl-tab", NextTab, Some("Pecan")),
            KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Pecan")),
            KeyBinding::new(
                platform::choose("cmd-c", "ctrl-c"),
                gpui_kit::component::input::Copy,
                None,
            ),
            KeyBinding::new(
                platform::choose("cmd-a", "ctrl-a"),
                gpui_kit::component::input::SelectAll,
                None,
            ),
            KeyBinding::new(platform::choose("cmd-o", "ctrl-o"), Open, Some("Pecan")),
            KeyBinding::new(
                platform::choose("cmd-s", "ctrl-s"),
                SaveDocument,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-shift-s", "ctrl-shift-s"),
                SaveDocumentAs,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-e", "ctrl-e"),
                ToggleSource,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-shift-o", "ctrl-shift-o"),
                OpenFolder,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-alt-p", "ctrl-alt-p"),
                OriginalPdf,
                Some("Pecan"),
            ),
            KeyBinding::new(platform::choose("cmd-f", "ctrl-f"), Find, Some("Pecan")),
            KeyBinding::new(
                platform::choose("cmd-b", "ctrl-b"),
                ToggleSidebar,
                Some("Pecan"),
            ),
            KeyBinding::new(
                platform::choose("cmd-l", "ctrl-l"),
                ToggleOutline,
                Some("Pecan"),
            ),
            KeyBinding::new(platform::choose("cmd-r", "ctrl-r"), Reload, Some("Pecan")),
            KeyBinding::new(
                if cfg!(target_os = "macos") {
                    "ctrl-cmd-r"
                } else {
                    "ctrl-alt-r"
                },
                MoreRecent,
                Some("Pecan"),
            ),
        ]);
        #[cfg(target_os = "macos")]
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
        ]);
        #[cfg(target_os = "windows")]
        cx.bind_keys([KeyBinding::new("alt-f4", CloseWindow, None)]);
        menus::install(cx);
        install_fullscreen_escape(cx);
        let mut initial = initial;
        while let Ok(paths) = opened_paths.try_recv() {
            initial.extend(paths);
        }
        open_reader(initial, launch, cx).expect("unable to create the Pecan desktop window");
        app::offer_recovery(cx);
        update::check(cx);
        cx.activate(true);
        cx.spawn(async move |cx| {
            while let Ok(paths) = opened_paths.recv().await {
                cx.update(|cx| {
                    cx.activate(true);
                    if !app::open_in_front(paths.clone(), cx)
                        && let Err(error) =
                            open_reader(paths, pecan_config::startup::Launch::Returning, cx)
                    {
                        tracing::error!(%error, "unable to open a window for opened files");
                    }
                });
            }
        })
        .detach();
    });
}

fn open_reader(
    paths: Vec<std::path::PathBuf>,
    launch: pecan_config::startup::Launch,
    cx: &mut App,
) -> anyhow::Result<()> {
    let options = native_window_options("碧果", cx, size(px(1280.), px(860.)));
    gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| app::Reader::new(paths, launch, window, cx))
    })?;
    Ok(())
}

/// Keep native window controls, DPI scaling and client-drawn chrome on each platform.
fn native_window_options(title: &str, cx: &App, dimensions: Size<Pixels>) -> WindowOptions {
    let mut options = TitleBar::window_options();
    options.kind = WindowKind::Normal;
    options.is_movable = true;
    options.app_owns_titlebar_drag = true;
    options.is_resizable = true;
    options.is_minimizable = true;
    options.window_decorations = Some(WindowDecorations::Client);
    options.window_bounds = Some(WindowBounds::Windowed(Bounds::centered(
        None, dimensions, cx,
    )));
    options.window_min_size = Some(size(px(760.), px(500.)));
    if let Some(titlebar) = options.titlebar.as_mut() {
        titlebar.title = Some(title.to_string().into());
    }
    // Wayland groups windows by this id, and X11 uses it as the window class
    // so the desktop entry's icon and launcher match the running process.
    #[cfg(target_os = "linux")]
    {
        options.app_id = Some(platform::APP_ID.into());
        options.icon = Some(brand::window_icon());
    }
    options
}

fn install_fullscreen_escape(cx: &mut App) {
    cx.intercept_keystrokes(|event, window, cx| {
        if event.keystroke.key == "escape"
            && window.is_fullscreen()
            && !window.has_active_dialog(cx)
            && !viewer::is_media_window(window, cx)
        {
            window.toggle_fullscreen();
            cx.stop_propagation();
        }
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, component::WindowExt};
    struct Empty;
    impl gpui_kit::Render for Empty {
        fn render(
            &mut self,
            _: &mut gpui_kit::Window,
            _: &mut gpui_kit::Context<Self>,
        ) -> impl gpui_kit::IntoElement {
            gpui_kit::div()
        }
    }
    /// Records GPUI's `log` errors, which reach users through the tracing bridge.
    struct ErrorLog(std::sync::Mutex<Vec<String>>);
    impl log::Log for ErrorLog {
        fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
            metadata.level() <= log::Level::Error
        }
        fn log(&self, record: &log::Record<'_>) {
            if self.enabled(record.metadata()) {
                self.0.lock().unwrap().push(record.args().to_string());
            }
        }
        fn flush(&self) {}
    }
    static ERRORS: ErrorLog = ErrorLog(std::sync::Mutex::new(Vec::new()));

    #[gpui_kit::test]
    fn closing_a_window_before_its_activation_arrives_logs_no_error(cx: &mut TestAppContext) {
        let _ = log::set_logger(&ERRORS);
        log::set_max_level(log::LevelFilter::Error);
        let (main, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(TitleBar::window_options(), cx, |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        cx.run_until_parked();
        let (viewer, _) = cx.update(|cx| {
            gpui_kit::open_window(TitleBar::window_options(), cx, |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        // Like Escape in an image viewer: the window activates and closes in one turn,
        // so the platform's activation notice arrives after GPUI removed the window.
        cx.update_window(viewer, |_, window, _| {
            window.activate_window();
            window.remove_window();
        })
        .unwrap();
        cx.run_until_parked();
        assert!(cx.windows().contains(&main));
        let errors = ERRORS.0.lock().unwrap().clone();
        assert!(
            !errors
                .iter()
                .any(|error| error.contains("active status change")),
            "{errors:?}"
        );
    }

    #[gpui_kit::test]
    fn escape_closes_dialog_before_exiting_fullscreen(cx: &mut TestAppContext) {
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            install_fullscreen_escape(cx);
            gpui_kit::open_window(TitleBar::window_options(), cx, |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.toggle_fullscreen();
            window.open_dialog(cx, |dialog, _, _| dialog.title("Test dialog"));
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.simulate_keystrokes(handle, "escape");
        cx.update_window(handle, |_, window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                window.is_fullscreen(),
                "Escape first closes the active dialog"
            );
            assert!(!window.has_active_dialog(cx));
        })
        .unwrap();
        cx.simulate_keystrokes(handle, "escape");
        cx.update_window(handle, |_, window, _| {
            assert!(!window.is_fullscreen(), "Escape then exits fullscreen")
        })
        .unwrap();
    }
}
