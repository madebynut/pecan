//! Native original-page PDF viewing, alongside the common reflow reader.
use std::path::Path;

#[cfg(target_os = "macos")]
mod native {
    use anyhow::{Context as _, Result, bail};
    use block2::RcBlock;
    use gpui_kit::{App, Global};
    use objc2::{
        AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
        runtime::ProtocolObject,
    };
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSColor, NSEvent, NSEventType, NSScreen, NSWindow,
        NSWindowCollectionBehavior, NSWindowStyleMask, NSWindowWillCloseNotification,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
        NSURL,
    };
    use objc2_pdf_kit::{PDFDisplayMode, PDFDocument, PDFView};
    use std::{
        path::{Path, PathBuf},
        ptr::NonNull,
    };

    define_class!(
        #[unsafe(super(NSWindow))]
        #[name = "PecanOriginalPdfWindow"]
        #[thread_kind = MainThreadOnly]
        struct OriginalWindow;

        impl OriginalWindow {
            #[unsafe(method(sendEvent:))]
            fn send_event(&self, event: &NSEvent) {
                if event.r#type() == NSEventType::KeyDown && event.keyCode() == 53 {
                    self.close();
                } else {
                    // SAFETY: The event and window are live AppKit objects on the main thread.
                    unsafe { let _: () = msg_send![super(self), sendEvent: event]; }
                }
            }
        }
    );

    struct NativeWindow {
        id: usize,
        path: PathBuf,
        window: Retained<OriginalWindow>,
        observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
    }
    impl Drop for NativeWindow {
        fn drop(&mut self) {
            // SAFETY: App globals and their native windows are created/dropped on the main thread;
            // the observer token came from this notification center and is still retained.
            unsafe {
                NSNotificationCenter::defaultCenter().removeObserver((*self.observer).as_ref());
            }
            self.window.close();
        }
    }
    #[derive(Default)]
    struct Windows {
        next_id: usize,
        open: Vec<NativeWindow>,
    }
    impl Global for Windows {}

    fn read_document(path: &Path) -> Result<Retained<PDFDocument>> {
        if path.metadata()?.len() > pecan_document::MAX_DOCUMENT_BYTES as u64 {
            bail!("PDF 超过 32 MiB 阅读预算");
        }
        let url = NSURL::fileURLWithPath(&NSString::from_str(
            path.to_str().context("PDF 路径不是 UTF-8")?,
        ));
        // SAFETY: URL is a retained local file URL. PDFKit owns parsing and page resources.
        let document = unsafe { PDFDocument::initWithURL(PDFDocument::alloc(), &url) }
            .context("PDFKit 无法读取此文件")?;
        // SAFETY: This is a valid, retained PDFDocument; the query does not mutate shared state.
        let pages = unsafe { document.pageCount() };
        if pages == 0 || pages > 2000 {
            bail!("PDF 原版查看支持 1–2000 页");
        }
        Ok(document)
    }

    pub(super) fn is_active(cx: &App) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        let Some(key) = NSApplication::sharedApplication(mtm).keyWindow() else {
            return false;
        };
        cx.has_global::<Windows>()
            && cx
                .global::<Windows>()
                .open
                .iter()
                .any(|item| std::ptr::eq::<NSWindow>(&**item.window, &*key))
    }

    /// Let native-window menu actions operate on the PDF window, never a background reader tab.
    pub(super) fn perform(action: super::WindowAction, cx: &mut App) -> bool {
        if !is_active(cx) {
            return false;
        }
        let mtm = MainThreadMarker::new().expect("native key window is on the main thread");
        let Some(key) = NSApplication::sharedApplication(mtm).keyWindow() else {
            return false;
        };
        match action {
            super::WindowAction::Close => key.performClose(None),
            super::WindowAction::Fullscreen => key.toggleFullScreen(None),
            super::WindowAction::Minimize => key.miniaturize(None),
            super::WindowAction::Zoom => key.zoom(None),
        }
        true
    }

    pub(super) fn open(path: &Path, cx: &mut App) -> Result<()> {
        let mtm = MainThreadMarker::new().context("PDF 原版查看需要 AppKit 主线程")?;
        if !cx.has_global::<Windows>() {
            cx.set_global(Windows::default());
        }
        let path = path.canonicalize()?;
        if let Some(item) = cx
            .global::<Windows>()
            .open
            .iter()
            .find(|item| item.path == path)
        {
            item.window.makeKeyAndOrderFront(None);
            return Ok(());
        }
        if cx.global::<Windows>().open.len() >= 8 {
            bail!("最多同时打开 8 个 PDF 原版窗口");
        }
        let document = read_document(&path)?;
        // SAFETY: This is a valid, retained PDFDocument; queries do not mutate shared state.
        let pages = unsafe { document.pageCount() };
        if pages == 0 {
            bail!("PDF 没有可显示的页面");
        }
        let frame = NSScreen::mainScreen(mtm).map_or(
            NSRect::new(NSPoint::new(0., 0.), NSSize::new(1280., 860.)),
            |screen| screen.visibleFrame(),
        );
        let styles = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        // SAFETY: MainThreadMarker enforces AppKit affinity; frame and style are valid native values.
        // Release-on-close is disabled because Rust retains and releases this window via its observer.
        let window = unsafe {
            let allocated = OriginalWindow::alloc(mtm);
            let window: Retained<OriginalWindow> = msg_send![allocated, initWithContentRect: frame, styleMask: styles, backing: NSBackingStoreType::Buffered, defer: false];
            window.setReleasedWhenClosed(false);
            window
        };
        window.setFrame_display(frame, false);
        window.setMinSize(NSSize::new(640., 420.));
        window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenPrimary);
        window.setTitle(&NSString::from_str(&format!(
            "{} · 原版 · {} 页",
            path.file_name().unwrap_or_default().to_string_lossy(),
            pages
        )));
        let content = NSRect::new(
            NSPoint::new(0., 0.),
            window.contentRectForFrameRect(frame).size,
        );
        // SAFETY: The view is created, configured and installed only on the main thread;
        // setDocument retains the PDFDocument and setContentView retains the view.
        let view = unsafe {
            let view = PDFView::initWithFrame(PDFView::alloc(mtm), content);
            view.setDocument(Some(&document));
            view.setDisplayMode(PDFDisplayMode::SinglePageContinuous);
            view.setDisplaysPageBreaks(true);
            view.setAutoScales(true);
            view.setBackgroundColor(&NSColor::colorWithSRGBRed_green_blue_alpha(
                1.,
                252. / 255.,
                244. / 255.,
                1.,
            ));
            view
        };
        window.setContentView(Some(&view));
        let id = cx.global::<Windows>().next_id;
        cx.global_mut::<Windows>().next_id += 1;
        let app = cx.to_async();
        let on_close = RcBlock::new(move |_: NonNull<NSNotification>| {
            // Schedule cleanup after AppKit dispatch, avoiding a nested mutable App borrow.
            app.spawn(async move |cx| {
                cx.update(|cx| {
                    if cx.has_global::<Windows>() {
                        cx.global_mut::<Windows>().open.retain(|item| item.id != id);
                    }
                });
            })
            .detach();
        });
        // SAFETY: Observe only this window's close notification. The center retains the copied
        // block; NativeWindow owns the returned token and removes it before releasing the window.
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSWindowWillCloseNotification),
                Some(&window),
                None,
                &on_close,
            )
        };
        window.makeKeyAndOrderFront(None);
        // PDFKit lays out after installation; start at the first page rather than its initial center.
        // SAFETY: The retained view and document are installed in a live main-thread window.
        unsafe {
            view.goToFirstPage(None);
        }
        cx.global_mut::<Windows>().open.push(NativeWindow {
            id,
            path,
            window,
            observer,
        });
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn native_pdfkit_reads_original_text_and_scanned_pages() {
            objc2::rc::autoreleasepool(|_| {
                let directory =
                    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pecan-import/tests/fixtures");
                for name in ["text.pdf", "scan.pdf", "mixed.pdf"] {
                    let document = read_document(&directory.join(name)).unwrap();
                    // SAFETY: The document and returned page are retained PDFKit objects;
                    // these read-only queries are supported by PDFDocument's AnyThread API.
                    unsafe {
                        assert!(
                            document.pageCount() > 0,
                            "{name} preserves its original pages"
                        );
                        assert!(
                            document.pageAtIndex(0).is_some(),
                            "{name} has a renderable original page"
                        );
                        if name == "text.pdf" {
                            assert!(
                                document
                                    .string()
                                    .unwrap()
                                    .to_string()
                                    .contains("Fixture Document"),
                                "original text remains selectable"
                            );
                        }
                    }
                }
            });
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum WindowAction {
    Close,
    Fullscreen,
    Minimize,
    Zoom,
}

pub(crate) fn perform(action: WindowAction, cx: &mut gpui_kit::App) -> bool {
    #[cfg(target_os = "macos")]
    {
        if crate::quicklook::perform(action, cx) {
            return true;
        }
        native::perform(action, cx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (action, cx);
        false
    }
}

pub(crate) fn open(path: &Path, cx: &mut gpui_kit::App) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        native::open(path, cx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // The reflow reader is shared; original pages use the user's PDF association.
        let path = path.canonicalize()?;
        cx.open_with_system(&path);
        Ok(())
    }
}

/// Whether the frontmost native window is an native document or image viewer.
pub(crate) fn is_active(cx: &gpui_kit::App) -> bool {
    #[cfg(target_os = "macos")]
    {
        crate::quicklook::is_active() || native::is_active(cx)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
        false
    }
}
