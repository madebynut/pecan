use gpui_kit::component::StyledExt as _;
use std::{sync::Arc, time::Duration};

use gpui_kit::{
    AnyElement, Bounds, ClipboardItem, Context, Entity, FocusHandle, Image, ImageFormat,
    IntoElement, Pixels, Render, RenderImage, ScrollHandle, Size, Subscription, Task, Window,
    component::{
        ActiveTheme, Disableable, IconName, Selectable, Theme, h_flex,
        input::{Editor, EditorState},
        menu::{DropdownMenu as _, PopupMenuItem},
        v_flex,
    },
    div, img,
    prelude::*,
    px,
};
use pecan_mermaid::{Appearance, Diagram, DiagramFonts, Region};

use crate::{preferences::Preferences, ui::tool_button, viewer};

gpui_kit::actions!(pecan_diagram, [CloseDiagram, ZoomIn, ZoomOut]);

/// Inline diagrams use the reading width; vertical space follows their aspect ratio.
const CARD_INSET: f32 = 16.;
/// Device pixels rendered around the visible part of an enlarged diagram, so short
/// pans stay sharp while the next region renders.
const TILE_MARGIN: u32 = 256;

/// A raster drawn 1:1 with device pixels. Its logical size derives from whole
/// pixels, so the GPU's linear sampler never resamples it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Exact {
    region: Region,
    density: f32,
}

impl Exact {
    fn new(diagram: &Diagram, logical_width: f32, density: f32) -> Self {
        let width = (logical_width * density).round().max(1.);
        Self {
            region: Region::whole(diagram, width / diagram.dimensions().0),
            density,
        }
    }
    fn size(&self) -> Size<Pixels> {
        gpui_kit::size(
            px(self.region.width as f32 / self.density),
            px(self.region.height as f32 / self.density),
        )
    }
}

/// Whole rasters beyond the pixel budget render smaller and are scaled for display.
fn budgeted(diagram: &Diagram, region: Region) -> Region {
    if region.scale <= diagram.max_scale() {
        region
    } else {
        Region::whole(diagram, diagram.max_scale())
    }
}

/// Rasters are opaque on the canvas color: transparent edges would blend with
/// black texels under linear filtering, which shows as grey halos on light paper.
/// The newest decoded raster, else the one it replaces while decoding; once the
/// newest is ready the old one stops being drawn and `raster::visible` frees it.
fn current_pixels(
    image: &Option<Arc<Image>>,
    previous: &mut Option<Arc<Image>>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> Option<Arc<RenderImage>> {
    let current = image
        .as_ref()
        .and_then(|image| crate::raster::visible(image, window, cx));
    if current.is_some() {
        *previous = None;
        return current;
    }
    previous
        .as_ref()
        .and_then(|image| crate::raster::visible(image, window, cx))
}

fn canvas_rgb(cx: &gpui_kit::App) -> [u8; 3] {
    let color = cx.theme().background.to_rgb();
    [color.r, color.g, color.b].map(|channel| (channel * 255.).round() as u8)
}

pub(crate) use pecan_mermaid::RenderQueue;

pub(crate) struct DiagramCard {
    source: Arc<str>,
    queue: RenderQueue,
    appearance: Appearance,
    fonts: DiagramFonts,
    diagram: Option<Diagram>,
    image: Option<Arc<Image>>,
    /// Shown until `image` decodes, so resizing never flashes an empty card.
    previous: Option<Arc<Image>>,
    error: Option<String>,
    source_view: Option<Entity<EditorState>>,
    show_source: bool,
    copied: bool,
    available: Size<Pixels>,
    requested: Option<(Exact, [u8; 3])>,
    _job: Option<Task<()>>,
    _display: Subscription,
}

impl DiagramCard {
    pub(crate) fn new(
        source: Arc<str>,
        queue: RenderQueue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let appearance = if cx.theme().is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let fonts = Preferences::shared(cx).read(cx).data.diagram_fonts();
        let worker_fonts = fonts.clone();
        let worker_source = source.clone();
        let worker_queue = queue.clone();
        let background = cx.background_spawn(async move {
            let _permit = worker_queue.slots.acquire().await;
            worker_queue
                .renderer
                .render_with_fonts(&worker_source, appearance, &worker_fonts)
        });
        let job = cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |view, cx| {
                match result {
                    Ok(diagram) => view.diagram = Some(diagram),
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        });
        Self {
            source,
            queue,
            appearance,
            fonts,
            diagram: None,
            image: None,
            previous: None,
            error: None,
            source_view: None,
            show_source: false,
            copied: false,
            available: Size::default(),
            requested: None,
            _job: Some(job),
            _display: cx.observe_window_display(window, |_, _, cx| cx.notify()),
        }
    }
    fn export(&mut self, format: crate::export::DiagramFormat, cx: &mut Context<Self>) {
        let Some(diagram) = self.diagram.clone() else {
            return;
        };
        let background = canvas_rgb(cx);
        let weak = cx.weak_entity();
        crate::export::save_diagram(
            crate::export::DiagramInput {
                diagram,
                background,
                slots: self.queue.slots.clone(),
            },
            format,
            cx,
            move |result, cx| {
                let _ = weak.update(cx, |view, cx| {
                    if let Err(error) = result {
                        view.error = Some(format!("导出失败：{error:#}"));
                    }
                    cx.notify();
                });
            },
        );
    }
    fn rasterize(&mut self, exact: Exact, background: [u8; 3], cx: &mut Context<Self>) {
        let Some(diagram) = self.diagram.clone() else {
            return;
        };
        if self.requested == Some((exact, background)) {
            return;
        }
        // The first raster starts at once; later ones coalesce resize steps.
        let delay = if self.requested.is_none() {
            Duration::ZERO
        } else {
            Duration::from_millis(80)
        };
        self.requested = Some((exact, background));
        let slots = self.queue.slots.clone();
        let executor = cx.background_executor().clone();
        let background = cx.background_spawn(async move {
            executor.timer(delay).await;
            let _permit = slots.acquire().await;
            diagram.png_region(budgeted(&diagram, exact.region), Some(background))
        });
        self._job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |view, cx| {
                match result {
                    Ok(png) => {
                        let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                        view.previous = view.image.replace(image);
                    }
                    Err(error) => view.error = Some(error.to_string()),
                }
                cx.notify();
            });
        }));
    }
    fn current_pixels(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<RenderImage>> {
        current_pixels(&self.image, &mut self.previous, window, cx)
    }
    fn copy(&mut self, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(self.source.to_string()));
        self.copied = true;
        cx.notify();
    }
    fn source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_source = !self.show_source;
        if self.source_view.is_none() {
            self.source_view =
                Some(cx.new(|cx| crate::mermaid_source::editor(&self.source, window, cx)));
        }
        cx.notify();
    }
    fn expand(&self, window: &mut Window, cx: &mut Context<Self>) {
        let options = viewer::options("Mermaid 图表", window, cx);
        if let Err(error) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| DiagramViewer::new(self, window, cx))
        }) {
            tracing::error!(%error, "unable to open diagram viewer");
        }
    }
}

impl Render for DiagramCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = Preferences::shared(cx).read(cx).data.clone();
        let interface = crate::presentation::interface_style(cx);
        let exact = self.diagram.as_ref().map(|diagram| {
            let width = (f32::from(self.available.width) - CARD_INSET * 2.)
                .max(1.)
                .min(diagram.dimensions().0);
            Exact::new(diagram, width, window.scale_factor())
        });
        if let Some(exact) = exact
            && !self.show_source
            && self.available.width > px(0.)
        {
            self.rasterize(exact, canvas_rgb(cx), cx);
        }
        let body: AnyElement = if self.show_source {
            div()
                .debug_selector(|| "diagram-source-editor".into())
                .h(px(280.))
                .child(
                    Editor::new(
                        self.source_view
                            .as_ref()
                            .expect("source editor exists when source mode is open"),
                    )
                    .readonly(true)
                    .h_full(),
                )
                .into_any_element()
        } else if let Some(exact) = exact {
            let size = exact.size();
            let pixels = self.current_pixels(window, cx);
            let content: AnyElement = if let Some(pixels) = pixels {
                img(pixels)
                    .debug_selector(|| "inline-diagram".into())
                    .w(size.width)
                    .h(size.height)
                    .into_any_element()
            } else {
                div()
                    .w(size.width)
                    .h(size.height)
                    .text_color(cx.theme().muted_foreground)
                    .when(self.image.is_none(), |this| {
                        this.child(
                            self.error
                                .as_deref()
                                .unwrap_or("正在准备清晰图表…")
                                .to_owned(),
                        )
                    })
                    .into_any_element()
            };
            div()
                .id("diagram-preview")
                .debug_selector(|| "diagram-preview".into())
                .relative()
                .cursor_pointer()
                .w_full()
                .flex()
                .justify_center()
                .p(px(CARD_INSET))
                // The raster is baked onto this color; keep the surface identical.
                .bg(cx.theme().background)
                .child(content)
                .on_click(cx.listener(|view, _, window, cx| view.expand(window, cx)))
                .into_any_element()
        } else {
            div()
                .h(px(160.))
                .p_6()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(
                    self.error
                        .clone()
                        .unwrap_or_else(|| "正在后台生成图表…".into()),
                )
                .into_any_element()
        };
        let weak = cx.weak_entity();
        let export_weak = weak.clone();
        v_flex()
            .relative()
            .w_full()
            .child(viewer::measure_available(
                self.available,
                move |available, cx| {
                    let _ = weak.update(cx, |view, cx| {
                        view.available = available;
                        cx.notify();
                    });
                },
            ))
            .refine_style(&interface)
            .text_size(px(prefs.ui_size as f32))
            .rounded(px(4.))
            .border_1()
            .border_color(cx.theme().border)
            .overflow_hidden()
            .child(
                h_flex()
                    .debug_selector(|| "diagram-header".into())
                    .w_full()
                    .gap_1()
                    .px(px(12.))
                    .py(px(6.))
                    .line_height(px(16.))
                    .bg(crate::presentation::ReadingPalette::for_app(cx).wash)
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child("MERMAID"),
                    )
                    .child(div().flex_1())
                    .child(
                        tool_button(
                            "diagram-source",
                            if self.show_source {
                                gpui_kit::assets::IconName::Network
                            } else {
                                gpui_kit::assets::IconName::CodeXml
                            },
                            if self.show_source {
                                "查看图表"
                            } else {
                                "查看源码"
                            },
                        )
                        .debug_selector(|| "diagram-source".into())
                        .selected(self.show_source)
                        .on_click(cx.listener(|view, _, window, cx| view.source(window, cx))),
                    )
                    .child(
                        tool_button(
                            "diagram-copy",
                            if self.copied {
                                IconName::Check
                            } else {
                                IconName::Copy
                            },
                            if self.copied {
                                "已复制源码"
                            } else {
                                "复制源码"
                            },
                        )
                        .debug_selector(|| "diagram-copy".into())
                        .on_click(cx.listener(|view, _, _, cx| view.copy(cx))),
                    )
                    .child(
                        tool_button(
                            "diagram-export",
                            gpui_kit::assets::IconName::Download,
                            "导出图表",
                        )
                        .debug_selector(|| "diagram-export".into())
                        .disabled(self.diagram.is_none())
                        .dropdown_menu(move |mut menu, _, _| {
                            for (label, format) in [
                                ("导出 SVG…", crate::export::DiagramFormat::Svg),
                                ("导出高清 PNG…", crate::export::DiagramFormat::Png),
                            ] {
                                let weak = export_weak.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(
                                    move |_, _, cx| {
                                        let _ = weak.update(cx, |view, cx| view.export(format, cx));
                                    },
                                ));
                            }
                            menu
                        }),
                    )
                    .child(
                        tool_button("diagram-expand", IconName::Maximize, "放大查看")
                            .debug_selector(|| "diagram-expand".into())
                            .on_click(cx.listener(|view, _, window, cx| view.expand(window, cx))),
                    ),
            )
            .child(body)
    }
}

struct Tile {
    region: Region,
    pixels: Arc<RenderImage>,
}

/// Expands the visible device rectangle by a margin within the whole raster and the
/// pixel budget. Very large windows trim the margin first, then shrink the region
/// around the visible centre; the rest of the view shows the fitted backdrop.
fn tile_region(whole: Region, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Option<Region> {
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let expand = |margin: u32| {
        let (x, y) = (x0.saturating_sub(margin), y0.saturating_sub(margin));
        Region {
            x,
            y,
            width: (x1 + margin).min(whole.width) - x,
            height: (y1 + margin).min(whole.height) - y,
            ..whole
        }
    };
    let fitted = [TILE_MARGIN, TILE_MARGIN / 2, 0]
        .into_iter()
        .map(expand)
        .find(|region| region.within(Region::MAX_EDGE))
        .unwrap_or_else(|| {
            let visible = expand(0);
            let (w, h) = (f64::from(visible.width), f64::from(visible.height));
            // Flooring both sides keeps the product within the budget.
            let factor = (Region::MAX_PIXELS as f64 / (w * h)).sqrt().min(1.);
            let width = ((w * factor) as u32).min(Region::MAX_EDGE);
            let height = ((h * factor) as u32).min(Region::MAX_EDGE);
            Region {
                x: visible.x + (visible.width - width) / 2,
                y: visible.y + (visible.height - height) / 2,
                width,
                height,
                ..whole
            }
        });
    Some(fitted)
}

pub(crate) struct DiagramViewer {
    focus: FocusHandle,
    error: Option<String>,
    source: Arc<str>,
    queue: RenderQueue,
    appearance: Appearance,
    fonts: DiagramFonts,
    diagram: Option<Diagram>,
    loading: bool,
    /// The whole diagram at the fitted size: exact at 100%, and the backdrop for tiles.
    image: Option<Arc<Image>>,
    previous: Option<Arc<Image>>,
    requested: Option<(Exact, [u8; 3])>,
    /// The visible region at the current zoom, drawn 1:1 above the backdrop.
    tile: Option<Tile>,
    tile_request: Option<Region>,
    /// Visible part of the enlarged diagram, in its own logical coordinates.
    visible: Option<Bounds<Pixels>>,
    scroll: ScrollHandle,
    pan: viewer::Pan,
    zoom: f32,
    revision: u64,
    _job: Option<Task<()>>,
    _tile_job: Option<Task<()>>,
    available: Size<Pixels>,
    _bounds: Subscription,
    _display: Subscription,
    _theme: Subscription,
    _prefs: Subscription,
}

impl DiagramViewer {
    fn new(card: &DiagramCard, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let source = card.source.clone();
        let queue = card.queue.clone();
        let appearance = card.appearance;
        let fonts = card.fonts.clone();
        let diagram = card.diagram.clone();
        let prefs = Preferences::shared(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Self {
            focus,
            error: None,
            source,
            queue,
            appearance,
            fonts,
            diagram,
            loading: false,
            image: None,
            previous: None,
            requested: None,
            tile: None,
            tile_request: None,
            visible: None,
            scroll: ScrollHandle::new(),
            pan: viewer::Pan::default(),
            zoom: 1.,
            revision: 0,
            _job: None,
            _tile_job: None,
            available: Size::default(),
            _bounds: cx.observe_window_bounds(window, |_, _, cx| cx.notify()),
            _display: cx.observe_window_display(window, |_, _, cx| cx.notify()),
            _theme: cx.observe_global::<Theme>(|_, cx| cx.notify()),
            _prefs: cx.observe(&prefs, |_, _, cx| cx.notify()),
        }
    }
    /// Theme and font changes regenerate the diagram before any raster is requested.
    fn load_diagram(&mut self, cx: &mut Context<Self>) {
        if self.diagram.is_some() || self.loading {
            return;
        }
        self.loading = true;
        let revision = self.revision;
        let (source, queue, appearance, fonts) = (
            self.source.clone(),
            self.queue.clone(),
            self.appearance,
            self.fonts.clone(),
        );
        let background = cx.background_spawn(async move {
            let _permit = queue.slots.acquire().await;
            queue
                .renderer
                .render_with_fonts(&source, appearance, &fonts)
        });
        self._job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |view, cx| {
                if view.revision == revision {
                    view.loading = false;
                    match result {
                        Ok(diagram) => view.diagram = Some(diagram),
                        Err(error) => view.error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        }));
    }
    fn rasterize(&mut self, exact: Exact, background: [u8; 3], cx: &mut Context<Self>) {
        let Some(diagram) = self.diagram.clone() else {
            return;
        };
        if self.requested == Some((exact, background)) {
            return;
        }
        self.requested = Some((exact, background));
        self.revision += 1;
        let revision = self.revision;
        let queue = self.queue.clone();
        let executor = cx.background_executor().clone();
        let background = cx.background_spawn(async move {
            // Coalesce resize events before starting bounded CPU work.
            executor.timer(Duration::from_millis(80)).await;
            let _permit = queue.slots.acquire().await;
            diagram.png_region(budgeted(&diagram, exact.region), Some(background))
        });
        self._job = Some(cx.spawn(async move |weak, cx| {
            let result = background.await;
            let _ = weak.update(cx, |view, cx| {
                if view.revision == revision {
                    match result {
                        Ok(png) => {
                            let image = Arc::new(Image::from_bytes(ImageFormat::Png, png));
                            view.previous = view.image.replace(image);
                        }
                        Err(error) => view.error = Some(error.to_string()),
                    }
                    cx.notify();
                }
            });
        }));
    }
    /// Keeps the visible part of a zoomed diagram rendered at exactly one texel per device pixel.
    fn update_tile(&mut self, current: Exact, background: [u8; 3], cx: &mut Context<Self>) {
        let (Some(diagram), Some(visible)) = (self.diagram.clone(), self.visible) else {
            return;
        };
        let backdrop = self
            .requested
            .map(|(exact, _)| budgeted(&diagram, exact.region).scale);
        if backdrop == Some(current.region.scale) || visible.size.width <= px(0.) {
            return;
        }
        let whole = current.region;
        let device = |logical: Pixels, round: fn(f32) -> f32, limit: u32| {
            (round(f32::from(logical) * current.density).max(0.) as u32).min(limit)
        };
        let (x0, y0) = (
            device(visible.left(), f32::floor, whole.width),
            device(visible.top(), f32::floor, whole.height),
        );
        let (x1, y1) = (
            device(visible.right(), f32::ceil, whole.width),
            device(visible.bottom(), f32::ceil, whole.height),
        );
        let covers = |region: &Region| {
            region.scale == whole.scale
                && region.x <= x0
                && region.y <= y0
                && region.x + region.width >= x1
                && region.y + region.height >= y1
        };
        let Some(region) = tile_region(whole, (x0, y0, x1, y1)) else {
            return;
        };
        // A budget-trimmed target never covers the whole visible area, so also
        // accept a tile that already is the target; margins still absorb small pans.
        let reusable = |existing: &Region| *existing == region || covers(existing);
        if self
            .tile
            .as_ref()
            .is_some_and(|tile| reusable(&tile.region))
            || self.tile_request.as_ref().is_some_and(reusable)
        {
            return;
        }
        self.tile_request = Some(region);
        let queue = self.queue.clone();
        let executor = cx.background_executor().clone();
        let job = cx.background_spawn(async move {
            // Pinch and pan steps arrive per frame; render only where they settle.
            executor.timer(Duration::from_millis(50)).await;
            let _permit = queue.slots.acquire().await;
            diagram.bgra_region(region, background)
        });
        self._tile_job = Some(cx.spawn(async move |weak, cx| {
            let result = job.await;
            let _ = weak.update(cx, |view, cx| {
                if view.tile_request != Some(region) {
                    return;
                }
                view.tile_request = None;
                let Some(buffer) = result
                    .ok()
                    .and_then(|bgra| image::RgbaImage::from_raw(region.width, region.height, bgra))
                else {
                    return;
                };
                let pixels = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
                if let Some(old) = view.tile.replace(Tile { region, pixels }) {
                    cx.drop_image(old.pixels, None);
                }
                cx.notify();
            });
        }));
    }
    fn invalidate(&mut self, cx: &mut gpui_kit::App) {
        self.diagram = None;
        self.loading = false;
        self.image = None;
        self.previous = None;
        self.requested = None;
        if let Some(tile) = self.tile.take() {
            cx.drop_image(tile.pixels, None);
        }
        self.tile_request = None;
        self.error = None;
        self.revision += 1;
        self._job = None;
        self._tile_job = None;
    }
    fn zoom(&mut self, factor: f32, cx: &mut Context<Self>) {
        let next = (self.zoom * factor).clamp(0.25, 4.);
        if self.zoom == next {
            return;
        }
        self.zoom = next;
        cx.notify();
    }
}

impl Render for DiagramViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = if cx.theme().is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        };
        let prefs = Preferences::shared(cx);
        let data = &prefs.read(cx).data;
        let fonts_changed = self.fonts.english() != data.mermaid_english_font
            || self.fonts.chinese() != data.mermaid_chinese_font;
        if self.appearance != appearance || fonts_changed {
            // Invalidate on theme or font changes; ordinary redraws reuse the sharp raster.
            self.fonts = data.diagram_fonts();
            self.appearance = appearance;
            self.invalidate(cx);
        }
        self.load_diagram(cx);
        let background = cx.theme().background;
        let (w, h) = self
            .diagram
            .as_ref()
            .map_or((800., 400.), |d| d.dimensions());
        let fit = viewer::fit_scale((w, h), self.available);
        let density = window.scale_factor();
        let fitted = self
            .diagram
            .as_ref()
            .map(|diagram| Exact::new(diagram, fit * w, density));
        let current = self
            .diagram
            .as_ref()
            .map(|diagram| Exact::new(diagram, fit * self.zoom * w, density));
        if let (Some(fitted), Some(current)) = (fitted, current)
            && self.available.width > px(0.)
        {
            let rgb = canvas_rgb(cx);
            self.rasterize(fitted, rgb, cx);
            self.update_tile(current, rgb, cx);
        }
        let weak = cx.entity().downgrade();
        let mut canvas = div()
            .id("diagram-canvas")
            .debug_selector(|| "diagram-canvas".into())
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .track_scroll(&self.scroll)
            .bg(background)
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
            ));
        if let Some(current) = current
            && (self.image.is_some() || self.previous.is_some())
        {
            let pixels = current_pixels(&self.image, &mut self.previous, window, cx);
            let tile = self
                .tile
                .as_ref()
                .filter(|tile| tile.region.scale == current.region.scale)
                .map(|tile| {
                    let logical = |value: u32| px(value as f32 / density);
                    img(tile.pixels.clone())
                        .debug_selector(|| "enlarged-diagram-tile".into())
                        .absolute()
                        .left(logical(tile.region.x))
                        .top(logical(tile.region.y))
                        .w(logical(tile.region.width))
                        .h(logical(tile.region.height))
                });
            let probe = cx.entity().downgrade();
            canvas = canvas.child(
                div().flex().min_w_full().min_h_full().child(
                    div()
                        .m_auto()
                        .flex_shrink_0()
                        .relative()
                        .w(current.size().width)
                        .h(current.size().height)
                        .when_some(pixels, |this, pixels| {
                            this.child(
                                img(pixels)
                                    .debug_selector(|| "enlarged-diagram".into())
                                    .size_full(),
                            )
                        })
                        .children(tile)
                        .child(
                            gpui_kit::canvas(
                                move |bounds, window, cx| {
                                    // The scroll viewport's mask is the visible area.
                                    let shown = bounds.intersect(&window.content_mask().bounds);
                                    let local =
                                        Bounds::new(shown.origin - bounds.origin, shown.size);
                                    let _ = probe.update(cx, |view, cx| {
                                        if view.visible != Some(local) {
                                            view.visible = Some(local);
                                            cx.notify();
                                        }
                                    });
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .top_0()
                            .left_0()
                            .size_full(),
                        ),
                ),
            );
        } else {
            canvas = canvas.child(
                self.error
                    .clone()
                    .unwrap_or_else(|| "正在准备清晰图表…".into()),
            );
        }
        let controls = viewer::toolbar(
            viewer::MediaKind::Diagram,
            viewer::fit_scale((w, h), self.available) * self.zoom * window.scale_factor() * 100.,
            (self.zoom <= 0.25, self.zoom >= 4.),
            cx.listener(|view, action, window, cx| match action {
                viewer::ZoomAction::In => view.zoom(1.25, cx),
                viewer::ZoomAction::Out => view.zoom(0.8, cx),
                viewer::ZoomAction::Fit | viewer::ZoomAction::Original => {
                    view.scroll.set_offset(gpui_kit::point(px(0.), px(0.)));
                    let dimensions = view
                        .diagram
                        .as_ref()
                        .map_or((800., 400.), |d| d.dimensions());
                    view.zoom = match action {
                        viewer::ZoomAction::Original => (1.
                            / (viewer::fit_scale(dimensions, view.available)
                                * window.scale_factor()))
                        .clamp(0.25, 4.),
                        _ => 1.,
                    };
                    cx.notify();
                }
            }),
            cx,
        );
        div()
            .size_full()
            .key_context("DiagramViewer")
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
            .on_action(cx.listener(|_, _: &CloseDiagram, window, _| window.remove_window()))
            .on_action(cx.listener(|view, _: &ZoomIn, _, cx| view.zoom(1.25, cx)))
            .on_action(cx.listener(|view, _: &ZoomOut, _, cx| view.zoom(0.8, cx)))
            .child(viewer::frame("Mermaid 图表", canvas, controls, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{TestAppContext, VisualTestContext, WindowBounds, WindowOptions, point, size};

    struct DiagramDocument(Entity<DiagramCard>);
    impl Render for DiagramDocument {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().relative().size_full().child(self.0.clone())
        }
    }

    /// Captures the enlarged viewer through Metal with real glyphs, for visual sharpness review.
    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "set PECAN_VERIFY_DOCUMENT and PECAN_METAL_OUTPUT; needs a Metal device"]
    fn enlarged_viewer_metal_capture() {
        use gpui_kit::{AppContext as _, HeadlessAppContext};
        let path = std::env::var_os("PECAN_VERIFY_DOCUMENT").unwrap();
        let output = std::path::PathBuf::from(std::env::var_os("PECAN_METAL_OUTPUT").unwrap());
        let document =
            pecan_document::Document::parse(std::fs::read_to_string(path).unwrap()).unwrap();
        let code = document
            .blocks()
            .iter()
            .find_map(|block| match block {
                pecan_document::Block::Mermaid { code, .. } => Some(code.clone()),
                _ => None,
            })
            .unwrap();
        let mut headless = HeadlessAppContext::with_platform(
            Arc::new(gpui_apple::AppleTextSystem::new()),
            Arc::new(crate::assets::ReaderAssets),
            gpui_platform::current_headless_renderer,
        );
        headless.allow_parking();
        let (card_window, document) = headless.update(|cx| {
            gpui_kit::init(cx);
            crate::fonts::install(cx);
            cx.set_reduce_motion(true);
            let prefs = Preferences::install(None, cx);
            prefs.update(cx, |prefs, cx| {
                prefs.change(|data| data.theme = "light".into(), cx)
            });
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let card = cx.new(|cx| DiagramCard::new(code, RenderQueue::new(), window, cx));
                cx.new(|_| DiagramDocument(card))
            })
            .unwrap()
        });
        let settle = |headless: &mut HeadlessAppContext| {
            for _ in 0..8 {
                headless.run_until_parked();
                headless.advance_clock(Duration::from_millis(120));
                for window in headless.update(|cx| cx.windows()) {
                    let _ = headless.update_window(window, |_, window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                }
            }
        };
        settle(&mut headless);
        headless
            .update_window(card_window, |_, window, cx| {
                let card = document.read(cx).0.clone();
                card.update(cx, |card, cx| card.expand(window, cx));
            })
            .unwrap();
        let viewer_window = headless
            .update(|cx| cx.windows())
            .into_iter()
            .find(|window| *window != card_window)
            .unwrap();
        for scale in [1., 2.] {
            headless
                .update_window(viewer_window, |_, window, _| {
                    window.set_scale_factor(scale);
                    window.resize(size(px(1200.), px(800.)));
                })
                .unwrap();
            for zoom in [1., 2.5] {
                headless
                    .update_window(viewer_window, |root, _, cx| {
                        let root = root.downcast::<gpui_kit::base::Root>().unwrap();
                        let view = root
                            .read(cx)
                            .view()
                            .clone()
                            .downcast::<DiagramViewer>()
                            .unwrap();
                        view.update(cx, |view, cx| {
                            view.zoom = zoom;
                            cx.notify();
                        });
                    })
                    .unwrap();
                settle(&mut headless);
                let image = headless.capture_screenshot(viewer_window).unwrap();
                image
                    .save(output.join(format!("viewer-light-{scale}x-zoom{zoom}.png")))
                    .unwrap();
            }
        }
    }

    #[gpui_kit::test]
    fn a_completed_budget_trimmed_tile_is_not_requested_again(cx: &mut TestAppContext) {
        let (handle, document) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let card = cx.new(|cx| {
                    DiagramCard::new("graph LR; A-->B".into(), RenderQueue::new(), window, cx)
                });
                cx.new(|_| DiagramDocument(card))
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        visual.run_until_parked();
        visual.update(|window, cx| {
            let card = document.read(cx).0.clone();
            card.update(cx, |card, cx| card.expand(window, cx));
        });
        visual.run_until_parked();
        let viewer_window = cx.windows().into_iter().find(|w| *w != handle).unwrap();
        cx.update_window(viewer_window, |root, _, cx| {
            let root = root.downcast::<gpui_kit::base::Root>().unwrap();
            let viewer = root
                .read(cx)
                .view()
                .clone()
                .downcast::<DiagramViewer>()
                .unwrap();
            viewer.update(cx, |view, cx| {
                let diagram = view.diagram.clone().unwrap();
                // A 2560×1440 logical viewport at 2x exceeds the pixel budget at 3x zoom.
                let current = Exact::new(&diagram, 2560. * 3., 2.);
                view.requested = None;
                view.visible = Some(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(2560.), px(1440.)),
                ));
                view.update_tile(current, [255; 3], cx);
                let region = view
                    .tile_request
                    .take()
                    .expect("the visible region is requested");
                assert!(
                    u64::from(region.width) * u64::from(region.height) < 5120 * 2880,
                    "the fixture must exercise budget trimming: {region:?}"
                );
                let pixels = Arc::new(RenderImage::new([image::Frame::new(
                    image::RgbaImage::new(1, 1),
                )]));
                view.tile = Some(Tile { region, pixels });
                view.update_tile(current, [255; 3], cx);
                assert_eq!(
                    view.tile_request, None,
                    "an unchanged view reuses the completed trimmed tile instead of re-rendering it"
                );
            });
        })
        .unwrap();
    }

    #[test]
    fn tile_regions_add_margins_within_the_raster_and_budget() {
        let whole = Region {
            scale: 2.,
            x: 0,
            y: 0,
            width: 10_000,
            height: 6_000,
        };
        let region = tile_region(whole, (1000, 50, 3000, 1500)).unwrap();
        assert_eq!(
            (region.x, region.y, region.width, region.height),
            (
                1000 - TILE_MARGIN,
                0,
                2000 + TILE_MARGIN * 2,
                1500 + TILE_MARGIN
            ),
            "margins extend around the visible area but not past the raster"
        );
        assert_eq!(region.scale, whole.scale);
        let huge = tile_region(whole, (0, 0, 5120, 2880)).unwrap();
        assert!(
            u64::from(huge.width) * u64::from(huge.height) <= Region::MAX_PIXELS,
            "a 5K viewport is trimmed to the pixel budget: {huge:?}"
        );
        assert!(tile_region(whole, (10, 10, 10, 20)).is_none());
    }

    struct Toggle {
        card: Entity<DiagramCard>,
        shown: bool,
    }
    impl Render for Toggle {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .when(self.shown, |this| this.child(self.card.clone()))
        }
    }

    #[gpui_kit::test]
    fn cached_offscreen_cards_release_decoded_pixels_and_atlas(cx: &mut TestAppContext) {
        let (handle, toggle) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let card = cx.new(|cx| {
                    DiagramCard::new("graph LR; A-->B".into(), RenderQueue::new(), window, cx)
                });
                cx.new(|_| Toggle { card, shown: true })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        let settle = |visual: &mut VisualTestContext| {
            for _ in 0..6 {
                visual.run_until_parked();
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
        };
        settle(&mut visual);
        assert!(visual.debug_bounds("inline-diagram").is_some());
        let card = toggle.read_with(&visual, |toggle, _| toggle.card.clone());
        let (image, pixels) = visual.update(|window, cx| {
            let image = card.read(cx).image.clone().unwrap();
            let pixels = image.clone().get_render_image(window, cx).unwrap();
            assert!(window.has_image_atlas_entry(&pixels));
            (image, pixels)
        });
        visual.update(|_, cx| {
            toggle.update(cx, |toggle, cx| {
                toggle.shown = false;
                cx.notify();
            })
        });
        settle(&mut visual);
        visual.update(|window, cx| {
            assert!(
                Arc::ptr_eq(&image, card.read(cx).image.as_ref().unwrap()),
                "the resident card keeps only its compact PNG"
            );
            assert!(
                !image.is_asset_cached(cx),
                "decoded pixels are released offscreen"
            );
            assert!(
                !window.has_image_atlas_entry(&pixels),
                "GPU atlas entry is released"
            );
        });
        visual.update(|_, cx| {
            toggle.update(cx, |toggle, cx| {
                toggle.shown = true;
                cx.notify();
            })
        });
        settle(&mut visual);
        assert!(
            visual.debug_bounds("inline-diagram").is_some(),
            "returning to the card decodes the retained PNG again"
        );
    }

    #[gpui_kit::test]
    fn inline_diagrams_follow_content_width_and_display_density(cx: &mut TestAppContext) {
        let (handle, document) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                let card = cx.new(|cx| {
                    DiagramCard::new(
                        "sequenceDiagram\n participant A as 客户端\n participant B as 接入服务\n participant C as 消息队列\n participant D as 存储服务\n participant E as 查询服务\n participant F as 管理服务\n participant G as 远控服务\n participant H as 终端组件\n A->>B: 上报结果\n B->>C: 发布定位消息\n C->>D: 订阅并存储\n E->>D: 查询最新位置\n".into(),
                        RenderQueue::new(), window, cx,
                    )
                });
                cx.new(|_| DiagramDocument(card))
            }).unwrap()
        });
        let card = document.read_with(cx, |doc, _| doc.0.clone());
        let mut visual = VisualTestContext::from_window(handle, cx);
        for (width, density) in [(900., 2.), (1180., 2.), (900., 3.)] {
            // Resizing re-reads the platform scale, so apply the density afterwards.
            visual.simulate_resize(size(px(width), px(900.)));
            visual.update(|window, _| window.set_scale_factor(density));
            for _ in 0..8 {
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_millis(100));
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            let image = visual.debug_bounds("inline-diagram").unwrap();
            assert!(
                image.size.width > px(width - 80.),
                "wide diagrams should use the content width instead of a fixed thumbnail: {image:?}"
            );
            visual.update(|_, cx| {
                let view = card.read(cx);
                let bytes = view.image.as_ref().unwrap().bytes();
                let raster_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                let raster_height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                assert_eq!(
                    (raster_width as f32, raster_height as f32),
                    (
                        f32::from(image.size.width) * density,
                        f32::from(image.size.height) * density
                    ),
                    "inline PNG maps one texel to each device pixel, so the GPU never resamples it"
                );
            });
        }
        visual.update(|window, cx| {
            let image = card.read(cx).image.clone().unwrap();
            window.refresh();
            window.draw(cx).clear(cx);
            assert!(
                Arc::ptr_eq(&image, card.read(cx).image.as_ref().unwrap()),
                "ordinary redraws reuse the raster"
            );
        });
    }

    #[gpui_kit::test]
    fn rendered_mermaid_opens_a_sharp_resizable_window(cx: &mut TestAppContext) {
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            cx.bind_keys([gpui_kit::KeyBinding::new(
                "escape",
                CloseDiagram,
                Some("DiagramViewer"),
            )]);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(760.), px(500.)),
                    ))),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    let card = cx.new(|cx| {
                        DiagramCard::new("graph LR; A-->B".into(), RenderQueue::new(), window, cx)
                    });
                    cx.new(|_| DiagramDocument(card))
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
        let preview = visual
            .debug_bounds("diagram-preview")
            .expect("rendered Mermaid has a click target");
        let header = visual.debug_bounds("diagram-header").unwrap();
        let expand = visual.debug_bounds("diagram-expand").unwrap();
        assert_eq!(expand.size, size(px(28.), px(28.)));
        assert_eq!(header.right() - expand.right(), px(12.));
        assert_eq!(expand.top() - header.top(), px(6.));
        visual.simulate_mouse_move(preview.center(), None, gpui_kit::Modifiers::default());
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            visual.debug_bounds("diagram-preview"),
            Some(preview),
            "hover and redraw keep the Mermaid hit target stable"
        );
        let source = visual.debug_bounds("diagram-source").unwrap();
        visual.simulate_click(source.center(), gpui_kit::Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(
            visual.debug_bounds("diagram-source-editor").is_some()
                && visual.debug_bounds("diagram-preview").is_none(),
            "the document card provides the readonly source view"
        );
        visual.update(|window, cx| {
            let root = window.root::<gpui_kit::base::Root>().flatten().unwrap();
            let doc = root
                .read(cx)
                .view()
                .clone()
                .downcast::<DiagramDocument>()
                .unwrap();
            let editor = doc
                .read(cx)
                .0
                .read(cx)
                .source_view
                .as_ref()
                .unwrap()
                .read(cx);
            assert_eq!(
                editor.language_name().as_ref(),
                "mermaid",
                "the source pane installs Mermaid highlighting"
            );
        });
        let copy = visual.debug_bounds("diagram-copy").unwrap();
        visual.simulate_click(copy.center(), gpui_kit::Modifiers::default());
        visual.update(|_, cx| {
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().unwrap(),
                "graph LR; A-->B",
                "the compact header copy button preserves exact Mermaid source",
            )
        });
        let source = visual.debug_bounds("diagram-source").unwrap();
        visual.simulate_click(source.center(), gpui_kit::Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        let preview = visual.debug_bounds("diagram-preview").unwrap();
        visual.simulate_mouse_move(point(px(0.), px(0.)), None, gpui_kit::Modifiers::default());
        visual.simulate_click(preview.center(), gpui_kit::Modifiers::default());
        visual.run_until_parked();
        let viewer_handle = cx
            .windows()
            .into_iter()
            .find(|viewer| *viewer != handle)
            .expect("Mermaid opens an independent window");
        let mut visual = VisualTestContext::from_window(viewer_handle, cx);
        for _ in 0..4 {
            visual.run_until_parked();
            visual.executor().advance_clock(Duration::from_millis(100));
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
        for dimensions in [size(px(640.), px(600.)), size(px(1300.), px(800.))] {
            visual.simulate_resize(dimensions);
            for _ in 0..4 {
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_millis(100));
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            let canvas = visual.debug_bounds("diagram-canvas").unwrap();
            let image = visual.debug_bounds("enlarged-diagram").unwrap();
            let footer = visual.debug_bounds("diagram-footer").unwrap();
            let toolbar = visual.debug_bounds("diagram-toolbar").unwrap();
            assert!(
                footer.origin.y >= canvas.bottom_right().y
                    && footer.contains(&toolbar.origin)
                    && footer.contains(&toolbar.bottom_right()),
                "zoom controls stay below the canvas without covering the image"
            );
            for name in ["modal-minus", "modal-plus", "modal-fit"] {
                let control = visual.debug_bounds(name).unwrap();
                assert!(
                    toolbar.contains(&control.center()),
                    "{name} stays inside the compact footer at every window size"
                );
            }
            assert!(
                image.size.width <= canvas.size.width - px(viewer::MEDIA_INSET * 2.) + px(1.)
                    && image.size.height
                        <= canvas.size.height - px(viewer::MEDIA_INSET * 2.) + px(1.),
                "the entire Mermaid fits both axes without scrolling"
            );
            let physical_width =
                visual.update(|window, _| f32::from(image.size.width) * window.scale_factor());
            // The test diagram is small enough to stay below the raster caps.
            visual.update(|window, cx| {
                let root = window.root::<gpui_kit::base::Root>().flatten().unwrap();
                let viewer = root
                    .read(cx)
                    .view()
                    .clone()
                    .downcast::<DiagramViewer>()
                    .unwrap();
                let bytes = viewer.read(cx).image.as_ref().unwrap().bytes();
                let raster_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                assert_eq!(
                    raster_width as f32, physical_width,
                    "the fitted PNG maps one texel to each of {physical_width} device pixels"
                );
            });
        }
        for name in ["modal-graph", "modal-source", "modal-copy"] {
            assert!(
                visual.debug_bounds(name).is_none(),
                "the image viewer does not offer document source actions"
            );
        }
        let initial = visual.debug_bounds("enlarged-diagram").unwrap();
        let plus = visual.debug_bounds("modal-plus").unwrap();
        visual.simulate_click(plus.center(), gpui_kit::Modifiers::default());
        visual.run_until_parked();
        visual.executor().advance_clock(Duration::from_millis(100));
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert!(
            visual.debug_bounds("enlarged-diagram").unwrap().size.width > initial.size.width,
            "the icon zoom control enlarges the rendered diagram"
        );
        for _ in 0..4 {
            visual.run_until_parked();
            visual.executor().advance_clock(Duration::from_millis(100));
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
            });
        }
        let enlarged = visual.debug_bounds("enlarged-diagram").unwrap();
        let tile = visual
            .debug_bounds("enlarged-diagram-tile")
            .expect("zoomed diagrams draw their visible region at device resolution");
        let shown = enlarged.intersect(&visual.debug_bounds("diagram-canvas").unwrap());
        assert!(
            tile.contains(&shown.origin)
                && tile.contains(&(shown.bottom_right() - point(px(1.), px(1.)))),
            "the sharp region covers everything visible: {tile:?} vs {shown:?}"
        );
        visual.update(|window, cx| {
            let root = window.root::<gpui_kit::base::Root>().flatten().unwrap();
            let view = root
                .read(cx)
                .view()
                .clone()
                .downcast::<DiagramViewer>()
                .unwrap();
            let region = view.read(cx).tile.as_ref().unwrap().region;
            assert_eq!(
                (region.width as f32, region.height as f32),
                (
                    f32::from(tile.size.width) * window.scale_factor(),
                    f32::from(tile.size.height) * window.scale_factor()
                ),
                "one tile texel per device pixel"
            );
        });
        let fit = visual.debug_bounds("modal-fit").unwrap();
        visual.simulate_click(fit.center(), gpui_kit::Modifiers::default());
        visual.run_until_parked();
        visual.update(|window, cx| {
            window.refresh();
            window.draw(cx).clear(cx);
        });
        assert_eq!(
            visual.debug_bounds("enlarged-diagram").unwrap().size.width,
            initial.size.width,
            "fit restores the displayed diagram size"
        );
        let viewer = visual.update(|window, cx| {
            window
                .root::<gpui_kit::base::Root>()
                .flatten()
                .unwrap()
                .read(cx)
                .view()
                .clone()
                .downcast::<DiagramViewer>()
                .unwrap()
        });
        let mut previous_png = None;
        for (theme, expected) in [
            ("light", Appearance::Light),
            ("dark", Appearance::Dark),
            ("light", Appearance::Light),
        ] {
            let old_image = viewer.read_with(cx, |view, _| (view.appearance, view.image.clone()));
            visual.update(|_, cx| {
                let mut data = (*Preferences::shared(cx).read(cx).data).clone();
                data.theme = theme.into();
                crate::preferences::apply_theme(&data, cx);
            });
            for _ in 0..6 {
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_millis(100));
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            let (appearance, revision, png) = visual.update(|_, cx| {
                let view = viewer.read(cx);
                (
                    view.appearance,
                    view.revision,
                    view.image.as_ref().unwrap().bytes().to_vec(),
                )
            });
            assert_eq!(appearance, expected, "the open viewer follows {theme} mode");
            if old_image.0 != expected
                && let Some(old) = old_image.1
            {
                assert!(
                    !cx.read(|cx| old.is_asset_cached(cx)),
                    "theme changes release the previous decoded raster"
                );
            }
            if let Some(previous) = previous_png {
                assert_ne!(png, previous, "switching theme recolors the actual diagram");
            }
            previous_png = Some(png);
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
                assert_eq!(
                    viewer.read(cx).revision,
                    revision,
                    "ordinary redraws preserve the raster request and cached image"
                );
            });
        }
        let database = pecan_mermaid::system_fonts();
        let installed = |candidates: &[&str]| {
            candidates
                .iter()
                .find(|name| {
                    database
                        .faces()
                        .any(|face| face.families.iter().any(|(family, _)| family == *name))
                })
                .unwrap_or_else(|| panic!("preview regression needs one of {candidates:?}"))
                .to_string()
        };
        let sans = installed(&["PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC"]);
        let serif = installed(&["Songti SC", "SimSun", "Noto Serif CJK SC"]);
        let mono = installed(&["Menlo", "Consolas", "DejaVu Sans Mono"]);
        for (english, chinese) in [
            (sans.as_str(), sans.as_str()),
            (mono.as_str(), serif.as_str()),
        ] {
            let old_image = viewer.read_with(cx, |view, _| view.image.clone().unwrap());
            let previous = old_image.bytes().to_vec();
            visual.update(|_, cx| {
                Preferences::shared(cx).update(cx, |prefs, cx| {
                    prefs.change(
                        |data| {
                            data.mermaid_english_font = english.into();
                            data.mermaid_chinese_font = chinese.into();
                        },
                        cx,
                    )
                });
            });
            for _ in 0..6 {
                visual.run_until_parked();
                visual.executor().advance_clock(Duration::from_millis(100));
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            let revision = viewer.read_with(cx, |view, _| {
                assert_eq!(view.fonts, DiagramFonts::new(english, chinese));
                assert!(
                    view.image.as_ref().unwrap().bytes() != previous.as_slice(),
                    "the open preview renders the new fonts"
                );
                view.revision
            });
            assert!(
                !cx.read(|cx| old_image.is_asset_cached(cx)),
                "font changes release the previous decoded raster"
            );
            visual.update(|window, cx| {
                window.refresh();
                window.draw(cx).clear(cx);
                assert_eq!(
                    viewer.read(cx).revision,
                    revision,
                    "ordinary redraws retain the new font raster"
                );
            });
        }
        assert!(
            visual.debug_bounds("modal-close").is_none(),
            "the viewer relies on native window controls instead of an extra close button"
        );
        let last_image = viewer.read_with(cx, |view, _| view.image.clone().unwrap());
        drop(viewer);
        visual.simulate_keystrokes("escape");
        cx.run_until_parked();
        assert!(
            !cx.windows().contains(&viewer_handle),
            "Escape removes only the viewer window"
        );
        assert!(cx.windows().contains(&handle), "the document stays open");
        assert!(
            !cx.read(|cx| last_image.is_asset_cached(cx)),
            "closing the preview releases its decoded raster"
        );
    }

    #[gpui_kit::test]
    #[ignore = "set PECAN_VERIFY_DOCUMENT to a local Markdown file with Mermaid diagrams"]
    fn external_document_diagrams_fit_and_render_at_retina_resolution(cx: &mut TestAppContext) {
        let path =
            std::env::var_os("PECAN_VERIFY_DOCUMENT").expect("provide a local validation document");
        let document =
            pecan_document::Document::parse(std::fs::read_to_string(path).unwrap()).unwrap();
        let diagrams: Vec<_> = document
            .blocks()
            .iter()
            .filter_map(|block| {
                if let pecan_document::Block::Mermaid { code, .. } = block {
                    Some(&**code)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !diagrams.is_empty(),
            "validation document must contain Mermaid"
        );
        let queue = RenderQueue::new();
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_reduce_motion(true);
            cx.bind_keys([gpui_kit::KeyBinding::new(
                "escape",
                CloseDiagram,
                Some("DiagramViewer"),
            )]);
            crate::install_fullscreen_escape(cx);
        });
        for (index, code) in diagrams.iter().enumerate() {
            let (handle, _) = cx.update(|cx| {
                gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                    let card =
                        cx.new(|cx| DiagramCard::new((*code).into(), queue.clone(), window, cx));
                    cx.new(|_| DiagramDocument(card))
                })
                .unwrap()
            });
            let mut visual = VisualTestContext::from_window(handle, cx);
            for _ in 0..6 {
                visual.run_until_parked();
                visual.update(|window, cx| {
                    window.refresh();
                    window.draw(cx).clear(cx);
                });
            }
            let target = visual
                .debug_bounds("diagram-preview")
                .expect("real Mermaid must render");
            visual.simulate_click(target.center(), gpui_kit::Modifiers::default());
            let viewer = cx
                .windows()
                .into_iter()
                .find(|other| *other != handle)
                .unwrap();
            let mut visual = VisualTestContext::from_window(viewer, cx);
            visual.update(|window, _| window.set_scale_factor(2.));
            for dimensions in [size(px(640.), px(600.)), size(px(1512.), px(950.))] {
                visual.simulate_resize(dimensions);
                for _ in 0..6 {
                    visual.run_until_parked();
                    visual.executor().advance_clock(Duration::from_millis(100));
                    visual.update(|window, cx| {
                        window.refresh();
                        window.draw(cx).clear(cx);
                    });
                }
                let canvas = visual.debug_bounds("diagram-canvas").unwrap();
                let image = visual.debug_bounds("enlarged-diagram").unwrap();
                assert!(
                    canvas.contains(&image.origin) && canvas.contains(&image.bottom_right()),
                    "diagram {index} is entirely visible: image {image:?}, canvas {canvas:?}"
                );
                visual.update(|window, cx| {
                    let root = window.root::<gpui_kit::base::Root>().flatten().unwrap();
                    let view = root
                        .read(cx)
                        .view()
                        .clone()
                        .downcast::<DiagramViewer>()
                        .unwrap();
                    let bytes = view.read(cx).image.as_ref().unwrap().bytes();
                    let raster_width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
                    let raster_height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
                    assert!(
                        raster_width as f32 >= f32::from(image.size.width) * window.scale_factor()
                            && raster_height as f32
                                >= f32::from(image.size.height) * window.scale_factor(),
                        "diagram {index} has enough source pixels for Retina display"
                    );
                });
            }
            visual.update(|window, _| window.toggle_fullscreen());
            visual.simulate_keystrokes("escape");
            cx.run_until_parked();
            assert!(
                !cx.windows().contains(&viewer),
                "Escape closes the fullscreen Mermaid viewer"
            );
            cx.update_window(handle, |_, window, _| window.remove_window())
                .unwrap();
            cx.run_until_parked();
        }
        eprintln!("validated {} real Mermaid diagrams", diagrams.len());
    }
}
