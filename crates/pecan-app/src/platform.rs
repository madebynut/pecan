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

/// Wayland app id and the X11 window class. The desktop file uses the same value.
pub(crate) const APP_ID: &str = "com.pecan.reader";

pub(crate) const REVEAL_FILE: &str = if cfg!(target_os = "macos") {
    "在 Finder 中显示"
} else if cfg!(target_os = "windows") {
    "在文件资源管理器中显示"
} else {
    "在文件管理器中显示"
};

pub(crate) fn titlebar_inset(fullscreen: bool) -> gpui_kit::Pixels {
    gpui_kit::px(if fullscreen { 0. } else { choose(80., 0.) })
}

/// Local paths from the `file:` URLs macOS sends for Finder, Open With and Dock drops.
///
/// Other schemes are ignored: Pecan registers no URL schemes, so they are not documents.
pub(crate) fn opened_paths(urls: &[String]) -> Vec<std::path::PathBuf> {
    urls.iter()
        .filter_map(|url| url::Url::parse(url).ok())
        .filter(|url| url.scheme() == "file")
        .filter_map(|url| url.to_file_path().ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn opened_urls_become_local_paths() {
        let paths = opened_paths(&[
            "file:///Users/me/%E6%96%87%E6%A1%A3/a%20b.md".into(),
            "file:///Users/me/Notes/".into(),
            "https://example.com/a.md".into(),
            "not a url".into(),
        ]);
        assert_eq!(
            paths,
            [
                std::path::PathBuf::from("/Users/me/文档/a b.md"),
                std::path::PathBuf::from("/Users/me/Notes/"),
            ]
        );
    }

    #[test]
    fn reveal_label_names_the_platform_file_manager() {
        if cfg!(target_os = "macos") {
            assert_eq!(REVEAL_FILE, "在 Finder 中显示");
        } else if cfg!(target_os = "windows") {
            assert_eq!(REVEAL_FILE, "在文件资源管理器中显示");
        } else {
            assert_eq!(REVEAL_FILE, "在文件管理器中显示");
        }
    }
}
