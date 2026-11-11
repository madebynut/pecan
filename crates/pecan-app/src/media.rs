//! Clickable Markdown images with the shared GPUI media preview.
use std::{collections::HashMap, path::Path, sync::Arc};

use gpui_kit::{
    App, Context, FocusHandle, Image, ImageCacheProvider, ImageFormat, ImageSource, IntoElement,
    Pixels, Render, RenderImage, Resource, ScrollHandle, SharedString, Size, Subscription, Window,
    component::{
        ActiveTheme, Theme, h_flex,
        text::{
            InlineElement, InlineRenderContext, MarkdownExtensions, MarkdownNode,
            MarkdownParseContext, MarkdownPlugin, markdown_ast,
        },
    },
    div, img,
    prelude::*,
    px, retain_all,
};
use pulldown_cmark::Parser;

use crate::viewer;

gpui_kit::actions!(
    pecan_image,
    [CloseImage, ZoomImageIn, ZoomImageOut, FitImage]
);

/// Keep full-resolution pixels cached; zoom only changes the element's geometry.
const MAX_ZOOM: f32 = 8.;
const MIN_ZOOM: f32 = 0.01;

#[derive(Clone)]
struct MarkdownImage {
    source: Result<PictureSource, String>,
    alt: String,
    width: Option<f32>,
    height: Option<f32>,
}

// Markdown plugins parse on worker threads; ImageSource's custom-loader
// variant is not Send. Store only the thread-safe sources we actually support.
#[derive(Clone)]
enum PictureSource {
    Resource(Resource),
    Bytes(Arc<Image>),
}

#[derive(Clone, Copy)]
enum Placement {
    Block,
    Inline,
}

struct ImagePlugin {
    directory: Arc<Path>,
    references: Arc<HashMap<String, String>>,
    placement: Placement,
}

pub(crate) fn extensions(
    base: MarkdownExtensions,
    source: &str,
    directory: &Path,
) -> MarkdownExtensions {
    // Code-only rows need no extra scan or plugin registration.
    if !source.contains("![")
        && !source
            .as_bytes()
            .windows(4)
            .any(|bytes| bytes.eq_ignore_ascii_case(b"<img"))
    {
        return base;
    }
    let references: Arc<HashMap<String, String>> = Arc::new(
        Parser::new(source)
            .reference_definitions()
            .iter()
            .map(|(key, definition)| (reference_key(key), definition.dest.to_string()))
            .collect(),
    );
    let directory: Arc<Path> = directory.into();
    base.plugin(ImagePlugin {
        directory: directory.clone(),
        references: references.clone(),
        placement: Placement::Block,
    })
    .plugin(ImagePlugin {
        directory,
        references,
        placement: Placement::Inline,
    })
}

impl ImagePlugin {
    fn image(&self, node: &markdown_ast::Node) -> Option<MarkdownImage> {
        if let markdown_ast::Node::Html(html) = node {
            let image = html_image(&html.value)?;
            return Some(MarkdownImage {
                source: resolve_source(&image.uri, &self.directory),
                alt: image.alt,
                width: image.width,
                height: image.height,
            });
        }
        let (uri, alt) = match node {
            markdown_ast::Node::Image(image) => (image.url.as_str(), image.alt.as_str()),
            markdown_ast::Node::ImageReference(image) => (
                self.references
                    .get(&reference_key(&image.identifier))?
                    .as_str(),
                image.alt.as_str(),
            ),
            markdown_ast::Node::Link(link) if link.children.len() == 1 => {
                return self.image(&link.children[0]);
            }
            _ => return None,
        };
        Some(MarkdownImage {
            source: resolve_source(uri, &self.directory),
            alt: alt.to_string(),
            width: None,
            height: None,
        })
    }
}

impl MarkdownPlugin for ImagePlugin {
    fn name(&self) -> &str {
        match self.placement {
            Placement::Block => "pecan-image-block",
            Placement::Inline => "pecan-image-inline",
        }
    }

    fn is_block(&self) -> bool {
        matches!(self.placement, Placement::Block)
    }

    fn parse(
        &self,
        node: &markdown_ast::Node,
        context: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let candidate = match (self.placement, node) {
            (Placement::Block, markdown_ast::Node::Paragraph(paragraph))
                if paragraph.children.len() == 1 =>
            {
                &paragraph.children[0]
            }
            (Placement::Inline, _) => node,
            (Placement::Block, markdown_ast::Node::Html(_)) => node,
            _ => return None,
        };
        let image = self.image(candidate)?;
        let alt = image.alt.clone();
        Some(
            MarkdownNode::new(self.name(), image)
                .text(alt.clone())
                .accessibility_label(format!("{alt}，点击放大图片"))
                .markdown(context.node_source(node).unwrap_or_default().to_string()),
        )
    }

    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement {
        preview(node, None, window, cx)
    }

    fn render_inline(
        &self,
        node: &MarkdownNode,
        context: &InlineRenderContext,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<InlineElement> {
        Some(InlineElement::new(preview(
            node,
            Some(f32::from(context.line_height())),
            window,
            cx,
        )))
    }
}

fn reference_key(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

use pecan_document::html_image;

fn resolve_source(uri: &str, directory: &Path) -> Result<PictureSource, String> {
    if uri.starts_with("data:") {
        let data = data_url::DataUrl::process(uri).map_err(|error| error.to_string())?;
        let mime = data.mime_type();
        let format = ImageFormat::from_mime_type(&format!("{}/{}", mime.type_, mime.subtype))
            .ok_or_else(|| "不支持此图片格式".to_string())?;
        let (bytes, _) = data.decode_to_vec().map_err(|error| error.to_string())?;
        return Ok(PictureSource::Bytes(Arc::new(Image::from_bytes(
            format, bytes,
        ))));
    }
    #[cfg(windows)]
    if Path::new(uri).is_absolute() {
        return Ok(PictureSource::Resource(Path::new(uri).to_path_buf().into()));
    }
    if let Ok(url) = url::Url::parse(uri) {
        return if url.scheme() == "file" {
            url.to_file_path()
                .map(|path| PictureSource::Resource(path.into()))
                .map_err(|()| "无效的本地图片路径".into())
        } else {
            Ok(PictureSource::Resource(Resource::Uri(
                uri.to_string().into(),
            )))
        };
    }
    let path = url::Url::from_directory_path(directory)
        .ok()
        .and_then(|base| base.join(uri).ok())
        .and_then(|url| url.to_file_path().ok())
        .unwrap_or_else(|| directory.join(uri));
    Ok(PictureSource::Resource(path.into()))
}

fn load_image(
    source: &PictureSource,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, String>> {
    match source {
        PictureSource::Resource(resource) => {
            let mut provider = retain_all("media-viewer-image");
            provider
                .provide(window, cx)
                .load(resource, window, cx)
                .map(|result| result.map_err(|error| error.to_string()))
        }
        PictureSource::Bytes(image) => crate::raster::visible_result(image, window, cx)
            .map(|result| result.map_err(|error| error.to_string())),
    }
}

fn load_reading_image(
    source: &PictureSource,
    node: &MarkdownNode,
    window: &mut Window,
    cx: &mut App,
) -> Option<Result<Arc<RenderImage>, String>> {
    match source {
        PictureSource::Resource(resource) => {
            // Element state owns this cache: leaving a virtual row releases its decoded pixels
            // and atlas entries rather than accumulating every visited image in App's cache.
            let mut provider = retain_all((
                "reading-image",
                node.source_range().map_or(0, |range| range.start),
            ));
            let cache = provider.provide(window, cx);
            cache
                .load(resource, window, cx)
                .map(|result| result.map_err(|error| error.to_string()))
        }
        PictureSource::Bytes(image) => crate::raster::visible_result(image, window, cx)
            .map(|result| result.map_err(|error| error.to_string())),
    }
}

fn preview(
    node: &MarkdownNode,
    inline_height: Option<f32>,
    window: &mut Window,
    cx: &mut App,
) -> gpui_kit::AnyElement {
    let Some(image) = node.data::<MarkdownImage>() else {
        return div().into_any_element();
    };
    let source = match &image.source {
        Ok(source) => source,
        Err(error) => return image_error(error).into_any_element(),
    };
    let loaded = load_reading_image(source, node, window, cx);
    if let Some(Err(error)) = &loaded {
        return image_error(error).into_any_element();
    }
    let dimensions = loaded
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .map_or((240., 160.), |image| {
            let size = image.size(0);
            (size.width.0 as f32, size.height.0 as f32)
        });
    let explicit = match (image.width, image.height) {
        (Some(width), Some(height)) => Some((width, height)),
        (Some(width), None) => Some((width, width * dimensions.1 / dimensions.0)),
        (None, Some(height)) => Some((height * dimensions.0 / dimensions.1, height)),
        (None, None) => None,
    };
    let dimensions = explicit.unwrap_or(dimensions);
    let scale = inline_height
        .filter(|_| explicit.is_none())
        .map_or(1., |height| height / dimensions.1);
    let width = dimensions.0 * scale;
    let height = dimensions.1 * scale;
    let image = image.clone();
    let pixels = loaded.and_then(Result::ok);
    #[cfg(target_os = "macos")]
    let system_image = image.clone();
    #[cfg(target_os = "macos")]
    let system_pixels = pixels.clone();
    let target = div()
        .id(node.source_range().map_or(0, |range| range.start))
        .debug_selector(|| "image-preview".into())
        .relative()
        .cursor_pointer()
        .when(inline_height.is_some(), |target| {
            target.w(px(width)).h(px(height)).max_w_full()
        })
        .when(inline_height.is_none(), |target| {
            target
                .w_full()
                .max_w(px(width))
                .aspect_ratio(width / height)
        })
        .when_some(pixels.as_ref(), |target, pixels| {
            target.child(
                img(pixels.clone())
                    .size_full()
                    .with_fallback(|| image_error("图片加载失败").into_any_element()),
            )
        })
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, window, cx| {
            window.prevent_default();
            gpui_kit::base::GlobalState::suppress_text_selection(cx);
        })
        .on_click(move |_, window, cx| {
            // Quick Look may receive the mouse-up after it becomes key. End the
            // document gesture before handing focus to either preview window.
            gpui_kit::base::TextSelection::clear(window, cx);
            cx.stop_propagation();
            open(image.clone(), pixels.as_ref(), window, cx);
        });
    #[cfg(target_os = "macos")]
    let target = {
        use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
        target.context_menu(move |menu, _, _| {
            let input = quicklook_input(&system_image, system_pixels.as_ref());
            let image = system_image.clone();
            let pixels = system_pixels.clone();
            menu.item(
                PopupMenuItem::new("系统快速查看")
                    .disabled(input.is_none())
                    .on_click(move |_, window, cx| {
                        gpui_kit::base::TextSelection::clear(window, cx);
                        if let Some(input) = quicklook_input(&image, pixels.as_ref()) {
                            crate::quicklook::show(
                                input,
                                |error, _| tracing::warn!(%error, "system preview unavailable"),
                                cx,
                            );
                        }
                    }),
            )
        })
    };
    if inline_height.is_some() {
        target.into_any_element()
    } else {
        h_flex()
            .w_full()
            .justify_center()
            .py_3()
            .child(target)
            .into_any_element()
    }
}

fn image_error(error: &str) -> impl IntoElement {
    div()
        .debug_selector(|| "image-error".into())
        .p_3()
        .child(format!("图片加载失败：{error}"))
}

fn open(
    image: MarkdownImage,
    pixels: Option<&Arc<RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    open_viewer(image, pixels.cloned(), window, cx);
}

#[cfg(target_os = "macos")]
fn quicklook_input(
    image: &MarkdownImage,
    pixels: Option<&Arc<RenderImage>>,
) -> Option<crate::quicklook::Input> {
    let source = image.source.as_ref().ok()?;
    Some(match source {
        PictureSource::Resource(Resource::Path(path)) => {
            crate::quicklook::Input::File(path.to_path_buf())
        }
        PictureSource::Bytes(encoded) => crate::quicklook::Input::Encoded {
            image: encoded.clone(),
            title: image.alt.clone(),
        },
        PictureSource::Resource(_) => {
            // Asset lookup needs a rendering view. A click has none: reuse the full
            // pixels owned by the visible element instead of starting another decode.
            let image_pixels = pixels?.clone();
            // Preserve animated sources in the portable viewer instead of silently flattening them.
            if image_pixels.frame_count() != 1 {
                return None;
            }
            crate::quicklook::Input::Pixels {
                image: image_pixels,
                title: image.alt.clone(),
            }
        }
    })
}

fn open_viewer(
    image: MarkdownImage,
    pixels: Option<Arc<RenderImage>>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = if image.alt.is_empty() {
        "图片预览".to_owned()
    } else {
        image.alt.clone()
    };
    let options = viewer::options(&title, window, cx);
    if let Err(error) = gpui_kit::open_window(options, cx, move |window, cx| {
        cx.new(|cx| ImageViewer::new(image, pixels, title.into(), window, cx))
    }) {
        tracing::error!(%error, "unable to open image viewer");
    }
}

#[derive(Clone, Copy, Debug)]
enum Scale {
    Fit,
    Fixed(f32),
}

pub(crate) struct ImageViewer {
    image: MarkdownImage,
    title: SharedString,
    focus: FocusHandle,
    scroll: ScrollHandle,
    pan: viewer::Pan,
    scale: Scale,
    current_scale: f32,
    available: Size<Pixels>,
    pixels: Option<Arc<RenderImage>>,
    _bounds: Subscription,
    _display: Subscription,
    _theme: Subscription,
}

impl ImageViewer {
    fn new(
        image: MarkdownImage,
        pixels: Option<Arc<RenderImage>>,
        title: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        cx.on_release(|view, cx| {
            if let Some(pixels) = view.pixels.take() {
                cx.drop_image(pixels, None);
            }
        })
        .detach();
        Self {
            image,
            title,
            focus,
            scroll: ScrollHandle::new(),
            pan: viewer::Pan::default(),
            scale: Scale::Fit,
            current_scale: 1.,
            available: Size::default(),
            pixels,
            _bounds: cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
            _display: cx.observe_window_display(window, |_, _, cx| cx.notify()),
            _theme: cx.observe_global::<Theme>(|_, cx| cx.notify()),
        }
    }

    fn zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        self.scale = Scale::Fixed((self.current_scale * factor).clamp(MIN_ZOOM, MAX_ZOOM));
        cx.notify();
    }

    fn reset(&mut self, scale: Scale, cx: &mut Context<Self>) {
        self.scale = scale;
        self.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
        cx.notify();
    }
}

impl Render for ImageViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.image.source.as_ref().ok();
        let loaded = self
            .pixels
            .as_ref()
            .map(|pixels| Ok(pixels.clone()))
            .or_else(|| source.and_then(|source| load_image(source, window, cx)));
        if let Some(Ok(pixels)) = &loaded {
            self.pixels = Some(pixels.clone());
        }
        let dimensions = loaded
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map_or((800., 450.), |image| {
                let size = image.size(0);
                (size.width.0 as f32, size.height.0 as f32)
            });
        self.current_scale = match self.scale {
            Scale::Fit => {
                viewer::fit_scale(dimensions, self.available).min(1. / window.scale_factor())
            }
            Scale::Fixed(scale) => scale,
        };
        let pixels = loaded
            .as_ref()
            .and_then(|result| result.as_ref().ok())
            .map(|image| ImageSource::from(image.clone()));
        let error = loaded
            .as_ref()
            .and_then(|result| result.as_ref().err())
            .or_else(|| self.image.source.as_ref().err());
        let weak = cx.entity().downgrade();
        let canvas = div()
            .id("image-canvas")
            .debug_selector(|| "image-canvas".into())
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .bg(cx.theme().background)
            .cursor(if self.pan.is_active() {
                gpui_kit::CursorStyle::ClosedHand
            } else {
                gpui_kit::CursorStyle::OpenHand
            })
            .on_mouse_down(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, event: &gpui_kit::MouseDownEvent, _, cx| {
                    view.pan.begin(event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .on_pinch(cx.listener(|view, event: &gpui_kit::PinchEvent, _, cx| {
                view.zoom((1. + event.delta).max(0.1), cx);
                cx.stop_propagation();
            }))
            .p(px(viewer::MEDIA_INSET))
            .child(viewer::measure_available(
                self.available,
                move |available, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        if view.available != available {
                            view.available = available;
                            cx.notify();
                        }
                    });
                },
            ))
            .child(if let Some(error) = error {
                image_error(error).into_any_element()
            } else if let Some(pixels) = pixels.filter(|_| self.available.width > px(0.)) {
                div()
                    .flex()
                    .min_w_full()
                    .min_h_full()
                    .child(
                        div().m_auto().flex_shrink_0().child(
                            img(pixels)
                                .id("enlarged-image")
                                .debug_selector(|| "enlarged-image".into())
                                .w(px(dimensions.0 * self.current_scale))
                                .h(px(dimensions.1 * self.current_scale))
                                .with_fallback(|| image_error("图片无法解码").into_any_element()),
                        ),
                    )
                    .into_any_element()
            } else {
                div().child("正在加载图片…").into_any_element()
            });
        let controls = viewer::toolbar(
            viewer::MediaKind::Image,
            self.current_scale * window.scale_factor() * 100.,
            (
                self.current_scale <= MIN_ZOOM,
                self.current_scale >= MAX_ZOOM,
            ),
            cx.listener(|view, action, window, cx| match action {
                viewer::ZoomAction::In => view.zoom(1.25, cx),
                viewer::ZoomAction::Out => view.zoom(0.8, cx),
                viewer::ZoomAction::Fit => view.reset(Scale::Fit, cx),
                viewer::ZoomAction::Original => {
                    view.reset(Scale::Fixed(1. / window.scale_factor()), cx)
                }
            }),
            cx,
        );
        div()
            .size_full()
            .key_context("ImageViewer")
            .track_focus(&self.focus)
            .on_mouse_move(
                cx.listener(|view, event: &gpui_kit::MouseMoveEvent, _, cx| {
                    if view.pan.is_active() {
                        view.pan.advance(event, &view.scroll);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.pan.end();
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                gpui_kit::MouseButton::Left,
                cx.listener(|view, _, _, cx| {
                    view.pan.end();
                    cx.notify();
                }),
            )
            .on_action(cx.listener(|_, _: &CloseImage, window, _| window.remove_window()))
            .on_action(cx.listener(|view, _: &ZoomImageIn, _, cx| view.zoom(1.25, cx)))
            .on_action(cx.listener(|view, _: &ZoomImageOut, _, cx| view.zoom(0.8, cx)))
            .on_action(cx.listener(|view, _: &FitImage, _, cx| view.reset(Scale::Fit, cx)))
            .child(viewer::frame(self.title.clone(), canvas, controls, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    use gpui_kit::ImgResourceLoader;
    use gpui_kit::{
        Bounds, Entity, KeyBinding, TestAppContext, VisualTestContext, WindowBounds, WindowOptions,
        base::TextView, component::text::TextViewState, point, size,
    };

    struct ImageDocument {
        text: Entity<TextViewState>,
        extensions: MarkdownExtensions,
    }
    impl Render for ImageDocument {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().relative().size_full().child(
                TextView::new(&self.text)
                    .markdown_extensions(self.extensions.clone())
                    .selectable(true),
            )
        }
    }

    fn draw(window: &mut Window, cx: &mut App) {
        window.refresh();
        window.draw(cx).clear(cx);
    }

    fn settle(visual: &mut VisualTestContext) {
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(draw);
        }
    }

    #[cfg(target_os = "macos")]
    #[gpui_kit::test]
    async fn remote_quicklook_reuses_full_pixels_without_a_rendering_view(cx: &mut TestAppContext) {
        use gpui_kit::Asset as _;
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("full-size.png");
        let mut encoder = png::Encoder::new(std::fs::File::create(&path).unwrap(), 3780, 2380);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&vec![255; 3780 * 2380 * 3])
            .unwrap();
        let load = cx.update(|cx| {
            let load: std::pin::Pin<
                Box<
                    dyn Future<Output = Result<Arc<RenderImage>, gpui_kit::ImageCacheError>> + Send,
                >,
            > = Box::pin(ImgResourceLoader::load(path.into(), cx));
            load
        });
        let pixels = load.await.unwrap();
        let image = MarkdownImage {
            source: Ok(PictureSource::Resource(Resource::Uri(
                "https://example.com/image.png".into(),
            ))),
            alt: "network image".into(),
            width: None,
            height: None,
        };
        // No Window exists here, matching the absence of a rendered view during mouse callbacks.
        let Some(crate::quicklook::Input::Pixels {
            image: exported, ..
        }) = quicklook_input(&image, Some(&pixels))
        else {
            panic!("a decoded network image must use the pixel snapshot");
        };
        assert!(
            Arc::ptr_eq(&pixels, &exported),
            "preview must reuse the complete decoded image"
        );
        assert_eq!(
            exported.size(0).width.0,
            3780,
            "original width is preserved"
        );
        assert_eq!(
            exported.size(0).height.0,
            2380,
            "original height is preserved"
        );
        assert!(
            quicklook_input(&image, None).is_none(),
            "an unfinished download uses the asynchronous fallback"
        );
    }

    #[gpui_kit::test]
    fn image_press_does_not_leave_selection_drag_when_release_is_lost(cx: &mut TestAppContext) {
        use gpui_kit::base::TextSelection;

        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(
            fixture.path().join("picture.svg"),
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="100"><rect width="300" height="100" fill="#555"/></svg>"##,
        )
        .unwrap();
        let source = "![picture](picture.svg)\n\nText after the image remains selectable.\n";
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| ImageDocument {
                    text: cx.new(|cx| TextViewState::markdown(source, cx)),
                    extensions: extensions(MarkdownExtensions::default(), source, fixture.path()),
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        settle(&mut visual);
        let image = visual.debug_bounds("image-preview").unwrap();
        let text_start = point(px(2.), image.bottom() + px(30.));
        let text_end = point(px(240.), text_start.y);
        visual.simulate_mouse_down(
            image.center(),
            gpui_kit::MouseButton::Left,
            Default::default(),
        );
        // A native preview can consume the release. Returning must not extend a blank-space anchor.
        visual.simulate_mouse_move(text_end, None, Default::default());
        settle(&mut visual);
        visual.update(|window, cx| {
            assert!(
                TextSelection::selected_text(window, cx).is_empty(),
                "hovering after an image press must not select document text even without mouse-up"
            );
        });
        visual.simulate_mouse_up(text_end, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_down(text_start, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_move(text_end, gpui_kit::MouseButton::Left, Default::default());
        visual.simulate_mouse_up(text_end, gpui_kit::MouseButton::Left, Default::default());
        settle(&mut visual);
        visual.update(|window, cx| {
            assert!(
                !TextSelection::selected_text(window, cx).is_empty(),
                "a subsequent deliberate text drag must still select normally"
            );
        });
    }

    #[gpui_kit::test]
    fn image_click_zoom_close_and_hover_preserve_cached_pixels(cx: &mut TestAppContext) {
        let fixture = tempfile::tempdir().unwrap();
        let path = fixture.path().join("test picture.svg");
        std::fs::write(&path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="1600" height="800"><rect width="1600" height="800" fill="#237e79"/></svg>"##,
        ).unwrap();
        let source = "![测试图片][picture]\n\n[picture]: test%20picture.svg\n\n末尾文字\n";
        let (handle, root) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            cx.bind_keys([KeyBinding::new("escape", CloseImage, Some("ImageViewer"))]);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1000.), px(800.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    cx.new(|cx| ImageDocument {
                        text: cx.new(|cx| TextViewState::markdown(source, cx)),
                        extensions: extensions(
                            MarkdownExtensions::default(),
                            source,
                            fixture.path(),
                        ),
                    })
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        settle(&mut visual);
        assert!(
            !cx.read(|cx| ImageSource::from(path.clone()).is_asset_cached(cx)),
            "reading images use a scoped cache"
        );
        let preview = visual
            .debug_bounds("image-preview")
            .expect("reference image must render a click target");
        assert_eq!(
            preview.size.width,
            preview.size.height * 2.,
            "image keeps its original aspect ratio"
        );
        visual.simulate_resize(size(px(560.), px(800.)));
        settle(&mut visual);
        let narrow = visual.debug_bounds("image-preview").unwrap();
        assert!(
            narrow.size.width < preview.size.width,
            "block images shrink with actual reading width"
        );
        assert!(
            (f32::from(narrow.size.width - narrow.size.height * 2.)).abs() < 1.,
            "narrowing never squashes an image"
        );
        visual.simulate_resize(size(px(1000.), px(800.)));
        settle(&mut visual);
        visual.simulate_mouse_move(preview.center(), None, gpui_kit::Modifiers::default());
        visual.update(draw);
        assert_eq!(
            visual.debug_bounds("image-preview"),
            Some(preview),
            "hover and redraw keep the image hit target stable"
        );
        visual.simulate_click(preview.center(), gpui_kit::Modifiers::default());
        settle(&mut visual);
        let viewer_handle = cx
            .windows()
            .into_iter()
            .find(|viewer| *viewer != handle)
            .expect("click opens an independent image window");
        let mut visual = VisualTestContext::from_window(viewer_handle, cx);
        settle(&mut visual);
        for dimensions in [size(px(640.), px(700.)), size(px(1200.), px(800.))] {
            visual.simulate_resize(dimensions);
            settle(&mut visual);
            let canvas = visual.debug_bounds("image-canvas").unwrap();
            let image = visual.debug_bounds("enlarged-image").unwrap();
            assert!(
                image.size.width <= canvas.size.width - px(viewer::MEDIA_INSET * 2.) + px(1.)
                    && image.size.height
                        <= canvas.size.height - px(viewer::MEDIA_INSET * 2.) + px(1.),
                "the full image fits both axes after resizing, without dragging"
            );
        }
        let initial = visual.debug_bounds("enlarged-image").unwrap();
        let viewer = visual.update(|window, cx| {
            window
                .root::<gpui_kit::base::Root>()
                .flatten()
                .unwrap()
                .read(cx)
                .view()
                .clone()
                .downcast::<ImageViewer>()
                .unwrap()
        });
        let cached = visual.update(|_, cx| viewer.read(cx).pixels.clone().unwrap());
        let canvas = visual.debug_bounds("image-canvas").unwrap();
        let toolbar = visual.debug_bounds("image-toolbar").unwrap();
        assert!(
            toolbar.top() >= canvas.bottom(),
            "image controls stay below the canvas, like Mermaid"
        );
        visual.simulate_event(gpui_kit::PinchEvent {
            position: canvas.center(),
            delta: 0.1,
            ..Default::default()
        });
        settle(&mut visual);
        assert!(
            visual.debug_bounds("enlarged-image").unwrap().size.width > initial.size.width,
            "trackpad pinch enlarges the image through the shared preview gesture"
        );
        let plus = visual.debug_bounds("image-plus").unwrap();
        visual.simulate_click(plus.center(), gpui_kit::Modifiers::default());
        settle(&mut visual);
        let enlarged = visual.debug_bounds("enlarged-image").unwrap();
        assert!(
            enlarged.size.width > initial.size.width,
            "zoom button enlarges the actual drawn image"
        );
        visual.simulate_mouse_down(
            canvas.center(),
            gpui_kit::MouseButton::Left,
            Default::default(),
        );
        visual.simulate_mouse_move(
            canvas.center() - point(px(120.), px(0.)),
            gpui_kit::MouseButton::Left,
            Default::default(),
        );
        settle(&mut visual);
        visual.update(|_, cx| {
            assert!(
                viewer.read(cx).scroll.offset().x < px(0.),
                "an enlarged image pans with a pointer drag"
            );
            assert!(
                Arc::ptr_eq(&cached, viewer.read(cx).pixels.as_ref().unwrap()),
                "zoom and pan preserve the same decoded pixels"
            );
        });
        // Switching windows may lose mouse-up; an unpressed move must cancel panning.
        visual.simulate_mouse_move(canvas.center(), None, Default::default());
        settle(&mut visual);
        visual.update(|_, cx| assert!(!viewer.read(cx).pan.is_active()));
        visual.simulate_mouse_up(
            canvas.center(),
            gpui_kit::MouseButton::Left,
            Default::default(),
        );
        visual.update(|_, cx| {
            assert!(
                !ImageSource::from(path.clone()).is_asset_cached(cx),
                "zoom reuses visible pixels without creating a global source cache"
            );
            assert_eq!(
                root.read(cx).text.read(cx).rendered_text().source(),
                source,
                "opening a preview preserves Markdown source"
            );
        });
        visual.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(
            !cx.windows().contains(&viewer_handle),
            "Escape closes only the media window"
        );
        assert!(cx.windows().contains(&handle), "the document remains open");
        let mut document_visual = VisualTestContext::from_window(handle, cx);
        document_visual.simulate_mouse_move(
            point(px(300.), preview.bottom() + px(40.)),
            None,
            Default::default(),
        );
        settle(&mut document_visual);
        document_visual.update(|window, cx| {
            assert!(
                gpui_kit::base::TextSelection::selected_text(window, cx).is_empty(),
                "returning from preview must not select text on pointer movement"
            );
        });
        assert!(
            !cx.read(|cx| ImageSource::from(path.clone()).is_asset_cached(cx)),
            "closing image previews releases their global source"
        );
    }

    #[gpui_kit::test]
    #[ignore = "set PECAN_VERIFY_DOCUMENT to a Markdown file with a local image"]
    fn external_document_image_fits_without_panning(cx: &mut TestAppContext) {
        let path = std::path::PathBuf::from(
            std::env::var_os("PECAN_VERIFY_DOCUMENT").expect("provide a local validation document"),
        );
        let source = std::fs::read_to_string(&path).unwrap();
        let mut image_start = None;
        let mut image_range = None;
        for (event, range) in Parser::new(&source).into_offset_iter() {
            match event {
                pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { .. }) => {
                    image_start = Some(range.start);
                }
                pulldown_cmark::Event::End(pulldown_cmark::TagEnd::Image) => {
                    image_range = Some(image_start.unwrap()..range.end);
                    break;
                }
                _ => {}
            }
        }
        let image_source = &source[image_range.expect("validation document must contain an image")];
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            cx.bind_keys([KeyBinding::new("escape", CloseImage, Some("ImageViewer"))]);
            crate::install_fullscreen_escape(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| ImageDocument {
                    text: cx.new(|cx| TextViewState::markdown(image_source, cx)),
                    extensions: extensions(
                        MarkdownExtensions::default(),
                        &source,
                        path.parent().unwrap(),
                    ),
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        settle(&mut visual);
        let target = visual
            .debug_bounds("image-preview")
            .expect("the document image loads");
        visual.simulate_click(target.center(), gpui_kit::Modifiers::default());
        let viewer = cx
            .windows()
            .into_iter()
            .find(|other| *other != handle)
            .unwrap();
        let mut visual = VisualTestContext::from_window(viewer, cx);
        visual.update(|window, _| window.set_scale_factor(2.));
        for dimensions in [size(px(1512.), px(950.)), size(px(640.), px(700.))] {
            visual.simulate_resize(dimensions);
            settle(&mut visual);
            let canvas = visual.debug_bounds("image-canvas").unwrap();
            let image = visual.debug_bounds("enlarged-image").unwrap();
            assert!(
                canvas.contains(&image.origin) && canvas.contains(&image.bottom_right()),
                "the real document image is entirely visible: image {image:?}, canvas {canvas:?}"
            );
            visual.update(|window, cx| {
                let root = window.root::<gpui_kit::base::Root>().flatten().unwrap();
                let view = root
                    .read(cx)
                    .view()
                    .clone()
                    .downcast::<ImageViewer>()
                    .unwrap();
                let source = view.read(cx).image.source.as_ref().unwrap().clone();
                let PictureSource::Resource(resource) = source else {
                    panic!("validation fixture must reference a local image file");
                };
                let _ = resource;
                let pixels = view.read(cx).pixels.clone().unwrap();
                assert!(
                    f32::from(image.size.width) * window.scale_factor()
                        <= pixels.size(0).width.0 as f32 + 1.,
                    "default fitting never stretches bitmap pixels beyond their original resolution"
                );
            });
        }
        visual.update(|window, _| window.toggle_fullscreen());
        settle(&mut visual);
        visual.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(
            !cx.windows().contains(&viewer),
            "Escape closes the fullscreen image viewer"
        );
        assert!(cx.windows().contains(&handle), "the document stays open");
    }

    #[gpui_kit::test]
    fn broken_data_image_shows_an_error_without_staying_in_loading_state(cx: &mut TestAppContext) {
        let source = "![invalid](data:image/png;base64,bm90IGEgcG5n)";
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| ImageDocument {
                    text: cx.new(|cx| TextViewState::markdown(source, cx)),
                    extensions: extensions(MarkdownExtensions::default(), source, Path::new("")),
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        settle(&mut visual);
        assert!(visual.debug_bounds("image-error").is_some());
        assert!(visual.debug_bounds("image-preview").is_none());
    }

    #[gpui_kit::test]
    fn inline_linked_data_image_remains_clickable(cx: &mut TestAppContext) {
        let source = "before [![badge](data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='40'%20height='20'%3E%3C/svg%3E)](https://example.com) after";
        let (handle, root) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|cx| ImageDocument {
                    text: cx.new(|cx| TextViewState::markdown(source, cx)),
                    extensions: extensions(
                        MarkdownExtensions::default(),
                        source,
                        Path::new("/tmp"),
                    ),
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        settle(&mut visual);
        let bounds = visual.debug_bounds("image-preview").unwrap();
        assert!(
            bounds.size.width < px(100.),
            "inline images stay within the surrounding text line"
        );
        visual.update(|_, cx| {
            let rendered = root.read(cx).text.read(cx).rendered_text();
            assert!(rendered.as_str().contains("before") && rendered.as_str().contains("after"));
            assert_eq!(rendered.source(), source);
        });
        visual.simulate_click(bounds.center(), gpui_kit::Modifiers::default());
        settle(&mut visual);
        assert_eq!(
            cx.windows().len(),
            2,
            "a linked image opens an independent viewer"
        );
    }

    #[test]
    fn fitting_handles_portrait_small_images_and_narrow_windows() {
        let scale = viewer::fit_scale((1000., 4000.), size(px(800.), px(600.)));
        assert!(
            1000. * scale <= 800. && 4000. * scale <= 600.,
            "portrait image fits both axes"
        );
        assert!(
            viewer::fit_scale((1000., 4000.), size(px(-10.), px(0.))) > 0.,
            "minimized layouts must keep positive dimensions"
        );
        assert!(resolve_source("data:image/unknown,bytes", Path::new("/tmp")).is_err());
        let html = html_image(
            r#"<IMG SRC='file.svg?a=1&amp;b=2' alt='图 &amp; 表' width='240' height='120'>"#,
        )
        .unwrap();
        assert_eq!(
            html.uri, "file.svg?a=1&b=2",
            "HTML entities in image URLs are decoded"
        );
        assert_eq!(html.alt, "图 & 表");
        assert_eq!((html.width, html.height), (Some(240.), Some(120.)));
        assert!(
            html_image("<div>keep this text<img src='file.svg'></div>").is_none(),
            "mixed HTML must keep its text"
        );
    }
}
