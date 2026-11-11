use gpui_kit::{
    App, Menu, MenuItem, OsAction,
    component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo},
};

use crate::{
    About, ClearRecent, CloseTab, CloseWindow, CopyDocument, DecreaseText, ExportHtml,
    ExportMarkdown, ExportPdf, ExportWord, Find, FocusReading, Help, IncreaseText, Minimize,
    MoreRecent, NextTab, Open, OpenFolder, OriginalPdf, PreviousTab, Quit, Reload, ResetText,
    RevealFile, ShowSettings, ToggleFullscreen, ToggleOutline, ToggleSidebar, ToggleTheme,
    ZoomWindow,
};

pub(crate) fn install(cx: &mut App) {
    // Global handlers execute while the active window is borrowed by dispatch.
    // Defer native operations until that borrow is released.
    cx.on_action(|_: &ToggleFullscreen, cx| {
        cx.defer(|cx| {
            if crate::pdf::perform(crate::pdf::WindowAction::Fullscreen, cx) {
                return;
            }
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.toggle_fullscreen());
            }
        })
    });
    cx.on_action(|_: &Minimize, cx| {
        cx.defer(|cx| {
            if crate::pdf::perform(crate::pdf::WindowAction::Minimize, cx) {
                return;
            }
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.minimize_window());
            }
        })
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        cx.defer(|cx| {
            if crate::pdf::perform(crate::pdf::WindowAction::Zoom, cx) {
                return;
            }
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.zoom_window());
            }
        })
    });
    cx.on_action(|_: &CloseWindow, cx| {
        cx.defer(|cx| {
            if crate::pdf::perform(crate::pdf::WindowAction::Close, cx) {
                return;
            }
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        })
    });
    refresh(cx);
}

pub(crate) fn refresh(cx: &mut App) {
    let recent = crate::recent::Recent::shared(cx);
    let recent_menu = recent_menu(recent.read(cx).entries());
    let menus = vec![
        application_menu(),
        Menu::new("文件").items([
            MenuItem::action("打开文档…", Open),
            MenuItem::action("打开文件夹…", OpenFolder),
            MenuItem::submenu(recent_menu),
            MenuItem::separator(),
            MenuItem::action("重新加载", Reload),
            MenuItem::action("查看 PDF 原版", OriginalPdf),
            MenuItem::action(crate::platform::REVEAL_FILE, RevealFile),
            MenuItem::separator(),
            MenuItem::action("导出 PDF…", ExportPdf),
            MenuItem::action("导出 Word…", ExportWord),
            MenuItem::action("导出 HTML…", ExportHtml),
            MenuItem::action("导出 Markdown…", ExportMarkdown),
            MenuItem::separator(),
            MenuItem::action("关闭标签页", CloseTab),
            MenuItem::action("关闭窗口", CloseWindow),
        ]),
        Menu::new("编辑").items([
            MenuItem::os_action("撤销", Undo, OsAction::Undo),
            MenuItem::os_action("重做", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("剪切", Cut, OsAction::Cut),
            MenuItem::os_action("复制", Copy, OsAction::Copy),
            MenuItem::os_action("粘贴", Paste, OsAction::Paste),
            MenuItem::os_action("全选", SelectAll, OsAction::SelectAll),
            MenuItem::separator(),
            MenuItem::action("复制 Markdown 全文", CopyDocument),
            MenuItem::action("查找…", Find),
        ]),
        Menu::new("视图").items([
            MenuItem::action("显示／隐藏文件栏", ToggleSidebar),
            MenuItem::action("显示／隐藏大纲", ToggleOutline),
            MenuItem::action("专注阅读", FocusReading),
            MenuItem::separator(),
            MenuItem::action("增大字号", IncreaseText),
            MenuItem::action("减小字号", DecreaseText),
            MenuItem::action("恢复默认字号", ResetText),
            MenuItem::separator(),
            MenuItem::action("切换浅色／深色", ToggleTheme),
            MenuItem::action("进入／退出全屏", ToggleFullscreen),
        ]),
        Menu::new("窗口").items([
            MenuItem::action("最小化", Minimize),
            MenuItem::action("缩放窗口", ZoomWindow),
            MenuItem::separator(),
            MenuItem::action("下一个标签页", NextTab),
            MenuItem::action("上一个标签页", PreviousTab),
        ]),
        Menu::new("帮助").items([MenuItem::action("碧果使用指南", Help)]),
    ];
    #[cfg(not(target_os = "macos"))]
    {
        use gpui_kit::component::{GlobalState, menu::AppMenuBar};
        GlobalState::global_mut(cx).set_app_menus(menus.iter().cloned().map(Menu::owned).collect());
        if let Some(menu) = cx.try_global::<DesktopMenu>().map(|menu| menu.0.clone()) {
            menu.update(cx, |menu, cx| menu.reload(cx));
        } else {
            let menu = AppMenuBar::new(cx);
            cx.set_global(DesktopMenu(menu));
        }
    }
    cx.set_menus(menus);
}

fn application_menu() -> Menu {
    let mut items = vec![
        MenuItem::action("关于碧果", About),
        MenuItem::separator(),
        MenuItem::action("设置…", ShowSettings),
        MenuItem::separator(),
    ];
    #[cfg(target_os = "macos")]
    items.extend([
        MenuItem::os_submenu("服务", gpui_kit::SystemMenuType::Services),
        MenuItem::separator(),
        MenuItem::action("隐藏碧果", crate::Hide),
        MenuItem::action("隐藏其他应用", crate::HideOthers),
        MenuItem::action("显示全部", crate::ShowAll),
        MenuItem::separator(),
    ]);
    items.push(MenuItem::action("退出碧果", Quit));
    Menu::new("Pecan").items(items)
}

#[cfg(not(target_os = "macos"))]
struct DesktopMenu(gpui_kit::Entity<gpui_kit::component::menu::AppMenuBar>);
#[cfg(not(target_os = "macos"))]
impl gpui_kit::Global for DesktopMenu {}

/// macOS owns its system menu; other platforms use the kit's same action menu.
pub(crate) fn bar(cx: &gpui_kit::App) -> Option<gpui_kit::AnyElement> {
    #[cfg(target_os = "macos")]
    {
        let _ = cx;
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        use gpui_kit::{IntoElement, ParentElement, Styled, px};
        cx.try_global::<DesktopMenu>().map(|menu| {
            gpui_kit::div()
                .w_full()
                .h(px(30.))
                .flex_shrink_0()
                .px_2()
                .child(menu.0.clone())
                .into_any_element()
        })
    }
}

fn recent_menu(entries: &[crate::recent::Entry]) -> Menu {
    use crate::recent::{Kind, OpenRecent, display_path};
    let mut items = Vec::new();
    for kind in [Kind::Folder, Kind::File] {
        let group: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind == kind)
            .take(10)
            .map(|entry| {
                MenuItem::action(
                    display_path(&entry.path),
                    OpenRecent {
                        entry: entry.clone(),
                    },
                )
            })
            .collect();
        if !group.is_empty() {
            if !items.is_empty() {
                items.push(MenuItem::separator());
            }
            items.extend(group);
        }
    }
    if !items.is_empty() {
        items.push(MenuItem::separator());
    }
    items.push(MenuItem::action("更多…", MoreRecent));
    items.push(MenuItem::separator());
    items.push(MenuItem::action("清空最近记录", ClearRecent).disabled(entries.is_empty()));
    Menu::new("打开最近文件").items(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{AppContext, TestAppContext, component::TitleBar};

    #[test]
    fn recent_menu_groups_and_limits_paths_and_keeps_actions_addressed_by_path() {
        use crate::recent::{Entry, Kind, OpenRecent};
        let directory = tempfile::tempdir().unwrap();
        let entries: Vec<_> = (0..24)
            .map(|index| Entry {
                path: directory.path().join(format!("{index}.md")),
                kind: if index % 2 == 0 {
                    Kind::File
                } else {
                    Kind::Folder
                },
            })
            .collect();
        let menu = recent_menu(&entries);
        let actions: Vec<_> = menu
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { action, .. } => action.as_any().downcast_ref::<OpenRecent>(),
                _ => None,
            })
            .collect();
        assert_eq!(actions.len(), 20, "ten folders and ten documents");
        assert!(
            actions[..10]
                .iter()
                .all(|action| action.entry.kind == Kind::Folder)
        );
        assert!(
            actions[10..]
                .iter()
                .all(|action| action.entry.kind == Kind::File)
        );
        assert_eq!(actions[0].entry.path, entries[1].path);
        assert_eq!(actions[10].entry.path, entries[0].path);
        let empty = recent_menu(&[]);
        assert_eq!(empty.items.len(), 3);
        assert!(
            !empty.items[0].is_disabled(),
            "the searchable empty state remains available"
        );
        assert!(
            empty.items[2].is_disabled(),
            "clearing an empty history is disabled"
        );
    }

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

    #[gpui_kit::test]
    fn native_menu_action_toggles_the_active_window(cx: &mut TestAppContext) {
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            install(cx);
            assert_eq!(cx.get_menus().unwrap().len(), 6);
            gpui_kit::open_window(TitleBar::window_options(), cx, |_, cx| cx.new(|_| Empty))
                .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.activate_window();
            window.draw(cx).clear(cx);
        })
        .unwrap();
        cx.update(|cx| cx.dispatch_action(&ToggleFullscreen));
        cx.update_window(handle, |_, window, _| {
            assert!(
                window.is_fullscreen(),
                "native action must enter fullscreen"
            )
        })
        .unwrap();
        cx.update(|cx| cx.dispatch_action(&ToggleFullscreen));
        cx.update_window(handle, |_, window, _| {
            assert!(
                !window.is_fullscreen(),
                "native action must leave fullscreen"
            )
        })
        .unwrap();
    }
}
