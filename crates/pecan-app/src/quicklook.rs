//! System Quick Look image previews with bounded, background file preparation.
use std::{
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result, bail};
use gpui_kit::{App, AppContext as _, Global, Image, RenderImage};
use tempfile::TempDir;

/// One shared panel retains at most one snapshot; preparation never runs on the UI thread.
const MAX_BYTES: usize = 32 * 1024 * 1024;
// Covers full-size 4K diagrams without permitting unbounded pixel snapshots.
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
static PREPARATION_SLOTS: async_lock::Semaphore = async_lock::Semaphore::new(2);

pub(crate) enum Input {
    File(PathBuf),
    Encoded {
        image: Arc<Image>,
        title: String,
    },
    Pixels {
        image: Arc<RenderImage>,
        title: String,
    },
}

struct Snapshot {
    path: PathBuf,
    _directory: TempDir,
}

fn filename(title: &str, extension: &str) -> String {
    let stem = Path::new(title)
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("image");
    let stem: String = stem
        .chars()
        .filter(|ch| !ch.is_control())
        .take(100)
        .collect();
    format!("{stem}.{extension}")
}

fn snapshot(input: Input) -> Result<Snapshot> {
    let directory = tempfile::Builder::new()
        .prefix("pecan-quicklook-")
        .tempdir()?;
    let path = match input {
        Input::File(source) => {
            let mut source_file = fs::File::open(&source)?;
            if source_file.metadata()?.len() > MAX_BYTES as u64 {
                bail!("image exceeds Quick Look file budget");
            }
            let target = directory
                .path()
                .join(source.file_name().context("image has no filename")?);
            let mut output = fs::File::create(&target)?;
            let copied = std::io::copy(
                &mut (&mut source_file).take(MAX_BYTES as u64 + 1),
                &mut output,
            )?;
            if copied > MAX_BYTES as u64 {
                bail!("image exceeds Quick Look file budget");
            }
            output.flush()?;
            target
        }
        Input::Encoded { image, title } => {
            if image.bytes().len() > MAX_BYTES {
                bail!("image exceeds Quick Look file budget");
            }
            let target = directory
                .path()
                .join(filename(&title, image.format().extension()));
            fs::write(&target, image.bytes())?;
            target
        }
        Input::Pixels { image, title } => {
            let size = image.size(0);
            let bytes = image.as_bytes(0).context("image has no decoded frame")?;
            let target = directory.path().join(filename(&title, "png"));
            encode_bgra(
                &target,
                size.width.0.try_into()?,
                size.height.0.try_into()?,
                bytes,
            )?;
            target
        }
    };
    if fs::metadata(&path)?.len() > MAX_BYTES as u64 {
        bail!("image exceeds Quick Look file budget");
    }
    Ok(Snapshot {
        path,
        _directory: directory,
    })
}

fn encode_bgra(path: &Path, width: u32, height: u32, bytes: &[u8]) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > MAX_PIXELS || pixels * 4 != bytes.len() as u64 {
        bail!("image exceeds Quick Look pixel budget");
    }
    let mut encoder = png::Encoder::new(fs::File::create(path)?, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()?;
    // Convert one row at a time: exporting a large source must not clone its entire RGBA buffer.
    let row_bytes = usize::try_from(u64::from(width) * 4)?;
    let mut rgba_row = vec![0; row_bytes];
    {
        let mut stream = writer.stream_writer_with_size(row_bytes)?;
        for bgra_row in bytes.chunks_exact(row_bytes) {
            rgba_row.copy_from_slice(bgra_row);
            for pixel in rgba_row.as_chunks_mut::<4>().0 {
                pixel.swap(0, 2);
            }
            stream.write_all(&rgba_row)?;
        }
        stream.finish()?;
    }
    writer.finish()?;
    Ok(())
}

#[derive(Default)]
struct Session {
    revision: u64,
    current: Option<native::Preview>,
}
impl Global for Session {}

/// Start a single shared native preview; newer clicks supersede queued preparations.
pub(crate) fn show(
    input: Input,
    failed: impl FnOnce(anyhow::Error, &mut App) + 'static,
    cx: &mut App,
) {
    let parent = match native::parent() {
        Ok(parent) => parent,
        Err(error) => {
            failed(error, cx);
            return;
        }
    };
    if !cx.has_global::<Session>() {
        cx.set_global(Session::default());
    }
    cx.global_mut::<Session>().revision += 1;
    let revision = cx.global::<Session>().revision;
    let prepared = cx.background_spawn(async move {
        let _permit = PREPARATION_SLOTS.acquire().await;
        snapshot(input)
    });
    cx.spawn(async move |cx| {
        let prepared = prepared.await;
        cx.update(|cx| {
            if cx.global::<Session>().revision != revision {
                return;
            }
            let result = prepared.and_then(|snapshot| {
                // Remove the previous responder before installing a replacement.
                drop(cx.global_mut::<Session>().current.take());
                native::Preview::open(snapshot, parent)
            });
            match result {
                Ok(preview) => cx.global_mut::<Session>().current = Some(preview),
                Err(error) => failed(error, cx),
            }
        });
    })
    .detach();
}

pub(crate) fn is_active() -> bool {
    native::is_active()
}

pub(crate) fn perform(action: crate::pdf::WindowAction, cx: &mut App) -> bool {
    if !native::perform(action) {
        return false;
    }
    if matches!(action, crate::pdf::WindowAction::Close) && cx.has_global::<Session>() {
        cx.global_mut::<Session>().revision += 1;
        drop(cx.global_mut::<Session>().current.take());
    }
    true
}

mod native {
    use super::Snapshot;
    use anyhow::{Context as _, Result, bail};
    use objc2::{
        DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
        runtime::ProtocolObject,
    };
    use objc2_app_kit::{NSApplication, NSResponder, NSWindow};
    use objc2_foundation::{NSInteger, NSObjectProtocol, NSString, NSURL};
    use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel, QLPreviewPanelDataSource};

    struct ControllerData {
        url: Retained<NSURL>,
    }
    define_class!(
        #[unsafe(super(NSResponder))]
        #[name = "PecanQuickLookController"]
        #[thread_kind = MainThreadOnly]
        #[ivars = ControllerData]
        struct Controller;

        // SAFETY: The subclass inherits NSObject's protocol behavior from NSResponder.
        unsafe impl NSObjectProtocol for Controller {}

        // SAFETY: The data source returns a retained file URL and a valid single-item index.
        unsafe impl QLPreviewPanelDataSource for Controller {
            #[unsafe(method(numberOfPreviewItemsInPreviewPanel:))]
            fn count(&self, _: Option<&QLPreviewPanel>) -> NSInteger { 1 }
            #[unsafe(method_id(previewPanel:previewItemAtIndex:))]
            #[unsafe(method_family = none)]
            fn item(&self, _: Option<&QLPreviewPanel>, index: NSInteger) -> Option<Retained<ProtocolObject<dyn QLPreviewItem>>> {
                (index == 0).then(|| ProtocolObject::from_retained(self.ivars().url.clone()))
            }
        }

        impl Controller {
            #[unsafe(method(acceptsPreviewPanelControl:))]
            fn accepts(&self, _: Option<&QLPreviewPanel>) -> bool { true }
            #[unsafe(method(beginPreviewPanelControl:))]
            fn begin(&self, panel: Option<&QLPreviewPanel>) {
                if let Some(panel) = panel {
                    // SAFETY: AppKit grants control here; Preview retains this data source until detached.
                    unsafe { panel.setDataSource(Some(ProtocolObject::from_ref(self))); }
                }
            }
            #[unsafe(method(endPreviewPanelControl:))]
            fn end(&self, panel: Option<&QLPreviewPanel>) {
                if let Some(panel) = panel {
                    // SAFETY: AppKit ends this controller's ownership; nil explicitly detaches the source.
                    unsafe { panel.setDataSource(None); }
                }
            }
        }
    );

    pub(super) struct Preview {
        parent: Retained<NSWindow>,
        previous: Option<Retained<NSResponder>>,
        controller: Retained<Controller>,
        panel: Retained<QLPreviewPanel>,
        _snapshot: Snapshot,
    }

    pub(super) fn parent() -> Result<Retained<NSWindow>> {
        let mtm = MainThreadMarker::new().context("Quick Look requires the main thread")?;
        NSApplication::sharedApplication(mtm)
            .keyWindow()
            .context("no parent window for Quick Look")
    }

    impl Preview {
        pub(super) fn open(snapshot: Snapshot, parent: Retained<NSWindow>) -> Result<Self> {
            let mtm = MainThreadMarker::new().context("Quick Look requires the main thread")?;
            let url = NSURL::fileURLWithPath(&NSString::from_str(
                snapshot.path.to_str().context("invalid image path")?,
            ));
            // SAFETY: AppKit objects are created and installed on the main thread. Retained fields
            // keep both endpoints of the unretained responder chain alive, including its predecessor.
            let preview = unsafe {
                let allocated = Controller::alloc(mtm).set_ivars(ControllerData { url });
                let controller: Retained<Controller> = msg_send![super(allocated), init];
                let previous = parent.nextResponder();
                let panel = QLPreviewPanel::sharedPreviewPanel(mtm)
                    .context("Quick Look panel is unavailable")?;
                controller.setNextResponder(previous.as_deref());
                parent.setNextResponder(Some(&controller));
                Self {
                    parent,
                    previous,
                    controller,
                    panel,
                    _snapshot: snapshot,
                }
            };
            preview.parent.makeKeyAndOrderFront(None);
            // Quick Look grants control when its panel is shown; updating an invisible panel alone
            // does not start the controller lifecycle on all supported macOS releases.
            preview.panel.makeKeyAndOrderFront(None);
            // SAFETY: The controller is now in the live parent's responder chain. State is modified
            // only after Quick Look grants control, and snapshot owns the preview file throughout.
            unsafe {
                preview.panel.updateController();
                if !preview.controls_panel() {
                    preview.panel.orderOut(None);
                    bail!("Quick Look did not accept the image controller");
                }
                preview.panel.reloadData();
                preview.panel.setCurrentPreviewItemIndex(0);
            }
            Ok(preview)
        }

        fn controls_panel(&self) -> bool {
            // SAFETY: Live retained panel and controller, queried on the main thread.
            unsafe {
                self.panel.currentController().is_some_and(|current| {
                    Retained::as_ptr(&current).cast::<()>()
                        == Retained::as_ptr(&self.controller).cast::<()>()
                })
            }
        }
    }

    impl Drop for Preview {
        fn drop(&mut self) {
            if self.controls_panel() {
                // SAFETY: Closing a full-screen panel first releases its separate system space.
                unsafe {
                    if self.panel.isInFullScreenMode() {
                        self.panel.exitFullScreenModeWithOptions(None);
                    }
                }
                self.panel.orderOut(None);
            }
            // SAFETY: Restore only the link we own, before releasing its controller/predecessor.
            // updateController lets Quick Look call endPreviewPanelControl while both remain retained.
            unsafe {
                if self.parent.nextResponder().is_some_and(|next| {
                    Retained::as_ptr(&next).cast::<()>()
                        == Retained::as_ptr(&self.controller).cast::<()>()
                }) {
                    self.parent.setNextResponder(self.previous.as_deref());
                    self.panel.updateController();
                }
            }
        }
    }

    pub(super) fn is_active() -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        // SAFETY: Main-thread access; existence is checked without allocating an idle panel.
        unsafe {
            QLPreviewPanel::sharedPreviewPanelExists(mtm)
                && QLPreviewPanel::sharedPreviewPanel(mtm)
                    .is_some_and(|panel| panel.isKeyWindow() || panel.isInFullScreenMode())
        }
    }

    pub(super) fn perform(action: crate::pdf::WindowAction) -> bool {
        if !is_active() {
            return false;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        // SAFETY: The shared panel exists and is active on the main thread; nil options are Apple's defaults.
        unsafe {
            let Some(panel) = QLPreviewPanel::sharedPreviewPanel(mtm) else {
                return false;
            };
            match action {
                crate::pdf::WindowAction::Close => {
                    if panel.isInFullScreenMode() {
                        panel.exitFullScreenModeWithOptions(None);
                    }
                    panel.orderOut(None);
                }
                crate::pdf::WindowAction::Minimize => panel.miniaturize(None),
                crate::pdf::WindowAction::Zoom => panel.zoom(None),
                crate::pdf::WindowAction::Fullscreen => {
                    if panel.isInFullScreenMode() {
                        panel.exitFullScreenModeWithOptions(None);
                    } else {
                        panel.enterFullScreenMode_withOptions(None, None);
                    }
                }
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshots_preserve_encoded_bytes_and_outlive_the_source() {
        let original = tempfile::tempdir().unwrap();
        let source = original.path().join("picture with spaces.gif");
        let bytes = b"GIF89a preserved encoded image";
        fs::write(&source, bytes).unwrap();
        let file = snapshot(Input::File(source)).unwrap();
        drop(original);
        assert_eq!(fs::read(&file.path).unwrap(), bytes);
        assert_eq!(file.path.file_name().unwrap(), "picture with spaces.gif");
        let directory = file.path.parent().unwrap().to_path_buf();
        drop(file);
        assert!(
            !directory.exists(),
            "replacing a native preview cleans up its snapshot"
        );
        let encoded = snapshot(Input::Encoded {
            image: Arc::new(Image::from_bytes(
                gpui_kit::ImageFormat::Gif,
                bytes.to_vec(),
            )),
            title: "../inline".into(),
        })
        .unwrap();
        assert_eq!(fs::read(&encoded.path).unwrap(), bytes);
        assert_eq!(encoded.path.file_name().unwrap(), "inline.gif");
    }

    #[test]
    fn large_bgra_exports_full_dimensions_with_row_conversion() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("large.png");
        let (width, height) = (3780, 2380);
        let mut pixels = vec![0; width as usize * height as usize * 4];
        pixels[..4].copy_from_slice(&[3, 2, 1, 255]);
        let end = pixels.len();
        pixels[end - 4..].copy_from_slice(&[30, 20, 10, 128]);
        encode_bgra(&target, width, height, &pixels).unwrap();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(fs::File::open(&target).unwrap()))
                .read_info()
                .unwrap();
        assert_eq!((reader.info().width, reader.info().height), (width, height));
        let mut rows = 0;
        while let Some(row) = reader.next_row().unwrap() {
            let data = row.data();
            if rows == 0 {
                assert_eq!(
                    &data[..4],
                    &[1, 2, 3, 255],
                    "first source pixel is preserved"
                );
            }
            if rows == height - 1 {
                assert_eq!(
                    &data[data.len() - 4..],
                    &[10, 20, 30, 128],
                    "last source pixel is preserved"
                );
            }
            rows += 1;
        }
        assert_eq!(rows, height, "the complete original image must be exported");
    }

    #[test]
    fn cached_bgra_exports_correct_pixels_and_rejects_resource_growth() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("cached.png");
        encode_bgra(&target, 2, 1, &[3, 2, 1, 255, 30, 20, 10, 128]).unwrap();
        let mut reader =
            png::Decoder::new(std::io::BufReader::new(fs::File::open(&target).unwrap()))
                .read_info()
                .unwrap();
        let mut decoded = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut decoded).unwrap();
        assert_eq!(
            &decoded[..info.buffer_size()],
            &[1, 2, 3, 255, 10, 20, 30, 128]
        );
        assert_eq!((info.width, info.height), (2, 1));
        assert!(encode_bgra(&target, 9000, 9000, &[]).is_err());
        let source = temp.path().join("huge.png");
        fs::File::create(&source)
            .unwrap()
            .set_len(MAX_BYTES as u64 + 1)
            .unwrap();
        assert!(snapshot(Input::File(source)).is_err());
    }
}
