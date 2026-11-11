//! Platform conventions shared by shortcuts, labels and window chrome.

pub(crate) const fn choose<T>(macos: T, other: T) -> T
where
    T: Copy,
{
    if cfg!(target_os = "macos") {
        macos
    } else {
        other
    }
}

pub(crate) const REVEAL_FILE: &str = choose("在 Finder 中显示", "在文件资源管理器中显示");

pub(crate) fn titlebar_inset(fullscreen: bool) -> gpui_kit::Pixels {
    gpui_kit::px(if fullscreen { 0. } else { choose(80., 0.) })
}
