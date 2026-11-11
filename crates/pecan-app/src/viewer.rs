//! Screen-sized native media windows and layout-driven fitting.
use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    App, IntoElement, MouseButton, MouseMoveEvent, Pixels, Point, ScrollHandle, SharedString, Size,
    Window, WindowBounds, WindowOptions, canvas,
    component::{ActiveTheme, Disableable, IconName, TitleBar, h_flex, v_flex},
    div,
    prelude::*,
    px, size,
};

#[derive(Clone, Copy)]
pub(crate) enum MediaKind {
    Image,
    Diagram,
}

#[derive(Clone, Copy)]
pub(crate) enum ZoomAction {
    In,
    Out,
    Fit,
    Original,
}

/// A shared pointer gesture; a lost mouse-up is ended by the next unpressed move.
#[derive(Default)]
pub(crate) struct Pan {
    previous: Option<Point<Pixels>>,
}
impl Pan {
    pub(crate) fn begin(&mut self, position: Point<Pixels>) {
        self.previous = Some(position);
    }
    pub(crate) fn end(&mut self) {
        self.previous = None;
    }
    pub(crate) fn is_active(&self) -> bool {
        self.previous.is_some()
    }
    pub(crate) fn advance(&mut self, event: &MouseMoveEvent, scroll: &ScrollHandle) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.end();
            return;
        }
        let Some(previous) = self.previous.replace(event.position) else {
            self.previous = None;
            return;
        };
        scroll.set_offset(scroll.offset() + event.position - previous);
    }
}

/// Both media types use the same compact footer, icon targets and zoom actions.
pub(crate) fn toolbar(
    kind: MediaKind,
    percent: f32,
    limits: (bool, bool),
    action: impl Fn(&ZoomAction, &mut Window, &mut App) + 'static,
    cx: &App,
) -> gpui_kit::AnyElement {
    let action = std::rc::Rc::new(action);
    let prefix = match kind {
        MediaKind::Image => "image",
        MediaKind::Diagram => "modal",
    };
    let selector = match kind {
        MediaKind::Image => "image-toolbar",
        MediaKind::Diagram => "diagram-toolbar",
    };
    let button = |suffix, icon, label, zoom, disabled| {
        let action = action.clone();
        crate::ui::tool_button(format!("{prefix}-{suffix}"), icon, label)
            .debug_selector(move || format!("{prefix}-{suffix}"))
            .disabled(disabled)
            .on_click(move |_, window, cx| action(&zoom, window, cx))
    };
    let controls = h_flex()
        .debug_selector(move || selector.into())
        .h(px(36.))
        .px_1()
        .gap_1()
        .items_center()
        .rounded(px(4.))
        .bg(cx.theme().muted)
        .border_1()
        .border_color(cx.theme().border)
        .child(button(
            "minus",
            IconName::Minus,
            "缩小（−）",
            ZoomAction::Out,
            limits.0,
        ))
        .child(
            div()
                .min_w(px(48.))
                .text_center()
                .text_size(px(12.))
                .child(format!("{percent:.0}%")),
        )
        .child(button(
            "plus",
            IconName::Plus,
            "放大（＋）",
            ZoomAction::In,
            limits.1,
        ))
        .child(div().w(px(1.)).h(px(14.)).bg(cx.theme().border))
        .child(button(
            "fit",
            IconName::Frame,
            "适应窗口",
            ZoomAction::Fit,
            false,
        ))
        .child(button(
            "original",
            IconName::Maximize,
            "原始尺寸",
            ZoomAction::Original,
            false,
        ));
    h_flex()
        .debug_selector(move || match kind {
            MediaKind::Image => "image-footer".into(),
            MediaKind::Diagram => "diagram-footer".into(),
        })
        .w_full()
        .h(px(52.))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .child(controls)
        .into_any_element()
}

/// Media viewers handle Escape by closing themselves, including in fullscreen.
pub(crate) fn is_media_window(window: &Window, cx: &App) -> bool {
    window
        .root::<gpui_kit::base::Root>()
        .flatten()
        .is_some_and(|root| {
            let view_type = root.read(cx).view().entity_type();
            view_type == std::any::TypeId::of::<crate::media::ImageViewer>()
                || view_type == std::any::TypeId::of::<crate::diagram::DiagramViewer>()
        })
}

/// A small breathing space around the media; the rest belongs to the image.
pub(crate) const MEDIA_INSET: f32 = 12.;

pub(crate) fn options(title: &str, parent: &Window, cx: &App) -> WindowOptions {
    let mut options = crate::native_window_options(title, cx, parent.viewport_size());
    if let Some(display) = parent.display(cx) {
        options.display_id = Some(display.id());
        options.window_bounds = Some(WindowBounds::Windowed(display.visible_bounds()));
    }
    options.window_min_size = Some(size(px(640.), px(420.)));
    options
}

pub(crate) fn frame(
    title: impl Into<SharedString>,
    content: impl IntoElement,
    toolbar: impl IntoElement,
    cx: &mut App,
) -> impl IntoElement {
    let prefs = crate::preferences::Preferences::shared(cx)
        .read(cx)
        .data
        .clone();
    let interface = crate::presentation::interface_style(cx);
    v_flex()
        .size_full()
        .refine_style(&interface)
        .text_size(px(prefs.ui_size as f32))
        .bg(cx.theme().background)
        .text_color(cx.theme().foreground)
        .child(
            TitleBar::new().child(
                div()
                    .w_full()
                    .px(px(90.))
                    .text_center()
                    .truncate()
                    .text_size(px(13.))
                    .child(title.into()),
            ),
        )
        .child(content)
        .child(toolbar)
}

/// Observe the actual canvas, including native titlebar and toolbar layout.
/// Notify only when its size changes, so this never starts a redraw loop.
pub(crate) fn measure_available(
    previous: Size<Pixels>,
    changed: impl FnOnce(Size<Pixels>, &mut App) + 'static,
) -> impl IntoElement {
    canvas(
        |bounds, _, _| bounds.size,
        move |_, available, window, cx| {
            if available != previous {
                window.defer(cx, move |_, cx| changed(available, cx));
            }
        },
    )
    .absolute()
    .size_full()
}

pub(crate) fn fit_scale(dimensions: (f32, f32), available: Size<Pixels>) -> f32 {
    ((f32::from(available.width) - MEDIA_INSET * 2.).max(1.) / dimensions.0.max(1.))
        .min((f32::from(available.height) - MEDIA_INSET * 2.).max(1.) / dimensions.1.max(1.))
}
